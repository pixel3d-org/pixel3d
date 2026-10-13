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
| `shell` | a curved sheet from one outline (`gen.shell`: `cup`, `bend`, `twist`, `thickness`; a card when thin): ears, leaves, wings, fins | `"shell": {"outline": "M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 C -0.025 0.07 -0.03 0.02 0 0 Z", "cup": 0.3}` |
| `silhouettes` | a solid from front, side and top outlines (`sdf.silhouettes`), like a blueprint | `"silhouettes": {"front": "M ...", "side": [[-0.015, 0], [0.015, 0], [0.015, 0.2], [-0.015, 0.2]], "round": "3mm"}` |
| `ornament` | a motif repeated along `path` points (with the `normal` out of the surface) or around a `band` (`gen.ornament`): `stud`, `pyramid`, `bead`, `rune`, `rope`, `scroll`; `size`, `relief`, `sink`, `count` or `spacing`, `variation` | `"ornament": {"motif": "rune", "band": {"radius": 0.019}, "size": "2cm"}` anchored on a haft |
| `feature` | a feature template with `params` (the `features` tool lists them): `ear`, `eye`, `horn`, `cable`, `wing`, `bolt`, `rivet`, `vent_grille`, `panel_inset`, `molding`, `tree`, `flower` and `grass_cards` are designed meshes; `head_base`, `hand` and `nose` are SDF shapes and blend like `sdf` parts | `"feature": "ear", "params": {"variant": "elf", "length": "8cm", "pointiness": 0.9}` |
| `strands` | hair, fur, eyebrows, beards or grass (`hair`, `fur`, `eyebrows`, `beard`, `grass`) grown on the part named by `params.on`, placed with it; `region` names landmarks of that part (or `{"groups": [...]}`, or `"all"`), the rest as in `gen.strands` | `"strands": "hair", "params": {"on": "head", "region": ["hairline_front", "temple_l", "nape", "temple_r"], "length": "22cm"}` |
| `mirror` | a reflected copy of another part (`plane`: x by default) | `"ear_r": {"mirror": "ear_l"}` |
| `instance` | another part's mesh, shared, at its own placement | `"post_2": {"instance": "post", "transform": {"position": [0.5, 0, 0]}}` |

Leave out `name`, `parent`, `position` and `target` in shape parameters: the spec places parts.

Optional keys:

- `detail`: the voxel size of an `sdf` part.
- `transform`: `position`, `rotate` (degrees about X, Y, Z) and `scale` in the root's space.
- `anchor`: where the part sits on another part (below).
- `join`: `separate` (default, its own object), `blend` (merged into its anchor parent with a fillet of `blend`, SDF parts on both sides), `weld` (stitched into its anchor parent by an exact boolean union: one closed mesh with a crisp seam, the part's faces grouped under its name; both must be closed), `decal` (kept hidden, baked onto the parent later).
- `edits`: ops run on the part after its shape, `[{"op": "edit.inset", "args": {...}, "label": "panel"}]`; leave out `target`. Keep manual work here: a recompile replaces hand edits to a part's mesh.
- `landmarks`: a category (`"humanoid_head"`, or `"category"` for the spec's `category`), or named surface coordinates of your own.
- `tags`, `material`, `importance`.

## Anchors and landmarks

`"anchor": {"on": "head", "landmark": "ear_root_l", "offset": [0, 0.01, 0]}` puts the part's origin on the parent's surface; the part keeps the parent's axes (or turns +Y along the surface normal with `"orient": "normal"`). Instead of a landmark you can give:

- `param`: `[u, v]` on the parent's bounding ellipsoid (u around +Y from +Z toward +X, v from bottom 0 to top 1), projected onto the surface;
- `ray`: `{"from": "+x", "height": 0.15}`, the first hit from that side;
- `nearest`: `[x, y, z]`, the closest surface point.

A skeleton is a named set of joints (`"skeletons": {"rig": {"joints": {"knee": {"at": [0.1, 0.5, 0], "axis": [0, -1, 0], "parent": "hip"}}}}`; archetypes make them for you). Its name works like a part name and its joints are its landmarks: `{"on": "rig", "landmark": "knee"}` puts the part's origin on the joint, and `"orient": "normal"` turns the part into the joint's frame (+Y along `axis`, +X along `side`). Constraints reach joints as `rig.knee`. A part placed on a joint can still blend or weld into a part: `"into": "body"` names it (by default a blend merges into the part its anchor is on).

Landmarks of every part: `top`, `bottom`, `front`, `back`, `left` (+X, the character's left), `right`, `center`, the names of its SDF shapes, and a feature part's own landmarks (an ear's `root`, `tip`, `lobe` and `rim_back`; mirrors reflect them). Category `humanoid_head` adds `crown`, `chin`, `forehead`, `nose_root`, `nose_base`, `mouth`, `eye_l/r`, `brow_l/r`, `cheek_l/r`, `jaw_l/r`, `ear_root_l/r`, `temple_l/r`, `hairline_front`, `nape` and `back_of_head`. Anchors re-resolve when the parent changes: lengthen the lamp's pole and the shade moves up without a rebuild.

## Archetypes

An archetype is a category as data (ADR 0011, ADR 0016): a skeleton, parts, default genes and critique thresholds from a few numbers. Name an instance in the spec's `archetypes` block:

```json
{"archetypes": {"dwarf": {"kind": "dwarf", "params": {"heads": 4.3, "build": 0.9, "hold": "right", "beard": "braided"}}}}
```

- The instance expands, every time the spec compiles, into a skeleton named after it (joints such as `pelvis`, `head`, `shoulder_l`, `wrist_r`, `grip_r`, `knee_l`) and ordinary parts anchored on those joints. Every `spec` answer lists them: `archetype dwarf (dwarf): 24 joints; parts torso, arm_l, hand_l, ...`.
- Patch a generated part like your own: `{"parts": {"head": {"params": {"brow": 1.0}}}}` merges onto it; a different shape key (`"sdf": [...]`) replaces its shape. `"omit": ["hair"]` leaves parts out, `"prefix": "elf_"` renames them (two instances cannot make the same part name).
- Move joints by number: `"joints": {"elbow_r": {"by": [0, 0.05, 0]}}` moves the elbow and everything below it; `at` sets a position. Parts follow.
- Place an instance with `transform` (`position`, `rotate`) or on another instance's joint with `"at": "dwarf.grip_r"`, which puts its root joint there in that joint's frame.
- Parts of your own anchor on joints: `{"on": "dwarf", "landmark": "head_top", "offset": [0, -0.05, 0]}` for a helmet.
- The `features` tool lists archetypes and describes each one's parameters, joints and parts.

Humanoids: `human` (7.5 heads), `elf` (8 heads, long legs, pointed ears), `dwarf` (4.3 heads, barrel chest, heavy boots, big hands, braided beard), `orc` (massive, tusks), `halfling` (5 heads, big bare feet). Parameters: `height`, `heads`, `build`, `belly`, `shoulders` (head heights), `legs` (crotch height share), `arms`, `hands`, `feet`, `stance`, `pose` (`relaxed`, `a_pose`, `t_pose`), `hold` (`none`, `right`, `left`, `both`: the forearm points forward and the fist closes around joint `grip_r`), `beard` (`none`, `short`, `full`, `braided`), `hair`, `detail`. The anatomy kit blends SDF masses (ribcage, pecs, lats, traps, belly, pelvis, glutes, deltoids, biceps, triceps, forearms, quads, hamstrings, calves, boots) into one `torso` mesh; the `head_base`, `nose`, `ear`, `eye` and `hand` features and eyebrow, beard and hair strands sit on top.

Weapons: `sword`, `dagger`, `axe`, `hammer` and `spear`, each on its centerline along +Y with the edge or face toward +X: joints `pommel`, `grip`, `guard`, `blade_base`, `tip` for blades; `butt`, `grip`, `grip_upper`, `head`, `top` and `edge` (spears: `tip`) for hafted ones. Parameters: `length`, `blade` or `head` (share of the length), `grip` (grip length, or where the hand holds from the butt), `width`, `curve` (a sweep, degrees), `bits` (axes: 1 or 2) and `style` (`dwarven` broad, thick and faceted; `human`; `elven` slim, leaf shaped and curved). The root joint is `grip`, so `{"axe": {"kind": "axe", "at": "dwarf.grip_r"}}` puts the axe in a fist that `hold: "right"` closed: the haft runs along the fist and the bit faces where the knuckles point.

Wings: `feathered_wings` and `membrane_wings`, a pair of `wing` features on wing bones (`shoulder_l`, `elbow_l`, `wrist_l`, `tip_l`, for membranes `finger_1_l` and on; `_r` mirrored) around a `root` joint: `{"wings": {"kind": "feathered_wings", "at": "elf.back"}}` spreads them from a body's back. Parameters: `span`, `apart`, `fold` (0 spread, 1 folded along the back), `raise`, `tilt` (0 in flight, 70 on display with the trailing edges hanging), `sweep`, `chord`, `primaries`, `secondaries`, `coverts`, `feathers` (`solid` or `cards`), `fingers`, `scallop`.

## Sculpting by numbers

A part's `deform` block moves points by offsets after its shape and edits, with pins held and a smooth falloff along the surface (it never reaches geometry that is not connected):

```json
"ear_l": {"sdf": [...], "deform": {"handles": [{"move": "tip", "by": [0, 0.01, -0.005]}], "pins": ["bottom"], "radius": "5cm"}}
```

Points are names (a landmark of the part, one of its SDF shapes, or a face group such as `ear_l.lobe` in the head a blended ear joined) or part-space coordinates. The `deform` tool does the same in one call: on a spec part it adds the handles to the part's `deform` block, so recompiles keep them; on any other mesh it runs `deform.handles` with world coordinates.

## Constraints

`"constraints": [...]` relate parts after their anchors resolve; the compiler moves (or for `proportion`, scales) parts until every one holds, in the order written. Landmark references are `part.landmark`.

| Constraint | Effect |
|---|---|
| `{"align": {"a": "ear_l.top", "b": "head.brow_l", "axis": "y", "offset": "1cm"}}` | moves `a`'s part so the coordinates match (`b` may be a number) |
| `{"rest_on": {"part": "chest", "on": "table"}}` | drops the part onto another's top surface (`on` is `ground`, y = 0, by default) |
| `{"symmetric": {"a": "lamp_l", "b": "lamp_r", "axis": "x"}}` | moves `b` to the mirror position of `a` |
| `{"distribute": {"parts": ["rivet_1", "rivet_2", "rivet_3"], "from": "plate.left", "to": [0.2, 0.1, 0], "spacing": "varied"}}` | lines origins up from `from` to `to`; `varied` gives a rhythm instead of even steps |
| `{"attach": {"part": "knob", "to": "door", "gap": "0mm"}}` | moves the part until the gap is exact (touching) |
| `{"clearance": {"a": "door", "b": "frame", "min": "5mm"}}` | pushes `a` away until they are at least `min` apart |
| `{"inside": {"part": "gem", "in": "socket"}}` | shifts the part into the other's bounds |
| `{"offset": {"part": "plume", "from": "helmet.crown", "by": [0, 0.02, 0]}}` | puts the part's origin at a landmark plus an offset |
| `{"proportion": {"a": "blade.length", "b": "haft.length", "ratio": 0.4}}` | scales `a`'s part (`.height`, `.width`, `.depth`, `.length`) |

Constrain a mirror's source, not the mirror. Constraints that pull against each other are reported as warnings with how far each is off.

## Briefs

`brief.set {"name": "elf", "brief": {...}}` (or the `brief` tool) records the request: `title`, `category`, `style` (`realistic`, `stylized`), `size` (`{"height": "0.26m"}` means within 10%, or `[min, max]`), `budget` (`{"tris": [8000, 14000]}`), `must_have` (`{"feature": "ear", "count": 2, "traits": {"length_ratio_to_head": [0.35, 0.6]}}`; features are parts named after them or tagged with them), `must_not` (`"hair_penetrating_head"`). It expands into checks, with the category's defaults (a `humanoid_head` is symmetric and closed, its width, depth and eye line within the style's range) and "parts attached to what they sit on". A spec answers the brief with `"brief": "elf"`; until then every item is pending. Traits that need later phases (pointiness, UV seams) stay pending instead of failing.

Anchored parts sunk into their parent on purpose (eyes in sockets) get `max_penetration` metadata, so diagnostics do not flag them.

## Visual DNA

A spec (or its brief) can say how the asset should feel with a `dna` block (ADR 0011). Every gene is written as a word or a few numbers and measured back from the geometry, so you can design what you cannot see:

```json
"dna": {"culture": "dwarven", "style": "stylized_painterly", "hierarchy": [0.65, 0.25, 0.1],
        "lines": "angular", "edges": "varied", "rhythm": "regular",
        "proportions": {"head_to_haft": [0.3, 0.5]}}
```

- `culture`: `dwarven` (angular lines, heavy primary form, varied chunky bevels), `elven` (flowing curves, strong taper, lighter masses, varied rhythm), `orcish` (heavy, crude, massive primary form), `human_medieval` (balanced). Presets set the defaults below; written genes override them.
- `style`: `realistic` or `stylized_painterly` (a stronger outline: front and side silhouette solidity 0.45 to 0.88).
- `hierarchy`: primary, secondary and tertiary volume shares (forms are parts, or a single part's shape groups).
- `lines`: `angular`, `flowing`, `heavy`, `balanced`, or `{"curved": [min, max], "taper": [min, max]}` (share of surface that bends smoothly; taper of elongated parts).
- `edges`: `varied`, `uniform`, `sharp`, `soft`, or `{"bevel_variation": [min, max]}` (coefficient of variation of bevel widths).
- `silhouette`: `{"solidity": [min, max]}`; `rhythm`: `varied` or `regular`; `proportions`: height ratios `<a>_to_<b>`, and for humanoids `heads` (height in head heights), `shoulders` (width in head heights) and `legs` (crotch height over height), read from the front silhouette of the parts tagged `anatomy`.
- `archetype`: an archetype's genes (its proportion ranges, which parts may bend, a body's curved surface); archetype instances set it, so a dwarf is measured against dwarf ranges.
- `gesture`: `{"parts": ["blade"], "bend": [0.02, 0.2]}`, how far elongated parts' centerlines bend (largest distance from the chord over the length); flowing (elven) lines expect a bend.
- `rest`: `[min, max]`, the share of the length free of small forms (rest areas).
- `surface` and `variation` are kept for Phase 6.

`perceive dna` shows each gene measured against its target, then the critique: `twinning` (equal forms side by side, or like forms at even spacing when the rhythm should vary), `tangent` (forms that just touch), `uniform_bevel` (one bevel width on forms of very different sizes), `weak_silhouette` (an outline that reads as a blob), `proportion` (a measure outside the archetype's range), `stiff` (a straight centerline where gesture is expected), `floating_detail` (a small form touching nothing) and `uniform_detail` (small forms everywhere, no rest area). On a spec, every critique comes with a spec patch, and the first four with a predicted result from a dry run:

```
proportion heads: {"merge":{"archetypes":{"dwarf":{"params":{"heads":4.53}}}}} -> resolved; dna 3/6 -> 5/6
stiff blade: {"merge":{"archetypes":{"s":{"params":{"curve":12}}}}} -> resolved; dna 4/6 -> 5/6
```

Run the patch with `spec patch` to apply it. Generated parts are fixed through their archetype's parameters when it has one (`heads`, `shoulders`, `legs`, `curve`), other parts through their transform, anchor offset or a deform handle. The critique reads art direction; `diagnose` keeps the geometry and intent findings, and they do not overlap: detail below pixel size is the `DETAIL_SCALE` diagnostic, a category's proportions of one part (a humanoid head's width to height, its eye line) are the `PROPORTIONS` diagnostic and broken mirror pairs are `ASYMMETRY`, while the critique's `proportion` covers archetype and written DNA ranges. `floating_detail` is the art-direction reading of `FLOATING_PART` for small forms, with the same seat-on-the-surface fix as a spec patch. Every `spec` answer for a spec with DNA ends with `dna 5/6 genes on target; critique: ...`.

## Vegetation

A tree is a `feature` part: `"oak": {"feature": "tree", "params": {"species": "broadleaf", "height": "12m", "crown": "dome", "seed": 3}}`.
- Parameters left out come from the species preset, as in `gen.tree`.
- Crown outlines need `gen.tree`; specs use the shape words.
- The part is tagged `vegetation`: it renders two-sided, and diagnostics accept its open cards.
- Flowers and grass are `"feature": "flower"` and `"feature": "grass_cards"` parts.
- Anchor other parts on `lm.fork` or `lm.crown_top`. Put several trees on terrain with `gen.scatter`.

The `dna` block takes a vegetation archetype: `"dna": {"vegetation": "broadleaf"}` (or `conifer`, `palm`, `birch`, `stylized`), or ranges such as `{"vegetation": {"preset": "broadleaf", "crown_ratio": [1.0, 1.2], "coverage": [0.85, 0.95]}}`. The genes are:
- `crown_ratio`: crown width over height;
- `crown_base` and `fork`: height shares;
- `trunk`: diameter at breast height over height;
- `limbs`;
- `coverage`;
- `root_spread`.

`perceive dna` measures them back.

## Provenance

Every face records the spec node that made it: a blended ear's faces in the head mesh trace to `/parts/ear_l/sdf/0`, an inset's faces to `/parts/body/edits/0`. Face groups name them too: `cranium`, `jaw`, `ear_l.lobe`, `nose.shape_0`, `panel.inner`.

## Example: a dwarven battle axe

`docs/agents/examples/battle_axe.json` builds a double-bit axe in one `spec set`: a `silhouettes` head (crescent front outline with rune holes, a lens top outline so the blades thin to an edge), a bevelled socket block, an octagonal `lathe` haft, a top spike, collar bands of different sizes, a cord wrap (`pipe` along a helix) and a faceted pommel, all anchored on the haft, with `dwarven` DNA. Its answer already says what to do next: the brief's triangle budget fails and the head is denser than its screen size needs.

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

The same head from feature templates and strands (`docs/agents/examples/feature_head.json`, a brief and one spec call) needs no coordinates at all: `head_base` brings proportions and the `humanoid_head` landmarks, `nose` blends in at `nose_root`, the `elf` ear and the eye sit at `ear_root_l` and in the `eye_l` socket, `hair` grows inside the loop `hairline_front`, `temple_l`, `nape`, `temple_r` and drapes back over the scalp, `brow_l` grows on the `brow_ridge_l` faces, and the mirrors make the right side. The brief answers 8 of 8. Change the character with parameters: `{"parts": {"head": {"params": {"brow": 0.9, "jaw": 1.2}}}}` makes it dwarven.

## Example: a dwarf warrior from archetypes

`docs/agents/examples/dwarf_warrior.json` builds a dwarf warrior in five calls: `features {"archetype": "dwarf"}`, a brief, one `spec set`, `perceive dna` and the one fix it proposes. The spec has two archetype instances, `dwarf` (`hold: "right"`, a braided beard with beads that collides with the mail) and `axe` placed `at: "dwarf.grip_r"`, plus gear of its own on the dwarf's joints: a mail shirt with a rope hem, a belt with a buckle and pyramid studs, a helmet with a nasal guard and a rune band, pauldrons (`pauldron_r` mirrors `pauldron_l`), studded bracers on the wrist joints and a rune band on the axe haft. The dwarf measures 4.4 heads with the shoulders and legs of the dwarf ranges; the critique finds the left bracer just touching the belt studs and its predicted fix resolves it. `feathered_wings.json` puts `feathered_wings` on an elf's `back` joint and an elven sword in its fist.
