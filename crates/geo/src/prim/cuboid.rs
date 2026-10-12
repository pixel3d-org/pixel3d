//! Box with optional rounded or chamfered edges.

use glam::DVec3;
use p3d_mesh::Mesh;

use super::soup::{Soup, tag_groups_and_sharp};
use super::{MAX_BEVEL_SEGMENTS, MAX_SEGMENTS, Origin, PrimError, check_segments, check_size, even_grid, place};

/// Box parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxSpec {
    /// Full size along X, Y, Z in meters.
    pub size: DVec3,
    /// Quads along X, Y, Z on the flat part of each face.
    pub segments: [u32; 3],
    /// Bevel radius in meters; 0 for sharp edges.
    pub bevel_width: f64,
    /// Segments across each bevel: 1 is a flat chamfer, more is rounder.
    pub bevel_segments: u32,
}

impl Default for BoxSpec {
    fn default() -> Self {
        BoxSpec { size: DVec3::ONE, segments: [1; 3], bevel_width: 0.0, bevel_segments: 1 }
    }
}

/// Face group names by tag: tag `2 * axis + (sign < 0)`, then the bevel.
const NAMES: [Option<&str>; 7] = [Some("right"), Some("left"), Some("top"), Some("bottom"), Some("front"), Some("back"), Some("bevel")];
const BEVEL: u16 = 6;

/// Unit direction of a non-negative parameter triple with per-axis signs.
/// Every shared vertex is computed from the same triple, so results are
/// bit-identical wherever regions meet.
#[inline]
fn dir(p: [f64; 3], s: [f64; 3]) -> DVec3 {
    let v = DVec3::from(p);
    let v = v / v.length();
    DVec3::new(s[0] * v.x, s[1] * v.y, s[2] * v.z)
}

/// Generates a box centered on X and Z. Groups: `right` (+X), `left`, `top`
/// (+Y), `bottom`, `front` (+Z), `back`, and `bevel` when beveled. Edges
/// between differently grouped faces are sharp.
///
/// With a bevel of `s` segments the edges become quarter-cylinders and the
/// corners sphere patches. Even `s` gives quads only; odd `s` needs exactly one
/// triangle per corner (a three-sided patch with an odd number of edges per
/// side cannot be filled with quads).
///
/// Complexity: O(faces) = O(sx sy + sy sz + sz sx + s x (sx + sy + sz) + s^2).
pub fn cuboid(spec: &BoxSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("size.x", spec.size.x)?;
    check_size("size.y", spec.size.y)?;
    check_size("size.z", spec.size.z)?;
    for (axis, &n) in spec.segments.iter().enumerate() {
        check_segments(["segments.x", "segments.y", "segments.z"][axis], n, 1, MAX_SEGMENTS)?;
    }
    let h = spec.size / 2.0;
    let r = if spec.bevel_segments == 0 { 0.0 } else { spec.bevel_width };
    let min_h = h.min_element();
    if !r.is_finite() || r < 0.0 || r >= min_h * (1.0 - 1e-6) {
        return Err(PrimError::InvalidParam {
            param: "bevel_width",
            value: format!("{}", spec.bevel_width),
            reason: format!("must be at least 0 and less than half the smallest size ({min_h} m)"),
        });
    }
    let s_count = if r > 0.0 { spec.bevel_segments } else { 0 };
    if r > 0.0 {
        check_segments("bevel_segments", s_count, 1, MAX_BEVEL_SEGMENTS)?;
    }

    let a = h - DVec3::splat(r);
    let n = spec.segments;
    let grid = [even_grid(a.x, n[0]), even_grid(a.y, n[1]), even_grid(a.z, n[2])];
    // tan of the arc angles k * 90 / S up to 45 degrees, exact at 0 and 45.
    let tans: Vec<f64> = (0..=s_count / 2)
        .map(|k| match k {
            0 => 0.0,
            _ if 2 * k == s_count => 1.0,
            _ => (f64::from(k) * std::f64::consts::FRAC_PI_2 / f64::from(s_count)).tan(),
        })
        .collect();

    let faces_hint = 2 * (n[0] * n[1] + n[1] * n[2] + n[2] * n[0]) as usize;
    let mut soup = Soup::with_capacity(faces_hint + 8, faces_hint, 4 * faces_hint);
    let pos = |inner: [f64; 3], d: DVec3| DVec3::from(inner) + d * r;

    // Flat faces.
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        for s in [1.0, -1.0] {
            let tag = (2 * axis + usize::from(s < 0.0)) as u16;
            let mut p = [0.0; 3];
            p[axis] = 1.0;
            let mut sign = [1.0; 3];
            sign[axis] = s;
            let d = dir(p, sign);
            let mut outward = DVec3::ZERO;
            outward[axis] = s;
            for i in 0..n[u] as usize {
                for j in 0..n[v] as usize {
                    let mut q = [0u32; 4];
                    for (c, (di, dj)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
                        let mut inner = [0.0; 3];
                        inner[axis] = s * a[axis];
                        inner[u] = grid[u][i + di];
                        inner[v] = grid[v][j + dj];
                        q[c] = soup.vert(pos(inner, d));
                    }
                    soup.poly_facing(&q, outward, tag);
                }
            }
        }
    }

    if r > 0.0 {
        let sc = s_count as usize;
        // Arc parameter of segment boundary k along an edge, from face A1 toward A2.
        let arc = |k: usize| if 2 * k <= sc { (1.0, tans[k]) } else { (tans[sc - k], 1.0) };

        // Edge strips: quarter cylinders along each inner box edge.
        for e in 0..3 {
            let (a1, a2) = ((e + 1) % 3, (e + 2) % 3);
            for s1 in [1.0, -1.0] {
                for s2 in [1.0, -1.0] {
                    let mut sign = [1.0; 3];
                    sign[a1] = s1;
                    sign[a2] = s2;
                    let point = |t: usize, k: usize| {
                        let (p1, p2) = arc(k);
                        let mut p = [0.0; 3];
                        p[a1] = p1;
                        p[a2] = p2;
                        let mut inner = [0.0; 3];
                        inner[a1] = s1 * a[a1];
                        inner[a2] = s2 * a[a2];
                        inner[e] = grid[e][t];
                        pos(inner, dir(p, sign))
                    };
                    for t in 0..n[e] as usize {
                        for k in 0..sc {
                            let q = [point(t, k), point(t + 1, k), point(t + 1, k + 1), point(t, k + 1)].map(|p| soup.vert(p));
                            let mid = (k as f64 + 0.5) * std::f64::consts::FRAC_PI_2 / sc as f64;
                            let mut outward = DVec3::ZERO;
                            outward[a1] = s1 * mid.cos();
                            outward[a2] = s2 * mid.sin();
                            soup.poly_facing(&q, outward, BEVEL);
                        }
                    }
                }
            }
        }

        // Corner patches: three quadrant grids of m x m cells; for odd S also
        // three strips between quadrants and one center triangle.
        let m = sc / 2;
        for sx in [1.0, -1.0] {
            for sy in [1.0, -1.0] {
                for sz in [1.0, -1.0] {
                    let sign = [sx, sy, sz];
                    let inner = [sx * a.x, sy * a.y, sz * a.z];
                    let emit = |soup: &mut Soup, params: &[[f64; 3]]| {
                        let ids: Vec<u32> = params.iter().map(|&p| soup.vert(pos(inner, dir(p, sign)))).collect();
                        let centroid = params.iter().fold([0.0; 3], |acc, p| [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]]);
                        soup.poly_facing(&ids, dir(centroid, sign), BEVEL);
                    };
                    let at = |q: usize, b: f64, c: f64| {
                        let mut p = [0.0; 3];
                        p[q] = 1.0;
                        p[(q + 1) % 3] = b;
                        p[(q + 2) % 3] = c;
                        p
                    };
                    for q in 0..3 {
                        for i in 0..m {
                            for j in 0..m {
                                let (u0, u1, v0, v1) = (tans[i], tans[i + 1], tans[j], tans[j + 1]);
                                emit(&mut soup, &[at(q, u0, v0), at(q, u1, v0), at(q, u1, v1), at(q, u0, v1)]);
                            }
                        }
                    }
                    if sc % 2 == 1 {
                        let um = tans[m];
                        for q in 0..3 {
                            // Strip across the cube edge between quadrant q and quadrant q + 1.
                            let q1 = (q + 1) % 3;
                            for j in 0..m {
                                let (v0, v1) = (tans[j], tans[j + 1]);
                                // In quadrant q1 the free axes are (q + 2, q): the shared axis q + 2 comes first.
                                emit(&mut soup, &[at(q, um, v0), at(q, um, v1), at(q1, v1, um), at(q1, v0, um)]);
                            }
                        }
                        emit(&mut soup, &[at(0, um, um), at(1, um, um), at(2, um, um)]);
                    }
                }
            }
        }
    }

    let (mut mesh, tags) = soup.finish()?;
    tag_groups_and_sharp(&mut mesh, &tags, if r > 0.0 { &NAMES } else { &NAMES[..6] }, |x, y| x != y);
    place(&mut mesh, origin, h.y);
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

    fn check_closed(m: &Mesh) {
        m.validate().unwrap();
        assert_eq!(euler(m), 2, "closed genus-0 surface");
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // Outward: every face normal points away from the center.
        let c = {
            let (lo, hi) = m.bounds().unwrap();
            (lo + hi) / 2.0
        };
        for f in m.faces() {
            assert!(m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0, "face {f} points inward");
        }
    }

    #[test]
    fn plain_box() {
        let m = cuboid(&BoxSpec { size: DVec3::new(2.0, 1.0, 0.5), ..BoxSpec::default() }, Origin::Center).unwrap();
        check_closed(&m);
        assert_eq!((m.vert_count(), m.face_count()), (8, 6));
        assert_eq!(m.bounds().unwrap(), (Vec3::new(-1.0, -0.5, -0.25), Vec3::new(1.0, 0.5, 0.25)));
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 12);
        for (name, n) in
            [("top", Vec3::Y), ("bottom", Vec3::NEG_Y), ("front", Vec3::Z), ("back", Vec3::NEG_Z), ("right", Vec3::X), ("left", Vec3::NEG_X)]
        {
            let g = m.group(name).unwrap();
            assert_eq!(g.count(), 1, "{name}");
            assert!(g.iter().all(|f| m.face_normal(f).abs_diff_eq(n, 1e-6)), "{name}");
        }
        assert!(m.group("bevel").is_none());
    }

    #[test]
    fn segmented_box_and_bottom_origin() {
        let m = cuboid(&BoxSpec { segments: [2, 3, 4], ..BoxSpec::default() }, Origin::Bottom).unwrap();
        check_closed(&m);
        assert_eq!(m.face_count(), 2 * (2 * 3 + 3 * 4 + 4 * 2));
        let (lo, hi) = m.bounds().unwrap();
        assert!(lo.abs_diff_eq(Vec3::new(-0.5, 0.0, -0.5), 1e-6) && hi.abs_diff_eq(Vec3::new(0.5, 1.0, 0.5), 1e-6));
        assert_eq!(m.group("front").unwrap().count(), 2 * 3);
    }

    #[test]
    fn chamfer_has_one_triangle_per_corner() {
        let m = cuboid(&BoxSpec { bevel_width: 0.1, bevel_segments: 1, ..BoxSpec::default() }, Origin::Center).unwrap();
        check_closed(&m);
        assert_eq!((m.vert_count(), m.face_count(), m.triangle_count()), (24, 26, 44));
        assert_eq!(m.faces().filter(|&f| m.face_len(f) == 3).count(), 8);
        assert_eq!(m.group("bevel").unwrap().count(), 20);
        // Only flat-face borders are sharp: 6 faces x 4 edges.
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 24);
    }

    #[test]
    fn even_bevel_is_all_quads_and_round() {
        for segs in [2, 4, 6] {
            let r = 0.2;
            let m = cuboid(&BoxSpec { bevel_width: r, bevel_segments: segs, segments: [2, 1, 3], ..BoxSpec::default() }, Origin::Center).unwrap();
            check_closed(&m);
            assert!(m.faces().all(|f| m.face_len(f) == 4), "segments {segs}");
            // Bevel vertices lie on the rounded surface: distance r from the inner box.
            let a = Vec3::splat(0.5 - r as f32);
            for v in m.verts() {
                let p = m.positions[v as usize];
                let d = (p - p.clamp(-a, a)).length();
                assert!((d - r as f32).abs() < 1e-5, "vertex {v} at {d}");
            }
        }
    }

    #[test]
    fn odd_bevels_have_exactly_eight_triangles() {
        for segs in [3, 5] {
            let m = cuboid(&BoxSpec { bevel_width: 0.1, bevel_segments: segs, ..BoxSpec::default() }, Origin::Center).unwrap();
            check_closed(&m);
            assert_eq!(m.faces().filter(|&f| m.face_len(f) == 3).count(), 8);
            assert!(m.faces().all(|f| matches!(m.face_len(f), 3 | 4)));
        }
    }

    #[test]
    fn rejects_bad_params() {
        let bad = |s: BoxSpec| cuboid(&s, Origin::Center).is_err();
        assert!(bad(BoxSpec { bevel_width: 0.5, ..BoxSpec::default() }), "bevel as wide as the box");
        assert!(bad(BoxSpec { bevel_width: -0.1, ..BoxSpec::default() }));
        assert!(bad(BoxSpec { segments: [0, 1, 1], ..BoxSpec::default() }));
        assert!(bad(BoxSpec { size: DVec3::new(1.0, 0.0, 1.0), ..BoxSpec::default() }));
        assert!(bad(BoxSpec { bevel_width: 0.1, bevel_segments: 65, ..BoxSpec::default() }));
        // Zero bevel segments means no bevel at all.
        assert!(!bad(BoxSpec { bevel_width: 0.1, bevel_segments: 0, ..BoxSpec::default() }));
    }
}
