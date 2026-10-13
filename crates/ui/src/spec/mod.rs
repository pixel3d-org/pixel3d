//! The Model Spec editor: the GUI side of spec-first modelling (ADR 0010).
//! Selecting a spec root, or one of its part objects (which jumps to its
//! part), opens the Spec tab of Properties: the parts, the selected part's
//! shape, anchor, join, transform and tags, and the raw JSON source with
//! errors at their paths. Every edit is a `spec.patch` merge patch run as a
//! background job through the normal op path (one undo step). SDF parts
//! reuse the Shapes editor, which then patches the spec instead of running
//! the SDF op.
//!
//! Also here: what the Outliner shows for specs (kinds, joins, blended
//! parts), the landmark overlay (labelled points with normal ticks; a click
//! while picking sets the anchor), and the provenance of the face under the
//! pointer.

pub mod panel;

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Pos2, Stroke, pos2, vec2};
use glam::{Mat4, Vec3};
use p3d_engine::{JobStatus, OpCall};
use p3d_scene::{Document, ObjectKey, ProvEntry};
use p3d_spec::LandmarkPoint;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::viewport::ViewportEvent;

/// Starter specs for Add > Spec: name and menu label.
pub const STARTERS: [(&str, &str); 2] = [("lamp", "Lamp"), ("elf_head", "Elf head")];

/// The source of a starter spec: the lamp of the compiler tests and the elf
/// head of `docs/agents/spec.md`.
pub fn starter(name: &str) -> Option<Value> {
    match name {
        "lamp" => Some(json!({ "parts": {
            "base": { "primitive": { "cylinder": { "radius": 0.1, "height": 0.02 } } },
            "pole": { "primitive": { "cylinder": { "radius": 0.01, "height": 0.4 } }, "anchor": { "on": "base", "landmark": "top" } },
            "shade": { "primitive": { "cone": { "radius_bottom": 0.12, "radius_top": 0.05, "height": 0.15 } }, "anchor": { "on": "pole", "landmark": "top" } }
        } })),
        "elf_head" => Some(json!({ "category": "humanoid_head", "parts": {
            "head": { "sdf": [
                { "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" },
                { "ellipsoid": { "at": [0, 0.06, 0.035], "radii": [0.055, 0.045, 0.055] }, "name": "jaw", "smooth_union": "2.5cm" }],
                "detail": "5mm", "landmarks": "category" },
            "nose": { "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0, -0.025, 0.012], "radius": "6mm", "radius_b": "8mm" } }],
                "anchor": { "on": "head", "landmark": "nose_root" }, "join": "blend", "blend": "6mm" },
            "ear_l": { "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0.04, 0.06, -0.02], "radius": "1cm", "radius_b": "2mm" }, "name": "lobe" }],
                "anchor": { "on": "head", "landmark": "ear_root_l" }, "join": "blend", "blend": "8mm" },
            "ear_r": { "mirror": "ear_l" },
            "eye_l": { "primitive": { "uv_sphere": { "radius": 0.012, "origin": "center" } },
                "anchor": { "on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008] } },
            "eye_r": { "mirror": "eye_l" }
        } })),
        _ => None,
    }
}

/// What the editor and the Outliner show of one part.
#[derive(Debug, Clone, PartialEq)]
pub struct PartInfo {
    pub name: String,
    /// `sdf`, `cylinder`, `lathe`, `mirror of ear_l`, `instance of post`.
    pub kind: String,
    /// The shape key: `sdf`, `primitive`, `lathe`, ..., `mirror`, `instance`.
    pub key: &'static str,
    /// `separate`, `blend`, `weld` or `decal` (a mirror joins like its
    /// source).
    pub join: &'static str,
    /// The part its anchor is on.
    pub on: Option<String>,
    /// Its object (none for a blended part).
    pub object: Option<ObjectKey>,
    /// The object whose mesh a blended part is merged into.
    pub blend_into: Option<ObjectKey>,
}

/// The parts of the spec on `root`, in name order. A spec that does not
/// parse lists its part names with an unknown kind.
///
/// Complexity: O(size of the spec + parts x (mirror chain + blend chain)).
pub fn parts_of(doc: &Document, root: ObjectKey) -> Vec<PartInfo> {
    use p3d_spec::model::{Join, Kind};
    let Some(record) = doc.spec(root) else { return Vec::new() };
    let object = |name: &str| part_object(doc, root, name);
    let Ok(spec) = p3d_spec::parse(&record.source) else {
        let names: Vec<String> = record.source.get("parts").and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
        return names
            .into_iter()
            .map(|n| PartInfo { object: object(&n), name: n, kind: "?".into(), key: "", join: "separate", on: None, blend_into: None })
            .collect();
    };
    let bound = spec.parts.len();
    // A mirror joins like the end of its mirror chain, on that part's parent.
    let effective = |name: &str| -> (Join, Option<String>) {
        let Some(mut p) = spec.parts.get(name) else { return (Join::Separate, None) };
        let mut on = p.anchor.as_ref().and_then(|a| a.on.clone());
        for _ in 0..bound {
            let Some(src) = p.mirror.as_deref().and_then(|m| spec.parts.get(m)) else { break };
            p = src;
            on = p.anchor.as_ref().and_then(|a| a.on.clone());
        }
        (p.join(), on)
    };
    spec.parts
        .iter()
        .map(|(name, p)| {
            let (kind, key) = match p.kind(&format!("/parts/{name}")) {
                Ok(Kind::Sdf(_)) => ("sdf".to_owned(), "sdf"),
                Ok(Kind::Primitive(k, _)) => (k.to_owned(), "primitive"),
                Ok(Kind::Op(k, _, _)) => (k.to_owned(), k),
                Ok(Kind::Mirror(src)) => (format!("{} {src}", t("mirror of")), "mirror"),
                Ok(Kind::Instance(src)) => (format!("{} {src}", t("instance of")), "instance"),
                Err(_) => ("?".to_owned(), ""),
            };
            let (join, on) = effective(name);
            let join = match join {
                Join::Separate => "separate",
                Join::Blend => "blend",
                Join::Weld => "weld",
                Join::Decal => "decal",
            };
            // A blended part merges into its parent, or further up when the
            // parent is blended too (blended parts have no object).
            let mut blend_into = None;
            if join == "blend" {
                let mut at = on;
                for _ in 0..bound {
                    let Some(n) = at else { break };
                    if let Some(k) = object(&n) {
                        blend_into = Some(k);
                        break;
                    }
                    at = effective(&n).1;
                }
            }
            let on = p.anchor.as_ref().and_then(|a| a.on.clone());
            PartInfo { name: name.clone(), kind, key, join, on, object: object(name), blend_into }
        })
        .collect()
}

/// The object a part compiled into (none for a blended part).
pub fn part_object(doc: &Document, root: ObjectKey, part: &str) -> Option<ObjectKey> {
    doc.object_by_id(doc.spec(root)?.parts.get(part)?.object?)
}

/// The spec root and part of an object: a root itself (no part), or a
/// part object under its root.
pub fn spec_of(doc: &Document, k: ObjectKey) -> Option<(ObjectKey, Option<String>)> {
    if doc.spec(k).is_some() {
        return Some((k, None));
    }
    let o = doc.object(k)?;
    let root = o.parent()?;
    let record = doc.spec(root)?;
    record.parts.contains_key(o.name()).then(|| (root, Some(o.name().to_owned())))
}

/// The JSON of one part of the spec on `root`.
pub fn part_source(doc: &Document, root: ObjectKey, part: &str) -> Option<Value> {
    doc.spec(root)?.source.get("parts")?.get(part).cloned()
}

/// An error at a spec path (`/parts/eye_l/anchor/landmark`), with the line
/// of the raw view it points at when known.
#[derive(Debug, Clone, PartialEq)]
pub struct SpecIssue {
    pub path: String,
    pub message: String,
    pub line: Option<usize>,
}

/// Landmarks by part, for one document state.
type LandmarkCache = (u64, BTreeMap<(ObjectKey, String), Arc<Vec<LandmarkPoint>>>);

/// Every spec root's parts, by root.
pub type SpecParts = BTreeMap<ObjectKey, Vec<PartInfo>>;

/// The spec node of a face: its object, part and provenance entry.
type Provenance = (ObjectKey, String, ProvEntry);

/// The editor between frames.
#[derive(Default)]
pub struct SpecEditor {
    /// The spec root being edited, and the selected part.
    pub root: Option<ObjectKey>,
    pub part: Option<String>,
    /// The object that was active when the editor last followed it.
    followed: Option<ObjectKey>,
    /// The raw JSON view: the text and whether it was edited.
    pub json: String,
    pub json_edited: bool,
    /// The revision the JSON text was taken at.
    json_revision: u64,
    /// Why the raw text was not applied (JSON syntax or spec errors).
    pub json_error: Option<SpecIssue>,
    /// The last run's error, at its spec path.
    pub error: Option<SpecIssue>,
    /// The patch running and the next one waiting.
    commit: Option<p3d_engine::Job>,
    queued: Option<(Vec<OpCall>, String)>,
    /// The selected part as edited while a value is dragged (the source
    /// changes when the drag ends), and the patch that will commit it.
    pub draft: Option<(ObjectKey, String, Value)>,
    pub pending: Option<(Value, String)>,
    /// The blended part row under the pointer (its faces light up).
    pub hovered: Option<(ObjectKey, String)>,
    pub hover_next: Option<(ObjectKey, String)>,
    /// Picking a landmark for the selected part's anchor in the viewport.
    pub picking: bool,
    landmarks: Option<LandmarkCache>,
    parts: Option<(u64, Arc<SpecParts>)>,
    highlight: Option<(u64, (Arc<p3d_render::EditOverlay>, u64))>,
    /// The face provenance under the pointer: where and in which scene it
    /// was asked, and the answer (object, part, entry).
    provenance: Option<(Pos2, u64, Option<Provenance>)>,
    /// Whether the provenance label showed last frame (the status bar
    /// repeats it).
    prov_shown: bool,
}

impl SpecEditor {
    pub fn busy(&self) -> bool {
        self.commit.is_some() || self.queued.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.root.is_some()
    }

    /// The face provenance shown under the pointer, as status bar text.
    pub fn hover_text(&self) -> Option<String> {
        let (_, _, Some((_, part, e))) = self.provenance.as_ref().filter(|_| self.prov_shown)? else { return None };
        Some(format!("{part}: {} ({})", e.path, e.op))
    }
}

/// A JSON pointer from an op error (`/ops/0/args/spec/parts/...` or
/// `/merge/parts/...`) as a path into the spec (`/parts/...`).
pub fn spec_path(op_path: &str) -> String {
    let mut rest = op_path;
    if let Some(r) = rest.strip_prefix("/ops/") {
        rest = r.find('/').map_or("", |i| &r[i..]);
        rest = rest.strip_prefix("/args").unwrap_or(rest);
    }
    for prefix in ["/spec", "/merge"] {
        if let Some(r) = rest.strip_prefix(prefix) {
            return r.to_owned();
        }
    }
    rest.to_owned()
}

/// The 0-based line of `text` (JSON) where the value at `pointer` starts,
/// or of its deepest ancestor that exists.
///
/// Complexity: O(text length x pointer depth).
pub fn pointer_line(text: &str, pointer: &str) -> Option<usize> {
    let target: Vec<String> = pointer.split('/').skip(1).map(|s| s.replace("~1", "/").replace("~0", "~")).collect();
    // One frame per open container: the segment of the value being read.
    struct Frame {
        object: bool,
        seg: String,
        index: usize,
        want_key: bool,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut best: Option<(usize, usize)> = None;
    let mut line = 0;
    // A value starts here: does its path match the target, or a prefix?
    let mut at_value = |stack: &[Frame], line: usize| -> bool {
        let depth = stack.len();
        if depth > target.len() || !stack.iter().zip(&target).all(|(f, t)| f.seg == *t) {
            return false;
        }
        if best.is_none_or(|(d, _)| depth > d) {
            best = Some((depth, line));
        }
        depth == target.len()
    };
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\n' => line += 1,
            b' ' | b'\t' | b'\r' | b':' => {}
            b'"' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                let raw = text.get(start..(i + 1).min(text.len())).unwrap_or("");
                match stack.last_mut() {
                    Some(f) if f.object && f.want_key => {
                        f.seg = serde_json::from_str::<String>(raw).unwrap_or_default();
                        f.want_key = false;
                    }
                    _ => {
                        if at_value(&stack, line) {
                            return Some(line);
                        }
                    }
                }
            }
            b'{' | b'[' => {
                if at_value(&stack, line) {
                    return Some(line);
                }
                let object = c == b'{';
                stack.push(Frame { object, seg: if object { String::new() } else { "0".into() }, index: 0, want_key: object });
            }
            b'}' | b']' => {
                stack.pop();
            }
            b',' => {
                if let Some(f) = stack.last_mut() {
                    if f.object {
                        f.want_key = true;
                    } else {
                        f.index += 1;
                        f.seg = f.index.to_string();
                    }
                }
            }
            _ => {
                if at_value(&stack, line) {
                    return Some(line);
                }
                while i + 1 < bytes.len() && !matches!(bytes[i + 1], b',' | b'}' | b']' | b' ' | b'\n' | b'\r' | b'\t') {
                    i += 1;
                }
            }
        }
        i += 1;
    }
    best.map(|b| b.1)
}

impl PixelApp {
    /// The Spec tab's selected part when it is an SDF part (the Shapes
    /// editor then edits it).
    pub(crate) fn spec_sdf_part(&self) -> Option<(ObjectKey, String)> {
        let root = self.spec.root?;
        let part = self.spec.part.clone()?;
        let p = self.doc().spec(root)?.source.get("parts")?.get(&part)?;
        p.get("sdf").is_some().then_some((root, part))
    }

    /// The spec root as an op target.
    fn spec_target(&self) -> Option<String> {
        self.doc().object(self.spec.root?).map(|o| o.public_id())
    }

    /// Patches the edited spec with a JSON Merge Patch, as a background job
    /// (one undo step).
    pub fn spec_patch(&mut self, merge: Value, label: &str) -> Result<Value, String> {
        let target = self.spec_target().ok_or(t("select a spec root or one of its parts"))?;
        let call = OpCall::new("spec.patch", json!({ "target": target, "merge": merge }));
        self.spec_run(vec![call], label);
        Ok(json!({ "patching": target }))
    }

    /// Patches the selected part: `fields` merge into it.
    pub fn spec_patch_part(&mut self, fields: Value, label: &str) -> Result<Value, String> {
        let part = self.spec.part.clone().ok_or(t("select a part"))?;
        self.spec_patch(json!({ "parts": { part: fields } }), label)
    }

    /// Runs spec op calls as a background job (one undo step); while one
    /// runs, the newest waiting call follows it.
    fn spec_run(&mut self, calls: Vec<OpCall>, label: &str) {
        self.spec.error = None;
        self.spec.queued = Some((calls, label.to_owned()));
        self.spec_start();
    }

    fn spec_start(&mut self) {
        if self.spec.commit.is_some() {
            return;
        }
        if let Some((calls, label)) = self.spec.queued.take() {
            self.spec.commit = Some(self.session.spawn_job(calls, &label));
        }
    }

    /// Replaces the spec source with JSON text (the raw view's Apply),
    /// checked first so errors show at their lines.
    pub fn spec_apply_json(&mut self, text: &str) -> Result<Value, String> {
        let root = self.spec.root.ok_or(t("select a spec root"))?;
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                let message = format!("{} {}, {} {}: {e}", t("line"), e.line(), t("column"), e.column());
                self.spec.json_error = Some(SpecIssue { path: String::new(), message: message.clone(), line: e.line().checked_sub(1) });
                return Err(message);
            }
        };
        if let Err(e) = p3d_spec::parse(&value) {
            let line = pointer_line(text, &e.path);
            self.spec.json_error = Some(SpecIssue { path: e.path.clone(), message: e.message.clone(), line });
            return Err(format!("{}: {}", e.path, e.message));
        }
        self.spec.json_error = None;
        let doc = self.doc();
        let name = doc.object(root).map(|o| o.name().to_owned()).ok_or(t("the spec root is gone"))?;
        let parent = doc.object(root).and_then(|o| o.parent()).and_then(|p| doc.object(p)).map(|o| o.public_id());
        let mut args = json!({ "name": name, "spec": value });
        if let Some(p) = parent {
            args["parent"] = json!(p);
        }
        self.spec_run(vec![OpCall::new("spec.set", args)], t("Edit spec"));
        Ok(json!({ "applied": name }))
    }

    /// Per frame: takes in finished runs and follows the active object.
    ///
    /// Complexity: O(spec size) when the selection or revision changed.
    pub(crate) fn spec_tick(&mut self, ctx: &egui::Context) {
        self.spec.hovered = self.spec.hover_next.take();
        if let Some(mut job) = self.spec.commit.take() {
            match self.session.poll_job(&mut job) {
                JobStatus::Pending(_) => self.spec.commit = Some(job),
                JobStatus::Done(r) => {
                    self.committed(&r);
                    self.prune_selection();
                    // The raw text was applied: it follows the source again.
                    self.spec.json_edited = false;
                    self.spec_start();
                }
                JobStatus::Failed(e) => {
                    let path = e.error.path.as_deref().map(spec_path).unwrap_or_default();
                    let mut message = e.error.message.clone();
                    if !e.error.suggestions.is_empty() {
                        message = format!("{message} ({} {})", t("did you mean"), e.error.suggestions.join(", "));
                    }
                    self.set_status(e.error.to_string());
                    let line = pointer_line(&self.spec.json, &path);
                    self.spec.error = Some(SpecIssue { path, message, line });
                    self.spec_start();
                }
            }
            if !self.spec.busy() && self.spec.pending.is_none() {
                self.spec.draft = None;
            }
        }
        if self.spec.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        self.spec_follow();
    }

    /// Opens the editor on the active object's spec: a root, or a part
    /// object (its part is selected). Brings the Spec tab forward when the
    /// selection moves onto a spec.
    ///
    /// Complexity: O(spec size) when the revision changed (the raw text).
    pub(crate) fn spec_follow(&mut self) {
        let active = self.active_object();
        let doc = Arc::clone(self.session.doc());
        if active != self.spec.followed {
            self.spec.followed = active;
            match active.and_then(|k| spec_of(&doc, k)) {
                Some((root, part)) => {
                    if self.spec.root != Some(root) {
                        self.spec.json_edited = false;
                        self.spec.json_error = None;
                        self.spec.error = None;
                        self.spec.part = None;
                    }
                    self.spec.root = Some(root);
                    // A part object selects its part; the root keeps the
                    // part chosen in the tab.
                    if part.is_some() {
                        self.spec.part = part;
                    }
                    self.spec.picking = false;
                    self.ui.inspector_tab = crate::inspector::Tab::Spec;
                }
                None => {
                    self.spec.root = None;
                    self.spec.part = None;
                    self.spec.picking = false;
                }
            }
        }
        // The root or the part may be gone (undo, delete, a patch).
        if self.spec.root.is_some_and(|r| doc.spec(r).is_none()) {
            self.spec.root = None;
            self.spec.part = None;
        }
        if let (Some(root), Some(part)) = (self.spec.root, &self.spec.part)
            && part_source(&doc, root, part).is_none()
        {
            self.spec.part = None;
        }
        if self.spec.draft.as_ref().is_some_and(|(r, p, _)| Some(*r) != self.spec.root || Some(p) != self.spec.part.as_ref()) {
            self.spec.draft = None;
            self.spec.pending = None;
        }
        // The raw view follows the source unless it is being edited.
        if let Some(root) = self.spec.root
            && !self.spec.json_edited
            && (self.spec.json_revision != doc.revision || self.spec.json.is_empty())
        {
            self.spec.json = doc.spec(root).and_then(|r| serde_json::to_string_pretty(&r.source).ok()).unwrap_or_default();
            self.spec.json_revision = doc.revision;
        }
    }

    /// Selects a part of the edited spec (or the whole spec with `None`).
    pub fn spec_select_part(&mut self, part: Option<&str>) -> Result<Value, String> {
        let root = self.spec.root.ok_or(t("select a spec root or one of its parts"))?;
        if let Some(p) = part
            && part_source(self.doc(), root, p).is_none()
        {
            return Err(format!("{} `{p}`", t("the spec has no part")));
        }
        self.spec.part = part.map(str::to_owned);
        self.spec.picking = false;
        Ok(json!({ "part": self.spec.part }))
    }

    /// Every spec root's parts, for the Outliner (built once per document
    /// state).
    ///
    /// Complexity: O(objects + spec sizes) when the scene changed, else O(1).
    pub fn spec_parts(&mut self) -> Arc<SpecParts> {
        let key = self.scene_key();
        if let Some((k, p)) = &self.spec.parts
            && *k == key
        {
            return Arc::clone(p);
        }
        let doc = self.doc();
        let map: SpecParts = doc.spec_roots().map(|r| (r, parts_of(doc, r))).collect();
        let map = Arc::new(map);
        self.spec.parts = Some((key, Arc::clone(&map)));
        map
    }

    /// The landmarks of a part of the edited spec in world space (found
    /// once per document state).
    ///
    /// Complexity: as `part_landmarks` when not cached, else O(log parts).
    pub fn spec_landmarks_of(&mut self, part: &str) -> Arc<Vec<LandmarkPoint>> {
        let Some(root) = self.spec.root else { return Arc::default() };
        let key = self.scene_key();
        if self.spec.landmarks.as_ref().is_none_or(|(k, _)| *k != key) {
            self.spec.landmarks = Some((key, BTreeMap::new()));
        }
        let id = (root, part.to_owned());
        if let Some(l) = self.spec.landmarks.as_ref().and_then(|(_, m)| m.get(&id)) {
            return Arc::clone(l);
        }
        let l = Arc::new(p3d_spec::part_landmarks(self.doc(), root, part).unwrap_or_default());
        if let Some((_, m)) = &mut self.spec.landmarks {
            m.insert(id, Arc::clone(&l));
        }
        l
    }

    /// The landmarks the overlay shows: the anchor parent's while picking,
    /// else the selected part's.
    pub fn spec_shown_landmarks(&mut self) -> Option<(String, Arc<Vec<LandmarkPoint>>)> {
        let part = if self.spec.picking { self.spec_anchor_on()? } else { self.spec.part.clone()? };
        let l = self.spec_landmarks_of(&part);
        Some((part, l))
    }

    /// The part the selected part's anchor is on.
    pub fn spec_anchor_on(&self) -> Option<String> {
        let root = self.spec.root?;
        let part = self.spec.part.as_deref()?;
        let a = self.doc().spec(root)?.source.get("parts")?.get(part)?.get("anchor")?;
        a.get("on").and_then(Value::as_str).map(str::to_owned)
    }

    /// The faces of the blended part row under the pointer, lit in the mesh
    /// it merged into (its face groups are named `<part>.<shape>`).
    ///
    /// Complexity: O(1) cached; a rebuild is O(V + E + F) of the mesh.
    pub(crate) fn spec_highlight(&mut self) -> Option<(Arc<p3d_render::EditOverlay>, u64)> {
        use std::hash::{Hash, Hasher};
        let (root, name) = self.spec.hovered.clone()?;
        let parts = self.spec_parts();
        let k = parts.get(&root)?.iter().find(|p| p.name == name)?.blend_into?;
        let (part, mesh, _) = self.viewport.part_of(k)?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(&mesh) as usize, part, &name).hash(&mut h);
        let key = h.finish();
        if let Some((cached, o)) = &self.spec.highlight
            && *cached == key
        {
            return Some(o.clone());
        }
        let prefix = format!("{name}.");
        let mut faces = std::collections::BTreeSet::new();
        for (g, group) in mesh.groups() {
            if g.starts_with(&prefix) && group.kind() == p3d_mesh::ElementKind::Face {
                faces.extend(group.iter());
            }
        }
        let none = std::collections::BTreeSet::new();
        let sets = p3d_render::ElementSets { verts: &none, edges: &none, faces: &faces };
        let o = (Arc::new(p3d_render::edit_overlay(&mesh, part, sets, false)), key);
        self.spec.highlight = Some((key, o.clone()));
        Some(o)
    }

    /// The spec path and op of the face under `at`, when it belongs to a
    /// spec part object (asked again only when the pointer, the view or the
    /// scene changed).
    ///
    /// Complexity: O(parts x log F) for the ray plus O(F) for the face's
    /// provenance entry.
    pub fn spec_provenance_at(&mut self, at: Pos2) -> Option<Provenance> {
        use std::hash::{Hash, Hasher};
        let rect = self.viewport.rect;
        let vp = self.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.scene_key().hash(&mut h);
        for v in vp.to_cols_array() {
            v.to_bits().hash(&mut h);
        }
        let key = h.finish();
        if let Some((p, k, r)) = &self.spec.provenance
            && *p == at
            && *k == key
        {
            return r.clone();
        }
        let r = self.viewport.hit_at(at).and_then(|(k, face, _)| {
            let (part, entry) = p3d_spec::face_provenance(self.doc(), k, face)?;
            Some((k, part, entry))
        });
        self.spec.provenance = Some((at, key, r.clone()));
        r
    }

    /// Runs a `spec.*` editor command or `landmarks.toggle`.
    pub(crate) fn spec_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let s = |k: &str| params.get(k).and_then(Value::as_str);
        match id {
            "spec.starter" => {
                let which = s("name").unwrap_or("lamp");
                let source = starter(which).ok_or_else(|| format!("{} `{which}`: lamp or elf_head", t("no starter")))?;
                let name = crate::outliner::free_name(self.doc(), which);
                let r = self.execute(&[OpCall::new("spec.set", json!({ "name": name, "spec": source }))])?;
                if let Ok(k) = self.doc().resolve(&name) {
                    self.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
                    self.spec_follow();
                }
                Ok(r)
            }
            "spec.select_part" => {
                if let Some(r) = s("root") {
                    let k = self.doc().resolve(r).map_err(|e| e.to_string())?;
                    self.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
                    self.spec_follow();
                }
                self.spec_select_part(s("part"))
            }
            "spec.json" => {
                let text = match (s("text"), params.get("spec")) {
                    (Some(text), _) => text.to_owned(),
                    (None, Some(v)) => serde_json::to_string_pretty(v).map_err(|e| e.to_string())?,
                    _ => return Err(t("text or spec is required").to_owned()),
                };
                self.spec.json = text.clone();
                self.spec.json_edited = true;
                self.spec_apply_json(&text)
            }
            "spec.pick_landmark" => {
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!self.spec.picking);
                if on && self.spec_anchor_on().is_none() {
                    return Err(t("the part has no anchor: choose the part it sits on first").to_owned());
                }
                self.spec.picking = on;
                Ok(json!({ "picking": self.spec.picking }))
            }
            "spec.set_landmark" => {
                let name = s("landmark").ok_or(t("landmark is required"))?.to_owned();
                self.spec_set_landmark(&name)
            }
            "landmarks.toggle" | "view.landmarks" => {
                self.ui.view.landmarks = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.landmarks);
                Ok(json!({ "landmarks": self.ui.view.landmarks }))
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// Anchors the selected part at a landmark of the part it is on.
    pub fn spec_set_landmark(&mut self, name: &str) -> Result<Value, String> {
        let part = self.spec.part.clone().ok_or(t("select a part"))?;
        let on = self.spec_anchor_on().ok_or(t("the part has no anchor: choose the part it sits on first"))?;
        self.spec.picking = false;
        let anchor = json!({ "on": on, "landmark": name, "param": null, "ray": null, "nearest": null });
        self.spec_patch_part(json!({ "anchor": anchor }), &format!("{} {part}", t("Anchor")))
    }

    /// The editor's state for agents (`ui.inspect`): the parts, errors at
    /// their paths, the landmarks found so far, the ones on screen with
    /// their positions (overlay or picking), and the face provenance shown
    /// under the pointer.
    ///
    /// Complexity: O(spec size + cached landmarks).
    pub(crate) fn spec_state(&self) -> Value {
        let doc = self.doc();
        let id = |k: ObjectKey| doc.object(k).map(|o| o.public_id());
        let parts = self.spec.root.map(|r| {
            parts_of(doc, r)
                .into_iter()
                .map(|p| {
                    json!({ "name": p.name, "kind": p.kind, "join": p.join, "on": p.on,
                        "object": p.object.and_then(id), "blendInto": p.blend_into.and_then(id) })
                })
                .collect::<Vec<_>>()
        });
        let cached = |part: &str| self.spec.landmarks.as_ref().and_then(|(_, m)| m.get(&(self.spec.root?, part.to_owned())));
        let landmarks = self.spec.landmarks.as_ref().map(|(_, m)| {
            m.iter()
                .filter(|((r, _), _)| Some(*r) == self.spec.root)
                .map(|((_, p), l)| (p.clone(), json!(l.iter().map(|x| x.name.clone()).collect::<Vec<_>>())))
                .collect::<serde_json::Map<_, _>>()
        });
        // The landmarks drawn in the viewport, where they are on screen.
        let rect = self.viewport.rect;
        let shown_part = if self.spec.picking { self.spec_anchor_on() } else { self.spec.part.clone().filter(|_| self.ui.view.landmarks) };
        let shown = shown_part.as_deref().and_then(cached).filter(|_| rect.is_positive()).map(|l| {
            let vp = self.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
            l.iter().filter_map(|x| project(vp, rect, x.point).map(|at| json!({ "name": x.name, "at": [at.x, at.y] }))).collect::<Vec<_>>()
        });
        let provenance = self
            .spec
            .provenance
            .as_ref()
            .filter(|_| self.spec.prov_shown)
            .and_then(|(_, _, r)| r.as_ref())
            .map(|(k, part, e)| json!({ "object": id(*k), "part": part, "path": e.path, "op": e.op }));
        let issue = |i: &SpecIssue| json!({ "path": i.path, "message": i.message, "line": i.line });
        json!({
            "root": self.spec.root.and_then(id),
            "part": self.spec.part,
            "parts": parts,
            "busy": self.spec.busy(),
            "error": self.spec.error.as_ref().map(issue),
            "jsonError": self.spec.json_error.as_ref().map(issue),
            "jsonEdited": self.spec.json_edited,
            "picking": self.spec.picking,
            "landmarksShown": self.ui.view.landmarks,
            "landmarks": landmarks,
            "onScreen": shown_part.map(|p| json!({ "part": p, "landmarks": shown })),
            "provenance": provenance,
        })
    }
}

/// Label rectangles already placed on screen, bucketed in square cells so
/// a new label is only checked against its neighbours (placed labels do
/// not overlap, so a cell holds a few at most).
#[derive(Default)]
struct Placed(BTreeMap<(i32, i32), Vec<egui::Rect>>);

impl Placed {
    const CELL: f32 = 64.0;

    /// Complexity: O(cells the rectangle spans).
    fn cells(r: egui::Rect) -> impl Iterator<Item = (i32, i32)> {
        let c = |v: f32| (v / Self::CELL).floor() as i32;
        let (x0, x1, y0, y1) = (c(r.left()), c(r.right()), c(r.top()), c(r.bottom()));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    /// Complexity: O(labels in the cells the rectangle spans).
    fn free(&self, r: egui::Rect) -> bool {
        Self::cells(r).all(|c| self.0.get(&c).is_none_or(|v| v.iter().all(|o| !o.intersects(r))))
    }

    /// Complexity: O(cells the rectangle spans x log cells).
    fn add(&mut self, r: egui::Rect) {
        for c in Self::cells(r) {
            self.0.entry(c).or_default().push(r);
        }
    }
}

/// The landmark overlay (View > Overlays > Landmarks, or while picking an
/// anchor): points with short ticks along their normals, labelled where a
/// label fits (the one under the pointer always). A click on one while
/// picking anchors the selected part there (and selects no object).
///
/// Complexity: O(landmarks) per frame; the landmarks are found once per
/// document state.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui, events: &mut Vec<ViewportEvent>) {
    let rect = app.viewport.rect;
    if app.ui.mode.is_edit() || !rect.is_positive() || !(app.ui.view.landmarks || app.spec.picking) {
        return;
    }
    let Some((_, points)) = app.spec_shown_landmarks() else { return };
    let tk = Tokens::get();
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let p = ui.painter_at(rect);
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|q| rect.contains(*q));
    let color = if app.spec.picking { tk.accent } else { tk.axis_y };
    let screen: Vec<(Pos2, &LandmarkPoint)> = points.iter().filter_map(|l| Some((project(vp, rect, l.point)?, l))).collect();
    // The landmark under the pointer, if any.
    let hot = pointer.and_then(|q| {
        screen.iter().filter(|(s, _)| s.distance(q) <= 9.0).min_by(|a, b| a.0.distance(q).total_cmp(&b.0.distance(q))).map(|(s, _)| *s)
    });
    for (at, l) in &screen {
        if let Some(q) = project(vp, rect, l.point + l.normal * 0.01)
            && q.distance(*at) > 0.5
        {
            p.line_segment([*at, *at + (q - *at).normalized() * 16.0], Stroke::new(1.5, color));
        }
        let r = if Some(*at) == hot { 5.5 } else { 4.0 };
        p.circle(*at, r, color, Stroke::new(1.0, egui::Color32::from_black_alpha(170)));
    }
    // Labels: the hot one first, then each where it fits beside its point.
    let mut placed = Placed::default();
    let mut order: Vec<&(Pos2, &LandmarkPoint)> = screen.iter().collect();
    order.sort_by_key(|(s, _)| Some(*s) != hot);
    for (at, l) in order {
        let g = p.layout_no_wrap(l.name.clone(), theme::small(), tk.text);
        let size = g.size() + vec2(8.0, 4.0);
        let spots = [vec2(7.0, -size.y - 3.0), vec2(7.0, 3.0), vec2(-size.x - 7.0, -size.y - 3.0), vec2(-size.x - 7.0, 3.0)];
        let is_hot = Some(*at) == hot;
        let Some(label) = spots.iter().map(|d| egui::Rect::from_min_size(*at + *d, size)).find(|r| is_hot || placed.free(*r)) else { continue };
        placed.add(label);
        p.rect_filled(label, 4.0, tk.popup.gamma_multiply(if is_hot { 1.0 } else { 0.85 }));
        p.galley(label.min + vec2(4.0, 2.0), g, tk.text);
    }
    if !app.spec.picking {
        return;
    }
    let click = ui.input(|i| i.pointer.primary_clicked().then(|| i.pointer.interact_pos()).flatten());
    if let Some(at) = click.filter(|at| rect.contains(*at))
        && let Some((_, l)) = screen.iter().filter(|(s, _)| s.distance(at) <= 9.0).min_by(|a, b| a.0.distance(at).total_cmp(&b.0.distance(at)))
    {
        let name = l.name.clone();
        app.viewport.cancel_pick();
        events.retain(|e| !matches!(e, ViewportEvent::Picked { .. } | ViewportEvent::MissedAt { .. }));
        if let Err(e) = app.spec_set_landmark(&name) {
            app.set_status(e);
        }
    }
}

/// The spec path and op of the face under a still pointer, as a small
/// label beside it (Object Mode, over a spec part object); the status bar
/// repeats it.
///
/// Complexity: O(1) while the pointer moves; see `spec_provenance_at` once
/// it rests.
pub fn provenance_tip(app: &mut PixelApp, ui: &egui::Ui) {
    app.spec.prov_shown = false;
    let rect = app.viewport.rect;
    if app.ui.mode.is_edit() || app.gizmo.over_handle() || !rect.is_positive() || app.spec_parts().is_empty() {
        return;
    }
    let Some(at) = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)) else { return };
    let (down, still) = ui.input(|i| (i.pointer.any_down(), i.pointer.time_since_last_movement()));
    if down {
        return;
    }
    // Ask once the pointer rests, not on every move.
    const REST: f32 = 0.15;
    if still < REST {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f32(REST - still + 0.01));
        return;
    }
    let Some((_, part, entry)) = app.spec_provenance_at(at) else { return };
    app.spec.prov_shown = true;
    let tk = Tokens::get();
    let p = ui.painter_at(rect);
    let title = p.layout_no_wrap(part, theme::small(), tk.text);
    let line = p.layout_no_wrap(format!("{}  {}", entry.path, entry.op), theme::small(), tk.text_dim);
    let size = vec2(title.size().x.max(line.size().x), title.size().y + line.size().y + 2.0) + vec2(12.0, 8.0);
    let mut r = egui::Rect::from_min_size(at + vec2(16.0, 16.0), size);
    if r.right() > rect.right() {
        r = r.translate(vec2(-(size.x + 24.0), 0.0));
    }
    if r.bottom() > rect.bottom() {
        r = r.translate(vec2(0.0, -(size.y + 24.0)));
    }
    p.rect(r, 5.0, tk.popup, Stroke::new(1.0, tk.popup_border), egui::StrokeKind::Inside);
    let y2 = r.top() + 4.0 + title.size().y + 2.0;
    p.galley(pos2(r.left() + 6.0, r.top() + 4.0), title, tk.text);
    p.galley(pos2(r.left() + 6.0, y2), line, tk.text_dim);
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

#[cfg(test)]
mod tests;
