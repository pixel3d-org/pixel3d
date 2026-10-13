# ADR 0009: Agent-native vegetation

- Status: accepted
- Date: 2026-10-13

## Context

Trees, bushes, grass and flowers are among the most common game assets and among the hardest for agents. With plain modelling ops an agent can build a stylized low-poly tree: a swept, tapered trunk with root flares and branches unioned into one mesh, and a crown of faceted noisy clumps unioned into one shell. That took one `run` call of 30 ops and about 2.5k triangles. Realistic and painterly foliage is a different technique. Game trees render leaves as alpha-textured cards (single leaves or leaf clusters), with three properties:

- normals bent from the crown's overall volume, so a few hundred cards shade like a soft mass;
- wind data in vertex colors;
- LOD chains that end in camera-facing billboards.

An agent cannot place hundreds of cards one at a time, cannot judge their look from text, and has no textures to put on them. The spec already plans the parts (`leaf` as a `shell` feature, `branch` as strands, `grass_cards`, strands output as cards, clumps and shells, generated hair atlases in Phase 6) but nothing that ties them into vegetation.

## Decision

Vegetation is a parametric system that agents drive with a handful of meaningful numbers and check through text perception, like hair (17.6):

- **`tree` feature and `gen.tree` op (Phase 3c):**
  - Inputs: a species preset (`broadleaf`, `conifer`, `palm`, `birch`, `stylized`), height, trunk radius and lean, branch levels (1 to 3), branching angle and spacing, root flares, and a crown hull (sphere, ellipsoid, cone, umbrella, column, or a silhouette) that the branches grow toward (space colonization, seeded).
  - Wood: swept tapered branches with smooth unions at the joints (booleans now, SDF blends once Phase 3b lands).
  - Groups for trunk, levels, roots and twig tips.
- **Foliage modes from the same tree:**
  - `clumps`: faceted or smooth blobs for stylized trees.
  - `cards`: leaf or cluster quads along the twigs, sized and oriented by params, with UVs into a leaf atlas, sorted inner to outer.
  - `shell`: a single crown skin.

  Density, card size and coverage are parameters, never hand placement.
- **Crown normal transfer:** card normals come from the crown hull (or a smoothed proxy of the clumps), blendable with the card's own normal. This is what makes low-poly foliage read as volume.
- **Wind data:** per-vertex branch level, phase and stiffness, written in each engine profile's convention (vertex colors or UV2).
- **Grass and flowers:** `grass_cards` through strands, a `flower` feature (petal shells around a center, a stem strand, leaves), and placement on surfaces with `gen.scatter`.
- **Textures (Phase 6):** generated leaf and cluster atlases from parametric leaf outlines (alpha, normal, albedo variation), bark and grass textures, and leaf normal and thickness bakes.
- **LODs (Phase 5):** card thinning and merging per LOD, and a final billboard or octahedral impostor.
- **Perception and checks:** a `vegetation` aspect reports crown silhouette and coverage, card count, estimated overdraw, gaps and trunk visibility. Expectations cover crown width and height, density and an overdraw budget.

## Styles

Cards are the core technique for both realistic and stylized trees. Solid clumps are only the cheapest style. A style preset is a combination of card layout, atlas generator and shading parameters, never a different mesh technique. There are three reference targets:

- **Realistic (AAA, about 10k triangles per tree at LOD0):**
  - Branch-cluster cards (several leaves per card) along twigs, plus a few single-leaf cards at the silhouette.
  - Photoreal-style generated atlases with several cluster variants.
  - Normals mostly from the cards, softened toward the crown.
  - Translucency and thickness maps.
- **Stylized painted (in the spirit of League of Legends):**
  - Fewer, larger cluster cards with hand-painted-looking atlases: flat color shapes, rim and gradient baked in.
  - Strong crown normal transfer, so lighting reads as one soft mass.
  - Bold silhouettes from the crown hull.
- **Solid low poly:** faceted clumps or a crown shell with no alpha. Simple trees can sit on a few spheres or cones over the trunk.

What makes cards look good becomes named parameters the agent sets, instead of hand work:

- **Leaf angles:** phyllotaxis (137.5 degrees by default), spread around the twig, gravity droop growing toward the tips, sun-facing bias.
- **Repetition fixes:**
  - each card picks a random atlas variant, flip and rotation, seeded;
  - a hue and brightness jitter per card in vertex color;
  - a crown-scale gradient (darker and cooler inside, AO baked into vertex color);
  - a size falloff from the crown center to the silhouette.
- **Density and coverage:** a coverage target for the crown silhouette (for example 85%) with gaps allowed where the hull says so, instead of a card count.
- **Overdraw:** cards are clipped to their atlas alpha (trimmed card shapes, not plain quads), and an overdraw estimate is reported and budgeted.

Each style gets a reference session and checks in its phase:

- triangle budget per LOD;
- silhouette coverage;
- card count;
- overdraw estimate;
- that no two neighbouring cards use the same atlas variant with the same rotation.

## Consequences

The Phase 3c checklist gains the tree generator, foliage modes, wind data and flowers. Phase 5 gains vegetation LODs and impostors, and Phase 6 gains the generated foliage atlases and the normal transfer bake. Until then, stylized clump trees can be built with existing ops, as `docs/agents/examples/stylized_tree.json` shows.
