//! Shading: sharp edges by angle and weighted custom normals.

use glam::Vec3;
use p3d_mesh::{EdgeFlags, Mesh};

/// What [`sharp_by_angle`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SharpReport {
    /// Edges now sharp that were not.
    pub marked: usize,
    /// Edges no longer sharp.
    pub cleared: usize,
    /// Total sharp edges after the call.
    pub sharp: usize,
}

/// Marks edges whose faces meet at more than `angle_deg` sharp and the rest
/// smooth; with `keep_existing`, edges already sharp stay sharp. Boundary
/// edges are left alone. Custom normals are dropped so the flags take
/// effect.
///
/// Complexity: O(E + F + C).
pub fn sharp_by_angle(mesh: &mut Mesh, angle_deg: f32, keep_existing: bool) -> SharpReport {
    let cos = angle_deg.clamp(0.0, 180.0).to_radians().cos();
    let normals = mesh.face_normals();
    let mut r = SharpReport::default();
    let edges: Vec<u32> = mesh.edges().collect();
    for e in edges {
        let [Some(a), Some(b)] = mesh.edge_faces(e) else { continue };
        let was = mesh.edge_flags(e).contains(EdgeFlags::SHARP);
        let want = normals[a as usize].dot(normals[b as usize]) < cos || (keep_existing && was);
        if want != was {
            mesh.set_edge_flag(e, EdgeFlags::SHARP, want);
            if want {
                r.marked += 1;
            } else {
                r.cleared += 1;
            }
        }
        r.sharp += usize::from(want);
    }
    mesh.corner_normal = None;
    r
}

/// How [`weighted_normals`] weighs the faces around a vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Weighting {
    /// By face area: large flat faces dominate (the hard-surface look).
    #[default]
    Area,
    /// By the face's corner angle at the vertex.
    Angle,
    /// By both.
    AreaAngle,
}

/// Sets custom corner normals from the faces around each vertex, weighted
/// by area and/or corner angle, smoothing only within runs of faces that
/// are not separated by sharp edges or the boundary. With area weighting a
/// large flat face keeps its normal across small bevel faces next to it,
/// which makes low-poly hard-surface models shade cleanly in engines.
/// Returns the number of corners set.
///
/// Complexity: O(V + F + C).
pub fn weighted_normals(mesh: &mut Mesh, weighting: Weighting) -> usize {
    let face_n = mesh.face_normals();
    let face_a: Vec<f32> = (0..mesh.face_slots() as u32).map(|f| if mesh.is_face_alive(f) { mesh.face_area(f) } else { 0.0 }).collect();
    let mut out = vec![Vec3::ZERO; mesh.he_slots()];
    let mut fan: Vec<u32> = Vec::new();
    let mut set = 0usize;
    for v in mesh.verts() {
        fan.clear();
        fan.extend(mesh.vert_outgoing(v));
        let k = fan.len();
        let breaks = |j: usize| -> bool {
            let h = fan[j % k];
            mesh.is_boundary_he(h) || mesh.is_boundary_he(h ^ 1) || mesh.edge_flags(h >> 1).contains(EdgeFlags::SHARP)
        };
        let start = (0..k).find(|&j| breaks(j)).unwrap_or(0);
        let mut j = start;
        while j < start + k {
            let run = j;
            let mut sum = Vec3::ZERO;
            loop {
                let h = fan[j % k];
                if let Some(f) = mesh.face(h) {
                    let w = match weighting {
                        Weighting::Area => face_a[f as usize],
                        Weighting::Angle => mesh.corner_angle(h),
                        Weighting::AreaAngle => face_a[f as usize] * mesh.corner_angle(h),
                    };
                    sum += face_n[f as usize] * w;
                }
                j += 1;
                if j >= start + k || breaks(j) {
                    break;
                }
            }
            let n = sum.normalize_or_zero();
            for i in run..j {
                let h = fan[i % k];
                if !mesh.is_boundary_he(h) {
                    out[h as usize] = n;
                    set += 1;
                }
            }
        }
    }
    mesh.corner_normal = Some(out);
    mesh.revision += 1;
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, CapKind, CylinderSpec, Origin, cuboid, cylinder};
    use glam::DVec3;

    #[test]
    fn sharp_by_angle_on_a_cylinder() {
        let mut m = cylinder(&CylinderSpec { segments: 16, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Center).unwrap();
        // Clear the rims first, then let the angle find them again.
        let r = sharp_by_angle(&mut m, 180.0, false);
        assert_eq!((r.cleared, r.sharp), (32, 0));
        let r = sharp_by_angle(&mut m, 30.0, false);
        assert_eq!((r.marked, r.sharp), (32, 32), "the two 90 degree rims, not the 22.5 degree side edges");
        let r = sharp_by_angle(&mut m, 10.0, true);
        assert_eq!(r.sharp, 48, "side edges too now");
    }

    #[test]
    fn weighted_normals_keep_big_faces_flat() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, bevel_width: 0.02, bevel_segments: 1, ..BoxSpec::default() }, Origin::Center).unwrap();
        // Smooth everything: plain smooth normals would bend the big faces.
        for e in m.edges().collect::<Vec<_>>() {
            m.set_edge_flag(e, EdgeFlags::SHARP, false);
        }
        let set = weighted_normals(&mut m, Weighting::Area);
        assert_eq!(set, m.faces().map(|f| m.face_len(f)).sum::<usize>());
        let n = m.corner_normal.clone().unwrap();
        let top = m.group("top").unwrap().iter().next().unwrap();
        for h in m.face_halfedges(top) {
            assert!(n[h as usize].dot(Vec3::Y) > 0.99, "top corners stay up: {}", n[h as usize]);
        }
        // Tri buffers use them.
        let b = m.to_tri_buffers(&p3d_mesh::TriOptions::default());
        assert!(b.normals.iter().all(|x| (x.length() - 1.0).abs() < 1e-4));
    }
}
