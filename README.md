# pixel3d

pixel3d is a cross-platform 3D modelling application for Windows, macOS and Linux, written entirely in Rust. It makes game assets: clean topology, clean UVs, correct bakes, polycounts on budget, LODs, collision, rigs and animations, exported in one step to Unity, Unreal Engine and Godot. AI agents and people use exactly the same tools.

> Status: early development. Nothing is usable yet.

## Why

General-purpose modelling tools driven by agents through free-form scripts are slow, fragile and unsafe: operators depend on hidden context, errors are vague, the agent has to look at screenshots to understand what it built, and the exported asset still needs manual fixing before it works in an engine. pixel3d is designed the other way round.

## Pillars

1. **Agent-native and spec-first.** A model's source of truth is a declarative Model Spec: named parts, shapes, parametric features, strands for hair and fur, anchors at landmarks, and constraints. Geometry is compiled from it, the way source code compiles into a binary.
   - Agents understand the result through text perception: measurements, outlines, sections, relations, probes and diffs. Screenshots are never required.
   - Problems are reported like compiler diagnostics: a location on a named part, the cause, the fixes and their predicted results.
   - The user's request becomes a Brief whose expectations run as tests on every compile.
   - Every action is a typed, validated, deterministic and undoable operation. There is no free-form code execution.
2. **Game-ready by default.** Exported assets have clean topology, non-overlapping UVs with consistent texel density, correct bakes, a polycount on budget, LODs, collision, a rig and animations when needed, and the axes, units, naming and texture packing the target engine expects.
3. **Understandable.** About a hundred well-named, orthogonal operations instead of thousands of operators and modifiers. A visible history instead of a hidden modifier stack. A Health panel that says what is wrong and fixes it in one click.

## Features (planned)

- Half-edge mesh kernel, primitives, generators, edit operations, subdivision, deformers, booleans and cleanup.
- SDF modelling with field-aligned quad remeshing, a library of parametric features (ears, noses, hands, horns, bolts, rivets) and strands (hair cards, fur shells).
- Automatic UVs: seam placement in hidden areas, low-distortion unwrapping, island straightening, texel-density-aware packing, with quality metrics.
- Decimation, LOD chains, collision proxies and budget checks per asset class.
- Materials, channel packing and CPU baking of normal, AO, curvature, thickness and more.
- Rigging with templates and engine name maps, automatic skin weights, IK, and animation clips with generators and compression.
- Engine profiles for Unity, Unreal Engine, Godot and plain glTF, with validated exports.
- An MCP server for agents, a procedural node graph, a sandboxed TypeScript/JavaScript scripting tier and WASM addons.
- A desktop GUI built with egui and wgpu.

## Building

Requires the Rust toolchain pinned in `rust-toolchain.toml` (installed automatically by rustup).

```sh
cargo run -p pixel3d          # desktop app
cargo run -p pixel3d-cli      # headless command-line interface
cargo xtask ci                # everything CI runs
```

## License

Licensed under either of the MIT License ([LICENSE-MIT](LICENSE-MIT)) or the Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option. See [NOTICE](NOTICE) and [ATTRIBUTION.md](ATTRIBUTION.md).
