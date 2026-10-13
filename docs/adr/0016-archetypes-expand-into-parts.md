# ADR 0016: Archetypes expand into editable parts on named skeletons

- Status: accepted
- Date: 2026-10-13

## Context

ADR 0011 makes every category a genome template: a skeleton, parts, default genes, expectations and critique thresholds, so that "a dwarf about 4.3 heads tall with a barrel chest and a braided beard" becomes the right masses at the right landmarks from a few numbers. Phase 3c starts the library (weapon centerlines, humanoid races with anatomy kits, wings). ADR 0010 says how parts compile, but not how an agent asks for an archetype, where its skeleton lives, how parts anchor to joints inside a body (not on a surface), or how an agent edits one part of a generated body without losing the link to the archetype.

Options considered:

1. **An op that writes parts into the spec.** Simple, but the link is lost: changing `heads` or moving a joint afterwards does not re-derive the parts, and the spec becomes hundreds of lines the agent did not write.
2. **A part kind** (`{"archetype": "dwarf"}` as one part). One part would compile into many objects, which breaks the one-object-per-part rule of ADR 0010 and every tool built on it.
3. **A spec-level block expanded at parse time** into parts and a skeleton, with the spec's own parts layered on top.

## Decision

Option 3.

**The `archetypes` block.** A spec may carry named archetype instances:

```json
"archetypes": {
  "dwarf": {"kind": "dwarf", "params": {"heads": 4.3, "hold": "right"}},
  "axe": {"kind": "axe", "at": "dwarf.grip_r", "params": {"length": "0.9m"}}
}
```

Each instance has a `kind` (an archetype id), `params` (described like feature parameters: kind, default, range, errors at their path), optional `joints` edits (`{"elbow_r": {"by": [0, 0.05, 0]}}` moves a joint and the joints below it; `at` sets it), an optional placement (`at`: another instance's joint, or `transform`), a `prefix` for its part names and `omit` (generated parts to leave out).

**Expansion.** Parsing a spec expands every instance, in dependency order of `at`, into:

- a **skeleton** named after the instance: joints with a frame (`at`, `axis` = the joint's +Y, `side` = its +X, `parent`) and curves (chains of joints such as `spine` or `arm_l`);
- **parts**, written as ordinary spec parts (SDF anatomy, feature parts, strands, generators) anchored on joints or on each other;
- **default genes** merged under the spec's `dna` (the archetype's culture and proportion ranges); the spec's own genes win.

The expansion is pure data: O(parts), no geometry, deterministic. The source keeps only the `archetypes` block and the agent's own parts, so the spec stays short and a changed parameter re-derives every generated part; unchanged parts keep their cached meshes (ADR 0010 content hashes).

**Agents patch generated parts as usual.** A part in the spec's own `parts` with the name of a generated part is merged onto it (objects merge, values replace). Giving it a different shape key (`sdf`, `feature`, `lathe`, ...) replaces the generated shape. `omit` removes generated parts. So `{"parts": {"head": {"params": {"brow": 1.0}}}}` deepens one dwarf's brow without touching the archetype.

**Skeletons are landmarks.** A spec may also write `skeletons` by hand. A skeleton's name works like a part name in anchors and constraints, and its joints are its landmarks: `{"anchor": {"on": "dwarf", "landmark": "wrist_l", "orient": "normal"}}` puts a part's origin at the joint and, with `orient: "normal"`, turns the part into the joint's frame (+Y along `axis`, +X along `side`). Skeleton names and part names share one namespace. Rigging (Phase 10) will fit to these joints.

**`into`.** A part placed on a joint can still `blend` or `weld` into another part: `"into": "torso"` names the part it merges into (by default the part its anchor is on). Limbs anchored at shoulder and hip joints blend into the torso this way.

**Instances place instances.** `at: "dwarf.grip_r"` puts the instance's root joint (a weapon's `grip`, wings' `root`) on that joint with its frame, so an axe sits in the right fist with its haft along the fist's channel and its blade facing where the knuckles point. Cycles are an error at `/archetypes/<name>/at`.

**Where it lives.** The library (`crates/spec/src/archetype/`) is code plus data tables, one generator per family: humanoids (`human`, `elf`, `dwarf`, `orc`, `halfling`), weapons (`sword`, `dagger`, `axe`, `hammer`, `spear`) and wings (`feathered_wings`, `membrane_wings`). Vegetation archetypes follow ADR 0009. Proportion ranges and gesture expectations per archetype live with the other genes in `crates/perceive/src/dna.rs`, so `perceive dna` can measure any mesh against an archetype, spec or not. The `features` tool lists and describes archetypes with their parameters, joints and parts.

## Consequences

- A body blockout is one `archetypes` entry; the agent then works part by part with ordinary patches, and every generated part keeps provenance (`/parts/arm_l/sdf/2`) like a written one.
- Archetype changes are compile-time data changes, so new races and weapons are table entries plus, at most, a generator.
- The expanded spec can differ from the source: `spec get` shows the source, and every `spec` answer lists each instance with its joints and generated parts.
- Two instances that generate the same part name are an error until one gets a `prefix`.
