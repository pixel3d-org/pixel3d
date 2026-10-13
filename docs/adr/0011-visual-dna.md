# ADR 0011: Visual DNA, a form language agents write, compile and read back

- Status: accepted
- Date: 2026-10-13

## Context

The Model Spec (ADR 0010) says which parts exist and where they sit. It does not say how the asset should feel, and nothing measures whether it does. That missing layer is why agent-made models look like toys even when the parts are right. Written from the agent's side, the failures are:

1. **No aesthetic signal.** An agent can check that an axe has a head and a haft, not that the blade reads as heavy, the curves have tension, or the detail has rest areas. Screenshots do not fix this: they are lossy, slow, and an agent judges 3D form from a 2D image worse than an artist does.
2. **Wrong vocabulary.** Artists think in gesture, silhouette, primary, secondary and tertiary forms, shape language (dwarven blocks, elven S-curves), rhythm with variation, edge wear that tells a story. Tools offer vertices and operators.
3. **Detail is hand sculpted.** AAA quality comes from layers: big forms, then mid forms, then wear, chips, pores, grain, runes, stitches, mostly sculpted in ZBrush and baked. An agent cannot sculpt by hand.
4. **Knowledge does not reach geometry.** An agent knows that a dwarf is about 4.3 heads tall with a barrel chest and big hands, or that a broadleaf crown is a clumped hull. Nothing turns that text knowledge into the right masses at the right landmarks.
5. **Every category is new work.** Without a shared structure, a tree, wings, a beard and a creature each need their own way of thinking.

What agents are good at: text, numbers, named concepts, relations, SVG paths, JSON trees, rules, and the compile, measure, fix loop.

## Decision

Every Model Spec may carry a `dna` block: a small set of named genes in five layers. Each gene is:

- **written** as a word or a few numbers (`"lines": "flowing"`, `"taper": 0.6`),
- **compiled** into concrete shape decisions by generators (with defaults from the archetype and the style),
- **measured back** from the compiled geometry by `perceive dna`, in the same units,
- **checked** by art-direction diagnostics with spec-patch fixes and predicted results.

One gene, three directions (write, compile, measure) is what makes it native to an agent. It works like a type system for appearance: the agent states intent, the compiler realizes it, and the checker reports the difference in numbers.

### Layers

1. **Skeleton.** The structure everything hangs on: a graph of named joints and curves. Examples: creature bones, a tree's branch hierarchy, wing bones and feather tracts, a weapon's centerline (haft, blade spine), a building's grid. Archetypes generate it (a dwarf preset is 4.3 heads tall); the agent edits joints as landmarks. Parts anchor to it, and rigging (Phase 10) reuses it.
2. **Form language.**
   - `proportions`: ratios between named measures (head to height, blade to haft, crown to trunk).
   - `hierarchy`: primary, secondary and tertiary forms (default volume shares about 60 / 30 / 10), detail clustered with rest areas (`busy`, `quiet` zones).
   - `lines`: the curve vocabulary: share of straights and curves, C and S curves, taper, gesture strength (`angular`, `flowing`, `heavy`).
   - `silhouette`: what each view should read as at the asset class view distance, with negative space and distinct extremes.
   - `rhythm`: repeated elements spaced with variation (never equal), with an asymmetry amount.
   - `edges`: bevel widths that vary with form size, and the hard or soft transitions.
3. **Surface.**
   - Material zones.
   - Wear rules: exposed edges and tips worn, crevices dirty, driven by curvature, AO and thickness masks.
   - Damage: chips, cracks and nicks, by density and size.
   - Micro detail, with direction along the form: wood grain along the haft, pores, scales.
   - Ornament: runes, filigree, knotwork, stitches and tattoos as SVG patterns (agents write SVG fluently), placed along paths or at surface coordinates and baked as decals.
4. **Culture and style.** Named bundles of genes:
   - `dwarven`: blocky, angular, heavy bases, geometric runes, bronze, gold and iron, symmetric, chunky bevels.
   - `elven`: tapering, S-curves, leaf and vine motifs, silver and white, slight asymmetry.
   - Also `orcish`, `human_medieval`, `stylized_painterly` (proportions exaggerated 1.2 to 1.4, broad bevels, painted gradients) and `realistic`.

   A preset sets defaults; the spec overrides single genes.
5. **Variation.** The seed, asymmetry, and noise amplitude per scale band. `variants: n` gives a family of siblings from one DNA.

### Measurement and critique

`perceive dna` reports each gene as measured value, target and verdict. For example:

- `lines: 72% curved (elven 60 to 85) ok`
- `taper: 0.31 at blade tip (target >= 0.4) low`
- `hierarchy: secondary forms are 45% of primary volume (target 20 to 35): they compete`
- `edges: 96% of bevels are 4 mm (uniform reads as CG)`

Art-direction diagnostics use the codes of concept-art critique, each with a fix as a spec patch and its predicted result:

- `tangent`: forms or lines that just touch.
- `twinning`: equal sizes or spacing side by side.
- `uniform_detail`: no rest areas.
- `weak_silhouette`.
- `uniform_bevel`.
- `proportion`: outside the archetype range.
- `stiff`: a straight centerline where the archetype expects gesture.
- `detail_scale`: detail below pixel size at view distance; turn it into a normal map.
- `floating_detail`.

### Archetypes

Each category is a genome template: a skeleton, parts, default genes, expectations and critique thresholds. A new category is data plus generators, not a new code path. The library grows through:

- **Weapons:** axe, sword, hammer, bow, staff.
- **Characters:** humanoids by race (human, dwarf, elf, orc), with anatomy kits of SDF muscle and fat masses anchored to bones.
- **Creatures:** quadruped, dragon, insectoid.
- **Wings:** feathered (primaries, secondaries, coverts as layered cards), membrane (shell with vein decals), insect.
- **Vegetation:** ADR 0009.
- **Props and architecture.**

Hard cases map onto shared systems:

- **Hair and beards** are strands with grooming genes (braids, beads, clump profile, flyaways).
- **Tattoos** are SVG decals wrapped by surface coordinates, with symmetry.
- **Scales and skin folds** are displacement patterns in the surface layer.

### Reference distillation

`dna extract` measures the genes of a reference mesh or outline, so an agent can say "like this, but dwarven". Later, Phase 18 adds the same for clean concept images.

## Phasing

- **3b:**
  - The `dna` block in the spec schema (validated, stored, versioned).
  - Style and culture presets as defaults for existing knobs (bevel, exaggeration, proportions).
  - `perceive dna` v0 for genes measurable on current geometry: proportions, silhouette, curve share, taper, form hierarchy by part volume, bevel uniformity.
  - The first critique codes.
- **3c:**
  - Archetype skeletons and the start of the library: weapon centerlines, humanoid races, wings, vegetation.
  - Grooming genes on strands, and ornament along paths as geometry.
  - The full critique set.
- **6:** the surface layer compiles (wear, damage, micro detail, ornament and tattoo decals) into bakes and texture synthesis.
- **7:** the benchmark scores DNA genes alongside topology and UV metrics.
- **10:** rigging fits to the DNA skeleton.
- **18:** DNA extraction from reference images.

## Consequences

- Agents get an aesthetic feedback signal in numbers, so "looks like a toy" becomes a list of measured genes with fixes.
- Presets carry art knowledge (dwarven, elven, painterly) as data that users and addons can extend.
- Every gene needs a measurement before it ships. A gene that cannot be measured is a preset knob, not a gene.
