//! The SDF shape editor: the GUI side of agent-native sculpting. An object
//! made by the SDF op keeps its shapes in its history; selecting it opens
//! the Shapes tab of Properties (the ordered shape list, one row per shape,
//! and the selected shape's fields from the op schema) and draws a wire
//! proxy of every shape in the viewport, where a click selects a shape and
//! the gizmo moves, turns and scales it.
//!
//! While a value is dragged the mesh is rebuilt at a coarse detail (three
//! times the voxel) on a background thread, one run at a time, newest
//! shapes first; on release the op runs once at full detail as one undo
//! step through the normal op path (a background job), rebuilding the
//! object in place (`target`). Same shapes and seed, same mesh.

pub mod panel;
pub mod proxy;
pub mod shapes;

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use glam::{Mat4, Quat, Vec3};
use p3d_engine::{JobStatus, OpCall};
use p3d_scene::{Document, ObjectKey};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::viewport::ViewportEvent;
use crate::{PixelApp, inspector};
use shapes::Path;

/// Names of the op that builds and rebuilds SDF objects, newest first
/// (`gen.sdf` became `sdf.mesh`); an object's history may hold either.
const OPS: [&str; 2] = ["sdf.mesh", "gen.sdf"];

/// The SDF op's id in `registry`: its newest name that is registered.
pub fn op_id(registry: &p3d_ops::Registry) -> &'static str {
    OPS.iter().copied().find(|id| registry.get(id).is_ok()).unwrap_or(OPS[1])
}

/// The shapes Add > SDF Shape starts with: a body and a head blended.
pub fn starter() -> Value {
    json!({ "shapes": [
        { "name": "body", "ellipsoid": { "at": [0, 0.35, 0], "radii": [0.3, 0.35, 0.25] } },
        { "name": "head", "smooth_union": 0.08, "sphere": { "at": [0, 0.82, 0], "radius": 0.18 } }
    ] })
}
/// The op's voxel size limits (meters).
const MIN_DETAIL: f64 = 1e-4;
const MAX_DETAIL: f64 = 0.5;
/// The live preview's voxel against the full one.
const COARSE: f64 = 3.0;

/// Whether an op id is the SDF op (by any of its names).
pub fn is_sdf_op(op: &str) -> bool {
    OPS.contains(&op)
}

/// Ops that leave a mesh as it is: a rebuild keeps what they did.
pub(crate) fn keeps_mesh(op: &str) -> bool {
    matches!(
        op,
        "scene.create_empty"
            | "scene.duplicate"
            | "scene.rename"
            | "scene.parent"
            | "scene.unparent"
            | "scene.set_transform"
            | "scene.tag"
            | "scene.set_metadata"
    ) || op.starts_with("expect.")
        || op.starts_with("select.")
}

/// What an SDF object's history says: its shapes and detail from the last
/// SDF op, and the labels of later ops that changed its mesh (a rebuild
/// drops them).
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub shapes: Vec<Value>,
    pub detail: Option<f64>,
    pub later: Vec<String>,
}

/// The shapes of an SDF object, or `None` for other objects.
///
/// Complexity: O(history length).
pub fn source_of(doc: &Document, k: ObjectKey) -> Option<Source> {
    let nodes = &doc.history(k)?.nodes;
    let i = nodes.iter().rposition(|n| is_sdf_op(&n.op))?;
    let p = &nodes[i].params;
    let shapes = p.get("shapes")?.as_array()?.clone();
    let later = nodes[i + 1..].iter().filter(|n| !keeps_mesh(&n.op)).map(|n| n.label.clone()).collect();
    Some(Source { shapes, detail: p.get("detail").and_then(Value::as_f64), later })
}

/// The coarse preview: one background run at a time.
#[derive(Default)]
struct Preview {
    /// A version of the shapes waiting for its run.
    wanted: bool,
    running: Option<Receiver<Result<Arc<Document>, String>>>,
    /// The previewed document and its scene key.
    doc: Option<(Arc<Document>, u64)>,
    counter: u64,
}

/// The editor between frames.
#[derive(Default)]
pub struct SdfEditor {
    /// The SDF object being edited (the active object), or the object of
    /// the spec part being edited (none for a blended part).
    pub object: Option<ObjectKey>,
    /// The spec root and SDF part being edited, when the shapes come from a
    /// Model Spec: edits then go through `spec.patch`.
    pub part: Option<(ObjectKey, String)>,
    /// The revision the shapes match (read from the history, or committed).
    revision: u64,
    /// The shapes as edited (the op's JSON).
    pub shapes: Vec<Value>,
    /// Voxel size in meters; automatic when `None`.
    pub detail: Option<f64>,
    pub selected: Option<Path>,
    /// The row under the pointer: its faces light up in the viewport.
    pub hovered: Option<Path>,
    /// The row the panel found under the pointer this frame (shown next frame).
    pub hover_next: Option<Path>,
    /// The pointer is over the Shapes tab (Delete then deletes the selected shape),
    /// as of the last frame, and as the panel found it this frame.
    pub panel_hovered: bool,
    pub panel_hover_next: bool,
    /// Shapes changed by a drag still going on (not committed yet).
    pub live: bool,
    /// The shapes when a gizmo drag started.
    drag_start: Option<Vec<Value>>,
    /// Ops after the shapes that changed the mesh (a rebuild drops them).
    pub later: Vec<String>,
    /// Why the last run failed.
    pub error: Option<String>,
    /// Bumped whenever the shapes change.
    version: u64,
    proxies: Option<(u64, Arc<Vec<proxy::Proxy>>)>,
    preview: Preview,
    /// The full-detail run being committed, and the next one waiting.
    commit: Option<p3d_engine::Job>,
    queued: Option<(Vec<OpCall>, String)>,
    highlight: Option<(u64, (Arc<p3d_render::EditOverlay>, u64))>,
}

impl SdfEditor {
    /// Background work still running (a preview or a commit).
    pub fn busy(&self) -> bool {
        self.preview.running.is_some() || self.commit.is_some() || self.queued.is_some()
    }

    /// The live preview document and its scene key, while one is shown.
    pub fn preview_doc(&self) -> Option<(Arc<Document>, u64)> {
        self.preview.doc.clone()
    }

    /// The proxies of the current shapes (rebuilt when they change).
    ///
    /// Complexity: O(1) cached, else as [`proxy::proxies`].
    pub fn proxies(&mut self) -> Arc<Vec<proxy::Proxy>> {
        if let Some((v, p)) = &self.proxies
            && *v == self.version
        {
            return Arc::clone(p);
        }
        let p = Arc::new(proxy::proxies(&self.shapes));
        self.proxies = Some((self.version, Arc::clone(&p)));
        p
    }

    /// The voxel the full run uses: the set detail, else the op's default
    /// (64 voxels across the largest side), estimated from the proxies.
    pub fn voxel(&mut self) -> f64 {
        if let Some(d) = self.detail {
            return d;
        }
        let side = proxy::bounds(&self.proxies()).map_or(0.2, |(lo, hi)| f64::from((hi - lo).max_element()));
        (side / 64.0).clamp(MIN_DETAIL, MAX_DETAIL)
    }

    fn changed(&mut self) {
        self.version += 1;
        self.error = None;
    }

    /// Whether the editor is open (on an SDF object or on a spec's SDF part).
    pub fn is_open(&self) -> bool {
        self.object.is_some() || self.part.is_some()
    }

    fn load(&mut self, k: ObjectKey, revision: u64, src: Source) {
        let keep = self.object == Some(k) && self.part.is_none();
        self.part = None;
        self.object = Some(k);
        self.fill(revision, src, keep);
    }

    /// Opens the editor on the SDF part `part` of the spec on `root`;
    /// `object` is the part's object (none for a part blended into its
    /// parent: no proxies or gizmo then).
    fn load_part(&mut self, root: ObjectKey, part: &str, object: Option<ObjectKey>, revision: u64, src: Source) {
        let keep = self.part.as_ref().is_some_and(|(r, p)| *r == root && p == part);
        self.part = Some((root, part.to_owned()));
        self.object = object;
        self.fill(revision, src, keep);
    }

    fn fill(&mut self, revision: u64, src: Source, keep: bool) {
        self.revision = revision;
        self.shapes = src.shapes;
        self.detail = src.detail;
        self.later = src.later;
        self.live = false;
        self.drag_start = None;
        self.changed();
        if !keep || self.selected.as_ref().is_some_and(|p| shapes::get(&self.shapes, p).is_none()) {
            self.selected = None;
        }
        self.hovered = None;
    }

    fn close(&mut self) {
        self.object = None;
        self.part = None;
        self.selected = None;
        self.hovered = None;
        self.live = false;
        self.drag_start = None;
    }
}

/// The shapes and detail of an SDF part of the spec on `root`, with
/// lengths in meters (the source may say `"2cm"`), or `None` when the part
/// is not an SDF part.
///
/// Complexity: O(shapes of the part).
pub fn spec_part_source(doc: &Document, root: ObjectKey, part: &str) -> Option<Source> {
    let p = doc.spec(root)?.source.get("parts")?.get(part)?;
    let shapes =
        p.get("sdf")?.as_array()?.iter().map(|s| proxy::parse(s).and_then(|t| serde_json::to_value(t).ok()).unwrap_or_else(|| s.clone())).collect();
    let detail = p.get("detail").and_then(|d| serde_json::from_value::<p3d_ops::params::Length>(d.clone()).ok()).map(|l| l.0);
    Some(Source { shapes, detail, later: Vec::new() })
}

/// How a change of the shapes is applied.
pub enum Apply<'a> {
    /// A drag is going on: preview at coarse detail.
    Live,
    /// Done: rebuild at full detail as one undo step labeled so.
    Commit(&'a str),
}

impl PixelApp {
    /// The op call that rebuilds the edited object from the editor's shapes
    /// at `detail` (the set detail when `None`).
    fn sdf_call(&self, detail: Option<f64>) -> Option<OpCall> {
        // A spec's SDF part: a merge patch of its shapes and detail.
        if let Some((root, part)) = &self.sdf.part {
            let id = self.doc().object(*root)?.public_id();
            let d = detail.or(self.sdf.detail).map_or(Value::Null, |d| json!(d));
            let merge = json!({ "parts": { part.as_str(): { "sdf": self.sdf.shapes, "detail": d } } });
            return Some(OpCall::new("spec.patch", json!({ "target": id, "merge": merge })));
        }
        let k = self.sdf.object?;
        let id = self.doc().object(k)?.public_id();
        let mut p = json!({ "target": id, "shapes": self.sdf.shapes });
        if let Some(d) = detail.or(self.sdf.detail) {
            p["detail"] = json!(d);
        }
        Some(OpCall::new(op_id(self.session.registry()), p))
    }

    /// Replaces the shapes and previews or commits them.
    pub fn set_sdf_shapes(&mut self, new: Vec<Value>, how: Apply<'_>) {
        self.sdf.shapes = new;
        self.sdf.changed();
        self.apply_sdf(how);
    }

    /// Previews or commits the editor's current shapes.
    pub fn apply_sdf(&mut self, how: Apply<'_>) {
        match how {
            Apply::Live => {
                self.sdf.live = true;
                self.sdf.preview.wanted = true;
                self.start_sdf_preview();
            }
            Apply::Commit(label) => {
                self.sdf.live = false;
                if let Some(call) = self.sdf_call(None) {
                    self.sdf.queued = Some((vec![call], label.to_owned()));
                    self.start_sdf_commit();
                }
            }
        }
    }

    /// Starts the coarse preview of the newest shapes when none runs.
    fn start_sdf_preview(&mut self) {
        if self.sdf.preview.running.is_some() || !self.sdf.preview.wanted {
            return;
        }
        let coarse = (self.sdf.voxel() * COARSE).clamp(MIN_DETAIL, MAX_DETAIL);
        let Some(call) = self.sdf_call(Some(coarse)) else { return };
        self.sdf.preview.wanted = false;
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::clone(self.session.registry());
        let doc = Document::clone(self.session.doc());
        let seed = self.session.seed();
        let spawned = std::thread::Builder::new().name("sdf preview".into()).spawn(move || {
            let mut work = p3d_engine::Session::with_registry(registry);
            work.set_seed(seed);
            work.load(doc);
            let r = work.execute(&[call], &Default::default()).map(|_| Arc::clone(work.doc())).map_err(|e| e.error.to_string());
            // The editor may have moved on; nothing to do then.
            let _ = tx.send(r);
        });
        if spawned.is_ok() {
            self.sdf.preview.running = Some(rx);
        }
    }

    /// Starts the queued full-detail run when none runs.
    fn start_sdf_commit(&mut self) {
        if self.sdf.commit.is_some() {
            return;
        }
        if let Some((calls, label)) = self.sdf.queued.take() {
            self.sdf.commit = Some(self.session.spawn_job(calls, &label));
        }
    }

    /// Per frame: follows the active object, takes in finished previews and
    /// commits, and keeps frames coming while work runs.
    ///
    /// Complexity: O(history length) when the object or revision changed.
    pub(crate) fn sdf_tick(&mut self, ctx: &egui::Context) {
        self.sdf.hovered = self.sdf.hover_next.take();
        self.sdf.panel_hovered = std::mem::take(&mut self.sdf.panel_hover_next);
        // Finished runs first, so the history read below is current.
        if let Some(mut job) = self.sdf.commit.take() {
            match self.session.poll_job(&mut job) {
                JobStatus::Pending(_) => self.sdf.commit = Some(job),
                JobStatus::Done(r) => {
                    self.committed(&r);
                    self.prune_selection();
                    self.sdf.revision = r.revision;
                    self.start_sdf_commit();
                }
                JobStatus::Failed(e) => {
                    let msg = e.error.to_string();
                    self.set_status(msg.clone());
                    self.sdf.error = Some(msg);
                    self.start_sdf_commit();
                }
            }
        }
        if let Some(rx) = &self.sdf.preview.running {
            match rx.try_recv() {
                Ok(r) => {
                    self.sdf.preview.running = None;
                    match r {
                        Ok(doc) if self.sdf.live || self.sdf.commit.is_some() || self.sdf.queued.is_some() => {
                            self.sdf.preview.counter += 1;
                            self.sdf.preview.doc = Some((doc, self.sdf.preview.counter));
                            self.sdf.error = None;
                        }
                        Ok(_) => {}
                        Err(e) => self.sdf.error = Some(e),
                    }
                    self.start_sdf_preview();
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.sdf.preview.running = None,
            }
        }
        // Nothing pending: the document shows the committed mesh.
        if !self.sdf.live && self.sdf.commit.is_none() && self.sdf.queued.is_none() {
            self.sdf.preview.doc = None;
            self.sdf.preview.wanted = false;
        }
        if self.sdf.busy() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        self.sdf_follow();
    }

    /// Opens the editor on the active object when it is an SDF object (the
    /// Shapes tab comes forward), closes it otherwise, and reads the shapes
    /// again after undo, redo or an agent's change.
    ///
    /// Complexity: O(history length) when something changed.
    pub(crate) fn sdf_follow(&mut self) {
        let doc = Arc::clone(self.session.doc());
        let idle = !self.sdf.live && self.sdf.commit.is_none() && self.sdf.queued.is_none();
        // The Spec tab's selected part, when it is an SDF part.
        if let Some((root, part)) = self.spec_sdf_part()
            && let Some(s) = spec_part_source(&doc, root, &part)
        {
            let same = self.sdf.part.as_ref().is_some_and(|(r, p)| *r == root && *p == part);
            if !same || (idle && doc.revision != self.sdf.revision) {
                let object = crate::spec::part_object(&doc, root, &part);
                self.sdf.load_part(root, &part, object, doc.revision, s);
            }
            return;
        }
        if self.sdf.part.is_some() {
            self.sdf.close();
        }
        // Spec part objects are edited through their spec, never directly.
        let active = self.active_object().filter(|k| crate::spec::spec_of(&doc, *k).is_none());
        let src = active.and_then(|k| source_of(&doc, k).map(|s| (k, s)));
        match src {
            Some((k, s)) if self.sdf.object != Some(k) => {
                self.sdf.load(k, doc.revision, s);
                self.ui.inspector_tab = inspector::Tab::Shapes;
            }
            Some((k, s)) => {
                if idle && doc.revision != self.sdf.revision {
                    self.sdf.load(k, doc.revision, s);
                }
            }
            None => {
                if self.sdf.object.is_some() {
                    self.sdf.close();
                }
            }
        }
    }

    /// The object the editor's proxies and gizmo work on, while it is shown:
    /// the edited object when it is active, or a spec part's object while
    /// its root or itself is active.
    pub fn sdf_shown_object(&self) -> Option<ObjectKey> {
        let k = self.sdf.object?;
        let active = self.active_object()?;
        (active == k || self.sdf.part.as_ref().is_some_and(|(root, _)| *root == active)).then_some(k)
    }

    /// The selected shape's frame for the gizmo: its center and rotation in
    /// world space (Object Mode, a shape selected on the active object).
    ///
    /// Complexity: O(depth of the shape).
    pub fn sdf_gizmo_subject(&self) -> Option<(Vec3, Quat)> {
        if self.ui.mode.is_edit() {
            return None;
        }
        let k = self.sdf_shown_object()?;
        let path = self.sdf.selected.as_ref()?;
        let (c, r) = proxy::frame_of(&self.sdf.shapes, path)?;
        let world = Mat4::from(self.doc().world_affine(k)?);
        let (_, wr, _) = world.to_scale_rotation_translation();
        Some((world.transform_point3(c), wr * r))
    }

    /// A gizmo drag on the selected shape starts.
    pub(crate) fn sdf_begin_drag(&mut self) {
        self.sdf.drag_start = Some(self.sdf.shapes.clone());
    }

    /// The gizmo drag moved: `delta` is the world transform so far.
    pub(crate) fn sdf_drag(&mut self, delta: Mat4) {
        let (Some(start), Some(path), Some(k)) = (self.sdf.drag_start.clone(), self.sdf.selected.clone(), self.sdf.object) else { return };
        let Some(world) = self.doc().world_affine(k).map(Mat4::from) else { return };
        let mut next = start;
        if proxy::transform(&mut next, &path, world.inverse() * delta * world) {
            self.set_sdf_shapes(next, Apply::Live);
        }
    }

    /// The gizmo drag ended: commit (`Some(label)`) or put the shapes back.
    pub(crate) fn sdf_end_drag(&mut self, commit: Option<&str>) {
        let Some(start) = self.sdf.drag_start.take() else { return };
        match commit {
            Some(label) if start != self.sdf.shapes => self.apply_sdf(Apply::Commit(label)),
            _ => {
                self.sdf.shapes = start;
                self.sdf.changed();
                self.sdf.live = false;
            }
        }
    }

    /// Whether a gizmo drag runs on a shape.
    pub fn sdf_dragging(&self) -> bool {
        self.sdf.drag_start.is_some()
    }

    /// The faces of the row under the pointer, lit in the viewport.
    ///
    /// Complexity: O(1) cached; a rebuild is O(V + E + F) of the mesh.
    pub(crate) fn sdf_highlight(&mut self) -> Option<(Arc<p3d_render::EditOverlay>, u64)> {
        use std::hash::{Hash, Hasher};
        let k = self.sdf.object.filter(|_| !self.ui.mode.is_edit())?;
        let path = self.sdf.hovered.clone()?;
        let (part, mesh, _) = self.viewport.part_of(k)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(&mesh) as usize, part, &path, self.sdf.version).hash(&mut h);
        let key = h.finish();
        if let Some((cached, o)) = &self.sdf.highlight
            && *cached == key
        {
            return Some(o.clone());
        }
        let mut faces = std::collections::BTreeSet::new();
        for g in shapes::groups_under(&self.sdf.shapes, &path) {
            if let Some(group) = mesh.group(&g) {
                faces.extend(group.iter());
            }
        }
        let none = std::collections::BTreeSet::new();
        let sets = p3d_render::ElementSets { verts: &none, edges: &none, faces: &faces };
        let o = (Arc::new(p3d_render::edit_overlay(&mesh, part, sets, false)), key);
        self.sdf.highlight = Some((key, o.clone()));
        Some(o)
    }

    /// Runs a `shapes.*` command (the panel's buttons, for agents and tests).
    pub(crate) fn sdf_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if !self.sdf.is_open() {
            return Err(t("select an SDF object first").to_owned());
        }
        let path_param = |key: &str| params.get(key).and_then(|v| serde_json::from_value::<Path>(v.clone()).ok());
        let selected = path_param("path").or_else(|| self.sdf.selected.clone());
        let mut list = self.sdf.shapes.clone();
        let (label, select): (String, Option<Path>) = match id {
            "shapes.select" => {
                let p = match params.get("name").and_then(Value::as_str) {
                    Some(n) => shapes::flatten(&list).into_iter().find(|(_, s)| shapes::name_of(s) == Some(n)).map(|(p, _)| p),
                    None => path_param("path"),
                };
                if let Some(p) = &p
                    && shapes::get(&list, p).is_none()
                {
                    return Err(t("no shape there").to_owned());
                }
                self.sdf.selected = p.clone();
                return Ok(json!({ "selected": p }));
            }
            "shapes.add" => {
                let prim = params.get("prim").and_then(Value::as_str).unwrap_or("sphere");
                if !shapes::PRIMS.contains(&prim) || prim == "group" {
                    return Err(format!("{} `{prim}`: sphere, ellipsoid, box, capsule, cone, torus or cylinder", t("unknown primitive")));
                }
                let (at, size) = self.sdf_new_place(selected.as_deref());
                let name = shapes::unique_name(&list, prim);
                let p = shapes::insert_after(&mut list, selected.as_deref(), shapes::new_shape(prim, at, size, &name));
                (format!("{} {name}", t("Add")), Some(p))
            }
            "shapes.duplicate" => {
                let p = selected.ok_or(t("select a shape"))?;
                let copy = shapes::duplicate(&mut list, &p).ok_or(t("no shape there"))?;
                (format!("{} {}", t("Duplicate"), shapes::label(&list, &copy)), Some(copy))
            }
            "shapes.delete" => {
                let p = selected.ok_or(t("select a shape"))?;
                let name = shapes::label(&list, &p);
                shapes::remove(&mut list, &p).map_err(|e| t(e).to_owned())?;
                (format!("{} {name}", t("Delete")), None)
            }
            "shapes.group" => {
                let p = selected.ok_or(t("select a shape"))?;
                let g = shapes::wrap_in_group(&mut list, &p).ok_or(t("no shape there"))?;
                (t("Group shape").to_owned(), Some(g))
            }
            "shapes.move" => {
                let from = path_param("from").or(selected).ok_or(t("from is required"))?;
                let before = path_param("before").ok_or(t("before is required: a shape path, or one past the last"))?;
                let to = shapes::move_to(&mut list, &from, &before).ok_or(t("the shape cannot move there"))?;
                (format!("{} {}", t("Reorder"), shapes::label(&list, &to)), Some(to))
            }
            "shapes.hide" => {
                let p = selected.ok_or(t("select a shape"))?;
                let s = shapes::get_mut(&mut list, &p).ok_or(t("no shape there"))?;
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!shapes::is_hidden(s));
                shapes::set_hidden(s, on);
                (format!("{} {}", if on { t("Hide") } else { t("Show") }, shapes::label(&list, &p)), Some(p))
            }
            "shapes.join" => {
                let p = selected.ok_or(t("select a shape"))?;
                let join = params.get("join").and_then(Value::as_str).ok_or(t("join is required"))?;
                if !shapes::JOINS.iter().any(|(j, _)| *j == join) {
                    return Err(format!("{} `{join}`", t("unknown join")));
                }
                let s = shapes::get_mut(&mut list, &p).ok_or(t("no shape there"))?;
                let blend = params.get("blend").and_then(Value::as_f64).or(shapes::join_of(s).1).unwrap_or(0.01);
                shapes::set_join(s, join, blend);
                (format!("{} {}", t("Join"), shapes::label(&list, &p)), Some(p))
            }
            "shapes.mirror" => {
                let p = selected.ok_or(t("select a shape"))?;
                let axis = params.get("axis").and_then(Value::as_str).unwrap_or("");
                if !shapes::MIRRORS.contains(&axis) {
                    return Err(t("axis is x, y, z or empty").to_owned());
                }
                let s = shapes::get_mut(&mut list, &p).ok_or(t("no shape there"))?;
                shapes::set_mirror(s, axis);
                (format!("{} {}", t("Mirror"), shapes::label(&list, &p)), Some(p))
            }
            "shapes.set" => {
                let p = selected.ok_or(t("select a shape"))?;
                let shape = params.get("shape").filter(|s| s.is_object()).ok_or(t("shape is required: the shape's JSON"))?;
                proxy::parse(shape).ok_or(t("not a valid shape"))?;
                *shapes::get_mut(&mut list, &p).ok_or(t("no shape there"))? = shape.clone();
                (format!("{} {}", t("Edit"), shapes::label(&list, &p)), Some(p))
            }
            "shapes.detail" => {
                self.sdf.detail = params.get("detail").and_then(Value::as_f64).map(|d| d.clamp(MIN_DETAIL, MAX_DETAIL));
                (t("Detail").to_owned(), selected)
            }
            other => return Err(format!("{} `{other}`", t("unknown command"))),
        };
        self.sdf.selected = select;
        self.set_sdf_shapes(list, Apply::Commit(&label));
        Ok(json!({ "selected": self.sdf.selected, "shapes": self.sdf.shapes.len() }))
    }

    /// Where a new shape goes: next to the selected one (offset along X),
    /// else at the shapes' center; and its size from the shapes.
    fn sdf_new_place(&mut self, near: Option<&[usize]>) -> ([f64; 3], f64) {
        let proxies = self.sdf.proxies();
        let (lo, hi) = proxy::bounds(&proxies).unwrap_or((Vec3::splat(-0.1), Vec3::splat(0.1)));
        let size = f64::from((hi - lo).max_element() * 0.3).max(0.01);
        let at = match near.and_then(|p| proxy::frame_of(&self.sdf.shapes, p)) {
            Some((c, _)) => c + Vec3::X * size as f32,
            None => (lo + hi) * 0.5,
        };
        ([f64::from(at.x), f64::from(at.y), f64::from(at.z)], size)
    }

    /// The editor's state for agents (`ui.inspect`).
    pub(crate) fn sdf_state(&self) -> Value {
        let doc = self.doc();
        json!({
            "object": self.sdf.object.and_then(|k| doc.object(k)).map(|o| o.public_id()),
            "part": self.sdf.part.as_ref().map(|(_, p)| p.clone()),
            "shapes": self.sdf.is_open().then(|| shapes::flatten(&self.sdf.shapes).into_iter().map(|(p, _)| json!({ "path": p, "label": shapes::label(&self.sdf.shapes, &p) })).collect::<Vec<_>>()),
            "selected": self.sdf.selected,
            "selectedShape": self.sdf.selected.as_ref().and_then(|p| shapes::get(&self.sdf.shapes, p)),
            "previewing": self.sdf.preview.doc.is_some(),
            "busy": self.sdf.busy(),
            "error": self.sdf.error,
        })
    }
}

/// Draws the shape proxies over the viewport and lets a click select a
/// shape (the click then selects no object). Object Mode only.
///
/// Complexity: O(proxy points) per frame.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui, events: &mut Vec<ViewportEvent>) {
    let rect = app.viewport.rect;
    if app.ui.mode.is_edit() || !rect.is_positive() {
        return;
    }
    let Some(k) = app.sdf_shown_object() else { return };
    let Some(world) = app.doc().world_affine(k).map(Mat4::from) else { return };
    let proxies = app.sdf.proxies();
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let on_screen = proxy::screen(&proxies, world, vp, rect);
    let tk = crate::theme::Tokens::get();
    let p = ui.painter_at(rect);
    let (selected, hovered) = (app.sdf.selected.clone(), app.sdf.hovered.clone());
    // Selected last, so it draws on top.
    let mut order: Vec<usize> = (0..on_screen.len()).collect();
    order.sort_by_key(|&i| (Some(&on_screen[i].0) == selected.as_ref(), Some(&on_screen[i].0) == hovered.as_ref()));
    for i in order {
        let (path, hidden, segs) = &on_screen[i];
        let stroke = if Some(path) == selected.as_ref() {
            egui::Stroke::new(1.6, tk.accent)
        } else if Some(path) == hovered.as_ref() {
            egui::Stroke::new(1.3, tk.text)
        } else if *hidden {
            egui::Stroke::new(0.8, tk.text_faint.gamma_multiply(0.35))
        } else {
            egui::Stroke::new(1.0, tk.text_dim.gamma_multiply(0.7))
        };
        for [a, b] in segs {
            p.line_segment([*a, *b], stroke);
        }
    }
    // A click on a proxy selects its shape instead of an object.
    let click = ui.input(|i| (i.pointer.primary_clicked() && !i.modifiers.alt).then(|| i.pointer.interact_pos()).flatten());
    if let Some(at) = click.filter(|at| rect.contains(*at))
        && !app.gizmo.over_handle()
        && let Some(path) = proxy::pick(&on_screen, at)
    {
        app.sdf.selected = Some(path);
        app.viewport.cancel_pick();
        events.retain(|e| !matches!(e, ViewportEvent::Picked { .. } | ViewportEvent::MissedAt { .. }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    /// A two-shape SDF object, selected.
    pub(crate) fn blob() -> PixelApp {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run(
            "shapes.new",
            json!({ "name": "blob", "detail": 0.02, "shapes": [
                { "name": "body", "sphere": { "at": [0, 0.1, 0], "radius": 0.1 } },
                { "name": "head", "sphere": { "at": [0, 0.22, 0], "radius": 0.06 }, "smooth_union": 0.03 }
            ] }),
        )
        .unwrap();
        a.sdf_follow();
        a
    }

    /// Lets background runs finish and takes them in.
    pub(crate) fn settle(a: &mut PixelApp) {
        let ctx = egui::Context::default();
        for _ in 0..2000 {
            a.sdf_tick(&ctx);
            if !a.sdf.busy() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the SDF runs did not finish");
    }

    #[test]
    fn selecting_an_sdf_object_opens_the_editor() {
        let mut a = blob();
        let k = a.doc().resolve("blob").unwrap();
        assert_eq!((a.sdf.object, a.ui.inspector_tab, a.sdf.shapes.len()), (Some(k), inspector::Tab::Shapes, 2));
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        a.sdf_follow();
        assert_eq!(a.sdf.object, None, "a box is not an SDF object");
        a.run("edit.select", json!({ "target": "blob" })).unwrap();
        a.sdf_follow();
        assert_eq!(a.sdf.object, Some(k));
    }

    #[test]
    fn edits_preview_coarse_and_commit_one_step_in_place() {
        let mut a = blob();
        let k = a.doc().resolve("blob").unwrap();
        let (steps, faces) = (a.session.history().len(), a.doc().object_mesh(k).unwrap().face_count());
        a.run("shapes.select", json!({ "name": "head" })).unwrap();
        // A drag: the mesh is previewed coarsely, the document is untouched.
        a.sdf_begin_drag();
        a.sdf_drag(Mat4::from_translation(Vec3::X * 0.05));
        let ctx = egui::Context::default();
        for _ in 0..2000 {
            a.sdf_tick(&ctx);
            if a.sdf.preview_doc().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (doc, _) = a.sdf.preview_doc().expect("a preview");
        let coarse = doc.object_mesh(k).unwrap().face_count();
        assert!(coarse < faces, "the preview is coarser: {coarse} faces against {faces}");
        assert_eq!(a.session.history().len(), steps, "nothing committed while dragging");
        // Release: one undo step at full detail, the same object.
        a.sdf_end_drag(Some("Move head"));
        settle(&mut a);
        assert_eq!(a.session.history().len(), steps + 1);
        assert_eq!(a.doc().resolve("blob").unwrap(), k);
        assert!(a.sdf.preview_doc().is_none());
        let src = source_of(a.doc(), k).unwrap();
        assert_eq!(src.shapes[1]["sphere"]["at"], json!([0.05, 0.22, 0.0]));
        assert!(a.doc().object_mesh(k).unwrap().group("head").is_some());
        // Undo brings the old shapes back into the editor.
        a.run("edit.undo", Value::Null).unwrap();
        a.sdf_follow();
        assert_eq!(a.sdf.shapes[1]["sphere"]["at"], json!([0.0, 0.22, 0.0]));
    }

    #[test]
    fn list_commands_add_hide_group_and_reorder() {
        let mut a = blob();
        let k = a.doc().resolve("blob").unwrap();
        a.run("shapes.select", json!({ "name": "head" })).unwrap();
        a.run("shapes.add", json!({ "prim": "capsule" })).unwrap();
        settle(&mut a);
        assert_eq!(a.sdf.selected, Some(vec![2]));
        assert!(a.doc().object_mesh(k).unwrap().group("capsule").is_some());
        a.run("shapes.hide", Value::Null).unwrap();
        settle(&mut a);
        assert!(a.doc().object_mesh(k).unwrap().group("capsule").is_none(), "hidden shapes leave the mesh");
        a.run("shapes.group", json!({ "path": [0] })).unwrap();
        a.run("shapes.move", json!({ "from": [1], "before": [0] })).unwrap();
        settle(&mut a);
        let src = source_of(a.doc(), k).unwrap();
        assert_eq!(shapes::name_of(&src.shapes[0]), Some("head"));
        assert_eq!(shapes::prim_key(&src.shapes[1]), Some("group"));
        assert!(a.run("shapes.delete", json!({ "path": [1, 0] })).is_err(), "a group keeps one shape");
        assert!(a.run("shapes.add", json!({ "prim": "teapot" })).is_err());
    }

    #[test]
    fn later_mesh_edits_are_named() {
        let mut a = blob();
        a.run("scene.set_transform", json!({ "target": "blob", "position": [1, 0, 0] })).unwrap();
        a.run("subd.catmull_clark", json!({ "target": "blob" })).unwrap();
        let k = a.doc().resolve("blob").unwrap();
        assert_eq!(source_of(a.doc(), k).unwrap().later.len(), 1, "the move keeps the mesh, the subdivision does not");
    }

    #[test]
    fn remesh_to_quads_and_delete_over_the_tab() {
        let mut a = blob();
        let k = a.doc().resolve("blob").unwrap();
        // The Shapes tab's button opens the quad remesh card on the object.
        a.open_job_card("remesh.quad").unwrap();
        assert_eq!(a.job_card.target, Some(k));
        a.run("job.run", json!({ "edge": 0.03 })).unwrap();
        let ctx = egui::Context::default();
        for _ in 0..4000 {
            a.job_card_tick(&ctx);
            if !a.job_card.busy() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        a.sdf_follow();
        assert_eq!(a.sdf.later.len(), 1, "the editor warns that a rebuild replaces the remesh");
        assert!(a.doc().object_mesh(k).unwrap().faces().all(|f| a.doc().object_mesh(k).unwrap().face_len(f) == 4));
        // Delete with the pointer over the tab deletes the selected shape.
        a.run("shapes.select", json!({ "name": "head" })).unwrap();
        a.sdf.panel_hovered = true;
        let ev = egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        ctx.begin_pass(egui::RawInput { events: vec![ev], ..Default::default() });
        crate::chrome::shortcuts(&mut a, &ctx);
        ctx.end_pass().textures_delta.clear();
        settle(&mut a);
        assert_eq!((a.doc().object_count(), a.sdf.shapes.len()), (1, 1), "the shape went, the object stayed");
    }

    #[test]
    fn hovering_a_row_lights_its_faces_and_clicks_pick_proxies() {
        let mut a = blob();
        let k = a.doc().resolve("blob").unwrap();
        let doc = Arc::clone(a.session.doc());
        a.viewport.scene(&doc, a.scene_key(), &std::collections::BTreeSet::new());
        assert!(a.sdf_highlight().is_none(), "nothing hovered");
        // The panel saw the pointer on the head's row last frame.
        a.sdf.hover_next = Some(vec![1]);
        a.sdf_tick(&egui::Context::default());
        let (o, _) = a.sdf_highlight().expect("the head's faces light up");
        let head = doc.object_mesh(k).unwrap().group("head").unwrap().count();
        // Each lit face is drawn as triangles (3 vertices each).
        assert!(o.faces.len() >= head * 3, "{} vertices for {head} faces", o.faces.len());
        // A click on the body's proxy selects the body, not an object.
        a.viewport.rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        a.viewport.camera.set_view(p3d_render::ViewKind::Front);
        let world = Mat4::from(a.doc().world_affine(k).unwrap());
        let vp = a.viewport.camera.view_proj(1.0);
        let on = proxy::screen(&a.sdf.proxies(), world, vp, a.viewport.rect);
        let at = on.iter().find(|(p, _, _)| p == &vec![0]).and_then(|(_, _, s)| s.first()).map(|[p, _]| *p).unwrap();
        assert_eq!(proxy::pick(&on, at), Some(vec![0]));
    }
}
