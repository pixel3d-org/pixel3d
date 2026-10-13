//! The Handles tool (H): sculpting by numbers in the viewport. A click on
//! the active mesh places a handle, Shift+click a pin; dragging a handle (or
//! straight from the surface) pulls it in the view plane while the surface
//! within the radius follows, previewed on a background thread with the
//! `deform.handles` kernel. The radius shows as a ring on the surface;
//! `[` and `]`, the mouse wheel while dragging, or the card's slider change
//! it. On release the pull is one undo step: `deform.handles` on a mesh, or
//! for a Model Spec part a `spec.patch` that adds the handle to the part's
//! `deform` block (part space; a landmark's name when the press lands on
//! one), so recompiles keep it. A part's handles and pins show as markers:
//! drag one to change its offset, Delete removes the selected one.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use egui::{Pos2, Stroke, pos2, vec2};
use glam::{Affine3A, Mat4, Vec3};
use p3d_engine::{JobStatus, OpCall};
use p3d_scene::{Document, ObjectKey};
use serde_json::{Map, Value, json};

use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::{PixelApp, Tool};

/// A point placed with the tool and not in any op yet (object space): a
/// handle to drag, or a pin held by the next pulls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub at: Vec3,
    pub pin: bool,
}

/// A marker in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// A placed point, by index.
    Placed(usize),
    /// A handle or a pin of the spec part's `deform` block.
    Handle(usize),
    Pin(usize),
}

impl Marker {
    fn kind(self) -> &'static str {
        match self {
            Marker::Placed(_) => "placed",
            Marker::Handle(_) => "handle",
            Marker::Pin(_) => "pin",
        }
    }

    fn index(self) -> usize {
        match self {
            Marker::Placed(i) | Marker::Handle(i) | Marker::Pin(i) => i,
        }
    }
}

/// A pull in progress.
#[derive(Debug, Clone, PartialEq)]
struct Drag {
    /// What is pulled: a marker, or the surface (a new handle).
    marker: Option<Marker>,
    /// The grabbed point (world), and the normal of the plane it moves in.
    start: Vec3,
    normal: Vec3,
    /// The handle's point in object space, or the landmark it is on.
    at: Vec3,
    landmark: Option<String>,
    /// Where the pointer has it now (world).
    now: Vec3,
}

/// The background preview: one run at a time, the newest waiting.
#[derive(Default)]
struct Preview {
    wanted: Option<OpCall>,
    running: Option<Receiver<Result<Arc<Document>, String>>>,
    doc: Option<(Arc<Document>, u64)>,
    counter: u64,
}

/// The tool between frames.
#[derive(Default)]
pub struct HandleTool {
    /// The object the placed points belong to.
    pub object: Option<ObjectKey>,
    pub points: Vec<Placed>,
    /// Reach along the surface in object space; 40% of the mesh's diagonal
    /// when `None`.
    pub radius: Option<f32>,
    drag: Option<Drag>,
    pub selected: Option<Marker>,
    preview: Preview,
    commit: Option<p3d_engine::Job>,
    queued: Option<(Vec<OpCall>, String)>,
    pub error: Option<String>,
    /// Markers on screen last frame (for agents).
    pub screen: Vec<(Marker, Pos2)>,
    /// The surface under the pointer (world point and normal), and where
    /// and in which scene it was asked.
    hover: Option<(Vec3, Vec3)>,
    hover_key: Option<(Pos2, u64)>,
}

impl HandleTool {
    pub fn busy(&self) -> bool {
        self.commit.is_some() || self.queued.is_some() || self.preview.running.is_some()
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn preview_doc(&self) -> Option<(Arc<Document>, u64)> {
        self.preview.doc.clone()
    }
}

/// A coordinate rounded to a hundredth of a millimetre (spec text stays short).
fn round(v: f32) -> f64 {
    (f64::from(v) * 1e5).round() / 1e5
}

fn triple(v: Vec3) -> Value {
    json!([round(v.x), round(v.y), round(v.z)])
}

/// A `[x, y, z]` of lengths.
fn vec3_of(v: &Value) -> Option<Vec3> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    let l = |v: &Value| serde_json::from_value::<p3d_ops::params::Length>(v.clone()).ok().map(|l| l.0 as f32);
    Some(Vec3::new(l(&a[0])?, l(&a[1])?, l(&a[2])?))
}

/// A world point on the viewport, or none behind the camera.
fn project(vp: Mat4, rect: egui::Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

impl PixelApp {
    /// The mesh the tool works on: the active object in Object Mode (a spec
    /// root works on its selected part's object), and the spec part it is.
    pub(crate) fn handles_target(&self) -> Option<(ObjectKey, Option<(ObjectKey, String)>)> {
        if self.ui.mode.is_edit() {
            return None;
        }
        let doc = self.doc();
        let active = self.active_object()?;
        let k = match crate::spec::spec_of(doc, active) {
            Some((root, None)) => crate::spec::part_object(doc, root, self.spec.part.as_deref()?)?,
            _ => active,
        };
        doc.object_mesh(k)?;
        let part = match crate::spec::spec_of(doc, k) {
            Some((root, Some(part))) => Some((root, part)),
            _ => None,
        };
        Some((k, part))
    }

    /// The part's `deform` block as written (an empty one when none).
    fn handles_deform(&self, part: &Option<(ObjectKey, String)>) -> Map<String, Value> {
        part.as_ref()
            .and_then(|(root, name)| crate::spec::part_source(self.doc(), *root, name))
            .and_then(|p| p.get("deform").and_then(Value::as_object).cloned())
            .unwrap_or_default()
    }

    /// The radius in object space: set, else the part's, else 40% of the
    /// mesh's diagonal.
    pub fn handles_radius(&self, k: ObjectKey) -> f32 {
        if let Some(r) = self.handles.radius {
            return r;
        }
        self.doc().object_mesh(k).and_then(|m| m.bounds()).map_or(0.1, |(lo, hi)| (hi - lo).length() * 0.4)
    }

    /// Follows the target: a new object starts with no placed points and
    /// its part's radius.
    fn handles_follow(&mut self) {
        let target = self.handles_target();
        if target.as_ref().map(|t| t.0) != self.handles.object {
            self.handles.object = target.as_ref().map(|t| t.0);
            self.handles.points.clear();
            self.handles.selected = None;
            self.handles.drag = None;
            self.handles.radius = target.as_ref().and_then(|(_, part)| {
                let d = self.handles_deform(part);
                serde_json::from_value::<p3d_ops::params::Length>(d.get("radius")?.clone()).ok().map(|l| l.0 as f32)
            });
        }
    }

    /// Every marker with its world point and, for a part's handle, where it
    /// was before its offset.
    ///
    /// Complexity: O(markers) plus the part's landmarks when not cached.
    fn handle_markers(&mut self) -> Vec<(Marker, Vec3, Option<Vec3>)> {
        let Some((k, part)) = self.handles_target() else { return Vec::new() };
        let world = self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY);
        let mut out: Vec<(Marker, Vec3, Option<Vec3>)> =
            self.handles.points.iter().enumerate().map(|(i, p)| (Marker::Placed(i), world.transform_point3(p.at), None)).collect();
        let Some((_, name)) = &part else { return out };
        let deform = self.handles_deform(&part);
        let marks = self.spec_landmarks_of(name);
        let doc = Arc::clone(self.session.doc());
        // A named point: a landmark, else a face group's center.
        let named = |n: &str| -> Option<Vec3> {
            if let Some(l) = marks.iter().find(|l| l.name == n) {
                return Some(l.point);
            }
            let m = doc.object_mesh(k)?;
            let g = m.group(n)?;
            let vs: Vec<Vec3> = g.iter().flat_map(|f| m.face_verts(f).collect::<Vec<_>>()).map(|v| m.positions[v as usize]).collect();
            (!vs.is_empty()).then(|| world.transform_point3(vs.iter().sum::<Vec3>() / vs.len() as f32))
        };
        for (i, h) in deform.get("handles").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let by = h.get("by").and_then(vec3_of).unwrap_or(Vec3::ZERO);
            match h.get("move") {
                Some(Value::String(n)) => {
                    if let Some(p) = named(n) {
                        out.push((Marker::Handle(i), p, None));
                    }
                }
                Some(v) => {
                    if let Some(at) = vec3_of(v) {
                        out.push((Marker::Handle(i), world.transform_point3(at + by), Some(world.transform_point3(at))));
                    }
                }
                None => {}
            }
        }
        for (i, q) in deform.get("pins").and_then(Value::as_array).into_iter().flatten().enumerate() {
            let p = match q {
                Value::String(n) => named(n),
                v => vec3_of(v).map(|at| world.transform_point3(at)),
            };
            if let Some(p) = p {
                out.push((Marker::Pin(i), p, None));
            }
        }
        out
    }

    /// The surface of the target under `at`: world point and normal.
    ///
    /// Complexity: O(parts x log F) for the ray.
    fn handles_surface(&mut self, at: Pos2) -> Option<(Vec3, Vec3)> {
        let (k, _) = self.handles_target()?;
        let (hit, face, point) = self.viewport.hit_at(at)?;
        if hit != k {
            return None;
        }
        let world = self.doc().world_affine(k)?;
        let n = self.doc().object_mesh(k).map_or(Vec3::Y, |m| m.face_normal(face));
        Some((point, world.transform_vector3(n).normalize_or(Vec3::Y)))
    }

    /// The point in the drag plane under the pointer.
    fn handles_plane_point(&self, d: &Drag, at: Pos2) -> Option<Vec3> {
        let r = self.viewport.rect;
        let (o, dir) = self.viewport.camera.ray(at.x - r.left(), at.y - r.top(), r.width(), r.height());
        let den = dir.dot(d.normal);
        if den.abs() < 1e-6 {
            return None;
        }
        let t = (d.start - o).dot(d.normal) / den;
        (t > 0.0).then(|| o + dir * t)
    }

    /// Starts a pull at `at`: on a marker, else on the target's surface (a
    /// part's landmark there is used by name).
    fn handles_begin(&mut self, at: Pos2) {
        let Some((k, part)) = self.handles_target() else { return };
        let world = self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY);
        let local = world.inverse();
        let rect = self.viewport.rect;
        let vp = self.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
        let toward = -self.viewport.camera.ray(at.x - rect.left(), at.y - rect.top(), rect.width(), rect.height()).1;
        let markers = self.handle_markers();
        let near = markers
            .iter()
            .filter(|(m, ..)| !matches!(m, Marker::Pin(_)) && !matches!(m, Marker::Placed(i) if self.handles.points.get(*i).is_some_and(|p| p.pin)))
            .filter_map(|(m, p, _)| project(vp, rect, *p).map(|s| (*m, *p, s.distance(at))))
            .filter(|(.., d)| *d <= 9.0)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        let drag = if let Some((m, p, _)) = near {
            Drag { marker: Some(m), start: p, normal: toward, at: local.transform_point3(p), landmark: None, now: p }
        } else {
            let Some((p, _)) = self.handles_surface(at) else { return };
            // On a part, a press on one of its landmarks pulls the landmark.
            let landmark = part.as_ref().and_then(|(_, name)| {
                let marks = self.spec_landmarks_of(name);
                marks
                    .iter()
                    .filter_map(|l| project(vp, rect, l.point).map(|s| (l, s.distance(at))))
                    .filter(|(_, d)| *d <= 9.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(l, _)| (l.name.clone(), l.point))
            });
            let start = landmark.as_ref().map_or(p, |l| l.1);
            Drag { marker: None, start, normal: toward, at: local.transform_point3(start), landmark: landmark.map(|l| l.0), now: start }
        };
        self.handles.selected = drag.marker;
        self.handles.drag = Some(drag);
        self.handles.error = None;
    }

    /// The pull as an offset in object space.
    fn handles_offset(&self, d: &Drag) -> Vec3 {
        let k = self.handles.object;
        let local = k.and_then(|k| self.doc().world_affine(k)).unwrap_or(Affine3A::IDENTITY).inverse();
        local.transform_vector3(d.now - d.start)
    }

    /// The pins of the next pull, in object space: placed pins, and a
    /// part's pins given as coordinates.
    fn handles_pins(&self, part: &Option<(ObjectKey, String)>) -> Vec<Value> {
        let mut pins: Vec<Value> = self.handles.points.iter().filter(|p| p.pin).map(|p| triple(p.at)).collect();
        if part.is_some() {
            let d = self.handles_deform(part);
            pins.extend(d.get("pins").and_then(Value::as_array).into_iter().flatten().filter(|v| v.is_array()).cloned());
        }
        pins
    }

    /// A `deform.handles` call pulling `at` (object space) by `by`.
    fn handles_call(&self, k: ObjectKey, at: Vec3, by: Vec3, part: &Option<(ObjectKey, String)>) -> Option<OpCall> {
        let id = self.doc().object(k)?.public_id();
        let mut args = json!({ "target": id, "handles": [{ "at": triple(at), "by": triple(by) }], "space": "local",
            "radius": round(self.handles_radius(k)) });
        let pins = self.handles_pins(part);
        if !pins.is_empty() {
            args["pins"] = Value::Array(pins);
        }
        Some(OpCall::new("deform.handles", args))
    }

    /// Previews the pull in progress in the background.
    fn handles_preview(&mut self) {
        let (Some(d), Some((k, part))) = (self.handles.drag.clone(), self.handles_target()) else { return };
        let by = self.handles_offset(&d);
        if by == Vec3::ZERO {
            return;
        }
        // A part's handle already moved its point: pull from where it is.
        let at = match d.marker {
            Some(Marker::Handle(_)) => self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY).inverse().transform_point3(d.start),
            _ => d.at,
        };
        self.handles.preview.wanted = self.handles_call(k, at, by, &part);
        self.handles_start_preview();
    }

    fn handles_start_preview(&mut self) {
        if self.handles.preview.running.is_some() {
            return;
        }
        let Some(call) = self.handles.preview.wanted.take() else { return };
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::clone(self.session.registry());
        let doc = Document::clone(self.session.doc());
        let spawned = std::thread::Builder::new().name("handles preview".into()).spawn(move || {
            let mut work = p3d_engine::Session::with_registry(registry);
            work.load(doc);
            let r = work.execute(&[call], &Default::default()).map(|_| Arc::clone(work.doc())).map_err(|e| e.error.to_string());
            let _ = tx.send(r);
        });
        if spawned.is_ok() {
            self.handles.preview.running = Some(rx);
        }
    }

    /// Ends the pull: one undo step (the op on a mesh, a spec patch on a
    /// part); a pull of nothing changes nothing.
    fn handles_commit(&mut self) {
        let Some(d) = self.handles.drag.take() else { return };
        let Some((k, part)) = self.handles_target() else { return };
        let by = self.handles_offset(&d);
        if by.length() < 1e-7 {
            self.handles.preview.doc = None;
            return;
        }
        let label = t("Pull handle");
        let call = match &part {
            None => {
                let call = self.handles_call(k, d.at, by, &part);
                // The handle stays where it was pulled to, ready for more.
                match d.marker {
                    Some(Marker::Placed(i)) => {
                        if let Some(p) = self.handles.points.get_mut(i) {
                            p.at = d.at + by;
                        }
                    }
                    _ => {
                        self.handles.points.push(Placed { at: d.at + by, pin: false });
                        self.handles.selected = Some(Marker::Placed(self.handles.points.len() - 1));
                    }
                }
                call
            }
            Some((root, name)) => {
                let mut deform = self.handles_deform(&part);
                let mut handles = deform.get("handles").and_then(Value::as_array).cloned().unwrap_or_default();
                match d.marker {
                    Some(Marker::Handle(i)) if i < handles.len() => {
                        let old = handles[i].get("by").and_then(vec3_of).unwrap_or(Vec3::ZERO);
                        handles[i]["by"] = triple(old + by);
                    }
                    other => {
                        let at = d.landmark.clone().map_or_else(|| triple(d.at), Value::String);
                        handles.push(json!({ "move": at, "by": triple(by) }));
                        if let Some(Marker::Placed(i)) = other
                            && i < self.handles.points.len()
                        {
                            self.handles.points.remove(i);
                        }
                        self.handles.selected = Some(Marker::Handle(handles.len() - 1));
                    }
                }
                deform.insert("handles".into(), Value::Array(handles));
                if let Some(r) = self.handles.radius {
                    deform.insert("radius".into(), json!(round(r)));
                }
                self.handles_patch(*root, name, deform)
            }
        };
        if let Some(call) = call {
            self.handles.queued = Some((vec![call], label.to_owned()));
            self.handles_start_commit();
        }
    }

    /// A spec patch setting a part's `deform` block (removed when empty).
    fn handles_patch(&self, root: ObjectKey, part: &str, deform: Map<String, Value>) -> Option<OpCall> {
        let id = self.doc().object(root)?.public_id();
        let empty = deform.get("handles").and_then(Value::as_array).is_none_or(Vec::is_empty);
        let value = if empty { Value::Null } else { Value::Object(deform) };
        let merge = json!({ "parts": { part: { "deform": value } } });
        Some(OpCall::new("spec.patch", json!({ "target": id, "merge": merge })))
    }

    fn handles_start_commit(&mut self) {
        if self.handles.commit.is_some() {
            return;
        }
        if let Some((calls, label)) = self.handles.queued.take() {
            self.handles.commit = Some(self.session.spawn_job(calls, &label));
        }
    }

    /// Places a handle (or a pin) on the target's surface at `at`; a click
    /// on a marker selects it, on another mesh selects that object.
    fn handles_click(&mut self, at: Pos2, pin: bool) {
        let rect = self.viewport.rect;
        let vp = self.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
        let markers = self.handle_markers();
        if let Some((m, ..)) = markers.iter().filter(|(_, p, _)| project(vp, rect, *p).is_some_and(|s| s.distance(at) <= 9.0)).min_by(|a, b| {
            let d = |p: Vec3| project(vp, rect, p).map_or(f32::MAX, |s| s.distance(at));
            d(a.1).total_cmp(&d(b.1))
        }) {
            self.handles.selected = Some(*m);
            return;
        }
        let Some((k, part)) = self.handles_target() else { return };
        match self.handles_surface(at) {
            Some((p, _)) => {
                let local = self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY).inverse().transform_point3(p);
                match (&part, pin) {
                    (Some((root, name)), true) => {
                        let mut deform = self.handles_deform(&part);
                        let mut pins = deform.get("pins").and_then(Value::as_array).cloned().unwrap_or_default();
                        pins.push(triple(local));
                        deform.insert("pins".into(), Value::Array(pins));
                        // Pins alone do nothing: kept with the next handle.
                        if deform.get("handles").and_then(Value::as_array).is_some_and(|h| !h.is_empty()) {
                            if let Some(call) = self.handles_patch(*root, name, deform) {
                                self.handles.queued = Some((vec![call], t("Pin").to_owned()));
                                self.handles_start_commit();
                            }
                        } else {
                            self.handles.points.push(Placed { at: local, pin: true });
                        }
                    }
                    _ => {
                        self.handles.points.push(Placed { at: local, pin });
                        self.handles.selected = Some(Marker::Placed(self.handles.points.len() - 1));
                    }
                }
            }
            None => {
                // Another object under the pointer: select it.
                if let Some((other, ..)) = self.viewport.hit_at(at) {
                    self.handle_viewport(vec![crate::viewport::ViewportEvent::Picked { object: Some(other), add: false }]);
                } else {
                    self.handles.selected = None;
                }
            }
        }
    }

    /// Deletes a marker: a placed point, or a part's handle or pin.
    pub(crate) fn handles_delete(&mut self, m: Marker) -> Result<(), String> {
        let (_, part) = self.handles_target().ok_or(t("select a mesh in Object Mode"))?;
        match m {
            Marker::Placed(i) => {
                if i >= self.handles.points.len() {
                    return Err(t("no such point").to_owned());
                }
                self.handles.points.remove(i);
            }
            Marker::Handle(i) | Marker::Pin(i) => {
                let (root, name) = part.clone().ok_or(t("only spec parts keep handles"))?;
                let mut deform = self.handles_deform(&part);
                let key = if matches!(m, Marker::Handle(_)) { "handles" } else { "pins" };
                let mut list = deform.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
                if i >= list.len() {
                    return Err(t("no such handle").to_owned());
                }
                list.remove(i);
                deform.insert(key.into(), Value::Array(list));
                if let Some(call) = self.handles_patch(root, &name, deform) {
                    self.handles.queued = Some((vec![call], t("Delete handle").to_owned()));
                    self.handles_start_commit();
                }
            }
        }
        self.handles.selected = None;
        Ok(())
    }

    /// Per frame: takes in finished runs and follows the target.
    ///
    /// Complexity: O(1) besides finished runs.
    pub(crate) fn handles_tick(&mut self, ctx: &egui::Context) {
        if let Some(mut job) = self.handles.commit.take() {
            match self.session.poll_job(&mut job) {
                JobStatus::Pending(_) => self.handles.commit = Some(job),
                JobStatus::Done(r) => {
                    self.committed(&r);
                    if self.handles.drag.is_none() && self.handles.queued.is_none() {
                        self.handles.preview.doc = None;
                    }
                    self.handles_start_commit();
                }
                JobStatus::Failed(e) => {
                    let msg = e.error.to_string();
                    self.set_status(msg.clone());
                    self.handles.error = Some(msg);
                    self.handles.preview.doc = None;
                    self.handles_start_commit();
                }
            }
        }
        if let Some(rx) = &self.handles.preview.running {
            match rx.try_recv() {
                Ok(r) => {
                    self.handles.preview.running = None;
                    match r {
                        Ok(doc) if self.handles.drag.is_some() || self.handles.commit.is_some() || self.handles.queued.is_some() => {
                            self.handles.preview.counter += 1;
                            self.handles.preview.doc = Some((doc, self.handles.preview.counter));
                        }
                        Ok(_) => {}
                        Err(e) => self.handles.error = Some(e),
                    }
                    self.handles_start_preview();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.handles.preview.running = None,
            }
        }
        if self.handles.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.handles_follow();
    }

    /// Runs a `handles.*` command. Points and offsets are world
    /// coordinates in meters.
    pub(crate) fn handles_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let (k, part) = self.handles_target().ok_or(t("select a mesh (or a spec part) in Object Mode"))?;
        self.handles_follow();
        let world = self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY);
        let local = world.inverse();
        let point = |key: &str| params.get(key).and_then(vec3_of);
        match id {
            "handles.add" | "handles.pin" => {
                let at = point("at").ok_or(t("at is required: [x, y, z] in meters (world)"))?;
                let pin = id == "handles.pin";
                if pin && let Some((root, name)) = &part {
                    let mut deform = self.handles_deform(&part);
                    let mut pins = deform.get("pins").and_then(Value::as_array).cloned().unwrap_or_default();
                    pins.push(triple(local.transform_point3(at)));
                    deform.insert("pins".into(), Value::Array(pins));
                    if deform.get("handles").and_then(Value::as_array).is_some_and(|h| !h.is_empty()) {
                        let call = self.handles_patch(*root, name, deform).ok_or(t("the spec root is gone"))?;
                        self.handles.queued = Some((vec![call], t("Pin").to_owned()));
                        self.handles_start_commit();
                        return Ok(json!({ "pinned": "spec" }));
                    }
                }
                self.handles.points.push(Placed { at: local.transform_point3(at), pin });
                let i = self.handles.points.len() - 1;
                if let (false, Some(by)) = (pin, point("by")) {
                    return self.handles_pull(Marker::Placed(i), by);
                }
                self.handles.selected = Some(Marker::Placed(i));
                Ok(json!({ "placed": i, "pin": pin }))
            }
            "handles.move" => {
                let by = point("by").ok_or(t("by is required: [dx, dy, dz] in meters (world)"))?;
                let i = params.get("index").and_then(Value::as_u64).map(|i| i as usize);
                let m = match (params.get("kind").and_then(Value::as_str), i) {
                    (Some("handle"), Some(i)) => Marker::Handle(i),
                    (_, Some(i)) => Marker::Placed(i),
                    (_, None) => self.handles.selected.ok_or(t("index is required"))?,
                };
                self.handles_pull(m, by)
            }
            "handles.radius" => {
                let r = params.get("radius").cloned().unwrap_or(Value::Null);
                self.handles.radius = if r.is_null() {
                    None
                } else {
                    let l = serde_json::from_value::<p3d_ops::params::Length>(r).map_err(|e| e.to_string())?;
                    Some((l.0 as f32).max(1e-4))
                };
                Ok(json!({ "radius": self.handles.radius, "used": self.handles_radius(k) }))
            }
            "handles.delete" => {
                let i = params.get("index").and_then(Value::as_u64).map(|i| i as usize).ok_or(t("index is required"))?;
                let m = match params.get("kind").and_then(Value::as_str).unwrap_or("placed") {
                    "handle" => Marker::Handle(i),
                    "pin" => Marker::Pin(i),
                    _ => Marker::Placed(i),
                };
                self.handles_delete(m)?;
                Ok(json!({ "deleted": m.kind(), "index": i }))
            }
            "handles.clear" => {
                self.handles.points.clear();
                self.handles.selected = None;
                Ok(json!({ "cleared": true }))
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// Pulls a marker by a world offset at once (one undo step).
    fn handles_pull(&mut self, m: Marker, by: Vec3) -> Result<Value, String> {
        let markers = self.handle_markers();
        let (_, p, _) = markers.into_iter().find(|(x, ..)| *x == m).ok_or(t("no such handle"))?;
        let k = self.handles.object.ok_or(t("no target"))?;
        let local = self.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY).inverse();
        self.handles.drag = Some(Drag { marker: Some(m), start: p, normal: Vec3::Z, at: local.transform_point3(p), landmark: None, now: p + by });
        self.handles_commit();
        Ok(json!({ "pulled": m.kind(), "index": m.index() }))
    }

    /// The tool's state for agents (`ui.inspect`).
    ///
    /// Complexity: O(markers + vertices of the mesh).
    pub(crate) fn handles_state(&self) -> Value {
        let doc = self.doc();
        let h = &self.handles;
        let k = h.object;
        let mesh = k
            .and_then(|k| doc.object_mesh(k))
            .map(|m| json!({ "vertices": m.vert_count(), "bounds": m.bounds().map(|(lo, hi)| [lo.to_array(), hi.to_array()]) }));
        json!({
            "active": self.ui.tool == Tool::Handles,
            "object": k.and_then(|k| doc.object(k)).map(|o| o.public_id()),
            "radius": k.map(|k| self.handles_radius(k)),
            "radiusSet": h.radius,
            "placed": h.points.iter().map(|p| json!({ "at": p.at.to_array(), "pin": p.pin })).collect::<Vec<_>>(),
            "markers": h.screen.iter().map(|(m, at)| json!({ "kind": m.kind(), "index": m.index(), "at": [at.x, at.y] })).collect::<Vec<_>>(),
            "selected": h.selected.map(|m| json!({ "kind": m.kind(), "index": m.index() })),
            "hover": h.hover.map(|(p, _)| p.to_array()),
            "dragging": h.drag.is_some(),
            "previewing": h.preview.doc.is_some(),
            "busy": h.busy(),
            "error": h.error,
            "mesh": mesh,
        })
    }
}

/// The tool in the viewport: the radius ring under the pointer, the
/// markers, and the clicks and pulls (Object Mode, Handles tool).
///
/// Complexity: O(markers + ring points) per frame, plus a ray cast when the
/// pointer moves.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui) {
    let rect = app.viewport.rect;
    app.handles.screen.clear();
    if app.ui.tool != Tool::Handles || app.ui.mode.is_edit() || !rect.is_positive() {
        app.handles.hover = None;
        return;
    }
    let input = app.viewport.tool_input;
    // Radius: [ and ] keys, the wheel while pulling.
    let (smaller, larger) = ui.input(|i| (i.key_pressed(egui::Key::OpenBracket), i.key_pressed(egui::Key::CloseBracket)));
    let wheel = std::mem::take(&mut app.viewport.scrolled);
    if let Some(k) = app.handles.object {
        let mut r = app.handles_radius(k);
        let before = r;
        if input.hover.is_some() || app.handles.drag.is_some() {
            if smaller {
                r *= 0.9;
            }
            if larger {
                r *= 1.1;
            }
        }
        if wheel != 0.0 {
            r *= 1.1f32.powf(wheel);
        }
        if r != before {
            app.handles.radius = Some(r.max(1e-4));
            app.handles_preview();
        }
    }
    // Clicks and pulls.
    if let Some(at) = input.started {
        app.handles_begin(at);
    }
    if input.dragging
        && let Some(at) = input.pointer
        && let Some(d) = app.handles.drag.clone()
        && let Some(now) = app.handles_plane_point(&d, at)
        && now != d.now
    {
        if let Some(drag) = &mut app.handles.drag {
            drag.now = now;
        }
        app.handles_preview();
    }
    if input.stopped {
        app.handles_commit();
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) && app.handles.drag.take().is_some() {
        app.handles.preview.doc = None;
    }
    if let Some(at) = input.clicked {
        app.handles_click(at, input.shift);
    }
    // The surface under the pointer (asked again when it moves).
    let key = app.scene_key();
    match input.hover {
        Some(at) if app.handles.drag.is_none() => {
            if app.handles.hover_key != Some((at, key)) {
                app.handles.hover = app.handles_surface(at);
                app.handles.hover_key = Some((at, key));
            }
        }
        _ => app.handles.hover = None,
    }
    // Drawing.
    let tk = Tokens::get();
    let p = ui.painter_at(rect);
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let Some(k) = app.handles.object else { return };
    let world = app.doc().world_affine(k).unwrap_or(Affine3A::IDENTITY);
    let r_world = world.transform_vector3(Vec3::X * app.handles_radius(k)).length();
    let ring = |center: Vec3, normal: Vec3, color: egui::Color32| {
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        let pts: Vec<Pos2> = (0..=48)
            .filter_map(|i| {
                let a = i as f32 / 48.0 * std::f32::consts::TAU;
                project(vp, rect, center + (u * a.cos() + v * a.sin()) * r_world)
            })
            .collect();
        if pts.len() > 2 {
            p.add(egui::Shape::line(pts, Stroke::new(1.3, color)));
        }
    };
    if let Some((c, n)) = app.handles.hover {
        ring(c, n, tk.accent.gamma_multiply(0.8));
        if let Some(s) = project(vp, rect, c) {
            p.circle_filled(s, 2.5, tk.accent);
        }
    }
    if let Some(d) = app.handles.drag.clone() {
        ring(d.start, d.normal, tk.accent);
        if let (Some(a), Some(b)) = (project(vp, rect, d.start), project(vp, rect, d.now)) {
            p.extend(egui::Shape::dashed_line(&[a, b], Stroke::new(1.2, tk.accent), 4.0, 3.0));
            p.circle(b, 5.5, tk.accent, Stroke::new(1.0, egui::Color32::from_black_alpha(170)));
        }
    }
    let markers = app.handle_markers();
    let selected = app.handles.selected;
    for (m, at, from) in markers {
        let Some(s) = project(vp, rect, at) else { continue };
        if let Some(f) = from.and_then(|f| project(vp, rect, f)) {
            p.extend(egui::Shape::dashed_line(&[f, s], Stroke::new(1.0, tk.text_dim), 3.0, 3.0));
            p.circle_filled(f, 2.0, tk.text_dim);
        }
        let pin = match m {
            Marker::Pin(_) => true,
            Marker::Placed(i) => app.handles.points.get(i).is_some_and(|q| q.pin),
            Marker::Handle(_) => false,
        };
        let edge = Stroke::new(
            if selected == Some(m) { 2.0 } else { 1.0 },
            if selected == Some(m) { tk.text } else { egui::Color32::from_black_alpha(170) },
        );
        if pin {
            p.rect(egui::Rect::from_center_size(s, vec2(8.0, 8.0)), 1.0, tk.health_warn, edge, egui::StrokeKind::Middle);
        } else if matches!(m, Marker::Placed(_)) {
            p.circle(s, 5.0, tk.viewport_bottom, Stroke::new(2.0, tk.accent));
        } else {
            p.circle(s, 5.0, tk.accent, edge);
        }
        app.handles.screen.push((m, s));
    }
    // What the tool does, in the corner.
    let text = format!(
        "{}  {}  {}",
        t("Click places a handle, Shift+click a pin, drag pulls"),
        p3d_math::format_length(f64::from(r_world)),
        t("radius ([ ] or the wheel while pulling)")
    );
    p.text(rect.center_bottom() + vec2(0.0, -10.0), egui::Align2::CENTER_BOTTOM, text, theme::small(), tk.text_dim);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::Services;
    use crate::viewport::ViewportEvent;

    fn app() -> PixelApp {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.quad_sphere", json!({ "name": "ball", "radius": 0.1, "origin": "center" })).unwrap();
        a.run("ui.tool", json!({ "tool": "handles" })).unwrap();
        a
    }

    fn settle(a: &mut PixelApp) {
        let ctx = egui::Context::default();
        for _ in 0..4000 {
            a.handles_tick(&ctx);
            a.spec_tick(&ctx);
            if !a.handles.busy() && !a.spec.busy() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the handle runs did not finish");
    }

    fn top(a: &PixelApp, name: &str) -> f32 {
        let doc = a.doc();
        let k = doc.resolve(name).unwrap();
        let w = doc.world_affine(k).unwrap();
        let (lo, hi) = doc.object_mesh(k).unwrap().bounds().unwrap();
        w.transform_point3(hi).y.max(w.transform_point3(lo).y)
    }

    #[test]
    fn a_pull_is_one_undo_step_and_pins_hold() {
        let mut a = app();
        let (steps, before) = (a.session.history().len(), top(&a, "ball"));
        a.run("handles.add", json!({ "at": [0, 0.1, 0], "by": [0, 0.05, 0] })).unwrap();
        settle(&mut a);
        assert_eq!(a.session.history().len(), steps + 1);
        assert!((top(&a, "ball") - before - 0.05).abs() < 1e-3, "{before} -> {}", top(&a, "ball"));
        // The handle stays where it was pulled to.
        assert!(a.handles.points[0].at.distance(Vec3::new(0.0, 0.15, 0.0)) < 1e-4);
        a.run("edit.undo", Value::Null).unwrap();
        assert!((top(&a, "ball") - before).abs() < 1e-6);
        // A pin on the bottom holds it while the top moves.
        a.run("handles.clear", Value::Null).unwrap();
        a.run("handles.pin", json!({ "at": [0, -0.1, 0] })).unwrap();
        a.run("handles.radius", json!({ "radius": 0.5 })).unwrap();
        let bottom = |a: &PixelApp| a.doc().object_mesh(a.doc().resolve("ball").unwrap()).unwrap().bounds().unwrap().0.y;
        a.run("handles.add", json!({ "at": [0, 0.1, 0], "by": [0, 0.03, 0] })).unwrap();
        settle(&mut a);
        assert!((bottom(&a) + 0.1).abs() < 1e-4, "{}", bottom(&a));
        assert!(top(&a, "ball") > before + 0.02);
        let args = &a.doc().history(a.doc().resolve("ball").unwrap()).unwrap().nodes.last().unwrap().params;
        assert_eq!((args["space"].clone(), args["radius"].clone(), args["pins"].as_array().map(Vec::len)), (json!("local"), json!(0.5), Some(1)));
    }

    #[test]
    fn a_drag_in_the_view_plane_pulls_the_surface() {
        let mut a = app();
        let doc = Arc::clone(a.session.doc());
        a.viewport.scene(&doc, a.scene_key(), &std::collections::BTreeSet::new());
        a.viewport.rect = egui::Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        a.viewport.camera.set_view(p3d_render::ViewKind::Front);
        a.viewport.camera.frame(Vec3::splat(-0.15), Vec3::splat(0.15), 1.0);
        let vp = a.viewport.camera.view_proj(1.0);
        let at = project(vp, a.viewport.rect, Vec3::new(0.0, 0.06, 0.08)).unwrap();
        a.handles_follow();
        a.handles_begin(at);
        let d = a.handles.drag.clone().expect("the press landed on the ball");
        let up = a.handles_plane_point(&d, at - egui::vec2(0.0, 40.0)).unwrap();
        // The plane faces the camera along the ray: mostly up, a little in depth.
        assert!(up.y > d.start.y + 0.03 && (up.z - d.start.z).abs() < 0.01, "{up} from {}", d.start);
        a.handles.drag.as_mut().unwrap().now = up;
        let before = top(&a, "ball");
        a.handles_commit();
        settle(&mut a);
        assert!(top(&a, "ball") > before + 0.002, "{before} -> {}", top(&a, "ball"));
        assert!(a.handles.points.len() == 1 && a.handles.selected == Some(Marker::Placed(0)));
    }

    #[test]
    fn spec_parts_keep_their_handles_in_part_space() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("spec.starter", json!({ "name": "lamp" })).unwrap();
        let k = a.doc().resolve("lamp/shade").unwrap();
        a.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
        a.spec_follow();
        a.run("ui.tool", json!({ "tool": "handles" })).unwrap();
        let root = a.doc().resolve("lamp").unwrap();
        // The shade's top rim, pulled up 2 cm.
        let w = a.doc().world_affine(k).unwrap();
        let rim = w.transform_point3(Vec3::new(0.05, 0.15, 0.0));
        let steps = a.session.history().len();
        a.run("handles.add", json!({ "at": rim.to_array(), "by": [0, 0.02, 0] })).unwrap();
        settle(&mut a);
        assert_eq!(a.session.history().len(), steps + 1);
        let deform = a.doc().spec(root).unwrap().source["parts"]["shade"]["deform"].clone();
        assert_eq!(deform["handles"].as_array().map(Vec::len), Some(1), "{deform}");
        assert_eq!(deform["handles"][0]["by"], json!([0.0, 0.02, 0.0]));
        assert!(vec3_of(&deform["handles"][0]["move"]).is_some_and(|p| p.distance(Vec3::new(0.05, 0.15, 0.0)) < 1e-4), "part space: {deform}");
        // Its marker, pulled again, adds to its offset.
        assert!(a.handle_markers().iter().any(|(m, ..)| *m == Marker::Handle(0)));
        a.run("handles.move", json!({ "kind": "handle", "index": 0, "by": [0, 0.01, 0] })).unwrap();
        settle(&mut a);
        let by = a.doc().spec(root).unwrap().source["parts"]["shade"]["deform"]["handles"][0]["by"].clone();
        assert!(vec3_of(&by).is_some_and(|b| (b.y - 0.03).abs() < 1e-5), "{by}");
        a.run("handles.delete", json!({ "kind": "handle", "index": 0 })).unwrap();
        settle(&mut a);
        assert!(a.doc().spec(root).unwrap().source["parts"]["shade"].get("deform").is_none());
    }
}
