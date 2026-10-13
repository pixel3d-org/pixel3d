//! Transforms in the viewport: the gizmo of the Move, Rotate and Scale
//! tools, drawn like Blender's (axis arrows, plane handles and a free-move
//! center; rotation rings and a view ring; scale handles and a uniform
//! ring), and the modal transforms G, R and S.
//!
//! Both follow the header: the pivot point and the transform orientation
//! (Global, Local, Normal, Gimbal, View, Cursor, Parent), snapping (grid
//! steps, or vertices, edges, edge centers, perpendicular feet, faces and
//! volumes found through each mesh's BVH; Ctrl inverts) and proportional
//! editing. During a drag X, Y and Z constrain to an axis (Shift: the
//! plane without it), and typed values override the pointer (Dynamic
//! Input: `0.2`, `20cm`, `45`, `-1.5`), shown in the tool settings row.
//!
//! Dragging previews the result and commits one undo step: in Object Mode
//! `scene.set_transform` per selected object (in parent space), in Edit
//! Mode `edit.transform` on the picked elements, previewed on a working
//! copy of the document. Esc or a right-click cancels.

use std::sync::Arc;

use egui::{Color32, Event, Key, Pos2, Rect, Sense, Stroke, pos2, vec2};
use glam::{Mat3, Mat4, Quat, Vec3};
use p3d_engine::OpCall;
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use p3d_render::OrbitCamera;
use p3d_scene::{Document, ObjectKey};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::theme::Tokens;
use crate::transform::{Orientation, Pivot, SnapBase, SnapTarget};
use crate::{PixelApp, Tool};

/// Axis length on screen (points) and how near a handle must be.
const SIZE: f32 = 88.0;
const HIT: f32 = 8.0;
const RING: usize = 64;
/// Most moving points tried for the Closest snap base.
const MAX_SOURCES: usize = 65_536;

/// What a transform does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

impl Kind {
    fn of(tool: Tool) -> Option<Kind> {
        match tool {
            Tool::Move => Some(Kind::Move),
            Tool::Rotate => Some(Kind::Rotate),
            Tool::Scale => Some(Kind::Scale),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Move => t("Move"),
            Kind::Rotate => t("Rotate"),
            Kind::Scale => t("Scale"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Axis(usize),
    Plane(usize),
    Free,
    Ring(usize),
    ViewRing,
    Uniform,
}

/// An axis or plane constraint (G X, G Shift+X).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Constraint {
    None,
    Axis(usize),
    Plane(usize),
}

/// The handle whose math a kind and constraint use.
fn handle_for(kind: Kind, c: Constraint) -> Handle {
    match (kind, c) {
        (Kind::Move, Constraint::None) => Handle::Free,
        (Kind::Move, Constraint::Axis(i)) | (Kind::Scale, Constraint::Axis(i)) => Handle::Axis(i),
        (Kind::Move, Constraint::Plane(i)) => Handle::Plane(i),
        (Kind::Rotate, Constraint::Axis(i) | Constraint::Plane(i)) => Handle::Ring(i),
        (Kind::Rotate, Constraint::None) => Handle::ViewRing,
        (Kind::Scale, _) => Handle::Uniform,
    }
}

fn constraint_of(h: Handle) -> Constraint {
    match h {
        Handle::Axis(i) | Handle::Ring(i) => Constraint::Axis(i),
        Handle::Plane(i) => Constraint::Plane(i),
        _ => Constraint::None,
    }
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

/// A mesh snapping may land on: the mesh, its BVH, its world matrix, and
/// whether it is the edited one (its moving vertices are no targets).
type SnapMesh = (Arc<Mesh>, Arc<TriBvh>, Mat4, bool);

struct Drag {
    kind: Kind,
    handle: Handle,
    /// G, R or S from the keyboard (no button held; a click confirms).
    modal: bool,
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
    /// Points that move (world, at the start) and the active one, for the
    /// snap base.
    sources: Vec<Vec3>,
    active: Option<Vec3>,
    /// Meshes snapping may land on (built only when geometry targets are on).
    snap_meshes: Vec<SnapMesh>,
    snapped: Option<crate::snap::Hit>,
    /// Dynamic Input: the typed value.
    typed: String,
    /// The drag moves a shape of the SDF editor, not objects.
    sdf: bool,
}

/// The gizmo between frames: the drag, the handle under the pointer, a
/// modal transform asked for by a key, and the Edit Mode preview document.
#[derive(Default)]
pub struct GizmoState {
    drag: Option<Drag>,
    hot: Option<Handle>,
    /// G, R or S was pressed: start that modal transform next frame.
    pub pending: Option<Kind>,
    /// The document with the dragged elements moved, and its scene key.
    pub preview: Option<(Arc<Document>, u64)>,
    preview_counter: u64,
}

impl GizmoState {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The pointer is on a handle, or a drag runs: clicks belong to the gizmo.
    pub fn over_handle(&self) -> bool {
        self.hot.is_some() || self.drag.is_some()
    }

    /// A modal transform has the keyboard and the pointer.
    pub fn modal(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.modal) || self.pending.is_some()
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

/// The parent's world rotation of `k` (identity at the top level).
fn parent_rotation(doc: &Document, k: ObjectKey) -> Quat {
    doc.object(k).and_then(|o| o.parent()).map_or(Quat::IDENTITY, |p| world(doc, p).to_scale_rotation_translation().1)
}

/// Gimbal axes of an object: the Euler rotation's own axes (X, then Y,
/// then Z about the parent's axes): X turned by Y and Z, Y turned by Z, and
/// the parent's Z.
fn gimbal_axes(doc: &Document, k: ObjectKey) -> Option<[Vec3; 3]> {
    let o = doc.object(k)?;
    let [_, y, z] = p3d_math::transform::rotation_to_euler_deg(o.transform.rotation);
    let (rz, ry) = (Quat::from_rotation_z(z.to_radians()), Quat::from_rotation_y(y.to_radians()));
    let p = parent_rotation(doc, k);
    Some([(p * rz * ry * Vec3::X).normalize(), (p * rz * Vec3::Y).normalize(), (p * Vec3::Z).normalize()])
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

/// The active element of the edited mesh in world space: the last picked
/// vertex, the middle of the last picked edge, or the center of the last
/// picked face.
fn active_element(app: &PixelApp, doc: &Document) -> Option<Vec3> {
    let (k, id) = (app.edit.object?, app.edit.active?);
    let m = app.edit.mesh()?;
    let model = world(doc, k);
    let pos = |v: u32| m.positions.get(v as usize).copied();
    let local = match app.edit.picked().1? {
        crate::SelectMode::Vert => pos(id)?,
        crate::SelectMode::Edge => {
            let [a, b] = m.edge_verts(id);
            (pos(a)? + pos(b)?) * 0.5
        }
        _ => {
            let (sum, n) = m.face_verts(id).filter_map(pos).fold((Vec3::ZERO, 0.0), |(s, n), p| (s + p, n + 1.0));
            if n == 0.0 {
                return None;
            }
            sum / n
        }
    };
    Some(model.transform_point3(local))
}

/// The pivot and axes for the current selection and header settings.
///
/// Complexity: O(selected objects) or O(selected vertices + faces).
fn frame(app: &PixelApp, doc: &Document, cam: &OrbitCamera) -> Option<Frame> {
    let s = &app.ui.transform;
    let edit = app.ui.mode.is_edit();
    // A shape selected in the SDF editor: the gizmo sits on it.
    if let Some((center, rot)) = app.sdf_gizmo_subject() {
        let own = [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z];
        let axes = match s.orientation {
            Orientation::Global => [Vec3::X, Vec3::Y, Vec3::Z],
            Orientation::View => {
                let inv = cam.view().inverse();
                [inv.x_axis.truncate().normalize(), inv.y_axis.truncate().normalize(), inv.z_axis.truncate().normalize()]
            }
            Orientation::Cursor => {
                let q = s.cursor_rotation();
                [q * Vec3::X, q * Vec3::Y, q * Vec3::Z]
            }
            _ => own,
        };
        let pivot = if s.pivot == Pivot::Cursor { s.cursor() } else { center };
        return Some(Frame { pivot, axes });
    }
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
        Pivot::Active if edit => active_element(app, doc).unwrap_or(median),
        Pivot::Active => active_world.map_or(median, |m| m.transform_point3(Vec3::ZERO)),
        _ => median,
    };
    let local = active_world.map(rotation_axes);
    let subject = if edit { app.edit.object } else { app.active_object() };
    let global = [Vec3::X, Vec3::Y, Vec3::Z];
    let axes = match s.orientation {
        Orientation::Global => global,
        Orientation::Cursor => {
            let q = s.cursor_rotation();
            [q * Vec3::X, q * Vec3::Y, q * Vec3::Z]
        }
        Orientation::Local => local.unwrap_or(global),
        Orientation::Gimbal => subject.and_then(|k| gimbal_axes(doc, k)).unwrap_or(global),
        Orientation::Parent => subject.map(|k| parent_rotation(doc, k)).map(|q| [q * Vec3::X, q * Vec3::Y, q * Vec3::Z]).unwrap_or(global),
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
            _ => local.unwrap_or(global),
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

fn shapes(kind: Kind, f: &Frame, cam: &OrbitCamera, rect: Rect) -> Option<Shapes> {
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let center = project(vp, rect, f.pivot)?;
    let len = SIZE * meters_per_point(cam, rect, f.pivot);
    let start = if kind == Kind::Rotate { 0.0 } else { 0.2 };
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

/// The handle under `p` for `kind`.
fn hit(kind: Kind, s: &Shapes, p: Pos2) -> Option<Handle> {
    let near = |a: Pos2, b: Pos2| segment_distance(p, a, b);
    let axis =
        || (0..3).filter(|&i| s.axes[i].2).map(|i| (near(s.axes[i].0, s.axes[i].1), i)).filter(|(d, _)| *d <= HIT).min_by(|a, b| a.0.total_cmp(&b.0));
    match kind {
        Kind::Move => {
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
            axis().map(|(_, i)| Handle::Axis(i))
        }
        Kind::Rotate => {
            let ring = (0..3)
                .filter_map(|i| s.rings[i].iter().map(|(a, b)| near(*a, *b)).min_by(f32::total_cmp).map(|d| (d, i)))
                .filter(|(d, _)| *d <= HIT)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            ring.map(|(_, i)| Handle::Ring(i)).or_else(|| ((p.distance(s.center) - s.outer).abs() <= HIT).then_some(Handle::ViewRing))
        }
        Kind::Scale => axis().map(|(_, i)| Handle::Axis(i)).or_else(|| ((p.distance(s.center) - s.outer).abs() <= HIT).then_some(Handle::Uniform)),
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

/// The typed Dynamic Input value: meters, radians or a scale factor.
fn typed_value(kind: Kind, text: &str) -> Option<f32> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    // A lone sign or point is a value still being typed: zero so far.
    if matches!(s, "-" | "+" | "." | "-." | "+.") {
        return Some(if kind == Kind::Scale { 1.0 } else { 0.0 });
    }
    let v = match kind {
        Kind::Move => p3d_math::units::parse_length(s).ok()?,
        Kind::Rotate => p3d_math::units::parse_angle(s).ok()?,
        Kind::Scale => s.parse::<f64>().ok()?,
    };
    Some(v as f32)
}

/// The world point that snaps: per Snap With, the moving point nearest to
/// the target, the pivot, the median, or the active element.
fn snap_base(drag: &Drag, base: SnapBase, target: Vec3) -> Vec3 {
    match base {
        SnapBase::Center => drag.frame.pivot,
        SnapBase::Median if !drag.sources.is_empty() => drag.sources.iter().copied().sum::<Vec3>() / drag.sources.len() as f32,
        SnapBase::Active => drag.active.unwrap_or(drag.frame.pivot),
        SnapBase::Closest => drag
            .sources
            .iter()
            .copied()
            .min_by(|a, b| (*a - target).length_squared().total_cmp(&(*b - target).length_squared()))
            .unwrap_or(drag.frame.pivot),
        SnapBase::Median => drag.frame.pivot,
    }
}

/// The delta for the pointer at `p`: typed values first, then snapping to
/// geometry, then the pointer with grid steps. Returns the snap hit too.
///
/// Complexity: O(1), plus a snap search (O(meshes x rays x log F)) and
/// O(moving points) for the Closest snap base when geometry snapping is on.
fn delta_at(drag: &Drag, app: &PixelApp, cam: &OrbitCamera, rect: Rect, p: Pos2, snapping: bool) -> (Delta, Option<crate::snap::Hit>) {
    let f = &drag.frame;
    let s = &app.ui.transform;
    let mut d = Delta::default();
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let center = project(vp, rect, f.pivot).unwrap_or(p);
    let c = constraint_of(drag.handle);
    // Dynamic Input: a typed value along the constraint (X when free).
    if let Some(v) = typed_value(drag.kind, &drag.typed) {
        let axis = match c {
            Constraint::Axis(i) => Some(i),
            Constraint::Plane(i) => Some((i + 1) % 3),
            Constraint::None => None,
        };
        match drag.kind {
            Kind::Move => d.translate = f.axes[axis.unwrap_or(0)] * v,
            Kind::Rotate => d.rotate = Some((axis, v)),
            Kind::Scale => match axis {
                Some(i) => d.scale[i] = v,
                None => d.scale = Vec3::splat(v),
            },
        }
        return (d, None);
    }
    let step = if s.snaps_to(SnapTarget::Increment) || s.snaps_to(SnapTarget::Grid) { cam.grid_cell() } else { 0.0 };
    let snap_move = snapping && s.affect_move;
    match (drag.kind, drag.handle) {
        (Kind::Move, Handle::Axis(_) | Handle::Plane(_) | Handle::Free) => {
            let raw = match (drag.anchor, anchor(drag.handle, f, cam, rect, p)) {
                (Some(a0), Some(a1)) => a1 - a0,
                _ => Vec3::ZERO,
            };
            let constrain = |v: Vec3| match c {
                Constraint::Axis(i) => f.axes[i] * v.dot(f.axes[i]),
                Constraint::Plane(i) => v - f.axes[i] * v.dot(f.axes[i]),
                Constraint::None => v,
            };
            let raw = constrain(raw);
            // Snapping to geometry: the base point lands on the target.
            if snap_move && !drag.snap_meshes.is_empty() {
                let geometry: Vec<SnapTarget> =
                    s.snap_targets.iter().copied().filter(|t| !matches!(t, SnapTarget::Increment | SnapTarget::Grid)).collect();
                let cands: Vec<crate::snap::Candidate<'_>> = drag
                    .snap_meshes
                    .iter()
                    .map(|(m, b, model, edited)| crate::snap::Candidate {
                        mesh: m,
                        bvh: b,
                        model: *model,
                        moving: if *edited { app.edit.flushed().map(|fl| &fl.verts) } else { None },
                    })
                    .collect();
                let from = drag.frame.pivot + raw;
                if let Some(h) = crate::snap::find(&cands, cam, rect, p, from, &geometry, s.face_nearest) {
                    let base = snap_base(drag, s.snap_base, h.point);
                    d.translate = constrain(h.point - base);
                    return (d, Some(h));
                }
            }
            d.translate = if snap_move && step > 0.0 { f.axes.iter().map(|a| *a * snap(raw.dot(*a), step)).sum() } else { raw };
        }
        (_, Handle::Ring(i)) => {
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
        (_, Handle::ViewRing) => {
            let angle = screen_angle(center, drag.start, p);
            d.rotate = Some((None, if snapping && s.affect_rotate { snap(angle, 15f32.to_radians()) } else { angle }));
        }
        (_, Handle::Axis(i)) => {
            // Scale along an axis: the pointer's distance along the axis on
            // screen (from the gizmo), or from the pivot (modal).
            let (a, b, _) = shapes(Kind::Scale, f, cam, rect).map_or((center, center, false), |s| s.axes[i]);
            let dir = (b - a).normalized();
            let (d0, d1) = if drag.modal {
                (drag.start.distance(center), (p - center).dot(dir).signum() * p.distance(center))
            } else {
                ((drag.start - center).dot(dir), (p - center).dot(dir))
            };
            let mut k = if d0.abs() > 1.0 { d1 / d0 } else { 1.0 };
            if snapping && s.affect_scale {
                k = snap(k, 0.1);
            }
            d.scale[i] = k;
        }
        (_, _) => {
            let (d0, d1) = (drag.start.distance(center), p.distance(center));
            let mut k = if d0 > 1.0 { d1 / d0 } else { 1.0 };
            if snapping && s.affect_scale {
                k = snap(k, 0.1);
            }
            d.scale = Vec3::splat(k);
        }
    }
    (d, None)
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
        Pivot::Median => ("median", Vec3::ZERO),
        Pivot::BoundsCenter => ("bounds_center", Vec3::ZERO),
        Pivot::Individual => ("individual", Vec3::ZERO),
        Pivot::Cursor => ("point", inv.transform_point3(s.cursor())),
        Pivot::Active => ("point", inv.transform_point3(drag.frame.pivot)),
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

/// Where the gizmo's handles are on screen (points), for agents and tests:
/// the center and the axis ends; `null` without a gizmo.
pub fn handles_json(app: &PixelApp) -> Value {
    let rect = app.viewport.rect;
    let Some(kind) = Kind::of(app.ui.tool) else { return Value::Null };
    if !rect.is_positive() {
        return Value::Null;
    }
    let cam = app.viewport.camera;
    let Some(f) = frame(app, app.doc(), &cam) else { return Value::Null };
    let Some(s) = shapes(kind, &f, &cam, rect) else { return Value::Null };
    let at = |p: Pos2| json!([p.x, p.y]);
    json!({ "center": at(s.center), "x": at(s.axes[0].1), "y": at(s.axes[1].1), "z": at(s.axes[2].1), "ring": s.outer })
}

/// What the tool settings row shows during a transform: the kind, the
/// constraint and the value (typed or from the pointer).
pub fn status_text(app: &PixelApp) -> Option<String> {
    let d = app.gizmo.drag.as_ref()?;
    let axis_name = |i: usize| ["X", "Y", "Z"][i];
    let along = match constraint_of(d.handle) {
        Constraint::Axis(i) => format!("  {} {}", t("along"), axis_name(i)),
        Constraint::Plane(i) => format!("  {} {}", t("locked"), axis_name(i)),
        Constraint::None => String::new(),
    };
    let value = match d.kind {
        Kind::Move => {
            let v = d.delta.translate;
            format!(
                "{} {} {}",
                p3d_math::units::format_length(f64::from(v.x)),
                p3d_math::units::format_length(f64::from(v.y)),
                p3d_math::units::format_length(f64::from(v.z))
            )
        }
        Kind::Rotate => format!("{:.2}\u{b0}", d.delta.rotate.map_or(0.0, |(_, a)| a.to_degrees())),
        Kind::Scale => format!("{:.3} {:.3} {:.3}", d.delta.scale.x, d.delta.scale.y, d.delta.scale.z),
    };
    let typed = if d.typed.is_empty() { String::new() } else { format!("  [{}]", d.typed) };
    let snapped = d.snapped.map(|h| format!("  {} {}", t("snapped to"), h.target.label())).unwrap_or_default();
    Some(format!("{}{along}  {value}{typed}{snapped}", d.kind.label()))
}

/// Builds a drag for `kind` starting at `p` (meshes for snapping only when
/// geometry targets are on).
///
/// Complexity: O(selected objects and their descendants, or selected
/// vertices); with geometry snapping, a BVH per visible mesh not yet built.
fn begin(app: &mut PixelApp, kind: Kind, handle: Handle, modal: bool, f: Frame, p: Pos2) -> Drag {
    let cam = app.viewport.camera;
    let rect = app.viewport.rect;
    let doc = Arc::clone(app.session.doc());
    let edit = app.ui.mode.is_edit();
    let sdf = !edit && app.sdf_gizmo_subject().is_some();
    if sdf {
        app.sdf_begin_drag();
    }
    let moved = if edit || sdf { Vec::new() } else { moved_objects(app, &doc) };
    let objects: Vec<Moved> = moved
        .iter()
        .map(|&k| {
            let start = world(&doc, k);
            let below = doc.subtree(k).into_iter().filter(|c| *c != k).map(|c| (c, world(&doc, c))).collect();
            (k, start, below)
        })
        .collect();
    let edit_target = if edit { app.edit.object.map(|k| (k, world(&doc, k))) } else { None };
    // The points that move, for Snap With.
    let (sources, active) = if edit {
        (edit_points(app, &doc).map(|(_, _, p)| p).unwrap_or_default(), active_element(app, &doc))
    } else if sdf {
        (vec![f.pivot], Some(f.pivot))
    } else {
        let mut pts = Vec::new();
        for (_, m, _) in &objects {
            pts.push(m.transform_point3(Vec3::ZERO));
        }
        for &k in &moved {
            if let Some((lo, hi)) = doc.object_mesh(k).and_then(|m| m.bounds()) {
                let m = world(&doc, k);
                for i in 0..8 {
                    let c =
                        Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
                    pts.push(m.transform_point3(c));
                }
            }
        }
        (pts, app.active_object().map(|k| world(&doc, k).transform_point3(Vec3::ZERO)))
    };
    let mut sources = sources;
    sources.truncate(MAX_SOURCES);
    // Meshes to snap to, by the Target Selection settings.
    let s = app.ui.transform.clone();
    let geometry = s.snap_targets.iter().any(|t| !matches!(t, SnapTarget::Increment | SnapTarget::Grid));
    let mut snap_meshes = Vec::new();
    if geometry {
        let mut moving: std::collections::BTreeSet<ObjectKey> =
            objects.iter().flat_map(|(k, _, below)| std::iter::once(*k).chain(below.iter().map(|(c, _)| *c))).collect();
        // A shape never snaps onto its own mesh.
        moving.extend(app.sdf.object.filter(|_| sdf));
        for (object, mesh, model) in app.viewport.parts_list() {
            let edited = edit && Some(object) == app.edit.object;
            let take = if edit { if edited { s.include_edited } else { s.include_nonedited } } else { !moving.contains(&object) };
            if take {
                let bvh = app.viewport.bvh(&mesh);
                snap_meshes.push((mesh, bvh, model, edited));
            }
        }
    }
    Drag {
        kind,
        handle,
        modal,
        frame: f,
        start: p,
        anchor: anchor(handle, &f, &cam, rect, p),
        delta: Delta::default(),
        objects,
        edit: edit_target,
        sources,
        active,
        snap_meshes,
        snapped: None,
        typed: String::new(),
        sdf,
    }
}

/// Keys during a drag: X, Y, Z (Shift: planes) change the constraint;
/// digits, signs, points and unit letters type a value (Backspace edits).
/// Returns whether the constraint changed.
fn drag_keys(ui: &egui::Ui, drag: &mut Drag) -> bool {
    let mut changed = false;
    let events = ui.input(|i| i.events.clone());
    for e in events {
        match e {
            Event::Key { key: key @ (Key::X | Key::Y | Key::Z), pressed: true, modifiers, .. } if !modifiers.command => {
                let i = match key {
                    Key::X => 0,
                    Key::Y => 1,
                    _ => 2,
                };
                let wanted = if modifiers.shift && drag.kind != Kind::Rotate { Constraint::Plane(i) } else { Constraint::Axis(i) };
                // Pressing the same axis again frees it, as in Blender.
                let next = if constraint_of(drag.handle) == wanted { Constraint::None } else { wanted };
                drag.handle = handle_for(drag.kind, next);
                changed = true;
            }
            Event::Key { key: Key::Backspace, pressed: true, .. } => {
                drag.typed.pop();
            }
            Event::Text(text) => {
                for ch in text.chars() {
                    let unit_letter = ch.is_ascii_alphabetic() && !matches!(ch.to_ascii_lowercase(), 'x' | 'y' | 'z');
                    if ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+') || unit_letter {
                        drag.typed.push(ch);
                    }
                }
            }
            _ => {}
        }
    }
    changed
}

/// Lays out the gizmo over the viewport `rect`: hit testing, dragging,
/// modal transforms, previews, commits and drawing.
///
/// Complexity: O(selected objects and their descendants) per frame while
/// dragging in Object Mode; in Edit Mode a preview op run per pointer move.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui, rect: Rect) {
    let cam = app.viewport.camera;
    let doc = Arc::clone(app.session.doc());
    let pointer = ui.input(|i| i.pointer.hover_pos());
    // A modal transform asked for by G, R or S starts at the pointer.
    if let Some(kind) = app.gizmo.pending.take()
        && app.gizmo.drag.is_none()
        && rect.is_positive()
        && let Some(f) = frame(app, &doc, &cam)
    {
        let at = pointer.filter(|p| rect.contains(*p)).unwrap_or(rect.center());
        let handle = handle_for(kind, Constraint::None);
        let drag = begin(app, kind, handle, true, f, at);
        app.gizmo.drag = Some(drag);
    }
    let modal = app.gizmo.drag.as_ref().is_some_and(|d| d.modal);
    let kind = app.gizmo.drag.as_ref().map(|d| d.kind).or_else(|| Kind::of(app.ui.tool));
    let Some(kind) = kind.filter(|_| rect.is_positive()) else {
        cancel(app);
        return;
    };
    let frame_now = match &app.gizmo.drag {
        Some(d) => Some(d.frame),
        None => frame(app, &doc, &cam),
    };
    let Some(f) = frame_now else {
        cancel(app);
        return;
    };
    // Keys during any drag: constraints and typed values.
    if let Some(drag) = app.gizmo.drag.as_mut()
        && drag_keys(ui, drag)
    {
        drag.anchor = anchor(drag.handle, &drag.frame, &cam, rect, drag.start);
    }
    let snapping = app.ui.view.snap != ui.input(|i| i.modifiers.command);
    if modal {
        let (confirm, abort) =
            ui.input(|i| (i.key_pressed(Key::Enter) || i.pointer.primary_clicked(), i.key_pressed(Key::Escape) || i.pointer.secondary_clicked()));
        if abort {
            cancel(app);
            return;
        }
        if let Some(p) = pointer {
            update(app, &cam, rect, p, snapping);
        }
        if confirm {
            commit(app, &cam);
            return;
        }
        ui.ctx().request_repaint();
        draw(app, ui, rect, kind, &cam);
        return;
    }
    if Kind::of(app.ui.tool).is_none() && app.gizmo.drag.is_none() {
        return;
    }
    let Some(s) = shapes(kind, &f, &cam, rect) else { return };
    if app.gizmo.drag.is_none() {
        app.gizmo.hot = pointer.filter(|p| rect.contains(*p)).and_then(|p| hit(kind, &s, p));
    }
    // Take the pointer only over a handle or while dragging, so clicks and
    // boxes elsewhere still reach the viewport.
    if app.gizmo.hot.is_some() || app.gizmo.drag.is_some() {
        let resp = ui.interact(rect, ui.id().with("gizmo"), Sense::drag());
        if resp.drag_started()
            && let (Some(h), Some(p)) = (app.gizmo.hot, resp.interact_pointer_pos())
        {
            let drag = begin(app, kind, h, false, f, p);
            app.gizmo.drag = Some(drag);
        }
        let cancel_now = ui.input(|i| i.key_pressed(Key::Escape) || i.pointer.secondary_pressed());
        if app.gizmo.drag.is_some() && cancel_now {
            cancel(app);
            return;
        }
        if (resp.dragged() || app.gizmo.drag.as_ref().is_some_and(|d| !d.typed.is_empty()))
            && let Some(p) = resp.interact_pointer_pos().or(pointer)
        {
            update(app, &cam, rect, p, snapping);
        }
        if resp.drag_stopped() || (app.gizmo.drag.is_some() && ui.input(|i| i.key_pressed(Key::Enter))) {
            commit(app, &cam);
        }
    }
    draw(app, ui, rect, kind, &cam);
}

/// Recomputes the drag's delta for the pointer at `p` and previews it.
fn update(app: &mut PixelApp, cam: &OrbitCamera, rect: Rect, p: Pos2, snapping: bool) {
    let result = app.gizmo.drag.as_ref().map(|d| delta_at(d, app, cam, rect, p, snapping));
    if let (Some((dl, hit)), Some(d)) = (result, app.gizmo.drag.as_mut()) {
        d.delta = dl;
        d.snapped = hit;
    }
    preview(app, cam);
}

/// Shows the drag's result without changing the document.
fn preview(app: &mut PixelApp, cam: &OrbitCamera) {
    let Some(drag) = &app.gizmo.drag else { return };
    let view_axis = cam.toward();
    if drag.sdf {
        // The shape's numbers change; the mesh previews coarsely.
        let m = delta_matrix(&drag.delta, drag.frame.pivot, drag.frame.axes, view_axis);
        app.sdf_drag(m);
        return;
    }
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
    let label = drag.kind.label();
    let moved = drag.delta.translate != Vec3::ZERO || drag.delta.rotate.is_some_and(|(_, a)| a != 0.0) || drag.delta.scale != Vec3::ONE;
    if drag.sdf {
        let name = app.sdf.selected.as_ref().map(|p| crate::sdf::shapes::label(&app.sdf.shapes, p)).unwrap_or_default();
        let full = format!("{label} {name}");
        app.sdf_end_drag(moved.then_some(full.as_str()));
        return;
    }
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
    if let Some(d) = app.gizmo.drag.take() {
        app.viewport.preview.clear();
        app.gizmo.preview = None;
        if d.sdf {
            app.sdf_end_drag(None);
        }
    }
    app.gizmo.hot = None;
}

/// A long line through the pivot along axis `i` (the drag constraint).
#[allow(clippy::too_many_arguments)] // the drawing context and the axis
fn guide(p: &egui::Painter, f: &Frame, cam: &OrbitCamera, rect: Rect, i: usize, len: f32, color: Color32) {
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let Some(c) = project(vp, rect, f.pivot) else { return };
    for sign in [1.0, -1.0] {
        // The farthest point in front of the camera.
        let end = [400.0, 60.0, 12.0, 3.0].iter().find_map(|k| project(vp, rect, f.pivot + f.axes[i] * (sign * k * len)));
        if let Some(e) = end {
            p.line_segment([c, e], Stroke::new(1.2, color.gamma_multiply(0.85)));
        }
    }
}

/// The whole rotation ring `i` as screen segments, each marked front or back.
fn full_ring(f: &Frame, cam: &OrbitCamera, rect: Rect, i: usize, len: f32) -> Vec<(Pos2, Pos2, bool)> {
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let (u, v) = (f.axes[(i + 1) % 3], f.axes[(i + 2) % 3]);
    let to_eye = if cam.ortho { cam.toward() } else { (cam.eye() - f.pivot).normalize_or(cam.toward()) };
    let r = len * 0.9;
    let pts: Vec<(Vec3, Option<Pos2>)> = (0..=RING)
        .map(|k| {
            let a = k as f32 / RING as f32 * std::f32::consts::TAU;
            let off = (u * a.cos() + v * a.sin()) * r;
            (off, project(vp, rect, f.pivot + off))
        })
        .collect();
    pts.windows(2).filter_map(|w| Some((w[0].1?, w[1].1?, (w[0].0 + w[1].0).dot(to_eye) >= 0.0))).collect()
}

/// Paints the handles; the dragged or hovered one lights up. A modal
/// transform shows only its guides and a line from the pivot to the pointer.
fn draw(app: &PixelApp, ui: &egui::Ui, rect: Rect, kind: Kind, cam: &OrbitCamera) {
    let tk = Tokens::get();
    let Some(mut f) = app.gizmo.drag.as_ref().map(|d| d.frame).or_else(|| frame(app, app.doc(), cam)) else { return };
    if let Some(d) = &app.gizmo.drag {
        f.pivot += d.delta.translate;
    }
    let Some(s) = shapes(kind, &f, cam, rect) else { return };
    let p = ui.painter_at(rect);
    let active = app.gizmo.drag.as_ref().map(|d| d.handle).or(app.gizmo.hot);
    let colors = [tk.axis_x, tk.axis_y, tk.axis_z];
    let lit = |c: Color32, on: bool| if on { c.lerp_to_gamma(Color32::WHITE, 0.45) } else { c };
    let width = |on: bool| if on { 3.0 } else { 2.0 };
    let dragged = app.gizmo.drag.as_ref().map(|d| d.handle);
    let modal = app.gizmo.drag.as_ref().is_some_and(|d| d.modal);
    // While dragging only the grabbed direction shows, as in Blender.
    let shown = |h: Handle| !modal && dragged.is_none_or(|d| d == h);
    let len = SIZE * meters_per_point(cam, rect, f.pivot);
    match dragged.map(constraint_of) {
        Some(Constraint::Axis(i)) => guide(&p, &f, cam, rect, i, len, colors[i]),
        Some(Constraint::Plane(i)) => {
            for j in [(i + 1) % 3, (i + 2) % 3] {
                guide(&p, &f, cam, rect, j, len, colors[j]);
            }
        }
        _ => {}
    }
    match kind {
        Kind::Move | Kind::Scale => {
            for (i, (&(a, b, visible), color)) in s.axes.iter().zip(colors).enumerate() {
                if !visible || !shown(Handle::Axis(i)) {
                    continue;
                }
                let on = active == Some(Handle::Axis(i));
                let c = lit(color, on);
                p.line_segment([a, b], Stroke::new(width(on), c));
                let dir = (b - a).normalized();
                if kind == Kind::Move {
                    let side = vec2(-dir.y, dir.x);
                    p.add(egui::Shape::convex_polygon(vec![b + dir * 12.0, b + side * 5.0, b - side * 5.0], c, Stroke::NONE));
                } else {
                    p.rect_filled(Rect::from_center_size(b, vec2(9.0, 9.0)), 2.0, c);
                }
            }
            if kind == Kind::Move {
                for (i, (plane, color)) in s.planes.iter().zip(colors).enumerate() {
                    if let (Some(q), true) = (plane, shown(Handle::Plane(i))) {
                        let on = active == Some(Handle::Plane(i));
                        p.add(egui::Shape::convex_polygon(q.to_vec(), lit(color, on).gamma_multiply(if on { 0.8 } else { 0.45 }), Stroke::NONE));
                    }
                }
                if shown(Handle::Free) {
                    let on = active == Some(Handle::Free);
                    p.circle_stroke(s.center, 6.0, Stroke::new(width(on), lit(Color32::from_gray(225), on)));
                }
            } else if shown(Handle::Uniform) {
                let on = active == Some(Handle::Uniform);
                p.circle_stroke(s.center, s.outer, Stroke::new(width(on), lit(Color32::from_gray(200), on)));
            }
        }
        Kind::Rotate => {
            for (i, (ring, color)) in s.rings.iter().zip(colors).enumerate() {
                if dragged == Some(Handle::Ring(i)) {
                    // The whole ring while turning, the back half dimmer.
                    for (a, b, front) in full_ring(&f, cam, rect, i, len) {
                        p.line_segment(
                            [a, b],
                            Stroke::new(if front { 3.0 } else { 1.5 }, lit(color, front).gamma_multiply(if front { 1.0 } else { 0.5 })),
                        );
                    }
                } else if shown(Handle::Ring(i)) {
                    let on = active == Some(Handle::Ring(i));
                    for (a, b) in ring {
                        p.line_segment([*a, *b], Stroke::new(width(on) + 0.5, lit(color, on)));
                    }
                }
            }
            if shown(Handle::ViewRing) {
                let on = active == Some(Handle::ViewRing);
                p.circle_stroke(s.center, s.outer, Stroke::new(width(on), lit(Color32::from_gray(200), on)));
            }
        }
    }
    // While turning or scaling, a line from the pivot to the pointer.
    if dragged.is_some()
        && (kind != Kind::Move || modal)
        && let Some(at) = ui.input(|i| i.pointer.hover_pos())
    {
        p.extend(egui::Shape::dashed_line(&[s.center, at], Stroke::new(1.0, Color32::from_gray(210)), 5.0, 4.0));
    }
    // The snap target.
    if let Some(h) = app.gizmo.drag.as_ref().and_then(|d| d.snapped)
        && let Some(at) = project(cam.view_proj(rect.width() / rect.height().max(1.0)), rect, h.point)
    {
        p.circle_stroke(at, 7.0, Stroke::new(2.0, tk.snap));
        p.circle_filled(at, 2.5, tk.snap);
    }
    // The value being dragged, next to the pointer.
    if let (Some(text), Some(at)) = (status_text(app), ui.input(|i| i.pointer.hover_pos())) {
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

    fn setup() -> (PixelApp, OrbitCamera, Rect) {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        app.run("ui.tool", json!({ "tool": "move" })).unwrap();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        cam.frame(Vec3::splat(-1.0), Vec3::splat(1.0), 800.0 / 600.0);
        app.viewport.camera = cam;
        app.viewport.rect = rect;
        let doc = Arc::clone(app.session.doc());
        app.viewport.scene(&doc, app.scene_key(), &Default::default());
        (app, cam, rect)
    }

    #[test]
    fn dragging_an_axis_moves_the_selection_in_one_step() {
        let (mut app, cam, rect) = setup();
        let doc = Arc::clone(app.session.doc());
        let f = frame(&app, &doc, &cam).unwrap();
        let s = shapes(Kind::Move, &f, &cam, rect).unwrap();
        // The X arrow is under its tip.
        let tip = s.axes[0].1;
        assert_eq!(hit(Kind::Move, &s, tip), Some(Handle::Axis(0)));
        assert_eq!(hit(Kind::Move, &s, s.center), Some(Handle::Free));
        let mut drag = begin(&mut app, Kind::Move, Handle::Axis(0), false, f, tip);
        // Drag 100 points to the right with snapping on: whole grid cells.
        drag.delta = delta_at(&drag, &app, &cam, rect, tip + egui::vec2(100.0, 0.0), true).0;
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

    #[test]
    fn typed_values_and_constraints_override_the_pointer() {
        let (mut app, cam, rect) = setup();
        let doc = Arc::clone(app.session.doc());
        let f = frame(&app, &doc, &cam).unwrap();
        // G Z 20cm: 0.2 m along Z.
        let mut drag = begin(&mut app, Kind::Move, handle_for(Kind::Move, Constraint::Axis(2)), true, f, rect.center());
        drag.typed = "20cm".into();
        assert!((delta_at(&drag, &app, &cam, rect, rect.center(), false).0.translate - Vec3::new(0.0, 0.0, 0.2)).length() < 1e-6);
        // R X 45: a quarter turn's half around X.
        let mut drag = begin(&mut app, Kind::Rotate, handle_for(Kind::Rotate, Constraint::Axis(0)), true, f, rect.center());
        drag.typed = "45".into();
        let (d, _) = delta_at(&drag, &app, &cam, rect, rect.center(), false);
        assert_eq!(d.rotate.map(|(i, a)| (i, (a.to_degrees() * 1000.0).round() / 1000.0)), Some((Some(0), 45.0)));
        // S 0.5: half size everywhere; a lone sign is still being typed.
        let mut drag = begin(&mut app, Kind::Scale, handle_for(Kind::Scale, Constraint::None), true, f, rect.center());
        drag.typed = "0.5".into();
        assert_eq!(delta_at(&drag, &app, &cam, rect, rect.center(), false).0.scale, Vec3::splat(0.5));
        assert_eq!(typed_value(Kind::Move, "-"), Some(0.0));
        assert_eq!(typed_value(Kind::Move, "2 parsecs"), None);
        // The tool settings row reads the drag.
        app.gizmo.drag = Some(drag);
        assert!(status_text(&app).is_some_and(|s| s.starts_with("Scale") && s.contains("[0.5]")));
    }

    #[test]
    fn moving_snaps_the_base_point_onto_vertices() {
        let (mut app, cam, rect) = setup();
        // A second box to snap to, off to the right.
        app.run("prim.box", json!({ "name": "post", "position": [1.5, 0, 0] })).unwrap();
        app.run("edit.select", json!({ "target": "crate" })).unwrap();
        app.ui.transform.apply(&json!({ "snapTarget": "vertex", "snapBase": "closest" })).unwrap();
        app.ui.transform.apply(&json!({ "snapTarget": "increment" })).unwrap();
        let doc = Arc::clone(app.session.doc());
        app.viewport.scene(&doc, app.scene_key(), &Default::default());
        let f = frame(&app, &doc, &cam).unwrap();
        let drag = begin(&mut app, Kind::Move, Handle::Free, false, f, rect.center());
        assert!(!drag.snap_meshes.is_empty(), "the other box is a target");
        // Point at the post's lower left front corner.
        let post = doc.resolve("post").unwrap();
        let (lo, hi) = doc.object_mesh(post).and_then(|m| m.bounds()).unwrap();
        let corner = world(&doc, post).transform_point3(Vec3::new(lo.x, lo.y, hi.z));
        let at = project(cam.view_proj(rect.width() / rect.height()), rect, corner).unwrap() + egui::vec2(3.0, -3.0);
        let (d, hitp) = delta_at(&drag, &app, &cam, rect, at, true);
        let h = hitp.unwrap();
        assert_eq!(h.target, SnapTarget::Vertex);
        assert!((h.point - corner).length() < 1e-4);
        // The crate's nearest corner lands exactly on it.
        let base = snap_base(&drag, SnapBase::Closest, h.point);
        assert!((base + d.translate - corner).length() < 1e-4);
    }

    #[test]
    fn gimbal_and_cursor_orientations_and_the_active_element() {
        let (mut app, cam, _) = setup();
        app.run("scene.set_transform", json!({ "target": "crate", "rotation": [0, 90, 0] })).unwrap();
        let doc = Arc::clone(app.session.doc());
        let k = doc.resolve("crate").unwrap();
        let g = gimbal_axes(&doc, k).unwrap();
        // 90 degrees about Y: the gimbal X turns to -Z; Y and Z stay.
        assert!((g[0] - Vec3::NEG_Z).length() < 1e-5 && (g[1] - Vec3::Y).length() < 1e-5 && (g[2] - Vec3::Z).length() < 1e-5);
        app.ui.transform.apply(&json!({ "orientation": "cursor", "cursorRotation": [0, 0, 90] })).unwrap();
        let f = frame(&app, &doc, &cam).unwrap();
        assert!((f.axes[0] - Vec3::Y).length() < 1e-5, "the cursor's X");
        // Edit Mode: the Active Element pivot is the last picked vertex.
        app.run("ui.mode", json!({ "mode": "vert" })).unwrap();
        let v = doc.object_mesh(k).and_then(|m| m.verts().last()).unwrap();
        app.edit.click(Some(0), false);
        app.edit.click(Some(v), true);
        app.edit.overlay(0, crate::SelectMode::Vert, None);
        app.ui.transform.apply(&json!({ "pivot": "active", "orientation": "global" })).unwrap();
        let f = frame(&app, &doc, &cam).unwrap();
        let want = world(&doc, k).transform_point3(doc.object_mesh(k).unwrap().positions[v as usize]);
        assert!((f.pivot - want).length() < 1e-5);
    }
}
