# ADR 0008: manifold-rust is the boolean engine

- Status: accepted
- Date: 2026-10-13

## Context

`edit.boolean` (union, difference, intersect) needs a robust mesh boolean engine. The spec (3a) asks to benchmark the two pure Rust candidates, manifold-rust and boolmesh, behind a trait and to record the choice. Both descend from Emmett Lalish's Manifold:

- manifold-rust 0.16 is a port of Manifold 3.5. It also has a second "robust" engine on exact rational arithmetic for triangle soup. License Apache-2.0. Dependencies: clipper2-rust (BSL-1.0), dashu (MIT or Apache-2.0), num-traits.
- boolmesh 0.2 is a separate implementation in Manifold's style. It carries any per-corner value through the cut. License MPL-2.0 (allowed only as an unmodified dependency). Dependencies: glam 0.32, so a second glam next to our 0.34.

## Measurements

`cargo bench -p p3d-geo --bench boolean_eval` on an Apple M-series laptop. A result counts as correct when it builds into a closed `p3d_mesh::Mesh` and its volume is right. Volume is checked against analytic values, and for random pairs by inclusion and exclusion: |A| + |B| = |A union B| + |A intersect B|.

The 212 cases are 12 named ones plus 200 random pairs of boxes and ico spheres (rotated and offset, all three operations). The named cases include offset cubes for each operation, touching and coplanar cubes, a cube with itself, a 1e-9 shift, a rotated cube, and sphere and cube.

| | manifold-rust (Exact engine) | boolmesh |
|---|---|---|
| Correct results | 212 / 212 | 208 / 212 |
| Errors | 0 | 4 (empty result reported as an error, "empty pos matrix") |
| Not manifold, wrong volume, inconsistent | 0, 0, 0 | 0, 0, 0 |
| Union of two spheres, 3.9k triangles each | 7.4 ms | 10.0 ms |
| 16k | 26.6 ms | 41.9 ms |
| 64k | 98.0 ms | 153.4 ms |
| 256k | 395 ms | 671 ms |

Both scale about linearly. Correctness is a tie apart from boolmesh's empty-result handling, which a wrapper could fix.

## Decision

Use manifold-rust (Exact engine) behind `p3d_geo::boolean::Engine`, a trait with one method from two triangle meshes to a triangle mesh, so the engine can be replaced. Reasons:

- It is 1.6 to 1.7 times faster at every size.
- Its license is permissive (Apache-2.0), with no MPL file obligations.
- It carries cooperative cancellation and progress through the whole pipeline. That maps directly onto `OpCtx`, so long booleans can be cancelled.
- Per-triangle provenance (`face_id`, `run_original_id`) lets the op carry groups, materials and attributes from the source faces to the result.
- Its robust engine accepts non-manifold soup, so imported meshes can be handled later without changing engines.
- It is deterministic, including with its `parallel` feature.

boolmesh's per-corner value interpolation is attractive, but manifold-rust's vertex properties cover UVs as well.

The comparison harness stays in history (this ADR's commit). After the switch it is reduced to manifold-rust and kept as a correctness and speed check for engine upgrades.
