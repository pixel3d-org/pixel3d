# Status

Progress checklist for the roadmap. Tick a box in the commit that completes it; acceptance items record their evidence (test names, numbers, report links). Each completed phase gets a local tag `phase-N`.

**Current phase: 3a (Modelling ops). Phases 0 to 2 are met locally; their tags wait for CI on all three OSes (no remote yet).**

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
- [x] `query`: selectors
- [x] `ops`: trait, registry, schemas, errors, outputs
- [x] `engine`: Session, transactions, undo and redo, checkpoints, `catch_unwind` rollback, jobs, linear history graph
- [x] `.p3d` save and load (`docs/file-format.md`; 1M triangles: save 200 ms, load 116 ms)
- [x] Ops: `scene.*` (13), `prim.box`, `prim.cylinder`, `prim.quad_sphere`, `prim.plane`, `select.save/grow/shrink/group_delete/group_rename`
- [x] `cargo xtask codegen` (`docs/ops/*.md`, `sdk/ts/pixel3d.d.ts` (checked with `tsc --strict`), `sdk/wit/pixel3d.wit`; `--check` runs in `ci` and `ci-fast`)
- [x] `cargo xtask bench-scale` (criterion `scale_*` groups at n, 4n, 16n; log-log slope <= 1.25)

Acceptance:

- [x] Property tests green (`crates/mesh/tests/props.rs`: validity, compaction, triangle buffers, normals, flip, mirror transform, split and append on random meshes with random deletions; `crates/geo/tests/props.rs`: closed outward primitives over their parameter ranges; `cargo xtask ci`: 191 tests, 21 kernels within the n^1.25 scaling limit)
- [x] 1M-triangle grid builds in < 200 ms (37 ms, `cargo bench -p p3d-geo --bench prim -- prim/plane/1000000`, M-series laptop)
- [x] Save then load gives identical blob hashes (and identical archive bytes: `save_load_save_gives_identical_bytes_and_blob_hashes` in `crates/io/tests/p3d.rs`)
- [x] Undo and redo < 5 ms (0.44 us per undo + redo pair on a 1M-triangle, 201-object scene; `cargo bench -p p3d-engine --bench session`; tiny op transaction 0.95 us)

## Phase 2: Agent surface v0

- [x] `pixel3d-cli`: `mcp` over stdio, `run <script.json>`, `convert` (`apps/pixel3d-cli`; `run` also saves `.p3d` and prints per-op timings; `convert --to json`)
- [x] MCP tools: `scene_overview`, `describe`, `query`, `run`, `list_ops`, `describe_op`, `history`, `docs`, plus `perceive`, `diagnose`, `expect`, `analyze` (`crates/mcp`: transport-free `Agent` and an rmcp stdio server; guides, op list and scene as resources)
- [x] Perception v0: `perceive` aspects `structure`, `shape`, `silhouette`, `section`, `relations`, `probe`, `diff` (`crates/perceive`; text and JSON snapshots in `tests/aspects.rs`; 100k triangles, uncached including BVH builds: structure 22 ms, shape 27 ms, silhouette 21 ms, section 1 ms, relations 22 ms, probe 15 ms, diff 44 ms with findings cached per revision; `cargo bench -p p3d-perceive --bench aspects`)
- [x] Diagnostics v0: geometry and relations findings with locations and fixes (`crates/perceive/src/diagnose.rs`: 10 codes; fixes are op calls whose results the engine predicts on a copy, `crates/engine/src/predict.rs`; 100k triangles: 102 ms)
- [x] Expectations v0: `dims`, `symmetric`, `attached`, `not_intersecting`, `budget`, `watertight` (stored in the document and `.p3d`; ops `expect.add` and `expect.remove`; checks in `crates/perceive/src/expect.rs`, every type passing and failing in `crates/perceive/tests/expect.rs`)
- [x] `render`: offscreen wgpu (shaded, wire, matcap, contact sheet, part-label overlays) (`crates/render`: seven orthographic views, shaded, wire, shaded wire and matcap with 4x MSAA, labels only on visible parts from an ID pass; coverage within 2% of the projected area and golden images in `crates/render/tests/render.rs`; MCP `render` tool returns PNG image content)
- [x] `analyze` v0: topology metrics (`crates/analyze`: counts, face kinds, poles, angles, holes, pieces, cavities, degenerate elements, volume; `analyze` tool; 64k triangles in 3.5 ms)
- [x] `docs/agents/` guides v0 (`quickstart`, `modeling`, `debugging`; served by the `docs` tool and as `pixel3d://guide/*` resources)

Acceptance:

- [x] An MCP client builds a beveled crate in at most 3 calls (`apps/pixel3d-cli/tests/mcp_stdio.rs`, `crate_session`: one `run` builds the crate and adds two passing tests, then `describe` and a clean `diagnose`)
- [x] The broken-mesh corpus yields the exact expected findings (`p3d_testdata::scenes::broken_corpus`, 9 scenes: `crates/perceive/tests/diagnose.rs` asserts code, severity, part, side and fix op; `crates/engine/tests/fixes.rs` asserts every top fix is predicted to resolve its finding)
- [x] Perception outputs stay under 600 tokens (asserted for every aspect and diagnostics report in `crates/perceive/tests`; default tool answers under 800 in `crates/mcp/tests/agent.rs`)
- [x] MCP snapshot tests (the real `pixel3d-cli mcp` process over JSON-RPC: initialize, tools/list, resources/list and a session in `apps/pixel3d-cli/tests/snapshots`; every tool in `crates/mcp/tests/snapshots`)
- [x] CLI cold start < 150 ms (spawn to the MCP initialize answer, best of three: 2.9 ms release, 3.9 ms debug on an M-series laptop; `apps/pixel3d-cli/tests/mcp_stdio.rs`, `cold_start_to_initialized`)

## Phase 3a: Modelling ops

- [x] All primitives and generators (primitives: `box`, `cylinder`, `cone`, `quad_sphere`, `uv_sphere`, `ico_sphere`, `torus`, `tube`, `capsule`, `wedge`, `plane`; generators: `gen.lathe` (presets, points, smooth Bezier or curve profiles, partial turns, rings; slope 0.86), `gen.extrude_shape` (outlines with holes from points or joined curves, spade constrained Delaunay caps merged into quads, round or chamfered rims; slope 1.03), `gen.sweep` (rotation-minimizing frames by double reflection, twist, taper, mitered joints, fill or quad-grid caps; slope 0.99) and `gen.pipe` (filleted bends), `gen.loft` (resampled, aligned sections, straight or Catmull-Rom rows, caps; slope 0.93), `gen.array` (linear, radial, along a curve on rotation-minimizing frames, welding open seams; slope 0.71) and `gen.mirror` (mirror copy with seam weld, or bisect; slope 0.87), `gen.scatter` (seeded Poisson-disk instances on a hash grid, aligned to faces, random turn and scale; slope 1.12). Kernels in `crates/geo/src/generate/`, property tests in `crates/geo/tests/gen_props.rs`)
- [x] Curves (curve objects in the document and `.p3d` manifest; `curve.polyline`, `bezier` (automatic smooth handles or explicit ones), `circle`, `rectangle` (rounded corners), `resample` (arc length), `offset` (planar, miter or bevel joins, outward for closed curves), `fillet` (tangent arcs or chamfers), each in place or into a new object; kernels in `crates/geo/src/curve.rs` with property tests, slopes 0.91 to 1.09)
- [x] Edit ops (`edit.extrude`, `inset`, `bevel`, `loop_cut`, `bridge`, `fill`, `merge`, `merge_by_distance`, `dissolve`, `delete_elements`, `split`, `plane_cut`, `thicken`, `poke`, `triangulate`, `tris_to_quads`, `flip`, `recalc_normals`, `smooth`, `symmetrize`, `subdivide`; booleans below. All local edits apply one face patch (ADR 0003, `Mesh::apply_patch`, slope -0.01 for a fixed patch on growing meshes); property tests on random meshes in `crates/geo/tests/edit_props.rs`; 18 `scale_edit_*` kernels at slopes 0.95 to 1.05)
- [x] Shading and normals (`shade.smooth_by_angle`, `mark_sharp`, `clear_sharp`, `set_flat`, `set_smooth`, `weighted_normals` (area, angle or both; custom split normals that respect sharp edges), `crease`; kernels in `crates/geo/src/shade.rs`, slopes 1.00 and 1.01)
- [x] Subdivision (`subd.catmull_clark`: semi-sharp creases (DeRose et al. 1998; crease x 10 = sharpness, one level less per level), borders as creases with fixed corners, UVs and colors interpolated, groups, flags and creases carried; `crates/geo/src/subd.rs`, slope 1.04)
- [x] Deformers (`deform.bend`, `twist`, `taper`, `displace` (seeded fractal gradient noise along area-weighted normals); `crates/geo/src/deform.rs`, slopes 0.90 and 0.97; `lattice` is a later phase)
- [x] `cleanup.auto` and `cleanup.report` (weld until settled, degenerate collapse, specks, small holes except flat sheets, flat triangle pairs to quads, buried parts by BVH ray parity with cavities kept, outward by volume or area-weighted ray votes; idempotent by property test; slope 0.99; ADR 0007: soup repair such as non-manifold splits belongs to import)
- [x] Booleans (`edit.boolean`: difference, union, intersect with one or several tools; manifold-rust exact engine behind `p3d_geo::boolean::Engine` after a 212-case comparison with boolmesh (ADR 0008); cleanup merges the triangles of each source polygon back, refills faces with holes as convex polygons (Hertel and Mehlhorn), drops cut points on straight edges, welds slivers, interpolates UVs and colors, carries materials, sharp and seam flags, creases and the target's groups; `<label>.cut` and `<label>.edges`; warning for cuts across low-poly curved surfaces; property test on random primitives (closed results, volumes add up, 20,000 cases); slope 0.96)
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

- [x] Title bar, toolbar, docking, theme, code-painted icons, status bar (`crates/ui`: `chrome.rs` Blender-style top bar with the menus (integrated macOS title band), tool settings row with the contextual object actions, vertical tool column, viewport header; `docking.rs` craft-ui dock with a protected viewport; `theme.rs` dark and light tokens of project spec 20.3; `icons.rs` icons painted in code; `apps/pixel3d`, run `cargo run -p pixel3d -- --sample`; `--screenshot <png>` saves the window)
- [x] Start page, command line and palette, shortcuts, menus (native on macOS) (`chrome.rs` Start page and the shortcut table; `commands.rs` one index of menu commands, tools, modes and every registered op, searched by label, alias (Blender, Maya, 3ds Max names) and description, tested in `commands::tests`; `palette.rs` Cmd+K or Ctrl+K and F3; `cmdline.rs` the Console's command line with suggestions, Tab completion, history and `name=value` or JSON parameters, tested in `cmdline::tests::lines_run_and_log`; `menus.rs` one menu tree for the in-window menus and the native macOS menu, `apps/pixel3d/src/native_menu.rs` via muda)
- [x] Viewport (offscreen render plus composite), camera navigation, grid, view cube (`crates/render/src/viewport.rs` and `viewport.wgsl`: 4x MSAA HDR offscreen pass with reverse-Z depth, id pass, composite into the window, x-ray and the element overlay, tested headless in `crates/render/tests/viewport.rs`; `crates/ui/src/viewport.rs` orbit, pan and zoom from mouse, wheel and trackpad, smooth view changes; the view cube is the clickable navigation gizmo of ADR 0005, `navgizmo.rs`)
- [x] Picking, object and element selection, gizmo (GPU id picks for objects and a distance pick for curves; `elements.rs` vertex, edge and face picking with a CPU ray cast and box selection that skips hidden elements unless x-ray, tested in `elements::tests`; `gizmo.rs` Move, Rotate and Scale handles following the header's orientation, pivot and snapping, one undo step per drag through `scene.set_transform` or `edit.transform` (previewed on a working copy), tested in `gizmo::tests`)
- [x] Outliner, schema-driven Properties, History panel (`outliner.rs`: search, tree, eye toggles, rename in place, context menu; `inspector.rs`: Tool, Scene, Object and Mesh tabs, the transform edited through `scene.set_transform` with one undo step per drag, "adjust last operation" re-running the op; `schema_form.rs`: one schema-to-widget mapping, tested on every built-in op in `schema_form::tests::forms_lay_out_every_builtin_op`; `panels.rs` History: the undo list, click to step back)
- [x] Control server (`crates/control`: loopback JSON-lines server and client, at most 16 clients, 60 s timeout answering `not-run` or `may-have-run`, tested in `p3d_control::tests`; `crates/ui/src/remote.rs`: `ui.inspect`/`ui.state`, `ui.elements`, `ui.click`, `ui.drag`, `ui.key`, `ui.text` as real input events, `ui.screenshot`, `ui.render`, `ui.command`; `pixel3d --control PORT` and File > Allow agents to connect; tested in `remote::tests::agents_drive_the_app_through_the_server`)
- [x] kittest snapshots (`crates/ui/tests/snapshots.rs`: the main window in dark and light, Edit Mode with faces selected and the command palette, drawn headless with egui_kittest on wgpu and compared with `crates/ui/tests/snapshots/*.png` within a small tolerance; `UPDATE_SNAPSHOTS=1` rewrites them; green on macOS, other OSes not run yet)

Acceptance:

- [x] Open, edit and save a `.p3d` with gizmo and undo (`apps/pixel3d/tests/control.rs`, run with `cargo test -p pixel3d --test control -- --ignored`: the app saves and reopens a `.p3d`, the gizmo's X arrow is dragged with real pointer events through the control server, Cmd+Z undoes, the file is saved and a screenshot taken; passed on macOS)
- [x] 60 fps at 10M triangles (93 fps orbiting 10,485,760 triangles in 8 objects, release build, vsync off, `view.benchmark` driven over the control server, 1.6k x 1k window on an Apple Silicon Mac; UI frame time 0.24 ms because counts are cached per document state; other GPUs not measured yet)
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
