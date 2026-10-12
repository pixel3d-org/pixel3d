//! Lathe: a profile revolved around an axis.

use glam::{DVec2, DVec3, Vec3};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};

use super::{MAX_GEN_FACES, invalid, strip};
use crate::edit::Axis;
use crate::prim::soup::{Soup, tag_groups_and_sharp};
use crate::prim::{CapKind, MAX_SEGMENTS, PrimError, quad_grid_cap};

/// Profile joints that turn more than this many degrees get sharp rings.
pub const CORNER_DEG: f64 = 30.0;
/// Most profile points.
pub const MAX_PROFILE_POINTS: usize = 100_000;

const CAP_BOTTOM: u16 = 0;
const CAP_TOP: u16 = 1;
const SPAN0: u16 = 2;

/// Lathe parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct LatheSpec<'a> {
    /// Profile points `(radius, height)`. Radii are at least 0; only the
    /// ends of an open profile may sit on the axis (they become poles).
    pub profile: &'a [DVec2],
    /// The span (group `profile_<span>`) of each profile segment, counting up
    /// along the profile; every segment is its own span when empty.
    pub spans: &'a [u32],
    /// Joins the last profile point back to the first (a ring, no caps).
    pub closed: bool,
    /// Segments around the axis.
    pub segments: u32,
    /// Revolution in radians, up to a full turn. Partial turns start at the
    /// front (+Z) and leave their ends open.
    pub angle: f64,
    /// How the open ends of a full revolution are filled.
    pub cap: CapKind,
    /// The axis to revolve around (the profile's height runs along it).
    pub axis: Axis,
}

/// Twice the signed area of the profile's cross section: the profile closed
/// through the axis (open) or by itself (closed). Positive when the profile
/// runs counter-clockwise in the (radius, height) plane, which puts the
/// outside on its right.
///
/// Complexity: O(points).
fn signed_area(pts: &[DVec2], closed: bool) -> f64 {
    let mut ring: Vec<DVec2> = pts.to_vec();
    if !closed && let (Some(&first), Some(&last)) = (pts.first(), pts.last()) {
        ring.push(DVec2::new(0.0, last.y));
        ring.push(DVec2::new(0.0, first.y));
    }
    (0..ring.len()).map(|i| ring[i].perp_dot(ring[(i + 1) % ring.len()])).sum()
}

/// Revolves a profile around the Y axis (then turns Y onto `axis`). Groups:
/// `side` (every revolved face), `cap_top` and `cap_bottom` for filled ends,
/// and `profile_<i>` per span. The profile may run either way; faces always
/// point out of the solid it bounds. Rings at profile corners (turning more
/// than [`CORNER_DEG`]) and cap rims are sharp.
///
/// Complexity: O(points x segments + segments^2 / 16) for quad-grid caps.
pub fn lathe(spec: &LatheSpec) -> Result<Mesh, PrimError> {
    let n = spec.profile.len();
    let min = if spec.closed { 3 } else { 2 };
    if n < min || n > MAX_PROFILE_POINTS {
        return Err(invalid("profile", format!("{n} points"), format!("needs {min} to {MAX_PROFILE_POINTS} points")));
    }
    let segs = if spec.closed { n } else { n - 1 };
    let spans: Vec<u32> = if spec.spans.is_empty() { (0..segs as u32).collect() } else { spec.spans.to_vec() };
    if spans.len() != segs {
        return Err(invalid("spans", spans.len(), format!("needs one span per profile segment ({segs})")));
    }
    for (i, p) in spec.profile.iter().enumerate() {
        let end = !spec.closed && (i == 0 || i + 1 == n);
        if !p.is_finite() || p.x < 0.0 || (p.x == 0.0 && !end) {
            let reason = if end { "radii must be 0 or more" } else { "radii must be above 0 (only the ends of an open profile may sit on the axis)" };
            return Err(invalid("profile", format!("point {i} ({}, {})", p.x, p.y), reason));
        }
        let q = spec.profile[(i + 1) % n];
        if (i + 1 < n || spec.closed) && p.distance(q) <= 1e-9 {
            return Err(invalid("profile", format!("point {i}"), format!("coincides with point {}", (i + 1) % n)));
        }
    }
    let s = spec.segments;
    if !(3..=MAX_SEGMENTS).contains(&s) {
        return Err(invalid("segments", s, format!("must be between 3 and {MAX_SEGMENTS}")));
    }
    let tau = std::f64::consts::TAU;
    if !(spec.angle.is_finite() && spec.angle > 1e-6 && spec.angle <= tau + 1e-9) {
        return Err(invalid("angle", format!("{:.3} rad", spec.angle), "must be above 0 and at most a full turn"));
    }
    if segs.saturating_mul(s as usize) > MAX_GEN_FACES {
        return Err(invalid("segments", s, format!("{segs} profile segments x {s} would exceed {MAX_GEN_FACES} faces")));
    }
    let full = spec.angle >= tau - 1e-9;
    let mut pts = spec.profile.to_vec();
    let mut spans = spans;
    if signed_area(&pts, spec.closed) < 0.0 {
        pts.reverse();
        if spec.closed {
            // Segment j of the reversed ring is old segment n - 2 - j backwards; the last stays.
            let last = spans[n - 1];
            spans.truncate(n - 1);
            spans.reverse();
            spans.push(last);
        } else {
            spans.reverse();
        }
    }
    let caps_needed = full && !spec.closed && spec.cap != CapKind::None && (pts[0].x > 0.0 || pts[n - 1].x > 0.0);
    if caps_needed && spec.cap == CapKind::QuadGrid && !s.is_multiple_of(4) {
        let lower = (s / 4).max(1) * 4;
        return Err(invalid("segments", s, format!("quad_grid caps need a multiple of 4 (try {lower} or {})", lower + 4)));
    }

    // Quad-grid caps put grid corners on ring vertices at 45 + 90k degrees;
    // otherwise vertex 0 is at the front (+Z).
    let phase = if caps_needed && spec.cap == CapKind::QuadGrid { std::f64::consts::FRAC_PI_4 } else { std::f64::consts::FRAC_PI_2 };
    let cols = if full { s } else { s + 1 } as usize;
    let dirs: Vec<DVec3> = (0..cols)
        .map(|k| {
            let a = phase + spec.angle * k as f64 / f64::from(s);
            DVec3::new(a.cos(), 0.0, a.sin())
        })
        .collect();
    let mut soup = Soup::with_capacity(n * cols, segs * s as usize + 2, 4 * segs * s as usize);
    let rows: Vec<Vec<u32>> =
        pts.iter()
            .map(|p| {
                if p.x == 0.0 {
                    vec![soup.push_vert(DVec3::Y * p.y)]
                } else {
                    dirs.iter().map(|d| soup.push_vert(*d * p.x + DVec3::Y * p.y)).collect()
                }
            })
            .collect();
    for j in 0..segs {
        let tag = SPAN0 as u32 + spans[j];
        let tag = u16::try_from(tag).map_err(|_| invalid("spans", spans[j], "at most 65,000 spans"))?;
        strip(&mut soup, &rows[j], &rows[(j + 1) % n], full, tag);
    }
    let mut has_cap = [false; 2];
    if caps_needed {
        // The cross section runs counter-clockwise, so its first point is the bottom end.
        for (end, tag, up) in [(0, CAP_BOTTOM, -DVec3::Y), (n - 1, CAP_TOP, DVec3::Y)] {
            let p = pts[end];
            if p.x == 0.0 {
                continue;
            }
            has_cap[tag as usize] = true;
            let rim = &rows[end];
            match spec.cap {
                CapKind::None => {}
                CapKind::Ngon => soup.poly_facing(rim, up, tag),
                CapKind::Fan => {
                    let c = soup.push_vert(DVec3::Y * p.y);
                    for k in 0..rim.len() {
                        soup.poly_facing(&[c, rim[k], rim[(k + 1) % rim.len()]], up, tag);
                    }
                }
                CapKind::QuadGrid => quad_grid_cap(&mut soup, rim, &dirs, p.x, tag, |x, z| DVec3::new(x, p.y, z), |_| up),
            }
        }
    }

    let (mut mesh, tags) = soup.finish()?;
    // Groups in a fixed order: side, caps, spans.
    let faces_with = |pred: &dyn Fn(u16) -> bool| -> Vec<u32> { tags.iter().enumerate().filter(|(_, t)| pred(**t)).map(|(f, _)| f as u32).collect() };
    let mut groups: Vec<(String, Vec<u32>)> = vec![("side".into(), faces_with(&|t| t >= SPAN0))];
    if has_cap[CAP_TOP as usize] {
        groups.push(("cap_top".into(), faces_with(&|t| t == CAP_TOP)));
    }
    if has_cap[CAP_BOTTOM as usize] {
        groups.push(("cap_bottom".into(), faces_with(&|t| t == CAP_BOTTOM)));
    }
    let mut by_span: Vec<Vec<u32>> = Vec::new();
    for (f, &t) in tags.iter().enumerate() {
        if t >= SPAN0 {
            let i = usize::from(t - SPAN0);
            if by_span.len() <= i {
                by_span.resize(i + 1, Vec::new());
            }
            by_span[i].push(f as u32);
        }
    }
    groups.extend(by_span.into_iter().enumerate().filter(|(_, fs)| !fs.is_empty()).map(|(i, fs)| (format!("profile_{i}"), fs)));
    for (name, faces) in groups {
        // Faces of a fresh mesh are all live, so this cannot fail.
        let _ = mesh.set_group(&name, ElementKind::Face, faces);
    }
    // Cap rims are sharp.
    tag_groups_and_sharp(&mut mesh, &tags, &[], |a, b| (a < SPAN0) != (b < SPAN0));
    // So are rings at profile corners.
    let corner_cos = CORNER_DEG.to_radians().cos();
    for i in 0..n {
        if (!spec.closed && (i == 0 || i + 1 == n)) || rows[i].len() == 1 {
            continue;
        }
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        if (b - a).normalize().dot((c - b).normalize()) >= corner_cos {
            continue;
        }
        let ring = &rows[i];
        let steps = if full { ring.len() } else { ring.len() - 1 };
        for k in 0..steps {
            if let Some(h) = mesh.find_halfedge(ring[k], ring[(k + 1) % ring.len()]) {
                let e = mesh.edge_of(h);
                mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
            }
        }
    }
    turn_axis(&mut mesh, spec.axis);
    Ok(mesh)
}

/// Turns a mesh built around +Y so that +Y lands on `axis` (a proper
/// rotation, so winding is kept).
///
/// Complexity: O(vertices).
pub(crate) fn turn_axis(mesh: &mut Mesh, axis: Axis) {
    let map: fn(Vec3) -> Vec3 = match axis {
        Axis::Y => return,
        // Quarter turn about -Z: +Y to +X.
        Axis::X => |p| Vec3::new(p.y, -p.x, p.z),
        // Quarter turn about +X: +Y to +Z.
        Axis::Z => |p| Vec3::new(p.x, -p.z, p.y),
    };
    for p in &mut mesh.positions {
        *p = map(*p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(profile: &[DVec2]) -> LatheSpec<'_> {
        LatheSpec { profile, spans: &[], closed: false, segments: 16, angle: std::f64::consts::TAU, cap: CapKind::QuadGrid, axis: Axis::Y }
    }

    fn closed_outward(m: &Mesh) {
        m.validate().unwrap();
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
        let (lo, hi) = m.bounds().unwrap();
        let c = (lo + hi) / 2.0;
        let mut vol = 0.0f32;
        for f in m.faces() {
            vol += m.face_normal(f).dot(m.face_centroid(f) - c) * m.face_area(f);
        }
        assert!(vol > 0.0, "faces point inward");
    }

    #[test]
    fn barrel_profile_with_caps_either_direction() {
        let up = [DVec2::new(0.25, 0.0), DVec2::new(0.3, 0.45), DVec2::new(0.25, 0.9)];
        let down: Vec<DVec2> = up.iter().rev().copied().collect();
        for p in [&up[..], &down[..]] {
            let m = lathe(&spec(p)).unwrap();
            closed_outward(&m);
            assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only");
            let names: Vec<&str> = m.groups().map(|(n, _)| n).collect();
            assert_eq!(names, ["side", "cap_top", "cap_bottom", "profile_0", "profile_1"]);
            assert_eq!(m.group("side").unwrap().count(), 32);
            assert!(m.group("cap_top").unwrap().iter().all(|f| m.face_normal(f).abs_diff_eq(Vec3::Y, 1e-5)));
            // Cap rims are sharp; the bulge joint turns about 25 degrees and stays smooth.
            let sharp = m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count();
            assert_eq!(sharp, 32);
            let (lo, hi) = m.bounds().unwrap();
            assert!((hi.y - 0.9).abs() < 1e-6 && lo.y.abs() < 1e-6);
        }
    }

    #[test]
    fn poles_corners_and_partial_turns() {
        // A cup: closed bottom on the axis, a sharp corner at the base edge.
        let cup = [DVec2::new(0.0, 0.0), DVec2::new(0.4, 0.0), DVec2::new(0.4, 0.5)];
        let mut sp = spec(&cup);
        sp.segments = 12;
        let m = lathe(&sp).unwrap();
        m.validate().unwrap();
        // Open top rim, no cap there either since cap_top needs quad grid: 12 is a multiple of 4.
        assert_eq!(m.faces().filter(|&f| m.face_len(f) == 3).count(), 12, "triangles at the pole");
        assert_eq!(m.group("cap_top").unwrap().count(), m.faces().filter(|&f| m.face_normal(f).abs_diff_eq(Vec3::Y, 1e-5)).count());
        let sharp = m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count();
        assert_eq!(sharp, 12 + 12, "the base corner ring and the top cap rim");
        // Half a turn leaves open seams and skips caps.
        sp.angle = std::f64::consts::PI;
        sp.cap = CapKind::Ngon;
        let half = lathe(&sp).unwrap();
        half.validate().unwrap();
        assert!(half.group("cap_top").is_none());
        assert_eq!(half.face_count(), 12 * 2);
        assert!(half.positions.iter().all(|p| p.x <= 1e-6), "the half turn covers -X");
    }

    #[test]
    fn closed_profiles_make_rings_and_axes_turn() {
        // A square section ring (a washer).
        let sq = [DVec2::new(0.3, 0.0), DVec2::new(0.5, 0.0), DVec2::new(0.5, 0.2), DVec2::new(0.3, 0.2)];
        for (pts, axis) in [(sq.to_vec(), Axis::Y), (sq.iter().rev().copied().collect::<Vec<_>>(), Axis::Z)] {
            let m = lathe(&LatheSpec { closed: true, axis, ..spec(&pts) }).unwrap();
            closed_outward(&m);
            assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 0, "torus topology");
            assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 4 * 16);
            let (lo, hi) = m.bounds().unwrap();
            let height = (hi - lo)[if axis == Axis::Z { 2 } else { 1 }];
            assert!((height - 0.2).abs() < 1e-6, "{axis:?}: {lo} {hi}");
        }
    }

    #[test]
    fn spans_group_sampled_segments_and_bad_profiles_fail() {
        let pts = [DVec2::new(0.2, 0.0), DVec2::new(0.3, 0.1), DVec2::new(0.3, 0.2), DVec2::new(0.2, 0.3)];
        let m = lathe(&LatheSpec { spans: &[0, 0, 1], cap: CapKind::Fan, segments: 7, ..spec(&pts) }).unwrap();
        closed_outward(&m);
        assert_eq!((m.group("profile_0").unwrap().count(), m.group("profile_1").unwrap().count()), (14, 7));
        let bad = |p: &[DVec2]| lathe(&spec(p)).unwrap_err();
        assert!(matches!(bad(&[DVec2::new(0.2, 0.0)]), PrimError::InvalidParam { param: "profile", .. }));
        assert!(matches!(bad(&[DVec2::new(0.2, 0.0), DVec2::new(0.0, 0.5), DVec2::new(0.2, 1.0)]), PrimError::InvalidParam { param: "profile", .. }));
        assert!(matches!(bad(&[DVec2::new(-0.2, 0.0), DVec2::new(0.2, 1.0)]), PrimError::InvalidParam { param: "profile", .. }));
        assert!(matches!(lathe(&LatheSpec { segments: 10, ..spec(&pts) }), Err(PrimError::InvalidParam { param: "segments", .. })));
        assert!(lathe(&LatheSpec { segments: 10, cap: CapKind::None, ..spec(&pts) }).is_ok());
    }
}
