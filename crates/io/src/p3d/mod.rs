//! The `.p3d` file format: a zip with `manifest.json` and content-addressed
//! mesh blobs. See `docs/file-format.md`.
//!
//! - `manifest.json`: settings, objects (parents first), meshes, history,
//!   metadata; stable key order; unknown top-level fields are kept.
//! - `blobs/<blake3>.bin`: rkyv-serialized mesh arrays. Instances and copies
//!   of the same mesh share one blob. Blobs under 64 KiB are stored, larger
//!   ones deflated.
//!
//! Archives are reproducible: fixed timestamps and entry order, so the same
//! document always gives the same bytes.

pub mod blob;
pub mod manifest;
pub mod migrate;

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use glam::{Quat, Vec3};
use p3d_math::Trs;
use p3d_scene::{Document, MeshKey, ObjectKey, ObjectKind, OpGraph};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use manifest::{FORMAT, FORMAT_VERSION, Manifest, MeshEntry, ObjectEntry, TransformEntry};

/// Blobs at least this large are deflated; smaller ones are stored as is.
pub const DEFLATE_THRESHOLD: usize = 64 * 1024;

/// Why a file could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum P3dError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("archive: {0}")]
    Zip(String),
    #[error("not a pixel3d file: {0}")]
    Format(String),
    #[error(transparent)]
    Version(#[from] migrate::MigrateError),
    #[error("blob {hash}: {reason}")]
    Blob { hash: String, reason: String },
    #[error("object {id} (`{name}`): {reason}")]
    Object { id: u32, name: String, reason: String },
}

fn zip_err(e: zip::result::ZipError) -> P3dError {
    P3dError::Zip(e.to_string())
}

/// What a save wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// Distinct blob hashes, sorted.
    pub blobs: Vec<String>,
    pub bytes: u64,
}

/// Objects with parents first and siblings in order (depth-first).
///
/// Complexity: O(n).
fn ordered(doc: &Document) -> Vec<ObjectKey> {
    let mut out = Vec::with_capacity(doc.object_count());
    let mut stack: Vec<ObjectKey> = doc.roots().collect();
    stack.reverse();
    while let Some(k) = stack.pop() {
        out.push(k);
        if let Some(o) = doc.object(k) {
            stack.extend(o.children().iter().rev());
        }
    }
    out
}

/// A manifest, its encoded blobs by hash, and the mesh of each manifest
/// mesh id (in id order).
type Built = (Manifest, BTreeMap<String, Vec<u8>>, Vec<MeshKey>);

/// The manifest of a document, its encoded blobs by hash, and the mesh
/// of each manifest mesh id (in id order).
///
/// Complexity: O(total mesh size + objects).
fn build(doc: &Document) -> Result<Built, P3dError> {
    let mut mesh_ids: BTreeMap<MeshKey, u32> = BTreeMap::new();
    let mut meshes: Vec<MeshEntry> = Vec::new();
    let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut objects = Vec::with_capacity(doc.object_count());
    for k in ordered(doc) {
        let Some(o) = doc.object(k) else { continue };
        let mesh = match o.mesh_key() {
            None => None,
            Some(mk) => {
                if let Some(&id) = mesh_ids.get(&mk) {
                    Some(id)
                } else {
                    let m = doc.mesh(mk).ok_or_else(|| P3dError::Object { id: o.id(), name: o.name().into(), reason: "missing mesh".into() })?;
                    let bytes = blob::encode(m).map_err(|reason| P3dError::Object { id: o.id(), name: o.name().into(), reason })?;
                    let hash = blob::hash(&bytes);
                    blobs.entry(hash.clone()).or_insert(bytes);
                    let id = meshes.len() as u32;
                    meshes.push(MeshEntry {
                        id,
                        blob: hash,
                        verts: m.vert_count() as u32,
                        faces: m.face_count() as u32,
                        tris: m.triangle_count() as u32,
                    });
                    mesh_ids.insert(mk, id);
                    Some(id)
                }
            }
        };
        let t = o.transform;
        objects.push(ObjectEntry {
            id: o.id(),
            name: o.name().to_owned(),
            parent: o.parent().and_then(|p| doc.object(p)).map(|p| p.id()),
            kind: o.kind().as_str().to_owned(),
            mesh,
            transform: TransformEntry { translation: t.translation.to_array(), rotation: t.rotation.to_array(), scale: t.scale.to_array() },
            tags: o.tags.to_vec(),
            metadata: o.metadata.clone(),
            visible: o.visible,
            locked: o.locked,
            history: doc.history(k).map(|g| g.nodes.clone()).unwrap_or_default(),
        });
    }
    let manifest = Manifest {
        format: FORMAT.to_owned(),
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        settings: doc.settings.clone(),
        next_id: doc.next_id(),
        meshes,
        objects,
        expectations: doc.expectations().to_vec(),
        extras: doc.extras.clone(),
    };
    let mut keys: Vec<(u32, MeshKey)> = mesh_ids.into_iter().map(|(k, id)| (id, k)).collect();
    keys.sort_unstable();
    Ok((manifest, blobs, keys.into_iter().map(|(_, k)| k).collect()))
}

/// Serializes a document to `.p3d` bytes.
///
/// Complexity: O(total mesh size + objects).
pub fn to_bytes(doc: &Document) -> Result<(Vec<u8>, SaveReport), P3dError> {
    let (manifest, blobs, _) = build(doc)?;
    let json = serde_json::to_vec_pretty(&manifest).map_err(|e| P3dError::Format(e.to_string()))?;

    let base = SimpleFileOptions::default().last_modified_time(DateTime::DEFAULT).unix_permissions(0o644);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("manifest.json", base.compression_method(CompressionMethod::Deflated)).map_err(zip_err)?;
    zip.write_all(&json).map_err(|e| P3dError::Zip(e.to_string()))?;
    for (hash, bytes) in &blobs {
        // Level 1: mesh arrays compress about as well at the fastest level.
        let opts = if bytes.len() >= DEFLATE_THRESHOLD {
            base.compression_method(CompressionMethod::Deflated).compression_level(Some(1))
        } else {
            base.compression_method(CompressionMethod::Stored)
        };
        zip.start_file(format!("blobs/{hash}.bin"), opts).map_err(zip_err)?;
        zip.write_all(bytes).map_err(|e| P3dError::Zip(e.to_string()))?;
    }
    let out = zip.finish().map_err(zip_err)?.into_inner();
    let report = SaveReport { blobs: blobs.keys().cloned().collect(), bytes: out.len() as u64 };
    Ok((out, report))
}

/// A mesh as readable JSON: positions, faces as vertex index lists, per-face
/// materials, per-corner UVs (in face order) and groups, densely numbered.
///
/// Complexity: O(V + F + C + groups x slots).
fn mesh_json(mesh: &p3d_mesh::Mesh) -> serde_json::Value {
    let mut m = mesh.clone();
    m.compact();
    let faces: Vec<Vec<u32>> = m.faces().map(|f| m.face_verts(f).collect()).collect();
    let uvs: Vec<Vec<Vec<[f32; 2]>>> = (0..m.uv_layer_count())
        .map(|l| m.faces().map(|f| m.face_halfedges(f).map(|h| m.corner_uv(l, h).unwrap_or_default().to_array()).collect()).collect())
        .collect();
    let groups: Vec<serde_json::Value> = m
        .groups()
        .map(|(name, g)| serde_json::json!({ "name": name, "kind": g.kind().as_str(), "members": g.iter().collect::<Vec<u32>>() }))
        .collect();
    serde_json::json!({
        "positions": m.verts().map(|v| m.positions[v as usize].to_array()).collect::<Vec<_>>(),
        "faces": faces,
        "face_material": m.faces().map(|f| m.face_material(f)).collect::<Vec<_>>(),
        "corner_uv": uvs,
        "groups": groups,
    })
}

/// Everything in a document as one JSON value, for inspection: the
/// manifest (as stored in `.p3d`) with each mesh entry's data inlined as
/// `data` (see [`mesh_json`]).
///
/// Complexity: O(total mesh size + objects).
pub fn to_json(doc: &Document) -> Result<serde_json::Value, P3dError> {
    let (manifest, _, keys) = build(doc)?;
    let mut v = serde_json::to_value(&manifest).map_err(|e| P3dError::Format(e.to_string()))?;
    if let Some(meshes) = v.get_mut("meshes").and_then(serde_json::Value::as_array_mut) {
        for (entry, key) in meshes.iter_mut().zip(keys) {
            if let Some(m) = doc.mesh(key) {
                entry["data"] = mesh_json(m);
            }
        }
    }
    Ok(v)
}

/// Saves atomically: writes a temporary file next to `path`, flushes it to
/// disk, then renames it over `path`.
///
/// Complexity: O(total mesh size + objects).
pub fn save(doc: &Document, path: &Path) -> Result<SaveReport, P3dError> {
    let (bytes, report) = to_bytes(doc)?;
    let io = |e: std::io::Error| P3dError::Io { path: path.display().to_string(), source: e };
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document.p3d".into());
    let tmp = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // Best effort: do not leave the temporary file behind.
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(io)?;
    tracing::info!(path = %path.display(), bytes = report.bytes, blobs = report.blobs.len(), "saved");
    Ok(report)
}

/// Reads a `.p3d` file.
///
/// Complexity: O(file size).
pub fn load(path: &Path) -> Result<Document, P3dError> {
    let bytes = std::fs::read(path).map_err(|e| P3dError::Io { path: path.display().to_string(), source: e })?;
    from_bytes(&bytes)
}

fn read_entry(zip: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, P3dError> {
    let mut f = zip.by_name(name).map_err(|e| P3dError::Format(format!("{name}: {e}")))?;
    let mut buf = Vec::with_capacity(f.size().min(1 << 30) as usize);
    f.read_to_end(&mut buf).map_err(|e| P3dError::Zip(format!("{name}: {e}")))?;
    Ok(buf)
}

/// Parses `.p3d` bytes, checking blob hashes, mesh invariants and the
/// object hierarchy.
///
/// Complexity: O(file size + objects x log objects).
pub fn from_bytes(bytes: &[u8]) -> Result<Document, P3dError> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(zip_err)?;
    let json: serde_json::Value =
        serde_json::from_slice(&read_entry(&mut zip, "manifest.json")?).map_err(|e| P3dError::Format(format!("manifest.json: {e}")))?;
    if json.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT) {
        return Err(P3dError::Format("manifest.json has no `format: pixel3d`".into()));
    }
    let manifest: Manifest = serde_json::from_value(migrate::migrate(json)?).map_err(|e| P3dError::Format(format!("manifest.json: {e}")))?;

    let mut doc = Document::new();
    doc.settings = manifest.settings.clone();
    doc.extras = manifest.extras.clone();
    *doc.expectations_mut() = manifest.expectations.clone();

    // Meshes: each blob decoded once; copies share the decoded data.
    let mut decoded: BTreeMap<&str, std::sync::Arc<p3d_mesh::Mesh>> = BTreeMap::new();
    let mut mesh_keys: BTreeMap<u32, MeshKey> = BTreeMap::new();
    for m in &manifest.meshes {
        let arc = match decoded.get(m.blob.as_str()) {
            Some(a) => std::sync::Arc::clone(a),
            None => {
                let data = read_entry(&mut zip, &format!("blobs/{}.bin", m.blob))?;
                if blob::hash(&data) != m.blob {
                    return Err(P3dError::Blob { hash: m.blob.clone(), reason: "content does not match its hash (corrupt file)".into() });
                }
                let mesh = blob::decode(&data).map_err(|reason| P3dError::Blob { hash: m.blob.clone(), reason })?;
                let a = std::sync::Arc::new(mesh);
                decoded.insert(&m.blob, std::sync::Arc::clone(&a));
                a
            }
        };
        if mesh_keys.insert(m.id, doc.add_shared_mesh(arc)).is_some() {
            return Err(P3dError::Format(format!("mesh id {} appears twice", m.id)));
        }
    }

    // Objects, parents first: breadth-first from the roots over the listed order.
    let mut children: BTreeMap<Option<u32>, Vec<&ObjectEntry>> = BTreeMap::new();
    for o in &manifest.objects {
        children.entry(o.parent).or_default().push(o);
    }
    let mut keys: BTreeMap<u32, ObjectKey> = BTreeMap::new();
    let mut queue: std::collections::VecDeque<&ObjectEntry> = children.get(&None).map(|v| v.iter().copied().collect()).unwrap_or_default();
    let fail = |o: &ObjectEntry, reason: String| P3dError::Object { id: o.id, name: o.name.clone(), reason };
    while let Some(o) = queue.pop_front() {
        let kind = match (o.kind.as_str(), o.mesh) {
            ("empty", None) => ObjectKind::Empty,
            ("mesh", Some(m)) => ObjectKind::Mesh { mesh: *mesh_keys.get(&m).ok_or_else(|| fail(o, format!("unknown mesh {m}")))? },
            (k, _) => return Err(fail(o, format!("bad kind `{k}` or mesh reference"))),
        };
        let parent = o.parent.map(|p| keys.get(&p).copied().ok_or_else(|| fail(o, format!("parent {p} missing")))).transpose()?;
        let key = doc.create_object_with_id(o.id, &o.name, parent, kind).map_err(|e| fail(o, e.to_string()))?;
        let t = &o.transform;
        let trs = Trs { translation: Vec3::from(t.translation), rotation: Quat::from_array(t.rotation), scale: Vec3::from(t.scale) };
        if !trs.is_finite() {
            return Err(fail(o, "non-finite transform".into()));
        }
        if let Some(obj) = doc.object_mut(key) {
            obj.transform = trs;
            obj.tags = o.tags.iter().cloned().collect();
            obj.metadata = o.metadata.clone();
            obj.visible = o.visible;
            obj.locked = o.locked;
        }
        if !o.history.is_empty() {
            doc.set_history(key, OpGraph { nodes: o.history.clone() }).map_err(|e| fail(o, e.to_string()))?;
        }
        keys.insert(o.id, key);
        if let Some(kids) = children.get(&Some(o.id)) {
            queue.extend(kids.iter().copied());
        }
    }
    if keys.len() != manifest.objects.len() {
        let lost = manifest.objects.iter().find(|o| !keys.contains_key(&o.id));
        return Err(match lost {
            Some(o) => fail(o, "unreachable from the top level (missing parent or cycle)".into()),
            None => P3dError::Format("object ids repeat".into()),
        });
    }
    // Meshes no object uses would leak; files written by pixel3d have none.
    if doc.mesh_count() != mesh_keys.len() || mesh_keys.values().any(|&k| doc.mesh_users(k) == 0) {
        return Err(P3dError::Format("a mesh entry is used by no object".into()));
    }
    doc.reserve_ids_below(manifest.next_id);
    Ok(doc)
}
