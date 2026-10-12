# Status

Progress checklist for the roadmap. Tick a box in the commit that completes it; acceptance items record their evidence (test names, numbers, report links). Each completed phase gets a local tag `phase-N`.

**Current phase: 1 (Kernel)**

## Phase 0: Bootstrap

- [x] Repository and identity: `main` branch, local author and committer `hasan-muzak <hasanmuzak@hotmail.com>`, commit hooks installed, local-only files excluded
- [x] Hook tests: a banned word, a co-author trailer, an em dash and a malformed subject are rejected; a valid Conventional Commit passes
- [x] Reference study: cadcraft and craft-libs cloned outside the repository; craft-libs license confirmed as `MIT OR Apache-2.0` (craft-libs rev `3c9702725d88`), so `craft-ui` can be used directly
- [x] Workspace skeleton: every crate of the layer table as an empty library with workspace lints, `apps/pixel3d`, `apps/pixel3d-cli`, `xtask`
- [x] `rust-toolchain.toml` (1.98.1), `rustfmt.toml`, `clippy.toml`, build profiles
- [x] `deny.toml` with the license allow list and banned crates
- [x] `cargo xtask emdash`
- [x] `cargo xtask layers` (ranks inside layers: ADR 0001)
- [x] `cargo xtask ci` and `cargo xtask ci-fast` (ci-fast tests changed crates and their dependents)
- [x] CI workflows (`ci.yml`, `nightly.yml`): `cargo xtask ci` on Windows, macOS and Linux
- [x] Licenses: `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`, `ATTRIBUTION.md`
- [x] `README.md` (product description)
- [x] `docs/STATUS.md` with every phase as checkboxes
- [x] `docs/architecture.md`
- [x] Third-party versions re-checked on crates.io and pinned in `[workspace.dependencies]` (stable releases only: `smallvec` 1.16.3, `notify` 8.2.0, `wasmtime` 49.0.2; `muda` 0.21.2 patch update)

Verification notes:

- `transform-gizmo-egui` 0.11.0 and `egui-snarl` 0.12.0 both depend on `egui ^0.36`: compatible.
- `craft-ui` (craft-libs) is `MIT OR Apache-2.0` and needs Rust 1.95 or newer; the pinned toolchain is 1.98.1.
- Tangent-space candidates for Phase 6: `mikktspace` 0.3.0 (MIT or Apache-2.0), `bevy_mikktspace` 1.0.0 (Zlib and MIT or Apache-2.0).

Acceptance:

- [x] `cargo xtask ci` passes locally (macOS arm64, 2026-10-12: all 7 steps; 27 xtask tests in `emdash`, `layers`, `changed`; 33 crates, no layer violations; `cargo deny check` clean)
- [ ] CI passes on Windows, macOS and Linux (pending: no remote configured yet; the workflow runs on the first push)
- [x] Hook tests behave as specified
- [x] `git log` shows only `hasan-muzak <hasanmuzak@hotmail.com>` (author and committer of every commit)

## Phase 1: Kernel

- [x] `math`: unit parser, transforms, axis conventions
- [x] `mesh`: half-edge kernel, attributes, groups, validation, compaction, triangle buffers
- [x] `scene`: document, ids, names, hierarchy, settings
- [ ] `query`: selectors
- [ ] `ops`: trait, registry, schemas, errors, outputs
- [ ] `engine`: Session, transactions, undo and redo, checkpoints, `catch_unwind` rollback, jobs, linear history graph
- [ ] `.p3d` save and load
- [ ] Ops: `scene.*`, `prim.box`, `prim.cylinder`, `prim.quad_sphere`, `prim.plane`, `select.*` basics
- [ ] `cargo xtask codegen`
- [x] `cargo xtask bench-scale` (criterion `scale_*` groups at n, 4n, 16n; log-log slope <= 1.25)

Acceptance:

- [ ] Property tests green
- [x] 1M-triangle grid builds in < 200 ms (37 ms, `cargo bench -p p3d-geo --bench prim -- prim/plane/1000000`, M-series laptop)
- [ ] Save then load gives identical blob hashes
- [ ] Undo and redo < 5 ms

## Phase 2: Agent surface v0

- [ ] `pixel3d-cli`: `mcp` over stdio, `run <script.json>`, `convert`
- [ ] MCP tools: `scene_overview`, `describe`, `query`, `run`, `list_ops`, `describe_op`, `history`, `docs`
- [ ] Perception v0: `perceive` aspects `structure`, `shape`, `silhouette`, `section`, `relations`, `probe`, `diff`
- [ ] Diagnostics v0: geometry and relations findings with locations and fixes
- [ ] Expectations v0: `dims`, `symmetric`, `attached`, `not_intersecting`, `budget`, `watertight`
- [ ] `render`: offscreen wgpu (shaded, wire, matcap, contact sheet, part-label overlays)
- [ ] `analyze` v0: topology metrics
- [ ] `docs/agents/` guides v0

Acceptance:

- [ ] An MCP client builds a beveled crate in at most 3 calls
- [ ] The broken-mesh corpus yields the exact expected findings
- [ ] Perception outputs stay under 600 tokens
- [ ] MCP snapshot tests
- [ ] CLI cold start < 150 ms

## Phase 3a: Modelling ops

- [ ] All primitives and generators
- [ ] Curves
- [ ] Edit ops
- [ ] Shading and normals
- [ ] Subdivision
- [ ] Deformers
- [ ] `cleanup.auto`
- [ ] Booleans (benchmark manifold-rust against boolmesh, choose, ADR)
- [ ] glTF, OBJ and FBX import

Acceptance:

- [ ] Generated op docs
- [ ] Every op has tests and a bench
- [ ] The geometry of T1, T2, T4 and T5 can be modelled through MCP

## Phase 3b: Brief, Model Spec and SDF

- [ ] `spec`: Brief, Model Spec, parts, anchors, landmarks, surface coordinates
- [ ] `spec`: constraints and solver
- [ ] `spec`: joins `separate`, `blend`, `weld`, `decal`; mirror and instance; edit layers
- [ ] `spec`: incremental compile to the op graph, provenance
- [ ] `sdf`: SDF trees, narrow-band surface nets
- [ ] `remesh`: isotropic, then field-aligned quads with category topology rules
- [ ] Shape kinds `silhouettes` and `shell`; deform handles
- [ ] Perception `proportions`, `visibility`, `brief`
- [ ] Expectations from briefs and categories
- [ ] `history bisect`
- [ ] MCP tools `brief`, `spec`, `perceive`, `expect`, `deform`

Acceptance:

- [ ] The elf head blockout compiles from a spec
- [ ] A one-part patch recompiles in < 100 ms
- [ ] Every face has provenance
- [ ] Remeshed SDF parts reach >= 95% quads with no poles above valence 6 outside designed spots

## Phase 3c: Feature library, strands, full diagnostics

- [ ] `features` v1: `ear`, `nose`, `eye`, `head_base`, `hand`, `horn`, `bolt`, `rivet`, `vent_grille`, `panel_inset`, `molding`
- [ ] `strands`: hair, fur, eyebrows, grass, cables as cards, clumps and shells
- [ ] Full diagnostics: intent, edge flow, shading, detail scale, with predicted fixes
- [ ] MCP tool `features`

Acceptance:

- [ ] T11 geometry built in at most 6 agent calls with no image calls
- [ ] The diagnostics corpus passes
- [ ] Feature templates keep clean topology across their parameter ranges (property tests)

## Phase 4: UV

- [ ] Seams
- [ ] LSCM
- [ ] ARAP and SLIM
- [ ] Rectification and alignment
- [ ] Texel density
- [ ] Packer
- [ ] Lightmap UVs
- [ ] Metrics
- [ ] `uv_auto`
- [ ] UV layout render

Acceptance:

- [ ] Mean stretch <= 1.05, utilization >= 75%, zero overlaps, density CV <= 5% on the test corpus
- [ ] Beats xatlas on utilization and island count at equal or lower stretch
- [ ] 100k triangles < 1.5 s

## Phase 5: Optimization and glTF export

- [ ] Decimation and LOD chain
- [ ] Buffer optimization
- [ ] Collision primitives and convex hull
- [ ] glTF and GLB writer
- [ ] Profiles `gltf` and `godot`
- [ ] `validate`, `export`, `make_game_ready` (without bake)
- [ ] Godot smoke test in CI

Acceptance:

- [ ] T3 and T10 assets pass validation and import into headless Godot with correct scale and orientation

## Phase 6: Materials and baking

- [ ] Materials, presets, textures
- [ ] Channel packing, normal conventions, MikkTSpace
- [ ] CPU baking: normal, AO, curvature, thickness, ID, position, height
- [ ] `decal` joins baked into normal maps
- [ ] Generated hair atlas textures for strands
- [ ] Dilation; PNG, EXR and KTX2 output
- [ ] `bake` tool and full `make_game_ready`

Acceptance:

- [ ] Ray miss < 0.1% on test pairs
- [ ] Seam error under threshold
- [ ] 2k normal + AO bake < 10 s

## Phase 7: Benchmark v1

- [ ] Agent-eval harness
- [ ] Complete static-asset analyzer
- [ ] Tasks T1 to T5, T9, T10, T11 (static) and T13
- [ ] First report against both Blender contenders
- [ ] Gaps it reveals fixed or tracked

Acceptance:

- [ ] Report committed; pixel3d leads every static task, or each gap is tracked here

## Phase 8: GUI shell

- [ ] Title bar, toolbar, docking, theme, code-painted icons, status bar
- [ ] Start page, command line and palette, shortcuts, menus (native on macOS)
- [ ] Viewport (offscreen render plus composite), camera navigation, grid, view cube
- [ ] Picking, object and element selection, gizmo
- [ ] Outliner, schema-driven Properties, History panel
- [ ] Control server
- [ ] kittest snapshots

Acceptance:

- [ ] Open, edit and save a `.p3d` with gizmo and undo
- [ ] 60 fps at 10M triangles
- [ ] UI snapshot tests green on all OSes

## Phase 9: GUI tools

- [ ] Contextual toolbar for all ops, live preview, Quick Properties, Dynamic Input
- [ ] UV Editor
- [ ] Health, Bake and Export panels
- [ ] Workspaces
- [ ] `pixel3d-cli mcp --connect` to watch agents live

Acceptance:

- [ ] A first-time user completes T1 with the mouse alone in under 10 minutes (moderated test)
- [ ] An agent completes T1 while the user watches in the app

## Phase 10: FBX writer, Unity and Unreal profiles

- [ ] FBX writer: static meshes, materials, LODs, collisions, tangents
- [ ] ufbx read-back tests
- [ ] Local Unity and Unreal import scripts

Acceptance:

- [ ] T1, T2, T4, T5 import into Unity and Unreal with identity transforms, correct scale and materials

## Phase 11: Rigging

- [ ] Skeletons, templates and engine name maps
- [ ] Auto-fit
- [ ] Bone heat, voxel fallback, rigid bind
- [ ] Weight tools
- [ ] IK
- [ ] Rig validation
- [ ] Skinned glTF and FBX export

Acceptance:

- [ ] T6 rig: zero unweighted vertices, influences within profile, volume loss under threshold, imports as a humanoid in all three engines
- [ ] T12 quadruped rig passes the same checks with the `quadruped` template
- [ ] Spec-driven characters get skeleton fit from their landmarks with no manual joint placement

## Phase 12: Animation

- [ ] Clips, curves, keys
- [ ] Generators
- [ ] Retargeting
- [ ] Compression
- [ ] Validation
- [ ] Timeline UI
- [ ] Export

Acceptance:

- [ ] T6, T7, T8, T12 pass validation
- [ ] Foot slide < 1 cm per step; loops seamless

## Phase 13: Benchmark v2

- [ ] All thirteen tasks, report

Acceptance:

- [ ] Product goal met or remaining gaps tracked

## Phase 14: Plugins and scripting

- [ ] WASM host and WIT world
- [ ] Rust addon SDK with `#[p3d::op]`
- [ ] Capabilities and hot reload
- [ ] TypeScript/JavaScript scripting host
- [ ] `.p3dx` packaging and `addon` CLI
- [ ] Example addon and scripts
- [ ] Generated `pixel3d.d.ts`

Acceptance:

- [ ] Example Rust addon and TypeScript script each register an op callable from GUI, MCP and graph
- [ ] A trapping addon cannot crash the app
- [ ] Hot reload under 1 s

## Phase 15: Node graph editor

- [ ] Node graph editor over the history graph
- [ ] Branching
- [ ] Parameter expressions (TypeScript/JavaScript)
- [ ] `graph.variant`
- [ ] Disk cache

Acceptance:

- [ ] Editing an upstream parameter of a 30-node graph re-evaluates only the dirty subgraph
- [ ] Variants of T1 generated in one call

## Phase 16: Assistant panel

- [ ] `LlmProvider` trait and first provider
- [ ] Assistant dock tab: streaming chat, tool cards with undo, stop, approval mode, context chips, token meter
- [ ] API key in the OS keychain

Acceptance:

- [ ] T1 completed from the Assistant panel with every step undoable and visible

## Phase 17: Packaging and release

- [ ] Installers for all OSes
- [ ] macOS signing and notarization; Windows signing
- [ ] Auto-update
- [ ] Opt-in local crash reports
- [ ] User docs

Acceptance:

- [ ] Signed installers install and run on clean machines of all three OSes
- [ ] The notarized macOS build loads a WASM addon

## Phase 18: Advanced

- [ ] Reference-image silhouette extraction for briefs
- [ ] Feature library v2
- [ ] GPU baking
- [ ] USD
- [ ] Engine live-link plugins
- [ ] Procedural texture generators
- [ ] Asset library
- [ ] Light sculpting
