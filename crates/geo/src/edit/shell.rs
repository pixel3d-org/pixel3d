//! Thickening surfaces into shells, and smoothing vertex positions.

use glam::Vec3;
use p3d_mesh::{CornerSrc, EdgeFlags, Mesh};
use rustc_hash::FxHashMap;

use super::{AUTO_SHARP_DEG, KernelError, corners_normal, face_bits, live_faces, patch, region_fans, sharpen_by_angle};

/// Thickening parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThickenSpec {
    /// Wall thickness in meters.
    pub thickness: f32,
    /// Where the wall goes: -1 behind the surface (default), 0 centered on
    /// it, 1 in front of it.
    pub offset: f32,
}

impl Default for ThickenSpec {
    fn default() -> Self {
        ThickenSpec { thickness: 0.01, offset: -1.0 }
    }
}

/// Faces a thicken made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThickenResult {
    /// The new back faces (facing the other way).
    pub inner: Vec<u32>,
    /// Faces closing the wall along open borders.
    pub rim: Vec<u32>,
}

/// Largest stretch of the wall at creases, as a multiple of the thickness.
const MAX_STRETCH: f32 = 4.0;

/// Gives surfaces a wall thickness: a reversed copy of the faces is offset
/// along the vertex normals (stretched at creases so that every face keeps
/// the full thickness) and joined to the original along open borders. A
/// closed part becomes hollow. The selection must be whole surfaces: its
/// border must be the mesh's open border.
///
/// Complexity: O(c log c + valences) for the c corners of the faces.
pub fn thicken(mesh: &mut Mesh, faces: &[u32], spec: &ThickenSpec) -> Result<ThickenResult, KernelError> {
    let faces = live_faces(mesh, faces)?;
    if !(spec.thickness.is_finite() && spec.thickness > 0.0) {
        return Err(KernelError::OutOfRange {
            param: "thickness",
            message: "thickness must be more than 0".into(),
            hint: "for example 0.005 (5 mm)".into(),
        });
    }
    if !(spec.offset.is_finite() && (-1.0..=1.0).contains(&spec.offset)) {
        return Err(KernelError::OutOfRange {
            param: "offset",
            message: format!("offset {} is outside -1 to 1", spec.offset),
            hint: "-1 keeps the surface as the outside".into(),
        });
    }
    let region = face_bits(mesh, &faces);
    let in_region = |f: Option<u32>| f.is_some_and(|f| region.contains(f as usize));
    if let Some(h) =
        faces.iter().flat_map(|&f| mesh.face_halfedges(f).collect::<Vec<_>>()).find(|&h| mesh.face(h ^ 1).is_some() && !in_region(mesh.face(h ^ 1)))
    {
        return Err(KernelError::OutOfRange {
            param: "select",
            message: format!("the selection borders unselected faces at edge {}", h >> 1),
            hint: "select whole surfaces (or omit select): thicken walls only grow along open borders".into(),
        });
    }
    let fans = region_fans(mesh, &region, &faces);
    let (out_k, in_k) = ((spec.offset + 1.0) / 2.0, (1.0 - spec.offset) / 2.0);
    let mut p = patch(mesh);
    let mut copy_of: FxHashMap<u32, u32> = FxHashMap::default();
    let mut moved: Vec<(u32, Vec3)> = Vec::new();
    for fan in &fans.corners {
        let v = mesh.origin(fan[0]);
        let n = corners_normal(mesh, fan);
        let min_cos = fan.iter().filter_map(|&h| mesh.face(h)).map(|f| mesh.face_normal(f).dot(n)).fold(1.0f32, f32::min);
        let along = n * spec.thickness / min_cos.max(1.0 / MAX_STRETCH);
        let c = p.new_vert(mesh.positions[v as usize] - along * in_k, Some(v));
        if out_k > 0.0 {
            moved.push((v, along * out_k));
        }
        for &h in fan {
            copy_of.insert(h, c);
        }
    }
    let mut inner_idx = Vec::new();
    for &f in &faces {
        let hs: Vec<u32> = mesh.face_halfedges(f).collect();
        let vs: Vec<u32> = hs.iter().rev().map(|h| copy_of[h]).collect();
        let src: Vec<CornerSrc> = hs.iter().rev().map(|&h| CornerSrc::copy(h)).collect();
        inner_idx.push(p.add_face(&vs, &src, mesh.face_material(f), None));
    }
    let mut rim_idx = Vec::new();
    for &f in &faces {
        for h in mesh.face_halfedges(f) {
            if mesh.face(h ^ 1).is_none() {
                let (u, w) = (mesh.origin(h), mesh.dest(h));
                let n = mesh.next(h);
                let src = [CornerSrc::copy(n), CornerSrc::copy(h), CornerSrc::copy(h), CornerSrc::copy(n)];
                rim_idx.push(p.add_face(&[w, u, copy_of[&h], copy_of[&n]], &src, mesh.face_material(f), None));
            }
        }
    }
    let r = mesh.apply_patch(&p)?;
    for (v, off) in moved {
        mesh.positions[v as usize] += off;
    }
    let inner: Vec<u32> = inner_idx.iter().map(|&i| r.faces[i]).collect();
    let rim: Vec<u32> = rim_idx.iter().map(|&i| r.faces[i]).collect();
    sharpen_by_angle(mesh, &rim, AUTO_SHARP_DEG);
    Ok(ThickenResult { inner, rim })
}

/// Smoothing parameters (Taubin's lambda and mu passes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmoothSpec {
    pub iterations: u32,
    /// Strength of the smoothing pass (lambda), 0 to 1.
    pub factor: f32,
    /// Keep open-border vertices in place.
    pub pin_boundary: bool,
    /// Keep vertices on sharp edges in place.
    pub pin_sharp: bool,
}

impl Default for SmoothSpec {
    fn default() -> Self {
        SmoothSpec { iterations: 10, factor: 0.5, pin_boundary: true, pin_sharp: true }
    }
}

/// Largest number of smoothing iterations.
pub const MAX_SMOOTH_ITERATIONS: u32 = 1000;

/// Smooths vertex positions without shrinking (Taubin 1995): each
/// iteration moves every free vertex toward the mean of its neighbours by
/// `factor`, then back by a slightly larger factor, which removes noise but
/// keeps the overall size. Returns the number of vertices that moved.
///
/// Complexity: O(iterations x sum of valences) of the selected vertices.
pub fn smooth(mesh: &mut Mesh, verts: &[u32], spec: &SmoothSpec) -> Result<usize, KernelError> {
    if !(1..=MAX_SMOOTH_ITERATIONS).contains(&spec.iterations) {
        return Err(KernelError::OutOfRange {
            param: "iterations",
            message: format!("{} iterations is outside 1 to {MAX_SMOOTH_ITERATIONS}", spec.iterations),
            hint: "5 to 20 is typical".into(),
        });
    }
    if !(spec.factor.is_finite() && spec.factor > 0.0 && spec.factor <= 1.0) {
        return Err(KernelError::OutOfRange {
            param: "factor",
            message: format!("factor {} is outside 0 to 1", spec.factor),
            hint: "0.5 is a good default".into(),
        });
    }
    let mut free: Vec<u32> = verts.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
    free.sort_unstable();
    free.dedup();
    free.retain(|&v| {
        !(spec.pin_boundary && mesh.is_boundary_vert(v))
            && !(spec.pin_sharp && mesh.vert_outgoing(v).any(|h| mesh.edge_flags(h >> 1).contains(EdgeFlags::SHARP)))
    });
    // Taubin's pass-band choice k_pb = 0.1: mu = 1 / (0.1 - 1 / lambda).
    let lambda = spec.factor;
    let mu = 1.0 / (0.1 - 1.0 / lambda);
    let mut next: Vec<Vec3> = vec![Vec3::ZERO; free.len()];
    for _ in 0..spec.iterations {
        for w in [lambda, mu] {
            for (i, &v) in free.iter().enumerate() {
                let (sum, n) = mesh.vert_neighbors(v).fold((Vec3::ZERO, 0u32), |(s, n), u| (s + mesh.positions[u as usize], n + 1));
                let p = mesh.positions[v as usize];
                next[i] = if n == 0 { p } else { p + (sum / n as f32 - p) * w };
            }
            for (i, &v) in free.iter().enumerate() {
                mesh.positions[v as usize] = next[i];
            }
        }
    }
    mesh.revision += 1;
    Ok(free.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, QuadSphereSpec, cuboid, plane, quad_sphere};
    use glam::DVec3;

    #[test]
    fn thicken_a_plane_into_a_slab() {
        let mut m = plane(&PlaneSpec { segments_x: 2, segments_z: 2, ..PlaneSpec::default() }).unwrap();
        let all: Vec<u32> = m.faces().collect();
        let r = thicken(&mut m, &all, &ThickenSpec { thickness: 0.1, offset: -1.0 }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.inner.len(), r.rim.len()), (4, 8));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.y + 0.1).abs() < 1e-6 && hi.y.abs() < 1e-6, "{lo} {hi}");
        assert!(r.inner.iter().all(|&f| m.face_normal(f).abs_diff_eq(Vec3::NEG_Y, 1e-6)));
        // Centered: half above, half below.
        let mut c = plane(&PlaneSpec::default()).unwrap();
        thicken(&mut c, &[0], &ThickenSpec { thickness: 0.2, offset: 0.0 }).unwrap();
        let (lo, hi) = c.bounds().unwrap();
        assert!((lo.y + 0.1).abs() < 1e-6 && (hi.y - 0.1).abs() < 1e-6);
    }

    #[test]
    fn thicken_a_closed_box_makes_it_hollow() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let all: Vec<u32> = m.faces().collect();
        let r = thicken(&mut m, &all, &ThickenSpec { thickness: 0.1, offset: -1.0 }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.inner.len(), r.rim.len(), m.face_components().1), (6, 0, 2));
        // The inner box is 0.8 wide: even thickness at the corners.
        let inner_x: f32 =
            r.inner.iter().flat_map(|&f| m.face_verts(f).map(|v| m.positions[v as usize].x).collect::<Vec<_>>()).fold(f32::MIN, f32::max);
        assert!((inner_x - 0.4).abs() < 1e-5, "{inner_x}");
        // Partial selections of a closed mesh are refused.
        let mut b = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        assert!(thicken(&mut b, &[0], &ThickenSpec::default()).is_err());
    }

    #[test]
    fn taubin_smoothing_keeps_size() {
        let mut m = quad_sphere(&QuadSphereSpec { radius: 1.0, segments: 8 }, Origin::Center).unwrap();
        // Add noise, then smooth it away.
        for (i, p) in m.positions.iter_mut().enumerate() {
            *p *= 1.0 + 0.05 * ((i as f32 * 12.9898).sin());
        }
        let all: Vec<u32> = m.verts().collect();
        // Spread of the radius around its mean (the noise, not the size).
        let noise = |m: &Mesh| {
            let rs: Vec<f32> = m.verts().map(|v| m.positions[v as usize].length()).collect();
            let mean = rs.iter().sum::<f32>() / rs.len() as f32;
            (rs.iter().map(|r| (r - mean) * (r - mean)).sum::<f32>() / rs.len() as f32).sqrt()
        };
        let before = noise(&m);
        let moved = smooth(&mut m, &all, &SmoothSpec::default()).unwrap();
        assert_eq!(moved, all.len());
        assert!(noise(&m) < before * 0.5, "{} vs {before}", noise(&m));
        let mean_r = m.verts().map(|v| m.positions[v as usize].length()).sum::<f32>() / m.vert_count() as f32;
        assert!(mean_r > 0.95, "no shrinking: {mean_r}");
        // Pinned borders stay.
        let mut g = plane(&PlaneSpec { segments_x: 4, segments_z: 4, ..PlaneSpec::default() }).unwrap();
        let before = g.positions.clone();
        let all: Vec<u32> = g.verts().collect();
        assert_eq!(smooth(&mut g, &all, &SmoothSpec::default()).unwrap(), 9);
        assert!(g.verts().filter(|&v| g.is_boundary_vert(v)).all(|v| g.positions[v as usize] == before[v as usize]));
        assert!(smooth(&mut g, &all, &SmoothSpec { iterations: 0, ..SmoothSpec::default() }).is_err());
    }
}
