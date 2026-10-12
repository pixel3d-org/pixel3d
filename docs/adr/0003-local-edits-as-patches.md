# ADR 0003: Local mesh edits as face patches

- Status: accepted
- Date: 2026-10-13

## Context

Phase 3 adds about twenty edit operations (extrude, inset, bevel, loop cut, bridge, merge, dissolve, split, plane cut, poke, triangulate, tris to quads, subdivide, symmetrize, ...). Element ops must cost O(affected elements), not O(mesh) (project spec 21.1), and every edit must leave a valid 2-manifold half-edge mesh with consistent winding, attributes and groups.

Two common designs:

1. Euler operators (split edge, split face, join faces, collapse edge, ...) composed per op. Each op becomes a sequence of fine-grained pointer rewiring steps, every intermediate state must stay valid, and each op needs its own careful attribute and group handling.
2. Rebuilding the whole mesh from a polygon list after each edit. Simple and robust, but O(mesh) per op, so a batch of k edits costs O(k n).

## Decision

One kernel primitive in `p3d-mesh`: a `Patch` lists faces to remove, vertices to add and polygons to add, and `Mesh::apply_patch` applies it atomically. It behaves like `Mesh::from_polygons` restricted to the patch:

- New polygons reuse existing half-edges where they meet the rest of the mesh (a boundary half-edge, or one freed by a removed face) and create new edges elsewhere.
- Edges and vertices left without faces are deleted.
- The fans of every touched vertex are rebuilt from face links; a vertex whose faces fall apart into several fans is split (one copy per fan), so the result is always 2-manifold. Boundary loops are relinked per fan.
- Corner attributes of new corners are blends of two existing corners, read before anything changes; new faces join the face groups of a `like` face and take a material; new vertices join the vertex groups of a `like` vertex.
- A polygon edge already used in the same direction by a face that stays, or by another new polygon, is an error, and nothing changes on error.

The cost is O(c log c + valences of the touched vertices + groups x new elements) for the c corners involved, independent of the mesh size (`scale_mesh_patch_local` measures a slope of about 0 from 4k to 64k triangles). New elements are appended; `Mesh::compact` reclaims dead slots.

Every edit op in `p3d-geo` computes its removed faces and new polygons and applies one patch.

## Consequences

- Ops are short and uniform: geometry decisions in the op, topology bookkeeping in one well-tested place (unit tests plus property tests that re-insert, poke and extrude random regions of random meshes).
- Ops decide vertex identity themselves; for example extrude makes one copy per fan of region faces around a vertex, because a region can touch itself at a single vertex.
- Element ids of new faces and vertices differ from the removed ones; groups carry identity across edits, as the selector design already requires.
