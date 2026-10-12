//! Capsule (a cylinder with hemispherical ends) and wedge (a ramp).

use glam::DVec3;
use p3d_mesh::Mesh;

use super::cylinder::{quad_grid_cap, ring_dirs};
use super::soup::{Soup, tag_groups_and_sharp};
use super::{MAX_SEGMENTS, MIN_SIZE, Origin, PrimError, check_segments, check_size, place};

/// Capsule parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CapsuleSpec {
    pub radius: f64,
    /// Total height including both ends; more than twice the radius.
    pub height: f64,
    /// Segments around the axis; a multiple of 8 (so each end has a pole vertex).
    pub segments: u32,
    /// Segments along the straight part.
    pub height_segments: u32,
}

impl Default for CapsuleSpec {
    fn default() -> Self {
        CapsuleSpec { radius: 0.25, height: 1.0, segments: 16, height_segments: 1 }
    }
}

const SIDE: u16 = 0;
const TOP: u16 = 1;
const BOTTOM: u16 = 2;

/// Generates a capsule along Y with all-quad ends and no poles: each end is
/// the quad-grid cylinder cap laid onto the hemisphere by an azimuthal
/// equidistant map (distances from the pole are kept), like a quad sphere.
/// Groups: `side`, `cap_top`, `cap_bottom`; no sharp edges. Ring vertex
/// lines are at 45 + 90k degrees, as on quad-grid cylinders.
///
/// Complexity: O(segments x height_segments + segments^2).
pub fn capsule(spec: &CapsuleSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("radius", spec.radius)?;
    check_size("height", spec.height)?;
    let r = spec.radius;
    let straight = spec.height - 2.0 * r;
    if straight < MIN_SIZE {
        return Err(PrimError::InvalidParam {
            param: "height",
            value: format!("{}", spec.height),
            reason: format!("must exceed twice the radius ({} m); use a sphere for a round ball", 2.0 * r),
        });
    }
    check_segments("segments", spec.segments, 8, MAX_SEGMENTS)?;
    check_segments("height_segments", spec.height_segments, 1, MAX_SEGMENTS)?;
    let s = spec.segments;
    if !s.is_multiple_of(8) {
        let lower = (s / 8).max(1) * 8;
        return Err(PrimError::InvalidParam {
            param: "segments",
            value: s.to_string(),
            reason: format!("capsule ends need a multiple of 8 (try {lower} or {})", lower + 8),
        });
    }
    let hc = straight / 2.0;
    let dirs = ring_dirs(s, std::f64::consts::FRAC_PI_4);
    let rows = spec.height_segments;
    let mut soup = Soup::with_capacity((s * (rows + 1)) as usize + (s * s / 4) as usize, (s * rows + s * s / 4) as usize, 0);
    let mut ring: Vec<Vec<u32>> = Vec::with_capacity(rows as usize + 1);
    for j in 0..=rows {
        let y = if j == rows { hc } else { -hc + straight * f64::from(j) / f64::from(rows) };
        ring.push(dirs.iter().map(|d| soup.push_vert(*d * r + DVec3::Y * y)).collect());
    }
    for j in 0..rows as usize {
        for k in 0..s as usize {
            let k1 = (k + 1) % s as usize;
            soup.poly_facing(&[ring[j][k], ring[j][k1], ring[j + 1][k1], ring[j + 1][k]], dirs[k] + dirs[k1], SIDE);
        }
    }
    // The flat cap of radius pi r / 2 (the pole-to-equator arc) maps onto the hemisphere.
    let disc = std::f64::consts::FRAC_PI_2 * r;
    for (rim, sign, tag) in [(&ring[rows as usize], 1.0, TOP), (&ring[0], -1.0, BOTTOM)] {
        let center = DVec3::Y * (sign * hc);
        let map = |x: f64, z: f64| {
            let rho = x.hypot(z);
            if rho == 0.0 {
                return center + DVec3::Y * (sign * r);
            }
            let phi = rho / r;
            DVec3::new(x / rho, 0.0, z / rho) * (r * phi.sin()) + center + DVec3::Y * (sign * r * phi.cos())
        };
        quad_grid_cap(&mut soup, rim, &dirs, disc, tag, map, |c| c - center);
    }
    let (mut mesh, tags) = soup.finish()?;
    tag_groups_and_sharp(&mut mesh, &tags, &[Some("side"), Some("cap_top"), Some("cap_bottom")], |_, _| false);
    place(&mut mesh, origin, hc + r);
    Ok(mesh)
}

/// Wedge parameters: a ramp rising from the front edge to the back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WedgeSpec {
    /// Full size along X, Y, Z in meters.
    pub size: DVec3,
}

impl Default for WedgeSpec {
    fn default() -> Self {
        WedgeSpec { size: DVec3::ONE }
    }
}

const NAMES: [Option<&str>; 5] = [Some("bottom"), Some("back"), Some("slope"), Some("left"), Some("right")];

/// Generates a wedge (a triangular prism lying on a rectangle): the bottom
/// and the back are rectangles, the slope rises from the front bottom edge to
/// the top of the back, and the left and right ends are triangles. Groups:
/// `bottom`, `back`, `slope`, `left`, `right`; every edge is sharp.
///
/// Complexity: O(1).
pub fn wedge(spec: &WedgeSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("size.x", spec.size.x)?;
    check_size("size.y", spec.size.y)?;
    check_size("size.z", spec.size.z)?;
    let h = spec.size / 2.0;
    let mut soup = Soup::with_capacity(6, 5, 18);
    let p = |x: f64, y: f64, z: f64| DVec3::new(x * h.x, y * h.y, z * h.z);
    let [lbb, rbb, rfb, lfb, ltb, rtb] =
        [p(-1.0, -1.0, -1.0), p(1.0, -1.0, -1.0), p(1.0, -1.0, 1.0), p(-1.0, -1.0, 1.0), p(-1.0, 1.0, -1.0), p(1.0, 1.0, -1.0)]
            .map(|q| soup.push_vert(q));
    soup.poly_facing(&[lbb, rbb, rfb, lfb], DVec3::NEG_Y, 0);
    soup.poly_facing(&[lbb, rbb, rtb, ltb], DVec3::NEG_Z, 1);
    soup.poly_facing(&[lfb, rfb, rtb, ltb], DVec3::new(0.0, h.z, h.y), 2);
    soup.poly_facing(&[lbb, lfb, ltb], DVec3::NEG_X, 3);
    soup.poly_facing(&[rbb, rfb, rtb], DVec3::X, 4);
    let (mut mesh, tags) = soup.finish()?;
    tag_groups_and_sharp(&mut mesh, &tags, &NAMES, |a, b| a != b);
    place(&mut mesh, origin, h.y);
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use p3d_mesh::EdgeFlags;

    fn check_closed_convex(m: &Mesh) {
        m.validate().unwrap();
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // The vertex mean is inside a convex solid (the bounds center can lie on a face).
        let c = m.verts().map(|v| m.positions[v as usize]).sum::<Vec3>() / m.vert_count() as f32;
        for f in m.faces() {
            assert!(m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0, "face {f} points inward");
        }
    }

    #[test]
    fn capsules_are_round_quads_without_poles() {
        for (s, rows) in [(8, 1), (16, 2), (24, 1), (32, 3)] {
            let spec = CapsuleSpec { radius: 0.2, height: 1.0, segments: s, height_segments: rows };
            let m = capsule(&spec, Origin::Center).unwrap();
            check_closed_convex(&m);
            assert!(m.faces().all(|f| m.face_len(f) == 4), "{s}: quads only");
            assert!(m.verts().all(|v| matches!(m.valence(v), 3 | 4)), "{s}: no poles");
            let (lo, hi) = m.bounds().unwrap();
            assert!((hi - lo).abs_diff_eq(Vec3::new(0.4, 1.0, 0.4), 1e-5), "{s}: {}", hi - lo);
            // Every end vertex lies on its hemisphere.
            for (g, cy) in [("cap_top", 0.3f32), ("cap_bottom", -0.3)] {
                for f in m.group(g).unwrap().iter() {
                    for v in m.face_verts(f) {
                        let d = (m.positions[v as usize] - Vec3::new(0.0, cy, 0.0)).length();
                        assert!((d - 0.2).abs() < 1e-5, "{s} {g}: {d}");
                    }
                }
            }
            assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 0);
            // Ends keep reasonable quads.
            for f in m.group("cap_top").unwrap().iter() {
                for h in m.face_halfedges(f) {
                    assert!(m.corner_angle(h).to_degrees() > 35.0, "{s}: sliver at face {f}");
                }
            }
        }
        assert!(capsule(&CapsuleSpec { height: 0.5, radius: 0.25, ..CapsuleSpec::default() }, Origin::Center).is_err());
        assert!(capsule(&CapsuleSpec { segments: 12, ..CapsuleSpec::default() }, Origin::Center).is_err());
        let m = capsule(&CapsuleSpec::default(), Origin::Bottom).unwrap();
        assert!(m.bounds().unwrap().0.y.abs() < 1e-6);
    }

    #[test]
    fn wedge_is_a_ramp() {
        let m = wedge(&WedgeSpec { size: DVec3::new(2.0, 1.0, 3.0) }, Origin::Bottom).unwrap();
        check_closed_convex(&m);
        assert_eq!((m.vert_count(), m.face_count(), m.triangle_count()), (6, 5, 8));
        let (lo, hi) = m.bounds().unwrap();
        assert_eq!((lo, hi), (Vec3::new(-1.0, 0.0, -1.5), Vec3::new(1.0, 1.0, 1.5)));
        let slope = m.group("slope").unwrap().iter().next().unwrap();
        let n = m.face_normal(slope);
        assert!(n.y > 0.0 && n.z > 0.0 && n.x.abs() < 1e-6, "{n}");
        assert!(m.edges().all(|e| m.edge_flags(e).contains(EdgeFlags::SHARP)));
        for g in ["bottom", "back", "left", "right"] {
            assert_eq!(m.group(g).unwrap().count(), 1);
        }
    }
}
