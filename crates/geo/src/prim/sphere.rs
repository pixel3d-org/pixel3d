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

/// UV sphere parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvSphereSpec {
    pub radius: f64,
    /// Segments around the Y axis (meridians).
    pub segments: u32,
    /// Bands from pole to pole (parallels + 1).
    pub rings: u32,
}

impl Default for UvSphereSpec {
    fn default() -> Self {
        UvSphereSpec { radius: 0.5, segments: 32, rings: 16 }
    }
}

/// Generates a latitude-longitude sphere: quads between parallels and a fan
/// of triangles at each pole (two poles of valence `segments`). Meridian 0
/// faces +Z, so the mesh is mirror-symmetric across the YZ plane. No groups,
/// no sharp edges.
///
/// Complexity: O(segments x rings).
pub fn uv_sphere(spec: &UvSphereSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("radius", spec.radius)?;
    check_segments("segments", spec.segments, 3, MAX_SEGMENTS)?;
    check_segments("rings", spec.rings, 2, MAX_SEGMENTS)?;
    let (s, n, r) = (spec.segments as usize, spec.rings, spec.radius);
    let dirs = super::cylinder::ring_dirs(spec.segments, std::f64::consts::FRAC_PI_2);
    let faces = s * n as usize;
    let mut soup = Soup::with_capacity(s * (n as usize - 1) + 2, faces, 4 * faces);
    let top = soup.push_vert(DVec3::Y * r);
    // Parallels from the top down; parallel i is at polar angle pi i / n.
    let mut rows: Vec<Vec<u32>> = Vec::with_capacity(n as usize - 1);
    for i in 1..n {
        let theta = std::f64::consts::PI * f64::from(i) / f64::from(n);
        // Exact equator and mirror-symmetric heights.
        let (y, rho) = if 2 * i == n { (0.0, r) } else { (r * theta.cos(), r * theta.sin()) };
        rows.push(dirs.iter().map(|d| soup.push_vert(*d * rho + DVec3::Y * y)).collect());
    }
    let bottom = soup.push_vert(DVec3::NEG_Y * r);
    let out = |soup: &Soup, vs: &[u32]| vs.iter().fold(DVec3::ZERO, |acc, &v| acc + soup.position(v));
    for k in 0..s {
        let k1 = (k + 1) % s;
        let t = [top, rows[0][k1], rows[0][k]];
        soup.poly_facing(&t, out(&soup, &t), 0);
        for i in 0..rows.len() - 1 {
            let q = [rows[i][k], rows[i][k1], rows[i + 1][k1], rows[i + 1][k]];
            soup.poly_facing(&q, out(&soup, &q), 0);
        }
        let last = rows.len() - 1;
        let b = [bottom, rows[last][k], rows[last][k1]];
        soup.poly_facing(&b, out(&soup, &b), 0);
    }
    let (mut mesh, _) = soup.finish()?;
    place(&mut mesh, origin, r);
    Ok(mesh)
}

/// Ico sphere parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IcoSphereSpec {
    pub radius: f64,
    /// Subdivision level: 20 x 4^level triangles.
    pub subdivisions: u32,
}

impl Default for IcoSphereSpec {
    fn default() -> Self {
        IcoSphereSpec { radius: 0.5, subdivisions: 2 }
    }
}

/// Largest ico sphere subdivision level (20 x 4^8 = 1.3M triangles).
pub const MAX_ICO_SUBDIVISIONS: u32 = 8;

/// Generates a geodesic sphere: an icosahedron with a vertex at each pole,
/// each face split into 4^level triangles on a barycentric grid and projected
/// onto the sphere. Nearly equal triangles; twelve valence-5 vertices, all
/// others valence 6. Mirror-symmetric across the YZ plane. No groups, no
/// sharp edges.
///
/// Complexity: O(4^level).
pub fn ico_sphere(spec: &IcoSphereSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("radius", spec.radius)?;
    check_segments("subdivisions", spec.subdivisions, 0, MAX_ICO_SUBDIVISIONS)?;
    let r = spec.radius;
    let n = 1u32 << spec.subdivisions;
    // Poles at +-Y, two rings of five at latitude +-atan(1/2); the upper ring
    // starts at the front (+Z), the lower one is turned by 36 degrees.
    let lat = 0.5f64.atan();
    let (y, rho) = (lat.sin(), lat.cos());
    let mut corners = vec![DVec3::Y];
    let ring = |offset: f64, y: f64| {
        (0..5).map(move |k| {
            let a = std::f64::consts::FRAC_PI_2 + offset + std::f64::consts::TAU * f64::from(k) / 5.0;
            DVec3::new(rho * a.cos(), y, rho * a.sin())
        })
    };
    corners.extend(ring(0.0, y));
    corners.extend(ring(std::f64::consts::PI / 5.0, -y));
    corners.push(DVec3::NEG_Y);
    let mut tris: Vec<[usize; 3]> = Vec::with_capacity(20);
    for k in 0..5 {
        let (u0, u1) = (1 + k, 1 + (k + 1) % 5);
        let (l0, l1) = (6 + k, 6 + (k + 1) % 5);
        tris.extend([[0, u0, u1], [u0, l0, u1], [u1, l0, l1], [11, l1, l0]]);
    }

    let faces = 20 * (n * n) as usize;
    let mut soup = Soup::with_capacity(faces / 2 + 2, faces, 3 * faces);
    // A point on edge (a, b) at step t of n, always computed from the lower
    // corner index so that both faces of an edge weld exactly.
    let edge_point = |a: usize, b: usize, t: u32| -> DVec3 {
        let (lo, hi, t) = if a < b { (a, b, t) } else { (b, a, n - t) };
        let p = corners[lo] + (corners[hi] - corners[lo]) * (f64::from(t) / f64::from(n));
        p / p.length() * r
    };
    let mut grid: Vec<u32> = Vec::new();
    for [a, b, c] in tris {
        // Rows i = 0..=n from corner a; row i has i + 1 points from edge ab to edge ac.
        grid.clear();
        for i in 0..=n {
            for j in 0..=i {
                let p = if i == 0 {
                    corners[a] * r
                } else if j == 0 {
                    edge_point(a, b, i)
                } else if j == i {
                    edge_point(a, c, i)
                } else if i == n {
                    edge_point(b, c, j)
                } else {
                    let (wb, wc) = (f64::from(i - j) / f64::from(n), f64::from(j) / f64::from(n));
                    let q = corners[a] * (1.0 - wb - wc) + corners[b] * wb + corners[c] * wc;
                    q / q.length() * r
                };
                let p = if i == n && j == 0 {
                    corners[b] * r
                } else if i == n && j == n {
                    corners[c] * r
                } else {
                    p
                };
                grid.push(soup.vert(p));
            }
        }
        let at = |i: u32, j: u32| grid[(i * (i + 1) / 2 + j) as usize];
        for i in 0..n {
            for j in 0..=i {
                let t = [at(i, j), at(i + 1, j), at(i + 1, j + 1)];
                let c = t.iter().fold(DVec3::ZERO, |acc, &v| acc + soup.position(v));
                soup.poly_facing(&t, c, 0);
                if j < i {
                    let t = [at(i, j), at(i + 1, j + 1), at(i, j + 1)];
                    let c = t.iter().fold(DVec3::ZERO, |acc, &v| acc + soup.position(v));
                    soup.poly_facing(&t, c, 0);
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
    fn uv_spheres_have_two_poles() {
        for (s, n) in [(3, 2), (8, 4), (12, 7), (32, 16)] {
            let m = uv_sphere(&UvSphereSpec { radius: 1.5, segments: s, rings: n }, Origin::Center).unwrap();
            m.validate().unwrap();
            assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
            assert_eq!((m.vert_count(), m.face_count()), ((s * (n - 1) + 2) as usize, (s * n) as usize));
            assert!(m.verts().all(|v| (m.positions[v as usize].length() - 1.5).abs() < 1e-5));
            assert!(m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f)) > 0.0));
            assert_eq!(m.faces().filter(|&f| m.face_len(f) == 3).count(), (2 * s) as usize);
            assert!(m.verts().any(|v| m.positions[v as usize].abs_diff_eq(glam::Vec3::new(0.0, 0.0, 1.5), 1e-6)) || n % 2 == 1);
        }
        assert!(uv_sphere(&UvSphereSpec { rings: 1, ..UvSphereSpec::default() }, Origin::Center).is_err());
    }

    #[test]
    fn ico_spheres_are_geodesic() {
        for level in 0..4 {
            let m = ico_sphere(&IcoSphereSpec { radius: 2.0, subdivisions: level }, Origin::Center).unwrap();
            m.validate().unwrap();
            let tris = 20 * 4usize.pow(level);
            assert_eq!((m.face_count(), m.vert_count()), (tris, tris / 2 + 2));
            assert!(m.faces().all(|f| m.face_len(f) == 3 && m.face_normal(f).dot(m.face_centroid(f)) > 0.0));
            assert!(m.verts().all(|v| (m.positions[v as usize].length() - 2.0).abs() < 1e-5));
            assert_eq!(m.verts().filter(|&v| m.valence(v) == 5).count(), 12);
            assert!(m.verts().all(|v| matches!(m.valence(v), 5 | 6)));
            // Mirror symmetry across YZ.
            for v in m.verts() {
                let p = m.positions[v as usize];
                let q = glam::Vec3::new(-p.x, p.y, p.z);
                assert!(m.verts().any(|w| m.positions[w as usize].abs_diff_eq(q, 1e-5)), "{p} has no mirror");
            }
        }
        let m = ico_sphere(&IcoSphereSpec::default(), Origin::Bottom).unwrap();
        assert!(m.bounds().unwrap().0.y.abs() < 1e-6, "a pole vertex rests on the ground");
        assert!(ico_sphere(&IcoSphereSpec { subdivisions: 9, ..IcoSphereSpec::default() }, Origin::Center).is_err());
    }

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
