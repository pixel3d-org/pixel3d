# Model Specs

A Model Spec is the source of a model; meshes are its build output. You write named parts and how they relate, pixel3d compiles them, and you change the model by patching the spec. Unchanged parts are reused, so a patch to one part recompiles that part (and its mirrors and blend parent) only.

- `spec.set {"name": "elf_head", "spec": {...}}` creates the root object `elf_head` with one child object per part, or gives an existing spec root a new spec.
- `spec.patch {"target": "elf_head", "merge": {...}}` changes it with a JSON Merge Patch: objects merge, `null` removes a key or a whole part, other values replace. For one array element (one SDF shape) use JSON Patch: `"ops": [{"op": "replace", "path": "/parts/head/sdf/1/radii/0", "value": 0.06}]`.
- `spec.compile {"target": "elf_head", "force": true}` rebuilds everything (after hand edits or an upgrade).

Errors point into the spec: `/spec/parts/eye_l/anchor/landmark: elf_head has no landmark eye_ll` with suggestions.

## Parts

Each part has exactly one shape:

| Key | Shape | Example |
|---|---|---|
| `sdf` | blended SDF shapes, as in `sdf.mesh` | `"sdf": [{"ellipsoid": {"at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09]}, "name": "cranium"}]` |
| `primitive` | one primitive by kind (`box`, `cylinder`, `cone`, `uv_sphere`, `quad_sphere`, `ico_sphere`, `torus`, `tube`, `capsule`, `wedge`, `plane`) | `"primitive": {"cylinder": {"radius": 0.1, "height": 0.02}}` |
| `lathe`, `sweep`, `loft`, `extrude`, `pipe` | the `gen.*` op with these parameters (inline points) | `"lathe": {"preset": "barrel", "height": 0.9}` |
| `mirror` | a reflected copy of another part (`plane`: x by default) | `"ear_r": {"mirror": "ear_l"}` |
| `instance` | another part's mesh, shared, at its own placement | `"post_2": {"instance": "post", "transform": {"position": [0.5, 0, 0]}}` |

Leave out `name`, `parent`, `position` and `target` in shape parameters: the spec places parts.

Optional keys:

- `detail`: the voxel size of an `sdf` part.
- `transform`: `position`, `rotate` (degrees about X, Y, Z) and `scale` in the root's space.
- `anchor`: where the part sits on another part (below).
- `join`: `separate` (default, its own object), `blend` (merged into its anchor parent with a fillet of `blend`, SDF parts on both sides), `decal` (kept hidden, baked onto the parent later). `weld` is not built yet.
- `edits`: ops run on the part after its shape, `[{"op": "edit.inset", "args": {...}, "label": "panel"}]`; leave out `target`. Keep manual work here: a recompile replaces hand edits to a part's mesh.
- `landmarks`: a category (`"humanoid_head"`, or `"category"` for the spec's `category`), or named surface coordinates of your own.
- `tags`, `material`, `importance`.

## Anchors and landmarks

`"anchor": {"on": "head", "landmark": "ear_root_l", "offset": [0, 0.01, 0]}` puts the part's origin on the parent's surface; the part keeps the parent's axes (or turns +Y along the surface normal with `"orient": "normal"`). Instead of a landmark you can give:

- `param`: `[u, v]` on the parent's bounding ellipsoid (u around +Y from +Z toward +X, v from bottom 0 to top 1), projected onto the surface;
- `ray`: `{"from": "+x", "height": 0.15}`, the first hit from that side;
- `nearest`: `[x, y, z]`, the closest surface point.

Landmarks of every part: `top`, `bottom`, `front`, `back`, `left` (+X, the character's left), `right`, `center`, and the names of its SDF shapes. Category `humanoid_head` adds `crown`, `chin`, `forehead`, `nose_root`, `nose_base`, `mouth`, `eye_l/r`, `brow_l/r`, `cheek_l/r`, `jaw_l/r`, `ear_root_l/r`, `temple_l/r`, `hairline_front`, `nape` and `back_of_head`. Anchors re-resolve when the parent changes: lengthen the lamp's pole and the shade moves up without a rebuild.

## Provenance

Every face records the spec node that made it: a blended ear's faces in the head mesh trace to `/parts/ear_l/sdf/0`, an inset's faces to `/parts/body/edits/0`. Face groups name them too: `cranium`, `jaw`, `ear_l.lobe`, `nose.shape_0`, `panel.inner`.

## Example: an elf head blockout

```json
{"name": "elf_head", "spec": {"category": "humanoid_head", "parts": {
  "head": {"sdf": [
      {"ellipsoid": {"at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09]}, "name": "cranium"},
      {"ellipsoid": {"at": [0, 0.06, 0.035], "radii": [0.055, 0.045, 0.055]}, "name": "jaw", "smooth_union": "2.5cm"}],
    "detail": "5mm", "landmarks": "category"},
  "nose": {"sdf": [{"capsule": {"a": [0, 0, 0], "b": [0, -0.025, 0.012], "radius": "6mm", "radius_b": "8mm"}}],
    "anchor": {"on": "head", "landmark": "nose_root"}, "join": "blend", "blend": "6mm"},
  "ear_l": {"sdf": [{"capsule": {"a": [0, 0, 0], "b": [0.04, 0.06, -0.02], "radius": "1cm", "radius_b": "2mm"}, "name": "lobe"}],
    "anchor": {"on": "head", "landmark": "ear_root_l"}, "join": "blend", "blend": "8mm"},
  "ear_r": {"mirror": "ear_l"},
  "eye_l": {"primitive": {"uv_sphere": {"radius": 0.012, "origin": "center"}},
    "anchor": {"on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008]}},
  "eye_r": {"mirror": "eye_l"}}}}
```

The ears and nose blend into one closed head mesh; the eyes are separate objects sunk 8 mm into their sockets. Then `spec.patch` with `{"parts": {"ear_l": {"sdf": [...]}}}` reshapes both ears and rebuilds only the head.
