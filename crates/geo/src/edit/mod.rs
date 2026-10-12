//! Local edit kernels. Each one works out which faces to replace and with
//! what, then applies a single [`p3d_mesh::Patch`] (ADR 0003), so its cost
//! depends on the edited region, not on the mesh size.

mod extrude;
mod faces;
mod inset;
mod split;

pub use extrude::{ExtrudeMode, ExtrudeResult, ExtrudeSpec, extrude};
pub use faces::{QuadSpec, poke, triangulate, tris_to_quads};
pub use inset::{InsetMode, InsetResult, InsetSpec, inset};
pub use split::{MAX_SUBDIVIDE_LEVELS, SplitResult, SubdivideResult, split, subdivide};

use glam::Vec3;
use p3d_mesh::{EdgeFlags, EditError, FixedBitSet, Mesh, Patch};
use rustc_hash::FxHashMap;

/// Why an edit kernel refused to run. Nothing changes when it fails.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum KernelError {
    /// The mesh kernel rejected the result (it would not be manifold).
    #[error("{0}")]
    Topology(#[from] EditError),
    /// A parameter does not fit the geometry.
    #[error("{message}")]
    OutOfRange { param: &'static str, message: String, hint: String },
    /// The selection has nothing the edit can work on.
    #[error("{0}")]
    Empty(String),
}

/// Sorted, deduplicated live faces; an error names the first dead one.
///
/// Complexity: O(k log k).
pub(crate) fn live_faces(mesh: &Mesh, faces: &[u32]) -> Result<Vec<u32>, KernelError> {
    let mut out = faces.to_vec();
    out.sort_unstable();
    out.dedup();
    if let Some(&f) = out.iter().find(|&&f| !mesh.is_face_alive(f)) {
        return Err(KernelError::Topology(EditError::NoSuchFace(f)));
    }
    if out.is_empty() {
        return Err(KernelError::Empty("no faces selected".into()));
    }
    Ok(out)
}

/// Bitset over face slots.
///
/// Complexity: O(slots / 64 + k).
pub(crate) fn face_bits(mesh: &Mesh, faces: &[u32]) -> FixedBitSet {
    let mut bits = FixedBitSet::with_capacity(mesh.face_slots());
    for &f in faces {
        bits.insert(f as usize);
    }
    bits
}

/// The fans of region faces around region vertices: every region corner
/// (half-edge of a region face) maps to the index of its fan, and each fan
/// lists its corners in rotation order. A fan is open when it starts and ends
/// at region boundary edges, closed when it surrounds its vertex.
///
/// Complexity: O(region corners) expected.
pub(crate) struct Fans {
    pub of_corner: FxHashMap<u32, u32>,
    /// Corners of each fan, in rotation order.
    pub corners: Vec<Vec<u32>>,
    pub closed: Vec<bool>,
}

pub(crate) fn region_fans(mesh: &Mesh, region: &FixedBitSet, faces: &[u32]) -> Fans {
    let in_region = |f: Option<u32>| f.is_some_and(|f| region.contains(f as usize));
    let mut fans = Fans { of_corner: FxHashMap::default(), corners: Vec::new(), closed: Vec::new() };
    for &f in faces {
        for h in mesh.face_halfedges(f) {
            if fans.of_corner.contains_key(&h) {
                continue;
            }
            // Back up to the first corner of the fan (or around a closed one).
            let mut first = h;
            let mut closed = false;
            loop {
                let t = first ^ 1;
                if !in_region(mesh.face(t)) {
                    break;
                }
                let before = mesh.next(t);
                if before == h {
                    closed = true;
                    break;
                }
                first = before;
            }
            let id = fans.corners.len() as u32;
            let mut list = Vec::new();
            let mut g = first;
            loop {
                fans.of_corner.insert(g, id);
                list.push(g);
                let t = mesh.prev(g) ^ 1;
                if !in_region(mesh.face(t)) || t == first {
                    break;
                }
                g = t;
            }
            fans.corners.push(list);
            fans.closed.push(closed);
        }
    }
    fans
}

/// Area-weighted mean normal of the faces of some corners.
///
/// Complexity: O(corners x face size).
pub(crate) fn corners_normal(mesh: &Mesh, corners: &[u32]) -> Vec3 {
    corners.iter().filter_map(|&h| mesh.face(h)).map(|f| mesh.face_normal_area2(f)).sum::<Vec3>().normalize_or_zero()
}

/// Marks the edges of the given faces sharp where their faces meet at more
/// than `deg` degrees. Never clears a flag.
///
/// Complexity: O(corners of the faces).
pub(crate) fn sharpen_by_angle(mesh: &mut Mesh, faces: &[u32], deg: f32) {
    let cos = deg.to_radians().cos();
    let mut sharp = Vec::new();
    for &f in faces {
        for h in mesh.face_halfedges(f) {
            if let [Some(a), Some(b)] = mesh.edge_faces(h >> 1)
                && mesh.face_normal(a).dot(mesh.face_normal(b)) < cos
            {
                sharp.push(h >> 1);
            }
        }
    }
    for e in sharp {
        mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
    }
}

/// The angle beyond which edges made by an edit are marked sharp, matching
/// the default of `shade.smooth_by_angle`.
pub const AUTO_SHARP_DEG: f32 = 30.0;

/// Flags of the edge of every corner of the given faces, in corner order.
///
/// Complexity: O(corners).
pub(crate) fn corner_edge_flags(mesh: &Mesh, faces: &[u32]) -> Vec<EdgeFlags> {
    faces.iter().flat_map(|&f| mesh.face_halfedges(f).map(|h| mesh.edge_flags(h >> 1))).collect()
}

/// Sets the edge flags of the new faces' corners from a list in corner order
/// (adds flags, never clears).
///
/// Complexity: O(corners).
pub(crate) fn restore_edge_flags(mesh: &mut Mesh, faces: &[u32], flags: &[EdgeFlags]) {
    let hs: Vec<u32> = faces.iter().flat_map(|&f| mesh.face_halfedges(f)).collect();
    for (h, fl) in hs.into_iter().zip(flags) {
        for flag in [EdgeFlags::SHARP, EdgeFlags::SEAM] {
            if fl.contains(flag) {
                mesh.set_edge_flag(h >> 1, flag, true);
            }
        }
    }
}

/// An empty patch for a mesh; shared so kernels read alike.
pub(crate) fn patch(mesh: &Mesh) -> Patch {
    Patch::new(mesh)
}
