# ADR 0013: Strands grow from a surface region as guide curves, output as cards, clumps or shells

- Status: accepted
- Date: 2026-10-13

## Context

Phase 3c adds strands (project spec 17.6): hair, fur, eyebrows, beards and grass that agents describe with about ten parameters instead of drawing. The spec lists the stages (region, guides, clumping, output, checks) but leaves open how a loop of landmarks becomes a surface patch, what a guide is, how strands avoid the parent, and what each output looks like.

## Decision

**Region.** One of:
- `all`: the whole parent surface (fur).
- Face groups: for example the `cranium` group of an SDF head.
- A closed loop of points: landmarks such as `hairline_front`, `temple_l`, `nape` and `temple_r`, or coordinates. Each point snaps to its nearest vertex. Consecutive vertices are joined by shortest edge paths (Dijkstra, stopped at the target), and those edges cut the surface. Faces are flood-filled across uncut edges. The region is the component holding `inside` if given, otherwise the smallest component. A loop splits a closed surface into two caps, and the patch a loop is meant to bound (scalp, beard, brow) is the smaller one. Cost: O(k E log V) for k loop points plus O(F).

**Guides are clumps.** Roots are seeded Poisson-disk points on the region (`p3d_geo` scatter), spaced 0.7 sqrt(area / clumps) apart.
- Each guide is integrated as a polyline of `segments` steps. The direction comes from the style:
  - `swept_back`, `parted` (center or an x offset), `spiky`, `straight`, `combed` (a flow direction, mirrored outward on both sides), `upright`, `wind_blown`;
  - `ponytail`: toward an anchor, then hanging.
- The direction is blended toward gravity along the length, with seeded noise and a helical curl.
- After each step the point is pushed out of the parent to a growing offset (`lift` at the root, plus `volume` toward the tip), using the closest point on a triangle BVH. Strands never penetrate. Cost: O(guides x segments x log F).
- Draping styles (swept back, parted, straight, combed, ponytail) also hug: where the surface does not face down, a point above its offset is pulled back onto it, so hair rests on the scalp and sides. Once a strand leaves the surface it falls (gravity at least 0.5). Spiky, upright and wind-blown styles stand free.
- Child strands within a clump (for groom export) come later. v1 output needs clump curves and widths only.

**Outputs.**
- `cards`: tapered strips along each guide, lying flat across the surface (width direction = tangent x outward normal) and facing out.
  - Up to 3 layers, the outer ones lifted and jittered to cover gaps. An optional scalp cap (the region offset slightly) goes first, and flyaways (thin, noisier cards) last, so faces are ordered inner to outer for alpha sorting.
  - UVs: u across the card into one of 4 atlas columns (chosen per card), v from root 0 to tip 1. The atlas texture follows in Phase 6.
- `clumps`: each guide becomes a closed tapered tube with a lens (flattened) section and a pointed tip. Stylized hair, beards and horns.
- `shells`: the region copied N times, offset along vertex normals up to the length. Short fur; the noise alpha follows in Phase 6.

**Presets.** `hair`, `fur`, `eyebrows`, `beard` and `grass` set the defaults (style, output, length, clumps, gravity, lift). Every parameter can be overridden.

**Report.** Each result reports guides, faces and triangles, plus:
- coverage: the share of region area within a card's reach (catches bald spots), and where the largest uncovered spot is;
- overdraw: card area over region area;
- penetrating points: 0 by construction, checked anyway.

**Where it lives.** The kernel is `crates/strands` (L2). The `gen.strands` op grows strands on an object: loop names are vertex groups such as `lm.tip`. In a Model Spec, a part `{"strands": "hair", "params": {"on": "head", "region": [...]}}` resolves loop names as landmarks of `on` and is placed with `on`.

## Consequences

- Cards are open meshes by design. Strand objects are tagged `strands`, and diagnostics skip OPEN_BOUNDARY on them.
- Strands depend on the parent's mesh, so a parent change rebuilds them (the content hash includes the parent's).
- Clump curves are the data a groom exporter needs later (Alembic or engine groom assets).
