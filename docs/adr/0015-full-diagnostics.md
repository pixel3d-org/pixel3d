# ADR 0015: Full diagnostics: mesh checks in perceive, intent in the spec crate, spec-first fixes

- Status: accepted
- Date: 2026-10-13

## Context

Phase 3c completes `diagnose` (project spec 17.10): intent, edge flow and deformation, shading and detail scale, with causes from provenance and fixes whose results are predicted by a dry run. Phase 2 shipped geometry and relation findings in `p3d-perceive` (L3) with op fixes. The spec does not say where checks that need the Model Spec live (briefs, proportions and landmarks are in `p3d-spec`, which depends on `p3d-perceive`), how a fix on a compiled part survives the next compile, what "deforming region" or "joint" mean on a plain mesh, how loops around an eye are measured, or what thresholds apply.

## Decision

**Two layers, one ranking.** `p3d_perceive::diagnose` keeps every check that only needs meshes, objects, tags and metadata: geometry, relations, shading, detail scale, edge flow on deforming parts, strands. It exposes unranked `Draft`s and `rank`, which sorts, numbers and fills causes. `p3d_spec::diagnose` adds the checks that need the spec (expectations, brief items, proportions, mirror pairs, joints and face loops at landmarks), rewrites placement fixes of spec parts, names landmarks, and ranks everything together. The engine's predictions, the agent's `diagnose`, problems line and bisect use the spec layer.

**Finding fields.** `measured` and `threshold` are in `unit` (`mm`, `deg`, `px`, `%`, `count`, `ratio`), the message's units. `landmark` names the nearest landmark within a fifth of the part's size (for relation findings the other part's first: where it sits; a part's own `center` gives way to the landmark it is anchored at). `cause_path` is a JSON pointer into the part's spec: the provenance node of the face nearest the place, or the value behind the problem (`/parts/ear_l/anchor`, `/parts/hair/params/clumps`, `/parts/ear_r/params/length`). Plain objects keep "last changed by <op>".

**Spec-first fixes.** An op on a compiled object is undone by the next compile, so on spec parts fixes are spec patches: placement moves become the anchor offset (in the anchor's frame) or the position; mesh repairs become edit layers appended to `/parts/<name>/edits`. Mirrors and instances copy their source's shape (the compiler ignores edits on mirrors), so edit layers and shape parameters go to the source.

**Predictions.** The first fixes of errors and warnings are predicted (at most 8 runs): resolved or what remains, the measure the fix changes (strands coverage, drawn triangles), new findings. When the first option would not resolve the finding and a later one would, that one moves first, and `diagnose fix` applies options in that same order (it computes the same predicted list). Info findings and fixes running long ops (remeshes, flagged `long_running`) are left to `explain`, which predicts every option: a remesh takes seconds.

**Edge flow needs intent.** Edge flow checks run on parts tagged `deforming` (a spec part's `tags`), so hard-surface props are not judged by animation rules. Joints are landmarks whose names contain a joint word (`shoulder`, `elbow`, `wrist`, `hip`, `knee`, `ankle`, `neck`, `waist`, `knuckle`, `joint`). Loops across a joint: the limb axis is the direction the nearby surface normals avoid (smallest eigenvector of their covariance), the radius comes from the cross-section through the joint, and loops are one limb diameter over the median length of the faces along the axis there; 3 are needed. Triangles above 5%, poles of 6 or more edges in quad-dominant parts, and neighbouring faces more than 4x apart in size are flagged.

**Face loops as flow.** A walk looking for an exactly closed edge loop fails on real `remesh.quad` output, whose ring guides make the flow circle a point without closing perfect loops. `FACE_LOOPS` measures the flow instead: the length-weighted mean of `|sin 2a|` over the edges 0.4 to 1 radius from `eye_l`, `eye_r` and `mouth` (radius 15% and 18% of the head's height, the band the remesher guides), where `a` is the angle to the circle through the edge. Concentric loops and spokes measure 0, a grid or triangles about 0.64, a guided remesh with edges a tenth of the radius about 0.4; the limit is 0.5. Humanoid heads are parts with `humanoid_head` landmarks or the `head_base` feature; the finding is a warning on deforming heads and info otherwise. The fix is a `remesh.quad` edit with those loops and an edge of a tenth of the radius.

**Thresholds.**
- Smoothing: smooth edges folding more than 60 degrees, counted only when the faces along them cover at least 2.5% of the part (a few pinched voxels of a coarse SDF do not show; a smooth-shaded cube covers 100%).
- N-gons: corners more than 2% of the n-gon's size off its plane. Planar n-gons are left alone.
- Hard edges without a UV split: checked only on meshes with UVs (primitives have none until Phase 4).
- Detail scale: a part's middle extent (its cross-section from any side) under 2 px at the asset class's view distance on a 1080p, 60 degree screen. A plate stays (its width counts), a rivet or a wire under 2 px goes to a decal.
- Strands: coverage under 90% (where the compiler warns). `PENETRATING_STRANDS` samples strand faces (at most 2,048 per part) for points more than 0.5 mm inside the closed surface they grow on: the kernel's own count covers guide points only, while the cards between them cut through the scalp when the curves are coarse or unlifted.
- Asymmetry: left and right parts written out twice (`_l`/`_r`, `_left`/`_right`, `left_`/`right_`) whose root-space sizes or mirrored centers differ by more than 3%. The cause is the first value of the right part that differs from the mirrored left part (side words swapped, x offsets and y and z turns negated).

**Strands leave the relation pass.** Strands sit on their surface by design, and hair against a scalp is the most expensive pair (contact everywhere). Their relations are covered by the strands checks, so the diagnose relation pass skips them; a part alone but for its strands is not floating.

**Not done.** Silhouette mismatch to a reference waits for a reference outline format in briefs.

## Consequences

- Diagnose on the 101k-triangle elf head example takes about 0.3 s (0.6 s before, hair relations dominated); predictions add 0.2 to 0.3 s each. The 100k-triangle perception bench stays at about 106 ms. Every new pass is linear (`scale_diagnose_*` in `crates/perceive/benches/scale.rs`).
- Benchmark reference sessions declare an asset class; replays must set it, since detail scale depends on the view distance.
- The `ear` feature template is flagged (folded rim, walls near 0 mm): the template, not the checks, should change.
- `remesh.quad` at its default edge does not make loops strong enough for `FACE_LOOPS`; the fix asks for a finer edge.
