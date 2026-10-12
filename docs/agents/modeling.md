# Modeling with ops

Build assets as several named parts in a hierarchy (`crate`, `crate/lid`, `crate/handle_l`), not as one merged mesh: perception, diagnostics and tests reason about parts.

## Ops you have

- `prim.box`, `prim.cylinder`, `prim.cone`, `prim.quad_sphere`, `prim.uv_sphere`, `prim.ico_sphere`, `prim.torus`, `prim.tube`, `prim.capsule`, `prim.wedge`, `prim.plane`: real-size primitives with clean quads and semantic groups. Boxes take `bevel` (`{"width": "8mm", "segments": 2}`); round primitives adapt their segment count to the asset class. Prefer `quad_sphere` for balls, `capsule` for limbs, `torus` for hoops and rings, `tube` for pipes with wall thickness.
- `scene.*`: `create_empty`, `delete`, `duplicate` (copy or instance), `rename`, `parent`, `unparent`, `set_transform` (absolute, or `relative: true` to move by an offset), `apply_transform`, `set_origin`, `tag`, `set_metadata`, `join`, `separate`.
- `select.*`: `save` (selector to a group), `grow`, `shrink`, `group_delete`, `group_rename`.
- `edit.*`, shaping: `extrude` (pull faces out or push them in, groups `<label>.cap` and `<label>.side`), `inset` (a rim inside faces, optional depth for recesses, `<label>.inner` and `<label>.rim`), `bevel` (round or chamfer edges, `<label>.faces`), `thicken` (wall thickness for planes and open shells), `symmetrize` (mirror one half onto the other), `plane_cut` (cut along a plane, optionally drop a side and cap it), `smooth` (relax lumpy surfaces without shrinking).
- `edit.*`, topology: `loop_cut` (edge loops across a ring of quads), `bridge` (join two holes), `subdivide` (flat quad split), `poke`, `triangulate`, `tris_to_quads`, `split` (detach faces), `flip` (reverse whole parts), `merge`, `merge_by_distance` (weld seams after `scene.join`), `dissolve` (remove vertices, edges or faces; `mode: "limited"` simplifies flat areas), `fill` (close holes), `recalc_normals` (make parts face outward), `delete_elements`.
- Hard-surface detail: `inset` a face, then `extrude` its `.inner` group by a few millimeters in or out; `bevel` the sharp edges (`{"edges": {"sharp": true}}`) at 2 to 5 mm, 1 segment for small props, 2 or 3 for hero assets.
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
