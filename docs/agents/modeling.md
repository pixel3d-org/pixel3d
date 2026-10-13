# Modeling with ops

Build assets as several named parts in a hierarchy (`crate`, `crate/lid`, `crate/handle_l`), not as one merged mesh: perception, diagnostics and tests reason about parts.

## Ops you have

- `prim.box`, `prim.cylinder`, `prim.cone`, `prim.quad_sphere`, `prim.uv_sphere`, `prim.ico_sphere`, `prim.torus`, `prim.tube`, `prim.capsule`, `prim.wedge`, `prim.plane`: real-size primitives with clean quads and semantic groups. Boxes take `bevel` (`{"width": "8mm", "segments": 2}`); round primitives adapt their segment count to the asset class. Prefer `quad_sphere` for balls, `capsule` for limbs, `torus` for hoops and rings, `tube` for pipes with wall thickness.
- `gen.lathe`: revolve a profile (preset `barrel`, `vase`, `bottle`, `bowl`, `column`; points `[radius, height]`, `smooth: true` for a curve through them; or `{"curve": "name"}`) into round objects with groups `side`, `profile_<i>` (per span from the bottom), `cap_top`, `cap_bottom`. Use it for barrels, vases, bottles, columns, knobs, wheels.
- `gen.extrude_shape`: a flat outline (`outline` points plus `holes`, or a `curve` whose closed splines nest into holes; join an outline and hole curves with `scene.join`) turned into a solid of `depth`, with optional round or chamfered rim `bevel`. Groups `front`, `back`, `side`, `bevel`. Use it for signs, frames, gaskets, floor slabs (curves in the ground plane rise along +Y).
- `gen.sweep`: a cross section (`{"rectangle": [w, h]}`, `{"circle": r}`, points, or a curve) along a path (points, `smooth_path`, `path_closed`, or a curve) on twist-free frames, with `twist`, `scale` taper and mitered corners. Groups `side`, `cap_start`, `cap_end`, `profile_<i>`. Use it for rails, trims and moldings (open profiles), frames, handles, horns.
- `gen.pipe`: a round pipe along a path with smooth bends (`bend_radius`, twice the radius by default) and quad caps. Use it for plumbing, railings, cables, hoses, chair frames.
- `gen.loft`: a surface through cross sections (point lists or curves), resampled to one point count and aligned so it does not twist; `steps` and `smooth` add curved rows, `closed` makes rings. Groups `side`, `band_<i>`, `cap_start`, `cap_end`. Use it for square-to-round transitions, hulls, horns, vases from rings.
- `gen.array`: repeat a mesh in a line (`offset` or `relative_offset`), around an axis (`radial`) or along a curve (`curve`, copies turned to follow it); touching open edges weld. Use it for fences, stairs, bolt circles, spokes, chain links.
- `gen.mirror`: add the mirror image across an axis plane and weld the seam; `bisect` cuts and keeps one side first. Model one half, then mirror.
- `gen.scatter`: seeded, evenly spaced instances of a `source` object over a `target` surface (or `select`ed faces), standing on it with random turns and scales; instances share the source mesh. Use it for rocks on terrain, rivets on panels, props on tables.
- Generators built from a curve object land where the curve is (its parent and transform) unless you pass `parent` or `position`.
- `curve.*`: `polyline`, `bezier` (smooth through points, or explicit handles), `circle`, `rectangle` (optional rounded corners) create curve objects; `resample` (even spacing), `offset` (parallel outline, outward for closed curves) and `fillet` (round corners) reshape them in place or into a new object with `output`. Curves are profiles and paths for the generators.
- `scene.*`: `create_empty`, `delete`, `duplicate` (copy or instance), `rename`, `parent`, `unparent`, `set_transform` (absolute, or `relative: true` to move by an offset), `apply_transform`, `set_origin`, `tag`, `set_metadata`, `join` (meshes, or curves into one multi-spline curve), `separate`.
- `select.*`: `save` (selector to a group), `grow`, `shrink`, `group_delete`, `group_rename`.
- `edit.*`, shaping: `extrude` (pull faces out or push them in, groups `<label>.cap` and `<label>.side`), `inset` (a rim inside faces, optional depth for recesses, `<label>.inner` and `<label>.rim`), `bevel` (round or chamfer edges, `<label>.faces`), `thicken` (wall thickness for planes and open shells), `symmetrize` (mirror one half onto the other), `plane_cut` (cut along a plane, optionally drop a side and cap it), `smooth` (relax lumpy surfaces without shrinking).
- `edit.boolean`: cut tool objects out of a mesh (`difference`, the default), add them (`union`) or keep the overlap (`intersect`). Both must be closed. The result stays clean: split faces merge back, faces with holes become convex polygons, UVs, flags and groups carry over; the tools' faces are `<label>.cut`, the seam `<label>.edges`, and the tools are deleted unless `keep_tool`. Prefer generators and `extrude` for shapes they can make; use booleans for holes, slots and sockets in hard-surface parts, and avoid cutting low-poly curved surfaces (the facets show along the cut).
- `edit.*`, topology: `loop_cut` (edge loops across a ring of quads), `bridge` (join two holes), `subdivide` (flat quad split), `poke`, `triangulate`, `tris_to_quads`, `split` (detach faces), `flip` (reverse whole parts), `merge`, `merge_by_distance` (weld seams after `scene.join`), `dissolve` (remove vertices, edges or faces; `mode: "limited"` simplifies flat areas), `fill` (close holes), `recalc_normals` (make parts face outward), `delete_elements`.
- `edit.*`, moving elements: `transform` (move, rotate, scale selected vertices, edges or faces around a pivot, optionally with proportional editing), `shrink_fatten` (along normals), `edge_slide` (slide a loop across the surface); `plane_cut` with `select` cuts only those faces (a knife cut).
- Hard-surface detail: `inset` a face, then `extrude` its `.inner` group by a few millimeters in or out; `bevel` the sharp edges (`{"edges": {"sharp": true}}`) at 2 to 5 mm, 1 segment for small props, 2 or 3 for hero assets.
- `shade.*`: `smooth_by_angle` (30 degrees suits most props), `mark_sharp`, `clear_sharp`, `set_flat`, `set_smooth`, `weighted_normals` (run last on beveled hard-surface parts), `crease` (keeps edges sharp under subdivision).
- `subd.catmull_clark`: smooth subdivision of the whole mesh; crease edges first to keep them sharp.
- `deform.*`: `bend`, `twist`, `taper` (along an axis through the origin; give the shape segments along that axis first) and `displace` (seeded noise along normals, for rocks and terrain).
- `io.import` (or the `import` tool): reads glTF, GLB, OBJ and FBX with hierarchy, transforms, UVs, normals (hard edges become sharp), colors and material names (object metadata `materials`); every mesh is repaired into a clean manifold mesh, and a warning with a `cleanup.auto` fix says what is left to clean.
- `cleanup.auto`: one call makes a mesh clean (welds seams, collapses degenerate faces, removes specks and parts buried in other parts, fills holes up to 4 edges, joins flat triangle pairs into quads, turns parts outward); running it twice changes nothing. `cleanup.report` says what it would do without changing anything. Run it after imports and `scene.join`.
- `expect.add`, `expect.remove`: model tests.

`list_ops` with `search` finds ops by Blender or Maya names too (`cube`, `separate by loose parts`, `shift n`).

## Placing parts

- Children are placed in their parent's space: `{"op": "prim.box", "args": {"name": "lid", "parent": "crate", "position": [0, "600mm", 0]}}` sits on top of a 600 mm crate.
- Primitives stand on their origin (bottom center); `origin: "center"` centers them.
- Use `perceive` with `aspect: "probe"` and a landmark (`{"on": "crate", "landmark": "right"}`) to read the exact surface position and normal before attaching something there.
- Move by an offset with `scene.set_transform` and `relative: true`. Diagnostics fixes use exactly this form.
- Mirror pairs: create the left part, `scene.duplicate` it, then set the copy's position with x negated. `expect add symmetric {a, b}` checks them.

## Sizes and budgets

The document's asset class sets the triangle budget and adaptive detail:

| Class | Triangle budget |
|---|---|
| `prop_small` | 1,500 |
| `prop_medium` (default) | 6,000 |
| `prop_large` | 25,000 |
| `hero_prop` | 40,000 |

`analyze` and `expect add budget` measure against it.

## Selectors

Selectors pick elements by meaning:

```json
{"faces": {"group": "top"}}
{"faces": {"normal": {"dir": [0, 1, 0], "within_deg": 10}}}
{"edges": {"boundary": true}}
{"verts": {"in_box": {"min": [-1, 0, -1], "max": [0, 2, 1]}}}
{"and": [{"faces": {"group": "front"}}, {"faces": {"normal": {"dir": [1, 0, 0], "within_deg": 45}}}]}
{"grow": {"steps": 1, "of": {"faces": {"group": "top"}}}}
```

`in_box` uses object space. `query` shows what a selector picks before you change anything.
