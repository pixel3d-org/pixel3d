# ADR 0014: Vegetation kernel, tree templates and foliage data

- Status: accepted
- Date: 2026-10-13

## Context

ADR 0009 decides what vegetation is (a `tree` feature and `gen.tree` op, foliage as clumps, cards or a shell, crown normal transfer, wind data, flowers, grass, a `vegetation` aspect) and gives the broadleaf reference profile. It leaves open where the code lives, how branches grow and join, how the coverage target places cards, and how the per-vertex data is stored until the engine profiles convert it.

## Decision

**Where it lives.**
- The kernels are a new L2 crate `crates/vegetation` (rank 3, next to `strands`): crown hull, skeleton, wood, foliage, flowers.
- `features` moves to rank 4 so it can wrap both kernels. `tree`, `flower` and `grass_cards` are feature templates, so spec `feature` parts, the `features` tool and the GUI's feature form work for them unchanged.
- `grass_cards` is the strands `grass` preset grown on a ground patch, without the patch.
- Tree parameters the caller leaves out come from the species preset and style (`Params::given`). The listed defaults are the broadleaf ones.
- `gen.tree` is a typed op over the same template. It also takes a crown outline, keeps the report as metadata and tags the object.

**Crown hull.** A surface of revolution around a vertical axis through the fork:
- Profiles: `sphere`, `ellipsoid`, `dome` (widest at 36% of the crown height, flatter underneath: the broadleaf default), `cone`, `umbrella`, `column`, or an outline of points.
- Lumps: seeded noise on the radius (`crown_lumps`) make the silhouette irregular.
- Clumping: a low-frequency density field (two noise octaves at a third of the crown radius) ranks the twigs and leaves hard gaps where it is lowest (`clumping`), so leaves gather in masses with sky between them instead of an even ball.
- Normals for the normal transfer come from the smooth profile, so they stay soft.

**Skeleton.**
- Broadleaf and stylized trees are decurrent: a trunk to the fork, then explicit scaffold limbs at varied angles (0.72 to 1.22 times `branch_angle`), each reaching a little past halfway to the hull.
- Birches have a leader to 90% of the height.
- Finer branches grow by space colonization (Runions et al. 2007):
  - Attractors are seeded in the hull: dense in the outer shell, sparse in the core, and sparse in the lower middle except at the rim, so the dome stays open underneath.
  - Each attractor keeps its nearest node. Each round it checks only the nodes added that round, through a hash grid, so a round costs O(attractors x nearby new nodes), never attractors x nodes.
  - A continuing branch turns at most 12 to 30 degrees per step (with `gnarl`). A new branch leaves its parent between 45% of `branch_angle` and `branch_angle`. A light-seeking tropism bends branches up.
- Conifers grow structurally instead: whorls of drooping branches along the leader as long as the hull is wide there, smaller branches between whorls, and side shoots in each branch's plane.
- Palms have a curved trunk only; the fronds are foliage.
- Radii follow the pipe model with the exponent `ln 2 / ln(1 / taper)`, so an even split tapers by `taper` (0.7). Leaders and the decurrent trunk get explicit tapers, and scaffold limbs thicken toward the fork.
- Nodes split into chains: the heaviest child continues its parent's chain, the others start a chain one level deeper. Every branch at a decurrent fork starts a scaffold limb.

**Wood.**
- Each chain is a closed tube on rotation-minimizing frames: a cap inside its parent and a point at its tip. Only foliage faces are open.
- Joints are smooth without booleans. A branch starts inside its parent, heading along the parent, and bends out on a quadratic curve with a collar (1.3 times its radius, easing to 1), so it leaves the parent's surface at a shallow angle. Scaffold limbs start deep in the trunk's top, off its axis and nearly as wide, so together they continue the trunk.
- Buttress roots are part of the trunk's own surface: sharp cross-section lobes (5 sides per root) whose ridges reach `root_spread` times the radius at the ground. They fade over 3.6 radii of height and keep spreading below the ground.
- Rings are spaced by length (4 to 5 radii) and turn (12 degrees). Thin chains, and branches thinner than 1.2% of the trunk radius (12% for low poly), are left to the cards.
- The wood takes about 30% of the triangle budget with cards, 40% with clumps, 60% with a shell. Chains are built thickest first, while their parent is built.

**Foliage.**
- Cards:
  - Cluster cards are trimmed, folded hexagons (6 vertices, 2 quads, 4 triangles). Single-leaf cards are quads (2 triangles), placed only at the silhouette.
  - Candidates are twig nodes, ranked by a seeded weighted draw that favours the outer shell.
  - Orientation: phyllotaxis turn, spread from the twig, droop growing toward the tips, then a sun bias toward the sky.
  - Repetition fixes: a variant, flip and one of five turns are re-drawn while a neighbour within a card size has the same. Remaining clashes are reported as `repeats`.
  - Coverage: four side views of the hull silhouette are rasterized at 96 pixels. A card is kept when the share of new hull pixels it would cover passes a threshold that drops each pass (0.3, 0.15, 0.06, 0.01; a tenth of the first pass is kept anyway for depth). Placement stops at the coverage target or the budget. Coverage and overdraw (layers per drawn pixel) are reported.
  - Cards are emitted inner to outer for alpha sorting.
- Clumps are noisy icospheres (faceted or smooth), placed the same way and kept above the crown base.
- The shell is the hull turned with lumpy noise.
- Palm fronds are arching strips folded along the rachis.

**Per-vertex data** (until engine profiles convert it):
- Material slots: 0 bark, 1 foliage.
- UV0: bark is u around and v along in circumferences. Cards map into the leaf atlas: 4 columns, cluster variants in the top half, single leaves, sprays and fronds in the bottom half.
- Wind: vertex color alpha is the branch level (0 trunk, 1/3, 2/3, 1 for level 3 and foliage). UV1 holds the phase (x, 0 to 1, per branch with child offsets) and the stiffness (y, 1 rigid to 0 free, falling along each branch and card).
- Vertex color rgb is a tint:
  - per-card hue and brightness jitter;
  - the crown gradient (darker and cooler inside, warmer and lighter on the sunlit outside, darker low down);
  - a darker card base.
- Custom corner normals: cards and clumps blend their own normal toward the hull's by `normal_blend`; wood is smooth.
- Groups: `trunk`, `roots` (the flare), `level_1` to `level_3`, vertex group `twig_tips`, `foliage` (all foliage faces) and one of `foliage.cluster_cards`, `foliage.leaf_cards`, `foliage.clumps`, `foliage.shell` or `foliage.fronds`.
- Landmarks: `lm.base`, `lm.fork`, `lm.crown_top`, `lm.crown_center`, `lm.crown_base`.

**Objects.**
- Vegetation objects are tagged `vegetation`.
- Headless renders draw tagged objects two-sided, like strands, instead of showing card backs as inside-out.
- Diagnostics treat them like strands: open cards and overlapping tubes are by design.

## Consequences

- Trees are about 10k triangles at the broadleaf defaults (wood about 3k, about 1.3k to 1.6k cards). Any template parameter combination builds a valid mesh whose only open faces are foliage (property tests).
- The atlas layout and the wind encoding are part of the mesh contract. Phase 6 (generated atlases) and the engine profiles (wind in vertex colors or UV2) build on them.
- Smooth joints without booleans keep bark UVs clean, but the wood is overlapping closed tubes, not one manifold. A welded hero-tree option can come later through the boolean engine.
