//! Cube-sphere: the default sphere (even quads, no poles).

use glam::DVec3;
use p3d_mesh::Mesh;

use super::soup::Soup;
use super::{MAX_SEGMENTS, Origin, PrimError, check_segments, check_size, place};

/// Quad sphere parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadSphereSpec {
    pub radius: f64,
    /// Quads along each edge of the underlying cube (6 x segments^2 faces).
    pub segments: u32,
}

impl Default for QuadSphereSpec {
    fn default() -> Self {
        QuadSphereSpec { radius: 0.5, segments: 8 }
    }
}

/// Generates a sphere by projecting a subdivided cube. Grid lines are spaced
/// by equal angle (tangent spacing), which keeps quads close to square; the
/// eight cube corners become valence-3 vertices. No groups, no sharp edges.
///
/// Complexity: O(segments^2).
pub fn quad_sphere(spec: &QuadSphereSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("radius", spec.radius)?;
    check_segments("segments", spec.segments, 1, MAX_SEGMENTS / 4)?;
    let n = spec.segments;
    // tan(-45 + 90 i / n) with exact, symmetric ends.
    let g: Vec<f64> = (0..=n)
        .map(|i| {
            let t = |i: u32| (-std::f64::consts::FRAC_PI_4 + std::f64::consts::FRAC_PI_2 * f64::from(i) / f64::from(n)).tan();
            match i {
                0 => -1.0,
                _ if i == n => 1.0,
                _ if 2 * i == n => 0.0,
                _ if 2 * i > n => -t(n - i),
                _ => t(i),
            }
        })
        .collect();
    let faces = 6 * (n * n) as usize;
    let mut soup = Soup::with_capacity(faces + 2, faces, 4 * faces);
    let r = spec.radius;
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for s in [1.0, -1.0] {
            let mut outward = DVec3::ZERO;
            outward[axis] = s;
            for i in 0..n as usize {
                for j in 0..n as usize {
                    let q = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(di, dj)| {
                        let mut p = DVec3::ZERO;
                        p[axis] = s;
                        p[u] = g[i + di];
                        p[v] = g[j + dj];
                        // Same triple, same arithmetic: shared edge points weld exactly.
                        soup.vert(p / p.length() * r)
                    });
                    soup.poly_facing(&q, outward, 0);
                }
            }
        }
    }
    let (mut mesh, _) = soup.finish()?;
    place(&mut mesh, origin, r);
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_round_and_outward() {
        for n in [1, 2, 3, 8] {
            let m = quad_sphere(&QuadSphereSpec { radius: 2.0, segments: n }, Origin::Center).unwrap();
            m.validate().unwrap();
            let nn = (n * n) as usize;
            assert_eq!((m.face_count(), m.vert_count()), (6 * nn, 6 * nn + 2));
            assert!(m.verts().all(|v| (m.positions[v as usize].length() - 2.0).abs() < 1e-5));
            assert!(m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f)) > 0.0));
            assert_eq!(m.verts().filter(|&v| m.valence(v) == 3).count(), 8);
        }
    }

    #[test]
    fn bottom_origin() {
        let m = quad_sphere(&QuadSphereSpec::default(), Origin::Bottom).unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!(lo.y.abs() < 1e-6 && (hi.y - 1.0).abs() < 1e-6);
    }
}
