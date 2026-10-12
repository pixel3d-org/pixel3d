//! Inset faces: shrink them inward within their surface, leaving a rim of
//! quads around them (optionally pushing the inner part in or out).

use glam::Vec3;
use p3d_mesh::{CornerSrc, Mesh};
use rustc_hash::FxHashMap;

use super::{
    AUTO_SHARP_DEG, KernelError, corner_edge_flags, corners_normal, face_bits, live_faces, patch, region_fans, restore_edge_flags, sharpen_by_angle,
};

/// How a set of faces is inset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InsetMode {
    /// Connected faces shrink together; the rim follows the region's border.
    #[default]
    Region,
    /// Every face shrinks on its own, with a rim all around it.
    Individual,
}

/// Inset parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsetSpec {
    pub mode: InsetMode,
    /// Rim width in meters, measured inside the faces.
    pub thickness: f32,
    /// Offset of the inner part along the normals; negative pushes it in.
    pub depth: f32,
}

impl Default for InsetSpec {
    fn default() -> Self {
        InsetSpec { mode: InsetMode::Region, thickness: 0.02, depth: 0.0 }
    }
}

/// Faces an inset made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InsetResult {
    /// The shrunk faces, replacing the originals (same groups and materials).
    pub inner: Vec<u32>,
    /// The rim quads (they join the groups of the face they came from).
    pub rim: Vec<u32>,
}

/// Largest stretch of the rim at sharp corners, as a multiple of the thickness.
const MAX_STRETCH: f32 = 4.0;

/// Inward offset of a vertex between an incoming edge `x -> v` (in a face
/// with normal `n_in`) and an outgoing edge `v -> q` (normal `n_out`), so that
/// both edges move inward by `thickness` (a miter, stretched at most
/// [`MAX_STRETCH`] times).
fn miter(x: Vec3, v: Vec3, q: Vec3, n_in: Vec3, n_out: Vec3, thickness: f32) -> Vec3 {
    let a = n_in.cross(v - x).normalize_or_zero();
    let b = n_out.cross(q - v).normalize_or_zero();
    let m = (a + b).try_normalize().unwrap_or(b);
    m * thickness / m.dot(b).max(1.0 / MAX_STRETCH)
}

/// Insets faces by `thickness`, moving the inner part by `depth` along the
/// normals. In region mode only the region's border shrinks (inner vertices
/// stay, and a vertex where the region touches itself gets one copy per
/// fan); in individual mode every face shrinks toward its own middle.
///
/// Fails without changing anything when the rim would fold over (thickness
/// too large for a face).
///
/// Complexity: O(c log c + valences of the region's vertices) for the c
/// corners of the selected faces.
pub fn inset(mesh: &mut Mesh, faces: &[u32], spec: &InsetSpec) -> Result<InsetResult, KernelError> {
    let faces = live_faces(mesh, faces)?;
    if !(spec.thickness.is_finite() && spec.thickness > 0.0) {
        return Err(KernelError::OutOfRange {
            param: "thickness",
            message: "inset thickness must be more than 0".into(),
            hint: "use a rim width such as 0.01 (10 mm)".into(),
        });
    }
    if !spec.depth.is_finite() {
        return Err(KernelError::OutOfRange { param: "depth", message: "depth must be finite".into(), hint: "use 0 for a flat inset".into() });
    }
    let source_flags = corner_edge_flags(mesh, &faces);
    let pos = |v: u32| mesh.positions[v as usize];
    let mut p = patch(mesh);
    // Vertex of each corner in the inner faces: a copy, or the vertex itself
    // inside a region (moved by `depth` afterwards).
    let mut inner_of: FxHashMap<u32, u32> = FxHashMap::default();
    let mut moved: Vec<(u32, Vec3)> = Vec::new();
    let region = face_bits(mesh, &faces);
    let in_region = |f: Option<u32>| f.is_some_and(|f| region.contains(f as usize));
    match spec.mode {
        InsetMode::Region => {
            let fans = region_fans(mesh, &region, &faces);
            for (i, fan) in fans.corners.iter().enumerate() {
                let (first, last) = (fan[0], fan[fan.len() - 1]);
                let v = mesh.origin(first);
                let n = corners_normal(mesh, fan);
                let target = if fans.closed[i] {
                    if spec.depth != 0.0 {
                        moved.push((v, n * spec.depth));
                    }
                    v
                } else {
                    let (x, q) = (mesh.origin(mesh.prev(last)), mesh.dest(first));
                    let (n_in, n_out) = (mesh.face(last).map_or(n, |f| mesh.face_normal(f)), mesh.face(first).map_or(n, |f| mesh.face_normal(f)));
                    let off = miter(pos(x), pos(v), pos(q), n_in, n_out, spec.thickness);
                    p.new_vert(pos(v) + off + n * spec.depth, Some(v))
                };
                for &h in fan {
                    inner_of.insert(h, target);
                }
            }
        }
        InsetMode::Individual => {
            for &f in &faces {
                let n = mesh.face_normal(f);
                for h in mesh.face_halfedges(f) {
                    let v = mesh.origin(h);
                    let (x, q) = (mesh.origin(mesh.prev(h)), mesh.dest(h));
                    let off = miter(pos(x), pos(v), pos(q), n, n, spec.thickness);
                    inner_of.insert(h, p.new_vert(pos(v) + off + n * spec.depth, Some(v)));
                }
            }
        }
    }
    let rim_of = |h: u32| match spec.mode {
        InsetMode::Region => !in_region(mesh.face(h ^ 1)),
        InsetMode::Individual => true,
    };
    // Position of a vertex as it will be.
    let moved_map: FxHashMap<u32, Vec3> = moved.iter().copied().collect();
    let new_pos = |v: u32| p.new_vert_position(v).unwrap_or_else(|| pos(v) + moved_map.get(&v).copied().unwrap_or(Vec3::ZERO));
    let newell = |vs: &[Vec3]| -> Vec3 {
        let mut s = Vec3::ZERO;
        for i in 0..vs.len() {
            s += vs[i].cross(vs[(i + 1) % vs.len()]);
        }
        s
    };
    // Refuse folded results before changing anything.
    for &f in &faces {
        let n = mesh.face_normal(f);
        let inner: Vec<Vec3> = mesh.face_halfedges(f).map(|h| new_pos(inner_of[&h])).collect();
        let too_wide = || KernelError::OutOfRange {
            param: "thickness",
            message: format!("an inset of {} m folds face {f} over", spec.thickness),
            hint: format!("use less than half the face's shortest edge ({} m)", shortest_edge(mesh, f) / 2.0),
        };
        if newell(&inner).dot(n) <= 0.0 {
            return Err(too_wide());
        }
        // Every inner edge keeps its direction: a reversed edge means the
        // rim folded over (a face shrunk past its middle turns by 180 degrees
        // and keeps its winding, so the normal alone cannot tell).
        for h in mesh.face_halfedges(f) {
            let old = pos(mesh.dest(h)) - pos(mesh.origin(h));
            let new = new_pos(inner_of[&mesh.next(h)]) - new_pos(inner_of[&h]);
            if new.dot(old) <= 0.0 {
                return Err(too_wide());
            }
        }
    }
    for &f in &faces {
        p.remove_face(f);
    }
    for &f in &faces {
        let vs: Vec<u32> = mesh.face_halfedges(f).map(|h| inner_of[&h]).collect();
        let src: Vec<CornerSrc> = mesh.face_halfedges(f).map(CornerSrc::copy).collect();
        p.add_face(&vs, &src, mesh.face_material(f), Some(f));
    }
    for &f in &faces {
        for h in mesh.face_halfedges(f).filter(|&h| rim_of(h)) {
            let n = mesh.next(h);
            let src = [CornerSrc::copy(h), CornerSrc::copy(n), CornerSrc::copy(n), CornerSrc::copy(h)];
            p.add_face(&[mesh.origin(h), mesh.dest(h), inner_of[&n], inner_of[&h]], &src, mesh.face_material(f), Some(f));
        }
    }
    let r = mesh.apply_patch(&p)?;
    for (v, off) in moved {
        mesh.positions[v as usize] += off;
    }
    let (inner, rim) = r.faces.split_at(faces.len());
    restore_edge_flags(mesh, inner, &source_flags);
    if spec.depth != 0.0 {
        sharpen_by_angle(mesh, rim, AUTO_SHARP_DEG);
    }
    Ok(InsetResult { inner: inner.to_vec(), rim: rim.to_vec() })
}

/// Length of a face's shortest edge.
///
/// Complexity: O(face size).
fn shortest_edge(mesh: &Mesh, f: u32) -> f32 {
    mesh.face_halfedges(f).map(|h| mesh.edge_length(h >> 1)).fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    fn euler(m: &Mesh) -> i64 {
        m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
    }

    #[test]
    fn inset_a_box_top() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Bottom).unwrap();
        let top: Vec<u32> = m.group("top").unwrap().iter().collect();
        let r = inset(&mut m, &top, &InsetSpec { thickness: 0.1, ..InsetSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.inner.len(), r.rim.len(), euler(&m)), (1, 4, 2));
        assert!((m.face_area(r.inner[0]) - 0.64).abs() < 1e-5, "0.8 x 0.8 inner face");
        assert_eq!(m.group("top").unwrap().count(), 5, "inner and rim stay in the group");
        assert!(r.rim.iter().all(|&f| m.face_normal(f).abs_diff_eq(Vec3::Y, 1e-6)));
        // Push the inner face down: a recess.
        let r2 = inset(&mut m, &r.inner, &InsetSpec { thickness: 0.05, depth: -0.1, ..InsetSpec::default() }).unwrap();
        m.validate().unwrap();
        assert!((m.face_centroid(r2.inner[0]).y - 0.9).abs() < 1e-5);
    }

    #[test]
    fn region_inset_keeps_inner_vertices() {
        let mut m = plane(&PlaneSpec { size_x: 3.0, size_z: 3.0, segments_x: 3, segments_z: 3 }).unwrap();
        let all: Vec<u32> = m.faces().collect();
        let r = inset(&mut m, &all, &InsetSpec { thickness: 0.1, depth: 0.2, ..InsetSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!(r.rim.len(), 12, "rim only along the border");
        assert_eq!(m.vert_count(), 16 + 12);
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.y - 0.2).abs() < 1e-6 && lo.y.abs() < 1e-6);
        // The inner region is 2.8 m wide: corners moved by the miter.
        let inner_xs: Vec<f32> = r.inner.iter().flat_map(|&f| m.face_verts(f).map(|v| m.positions[v as usize].x).collect::<Vec<_>>()).collect();
        let w = inner_xs.iter().fold(f32::MIN, |a, &b| a.max(b)) - inner_xs.iter().fold(f32::MAX, |a, &b| a.min(b));
        assert!((w - 2.8).abs() < 1e-5, "{w}");
    }

    #[test]
    fn individual_inset_of_a_strip() {
        let mut m = plane(&PlaneSpec { size_x: 2.0, size_z: 1.0, segments_x: 2, segments_z: 1 }).unwrap();
        let r = inset(&mut m, &[0, 1], &InsetSpec { mode: InsetMode::Individual, thickness: 0.1, depth: 0.0 }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.inner.len(), r.rim.len()), (2, 8));
        for &f in &r.inner {
            assert!((m.face_area(f) - 0.8 * 0.8).abs() < 1e-5);
        }
    }

    #[test]
    fn too_wide_insets_are_refused() {
        let mut m = plane(&PlaneSpec::default()).unwrap();
        let before = m.clone();
        let e = inset(&mut m, &[0], &InsetSpec { thickness: 0.6, ..InsetSpec::default() }).unwrap_err();
        assert!(matches!(e, KernelError::OutOfRange { param: "thickness", .. }), "{e:?}");
        assert_eq!(m, before);
        assert!(inset(&mut m, &[0], &InsetSpec { thickness: 0.0, ..InsetSpec::default() }).is_err());
    }
}
