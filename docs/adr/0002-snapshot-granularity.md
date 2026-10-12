# ADR 0002: Snapshot granularity and per-transaction costs

- Status: accepted
- Date: 2026-10-12

## Context

Undo snapshots are whole documents shared copy-on-write. The document keeps its large tables (objects, name indexes, ids, meshes, history graphs) behind `Arc`, and every object and mesh behind its own `Arc`. A complexity audit of Phase 1 listed the costs that remain proportional to the scene rather than to the touched data:

1. The first write to a table after a snapshot clones that table: O(n) pointer copies for the object table, O(n) entries (with leaf-name strings) for the name indexes.
2. Appending to an object's history clones that object's history when a snapshot shares it: O(history length) per op on that object.
3. `Document::unique_name` probes `base`, `base_1`, `base_2`, ...: O(k) per call when k names with that base already exist under the parent.

None of these is quadratic within one operation. Each is linear per transaction or per call, which only adds up over long sequences (many transactions on huge scenes, many ops on one object, thousands of unnamed objects with one base name).

## Decision

Keep the current design for Phase 1 and measure it:

- The tiny-op transaction cost is benchmarked (`cargo bench -p p3d-engine --bench session`); the budget is 1 ms (project spec 21.8).
- Typical assets have tens to a few thousand objects, where a table copy is microseconds.
- Agents name the objects they create; `unique_name` probing only serves unnamed creation.

Move to persistent data structures when a measurement exceeds a budget:

- Object table and name indexes: a persistent hash array mapped trie or B-tree (O(log n) copy on write).
- History: chunked `Arc` segments or a persistent vector, so an append after a snapshot copies O(1) or O(log n).
- `unique_name`: a content-derived index of numeric suffixes per (parent, base), so the result stays deterministic across save and load.

## Consequences

- Complexity doc comments state these terms explicitly (`create_object`, `push_history`, `run_batch`).
- A scene of 100k objects pays roughly a millisecond per transaction for table copies; this is acceptable now and tracked against the 1 ms tiny-op budget.
