# Architecture

This document describes how pixel3d is put together: the principles, the crate layers and their machine-checked rules, the process and threading model, and the core data model. Decisions that change it are recorded as ADRs in `docs/adr/`.

## Principles

- **Spec-first and compiled.** A model's source of truth is a declarative Model Spec (named parts, shapes, parametric features, strands, anchors at landmarks, constraints). Meshes are build artifacts of the spec, with per-face provenance back to the spec node that produced them. Agents and users understand the result through text perception, debug it through compiler-style diagnostics, and verify it against expectations generated from the Brief.
- **One operation registry.** Every action is an operation in one registry. The same registry feeds the GUI (menus, toolbars, command palette, properties), the CLI, the MCP server, the control server, the node graph, scripts and WASM addons. Operations are typed, validated before execution, deterministic and undoable.
- **The UI never owns document data.** It renders the `Session` and sends commands.
- **Copy-on-write snapshots.** Documents and meshes are shared through `Arc` and cloned only where touched, which makes undo, previews, background jobs and transactions cheap.
- **Layered crates with a machine-checked dependency rule** (`cargo xtask layers`, below).
- **Performance by construction.** Structure-of-arrays, `u32` indices, adjacency walks and spatial structures instead of pairwise loops, reused scratch buffers, rayon for large data-parallel work with deterministic results. Every non-O(1) function documents its complexity.

## Crate layers

Directory `crates/<name>`, package `p3d-<name>`, library `p3d_<name>`.

| Layer | Crates | Responsibility |
|---|---|---|
| L0 foundation | `math`, `mesh` | Units, angles, axes, transforms, predicates; the half-edge kernel, attributes, groups, topology queries, validation, triangle buffers |
| L1 document | `scene`, `query` | Document, objects, hierarchy, materials, textures, skeletons, clips, ids, names, settings; the selector language |
| L2 algorithms | `geo`, `tex`, `uv`, `opt`, `rig`, `bake`, `anim`, `sdf`, `remesh`, `strands`, `vegetation`, `features` | Pure kernels with no IO and no global state |
| L2 algorithms | `render` | wgpu renderer for the viewport and headless renders, no egui |
| L3 IO and analysis | `profiles`, `io`, `analyze`, `perceive` | Engine profiles; `.p3d`, glTF, FBX, OBJ, USD; metrics and validation; perception, diagnostics and expectation checks |
| L4 engine | `ops`, `graph`, `spec`, `engine` | Op trait, registry and schemas; node graph evaluation; Brief and Model Spec compiler with provenance; Session, transactions, undo, jobs, events, fix prediction |
| L5 frontends | `plugin`, `control`, `mcp`, `ai`, `ui` | WASM addon and script hosts; JSON-lines control server; MCP server; assistant panel client; egui UI |
| support | `testdata` | Procedural test meshes and fixtures (dev-dependency only) |
| apps | `apps/pixel3d`, `apps/pixel3d-cli` | Desktop app; headless CLI |
| tooling | `xtask` | `ci`, `ci-fast`, `layers`, `emdash`, and later `codegen`, `bench-scale`, `dist` |

### Rules

A crate may depend on every crate of a lower layer, and on crates of its own layer with a lower rank (ADR 0001):

| Layer | Rank 0 | Rank 1 | Rank 2 | Rank 3 | Rank 4 |
|---|---|---|---|---|---|
| L0 | `math` | `mesh` | | | |
| L1 | `scene` | `query` | | | |
| L2 | `geo`, `tex` | `uv`, `opt`, `rig`, `bake` | `anim`, `sdf`, `remesh` | `strands`, `vegetation` | `features` |
| L3 | `profiles` | `io`, `analyze` | `perceive` | | |
| L4 | `ops` | `graph` | `spec` | `engine` | |
| L5 | `plugin`, `control` | `mcp` | `ai` | `ui` | |

- `render` is isolated: it depends on L0 and L1 only, and no L2 crate depends on it.
- `testdata` depends on L0 to L3 and is only ever a dev-dependency.
- Apps and `xtask` may depend on anything and are never dependencies.
- Nothing below L5 depends on egui, eframe, winit, rfd, muda, arboard, tokio, axum, reqwest, wasmtime or a script engine, directly or through other crates.
- Only `render` and L5 crates depend on wgpu.
- Only `mcp` and `ai` depend on tokio, axum and reqwest.
- Only `plugin` depends on wasmtime, rquickjs, boa_engine and oxc.

`cargo xtask layers` checks every rule against the resolved dependency graph and runs in `cargo xtask ci` and `cargo xtask ci-fast`. Dependencies that exist only for wasm32 targets are ignored: pixel3d builds native targets (addons are separate WASM guests). The table lives in `xtask/src/layers.rs`; keep it, this document and ADR 0001 in sync.

## Process and threading model

```
                       +----------------------------+
 MCP client --stdio--> | pixel3d-cli mcp            |  headless Session (one thread)
 (agent)               |  rmcp (tokio) <-> channel  |  jobs on rayon pool
                       +----------------------------+
                                 | --connect 127.0.0.1:PORT (JSON-lines)
                                 v
 +---------------------------------------------------------------+
 | apps/pixel3d (desktop)                                        |
 |  UI thread: App { session: Session, ui: UiState, viewport }   |
 |   - logic(): drain control-server queue, poll jobs, events    |
 |   - ui(): draw panels, viewport via egui_wgpu callback        |
 |  control server thread (loopback TCP, JSON-lines)             |
 |  job pool (rayon): uv, bake, decimate, weights, export        |
 |  render: p3d-render (wgpu) shared by viewport and headless    |
 +---------------------------------------------------------------+
```

- **Session** (`engine`) owns the current `Arc<Document>`, the undo stack, the selection, active jobs, the op registry and the event bus. It is synchronous and has a single owner thread.
- **Jobs.** Long operations (unwrap, bake, decimate, auto-weights, export) run on the rayon pool. A job receives an `Arc` snapshot of its inputs and returns a patch; the owner thread commits the patch as one transaction, or rejects it and reruns the job if its targets changed meanwhile. Jobs report progress and accept cancellation through atomics.
- **MCP.** `pixel3d-cli mcp` hosts a headless Session. With `--connect 127.0.0.1:PORT` it forwards to a running desktop app through the control server, so the user can watch an agent work live: every tool becomes an `mcp.call` request that the app answers with the same `Agent` on its own Session (the agent borrows it for the call, so the changes land in the app's history and selection), and `ui_state`, `ui_command` and `ui_screenshot` map to `ui.state`, `ui.command`/`ui.commands` and `ui.screenshot`/`ui.render`. File > Allow agents to connect listens on port 7879 (`p3d_control::DEFAULT_PORT`), or a free one when that is taken. Loopback is not a boundary (any local process, and any web page through a browser request, reaches 127.0.0.1), so each connection must first send `hello` with the server's random 256-bit token; the app publishes it in a user-only file (`p3d_control::token_path`, `<local data dir>/control/<port>.token`, mode 0600 on Unix) that `--connect` reads, and a line that is not JSON closes the connection, so an HTTP request never gets past its request line. The tokio runtime lives only inside `mcp` (and `ai`) and talks to the Session thread, or the thread that owns the control client, over channels.
- **Control server.** A loopback-only JSON-lines server in the desktop app. Requests are queued over a channel and drained by the UI thread.
- **Security.** No operation executes arbitrary code. Listeners bind 127.0.0.1 only. Import and export paths are canonicalized and confined to allowed roots (`p3d_io::sandbox::FileAccess` on the session; `pixel3d-cli` allows the current folder, the opened scene's folder and each `--allow-dir`; the desktop app allows the open project's folder and every folder the user took a file from in this run, through a file dialog or by dropping it on the window, and each `--allow-dir`, and agents connected to it get those folders and no more (`crates/ui/src/files.rs`); ops see it as `OpCtx::files`). Addons run in a WASM sandbox, scripts in a sandboxed JavaScript engine.

## Data model

- **Units and axes.** Right-handed, +Y up, +Z is the asset's front, 1 unit = 1 meter (the glTF convention). Angles are radians internally and degrees at every API boundary; the API accepts unit strings. Engine conversions happen only on export, through engine profiles.
- **Document.** Objects in a hierarchy with unique path-like names (`chest/lid`) and short stable ids (`o:12`). Meshes, materials, textures, skeletons, clips, graphs, briefs and specs are stored behind `Arc` and shared copy-on-write.
- **Mesh kernel.** A polygon half-edge mesh in structure-of-arrays form with `u32` indices. Half-edges are allocated in pairs so the twin of `h` is `h ^ 1`. N-gons are allowed, quads are first-class, triangles are produced for export. Attributes are columns (corner UVs and normals, colors, materials, edge flags, creases, skin weights). Named groups survive edits and are how agents and the node graph refer to geometry. `validate()` checks every invariant after every op in debug builds and always in tests.
- **Transactions.** Each call (GUI command, MCP call, script call) is one transaction: ops apply to a working copy where only touched meshes are cloned, results are validated, then committed atomically. Errors and panics roll everything back. Undo and redo are pointer swaps between snapshots.
- **Files.** `.p3d` is a zip container with a stable-order `manifest.json` (settings, objects, specs, briefs, groups, metadata), content-addressed mesh blobs, textures and a thumbnail.
- **Determinism.** Identical inputs and seed give bit-identical output on one platform and outputs within 1e-5 across platforms. Hash map iteration order never reaches an output.

## Quality gates

- `cargo xtask ci-fast` before every commit: formatting, clippy with warnings denied, tests of the changed crates and their dependents, layers, and the em dash scan.
- `cargo xtask ci` reproduces CI: the above on the whole workspace, plus `cargo deny check` (license allow list, banned crates, advisories, sources) and a release build. CI runs it on Windows, macOS and Linux.
- Library crates deny `unwrap`, `expect`, `panic!`, `todo!`, `unimplemented!`, `dbg!` and printing; `unsafe` code is forbidden everywhere.
