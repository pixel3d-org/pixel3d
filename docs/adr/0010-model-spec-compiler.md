# ADR 0010: Model Specs compile into one object per part

- Status: accepted
- Date: 2026-10-13

## Context

Phase 3b makes pixel3d spec-first (project spec 6.7 and 17.3): an agent writes a Model Spec (named parts, shapes, anchors at landmarks, joins, constraints) and pixel3d compiles it into meshes, incrementally, with provenance from every face back to the spec node that made it. Perception, diagnostics and expectations (Phase 2) already work on objects: `attached(ear_l, head)` names two objects, `relations` measures gaps between objects, `structure` prints the object tree. The spec does not say how parts map to objects, where the spec lives, how compiled meshes are cached, or how provenance survives the ops that build a part.

## Decision

**Storage.** The spec source is kept as the JSON the author wrote (`serde_json::Value`) on its root object, in a typed field `Object::spec: Option<Arc<SpecRecord>>`, next to the record of the last compile. Spec 6.7 sketches a `SlotMap<SpecKey, _>` in the document; a field on the object gives the same sharing (the record is behind `Arc`, objects are copied on write) and needs no key bookkeeping when objects are deleted or duplicated. Briefs are a document table by name. Both save in `manifest.json` (`objects[].spec`, `briefs`). The scene crate (L1) stores JSON only; typed parsing, validation and compilation live in `p3d-spec` (L4), so the document never depends on op parameter types.

**One object per part.** The root object is an empty named after the spec. Every part with a `separate` join (the default) becomes a mesh object directly under the root (flat, so names stay short and mirrors are siblings), placed by its object transform in root space; its mesh stays in part space. Existing perception, relations and expectations therefore work on parts unchanged. A part with a `blend` join has no object: its SDF shapes join its parent's SDF tree with a smooth union, and its faces are a group of the parent's mesh. `decal` parts compile like `separate` ones but are tagged `decal` and hidden (baked in Phase 6). `weld` follows in a later step.

**Shape kinds compile through ops.** A part's shape is one key naming a kind: `sdf` (the `sdf.mesh` shape list), `primitive` (`{"box": {...}}` runs `prim.box`), generators (`lathe`, `sweep`, `loft`, `extrude`, `pipe` run the `gen.*` op with those params), `mirror` and `instance` of another part. Then the part's `edits` (op calls) run in order. Parameters are validated by the op's own schema, errors are reported at the spec path (`/parts/ear_l/sdf/2/radius`). Compiler-owned arguments (`name`, `parent`, `position`) are refused in shapes.

**Incremental compile.** Each part has a content hash: its shape and edits (canonical JSON), the compiler version, the seed, and for a part with blended children their shapes and placements. A part whose hash matches its record keeps its mesh untouched; a changed anchor parent only moves the object. Parts compile in dependency order (anchors and mirrors resolve their sources first; cycles are an error at the part's path). A one-part patch recompiles that part, its mirrors and its blend parent.

**Provenance.** Ops carry face groups through every edit, so groups are the carrier while a part is built: the part's faces get a group per spec node (an SDF shape is `<shape name>`, an edit's created groups). After the build the compiler turns them into a table of `(spec path, op id)` entries and a per-face entry index, run-length encoded in the record (4 bytes per face in memory, a few bytes per run on disk). Every face has an entry: faces in no node group fall back to the part's shape node. The record also keeps the mesh's face count and revision, so a mesh edited outside the spec is detected and reported instead of silently losing provenance.

**Anchors.** An anchor names a parent part and a surface coordinate on it: a landmark, `param` (u, v on the part's bounding ellipsoid, projected to the surface), `ray` or `nearest`, plus an `offset`. Landmarks are generic extremes (`top`, `bottom`, `front`, `back`, `left`, `right`, `center`), the centers of named SDF shapes, category landmarks (fractions of the part's bounding ellipsoid projected onto the surface, for example `humanoid_head`) and the part's own `landmarks`. Left is +X (the character's left), up +Y, front +Z. By default the child keeps the parent's axes; `orient: "normal"` turns its +Y along the surface normal.

## Consequences

- A spec patch is an op (`spec.patch`), so it is one undoable transaction with the usual history, dry runs and fix prediction.
- Users who edit a part's mesh directly get a warning on the next compile; edit layers (`edits`) are the supported way to keep manual work.
- Blend joins need SDF on both sides in 3b; blending other kinds goes through a mesh distance field later.
- Moving parts to nested objects later would change object paths, so the flat layout is part of the file format from the start.
