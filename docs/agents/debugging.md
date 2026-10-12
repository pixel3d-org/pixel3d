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

Surface points for `probe`: `{"on": "crate", "landmark": "top"}` (top, bottom, front, back, left, right, center), `{"on": "crate", "nearest": [x, y, z]}`, `{"on": "crate", "ray": {"from": "+x", "at": [y, z]}}`, `{"on": "crate", "group": "front"}`.

## Diagnostics (`diagnose`)

Findings read like compiler errors: id, severity, code, part, message with numbers, place, the last op that changed the part, and fixes with their predicted result:

```
F1 warn FLOATING_PART lid -> crate: touches nothing: 12 mm from crate
    at (200, 412, -200) (bottom of lid); last changed by scene.set_transform (set_transform_1)
    fix: scene.set_transform {"position":[0.0,-0.012,0.0],"relative":true,"target":"crate/lid"} (move it 12 mm onto crate)  predicted: resolved
```

`diagnose` with `fix: "F1"` applies the first fix as one undoable step; `option: 1` picks another; `explain: "F1"` predicts every option.

| Code | Meaning | Fix |
|---|---|---|
| `INVERTED_NORMALS` | a closed piece faces inward | `edit.recalc_normals` |
| `OPEN_BOUNDARY` | holes in a part that is not a flat sheet | `edit.fill` |
| `DEGENERATE_FACES` | collapsed or sliver faces, edges under 1 micrometer | none yet |
| `DUPLICATE_FACES` | faces lying exactly on copies of themselves | none yet |
| `SELF_INTERSECTION` | the surface passes through itself | `scene.separate` when loose pieces overlap |
| `LOOSE_PIECES` | several separate pieces in one part (info) | `scene.separate` |
| `THIN_WALL` | walls under 1 mm | none yet |
| `FLOATING_PART` | a part touches nothing and is off the ground | move it onto its nearest part |
| `PENETRATION` | a part sinks deeper than a quarter of its thickness | pull it out |
| `Z_FIGHTING` | faces lying on same-facing faces of another part | move it 0.5 mm off |

Finding ids belong to one revision: run `diagnose` again after an edit.

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
