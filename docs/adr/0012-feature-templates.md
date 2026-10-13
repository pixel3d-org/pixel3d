# ADR 0012: Feature templates output SDF items or designed meshes, landmarks as vertex groups

- Status: accepted
- Date: 2026-10-13

## Context

Phase 3c adds the feature library (project spec 17.5): parametric templates for details agents cannot draw, such as ears, noses, eyes, hands, horns, bolts, vents, panels and moldings. They take semantic parameters, name their landmarks, and keep clean topology for every parameter combination. The spec does not say what a template produces, how its landmarks reach anchors and constraints, or how parameters are described for agents.

## Decision

**Two kinds of output.**
- Organic features (`nose`, `hand`, `head_base`) produce SDF items. A spec part that is such a feature behaves like an `sdf` part: it can `blend` into an SDF parent with a smooth fillet and be quad remeshed with loops. Outside specs, `gen.feature` meshes the items with surface nets.
- Designed features (`ear`, `eye`, `horn`, `bolt`, `rivet`, `vent_grille`, `panel_inset`, `molding`) produce meshes built by our generators (shell, lathe, sweep, extrude, tube rings). Their edge flow comes from the construction: rings, loops and rims. They are deterministic and closed unless the feature is a card.

**Landmarks are one-vertex groups named `lm.<name>`.** Ops, mirrors, transforms and booleans already carry vertex groups, so a feature's `tip` or `root` survives every edit. Anchors, constraints and deform handles look them up by name, as they do with generic and category landmarks. SDF features give landmark points instead, and the compiler resolves them on the field.

**Parameters are described, not just typed.**
- Each parameter has a kind (length, angle, ratio, count or choice), a default, a valid range and, where it differs, a stylized range.
- Values are clamped into the valid range only with a warning. An out-of-range request is an error at its path, so agents learn the limits.
- `features describe` prints the parameters, landmarks and typical budget; `features search` matches ids, labels and aliases.

**Templates live in `crates/features`** (L2) as code plus data tables. An addon registers more through the same trait later, like ops.

## Consequences

- Clean topology across parameter ranges is property-tested per feature: random parameters inside the ranges give valid, closed (or card) meshes with all landmarks present.
- A blended SDF feature trades designed loops for a seamless join; its flow comes from `remesh.quad` with guides.
