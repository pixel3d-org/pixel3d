# Seeing and fixing models without images

## Perception (`perceive`)

Every aspect answers one question in under about 600 tokens, in millimeters, naming parts.

| `aspect` | Answers | Options |
|---|---|---|
| `structure` | what is where: parts, sizes, box centers, groups, contacts, mirror pairs | `target` |
| `shape` | what each part looks like: best-fit primitive, axes, volume, how full its box is, thickness, edge sharpness, mirror symmetry | `target` |
| `silhouette` | how the outline reads: a polyline per outline labelled by the part forming each stretch, extremes | `view` (front, back, left, right, top, bottom), `points` |
| `section` | how thick, what is inside: loops of a slice, holes with wall thickness | `axis` (x, y, z), `at`, `points` |
| `relations` | do parts fit: touching, gaps, how deep a part sinks into another, floating parts | `target` |
| `probe` | what is at one spot: normal, face, groups, thickness, nearest other part | `point` |
| `diff` | what an edit changed: parts added, removed, moved or resized, outline shift, problems added and resolved | `from` (revision or checkpoint) |
| `visibility` | how it reads at the asset class's view distance on 1080p: pixel size per part, parts and walls too small to model (normal map instead), surface never seen, edges under a pixel, triangles against screen share | |
| `proportions` | a spec root's category ratios against its style's range (`width_to_height`, `depth_to_height`, `eye_line` for heads) and every part's size as a share of the main part | `target` (spec root) |
| `brief` | coverage of the brief the target's spec answers: each check passing, failing or pending with its measured value | `target` |
| `dna` | visual DNA: form hierarchy, curved share and creases, taper, silhouette solidity, bevel variation and part ratios against the spec's `dna` (or `dna` given here), then the art-direction critique (`twinning`, `tangent`, `uniform_bevel`, `weak_silhouette`) | `target`, `dna` |

Surface points for `probe`: `{"on": "crate", "landmark": "top"}` (top, bottom, front, back, left, right, center), `{"on": "crate", "nearest": [x, y, z]}`, `{"on": "crate", "ray": {"from": "+x", "at": [y, z]}}`, `{"on": "crate", "group": "front"}`.

## Diagnostics (`diagnose`)

Findings read like compiler errors: id, severity, code, part, message with numbers, place (world position, side, and the nearest named landmark), cause, and fixes with their predicted result:

```
F1 warn FLOATING_PART ear_l -> head: touches nothing: 28 mm from head
    at (103, 125, -7.4) (left of ear_l, near head.ear_root_l); /parts/ear_l/anchor places it (its anchor on head at ear_root_l)
    fix: spec.patch {"ops":[{"op":"add","path":"/parts/ear_l/anchor/offset","value":[0.0016,0.001,0.0013]}],"target":"elf"} (move it 28 mm onto head (anchor offset))  predicted: resolved
```

The JSON form of each finding also carries `measured` and `threshold` in `unit` (`mm`, `deg`, `px`, `%`, `count` or `ratio`), `landmark`, and `cause_path`: a JSON pointer into the part's spec (the node that made the faces there, or the value that causes the problem). On parts compiled from a spec, fixes are spec patches or edit layers (`/parts/<name>/edits`), so the next compile keeps them; on plain objects they are ops.

`diagnose` predicts the first fixes of errors and warnings by running them on a copy: `resolved` or what remains, the measure they change (`coverage 55% -> 93%`, `tris -48`) and findings they would add. When the first option would not resolve the problem and another would, that one comes first. Info findings and long-running fixes (remeshes) say `not predicted`: `explain: "F1"` predicts every option. `fix: "F1"` applies the top option as one undoable step; `option: 1` picks another.

| Code | Severity | Found when (threshold) | Cause | Fix |
|---|---|---|---|---|
| `INVERTED_NORMALS` | error | a closed piece faces inward | spec node of its faces | `edit.recalc_normals` |
| `OPEN_BOUNDARY` | warn | holes in a part that is not a flat sheet | spec node of its faces | `edit.fill` |
| `DEGENERATE_FACES` | warn | collapsed or sliver faces, edges under 1 micrometer | spec node of its faces | none yet |
| `DUPLICATE_FACES` | warn | faces lying exactly on copies of themselves | spec node of its faces | none yet |
| `SELF_INTERSECTION` | warn | the surface passes through itself | spec node of its faces | `scene.separate` when loose pieces overlap |
| `LOOSE_PIECES` | info | several separate pieces in one part | | `scene.separate` |
| `THIN_WALL` | warn | walls under 1 mm | spec node of its faces | `edit.shrink_fatten` to 1.5 mm |
| `FLOATING_PART` | warn | a part touches nothing and is off the ground (strands are left out) | its anchor or position | move it onto its nearest part (anchor offset or position) |
| `PENETRATION` | warn | a part sinks deeper than a quarter of its thickness (or its `max_penetration` metadata, set by spec anchors) | its anchor or position | pull it out |
| `Z_FIGHTING` | warn | faces lying on same-facing faces of another part | its anchor or position | move it 0.5 mm off |
| `EXPECTATION_FAILED` | error | a test of `expect` fails | the expectation | the fix of a finding on the same part |
| `BRIEF_UNMET` | warn | a brief item fails (size, budget, feature count, feature ratio, attached, symmetric, closed) | the brief item | scale the asset (size), scale the feature parts (ratio), else the fix of a finding on the part |
| `PROPORTIONS` | warn | a category ratio is outside its style's range (`width_to_height`, `depth_to_height`, `eye_line`) | the main part | scale its width or depth to the range |
| `ASYMMETRY` | warn | left and right parts written out twice differ by more than 3% in size or mirrored place | the first spec value of the right part that overrides the mirror | set it to the left part's value, or make the right part a `mirror` |
| `JOINT_LOOPS` | warn | under 3 edge loops within one limb diameter of a joint landmark (`elbow`, `knee`, `wrist`, ...) of a part tagged `deforming` | `height_segments` of a primitive, `detail` of an SDF part | more rings, finer voxels, or a `remesh.quad` edit |
| `FACE_LOOPS` | warn (info unless `deforming`) | the edge flow around `eye_l`, `eye_r` or `mouth` of a humanoid head does not circle them (flow 0.5 or more off circular) | spec node of its faces | a `remesh.quad` edit with loops around them |
| `DEFORM_TRIANGLES` | warn | over 5% triangles in a part tagged `deforming` | spec node of its faces | `edit.tris_to_quads` or a `remesh.quad` edit |
| `DEFORM_POLES` | warn | vertices with 6 or more edges in a quad part tagged `deforming` | spec node of its faces | a `remesh.quad` edit |
| `DENSITY_JUMP` | info | neighbouring faces of a `deforming` part differ more than 4x in size | spec node of its faces | a `remesh.quad` edit |
| `SMOOTHING_ARTIFACT` | warn | smooth-shaded edges fold more than 60 degrees over at least 2.5% of the surface | spec node of its faces | `shade.smooth_by_angle` 45 degrees |
| `CURVED_NGON` | warn | n-gons whose corners leave their plane by more than 2% of their size | spec node of its faces | `edit.triangulate` those faces |
| `HARD_EDGE_NO_SEAM` | warn | hard edges whose UVs run on across them (the bake draws a line) | spec node of its faces | `shade.clear_sharp` when they are gentle, else cut seams |
| `DETAIL_SCALE` | warn | a part's cross-section is under 2 px at the asset class's view distance on 1080p | the part | `join: "decal"` in the spec (tag `decal` on objects): bake it into textures |
| `BALD_SPOT` | warn | strands cover under 90% of their region (largest bald spot in cm2) | `params/clumps` of the strands part | more clumps with the card width kept, or wider cards |
| `PENETRATING_STRANDS` | warn | strand faces dip more than 0.5 mm into the surface they grow on (or the report counts guide points inside it) | `params/segments` of the strands part, or the open surface | more segments, lift, or close the surface |

Edge flow checks run on parts tagged `deforming` (spec `"tags": ["deforming"]`); joints are landmarks named after joints. Decals and hidden parts are left out of shading, detail and edge flow. Finding ids belong to one revision: run `diagnose` again after an edit.

## Expectations (`expect`)

Tests for the model run after every change; answers end with `tests N/M pass; failing: ...`.

```json
{"action": "add", "expect": {"dims": {"target": "crate", "width": {"min": "590mm", "max": "610mm"}}}}
{"action": "add", "expect": {"symmetric": {"a": "handle_l", "b": "handle_r"}}}
{"action": "add", "expect": {"attached": {"part": "lid", "to": "crate", "max_gap": "1mm"}}}
{"action": "add", "expect": {"not_intersecting": {"parts": ["lid", "handle_l", "handle_r"]}}}
{"action": "add", "expect": {"budget": {"max_tris": 1500}}}
{"action": "add", "expect": {"watertight": {"target": "crate"}}}
```

Write the expectations from the user's request first, then model until they pass.

## History

`history diff` (or `perceive` with `aspect: "diff"`) compares two states; `checkpoint` names a state before a risky change and `revert` returns to it as an undoable step.

## Finding the change that broke something

`history` with `"action": "bisect"` searches the undo history for the transaction that made a test fail: `{"test": {"expect": "E2"}}`, `{"test": {"finding": {"code": "PENETRATION", "part": "ear_l"}}}` or `{"test": {"brief": {"name": "elf", "item": "ear length"}}}`. It answers with the revision, the transaction and, for specs, the paths it changed (`/parts/ear_l/sdf/0/capsule/b/0: 0.05 -> 0.02`), in a handful of checks however long the history is.
