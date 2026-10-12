//! The transform gizmo of the Move, Rotate and Scale tools, drawn like
//! Blender's: axis arrows, plane handles and a free-move center; rotation
//! rings (front halves) and a view ring; scale handles and a uniform ring.
//! It sits at the pivot point and follows the transform orientation of the
//! header; snapping steps follow the snap settings (Ctrl inverts them).
//!
//! Dragging previews the result and commits one undo step on release: in
//! Object Mode `scene.set_transform` per selected object (in parent space),
//! in Edit Mode `edit.transform` on the picked elements, previewed on a
//! working copy of the document. Esc or a right-click cancels.

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use glam::{Mat3, Mat4, Quat, Vec3};
use p3d_engine::OpCall;
use p3d_render::OrbitCamera;
use p3d_scene::{Document, ObjectKey};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::theme::Tokens;
use crate::transform::{Orientation, Pivot, SnapTarget};
use crate::{PixelApp, Tool};

/// Axis length on screen (points) and how near a handle must be.
const SIZE: f32 = 88.0;
const HIT: f32 = 8.0;
const RING: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Axis(usize),
    Plane(usize),
    Free,
    Ring(usize),
    ViewRing,
    Uniform,
}

/// What a drag changed so far, in world space; scaling is along the axes.
#[derive(Clone, Copy, Debug)]
struct Delta {
    translate: Vec3,
    /// Around one of the axes (`Some(index)`) or the view axis (`None`).
    rotate: Option<(Option<usize>, f32)>,
    scale: Vec3,
}

impl Default for Delta {
    fn default() -> Self {
        Delta { translate: Vec3::ZERO, rotate: None, scale: Vec3::ONE }
    }
}

/// The pivot and the orientation axes (world, unit length).
#[derive(Clone, Copy, Debug)]
struct Frame {
    pivot: Vec3,
    axes: [Vec3; 3],
}

/// A moved object, its world matrix at the start, and its descendants'.
type Moved = (ObjectKey, Mat4, Vec<(ObjectKey, Mat4)>);

struct Drag {
    handle: Handle,
    frame: Frame,
    start: Pos2,
    /// Where the pointer ray met the constraint at the start (world).
    anchor: Option<Vec3>,
    delta: Delta,
    /// Object Mode: the moved objects and their world matrices at the start,
    /// and each one's descendants (which follow).
    objects: Vec<Moved>,
    /// Edit Mode: the edited object and its world matrix.
    edit: Option<(ObjectKey, Mat4)>,
}

/// The gizmo between frames: the drag, the handle under the pointer, and
/// the Edit Mode preview document.
#[derive(Default)]
pub struct GizmoState {
    drag: Option<Drag>,
    hot: Option<Handle>,
    /// The document with the dragged elements moved, and its scene key.
    pub preview: Option<(Arc<Document>, u64)>,
    preview_counter: u64,
}

impl GizmoState {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
}

/// Screen position (points) of a world point; `None` behind the camera.
fn project(vp: Mat4, rect: Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// World meters per screen point at `p`.
fn meters_per_point(cam: &OrbitCamera, rect: Rect, p: Vec3) -> f32 {
    let depth = if cam.ortho { cam.distance } else { (p - cam.eye()).dot(-cam.toward()).max(1e-3) };
    2.0 * depth * (cam.fov_y / 2.0).tan() / rect.height().max(1.0)
}

/// A unit basis with `z` as its third axis.
fn basis_from_z(z: Vec3) -> [Vec3; 3] {
    let z = z.normalize_or(Vec3::Z);
    let helper = if z.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let x = helper.cross(z).normalize();
    [x, z.cross(x), z]
}

/// The columns of an affine's rotation, normalized.
fn rotation_axes(m: Mat4) -> [Vec3; 3] {
    let m3 = Mat3::from_mat4(m);
    [m3.x_axis.normalize_or(Vec3::X), m3.y_axis.normalize_or(Vec3::Y), m3.z_axis.normalize_or(Vec3::Z)]
}

fn world(doc: &Document, k: ObjectKey) -> Mat4 {
    Mat4::from(doc.world_affine(k).unwrap_or_default())
}

/// The objects a drag moves: selected ones whose ancestors are not
/// selected (a child follows its parent).
///
/// Complexity: O(selected x depth).
fn moved_objects(app: &PixelApp, doc: &Document) -> Vec<ObjectKey> {
    app.selection
        .iter()
        .copied()
        .filter(|&k| {
            let mut p = doc.object(k).and_then(|o| o.parent());
            while let Some(q) = p {
                if app.selection.contains(&q) {
                    return false;
                }
                p = doc.object(q).and_then(|o| o.parent());
            }
            doc.object(k).is_some()
        })
        .collect()
}

/// Selected vertices of the edited mesh in world space.
///
/// Complexity: O(selected vertices).
fn edit_points(app: &PixelApp, doc: &Document) -> Option<(ObjectKey, Mat4, Vec<Vec3>)> {
    let k = app.edit.object?;
    let (m, f) = (app.edit.mesh()?, app.edit.flushed()?);
    let model = world(doc, k);
    let pts: Vec<Vec3> = f.verts.iter().filter_map(|&v| m.positions.get(v as usize)).map(|p| model.transform_point3(*p)).collect();
    (!pts.is_empty()).then_some((k, model, pts))
}

/// The pivot and axes for the current selection and header settings.
///
/// Complexity: O(selected objects) or O(selected vertices + faces).
fn frame(app: &PixelApp, doc: &Document, cam: &OrbitCamera) -> Option<Frame> {
    let s = &app.ui.transform;
    let edit = app.ui.mode.is_edit();
    let (points, active_world): (Vec<Vec3>, Option<Mat4>) = if edit {
        let (_, model, pts) = edit_points(app, doc)?;
        (pts, Some(model))
    } else {
        let moved = moved_objects(app, doc);
        if moved.is_empty() {
            return None;
        }
        let pts = moved.iter().map(|&k| world(doc, k).transform_point3(Vec3::ZERO)).collect();
        (pts, app.active_object().map(|k| world(doc, k)))
    };
    let median = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let pivot = match s.pivot {
        Pivot::Cursor => s.cursor(),
        Pivot::BoundsCenter if edit => {
            let (lo, hi) = points.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            (lo + hi) * 0.5
        }
        Pivot::BoundsCenter => app.selection_center(doc).unwrap_or(median),
        Pivot::Active if !edit => active_world.map_or(median, |m| m.transform_point3(Vec3::ZERO)),
        _ => median,
    };
    let local = active_world.map(rotation_axes);
    let axes = match s.orientation {
        Orientation::Global | Orientation::Cursor => [Vec3::X, Vec3::Y, Vec3::Z],
        Orientation::Local | Orientation::Gimbal => local.unwrap_or([Vec3::X, Vec3::Y, Vec3::Z]),
        Orientation::Parent => {
            app.active_object().and_then(|k| doc.object(k)?.parent()).map(|p| rotation_axes(world(doc, p))).unwrap_or([Vec3::X, Vec3::Y, Vec3::Z])
        }
        Orientation::View => {
            let inv = cam.view().inverse();
            [inv.x_axis.truncate().normalize(), inv.y_axis.truncate().normalize(), inv.z_axis.truncate().normalize()]
        }
        Orientation::Normal => match (edit, app.edit.mesh(), app.edit.flushed()) {
            (true, Some(m), Some(f)) if !f.faces.is_empty() => {
                let normals = m.face_normals();
                let n: Vec3 = f.faces.iter().filter_map(|&x| normals.get(x as usize)).copied().sum();
                let model = active_world.unwrap_or(Mat4::IDENTITY);
                basis_from_z(model.inverse().transpose().transform_vector3(n))
            }
            _ => local.unwrap_or([Vec3::X, Vec3::Y, Vec3::Z]),
        },
    };
    Some(Frame { pivot, axes })
}

/// A world point's transform by `delta` around `pivot` with `axes`.
fn delta_matrix(d: &Delta, pivot: Vec3, axes: [Vec3; 3], view_axis: Vec3) -> Mat4 {
    let around = |m: Mat4| Mat4::from_translation(pivot) * m * Mat4::from_translation(-pivot);
    let r = d.rotate.map_or(Mat4::IDENTITY, |(i, a)| {
        let axis = i.map_or(view_axis, |i| axes[i]);
        around(Mat4::from_quat(Quat::from_axis_angle(axis, a)))
    });
    let b = Mat4::from_mat3(Mat3::from_cols(axes[0], axes[1], axes[2]));
    let s = around(b * Mat4::from_scale(d.scale) * b.transpose());
    Mat4::from_translation(d.translate) * r * s
}

/// Closest point parameter along the line `p + a t` to the ray `o + d s`.
fn line_param(p: Vec3, a: Vec3, o: Vec3, d: Vec3) -> Option<f32> {
    let b = a.dot(d);
    let denom = 1.0 - b * b;
    if denom.abs() < 1e-6 {
        return None;
    }
    let w = p - o;
    Some((b * d.dot(w) - a.dot(w)) / denom)
}

fn plane_hit(o: Vec3, d: Vec3, p: Vec3, n: Vec3) -> Option<Vec3> {
    let dn = d.dot(n);
    if dn.abs() < 1e-6 {
        return None;
    }
    let t = (p - o).dot(n) / dn;
    (t > 0.0).then(|| o + d * t)
}

fn snap(x: f32, step: f32) -> f32 {
    if step > 0.0 { (x / step).round() * step } else { x }
}

/// Screen geometry of the handles for hit testing and drawing.
struct Shapes {
    center: Pos2,
    /// Axis start and end on screen, and whether the axis is shown.
    axes: [(Pos2, Pos2, bool); 3],
    planes: [Option<[Pos2; 4]>; 3],
    /// Front halves of the rotation rings as screen segments.
    rings: [Vec<(Pos2, Pos2)>; 3],
    outer: f32,
}

fn shapes(tool: Tool, f: &Frame, cam: &OrbitCamera, rect: Rect) -> Option<Shapes> {
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let center = project(vp, rect, f.pivot)?;
    let len = SIZE * meters_per_point(cam, rect, f.pivot);
    let start = if tool == Tool::Rotate { 0.0 } else { 0.2 };
    let axes = std::array::from_fn(|i| {
        let a = project(vp, rect, f.pivot + f.axes[i] * len * start).unwrap_or(center);
        let b = project(vp, rect, f.pivot + f.axes[i] * len).unwrap_or(center);
        // An axis pointing at the camera is too short to grab.
        (a, b, b.distance(center) > SIZE * 0.18)
    });
    let planes = std::array::from_fn(|i| {
        let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
        let (lo, hi) = (0.28 * len, 0.46 * len);
        let c = [u * lo + v * lo, u * hi + v * lo, u * hi + v * hi, u * lo + v * hi];
        let pts: Option<Vec<Pos2>> = c.iter().map(|o| project(vp, rect, f.pivot + *o)).collect();
        pts.and_then(|p| <[Pos2; 4]>::try_from(p).ok())
    });
    let to_eye = if cam.ortho { cam.toward() } else { (cam.eye() - f.pivot).normalize_or(cam.toward()) };
    let rings = std::array::from_fn(|i| {
        let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
        let r = len * 0.9;
        let pts: Vec<(Vec3, Option<Pos2>)> = (0..=RING)
            .map(|k| {
                let a = k as f32 / RING as f32 * std::f32::consts::TAU;
                let off = (u * a.cos() + v * a.sin()) * r;
                (off, project(vp, rect, f.pivot + off))
            })
            .collect();
        pts.windows(2)
            .filter(|w| w[0].0.dot(to_eye) >= -1e-4 * r && w[1].0.dot(to_eye) >= -1e-4 * r)
            .filter_map(|w| Some((w[0].1?, w[1].1?)))
            .collect()
    });
    Some(Shapes { center, axes, planes, rings, outer: SIZE * 1.08 })
}

fn polygon_contains(poly: &[Pos2; 4], p: Pos2) -> bool {
    let mut inside = false;
    for i in 0..4 {
        let (a, b) = (poly[i], poly[(i + 1) % 4]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
    }
    inside
}

/// The handle under `p` for `tool`.
fn hit(tool: Tool, s: &Shapes, p: Pos2) -> Option<Handle> {
    let near = |a: Pos2, b: Pos2| segment_distance(p, a, b);
    match tool {
        Tool::Move => {
            if p.distance(s.center) <= HIT + 2.0 {
                return Some(Handle::Free);
            }
            for i in 0..3 {
                if let Some(q) = &s.planes[i]
                    && polygon_contains(q, p)
                {
                    return Some(Handle::Plane(i));
                }
            }
            (0..3)
                .filter(|&i| s.axes[i].2)
                .map(|i| (near(s.axes[i].0, s.axes[i].1), i))
                .filter(|(d, _)| *d <= HIT)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, i)| Handle::Axis(i))
        }
        Tool::Rotate => {
            let ring = (0..3)
                .filter_map(|i| s.rings[i].iter().map(|(a, b)| near(*a, *b)).min_by(f32::total_cmp).map(|d| (d, i)))
                .filter(|(d, _)| *d <= HIT)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            ring.map(|(_, i)| Handle::Ring(i)).or_else(|| ((p.distance(s.center) - s.outer).abs() <= HIT).then_some(Handle::ViewRing))
        }
        Tool::Scale => {
            let axis = (0..3)
                .filter(|&i| s.axes[i].2)
                .map(|i| (near(s.axes[i].0, s.axes[i].1), i))
                .filter(|(d, _)| *d <= HIT)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            axis.map(|(_, i)| Handle::Axis(i)).or_else(|| ((p.distance(s.center) - s.outer).abs() <= HIT).then_some(Handle::Uniform))
        }
        _ => None,
    }
}

/// The ray under `p` (points).
fn ray(cam: &OrbitCamera, rect: Rect, p: Pos2) -> (Vec3, Vec3) {
    cam.ray(p.x - rect.left(), p.y - rect.top(), rect.width(), rect.height())
}

/// Where the pointer ray meets the handle's constraint (world).
fn anchor(h: Handle, f: &Frame, cam: &OrbitCamera, rect: Rect, p: Pos2) -> Option<Vec3> {
    let (o, d) = ray(cam, rect, p);
    match h {
        Handle::Axis(i) => line_param(f.pivot, f.axes[i], o, d).map(|t| f.pivot + f.axes[i] * t),
        Handle::Plane(i) | Handle::Ring(i) => plane_hit(o, d, f.pivot, f.axes[i]),
        Handle::Free => plane_hit(o, d, f.pivot, cam.toward()),
        Handle::ViewRing | Handle::Uniform => None,
    }
}

/// The delta for the pointer at `p`, with snapping applied.
fn delta_at(drag: &Drag, app: &PixelApp, cam: &OrbitCamera, rect: Rect, p: Pos2, snapping: bool) -> Delta {
    let f = &drag.frame;
    let s = &app.ui.transform;
    let mut d = Delta::default();
    let step = if s.snaps_to(SnapTarget::Increment) || s.snaps_to(SnapTarget::Grid) { cam.grid_cell() } else { 0.0 };
    let snap_move = snapping && s.affect_move;
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let center = project(vp, rect, f.pivot).unwrap_or(p);
    match drag.handle {
        Handle::Axis(i) if app.ui.tool == Tool::Move => {
            if let (Some(a0), Some(a1)) = (drag.anchor, anchor(drag.handle, f, cam, rect, p)) {
                let mut t = (a1 - a0).dot(f.axes[i]);
                if snap_move {
                    t = snap(t, step);
                }
                d.translate = f.axes[i] * t;
            }
        }
        Handle::Plane(_) | Handle::Free => {
            if let (Some(a0), Some(a1)) = (drag.anchor, anchor(drag.handle, f, cam, rect, p)) {
                let mut v = a1 - a0;
                if snap_move {
                    v = f.axes.iter().map(|a| *a * snap(v.dot(*a), step)).sum();
                }
                d.translate = v;
            }
        }
        Handle::Ring(i) => {
            let n = f.axes[i];
            let angle = match (drag.anchor, anchor(drag.handle, f, cam, rect, p)) {
                (Some(a0), Some(a1)) if n.dot(cam.toward()).abs() > 0.15 => {
                    let (u, v) = ((a0 - f.pivot).normalize_or_zero(), (a1 - f.pivot).normalize_or_zero());
                    u.cross(v).dot(n).atan2(u.dot(v))
                }
                // Seen edge-on: turn by the pointer's angle around the center.
                _ => screen_angle(center, drag.start, p) * if n.dot(cam.toward()) >= 0.0 { 1.0 } else { -1.0 },
            };
            d.rotate = Some((Some(i), if snapping && s.affect_rotate { snap(angle, 15f32.to_radians()) } else { angle }));
        }
        Handle::ViewRing => {
            let angle = screen_angle(center, drag.start, p);
            d.rotate = Some((None, if snapping && s.affect_rotate { snap(angle, 15f32.to_radians()) } else { angle }));
        }
        Handle::Axis(i) => {
            // Scale along an axis: the pointer's distance along the axis on screen.
            let (a, b, _) = shapes(Tool::Scale, f, cam, rect).map_or((center, center, false), |s| s.axes[i]);
            let dir = (b - a).normalized();
            let (d0, d1) = ((drag.start - center).dot(dir), (p - center).dot(dir));
            let mut k = if d0.abs() > 1.0 { d1 / d0 } else { 1.0 };
            if snapping && s.affect_scale {
                k = snap(k, 0.1);
            }
            d.scale[i] = k;
        }
        Handle::Uniform => {
            let (d0, d1) = (drag.start.distance(center), p.distance(center));
            let mut k = if d0 > 1.0 { d1 / d0 } else { 1.0 };
            if snapping && s.affect_scale {
                k = snap(k, 0.1);
            }
            d.scale = Vec3::splat(k);
        }
    }
    d
}

/// Signed screen angle (radians, counterclockwise positive) from `a` to `b` around `c`.
fn screen_angle(c: Pos2, a: Pos2, b: Pos2) -> f32 {
    let (u, v) = (a - c, b - c);
    // Screen y points down: flip for counterclockwise.
    -(u.x * v.y - u.y * v.x).atan2(u.x * v.x + u.y * v.y)
}

/// The `edit.transform` parameters for a drag's delta.
fn edit_params(app: &PixelApp, drag: &Drag, cam: &OrbitCamera) -> Option<Value> {
    let (k, model) = drag.edit?;
    let doc = app.doc();
    let id = doc.object(k)?.public_id();
    let select = app.element_op_state("edit.transform").ok().flatten()?;
    let inv = model.inverse();
    let view_axes = basis_from_z(cam.toward());
    let world_axes = match drag.delta.rotate {
        Some((None, _)) => view_axes,
        _ => drag.frame.axes,
    };
    let axes: [Vec3; 3] = world_axes.map(|a| inv.transform_vector3(a).normalize_or_zero());
    let t = inv.transform_vector3(drag.delta.translate);
    let translate: [f32; 3] = std::array::from_fn(|i| t.dot(axes[i]));
    let mut rotate = [0.0f32; 3];
    if let Some((i, a)) = drag.delta.rotate {
        rotate[i.unwrap_or(2)] = a.to_degrees();
    }
    let s = &app.ui.transform;
    let (pivot, point) = match s.pivot {
        Pivot::Median | Pivot::Active => ("median", Vec3::ZERO),
        Pivot::BoundsCenter => ("bounds_center", Vec3::ZERO),
        Pivot::Individual => ("individual", Vec3::ZERO),
        Pivot::Cursor => ("point", inv.transform_point3(s.cursor())),
    };
    let mut p = json!({
        "target": id,
        "select": select,
        "translate": translate,
        "rotate": rotate,
        "scale": drag.delta.scale.to_array(),
        "pivot": pivot,
        "pivot_point": point.to_array(),
        "axes": axes.map(|a| a.to_array()),
    });
    if s.proportional {
        p["proportional"] = json!({ "radius": s.radius, "falloff": s.falloff, "connected_only": s.connected_only });
    }
    Some(p)
}

/// The `scene.set_transform` calls for an Object Mode drag.
fn object_calls(app: &PixelApp, drag: &Drag, view_axis: Vec3) -> Vec<OpCall> {
    let doc = app.doc();
    let individual = app.ui.transform.pivot == Pivot::Individual;
    drag.objects
        .iter()
        .filter_map(|(k, start, _)| {
            let pivot = if individual { start.transform_point3(Vec3::ZERO) } else { drag.frame.pivot };
            let new = delta_matrix(&drag.delta, pivot, drag.frame.axes, view_axis) * *start;
            let parent = doc.object(*k)?.parent().map_or(Mat4::IDENTITY, |p| world(doc, p));
            let (scale, rotation, position) = (parent.inverse() * new).to_scale_rotation_translation();
            Some(OpCall::new(
                "scene.set_transform",
                json!({
                    "target": doc.object(*k)?.public_id(),
                    "position": position.to_array(),
                    "rotation": p3d_math::transform::rotation_to_euler_deg(rotation),
                    "scale": scale.to_array(),
                }),
            ))
        })
        .collect()
}

/// Lays out the gizmo over the viewport `rect`: hit testing, dragging,
/// previews, commits and drawing.
///
/// Complexity: O(selected objects and their descendants) per frame while
/// dragging in Object Mode; in Edit Mode a preview op run per pointer move.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui, rect: Rect) {
    let tool = app.ui.tool;
    if !matches!(tool, Tool::Move | Tool::Rotate | Tool::Scale) || !rect.is_positive() {
        cancel(app);
        return;
    }
    let cam = app.viewport.camera;
    let doc = Arc::clone(app.session.doc());
    let frame_now = match &app.gizmo.drag {
        Some(d) => Some(d.frame),
        None => frame(app, &doc, &cam),
    };
    let Some(f) = frame_now else {
        cancel(app);
        return;
    };
    let Some(s) = shapes(tool, &f, &cam, rect) else { return };
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if app.gizmo.drag.is_none() {
        app.gizmo.hot = pointer.filter(|p| rect.contains(*p)).and_then(|p| hit(tool, &s, p));
    }
    // Take the pointer only over a handle or while dragging, so clicks and
    // boxes elsewhere still reach the viewport.
    if app.gizmo.hot.is_some() || app.gizmo.drag.is_some() {
        let resp = ui.interact(rect, ui.id().with("gizmo"), Sense::drag());
        if resp.drag_started()
            && let (Some(h), Some(p)) = (app.gizmo.hot, resp.interact_pointer_pos())
        {
            let edit = app.ui.mode.is_edit();
            let objects = if edit {
                Vec::new()
            } else {
                moved_objects(app, &doc)
                    .into_iter()
                    .map(|k| {
                        let start = world(&doc, k);
                        let below = doc.subtree(k).into_iter().filter(|c| *c != k).map(|c| (c, world(&doc, c))).collect();
                        (k, start, below)
                    })
                    .collect()
            };
            let edit_target = if edit { app.edit.object.map(|k| (k, world(&doc, k))) } else { None };
            app.gizmo.drag = Some(Drag {
                handle: h,
                frame: f,
                start: p,
                anchor: anchor(h, &f, &cam, rect, p),
                delta: Delta::default(),
                objects,
                edit: edit_target,
            });
        }
        let cancel_now = ui.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.secondary_pressed());
        if app.gizmo.drag.is_some() && cancel_now {
            cancel(app);
            return;
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let snapping = app.ui.view.snap != ui.input(|i| i.modifiers.command);
            let delta = app.gizmo.drag.as_ref().map(|d| delta_at(d, app, &cam, rect, p, snapping));
            if let Some(dl) = delta {
                if let Some(d) = app.gizmo.drag.as_mut() {
                    d.delta = dl;
                }
                preview(app, &cam);
            }
        }
        if resp.drag_stopped() {
            commit(app, &cam);
        }
    }
    draw(app, ui, rect, tool, &cam);
}

/// Shows the drag's result without changing the document.
fn preview(app: &mut PixelApp, cam: &OrbitCamera) {
    let Some(drag) = &app.gizmo.drag else { return };
    let view_axis = cam.toward();
    if drag.edit.is_some() {
        let Some(params) = edit_params(app, drag, cam) else { return };
        let mut work = p3d_engine::Session::with_registry(Arc::clone(app.session.registry()));
        work.load(Document::clone(app.session.doc()));
        if work.execute(&[OpCall::new("edit.transform", params)], &Default::default()).is_ok() {
            app.gizmo.preview_counter += 1;
            app.gizmo.preview = Some((Arc::clone(work.doc()), app.gizmo.preview_counter));
        }
        return;
    }
    let individual = app.ui.transform.pivot == Pivot::Individual;
    let mut map = std::collections::BTreeMap::new();
    for (k, start, below) in &drag.objects {
        let pivot = if individual { start.transform_point3(Vec3::ZERO) } else { drag.frame.pivot };
        let m = delta_matrix(&drag.delta, pivot, drag.frame.axes, view_axis);
        map.insert(*k, m * *start);
        for (c, cw) in below {
            map.insert(*c, m * *cw);
        }
    }
    app.viewport.preview = map;
}

/// Commits the drag as one undo step.
fn commit(app: &mut PixelApp, cam: &OrbitCamera) {
    let Some(drag) = app.gizmo.drag.take() else { return };
    app.viewport.preview.clear();
    app.gizmo.preview = None;
    let label = match app.ui.tool {
        Tool::Rotate => t("Rotate"),
        Tool::Scale => t("Scale"),
        _ => t("Move"),
    };
    let moved = drag.delta.translate != Vec3::ZERO || drag.delta.rotate.is_some_and(|(_, a)| a != 0.0) || drag.delta.scale != Vec3::ONE;
    if !moved {
        return;
    }
    let r = if drag.edit.is_some() {
        match edit_params(app, &drag, cam) {
            Some(p) => app.execute_labeled(&[OpCall::new("edit.transform", p)], Some(label)),
            None => Err(t("nothing to transform").to_owned()),
        }
    } else {
        let calls = object_calls(app, &drag, cam.toward());
        app.execute_labeled(&calls, Some(label))
    };
    if let Err(e) = r {
        app.set_status(e);
    }
}

/// Drops a drag and its preview.
fn cancel(app: &mut PixelApp) {
    if app.gizmo.drag.take().is_some() {
        app.viewport.preview.clear();
        app.gizmo.preview = None;
    }
    app.gizmo.hot = None;
}

/// Paints the handles; the dragged or hovered one lights up.
fn draw(app: &PixelApp, ui: &egui::Ui, rect: Rect, tool: Tool, cam: &OrbitCamera) {
    let tk = Tokens::get();
    let Some(mut f) = app.gizmo.drag.as_ref().map(|d| d.frame).or_else(|| frame(app, app.doc(), cam)) else { return };
    if let Some(d) = &app.gizmo.drag {
        f.pivot += d.delta.translate;
    }
    let Some(s) = shapes(tool, &f, cam, rect) else { return };
    let p = ui.painter_at(rect);
    let active = app.gizmo.drag.as_ref().map(|d| d.handle).or(app.gizmo.hot);
    let colors = [tk.axis_x, tk.axis_y, tk.axis_z];
    let lit = |c: Color32, on: bool| if on { c.lerp_to_gamma(Color32::WHITE, 0.45) } else { c };
    let width = |on: bool| if on { 3.0 } else { 2.0 };
    match tool {
        Tool::Move | Tool::Scale => {
            for (i, (&(a, b, shown), color)) in s.axes.iter().zip(colors).enumerate() {
                if !shown {
                    continue;
                }
                let on = active == Some(Handle::Axis(i));
                let c = lit(color, on);
                p.line_segment([a, b], Stroke::new(width(on), c));
                let dir = (b - a).normalized();
                if tool == Tool::Move {
                    let side = vec2(-dir.y, dir.x);
                    p.add(egui::Shape::convex_polygon(vec![b + dir * 12.0, b + side * 5.0, b - side * 5.0], c, Stroke::NONE));
                } else {
                    p.rect_filled(Rect::from_center_size(b, vec2(9.0, 9.0)), 2.0, c);
                }
            }
            if tool == Tool::Move {
                for (i, (plane, color)) in s.planes.iter().zip(colors).enumerate() {
                    if let Some(q) = plane {
                        let on = active == Some(Handle::Plane(i));
                        p.add(egui::Shape::convex_polygon(q.to_vec(), lit(color, on).gamma_multiply(if on { 0.8 } else { 0.45 }), Stroke::NONE));
                    }
                }
                let on = active == Some(Handle::Free);
                p.circle_stroke(s.center, 6.0, Stroke::new(width(on), lit(Color32::from_gray(225), on)));
            } else {
                let on = active == Some(Handle::Uniform);
                p.circle_stroke(s.center, s.outer, Stroke::new(width(on), lit(Color32::from_gray(200), on)));
            }
        }
        Tool::Rotate => {
            for (i, (ring, color)) in s.rings.iter().zip(colors).enumerate() {
                let on = active == Some(Handle::Ring(i));
                for (a, b) in ring {
                    p.line_segment([*a, *b], Stroke::new(width(on) + 0.5, lit(color, on)));
                }
            }
            let on = active == Some(Handle::ViewRing);
            p.circle_stroke(s.center, s.outer, Stroke::new(width(on), lit(Color32::from_gray(200), on)));
        }
        _ => {}
    }
    // The value being dragged, next to the pointer.
    if let (Some(d), Some(at)) = (&app.gizmo.drag, ui.input(|i| i.pointer.hover_pos())) {
        let text = match (d.handle, d.delta.rotate) {
            (_, Some((_, a))) => format!("{:.1}\u{b0}", a.to_degrees()),
            (Handle::Axis(i), _) if tool == Tool::Scale => format!("{} {:.3}", ["X", "Y", "Z"][i], d.delta.scale[i]),
            (Handle::Uniform, _) => format!("{:.3}", d.delta.scale.x),
            _ => {
                let v = d.delta.translate;
                format!("{:.3} {:.3} {:.3} m", v.x, v.y, v.z)
            }
        };
        let g = p.layout_no_wrap(text, crate::theme::small(), tk.text);
        let r = Rect::from_min_size(at + vec2(16.0, 14.0), g.size() + vec2(12.0, 6.0));
        p.rect_filled(r, 6.0, tk.popup.gamma_multiply(0.92));
        p.galley(r.min + vec2(6.0, 3.0), g, tk.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_compose_around_the_pivot() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let d = Delta { rotate: Some((Some(1), std::f32::consts::FRAC_PI_2)), ..Default::default() };
        let m = delta_matrix(&d, Vec3::new(1.0, 0.0, 0.0), axes, Vec3::Z);
        // A quarter turn around +Y through (1, 0, 0) takes (2, 0, 0) to (1, 0, -1).
        assert!((m.transform_point3(Vec3::new(2.0, 0.0, 0.0)) - Vec3::new(1.0, 0.0, -1.0)).length() < 1e-5);
        let d = Delta { scale: Vec3::new(2.0, 1.0, 1.0), translate: Vec3::Y, ..Default::default() };
        let m = delta_matrix(&d, Vec3::ZERO, axes, Vec3::Z);
        assert!((m.transform_point3(Vec3::new(1.0, 1.0, 0.0)) - Vec3::new(2.0, 2.0, 0.0)).length() < 1e-5);
        assert_eq!(snap(0.26, 0.1), 0.3);
        assert!((line_param(Vec3::ZERO, Vec3::X, Vec3::new(2.0, 5.0, 0.0), Vec3::NEG_Y).unwrap() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn dragging_an_axis_moves_the_selection_in_one_step() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        app.run("ui.tool", json!({ "tool": "move" })).unwrap();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        cam.frame(Vec3::splat(-1.0), Vec3::splat(1.0), 800.0 / 600.0);
        let doc = Arc::clone(app.session.doc());
        let f = frame(&app, &doc, &cam).unwrap();
        let s = shapes(Tool::Move, &f, &cam, rect).unwrap();
        // The X arrow is under its tip.
        let tip = s.axes[0].1;
        assert_eq!(hit(Tool::Move, &s, tip), Some(Handle::Axis(0)));
        assert_eq!(hit(Tool::Move, &s, s.center), Some(Handle::Free));
        let mut drag = Drag {
            handle: Handle::Axis(0),
            frame: f,
            start: tip,
            anchor: anchor(Handle::Axis(0), &f, &cam, rect, tip),
            delta: Delta::default(),
            objects: vec![(doc.resolve("crate").unwrap(), world(&doc, doc.resolve("crate").unwrap()), Vec::new())],
            edit: None,
        };
        // Drag 100 points to the right with snapping on: whole grid cells.
        drag.delta = delta_at(&drag, &app, &cam, rect, tip + egui::vec2(100.0, 0.0), true);
        let step = cam.grid_cell();
        let x = drag.delta.translate.x;
        assert!(x > 0.0 && (x / step - (x / step).round()).abs() < 1e-4, "{x} in steps of {step}");
        assert_eq!(drag.delta.translate.y, 0.0);
        app.gizmo.drag = Some(drag);
        let steps = app.session.history().len();
        commit(&mut app, &cam);
        assert_eq!(app.session.history().len(), steps + 1, "one undo step");
        let k = app.doc().resolve("crate").unwrap();
        assert!((app.doc().object(k).unwrap().transform.translation.x - x).abs() < 1e-5);
    }
}
