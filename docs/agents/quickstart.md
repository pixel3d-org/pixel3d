# pixel3d quickstart

pixel3d makes 3D game assets. You build and check models with typed operations and read them back as text: no code, no screenshots needed.

## Conventions

- Units: meters. Lengths accept numbers (meters) or strings with units: `0.6`, `"600mm"`, `"60cm"`, `"2ft"`. Angles are degrees.
- Axes: right-handed, **+Y up**, **+Z is the asset's front**, X runs left (-) to right (+). Primitives stand on the ground: their origin is the bottom center.
- Objects are addressed by name (`crate`), path (`crate/lid`) or id (`o:3`). Ids never change; names can.
- Geometry is addressed by **groups**, never by indices: a box has `top`, `bottom`, `front`, `back`, `left`, `right` (and `bevel`); ops that create faces return groups such as `fill_1.faces`.
- Every `run` is one transaction: all ops succeed or nothing changes, and it is one undo step (`history undo`).

## The loop

1. `scene_overview`: what exists.
2. `expect add`: write the tests first (size, symmetry, parts attached, budget, watertight).
3. `run`: create and edit with ops in one batch.
4. Read the answer: every change ends with `problems:` and `tests N/M pass` lines.
5. `perceive` / `describe` to look closer; `diagnose` for problems with ready-to-run fixes and predicted results; `diagnose fix F2` applies one.
6. Repeat 3 to 5 until the tests pass and there are no problems.

## A beveled crate in two calls

```json
{"tool": "run", "arguments": {"ops": [
  {"op": "prim.box", "args": {"name": "crate", "width": "600mm", "height": "600mm", "depth": "600mm", "bevel": {"width": "8mm", "segments": 2}}},
  {"op": "expect.add", "args": {"expect": {"dims": {"target": "crate", "height": {"min": "590mm", "max": "610mm"}}}}},
  {"op": "expect.add", "args": {"expect": {"watertight": {"target": "crate"}}}}
]}}
```

```json
{"tool": "describe", "arguments": {"target": "crate"}}
```

## Tools

| Tool | Use it to |
|---|---|
| `scene_overview` | see every part with size, position and contacts |
| `describe` | summarize one object: size, origin, shape, topology, groups, contacts |
| `run` | apply ops (`list_ops`, `describe_op` show them) |
| `perceive` | look without images: `structure`, `shape`, `silhouette`, `section`, `relations`, `probe`, `diff` |
| `diagnose` | find problems with causes and fixes; `explain` or `fix` one |
| `expect` | add, remove, list and check model tests |
| `query` | count and locate elements a selector picks; `save_as` a group |
| `analyze` | topology metrics (the numbers quality is judged by) |
| `history` | undo, redo, checkpoints, revert, diff between states |
| `import` | bring in a glTF, GLB, OBJ or FBX file (meters, +Y up), meshes repaired; paths inside the allowed folders (the folder the server starts in, plus `--allow-dir`) |
| `render` | optional pictures to confirm: views tiled into one PNG with part names; never needed to find problems |
| `docs` | these guides: `quickstart`, `modeling`, `debugging`, `ops` |

## Errors

Errors name the problem, the JSON path in your request, a hint and the nearest valid values:

```
error OUT_OF_RANGE at /ops/0/args/bevel/width: bevel width 0.6 m must be less than half the smallest size
hint: use less than 0.5 m
```

Fix the argument at that path and send the call again; nothing was changed.
