//! Curve kernels on polylines and cubic Bezier splines: sampling, arc-length
//! resampling, planar offsets and fillets. Generators (lathe, sweep, loft,
//! pipe) consume the sampled polylines.
//!
//! A Bezier spline stores one `[in, out]` handle pair per point, as absolute
//! positions. Span `i` runs from point `i` (out handle) to point `i + 1` (in
//! handle); a closed spline has a last span back to point 0. Closed
//! polylines never repeat their first point.

use glam::Vec3;

use crate::edit::KernelError;

/// Most segments per Bezier span when sampling.
pub const MAX_RESOLUTION: u32 = 64;
/// Most segments per fillet arc.
pub const MAX_FILLET_SEGMENTS: u32 = 32;
/// Most points a curve kernel may produce.
pub const MAX_POINTS: usize = 1_000_000;

fn out_of_range(param: &'static str, message: String, hint: &str) -> KernelError {
    KernelError::OutOfRange { param, message, hint: hint.to_owned() }
}

fn check_points(n: usize) -> Result<(), KernelError> {
    if n <= MAX_POINTS {
        Ok(())
    } else {
        Err(out_of_range("points", format!("the result would have {n} points; the limit is {MAX_POINTS}"), "use fewer segments or points"))
    }
}

/// The point of a cubic Bezier span at `t` in 0..=1.
pub fn bezier_point(p0: Vec3, c0: Vec3, c1: Vec3, p1: Vec3, t: f32) -> Vec3 {
    let s = 1.0 - t;
    p0 * (s * s * s) + c0 * (3.0 * s * s * t) + c1 * (3.0 * s * t * t) + p1 * (t * t * t)
}

/// Whether a span is a straight segment: both handles lie on the chord
/// between its ends.
fn is_straight(p0: Vec3, c0: Vec3, c1: Vec3, p1: Vec3) -> bool {
    let d = p1 - p0;
    let len2 = d.length_squared();
    if len2 <= f32::MIN_POSITIVE {
        return c0.distance_squared(p0) <= f32::MIN_POSITIVE && c1.distance_squared(p0) <= f32::MIN_POSITIVE;
    }
    let on_chord = |q: Vec3| {
        let t = (q - p0).dot(d) / len2;
        (-1e-4..=1.0 + 1e-4).contains(&t) && (p0 + d * t).distance_squared(q) <= len2 * 1e-8
    };
    on_chord(c0) && on_chord(c1)
}

/// Smooth handles through the points: each point's tangent bisects its two
/// chords and each handle reaches a third of the chord on its side, so
/// uneven spacing does not overshoot. Open ends point at their neighbour.
///
/// Complexity: O(points).
pub fn auto_handles(points: &[Vec3], closed: bool) -> Vec<[Vec3; 2]> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let p = points[i];
            let prev = if i > 0 {
                Some(points[i - 1])
            } else if closed && n > 2 {
                Some(points[n - 1])
            } else {
                None
            };
            let next = if i + 1 < n {
                Some(points[i + 1])
            } else if closed && n > 2 {
                Some(points[0])
            } else {
                None
            };
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let (la, lb) = (p.distance(a), b.distance(p));
                    let t = ((p - a).normalize_or_zero() + (b - p).normalize_or_zero()).normalize_or_zero();
                    [p - t * (la / 3.0), p + t * (lb / 3.0)]
                }
                (None, Some(b)) => {
                    let h = (b - p) / 3.0;
                    [p - h, p + h]
                }
                (Some(a), None) => {
                    let h = (p - a) / 3.0;
                    [p - h, p + h]
                }
                (None, None) => [p, p],
            }
        })
        .collect()
}

/// A Bezier spline sampled to a polyline: `resolution` segments per curved
/// span and one per straight span. A closed spline does not repeat its
/// first point. Expects one handle pair per point.
///
/// Complexity: O(points x resolution).
pub fn sample_bezier(points: &[Vec3], handles: &[[Vec3; 2]], closed: bool, resolution: u32) -> Result<Vec<Vec3>, KernelError> {
    if !(1..=MAX_RESOLUTION).contains(&resolution) {
        return Err(out_of_range("resolution", format!("{resolution} is outside 1 to {MAX_RESOLUTION}"), "12 suits most curves"));
    }
    if handles.len() != points.len() {
        return Err(out_of_range(
            "handles",
            format!("{} handle pairs for {} points", handles.len(), points.len()),
            "give one [in, out] pair per point",
        ));
    }
    let n = points.len();
    let spans = match (n, closed) {
        (0 | 1, _) => 0,
        (n, true) => n,
        (n, false) => n - 1,
    };
    check_points(spans.saturating_mul(resolution as usize).saturating_add(1))?;
    let mut out = Vec::with_capacity(spans * resolution as usize + 1);
    if let Some(&p) = points.first() {
        out.push(p);
    }
    for i in 0..spans {
        let j = (i + 1) % n;
        let (p0, c0, c1, p1) = (points[i], handles[i][1], handles[j][0], points[j]);
        if !is_straight(p0, c0, c1, p1) {
            for k in 1..resolution {
                out.push(bezier_point(p0, c0, c1, p1, k as f32 / resolution as f32));
            }
        }
        if !(closed && j == 0) {
            out.push(p1);
        }
    }
    Ok(out)
}

/// Total length of a polyline.
///
/// Complexity: O(points).
pub fn length(points: &[Vec3], closed: bool) -> f32 {
    let open: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    match (closed, points.first(), points.last()) {
        (true, Some(&a), Some(&b)) if points.len() > 2 => open + a.distance(b),
        _ => open,
    }
}

/// Points without consecutive duplicates (closer than `eps`); a closed
/// polyline also drops a last point equal to its first.
///
/// Complexity: O(points).
pub fn dedup(points: &[Vec3], closed: bool, eps: f32) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|&q| q.distance(p) > eps) {
            out.push(p);
        }
    }
    while closed && out.len() > 1 && out[0].distance(out[out.len() - 1]) <= eps {
        out.pop();
    }
    out
}

fn too_short(what: &str) -> KernelError {
    KernelError::Empty(format!("the {what} has fewer than two distinct points"))
}

/// `count` points evenly spaced by arc length. An open polyline keeps both
/// ends; a closed one keeps its first point and spreads `count` points
/// around the loop. Sharp corners between samples are cut.
///
/// Complexity: O(points + count).
pub fn resample(points: &[Vec3], closed: bool, count: usize) -> Result<Vec<Vec3>, KernelError> {
    let pts = dedup(points, closed, 1e-7);
    if pts.len() < 2 {
        return Err(too_short("curve"));
    }
    let min = if closed { 3 } else { 2 };
    if count < min {
        return Err(out_of_range("count", format!("{count} points are too few for an {} curve", if closed { "closed" } else { "open" }), ""));
    }
    check_points(count)?;
    let total = length(&pts, closed);
    let segs = if closed { count } else { count - 1 };
    let step = total / segs as f32;
    let n = pts.len();
    let seg_count = if closed { n } else { n - 1 };
    let mut out = Vec::with_capacity(count);
    out.push(pts[0]);
    // Walk the segments once, emitting every sample that falls inside.
    let (mut seg, mut seg_start) = (0usize, 0.0f32);
    for k in 1..segs {
        let target = step * k as f32;
        loop {
            let (a, b) = (pts[seg], pts[(seg + 1) % n]);
            let len = a.distance(b);
            if target <= seg_start + len || seg + 1 >= seg_count {
                let t = if len > 0.0 { ((target - seg_start) / len).clamp(0.0, 1.0) } else { 0.0 };
                out.push(a.lerp(b, t));
                break;
            }
            seg_start += len;
            seg += 1;
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    Ok(out)
}

/// The normal of a curve's plane by Newell's method, or `None` for a
/// straight or degenerate curve. An open curve's normal points to the
/// positive side of its dominant axis so the result does not depend on
/// which way the curve runs; a closed curve's normal sees it
/// counter-clockwise.
///
/// Complexity: O(points).
pub fn plane_normal(points: &[Vec3], closed: bool) -> Option<Vec3> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    let c = points.iter().copied().sum::<Vec3>() / n as f32;
    let mut sum = Vec3::ZERO;
    for i in 0..n {
        let (a, b) = (points[i] - c, points[(i + 1) % n] - c);
        sum += a.cross(b);
    }
    let extent = points.iter().map(|p| p.distance(c)).fold(0.0f32, f32::max);
    if sum.length() <= extent * extent * 1e-6 {
        return None;
    }
    let mut normal = sum.normalize();
    if !closed {
        let i = normal.abs().max_position();
        if normal[i] < 0.0 {
            normal = -normal;
        }
    }
    Some(normal)
}

/// How far the farthest point lies from the plane through the centroid.
///
/// Complexity: O(points).
pub fn plane_deviation(points: &[Vec3], normal: Vec3) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let c = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    points.iter().map(|p| (*p - c).dot(normal).abs()).fold(0.0, f32::max)
}

/// A default plane normal for a straight line: +Y (the ground plane)
/// unless the line runs along Y, then +Z.
fn line_normal(dir: Vec3) -> Vec3 {
    let up = if dir.normalize_or_zero().y.abs() > 0.9 { Vec3::Z } else { Vec3::Y };
    (up - dir * (up.dot(dir) / dir.length_squared().max(f32::MIN_POSITIVE))).normalize_or(up)
}

/// Offset of a polyline within planes perpendicular to `normal` (the
/// curve's own plane when `None`). A closed curve grows outward for a
/// positive distance; an open curve moves to its left when seen from the
/// normal's tip. Corners are mitered; convex corners whose miter exceeds
/// `miter_limit` times the distance are beveled with two points. The
/// offset of a concave corner deeper than the curve allows overlaps itself.
///
/// Complexity: O(points).
pub fn offset(points: &[Vec3], closed: bool, distance: f32, normal: Option<Vec3>, miter_limit: f32) -> Result<Vec<Vec3>, KernelError> {
    if !distance.is_finite() {
        return Err(out_of_range("distance", "distance must be finite".into(), ""));
    }
    let pts = dedup(points, closed, 1e-7);
    if pts.len() < 2 {
        return Err(too_short("curve"));
    }
    let n = match normal {
        Some(v) if v.length_squared() > 0.0 && v.is_finite() => v.normalize(),
        Some(_) => return Err(out_of_range("normal", "normal must be a nonzero vector".into(), "for example [0, 1, 0]")),
        None => match plane_normal(&pts, closed) {
            Some(v) => {
                let dev = plane_deviation(&pts, v);
                let extent = pts.iter().map(|p| p.distance(pts[0])).fold(0.0f32, f32::max);
                if dev > extent.max(1e-3) * 1e-4 {
                    return Err(out_of_range(
                        "normal",
                        format!("the curve is not planar (a point is {dev:.4} m off its plane)"),
                        "pass normal to offset within planes perpendicular to it",
                    ));
                }
                v
            }
            None => line_normal(pts[pts.len() - 1] - pts[0]),
        },
    };
    let count = pts.len();
    let seg_count = if closed { count } else { count - 1 };
    // Left normal of each segment within the plane.
    let mut left = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let d = pts[(i + 1) % count] - pts[i];
        let l = n.cross(d).normalize_or_zero();
        if l == Vec3::ZERO {
            return Err(out_of_range("normal", format!("segment {i} runs along the normal"), "pass a normal perpendicular to the curve"));
        }
        left.push(l);
    }
    // A closed curve counter-clockwise around n has its outside on the right.
    let d = if closed && signed_area(&pts, n) > 0.0 { -distance } else { distance };
    let mut out = Vec::with_capacity(count + count / 2);
    for i in 0..count {
        let p = pts[i];
        let prev = if i > 0 {
            Some(left[i - 1])
        } else if closed {
            Some(left[seg_count - 1])
        } else {
            None
        };
        let next = if i < seg_count { Some(left[i]) } else { None };
        match (prev, next) {
            (Some(la), Some(lb)) => {
                let m = (la + lb).normalize_or_zero();
                let cos = m.dot(la);
                // Positive turn: the curve bends to its left, so the left side is inside.
                let turn = n.dot(la.cross(lb));
                let outer = turn * d < 0.0;
                if m == Vec3::ZERO || (outer && cos < 1.0 / miter_limit.max(1.0)) {
                    out.push(p + la * d);
                    out.push(p + lb * d);
                } else {
                    out.push(p + m * (d / cos));
                }
            }
            (Some(l), None) | (None, Some(l)) => out.push(p + l * d),
            (None, None) => out.push(p),
        }
    }
    Ok(out)
}

/// Twice the signed area around `n` (positive when counter-clockwise).
fn signed_area(points: &[Vec3], n: Vec3) -> f32 {
    let c = points[0];
    (1..points.len().saturating_sub(1)).map(|i| n.dot((points[i] - c).cross(points[i + 1] - c))).sum()
}

/// Rounds every corner with an arc of `radius` (shrunk where the segments
/// are too short) made of `segments` segments; one segment is a chamfer.
/// Straight joints and cusps stay as they are. Returns the new points.
///
/// Complexity: O(points x segments).
pub fn fillet(points: &[Vec3], closed: bool, radius: f32, segments: u32) -> Result<Vec<Vec3>, KernelError> {
    if !(radius.is_finite() && radius > 0.0) {
        return Err(out_of_range("radius", "radius must be positive".into(), ""));
    }
    if !(1..=MAX_FILLET_SEGMENTS).contains(&segments) {
        return Err(out_of_range("segments", format!("{segments} is outside 1 to {MAX_FILLET_SEGMENTS}"), "4 reads round on most props"));
    }
    let pts = dedup(points, closed, 1e-7);
    if pts.len() < 2 {
        return Err(too_short("curve"));
    }
    let n = pts.len();
    check_points(n.saturating_mul(segments as usize + 1))?;
    let mut out = Vec::with_capacity(n * (segments as usize + 1));
    for i in 0..n {
        let p = pts[i];
        let corner = closed || (i > 0 && i + 1 < n);
        if !corner || n < 3 {
            out.push(p);
            continue;
        }
        let (a, b) = (pts[(i + n - 1) % n], pts[(i + 1) % n]);
        let (da, db) = (p - a, b - p);
        let (la, lb) = (da.length(), db.length());
        let (ua, ub) = (da / la, db / lb);
        let theta = ua.dot(ub).clamp(-1.0, 1.0).acos();
        if theta < 1e-3 || theta > std::f32::consts::PI - 1e-3 {
            out.push(p);
            continue;
        }
        // An open curve's end segments belong to one corner only.
        let room_a = if !closed && i == 1 { la } else { la * 0.5 };
        let room_b = if !closed && i + 2 == n { lb } else { lb * 0.5 };
        let half = (theta * 0.5).tan();
        let t = (radius * half).min(room_a).min(room_b);
        let r = t / half;
        let (q0, q1) = (p - ua * t, p + ub * t);
        let center = p + (ub - ua).normalize() * (r / (theta * 0.5).cos());
        let (v0, v1) = (q0 - center, q1 - center);
        let sin = theta.sin();
        for k in 0..=segments {
            let s = k as f32 / segments as f32;
            let w0 = ((1.0 - s) * theta).sin() / sin;
            let w1 = (s * theta).sin() / sin;
            out.push(center + v0 * w0 + v1 * w1);
        }
    }
    Ok(out)
}

/// A plane for flat curves: two in-plane axes `u`, `v` with `u x v` the
/// plane's normal, so increasing angle runs counter-clockwise seen from
/// the normal's tip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Plane {
    /// The front plane (normal +Z).
    Xy,
    /// The ground plane (normal +Y).
    #[default]
    Xz,
    /// The side plane (normal +X).
    Yz,
}

impl Plane {
    /// In-plane axes `(u, v)`.
    pub fn axes(self) -> (Vec3, Vec3) {
        match self {
            Plane::Xy => (Vec3::X, Vec3::Y),
            Plane::Xz => (Vec3::X, Vec3::NEG_Z),
            Plane::Yz => (Vec3::Y, Vec3::Z),
        }
    }

    pub fn normal(self) -> Vec3 {
        let (u, v) = self.axes();
        u.cross(v)
    }
}

/// A closed regular polygon approximating a circle, starting on `u` and
/// running counter-clockwise around the plane's normal.
///
/// Complexity: O(segments).
pub fn circle(radius: f32, segments: u32, plane: Plane) -> Vec<Vec3> {
    let (u, v) = plane.axes();
    (0..segments)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / segments as f32;
            (u * a.cos() + v * a.sin()) * radius
        })
        .collect()
}

/// A closed rectangle centered on the origin, `size[0]` along `u` and
/// `size[1]` along `v`, counter-clockwise from the (-u, -v) corner, with
/// corners rounded by `corner_radius` in `corner_segments` segments when
/// the radius is positive.
///
/// Complexity: O(corner_segments).
pub fn rectangle(size: [f32; 2], corner_radius: f32, corner_segments: u32, plane: Plane) -> Result<Vec<Vec3>, KernelError> {
    let (u, v) = plane.axes();
    let (hx, hy) = (size[0] * 0.5, size[1] * 0.5);
    let pts = vec![u * -hx + v * -hy, u * hx + v * -hy, u * hx + v * hy, u * -hx + v * hy];
    if corner_radius > 0.0 { fillet(&pts, true, corner_radius, corner_segments) } else { Ok(pts) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-5
    }

    #[test]
    fn bezier_sampling_keeps_ends_and_straight_spans() {
        let pts = [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 1.0, 0.0)];
        // Span 0 is straight (handles on the chord), span 1 is curved.
        let handles = [
            [Vec3::ZERO, Vec3::new(0.3, 0.0, 0.0)],
            [Vec3::new(0.7, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)],
            [Vec3::new(2.0, 0.5, 0.0), Vec3::new(2.0, 1.5, 0.0)],
        ];
        let s = sample_bezier(&pts, &handles, false, 8).unwrap();
        assert_eq!(s.len(), 1 + 1 + 8);
        assert!(close(s[0], pts[0]) && close(s[1], pts[1]) && close(s[9], pts[2]));
        let closed = sample_bezier(&pts, &handles, true, 4).unwrap();
        // Spans 1 and 2 curve (4 points each), span 0 is straight; no repeat of point 0.
        assert_eq!(closed.len(), 1 + 1 + 4 + 3);
        assert!(sample_bezier(&pts, &handles[..2], false, 8).is_err());
        assert!(sample_bezier(&pts, &handles, false, 0).is_err());
    }

    #[test]
    fn auto_handles_pass_smoothly_through_points() {
        let pts: Vec<Vec3> = (0..5).map(|i| Vec3::new(i as f32, if i % 2 == 0 { 0.0 } else { 1.0 }, 0.0)).collect();
        let h = auto_handles(&pts, false);
        for (i, [a, b]) in h.iter().enumerate() {
            // Handles are collinear with the point (smooth joint).
            let (da, db) = (pts[i] - *a, *b - pts[i]);
            assert!(da.cross(db).length() < 1e-5, "joint {i} has a kink");
        }
        let s = sample_bezier(&pts, &h, false, 12).unwrap();
        assert!(s.iter().all(|p| p.y > -0.2 && p.y < 1.2), "no large overshoot");
    }

    #[test]
    fn resample_spreads_points_evenly() {
        let pts = [Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 3.0, 0.0)];
        let r = resample(&pts, false, 5).unwrap();
        assert_eq!(r.len(), 5);
        assert!(close(r[0], pts[0]) && close(r[4], pts[2]));
        for w in r.windows(2) {
            assert!((w[0].distance(w[1]) - 1.0).abs() < 1e-4 || w[0].x != w[1].x, "{w:?}");
        }
        let sq = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        let c = resample(&sq, true, 8).unwrap();
        assert_eq!(c.len(), 8);
        assert!(close(c[1], Vec3::new(0.5, 0.0, 0.0)) && close(c[7], Vec3::new(0.0, 0.5, 0.0)));
        assert!(resample(&sq, true, 2).is_err());
        assert!(resample(&[Vec3::ZERO, Vec3::ZERO], false, 4).is_err());
    }

    #[test]
    fn closed_offsets_grow_outward_either_winding() {
        let sq = rectangle([2.0, 2.0], 0.0, 1, Plane::Xy).unwrap();
        for pts in [sq.clone(), sq.iter().rev().copied().collect()] {
            let o = offset(&pts, true, 0.5, None, 4.0).unwrap();
            assert_eq!(o.len(), 4);
            assert!(o.iter().all(|p| (p.x.abs() - 1.5).abs() < 1e-5 && (p.y.abs() - 1.5).abs() < 1e-5), "{o:?}");
            let i = offset(&pts, true, -0.25, None, 4.0).unwrap();
            assert!(i.iter().all(|p| (p.x.abs() - 0.75).abs() < 1e-5), "{i:?}");
        }
    }

    #[test]
    fn open_offsets_move_left_and_bevel_sharp_corners() {
        let line = [Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)];
        // A straight line on the ground offsets within the ground plane (+Y normal): left of +X is -Z.
        let o = offset(&line, false, 0.1, None, 4.0).unwrap();
        assert!(close(o[0], Vec3::new(0.0, 0.0, -0.1)) && close(o[1], Vec3::new(2.0, 0.0, -0.1)), "{o:?}");
        // A hairpin turning right: the left (outer) side is beveled, the right (inner) side mitered.
        let pin = [Vec3::ZERO, Vec3::X, Vec3::new(0.0, -0.1, 0.0)];
        let outer = offset(&pin, false, 0.05, Some(Vec3::Z), 4.0).unwrap();
        assert_eq!(outer.len(), 4);
        let inner = offset(&pin, false, -0.05, Some(Vec3::Z), 4.0).unwrap();
        assert_eq!(inner.len(), 3);
        let bent = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0)];
        let twisted = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0)];
        assert!(offset(&bent, false, 0.1, None, 4.0).is_ok());
        assert!(matches!(offset(&twisted, false, 0.1, None, 4.0), Err(KernelError::OutOfRange { param: "normal", .. })));
        assert!(offset(&twisted, false, 0.1, Some(Vec3::new(0.0, 1.0, 1.0)), 4.0).is_ok());
        assert!(offset(&twisted, false, 0.1, Some(Vec3::Y), 4.0).is_err(), "a segment runs along the normal");
    }

    #[test]
    fn fillets_are_tangent_arcs_of_the_radius() {
        let sq = rectangle([2.0, 2.0], 0.0, 1, Plane::Xy).unwrap();
        let f = fillet(&sq, true, 0.25, 4).unwrap();
        assert_eq!(f.len(), 4 * 5);
        // Every arc point lies on a circle of radius 0.25 around its corner's inset center.
        for p in &f {
            let c = Vec3::new(0.75 * p.x.signum(), 0.75 * p.y.signum(), 0.0);
            assert!((p.distance(c) - 0.25).abs() < 1e-4, "{p} off its arc");
        }
        // A radius larger than the segments allow shrinks to fit.
        let big = fillet(&sq, true, 5.0, 2).unwrap();
        assert!(big.iter().all(|p| p.length() <= 1.0 + 1e-4));
        let line = [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 0.0, 0.0)];
        assert_eq!(fillet(&line, false, 0.1, 4).unwrap().len(), 3, "straight joints stay");
        let chamfer = fillet(&[Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0)], false, 0.2, 1).unwrap();
        assert_eq!(chamfer.len(), 4);
        assert!(close(chamfer[1], Vec3::new(0.8, 0.0, 0.0)) && close(chamfer[2], Vec3::new(1.0, 0.2, 0.0)));
    }

    #[test]
    fn circles_and_rectangles_run_counter_clockwise_around_their_plane() {
        for plane in [Plane::Xy, Plane::Xz, Plane::Yz] {
            let c = circle(0.5, 16, plane);
            assert_eq!(c.len(), 16);
            assert!(c.iter().all(|p| (p.length() - 0.5).abs() < 1e-5));
            assert!(signed_area(&c, plane.normal()) > 0.0);
            assert!((length(&c, true) - 16.0 * 2.0 * 0.5 * (std::f32::consts::PI / 16.0).sin()).abs() < 1e-4);
            assert_eq!(plane_normal(&c, true).map(|n| n.dot(plane.normal()) > 0.999), Some(true));
            let r = rectangle([1.0, 0.5], 0.1, 3, plane).unwrap();
            assert_eq!(r.len(), 16);
            assert!(signed_area(&r, plane.normal()) > 0.0);
        }
    }
}
