# ADR 0007: Cleanup works on manifold meshes; soup repair belongs to import

- Status: accepted
- Date: 2026-10-13

## Context

The spec (9.8) lists the `cleanup.auto` passes: merge by distance, remove degenerate faces and zero-length edges, delete loose vertices and edges, unify winding, orient outward, fill small holes, split non-manifold elements, remove interior faces and limited dissolve on flat regions.

A `p3d_mesh::Mesh` is a half-edge mesh. `Mesh::from_polygons` and `Mesh::apply_patch` refuse non-manifold edges, inconsistent winding within a part, faces without edges and loose edges, and they split vertices whose faces fall into several fans. So a `Mesh` that exists can never contain non-manifold elements, loose vertices or edges, or a part with mixed winding. Those defects exist only in polygon soup: imported files and generated polygon lists before they become a mesh.

## Decision

- `cleanup.auto` (kernel `p3d_geo::cleanup`) runs the passes that apply to a valid mesh, in this order:
  1. weld until nothing moves
  2. collapse zero-area faces along a tiny edge
  3. remove specks
  4. fill small holes, except on flat sheets
  5. join coplanar triangle pairs into quads
  6. remove closed parts buried in another closed part's material
  7. turn parts outward

  Turning outward uses the signed volume for closed parts and an area-weighted ray vote for open parts.
- Non-manifold splits, winding unification and loose element removal run where soup becomes a mesh: the importers' soup builder. That builder reports its repairs in the same counts as `cleanup.report`.
- The "limited dissolve on flat regions" pass joins coplanar triangle pairs into quads instead of dissolving flat areas into n-gons. Dissolving would destroy intended grids (a plane with segments for later bends), and game meshes want quads. `edit.dissolve` with `mode: "limited"` stays available for real n-gon simplification.
- "Interior faces" means closed, outward parts buried in the material of another closed part, judged by ray parity against the other parts' faces. Inward closed parts inside another part are cavities (hollow walls) and stay. Faces of parts that cross each other are a job for `edit.boolean`.
- Every pass is best effort. A pass that cannot run leaves the mesh unchanged and adds a warning. A second run changes nothing, and a property test on random meshes checks this.

## Consequences

- `cleanup.auto` is safe to run on any mesh, any number of times.
- Import must include a soup repair step: weld, drop degenerate polygons, split non-manifold edges and fans, unify winding by BFS. Without it, broken files cannot become a `Mesh` at all.
- Thresholds that decide on geometry (open-part orientation, hidden parts) use clear majorities and leave ambiguous cases alone. That keeps results stable when the same surface is cut into different faces.
