# ADR 0001: Ranks inside crate layers

- Status: accepted
- Date: 2026-10-12

## Context

The layer table (`docs/architecture.md`) puts several crates on the same layer, and some of them clearly need each other: `mesh` builds on `math`, `engine` runs `ops` and compiles `spec`, `features` builds on `sdf` and `remesh`, the assistant client in `ai` exposes the same tool definitions as `mcp`. A rule of "strictly lower layers only" would forbid all of that; a rule of "anything in the same layer" would allow cycles of intent and hide coupling. The renderer is a special case: it sits on L2 next to the kernels but must depend on L0 and L1 only.

## Decision

Every crate gets a layer and a rank. A crate may depend on any crate of a lower layer, and on crates of its own layer with a strictly lower rank. `render` is isolated: it depends only on lower layers and no L2 crate depends on it. `testdata` is a support crate: it may depend on L0 to L3, and other crates may use it only as a dev-dependency.

| Layer | Rank 0 | Rank 1 | Rank 2 | Rank 3 | Rank 4 |
|---|---|---|---|---|---|
| L0 | `math` | `mesh` | | | |
| L1 | `scene` | `query` | | | |
| L2 | `geo`, `tex` | `uv`, `opt`, `rig`, `bake` | `anim`, `sdf`, `remesh` | `strands`, `vegetation` | `features` |
| L3 | `profiles` | `io`, `analyze` | `perceive` | | |
| L4 | `ops` | `graph` | `spec` | `engine` | |
| L5 | `plugin`, `control` | `mcp` | `ai` | `ui` | |

(`features` moved to rank 4 when the vegetation templates arrived, ADR 0014.)

External crate rules, checked on direct dependencies of every crate and, for L0 to L4 and `testdata`, on everything reachable through normal and build dependencies:

- UI toolkit and windowing crates (egui family, eframe, winit, rfd, muda, arboard, accesskit, craft-ui, gizmo): L5 only.
- `wgpu`: `render` and L5 only (direct check only, because L3 and L4 may use `render`).
- `tokio`, `axum`, `reqwest`: `mcp` and `ai` only.
- `wasmtime`, `rquickjs`, `boa_engine`, `oxc`: `plugin` only.

`cargo xtask layers` implements the table (`xtask/src/layers.rs`) and runs in `ci` and `ci-fast`.

## Consequences

- Moving a crate to another rank is a one-line change in the table plus an update of this ADR and `docs/architecture.md`.
- Same-rank crates cannot depend on each other. If two of them need shared code, it moves down to a lower rank or layer.
- Dev-dependencies follow the same layer rules, except for `testdata`.
