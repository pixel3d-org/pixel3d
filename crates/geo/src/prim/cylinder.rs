//! Cylinders, cones and frustums along +Y with selectable cap topology.

use glam::DVec3;
use p3d_mesh::Mesh;

use super::soup::{Soup, tag_groups_and_sharp};
use super::{MAX_SEGMENTS, MAX_SIZE, MIN_SIZE, Origin, PrimError, check_segments, check_size, place};

/// How the caps are filled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CapKind {
    /// Quads only: a curved grid in the middle and rings around it.
    /// Needs a multiple of 4 segments.
    #[default]
    QuadGrid,
    /// One polygon per cap.
    Ngon,
    /// Triangles around a center vertex.
    Fan,
    /// Open ends.
    None,
}

/// Cylinder parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CylinderSpec {
    pub radius: f64,
    pub height: f64,
    /// Segments around the axis.
    pub segments: u32,
    /// Segments along the axis.
    pub height_segments: u32,
    pub cap: CapKind,
}

impl Default for CylinderSpec {
    fn default() -> Self {
        CylinderSpec { radius: 0.5, height: 1.0, segments: 32, height_segments: 1, cap: CapKind::QuadGrid }
    }
}

/// Cone or frustum parameters: a cylinder whose radius changes linearly from
/// bottom to top. One radius may be 0 for a pointed cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConeSpec {
    pub radius_bottom: f64,
    pub radius_top: f64,
    pub height: f64,
    /// Segments around the axis.
    pub segments: u32,
    /// Segments along the axis.
    pub height_segments: u32,
    pub cap: CapKind,
}

impl Default for ConeSpec {
    fn default() -> Self {
        ConeSpec { radius_bottom: 0.5, radius_top: 0.0, height: 1.0, segments: 32, height_segments: 1, cap: CapKind::QuadGrid }
    }
}

const SIDE: u16 = 0;
const TOP: u16 = 1;
const BOTTOM: u16 = 2;

/// Generates a cylinder around the Y axis. Groups: `side`, `cap_top`,
/// `cap_bottom` (no caps with [`CapKind::None`]). Rim edges are sharp.
/// Ring vertex 0 points to +Z (front), so the mesh is mirror-symmetric
/// across the YZ plane for any segment count.
///
/// Complexity: O(segments x height_segments + segments^2 / 16) for quad-grid
/// caps (the cap grid has segments^2 / 16 cells), O(segments x height_segments) otherwise.
pub fn cylinder(spec: &CylinderSpec, origin: Origin) -> Result<Mesh, PrimError> {
    check_size("radius", spec.radius)?;
    let cone_spec = ConeSpec {
        radius_bottom: spec.radius,
        radius_top: spec.radius,
        height: spec.height,
        segments: spec.segments,
        height_segments: spec.height_segments,
        cap: spec.cap,
    };
    cone(&cone_spec, origin)
}

fn check_radius(param: &'static str, r: f64) -> Result<(), PrimError> {
    if r == 0.0 { Ok(()) } else { check_size(param, r) }
}

/// Generates a cone or frustum around the Y axis, like [`cylinder`] with a
/// radius that changes linearly with height. A zero radius makes that end a
/// single apex vertex: the side row next to it is triangles and that end has
/// no cap. Groups: `side`, plus `cap_top` and `cap_bottom` for capped ends
/// with a radius. Rim edges are sharp.
///
/// Complexity: O(segments x height_segments + segments^2 / 16) for quad-grid
/// caps, O(segments x height_segments) otherwise.
pub fn cone(spec: &ConeSpec, origin: Origin) -> Result<Mesh, PrimError> {
    let (rb, rt) = (spec.radius_bottom, spec.radius_top);
    check_radius("radius_bottom", rb)?;
    check_radius("radius_top", rt)?;
    if rb == 0.0 && rt == 0.0 {
        return Err(PrimError::InvalidParam {
            param: "radius_bottom",
            value: "0".into(),
            reason: format!("at most one radius may be 0; the other must be between {MIN_SIZE} m and {MAX_SIZE} m"),
        });
    }
    check_size("height", spec.height)?;
    check_segments("segments", spec.segments, 3, MAX_SEGMENTS)?;
    check_segments("height_segments", spec.height_segments, 1, MAX_SEGMENTS)?;
    let s = spec.segments;
    if spec.cap == CapKind::QuadGrid && !s.is_multiple_of(4) {
        let lower = (s / 4).max(1) * 4;
        return Err(PrimError::InvalidParam {
            param: "segments",
            value: s.to_string(),
            reason: format!("quad_grid caps need a multiple of 4 (try {lower} or {})", lower + 4),
        });
    }
    let hh = spec.height / 2.0;
    // Quad-grid caps put grid corners on ring vertices at 45 + 90k degrees;
    // otherwise vertex 0 is at the front (+Z, 90 degrees).
    let phase = if spec.cap == CapKind::QuadGrid { std::f64::consts::FRAC_PI_4 } else { std::f64::consts::FRAC_PI_2 };
    let ring_dir = ring_dirs(s, phase);

    let rows = spec.height_segments;
    let mut soup = Soup::with_capacity((s * (rows + 1)) as usize, (s * rows) as usize + 2, (4 * s * rows) as usize);
    // Side rings, bottom to top; a zero-radius ring is one apex vertex.
    let mut ring: Vec<Vec<u32>> = Vec::with_capacity((rows + 1) as usize);
    for j in 0..=rows {
        let t = f64::from(j) / f64::from(rows);
        let (y, r) = match j {
            0 => (-hh, rb),
            _ if j == rows => (hh, rt),
            _ => (-hh + spec.height * t, rb + (rt - rb) * t),
        };
        ring.push(if r == 0.0 {
            vec![soup.push_vert(DVec3::Y * y)]
        } else {
            (0..s).map(|k| soup.push_vert(ring_dir[k as usize] * r + DVec3::Y * y)).collect()
        });
    }
    let at = |r: &[u32], k: usize| r[k % r.len()];
    for j in 0..rows as usize {
        for k in 0..s as usize {
            let k1 = (k + 1) % s as usize;
            let q = [at(&ring[j], k), at(&ring[j], k1), at(&ring[j + 1], k1), at(&ring[j + 1], k)];
            // An apex repeats one vertex: the quad becomes a triangle.
            let mut poly: smallvec::SmallVec<[u32; 4]> = smallvec::SmallVec::new();
            for (i, &v) in q.iter().enumerate() {
                if v != q[(i + 3) % 4] {
                    poly.push(v);
                }
            }
            soup.poly_facing(&poly, ring_dir[k] + ring_dir[k1], SIDE);
        }
    }

    let mut has = [true, false, false];
    if spec.cap != CapKind::None {
        for (rim, y, r, tag) in [(&ring[rows as usize], hh, rt, TOP), (&ring[0], -hh, rb, BOTTOM)] {
            if r == 0.0 {
                continue;
            }
            has[tag as usize] = true;
            let up = DVec3::Y * y.signum();
            match spec.cap {
                CapKind::None => {}
                CapKind::Ngon => soup.poly_facing(rim, up, tag),
                CapKind::Fan => {
                    let c = soup.push_vert(DVec3::Y * y);
                    for k in 0..s as usize {
                        soup.poly_facing(&[c, rim[k], rim[(k + 1) % s as usize]], up, tag);
                    }
                }
                CapKind::QuadGrid => quad_grid_cap(&mut soup, rim, &ring_dir, r, tag, |x, z| DVec3::new(x, y, z), |_| up),
            }
        }
    }

    let (mut mesh, tags) = soup.finish()?;
    let names = [has[0].then_some("side"), has[1].then_some("cap_top"), has[2].then_some("cap_bottom")];
    tag_groups_and_sharp(&mut mesh, &tags, &names, |a, b| a != b);
    place(&mut mesh, origin, hh);
    Ok(mesh)
}

/// Unit directions in the XZ plane of `s` ring vertices, the first at
/// `phase` radians from +X toward +Z.
///
/// Complexity: O(s).
pub(crate) fn ring_dirs(s: u32, phase: f64) -> Vec<DVec3> {
    (0..s)
        .map(|k| {
            let a = phase + std::f64::consts::TAU * f64::from(k) / f64::from(s);
            DVec3::new(a.cos(), 0.0, a.sin())
        })
        .collect()
}

/// Fills a disc of radius `r` bounded by `rim` (4m vertices) with quads: a
/// square grid of m x m cells in the middle, and rings that morph from that
/// square to the rim circle. Grid cells stay square, and ring quads at the
/// square's corners start near 45/135 degrees and relax toward rectangles
/// outward. New vertices are placed by `map(x, z)` from their position on the
/// flat disc (the rim is reused as is); `facing(centroid)` gives each quad's
/// outward direction.
///
/// Complexity: O(m^2 + m x rings) = O(segments^2).
pub(crate) fn quad_grid_cap(
    soup: &mut Soup,
    rim: &[u32],
    ring_dir: &[DVec3],
    r: f64,
    tag: u16,
    map: impl Fn(f64, f64) -> DVec3,
    facing: impl Fn(DVec3) -> DVec3,
) {
    let s = rim.len();
    let m = s / 4;
    let rings = m.div_ceil(2).max(1);
    // Similar cell sizes: grid cell 2b / m, ring width about (r - b) / rings.
    let b = r / (1.0 + 2.0 * rings as f64 / m as f64);

    // Square boundary in (u, v) (u along X, v along Z), counter-clockwise from
    // corner (1, 1): point k matches rim vertex k at 45 + 360 k / s degrees.
    let step = |t: usize| 1.0 - 2.0 * t as f64 / m as f64;
    let mut square: Vec<(f64, f64)> = Vec::with_capacity(s);
    square.extend((0..m).map(|t| (step(t), 1.0)));
    square.extend((0..m).map(|t| (-1.0, step(t))));
    square.extend((0..m).map(|t| (-step(t), -1.0)));
    square.extend((0..m).map(|t| (1.0, -step(t))));

    let quad = |soup: &mut Soup, q: [u32; 4]| {
        let c = q.iter().fold(DVec3::ZERO, |acc, &v| acc + soup.position(v)) / 4.0;
        soup.poly_facing(&q, facing(c), tag);
    };

    // Ring 0 is the square, the last ring is the rim; rings in between blend.
    let mut rings_v: Vec<Vec<u32>> = Vec::with_capacity(rings + 1);
    for j in 0..rings {
        let t = j as f64 / rings as f64;
        rings_v.push(
            (0..s)
                .map(|k| {
                    let (u, v) = square[k];
                    let p = DVec3::new(u * b, 0.0, v * b) * (1.0 - t) + ring_dir[k] * (r * t);
                    soup.push_vert(map(p.x, p.z))
                })
                .collect(),
        );
    }
    rings_v.push(rim.to_vec());
    for j in 0..rings {
        for k in 0..s {
            let k1 = (k + 1) % s;
            quad(soup, [rings_v[j][k], rings_v[j][k1], rings_v[j + 1][k1], rings_v[j + 1][k]]);
        }
    }

    // Square grid; its boundary is ring 0.
    let w = m + 1;
    let mut grid = vec![u32::MAX; w * w];
    let cell = |t: usize| -1.0 + 2.0 * t as f64 / m as f64;
    for (k, &(u, v)) in square.iter().enumerate() {
        let (i, j) = (((u + 1.0) * m as f64 / 2.0).round() as usize, ((v + 1.0) * m as f64 / 2.0).round() as usize);
        grid[j * w + i] = rings_v[0][k];
    }
    for j in 1..m {
        for i in 1..m {
            grid[j * w + i] = soup.push_vert(map(cell(i) * b, cell(j) * b));
        }
    }
    for j in 0..m {
        for i in 0..m {
            quad(soup, [grid[j * w + i], grid[j * w + i + 1], grid[(j + 1) * w + i + 1], grid[(j + 1) * w + i]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use p3d_mesh::EdgeFlags;

    fn check_closed(m: &Mesh) {
        m.validate().unwrap();
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        let (lo, hi) = m.bounds().unwrap();
        let c = (lo + hi) / 2.0;
        for f in m.faces() {
            assert!(m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0, "face {f} points inward");
        }
    }

    #[test]
    fn quad_grid_caps() {
        for s in [4, 8, 12, 32, 64, 256] {
            let m = cylinder(&CylinderSpec { segments: s, height_segments: 2, ..CylinderSpec::default() }, Origin::Center).unwrap();
            check_closed(&m);
            assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only, {s} segments");
            assert_eq!(m.group("side").unwrap().count(), (2 * s) as usize);
            assert!(m.group("cap_top").unwrap().iter().all(|f| m.face_normal(f).abs_diff_eq(Vec3::Y, 1e-5)));
            // Only the two rims are sharp.
            assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), (2 * s) as usize);
            // Cap faces keep a reasonable minimum angle (no slivers).
            for f in m.group("cap_bottom").unwrap().iter() {
                for h in m.face_halfedges(f) {
                    let a = m.corner_angle(h).to_degrees();
                    assert!(a > 40.0, "{s} segments: corner angle {a}");
                }
            }
        }
    }

    #[test]
    fn ngon_fan_and_open() {
        let ngon = cylinder(&CylinderSpec { segments: 7, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Center).unwrap();
        check_closed(&ngon);
        assert_eq!(ngon.face_count(), 9);
        let fan = cylinder(&CylinderSpec { segments: 7, cap: CapKind::Fan, ..CylinderSpec::default() }, Origin::Center).unwrap();
        check_closed(&fan);
        assert_eq!(fan.face_count(), 7 + 14);
        let open = cylinder(&CylinderSpec { segments: 7, cap: CapKind::None, ..CylinderSpec::default() }, Origin::Center).unwrap();
        open.validate().unwrap();
        assert_eq!(open.edges().filter(|&e| open.is_boundary_edge(e)).count(), 14);
        assert!(open.group("cap_top").is_none());
    }

    #[test]
    fn front_vertex_and_mirror_symmetry() {
        for s in [5, 6, 7, 8] {
            let m = cylinder(&CylinderSpec { segments: s, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Bottom).unwrap();
            assert!(m.verts().any(|v| m.positions[v as usize].abs_diff_eq(Vec3::new(0.0, 0.0, 0.5), 1e-6)), "{s}");
            for v in m.verts() {
                let p = m.positions[v as usize];
                let mirrored = Vec3::new(-p.x, p.y, p.z);
                assert!(m.verts().any(|w| m.positions[w as usize].abs_diff_eq(mirrored, 1e-5)), "{s}: {p} has no mirror");
            }
        }
    }

    #[test]
    fn rejects_bad_params() {
        let err = cylinder(&CylinderSpec { segments: 10, ..CylinderSpec::default() }, Origin::Center).unwrap_err();
        assert!(err.to_string().contains("try 8 or 12"), "{err}");
        assert!(cylinder(&CylinderSpec { segments: 2, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Center).is_err());
        assert!(cylinder(&CylinderSpec { radius: 0.0, ..CylinderSpec::default() }, Origin::Center).is_err());
    }

    #[test]
    fn pointed_cones_have_an_apex_and_one_cap() {
        for cap in [CapKind::QuadGrid, CapKind::Ngon, CapKind::Fan] {
            for rows in [1, 3] {
                let spec = ConeSpec { radius_bottom: 0.4, radius_top: 0.0, height: 1.0, segments: 8, height_segments: rows, cap };
                let m = cone(&spec, Origin::Bottom).unwrap();
                check_closed(&m);
                assert!(m.group("cap_top").is_none() && m.group("cap_bottom").is_some(), "{cap:?}");
                let apex: Vec<u32> = m.verts().filter(|&v| m.positions[v as usize].abs_diff_eq(Vec3::Y, 1e-6)).collect();
                assert_eq!(apex.len(), 1);
                assert_eq!(m.valence(apex[0]), 8);
                // Triangles only around the apex.
                let side = m.group("side").unwrap();
                assert_eq!(side.iter().filter(|&f| m.face_len(f) == 3).count(), 8);
                assert_eq!(side.count(), (8 * rows) as usize);
            }
        }
        // Upside down: the apex at the bottom.
        let m = cone(&ConeSpec { radius_bottom: 0.0, radius_top: 0.3, cap: CapKind::Ngon, ..ConeSpec::default() }, Origin::Bottom).unwrap();
        check_closed(&m);
        assert!(m.group("cap_bottom").is_none() && m.group("cap_top").is_some());
    }

    #[test]
    fn frustum_radii_and_sharp_rims() {
        let spec = ConeSpec { radius_bottom: 0.5, radius_top: 0.25, height: 2.0, segments: 16, height_segments: 2, cap: CapKind::QuadGrid };
        let m = cone(&spec, Origin::Center).unwrap();
        check_closed(&m);
        assert!(m.faces().all(|f| m.face_len(f) == 4));
        for v in m.verts() {
            let p = m.positions[v as usize];
            let r = Vec3::new(p.x, 0.0, p.z).length();
            let expect = 0.375 - 0.125 * p.y;
            assert!(r <= expect + 1e-5, "vertex {v} outside the frustum");
        }
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 32);
        let open = cone(&ConeSpec { cap: CapKind::None, ..spec }, Origin::Center).unwrap();
        open.validate().unwrap();
        assert!(open.group("cap_top").is_none());
    }

    #[test]
    fn cone_rejects_two_zero_radii() {
        let e = cone(&ConeSpec { radius_bottom: 0.0, radius_top: 0.0, ..ConeSpec::default() }, Origin::Center).unwrap_err();
        assert!(e.to_string().contains("at most one radius"), "{e}");
        assert!(cone(&ConeSpec { radius_top: 1e-6, ..ConeSpec::default() }, Origin::Center).is_err(), "tiny nonzero radius");
    }
}
