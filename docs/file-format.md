# The `.p3d` file format

A `.p3d` file is a zip archive. Version 1 holds:

| Entry | Content |
|---|---|
| `manifest.json` | Settings, objects, meshes, history, metadata, expectations (JSON, stable key order) |
| `blobs/<blake3>.bin` | One mesh's arrays, rkyv-serialized; the name is the blake3 hex digest of the bytes |

Thumbnails and textures join in later versions.

## Reproducibility

The same document always produces the same bytes: entries are written in a fixed order (`manifest.json`, then blobs sorted by hash), every entry has the timestamp 1980-01-01 and permissions 0644, meshes are compacted before they are serialized, and JSON maps are sorted. Saving, loading and saving again gives an identical file.

## Writing

Saves are atomic: the archive is written to `.<name>.tmp-<pid>` next to the target, flushed to disk, then renamed over the target. A failed save leaves the old file untouched.

Blobs smaller than 64 KiB are stored uncompressed; larger ones use deflate at level 1 (mesh arrays compress about as well at the fastest level). The manifest is always deflated.

## `manifest.json`

```json
{
  "format": "pixel3d",
  "format_version": 1,
  "app_version": "0.1.0",
  "settings": { "asset_class": "prop_medium", "mobile": false, "texel_density": null, "target_edge_length": null, "display_units": "metric" },
  "next_id": 5,
  "meshes": [
    { "id": 0, "blob": "<blake3 hex>", "verts": 8, "faces": 6, "tris": 12 }
  ],
  "objects": [
    {
      "id": 1,
      "name": "crate",
      "kind": "mesh",
      "mesh": 0,
      "transform": { "translation": [0, 0, 0], "rotation": [0, 0, 0, 1], "scale": [1, 1, 1] },
      "tags": ["low"],
      "metadata": { "hint": "oak" },
      "visible": true,
      "locked": false,
      "history": [ { "op": "prim.box", "version": 1, "params": { "width": 0.6 }, "label": "box_1" } ]
    }
  ],
  "expectations": [
    { "name": "E1", "check": { "attached": { "part": { "name": "lid", "id": 2 }, "to": { "name": "crate", "id": 1 }, "max_gap": 0.002, "max_penetration": null } } }
  ]
}
```

- `next_id`: object ids (`o:<id>`) are never reused, so the next id is stored rather than inferred.
- `meshes`: listed in first-use order. Two objects with the same mesh id are instances (editing one edits both). Two mesh ids with the same blob are independent copies that happen to be equal; the blob is stored once.
- `objects`: parents come before their children and siblings keep their order. `parent` is omitted for top-level objects. Optional fields (`parent`, `mesh`, `tags`, `metadata`, `history`) are omitted when empty.
- `expectations`: model tests (`dims`, `symmetric`, `attached`, `not_intersecting`, `budget`, `watertight`) in canonical units (meters). Objects are referenced by the name the author wrote and the id it resolved to, so renaming an object keeps its tests. Omitted when empty; files without it load as having none.
- `rotation` is a quaternion `x, y, z, w`; units are meters.
- Unknown top-level fields are kept and written back unchanged.

## Mesh blobs

Layout version 1 is a little-endian rkyv archive with 32-bit offsets (identical bytes on every platform) of:

- topology: `he_next`, `he_vert` (two per edge: half-edge `h` and its twin `h ^ 1`), `vert_he`, `face_he`;
- `positions` (`[f32; 3]` per vertex);
- `corner_uv` (up to two layers of `[f32; 2]` per half-edge), optional `corner_normal`, `corner_color`;
- `face_material` (`u16` per face), `edge_flags` (`u8` per edge: 1 seam, 2 sharp), optional `edge_crease`;
- `groups`: name, kind (0 vertices, 1 edges, 2 faces) and sorted members.

`he_prev` and `he_face` are not stored: they follow from `he_next` and `face_he` and are rebuilt on load.

## Reading

Loading checks, in order: the zip structure, `format` and `format_version` (newer versions are refused with a clear message; older ones are migrated in `crates/io/src/p3d/migrate.rs`), each blob's hash against its name, every mesh invariant (`Mesh::validate`), and the object hierarchy (every object reachable from the top level, no repeated ids, every mesh used).

## Performance

On an M-series laptop, a 1M-triangle mesh saves in about 200 ms and loads in about 116 ms (`cargo bench -p p3d-io --bench p3d`).
