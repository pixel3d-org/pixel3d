//! Curve objects in the viewport. They have no faces, so they are drawn as
//! polylines over the scene (cubic Bezier spans sampled) and picked by
//! their distance on screen. The sampled world-space polylines are cached
//! per document state and hidden set; drawing projects them each frame.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};

use egui::{Pos2, Rect, Stroke, pos2};
use glam::{Mat4, Vec3};
use p3d_render::OrbitCamera;
use p3d_scene::{Document, ObjectKey, Spline};

/// Points per Bezier span.
const SEGMENTS: usize = 12;
/// How near (points) a click must be to pick a curve.
const PICK_RADIUS: f32 = 6.0;

/// One sampled spline of a curve object, in world space.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub object: ObjectKey,
    pub points: Vec<Vec3>,
    pub closed: bool,
}

/// Samples `spline` in object space: polylines as they are, Bezier spans
/// in [`SEGMENTS`] steps.
///
/// Complexity: O(points x SEGMENTS).
pub fn sample(spline: &Spline) -> Vec<Vec3> {
    let pts = spline.positions();
    let Some(handles) = &spline.handles else { return pts };
    let n = pts.len();
    let mut out = Vec::with_capacity(spline.spans() * SEGMENTS + 1);
    if let Some(first) = pts.first() {
        out.push(*first);
    }
    for i in 0..spline.spans() {
        let j = (i + 1) % n;
        let (p0, p3) = (pts[i], pts[j]);
        let (p1, p2) = (Vec3::from(handles[i][1]), Vec3::from(handles[j][0]));
        for k in 1..=SEGMENTS {
            let t = k as f32 / SEGMENTS as f32;
            let u = 1.0 - t;
            out.push(p0 * (u * u * u) + p1 * (3.0 * u * u * t) + p2 * (3.0 * u * t * t) + p3 * (t * t * t));
        }
    }
    // A closed Bezier comes back to its start through the last span.
    if spline.closed {
        out.pop();
    }
    out
}

/// The visible curve objects as world-space polylines.
///
/// Complexity: O(objects + sampled points).
pub fn polylines(doc: &Document, hidden: &BTreeSet<ObjectKey>) -> Vec<Polyline> {
    let mut out = Vec::new();
    for (k, o) in doc.objects() {
        if hidden.contains(&k) || !o.visible {
            continue;
        }
        let Some(curve) = doc.object_curve(k) else { continue };
        let model = Mat4::from(doc.world_affine(k).unwrap_or_default());
        for s in curve.splines.iter().filter(|s| s.is_valid()) {
            let points = sample(s).into_iter().map(|p| model.transform_point3(p)).collect();
            out.push(Polyline { object: k, points, closed: s.closed });
        }
    }
    out
}

/// The polylines for the last document state and hidden set seen.
#[derive(Debug, Default)]
pub struct CurveCache {
    key: Option<u64>,
    lines: Vec<Polyline>,
}

impl CurveCache {
    /// The polylines for `scene_key` and `hidden`, sampled again only when
    /// either changed.
    ///
    /// Complexity: O(hidden) to check; a resample as [`polylines`].
    pub fn get(&mut self, scene_key: u64, doc: &Document, hidden: &BTreeSet<ObjectKey>) -> &[Polyline] {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        scene_key.hash(&mut h);
        hidden.hash(&mut h);
        let key = h.finish();
        if self.key != Some(key) {
            self.lines = polylines(doc, hidden);
            self.key = Some(key);
        }
        &self.lines
    }
}

/// Screen positions (points) of `points`; `None` for points behind the camera.
fn project(camera: &OrbitCamera, rect: Rect, points: &[Vec3]) -> Vec<Option<Pos2>> {
    let vp = camera.view_proj(rect.width() / rect.height().max(1.0));
    points
        .iter()
        .map(|p| {
            let c = vp * p.extend(1.0);
            (c.w > 1e-6).then(|| {
                let n = c.truncate() / c.w;
                pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height())
            })
        })
        .collect()
}

/// The segments of a polyline on screen (both ends in front of the camera).
fn segments(line: &Polyline, screen: &[Option<Pos2>]) -> Vec<(Pos2, Pos2)> {
    let n = screen.len();
    let count = if line.closed { n } else { n.saturating_sub(1) };
    (0..count).filter_map(|i| Some((screen[i]?, screen[(i + 1) % n]?))).collect()
}

/// Draws the curves, the selected ones in `selected_color`.
///
/// Complexity: O(sampled points).
pub fn draw(
    p: &egui::Painter,
    camera: &OrbitCamera,
    rect: Rect,
    lines: &[Polyline],
    selection: &BTreeSet<ObjectKey>,
    color: egui::Color32,
    selected_color: egui::Color32,
) {
    for line in lines {
        let screen = project(camera, rect, &line.points);
        let c = if selection.contains(&line.object) { selected_color } else { color };
        for (a, b) in segments(line, &screen) {
            p.line_segment([a, b], Stroke::new(1.5, c));
        }
    }
}

/// The curve object nearest to `at` (points) within a few points.
///
/// Complexity: O(sampled points).
pub fn pick(camera: &OrbitCamera, rect: Rect, lines: &[Polyline], at: Pos2) -> Option<ObjectKey> {
    let mut best: Option<(f32, ObjectKey)> = None;
    for line in lines {
        let screen = project(camera, rect, &line.points);
        for (a, b) in segments(line, &screen) {
            let ab = b - a;
            let t = if ab.length_sq() > 0.0 { ((at - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
            let d = at.distance(a + ab * t);
            if d <= PICK_RADIUS && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, line.object));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_spans_pass_through_their_points() {
        let s = Spline {
            points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            handles: Some(vec![[[0.0; 3], [0.0, 1.0, 0.0]], [[1.0, 1.0, 0.0], [0.0; 3]]]),
            closed: false,
        };
        let pts = sample(&s);
        assert_eq!(pts.len(), SEGMENTS + 1);
        assert_eq!(pts[0], Vec3::ZERO);
        assert!((pts[SEGMENTS] - Vec3::X).length() < 1e-6);
        assert!(pts[SEGMENTS / 2].y > 0.5, "bulges toward the handles");
        let poly = Spline::polyline([Vec3::ZERO, Vec3::X, Vec3::Y], true);
        assert_eq!(sample(&poly).len(), 3, "polylines stay as they are");
    }

    #[test]
    fn curves_are_picked_near_their_lines() {
        let line = Polyline { object: ObjectKey::default(), points: vec![Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)], closed: false };
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        cam.frame(Vec3::splat(-1.0), Vec3::splat(1.0), 1.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
        assert_eq!(pick(&cam, rect, std::slice::from_ref(&line), rect.center() + egui::vec2(0.0, 3.0)), Some(ObjectKey::default()));
        assert_eq!(pick(&cam, rect, std::slice::from_ref(&line), rect.center() + egui::vec2(0.0, 30.0)), None);
    }
}
