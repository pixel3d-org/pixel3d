//! Ring shapes: torus and tube (a cylinder with a hole).

use glam::DVec3;
use p3d_mesh::Mesh;

use super::cylinder::ring_dirs;
use super::soup::{Soup, tag_groups_and_sharp};
use super::{MAX_SEGMENTS, Origin, PrimError, check_segments, check_size, place};

/// Torus parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TorusSpec {
    /// Distance from the center to the middle of the ring.
    pub major_radius: f64,
    /// Radius of the ring's cross-section; less than the major radius.
    pub minor_radius: f64,
    /// Segments around the Y axis.
    pub segments: u32,
    /// Segments around the cross-section.
    pub sides: u32,
}

impl Default for TorusSpec {
    fn default() -> Self {
        TorusSpec { major_radius: 0.5, minor_radius: 0.15, segments: 32, sides: 12 }
    }
}

/// Generates a torus lying in the XZ plane: all quads, every vertex of
/// valence 4. Segment 0 faces +Z and side 0 is on the outer equator, so the
/// mesh is mirror-symmetric across the YZ plane (and across XZ for an even
/// number of sides). No groups, no sharp edges.
///
/// Complexity: O(segments x sides).
pub fn torus(spec: &TorusSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("major_radius", spec.major_radius)?;
    check_size("minor_radius", spec.minor_radius)?;
    if spec.minor_radius >= spec.major_radius * (1.0 - 1e-6) {
        return Err(PrimError::InvalidParam {
            param: "minor_radius",
            value: format!("{}", spec.minor_radius),
            reason: format!("must be less than the major radius ({} m), or the ring intersects itself", spec.major_radius),
        });
    }
    check_segments("segments", spec.segments, 3, MAX_SEGMENTS)?;
    check_segments("sides", spec.sides, 3, MAX_SEGMENTS)?;
    let (big, small) = (spec.major_radius, spec.minor_radius);
    let (s, t) = (spec.segments as usize, spec.sides as usize);
    let dirs = ring_dirs(spec.segments, std::f64::consts::FRAC_PI_2);
    // Cross-section angles with exact quarter points.
    const QUARTERS: [(f64, f64); 4] = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
    let section: Vec<(f64, f64)> = (0..t)
        .map(|j| {
            if (4 * j).is_multiple_of(t) {
                QUARTERS[4 * j / t]
            } else {
                let a = std::f64::consts::TAU * j as f64 / t as f64;
                (a.cos(), a.sin())
            }
        })
        .collect();
    let mut soup = Soup::with_capacity(s * t, s * t, 4 * s * t);
    let mut v: Vec<u32> = Vec::with_capacity(s * t);
    for d in &dirs {
        for &(c, sn) in &section {
            v.push(soup.push_vert(*d * (big + small * c) + DVec3::Y * (small * sn)));
        }
    }
    for k in 0..s {
        let k1 = (k + 1) % s;
        for j in 0..t {
            let j1 = (j + 1) % t;
            let q = [v[k * t + j], v[k1 * t + j], v[k1 * t + j1], v[k * t + j1]];
            let c = q.iter().fold(DVec3::ZERO, |acc, &x| acc + soup.position(x)) / 4.0;
            // Outward from the core circle's chord under the quad (exact for any segment count).
            let core = (dirs[k] + dirs[k1]) * (big / 2.0);
            soup.poly_facing(&q, c - core, 0);
        }
    }
    let (mut mesh, _) = soup.finish()?;
    place(&mut mesh, origin, small);
    Ok(mesh)
}

/// Tube parameters: a cylinder with a coaxial hole.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TubeSpec {
    pub outer_radius: f64,
    /// Less than the outer radius.
    pub inner_radius: f64,
    pub height: f64,
    /// Segments around the axis.
    pub segments: u32,
    /// Segments along the axis.
    pub height_segments: u32,
}

impl Default for TubeSpec {
    fn default() -> Self {
        TubeSpec { outer_radius: 0.5, inner_radius: 0.4, height: 1.0, segments: 32, height_segments: 1 }
    }
}

const OUTER: u16 = 0;
const INNER: u16 = 1;
const TOP: u16 = 2;
const BOTTOM: u16 = 3;

/// Generates a tube along Y: outer and inner walls joined by ring-shaped
/// caps, all quads. Groups: `outer`, `inner` (facing the axis), `cap_top`,
/// `cap_bottom`; the four rims are sharp. Ring vertex 0 faces +Z.
///
/// Complexity: O(segments x height_segments).
pub fn tube(spec: &TubeSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("outer_radius", spec.outer_radius)?;
    check_size("inner_radius", spec.inner_radius)?;
    if spec.inner_radius >= spec.outer_radius * (1.0 - 1e-6) {
        return Err(PrimError::InvalidParam {
            param: "inner_radius",
            value: format!("{}", spec.inner_radius),
            reason: format!("must be less than the outer radius ({} m)", spec.outer_radius),
        });
    }
    check_size("height", spec.height)?;
    check_segments("segments", spec.segments, 3, MAX_SEGMENTS)?;
    check_segments("height_segments", spec.height_segments, 1, MAX_SEGMENTS)?;
    let (s, rows) = (spec.segments as usize, spec.height_segments);
    let hh = spec.height / 2.0;
    let dirs = ring_dirs(spec.segments, std::f64::consts::FRAC_PI_2);
    let faces = 2 * s * (rows as usize + 1);
    let mut soup = Soup::with_capacity(2 * s * (rows as usize + 1), faces, 4 * faces);
    let mut rings = [Vec::new(), Vec::new()];
    for (w, r) in [spec.outer_radius, spec.inner_radius].into_iter().enumerate() {
        for j in 0..=rows {
            let y = if j == rows { hh } else { -hh + spec.height * f64::from(j) / f64::from(rows) };
            rings[w].push(dirs.iter().map(|d| soup.push_vert(*d * r + DVec3::Y * y)).collect::<Vec<u32>>());
        }
    }
    for (w, tag, sign) in [(0, OUTER, 1.0), (1, INNER, -1.0)] {
        for j in 0..rows as usize {
            for k in 0..s {
                let k1 = (k + 1) % s;
                let q = [rings[w][j][k], rings[w][j][k1], rings[w][j + 1][k1], rings[w][j + 1][k]];
                soup.poly_facing(&q, (dirs[k] + dirs[k1]) * sign, tag);
            }
        }
    }
    for (j, tag, up) in [(rows as usize, TOP, DVec3::Y), (0, BOTTOM, DVec3::NEG_Y)] {
        for k in 0..s {
            let k1 = (k + 1) % s;
            let q = [rings[0][j][k], rings[0][j][k1], rings[1][j][k1], rings[1][j][k]];
            soup.poly_facing(&q, up, tag);
        }
    }
    let (mut mesh, tags) = soup.finish()?;
    tag_groups_and_sharp(&mut mesh, &tags, &[Some("outer"), Some("inner"), Some("cap_top"), Some("cap_bottom")], |a, b| a != b);
    place(&mut mesh, origin, hh);
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use p3d_mesh::EdgeFlags;

    fn euler(m: &Mesh) -> i64 {
        m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
    }

    #[test]
    fn torus_is_a_closed_genus_one_quad_mesh() {
        for (s, t) in [(3, 3), (8, 6), (24, 12), (13, 7)] {
            let spec = TorusSpec { major_radius: 1.0, minor_radius: 0.25, segments: s, sides: t };
            let m = torus(&spec, Origin::Center).unwrap();
            m.validate().unwrap();
            assert_eq!(euler(&m), 0, "genus 1");
            assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
            assert!(m.faces().all(|f| m.face_len(f) == 4) && m.verts().all(|v| m.valence(v) == 4));
            // Every vertex lies on the tube around the core circle.
            for v in m.verts() {
                let p = m.positions[v as usize];
                let core = Vec3::new(p.x, 0.0, p.z).normalize() * 1.0;
                assert!(((p - core).length() - 0.25).abs() < 1e-5);
            }
            // Outward: away from the core circle (coarse rings cut inside it).
            for f in m.faces().filter(|_| s >= 8) {
                let c = m.face_centroid(f);
                let core = Vec3::new(c.x, 0.0, c.z).normalize();
                assert!(m.face_normal(f).dot(c - core) > 0.0, "face {f}");
            }
        }
        let m = torus(&TorusSpec::default(), Origin::Bottom).unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!(lo.y.abs() < 1e-6 && (hi.y - 0.3).abs() < 1e-6);
        assert!(m.verts().any(|v| m.positions[v as usize].abs_diff_eq(Vec3::new(0.0, 0.15, 0.65), 1e-6)), "front outer vertex");
        assert!(torus(&TorusSpec { minor_radius: 0.5, ..TorusSpec::default() }, Origin::Center).is_err());
    }

    #[test]
    fn tube_walls_and_ring_caps() {
        for (s, rows) in [(3, 1), (8, 3), (32, 2)] {
            let spec = TubeSpec { outer_radius: 0.5, inner_radius: 0.3, height: 1.0, segments: s, height_segments: rows };
            let m = tube(&spec, Origin::Center).unwrap();
            m.validate().unwrap();
            assert_eq!(euler(&m), 0, "a tube is a torus topologically");
            assert!(m.faces().all(|f| m.face_len(f) == 4));
            assert_eq!(m.group("inner").unwrap().count(), (s * rows) as usize);
            assert_eq!(m.group("cap_top").unwrap().count(), s as usize);
            for f in m.group("inner").unwrap().iter() {
                let c = m.face_centroid(f);
                assert!(m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) < 0.0, "inner walls face the axis");
            }
            for f in m.group("outer").unwrap().iter() {
                let c = m.face_centroid(f);
                assert!(m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
            }
            assert!(m.group("cap_bottom").unwrap().iter().all(|f| m.face_normal(f).abs_diff_eq(Vec3::NEG_Y, 1e-5)));
            assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), (4 * s) as usize);
        }
        assert!(tube(&TubeSpec { inner_radius: 0.5, ..TubeSpec::default() }, Origin::Center).is_err());
    }
}
