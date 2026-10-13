//! The outline editor: 2D canvases for the shapes drawn as outlines,
//! `gen.shell` (one outline and its holes, in the shell's plane) and
//! `sdf.silhouettes` (front, side and top outlines, like a blueprint).
//! Selecting such an object, or a spec part of that kind, opens it: the
//! Outline tab of Properties for objects, the Spec tab for parts. Points
//! drag on a millimetre grid, snap to the grid and to the other views'
//! extents, get added on a segment, deleted, and turned corner or smooth
//! with Bézier handles. While something is dragged, the result is rebuilt
//! in the background and shown in the viewport (coarser for silhouettes);
//! on release the op rebuilds the object in place (`target`), or a spec
//! part through `spec.patch`, as one undo step through the normal op path.
//! The outlines are written back as SVG path data in meters (M, L, C, Z),
//! or as point lists when they came as one and still are.
//!
//! While silhouettes are edited, the viewport draws the outlines as flat
//! wires on planes around the object, like a blueprint.

pub mod canvas;
pub mod panel;
pub mod path;

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use egui::{Pos2, Stroke, pos2};
use glam::{DVec2, Mat4, Vec3};
use p3d_engine::{JobStatus, OpCall};
use p3d_scene::{Document, ObjectKey};
use serde_json::{Map, Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::inspector;
use crate::theme::Tokens;
use path::{Node, Outline};

/// The op behind an outline-drawn object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Shell,
    Silhouettes,
}

impl Kind {
    pub fn op(self) -> &'static str {
        match self {
            Kind::Shell => "gen.shell",
            Kind::Silhouettes => "sdf.silhouettes",
        }
    }

    /// Its shape key in a Model Spec part.
    pub fn spec_key(self) -> &'static str {
        match self {
            Kind::Shell => "shell",
            Kind::Silhouettes => "silhouettes",
        }
    }

    fn of_op(op: &str) -> Option<Kind> {
        match op {
            "gen.shell" => Some(Kind::Shell),
            "sdf.silhouettes" => Some(Kind::Silhouettes),
            _ => None,
        }
    }

    /// The views it is drawn in.
    pub fn views(self) -> &'static [ViewId] {
        match self {
            Kind::Shell => &[ViewId::Outline],
            Kind::Silhouettes => &[ViewId::Front, ViewId::Side, ViewId::Top],
        }
    }

    /// Optional arguments the editor may leave out (a spec patch removes
    /// them with `null`).
    fn optional(self) -> &'static [&'static str] {
        match self {
            Kind::Shell => &["holes", "detail"],
            Kind::Silhouettes => &["side", "top", "detail"],
        }
    }
}

/// A canvas: the shell's plane, or one of the three blueprint views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ViewId {
    Outline,
    Front,
    Side,
    Top,
}

impl ViewId {
    pub const ALL: [ViewId; 4] = [ViewId::Outline, ViewId::Front, ViewId::Side, ViewId::Top];

    pub fn name(self) -> &'static str {
        match self {
            ViewId::Outline => "outline",
            ViewId::Front => "front",
            ViewId::Side => "side",
            ViewId::Top => "top",
        }
    }

    pub fn parse(s: &str) -> Option<ViewId> {
        ViewId::ALL.into_iter().find(|v| v.name() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            ViewId::Outline => t("Outline"),
            ViewId::Front => t("Front"),
            ViewId::Side => t("Side"),
            ViewId::Top => t("Top"),
        }
    }

    /// The axes of its horizontal and vertical directions.
    pub fn axes(self) -> (&'static str, &'static str) {
        match self {
            ViewId::Outline | ViewId::Front => ("x", "y"),
            ViewId::Side => ("z", "y"),
            ViewId::Top => ("x", "z"),
        }
    }

    /// A point of the view as a point in the part's space, on the plane at
    /// `depth` along the view's own axis.
    pub fn to_3d(self, p: DVec2, depth: f64) -> Vec3 {
        let v = match self {
            ViewId::Outline | ViewId::Front => [p.x, p.y, depth],
            ViewId::Side => [depth, p.y, p.x],
            ViewId::Top => [p.x, depth, p.y],
        };
        Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32)
    }
}

/// Which outline argument a layer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerKey {
    Outline,
    Hole(usize),
    Front,
    Side,
    Top,
}

impl LayerKey {
    pub fn view(self) -> ViewId {
        match self {
            LayerKey::Outline | LayerKey::Hole(_) => ViewId::Outline,
            LayerKey::Front => ViewId::Front,
            LayerKey::Side => ViewId::Side,
            LayerKey::Top => ViewId::Top,
        }
    }

    /// Its argument path: `outline`, `holes/0`, `front`, ...
    pub fn name(self) -> String {
        match self {
            LayerKey::Outline => "outline".into(),
            LayerKey::Hole(i) => format!("holes/{i}"),
            LayerKey::Front => "front".into(),
            LayerKey::Side => "side".into(),
            LayerKey::Top => "top".into(),
        }
    }

    pub fn parse(s: &str) -> Option<LayerKey> {
        match s {
            "outline" => Some(LayerKey::Outline),
            "front" => Some(LayerKey::Front),
            "side" => Some(LayerKey::Side),
            "top" => Some(LayerKey::Top),
            _ => s.strip_prefix("holes/").and_then(|i| i.parse().ok()).map(LayerKey::Hole),
        }
    }
}

/// One outline argument as edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub key: LayerKey,
    pub outline: Outline,
}

/// A point: its layer (index into the layers), contour and node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Sel {
    pub layer: usize,
    pub contour: usize,
    pub node: usize,
}

/// What a canvas drag moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    Node(Sel),
    /// A handle of a point: its outgoing one, or the incoming one.
    Handle(Sel, bool),
}

/// An outline object's arguments, read from its history or its spec part.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub kind: Kind,
    /// The op's arguments without name, parent, position and target.
    pub args: Map<String, Value>,
    /// Ops after it that changed the mesh (a rebuild drops them).
    pub later: Vec<String>,
}

/// Arguments that place a new object: left out of a rebuild.
const PLACING: [&str; 4] = ["name", "parent", "position", "target"];

/// The outline arguments of an object made by `gen.shell` or
/// `sdf.silhouettes`, or `None` for other objects.
///
/// Complexity: O(history length).
pub fn source_of(doc: &Document, k: ObjectKey) -> Option<Source> {
    let nodes = &doc.history(k)?.nodes;
    let i = nodes.iter().rposition(|n| Kind::of_op(&n.op).is_some())?;
    let kind = Kind::of_op(&nodes[i].op)?;
    let mut args = nodes[i].params.as_object()?.clone();
    for key in PLACING {
        args.remove(key);
    }
    let later = nodes[i + 1..].iter().filter(|n| !crate::sdf::keeps_mesh(&n.op)).map(|n| n.label.clone()).collect();
    Some(Source { kind, args, later })
}

/// The outline arguments of a spec part of kind `shell` or `silhouettes`.
///
/// Complexity: O(size of the part).
pub fn spec_part_source(doc: &Document, root: ObjectKey, part: &str) -> Option<Source> {
    let p = doc.spec(root)?.source.get("parts")?.get(part)?;
    [Kind::Shell, Kind::Silhouettes]
        .into_iter()
        .find_map(|kind| p.get(kind.spec_key()).and_then(Value::as_object).map(|a| Source { kind, args: a.clone(), later: Vec::new() }))
}

/// The layers of a kind's arguments (the arguments keep the rest).
///
/// Complexity: O(size of the outlines).
fn layers_of(kind: Kind, args: &mut Map<String, Value>) -> Result<Vec<Layer>, String> {
    let mut layers = Vec::new();
    let mut take = |key: LayerKey, v: Option<Value>| -> Result<(), String> {
        if let Some(v) = v {
            let outline = Outline::from_value(&v).map_err(|e| format!("{}: {e}", key.name()))?;
            layers.push(Layer { key, outline });
        }
        Ok(())
    };
    match kind {
        Kind::Shell => {
            take(LayerKey::Outline, args.remove("outline"))?;
            if let Some(Value::Array(holes)) = args.remove("holes") {
                for (i, h) in holes.into_iter().enumerate() {
                    take(LayerKey::Hole(i), Some(h))?;
                }
            }
        }
        Kind::Silhouettes => {
            take(LayerKey::Front, args.remove("front"))?;
            take(LayerKey::Side, args.remove("side"))?;
            take(LayerKey::Top, args.remove("top"))?;
        }
    }
    Ok(layers)
}

/// The background preview: one run at a time.
#[derive(Default)]
struct Preview {
    wanted: bool,
    running: Option<Receiver<Result<Arc<Document>, String>>>,
    doc: Option<(Arc<Document>, u64)>,
    counter: u64,
}

/// The editor between frames.
#[derive(Default)]
pub struct OutlineEditor {
    /// The object being edited (a spec part's object for a part).
    pub object: Option<ObjectKey>,
    /// The spec root and part, when the outlines come from a Model Spec.
    pub part: Option<(ObjectKey, String)>,
    pub kind: Option<Kind>,
    /// The revision the outlines were read at (or committed at).
    revision: u64,
    /// The op's other arguments: thickness, cup, bend, twist, round, detail.
    pub fields: Map<String, Value>,
    pub layers: Vec<Layer>,
    pub view: Option<ViewId>,
    pub selected: Option<Sel>,
    pub grab: Option<Grab>,
    /// The canvas mapping kept while a drag runs (the view does not jump).
    pub frozen: Option<canvas::Fit>,
    /// The outlines changed by a drag still going on.
    pub live: bool,
    pub later: Vec<String>,
    pub error: Option<String>,
    version: u64,
    preview: Preview,
    commit: Option<p3d_engine::Job>,
    queued: Option<(Vec<OpCall>, String)>,
    /// The selected view's points on screen last frame, and its canvas.
    pub screen: Vec<(Sel, Pos2)>,
    pub canvas: Option<egui::Rect>,
    /// The pointer is over the canvas (Delete then deletes the point), as
    /// of the last frame, and as the canvas found it this frame.
    pub canvas_hovered: bool,
    pub canvas_hover_next: bool,
}

/// How a change is applied.
#[derive(Debug, Clone, Copy)]
pub enum Apply<'a> {
    /// A drag is going on: preview in the background.
    Live,
    /// Done: rebuild as one undo step labeled so.
    Commit(&'a str),
}

impl OutlineEditor {
    pub fn is_open(&self) -> bool {
        self.kind.is_some()
    }

    pub fn busy(&self) -> bool {
        self.commit.is_some() || self.queued.is_some() || self.preview.running.is_some()
    }

    pub fn preview_doc(&self) -> Option<(Arc<Document>, u64)> {
        self.preview.doc.clone()
    }

    fn changed(&mut self) {
        self.version += 1;
        self.error = None;
    }

    /// Closes the editor; a rebuild still running finishes and lands.
    fn close(&mut self) {
        let (commit, queued) = (self.commit.take(), self.queued.take());
        let (running, counter) = (self.preview.running.take(), self.preview.counter);
        *self = OutlineEditor { commit, queued, ..OutlineEditor::default() };
        self.preview.running = running;
        self.preview.counter = counter;
    }

    /// Fills the editor from a source; `keep` keeps the view and selection.
    fn fill(&mut self, revision: u64, src: Source, keep: bool) {
        self.revision = revision;
        self.kind = Some(src.kind);
        self.later = src.later;
        let mut args = src.args;
        match layers_of(src.kind, &mut args) {
            Ok(layers) => {
                self.layers = layers;
                self.error = None;
            }
            Err(e) => {
                self.layers.clear();
                self.error = Some(format!("{}: {e}", t("the outline cannot be edited here")));
            }
        }
        self.fields = args;
        self.changed();
        self.preview.doc = None;
        self.live = false;
        self.grab = None;
        self.frozen = None;
        if !keep || self.view.is_none_or(|v| !src.kind.views().contains(&v)) {
            self.view = src.kind.views().first().copied();
        }
        if !keep || self.selected.is_some_and(|s| self.node(s).is_none()) {
            self.selected = None;
        }
    }

    /// A point, if it exists.
    pub fn node(&self, s: Sel) -> Option<&Node> {
        self.layers.get(s.layer)?.outline.contours.get(s.contour)?.get(s.node)
    }

    /// The layers drawn in a view, by index.
    pub fn layers_in(&self, view: ViewId) -> impl Iterator<Item = (usize, &Layer)> {
        self.layers.iter().enumerate().filter(move |(_, l)| l.key.view() == view)
    }

    /// The layer of an argument, if present.
    pub fn layer(&self, key: LayerKey) -> Option<usize> {
        self.layers.iter().position(|l| l.key == key)
    }

    /// The op's arguments as edited (no target).
    ///
    /// Complexity: O(nodes).
    pub fn args(&self) -> Map<String, Value> {
        let mut args = self.fields.clone();
        let mut holes = Vec::new();
        for l in &self.layers {
            match l.key {
                LayerKey::Hole(_) => holes.push(l.outline.to_value()),
                key => {
                    args.insert(key.name(), l.outline.to_value());
                }
            }
        }
        if !holes.is_empty() {
            args.insert("holes".into(), Value::Array(holes));
        }
        args
    }

    /// The edited outlines' size: the largest side over every view.
    ///
    /// Complexity: O(nodes).
    pub fn size(&self) -> f64 {
        self.layers.iter().filter_map(|l| l.outline.bounds()).map(|(lo, hi)| (hi - lo).max_element()).fold(0.0, f64::max)
    }

    /// The extents of a view's outlines along its two axes.
    ///
    /// Complexity: O(nodes of the view).
    pub fn view_bounds(&self, view: ViewId) -> Option<(DVec2, DVec2)> {
        path::bounds(self.layers_in(view).flat_map(|(_, l)| l.outline.contours.iter()))
    }

    /// The extents the other views set for `view`: vertical guides (x
    /// values) and horizontal ones (y values), in its coordinates.
    ///
    /// Complexity: O(nodes).
    pub fn guides(&self, view: ViewId) -> (Vec<f64>, Vec<f64>) {
        let b = |v: ViewId| self.view_bounds(v);
        let pair = |r: Option<(DVec2, DVec2)>, y: bool| r.map(|(lo, hi)| if y { vec![lo.y, hi.y] } else { vec![lo.x, hi.x] }).unwrap_or_default();
        match view {
            ViewId::Outline => (Vec::new(), Vec::new()),
            // Front (x, y): x from the top view, y from the side view.
            ViewId::Front => (pair(b(ViewId::Top), false), pair(b(ViewId::Side), true)),
            // Side (z, y): z from the top view (its vertical), y from the front.
            ViewId::Side => (pair(b(ViewId::Top), true), pair(b(ViewId::Front), true)),
            // Top (x, z): x from the front, z from the side view (its horizontal).
            ViewId::Top => (pair(b(ViewId::Front), false), pair(b(ViewId::Side), false)),
        }
    }
}

impl PixelApp {
    /// The Spec tab's selected part when it is a shell or silhouettes part.
    pub(crate) fn spec_outline_part(&self) -> Option<(ObjectKey, String)> {
        let root = self.spec.root?;
        let part = self.spec.part.clone()?;
        spec_part_source(self.doc(), root, &part).map(|_| (root, part))
    }

    /// Opens the editor on the Spec tab's outline part, or on the active
    /// object when it was made by `gen.shell` or `sdf.silhouettes`, and
    /// reads the outlines again after undo or another edit.
    ///
    /// Complexity: O(history length) when something changed.
    pub(crate) fn outline_follow(&mut self) {
        let doc = Arc::clone(self.session.doc());
        let idle = !self.outline.live && self.outline.commit.is_none() && self.outline.queued.is_none();
        if let Some((root, part)) = self.spec_outline_part()
            && let Some(src) = spec_part_source(&doc, root, &part)
        {
            let same = self.outline.part.as_ref().is_some_and(|(r, p)| *r == root && *p == part);
            if !same || (idle && doc.revision != self.outline.revision) {
                self.outline.part = Some((root, part.clone()));
                self.outline.object = crate::spec::part_object(&doc, root, &part);
                self.outline.fill(doc.revision, src, same);
            }
            return;
        }
        if self.outline.part.is_some() {
            self.outline.close();
        }
        let active = self.active_object().filter(|k| crate::spec::spec_of(&doc, *k).is_none());
        match active.and_then(|k| source_of(&doc, k).map(|s| (k, s))) {
            Some((k, s)) if self.outline.object != Some(k) => {
                self.outline.object = Some(k);
                self.outline.fill(doc.revision, s, false);
                self.ui.inspector_tab = inspector::Tab::Outline;
            }
            Some((_, s)) => {
                if idle && doc.revision != self.outline.revision {
                    self.outline.fill(doc.revision, s, true);
                }
            }
            None => {
                if self.outline.object.is_some() {
                    self.outline.close();
                }
            }
        }
    }

    /// The op call that rebuilds the edited object from the outlines; a
    /// coarse one (silhouettes at three times the voxel) for previews.
    fn outline_call(&self, coarse: bool) -> Option<OpCall> {
        let kind = self.outline.kind?;
        let mut args = self.outline.args();
        if coarse && kind == Kind::Silhouettes {
            let voxel = args
                .get("detail")
                .and_then(|d| serde_json::from_value::<p3d_ops::params::Length>(d.clone()).ok())
                .map_or((self.outline.size() / 64.0).clamp(1e-4, 1.0), |l| l.0);
            args.insert("detail".into(), json!((voxel * 3.0).clamp(1e-4, 1.0)));
        }
        if let Some((root, part)) = &self.outline.part {
            for k in kind.optional() {
                args.entry(*k).or_insert(Value::Null);
            }
            let id = self.doc().object(*root)?.public_id();
            let merge = json!({ "parts": { part.as_str(): { kind.spec_key(): args } } });
            return Some(OpCall::new("spec.patch", json!({ "target": id, "merge": merge })));
        }
        let id = self.doc().object(self.outline.object?)?.public_id();
        args.insert("target".into(), Value::String(id));
        Some(OpCall::new(kind.op(), Value::Object(args)))
    }

    /// Previews or commits the outlines as edited.
    pub fn apply_outline(&mut self, how: Apply<'_>) {
        self.outline.changed();
        match how {
            Apply::Live => {
                self.outline.live = true;
                self.outline.preview.wanted = true;
                self.start_outline_preview();
            }
            Apply::Commit(label) => {
                self.outline.live = false;
                if let Some(call) = self.outline_call(false) {
                    self.outline.queued = Some((vec![call], label.to_owned()));
                    self.start_outline_commit();
                }
            }
        }
    }

    fn start_outline_preview(&mut self) {
        if self.outline.preview.running.is_some() || !self.outline.preview.wanted {
            return;
        }
        let Some(call) = self.outline_call(true) else { return };
        self.outline.preview.wanted = false;
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::clone(self.session.registry());
        let doc = Document::clone(self.session.doc());
        let seed = self.session.seed();
        let spawned = std::thread::Builder::new().name("outline preview".into()).spawn(move || {
            let mut work = p3d_engine::Session::with_registry(registry);
            work.set_seed(seed);
            work.load(doc);
            let r = work.execute(&[call], &Default::default()).map(|_| Arc::clone(work.doc())).map_err(|e| e.error.to_string());
            let _ = tx.send(r);
        });
        if spawned.is_ok() {
            self.outline.preview.running = Some(rx);
        }
    }

    fn start_outline_commit(&mut self) {
        if self.outline.commit.is_some() {
            return;
        }
        if let Some((calls, label)) = self.outline.queued.take() {
            self.outline.commit = Some(self.session.spawn_job(calls, &label));
        }
    }

    /// Per frame: takes in finished previews and rebuilds, keeps frames
    /// coming while work runs, and follows the selection.
    ///
    /// Complexity: O(history length) when the object or revision changed.
    pub(crate) fn outline_tick(&mut self, ctx: &egui::Context) {
        if let Some(mut job) = self.outline.commit.take() {
            match self.session.poll_job(&mut job) {
                JobStatus::Pending(_) => self.outline.commit = Some(job),
                JobStatus::Done(r) => {
                    self.committed(&r);
                    self.prune_selection();
                    self.outline.revision = r.revision;
                    if !self.outline.live && self.outline.queued.is_none() {
                        self.outline.preview.doc = None;
                    }
                    self.start_outline_commit();
                }
                JobStatus::Failed(e) => {
                    let msg = e.error.to_string();
                    self.set_status(msg.clone());
                    self.outline.error = Some(msg);
                    self.outline.preview.doc = None;
                    self.start_outline_commit();
                }
            }
        }
        if let Some(rx) = &self.outline.preview.running {
            match rx.try_recv() {
                Ok(r) => {
                    self.outline.preview.running = None;
                    match r {
                        Ok(doc) if self.outline.live || self.outline.commit.is_some() || self.outline.queued.is_some() => {
                            self.outline.preview.counter += 1;
                            self.outline.preview.doc = Some((doc, self.outline.preview.counter));
                            self.outline.error = None;
                        }
                        Ok(_) => {}
                        Err(e) => self.outline.error = Some(e),
                    }
                    self.start_outline_preview();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.outline.preview.running = None,
            }
        }
        if self.outline.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.outline.canvas_hovered = std::mem::take(&mut self.outline.canvas_hover_next);
        self.outline_follow();
    }

    /// The layer and contour a command names (`layer`, `contour`), else the
    /// selected point's, else the selected view's first.
    fn outline_target(&self, params: &Value) -> Result<(usize, usize), String> {
        let e = &self.outline;
        let layer = match params.get("layer").and_then(Value::as_str) {
            Some(name) => {
                let key = LayerKey::parse(name).ok_or_else(|| format!("{} `{name}`: outline, holes/0, front, side or top", t("no layer")))?;
                e.layer(key).ok_or_else(|| format!("{} `{name}`", t("there is no outline")))?
            }
            None => match (e.selected, params.get("view").and_then(Value::as_str).and_then(ViewId::parse).or(e.view)) {
                (Some(s), None) => s.layer,
                (Some(s), Some(v)) if e.layers.get(s.layer).is_some_and(|l| l.key.view() == v) => s.layer,
                (_, Some(v)) => e.layers_in(v).next().map(|(i, _)| i).ok_or_else(|| format!("{} {}", t("no outline in the view"), v.name()))?,
                _ => return Err(t("no outline").to_owned()),
            },
        };
        let contour = params
            .get("contour")
            .and_then(Value::as_u64)
            .map(|c| c as usize)
            .or(e.selected.filter(|s| s.layer == layer).map(|s| s.contour))
            .unwrap_or(0);
        if e.layers.get(layer).is_none_or(|l| contour >= l.outline.contours.len()) {
            return Err(format!("{} {contour}", t("no contour")));
        }
        Ok((layer, contour))
    }

    /// Runs an `outline.*` command (the tab's buttons, for agents and tests).
    pub(crate) fn outline_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if id == "outline.new" {
            return self.outline_new(params);
        }
        if !self.outline.is_open() {
            return Err(t("select an object made by Shell or Silhouettes first").to_owned());
        }
        let index = || params.get("index").and_then(Value::as_u64).map(|i| i as usize);
        let point = |key: &str| -> Option<DVec2> {
            let a = params.get(key)?.as_array()?;
            let n = |v: &Value| serde_json::from_value::<p3d_ops::params::Length>(v.clone()).ok().map(|l| l.0);
            Some(DVec2::new(n(a.first()?)?, n(a.get(1)?)?))
        };
        match id {
            "outline.select_view" => {
                let v = params.get("view").and_then(Value::as_str).and_then(ViewId::parse).ok_or(t("view is outline, front, side or top"))?;
                let kind = self.outline.kind.ok_or(t("nothing to edit"))?;
                if !kind.views().contains(&v) {
                    return Err(format!("{} {}", kind.op(), t("has no such view")));
                }
                self.outline.view = Some(v);
                self.outline.selected = self.outline.selected.filter(|s| self.outline.layers.get(s.layer).is_some_and(|l| l.key.view() == v));
                Ok(json!({ "view": v.name() }))
            }
            "outline.select_point" => {
                let (layer, contour) = self.outline_target(params)?;
                let node = index().ok_or(t("index is required"))?;
                let s = Sel { layer, contour, node };
                self.outline.node(s).ok_or(t("no such point"))?;
                self.outline.selected = Some(s);
                self.outline.view = Some(self.outline.layers[layer].key.view());
                Ok(json!({ "selected": node }))
            }
            "outline.set_path" => {
                let v = params.get("path").ok_or(t("path is required: SVG path data or [[x, y], ...]"))?;
                let outline = Outline::from_value(v)?;
                let key = match params.get("layer").and_then(Value::as_str) {
                    Some(name) => LayerKey::parse(name).ok_or(t("no such layer"))?,
                    None => match self.outline.view.unwrap_or(ViewId::Outline) {
                        ViewId::Outline => LayerKey::Outline,
                        ViewId::Front => LayerKey::Front,
                        ViewId::Side => LayerKey::Side,
                        ViewId::Top => LayerKey::Top,
                    },
                };
                self.outline_put(key, Some(outline))?;
                self.apply_outline(Apply::Commit(t("Edit outline")));
                Ok(json!({ "layer": key.name() }))
            }
            "outline.set_points" => {
                let (layer, contour) = self.outline_target(params).or_else(|_| self.outline_target(&json!({ "contour": 0 })))?;
                let pts = params.get("points").and_then(Value::as_array).ok_or(t("points is required: [[x, y], ...]"))?;
                let nodes: Vec<Node> = pts
                    .iter()
                    .map(|p| {
                        let a = p.as_array().filter(|a| a.len() == 2)?;
                        let n = |v: &Value| serde_json::from_value::<p3d_ops::params::Length>(v.clone()).ok().map(|l| l.0);
                        Some(Node::corner(DVec2::new(n(&a[0])?, n(&a[1])?)))
                    })
                    .collect::<Option<_>>()
                    .ok_or(t("each point is [x, y]"))?;
                if nodes.len() < 3 {
                    return Err(t("a contour needs at least three points").to_owned());
                }
                self.outline.layers[layer].outline.contours[contour] = nodes;
                self.outline.selected = None;
                self.apply_outline(Apply::Commit(t("Edit outline")));
                Ok(json!({ "points": pts.len() }))
            }
            "outline.move_point" => {
                let (layer, contour) = self.outline_target(params)?;
                let node = index().or(self.outline.selected.map(|s| s.node)).ok_or(t("index is required"))?;
                let to = point("to").ok_or(t("to is required: [x, y] in meters"))?;
                let c = self.outline.layers.get_mut(layer).and_then(|l| l.outline.contours.get_mut(contour)).ok_or(t("no such contour"))?;
                c.get_mut(node).ok_or(t("no such point"))?.p = to;
                self.outline.selected = Some(Sel { layer, contour, node });
                self.apply_outline(Apply::Commit(t("Move point")));
                Ok(json!({ "moved": node }))
            }
            "outline.insert_point" => {
                let (layer, contour) = self.outline_target(params)?;
                let seg = params.get("segment").and_then(Value::as_u64).map(|i| i as usize).or(self.outline.selected.map(|s| s.node)).unwrap_or(0);
                let c = &mut self.outline.layers[layer].outline.contours[contour];
                if seg >= c.len() {
                    return Err(format!("{} {seg}", t("no segment")));
                }
                let tt = params.get("t").and_then(Value::as_f64).unwrap_or(0.5).clamp(0.0, 1.0);
                let node = path::insert(c, seg, tt);
                self.outline.selected = Some(Sel { layer, contour, node });
                self.apply_outline(Apply::Commit(t("Add point")));
                Ok(json!({ "inserted": node }))
            }
            "outline.delete_point" => {
                let (layer, contour) = self.outline_target(params)?;
                let node = index().or(self.outline.selected.map(|s| s.node)).ok_or(t("index is required"))?;
                self.outline_delete(Sel { layer, contour, node })?;
                Ok(json!({ "deleted": node }))
            }
            "outline.toggle_smooth" => {
                let (layer, contour) = self.outline_target(params)?;
                let node = index().or(self.outline.selected.map(|s| s.node)).ok_or(t("index is required"))?;
                let c = &mut self.outline.layers[layer].outline.contours[contour];
                if node >= c.len() {
                    return Err(t("no such point").to_owned());
                }
                path::toggle_smooth(c, node);
                let smooth = c[node].smooth;
                self.outline.selected = Some(Sel { layer, contour, node });
                self.apply_outline(Apply::Commit(if smooth { t("Smooth point") } else { t("Corner point") }));
                Ok(json!({ "smooth": smooth }))
            }
            "outline.set" => {
                let obj = params.as_object().ok_or(t("give the fields to set"))?;
                let known = ["cup", "bend", "twist", "thickness", "detail", "round"];
                for (k, v) in obj {
                    if !known.contains(&k.as_str()) {
                        return Err(format!("{} `{k}`: {}", t("unknown field"), known.join(", ")));
                    }
                    if v.is_null() {
                        self.outline.fields.remove(k);
                    } else {
                        self.outline.fields.insert(k.clone(), v.clone());
                    }
                }
                self.apply_outline(Apply::Commit(t("Shape settings")));
                Ok(Value::Object(self.outline.fields.clone()))
            }
            "outline.add" => {
                // A hole, or a missing side or top view, sized from the rest.
                let what = params.get("layer").and_then(Value::as_str).unwrap_or("hole");
                let key = match (self.outline.kind, what) {
                    (Some(Kind::Shell), "hole" | "holes") => LayerKey::Hole(self.outline.layers_in(ViewId::Outline).count().saturating_sub(1)),
                    (Some(Kind::Silhouettes), "side") => LayerKey::Side,
                    (Some(Kind::Silhouettes), "top") => LayerKey::Top,
                    _ => return Err(t("add a hole to a shell, or a side or top view to silhouettes").to_owned()),
                };
                if self.outline.layer(key).is_some() {
                    return Err(format!("{} {}", key.name(), t("is there already")));
                }
                let outline = self.outline_starter(key);
                self.outline_put(key, Some(outline))?;
                self.outline.view = Some(key.view());
                self.apply_outline(Apply::Commit(t("Add outline")));
                Ok(json!({ "layer": key.name() }))
            }
            "outline.remove" => {
                let name = params.get("layer").and_then(Value::as_str).ok_or(t("layer is required: holes/0, side or top"))?;
                let key = LayerKey::parse(name).ok_or(t("no such layer"))?;
                if matches!(key, LayerKey::Outline | LayerKey::Front) {
                    return Err(t("the main outline stays; edit it instead").to_owned());
                }
                if self.outline.kind == Some(Kind::Silhouettes) && self.outline.layers.len() <= 2 {
                    return Err(t("silhouettes need two views").to_owned());
                }
                self.outline_put(key, None)?;
                self.outline.selected = None;
                self.apply_outline(Apply::Commit(t("Remove outline")));
                Ok(json!({ "removed": name }))
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// Puts (or removes) the outline of an argument; holes stay numbered
    /// in order.
    fn outline_put(&mut self, key: LayerKey, outline: Option<Outline>) -> Result<(), String> {
        let e = &mut self.outline;
        match (e.layer(key), outline) {
            (Some(i), Some(o)) => e.layers[i].outline = o,
            (None, Some(o)) => e.layers.push(Layer { key, outline: o }),
            (Some(i), None) => {
                e.layers.remove(i);
            }
            (None, None) => return Err(format!("{} {}", t("there is no"), key.name())),
        }
        e.layers.sort_by_key(|l| l.key);
        let mut hole = 0;
        for l in &mut e.layers {
            if let LayerKey::Hole(_) = l.key {
                l.key = LayerKey::Hole(hole);
                hole += 1;
            }
        }
        e.selected = e.selected.filter(|s| e.node(*s).is_some());
        Ok(())
    }

    /// A first outline for a new layer: a hole in the middle of the shell,
    /// or a side or top view spanning the other views' extents.
    fn outline_starter(&self, key: LayerKey) -> Outline {
        let e = &self.outline;
        let front = e.view_bounds(ViewId::Front).or(e.view_bounds(ViewId::Outline)).unwrap_or((DVec2::splat(-0.05), DVec2::splat(0.05)));
        let depth = ((front.1 - front.0).min_element() * 0.25).max(0.005);
        match key {
            LayerKey::Hole(_) => {
                let c = (front.0 + front.1) * 0.5;
                let r = (front.1 - front.0) * 0.12;
                Outline::rect(c - r, c + r)
            }
            LayerKey::Side => {
                let z = e.view_bounds(ViewId::Top).map_or((-depth, depth), |(lo, hi)| (lo.y, hi.y));
                Outline::rect(DVec2::new(z.0, front.0.y), DVec2::new(z.1, front.1.y))
            }
            _ => {
                let z = e.view_bounds(ViewId::Side).map_or((-depth, depth), |(lo, hi)| (lo.x, hi.x));
                Outline::rect(DVec2::new(front.0.x, z.0), DVec2::new(front.1.x, z.1))
            }
        }
    }

    /// Deletes a point (a contour keeps three).
    pub(crate) fn outline_delete(&mut self, s: Sel) -> Result<(), String> {
        let c = self.outline.layers.get_mut(s.layer).and_then(|l| l.outline.contours.get_mut(s.contour)).ok_or(t("no such contour"))?;
        if s.node >= c.len() {
            return Err(t("no such point").to_owned());
        }
        if c.len() <= 3 {
            return Err(t("a contour keeps at least three points").to_owned());
        }
        c.remove(s.node);
        self.outline.selected = None;
        self.apply_outline(Apply::Commit(t("Delete point")));
        Ok(())
    }

    /// Add > Shell or Silhouettes: a starter object (leaf, ear or the axe
    /// head), selected.
    fn outline_new(&mut self, params: &Value) -> Result<Value, String> {
        let which = params.get("starter").and_then(Value::as_str).unwrap_or("leaf");
        let (op, mut args) = starter(which).ok_or_else(|| format!("{} `{which}`: leaf, ear or axe_head", t("no starter")))?;
        let name = crate::outliner::free_name(self.doc(), which);
        args["name"] = json!(name);
        let r = self.execute(&[OpCall::new(op, args)])?;
        if let Ok(k) = self.doc().resolve(&name) {
            self.handle_viewport(vec![crate::viewport::ViewportEvent::Picked { object: Some(k), add: false }]);
            self.outline_follow();
        }
        Ok(r)
    }

    /// The editor's state for agents (`ui.inspect`): the outlines as the
    /// op takes them, the selection, and the selected view's points on the
    /// canvas (screen positions), and the built mesh's size.
    ///
    /// Complexity: O(nodes + vertices of the mesh).
    pub(crate) fn outline_state(&self) -> Value {
        let e = &self.outline;
        let doc = self.doc();
        let layers: Vec<Value> = e
            .layers
            .iter()
            .map(|l| {
                json!({ "layer": l.key.name(), "view": l.key.view().name(), "contours": l.outline.contours.len(),
                "points": l.outline.contours.iter().map(Vec::len).sum::<usize>(), "value": l.outline.to_value() })
            })
            .collect();
        let points: Vec<Value> = e
            .screen
            .iter()
            .map(|(s, at)| json!({ "layer": e.layers.get(s.layer).map(|l| l.key.name()), "contour": s.contour, "index": s.node, "at": [at.x, at.y] }))
            .collect();
        // The built mesh, so a rebuild can be checked.
        let mesh = e.object.and_then(|k| doc.object_mesh(k)).map(|m| {
            let b = m.bounds().map(|(lo, hi)| [lo.to_array(), hi.to_array()]);
            json!({ "faces": m.face_count(), "bounds": b })
        });
        json!({
            "object": e.object.and_then(|k| doc.object(k)).map(|o| o.public_id()),
            "mesh": mesh,
            "part": e.part.as_ref().map(|(_, p)| p.clone()),
            "op": e.kind.map(Kind::op),
            "view": e.view.map(ViewId::name),
            "views": e.kind.map(|k| k.views().iter().map(|v| v.name()).collect::<Vec<_>>()),
            "layers": layers,
            "fields": e.fields,
            "selected": e.selected.map(|s| json!({ "layer": e.layers.get(s.layer).map(|l| l.key.name()), "contour": s.contour, "index": s.node })),
            "canvas": e.canvas.map(|r| [r.left(), r.top(), r.width(), r.height()]),
            "points": points,
            "dragging": e.grab.is_some(),
            "previewing": e.preview.doc.is_some(),
            "busy": e.busy(),
            "later": e.later,
            "error": e.error,
        })
    }
}

/// Starter objects: the op and its arguments.
pub fn starter(name: &str) -> Option<(&'static str, Value)> {
    match name {
        "leaf" => Some((
            "gen.shell",
            json!({ "outline": "M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 C -0.025 0.07 -0.03 0.02 0 0 Z", "cup": 0.3, "bend": 0.2, "detail": "5mm" }),
        )),
        "ear" => Some((
            "gen.shell",
            json!({ "outline": "M 0 0 C 0.03 0.01 0.035 0.06 0.01 0.1 C -0.005 0.07 -0.02 0.03 0 0 Z", "thickness": "3mm", "cup": 0.4, "bend": 0.15, "detail": "4mm" }),
        )),
        "axe_head" => Some((
            "sdf.silhouettes",
            json!({
                "front": "M -0.03 0.045 L 0.03 0.045 C 0.07 0.052 0.105 0.09 0.15 0.14 L 0.158 0.128 C 0.192 0.06 0.192 -0.06 0.158 -0.128 L 0.15 -0.14 C 0.105 -0.09 0.07 -0.052 0.03 -0.045 L -0.03 -0.045 C -0.07 -0.052 -0.105 -0.09 -0.15 -0.14 L -0.158 -0.128 C -0.192 -0.06 -0.192 0.06 -0.158 0.128 L -0.15 0.14 C -0.105 0.09 -0.07 0.052 -0.03 0.045 Z M 0.085 0 L 0.105 0.026 L 0.125 0 L 0.105 -0.026 Z M -0.085 0 L -0.105 0.026 L -0.125 0 L -0.105 -0.026 Z",
                "top": [[-0.2, 0], [-0.138, 0.0055], [-0.05, 0.0095], [0, 0.011], [0.05, 0.0095], [0.138, 0.0055], [0.2, 0], [0.138, -0.0055], [0.05, -0.0095], [0, -0.011], [-0.05, -0.0095], [-0.138, -0.0055]],
                "round": "1.5mm",
                "detail": "3.5mm"
            }),
        )),
        _ => None,
    }
}

/// The blueprint: while silhouettes are edited, each view's outline as a
/// flat wire on a plane beside the object (front behind it, side to its
/// left, top below), the selected view brighter.
///
/// Complexity: O(nodes x curve pieces) per frame.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui) {
    let rect = app.viewport.rect;
    let e = &app.outline;
    if app.ui.mode.is_edit() || !rect.is_positive() || e.kind != Some(Kind::Silhouettes) {
        return;
    }
    let Some(k) = e.object else { return };
    let active = app.active_object();
    if active != Some(k) && active != e.part.as_ref().map(|p| p.0) {
        return;
    }
    let Some(world) = app.doc().world_affine(k).map(Mat4::from) else { return };
    // The hull's extents: x and y from the front, z from the side or top.
    let f = e.view_bounds(ViewId::Front);
    let s = e.view_bounds(ViewId::Side);
    let tp = e.view_bounds(ViewId::Top);
    let (Some(f), true) = (f, s.is_some() || tp.is_some()) else { return };
    let z = s.map(|(lo, hi)| (lo.x, hi.x)).or(tp.map(|(lo, hi)| (lo.y, hi.y))).unwrap_or((0.0, 0.0));
    let gap = e.size() * 0.15;
    let depth = |v: ViewId| match v {
        ViewId::Front | ViewId::Outline => z.0 - gap,
        ViewId::Side => f.0.x - gap,
        ViewId::Top => f.0.y - gap,
    };
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0)) * world;
    let tk = Tokens::get();
    let p = ui.painter_at(rect);
    for l in &e.layers {
        let v = l.key.view();
        let color = if Some(v) == e.view { tk.accent } else { tk.text_dim.gamma_multiply(0.6) };
        for c in &l.outline.contours {
            let pts: Vec<Pos2> = path::flatten(c, 12).into_iter().filter_map(|q| project(vp, rect, v.to_3d(q, depth(v)))).collect();
            if pts.len() > 1 {
                p.add(egui::Shape::line(pts, Stroke::new(1.2, color)));
            }
        }
    }
}

/// A point of the object's space on the viewport, or none behind the
/// camera.
fn project(vp: Mat4, rect: egui::Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

#[cfg(test)]
mod tests;
