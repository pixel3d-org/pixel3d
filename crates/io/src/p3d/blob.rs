//! Mesh blobs: rkyv-serialized plain arrays, content-addressed by blake3.

use p3d_mesh::{Mesh, RawGroup, RawMesh};
use rkyv::rancor;

/// Archived group.
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct BlobGroup {
    pub name: String,
    pub kind: u8,
    pub members: Vec<u32>,
}

/// Archived mesh arrays (layout version 1). Little-endian, 32-bit offsets,
/// the same bytes on every platform. `he_prev` and `he_face` are not stored:
/// they follow from `he_next` and `face_he` and are rebuilt on load.
#[derive(Debug, Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct BlobMesh {
    pub he_next: Vec<u32>,
    pub he_vert: Vec<u32>,
    pub vert_he: Vec<u32>,
    pub face_he: Vec<u32>,
    pub positions: Vec<[f32; 3]>,
    pub corner_uv: Vec<Vec<[f32; 2]>>,
    pub corner_normal: Option<Vec<[f32; 3]>>,
    pub corner_color: Option<Vec<[f32; 4]>>,
    pub face_material: Vec<u16>,
    pub edge_flags: Vec<u8>,
    pub edge_crease: Option<Vec<f32>>,
    pub groups: Vec<BlobGroup>,
}

impl From<RawMesh> for BlobMesh {
    fn from(r: RawMesh) -> Self {
        BlobMesh {
            he_next: r.he_next,
            he_vert: r.he_vert,
            vert_he: r.vert_he,
            face_he: r.face_he,
            positions: r.positions,
            corner_uv: r.corner_uv,
            corner_normal: r.corner_normal,
            corner_color: r.corner_color,
            face_material: r.face_material,
            edge_flags: r.edge_flags,
            edge_crease: r.edge_crease,
            groups: r.groups.into_iter().map(|g| BlobGroup { name: g.name, kind: g.kind, members: g.members }).collect(),
        }
    }
}

impl From<BlobMesh> for RawMesh {
    fn from(b: BlobMesh) -> Self {
        let mut raw = RawMesh {
            he_next: b.he_next,
            he_prev: Vec::new(),
            he_vert: b.he_vert,
            he_face: Vec::new(),
            vert_he: b.vert_he,
            face_he: b.face_he,
            positions: b.positions,
            corner_uv: b.corner_uv,
            corner_normal: b.corner_normal,
            corner_color: b.corner_color,
            face_material: b.face_material,
            edge_flags: b.edge_flags,
            edge_crease: b.edge_crease,
            groups: b.groups.into_iter().map(|g| RawGroup { name: g.name, kind: g.kind, members: g.members }).collect(),
        };
        raw.rebuild_links();
        raw
    }
}

/// Serializes a mesh; equal meshes give equal bytes.
///
/// Complexity: O(mesh size).
pub fn encode(mesh: &Mesh) -> Result<Vec<u8>, String> {
    let blob = BlobMesh::from(mesh.to_raw());
    rkyv::to_bytes::<rancor::Error>(&blob).map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// Validates and deserializes a blob, then checks every mesh invariant.
///
/// Complexity: O(blob size).
pub fn decode(bytes: &[u8]) -> Result<Mesh, String> {
    // rkyv needs aligned input; zip entries come in arbitrary buffers.
    let mut aligned = rkyv::util::AlignedVec::<16>::with_capacity(bytes.len());
    aligned.extend_from_slice(bytes);
    let blob = rkyv::from_bytes::<BlobMesh, rancor::Error>(&aligned).map_err(|e| e.to_string())?;
    Mesh::from_raw(blob.into()).map_err(|e| e.to_string())
}

/// blake3 hex digest.
pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
