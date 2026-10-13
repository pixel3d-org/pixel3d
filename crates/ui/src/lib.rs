//! Desktop user interface built on egui.
//!
//! Layer L5 (frontends). See `docs/architecture.md`.
//!
//! A thin shell over [`p3d_engine::Session`] built with cadcraft's
//! techniques (project spec 20): panels read the session and change things
//! only through ops and `ui.*` commands ([`PixelApp::run`]), so undo, MCP
//! and the control server see the same actions. Nothing here owns document
//! data.

pub mod actions;
pub mod chrome;
pub mod cmdline;
pub mod commands;
pub mod control;
pub mod curves;
pub mod docking;
pub mod elements;
pub mod files;
pub mod gizmo;
pub mod health;
pub mod i18n;
mod icon_data;
pub mod icons;
pub mod inspector;
pub mod jobcard;
pub mod menus;
pub mod navgizmo;
pub mod outline;
pub mod outliner;
pub mod palette;
pub mod panels;
pub mod poles;
pub mod quick;
pub mod remote;
pub mod results;
pub mod schema_form;
pub mod screenshot;
pub mod sdf;
pub mod snap;
pub mod spec;
pub mod stats;
pub mod theme;
pub mod transform;
pub mod transform_bar;
pub mod vector_icon;
pub mod viewport;
pub mod widgets;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use p3d_engine::{BatchOptions, OpCall, Session};
use p3d_render::ViewKind;
use p3d_scene::{Document, ObjectKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::viewport::{ViewSettings, ViewportEvent, ViewportState};

/// The host storage key for [`PixelApp::prefs_json`].
pub const PREFS_KEY: &str = "pixel3d.prefs";
/// Console lines kept.
const CONSOLE_LINES: usize = 500;
/// How long status messages stay.
const STATUS_TIME: Duration = Duration::from_secs(6);

/// The active transform tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    #[default]
    Select,
    Move,
    Rotate,
    Scale,
    /// Edit Mode: drag a line to cut the faces it crosses.
    Knife,
}

/// Whole objects or mesh elements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectMode {
    #[default]
    Object,
    Vert,
    Edge,
    Face,
}

impl SelectMode {
    /// The icon of the mode.
    pub fn icon(self) -> icons::Icon {
        match self {
            SelectMode::Object => icons::Icon::ModeObject,
            SelectMode::Vert => icons::Icon::ModeVert,
            SelectMode::Edge => icons::Icon::ModeEdge,
            SelectMode::Face => icons::Icon::ModeFace,
        }
    }

    /// Edit mode: vertices, edges or faces of the active mesh.
    pub fn is_edit(self) -> bool {
        self != SelectMode::Object
    }

    pub fn label(self) -> &'static str {
        match self {
            SelectMode::Object => t("Object"),
            SelectMode::Vert => t("Vertex"),
            SelectMode::Edge => t("Edge"),
            SelectMode::Face => t("Face"),
        }
    }
}

/// Workspaces: a dock layout, tool set and overlays per kind of work
/// (project spec 20.4). One is enabled once its editors exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceKind {
    #[default]
    Model,
    Uv,
    Bake,
    Rig,
    Animate,
    Export,
}

impl WorkspaceKind {
    pub const ALL: [WorkspaceKind; 6] =
        [WorkspaceKind::Model, WorkspaceKind::Uv, WorkspaceKind::Bake, WorkspaceKind::Rig, WorkspaceKind::Animate, WorkspaceKind::Export];

    pub fn label(self) -> &'static str {
        match self {
            WorkspaceKind::Model => t("Model"),
            WorkspaceKind::Uv => t("UV"),
            WorkspaceKind::Bake => t("Bake"),
            WorkspaceKind::Rig => t("Rig"),
            WorkspaceKind::Animate => t("Animate"),
            WorkspaceKind::Export => t("Export"),
        }
    }

    /// What the workspace is for (its tab's tooltip).
    pub fn describe(self) -> &'static str {
        match self {
            WorkspaceKind::Model => t("Modelling: the viewport, Outliner, Properties and Health"),
            WorkspaceKind::Uv => t("UVs: the viewport beside the UV editor"),
            WorkspaceKind::Bake => t("Baking: high and low poly pairs and their maps"),
            WorkspaceKind::Rig => t("Rigging: a tall Outliner for the bone hierarchy"),
            WorkspaceKind::Animate => t("Animation: the timeline under the viewport"),
            WorkspaceKind::Export => t("Export: budget and health checks before export"),
        }
    }
}

/// Persisted UI state (serde, camelCase, so automation can read and set it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub docking: docking::Workspace,
    /// The layouts of the other workspaces, restored when switching back.
    pub layouts: std::collections::BTreeMap<WorkspaceKind, docking::Workspace>,
    pub theme: theme::ThemePref,
    pub view: ViewSettings,
    /// The tool settings row under the top bar.
    pub show_toolbar: bool,
    pub show_status_bar: bool,
    /// The tool rail shows small labels under its icons.
    pub rail_labels: bool,
    /// The open Properties tab.
    pub inspector_tab: inspector::Tab,
    /// The workspace shown (tabs in the top bar).
    pub workspace: WorkspaceKind,
    pub start_page: bool,
    pub tool: Tool,
    pub mode: SelectMode,
    /// The element mode Tab returns to.
    pub element_mode: SelectMode,
    /// Orientation, pivot, snapping, proportional editing, 3D cursor.
    pub transform: transform::TransformSettings,
    /// Recently opened or saved files, newest first.
    pub recent: Vec<String>,
}

/// Recent files kept.
const RECENT: usize = 10;

impl Default for UiState {
    fn default() -> Self {
        UiState {
            docking: docking::Workspace::default(),
            layouts: std::collections::BTreeMap::new(),
            theme: theme::ThemePref::default(),
            view: ViewSettings::default(),
            show_toolbar: true,
            show_status_bar: true,
            rail_labels: true,
            inspector_tab: inspector::Tab::default(),
            workspace: WorkspaceKind::Model,
            start_page: true,
            tool: Tool::Select,
            mode: SelectMode::Object,
            element_mode: SelectMode::Vert,
            transform: transform::TransformSettings::default(),
            recent: Vec::new(),
        }
    }
}

impl UiState {
    /// The icon of the current mode.
    pub fn mode_icon(&self) -> icons::Icon {
        self.mode.icon()
    }

    /// Puts `path` first in the recent files.
    ///
    /// Complexity: O(RECENT).
    pub fn remember(&mut self, path: &std::path::Path) {
        let s = path.display().to_string();
        self.recent.retain(|r| *r != s);
        self.recent.insert(0, s);
        self.recent.truncate(RECENT);
    }
}

/// Asks the user for a file to open.
pub type PickOpen = Box<dyn Fn() -> Option<PathBuf>>;
/// Asks the user where to save, given a suggested file name.
pub type PickSave = Box<dyn Fn(&str) -> Option<PathBuf>>;

/// Host services injected by the app (file dialogs), so this crate stays
/// free of platform code.
#[derive(Default)]
pub struct Services {
    pub pick_open: Option<PickOpen>,
    pub pick_save: Option<PickSave>,
    /// Asks for a model file to import (glTF, GLB, OBJ, FBX).
    pub pick_import: Option<PickOpen>,
}

/// Extensions `io.import` reads.
pub const MODEL_EXTENSIONS: [&str; 4] = ["glb", "gltf", "obj", "fbx"];

/// Whether `path` names a model file `io.import` reads.
pub fn is_model_file(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| MODEL_EXTENSIONS.iter().any(|m| e.eq_ignore_ascii_case(m)))
}

/// An op batch running in the background (imports) and its undo label.
struct RunningJob {
    job: p3d_engine::Job,
    label: String,
}

/// The last single-op transaction: Properties shows its parameters and
/// re-runs it with new ones (Blender's "Adjust Last Operation").
#[derive(Debug, Clone, PartialEq)]
pub struct LastOp {
    pub op: String,
    /// Canonical parameters (defaults filled in, units converted).
    pub params: Value,
    /// The revision the op produced; adjusting needs it to be current.
    pub revision: u64,
    /// Why the last adjustment was refused.
    pub error: Option<String>,
}

/// The desktop application state.
pub struct PixelApp {
    pub session: Session,
    pub ui: UiState,
    pub viewport: ViewportState,
    /// Selected objects (UI state, not document data).
    pub selection: BTreeSet<ObjectKey>,
    /// The object clicked last (Properties shows it).
    pub active: Option<ObjectKey>,
    /// Objects hidden in the viewport only (UI state, like Blender's eye).
    pub hidden: BTreeSet<ObjectKey>,
    /// The last single-op transaction, adjustable in Properties.
    pub last_op: Option<LastOp>,
    /// What the last transaction reported (summaries, warnings and fixes).
    pub result: Option<results::LastResult>,
    /// Background op batches (imports), committed when they finish.
    jobs: Vec<RunningJob>,
    /// The SDF shape editor (the Shapes tab and the viewport proxies).
    pub sdf: sdf::SdfEditor,
    /// The Model Spec editor (the Spec tab, landmarks and provenance).
    pub spec: spec::SpecEditor,
    /// The outline editor (the Outline tab: shells and silhouettes).
    pub outline: outline::OutlineEditor,
    /// The run card of a long op (remesh) and its background run.
    pub job_card: jobcard::JobCard,
    /// Poles of the shown meshes, for the pole overlay.
    pub poles: poles::PoleCache,
    /// The revision of an edit still being dragged: the next change of the
    /// same drag replaces it, so one drag is one undo step.
    coalesce: Option<u64>,
    /// A group to select when Edit Mode starts on its object (the faces a
    /// boolean cut), while the document is still at that revision.
    pub edit_focus: Option<(u64, ObjectKey, String)>,
    pub services: Services,
    /// The file the document was opened from or saved to.
    pub path: Option<PathBuf>,
    /// Folders file ops may use: the project's and those the user picked
    /// files from (the session's file access follows them).
    pub files: files::FileRoots,
    saved_revision: u64,
    /// Bumped on every document load so scene caches never confuse two files.
    generation: u64,
    pub status: Option<(String, Instant)>,
    pub console: Vec<String>,
    /// macOS: the content extends under the title bar (traffic lights).
    pub integrated_titlebar: bool,
    shown_theme: Option<(egui::Theme, bool)>,
    pub quit_requested: bool,
    pub frame_ms: f32,
    pub shots: screenshot::Shots,
    /// Diagnostics of the current document (the Health panel).
    pub health: health::HealthState,
    /// Scene statistics for the overlay and the status bar.
    pub stats: stats::StatsCache,
    /// Curve objects sampled for drawing and picking.
    pub curves: curves::CurveCache,
    /// Selected vertices, edges and faces of the mesh being edited.
    pub edit: elements::EditState,
    /// The control server and the input and screenshots it asked for.
    pub remote: remote::Remote,
    /// The hovered contextual action's preview, and the actions for the
    /// current selection (generated from the registry, cached).
    pub action_preview: actions::ActionPreview,
    actions: Option<(u64, std::sync::Arc<Vec<actions::Action>>)>,
    /// The transform gizmo: its drag and the Edit Mode preview.
    pub gizmo: gizmo::GizmoState,
    /// The command palette while open.
    pub palette: Option<palette::Palette>,
    /// The Console's command line.
    pub cmdline: cmdline::CmdLine,
    /// Quit once the pending screenshot is saved (`--screenshot`).
    pub quit_after_shot: bool,
    /// A running `view.benchmark`: start, end and frames drawn so far; and
    /// the last result.
    pub bench: Option<(std::time::Instant, std::time::Instant, u32)>,
    pub bench_result: Option<Value>,
    /// Snapshot tests: no frame rate and no timed status messages, so the
    /// same state always draws the same picture.
    pub stable: bool,
}

impl PixelApp {
    pub fn new(session: Session, services: Services) -> Self {
        PixelApp {
            saved_revision: session.revision(),
            session,
            ui: UiState::default(),
            viewport: ViewportState::default(),
            selection: BTreeSet::new(),
            active: None,
            hidden: BTreeSet::new(),
            last_op: None,
            result: None,
            jobs: Vec::new(),
            sdf: sdf::SdfEditor::default(),
            spec: spec::SpecEditor::default(),
            outline: outline::OutlineEditor::default(),
            job_card: jobcard::JobCard::default(),
            poles: poles::PoleCache::default(),
            coalesce: None,
            edit_focus: None,
            services,
            path: None,
            files: files::FileRoots::default(),
            generation: 0,
            status: None,
            console: Vec::new(),
            integrated_titlebar: false,
            shown_theme: None,
            quit_requested: false,
            frame_ms: 0.0,
            shots: screenshot::Shots::default(),
            health: health::HealthState::default(),
            stats: stats::StatsCache::default(),
            curves: curves::CurveCache::default(),
            edit: elements::EditState::default(),
            remote: remote::Remote::default(),
            action_preview: actions::ActionPreview::default(),
            actions: None,
            gizmo: gizmo::GizmoState::default(),
            palette: None,
            cmdline: cmdline::CmdLine::default(),
            quit_after_shot: false,
            stable: false,
            bench: None,
            bench_result: None,
        }
    }

    /// Uses eframe's wgpu device for the viewport. Without it the viewport
    /// shows a notice instead of 3D.
    pub fn set_wgpu(&mut self, rs: &egui_wgpu::RenderState) {
        self.viewport.set_picks(viewport::install(rs));
    }

    pub fn doc(&self) -> &Document {
        self.session.doc()
    }

    /// Background work still running (shown in the status bar).
    pub fn running_jobs(&self) -> usize {
        usize::from(self.health.busy())
            + self.jobs.len()
            + usize::from(self.sdf.busy())
            + usize::from(self.spec.busy())
            + usize::from(self.outline.busy())
            + usize::from(self.job_card.busy())
    }

    /// Whether the History or Console panel is open.
    /// Whether an element op can run on the edit selection now: `Ok` with
    /// the `select` parameter to pass (`None` for the whole mesh), or why
    /// not. Ops get the picked elements as `{"faces": {"ids": [...]}}`
    /// (likewise edges and vertices), valid at the current revision.
    ///
    /// Complexity: O(selected) plus the op's schema size.
    pub fn element_op_state(&self, op: &str) -> Result<Option<Value>, String> {
        let registry = self.session.registry();
        let Ok(o) = registry.get(op) else {
            return Err(format!("{} `{op}`, {}", t("Needs"), t("which is not built yet.")));
        };
        if !self.ui.mode.is_edit() || self.edit.object.is_none() {
            return Err(t("Works in Edit Mode (Tab) on the active mesh.").to_owned());
        }
        let schema = o.params_schema();
        let Some(_) = schema.pointer("/properties/select") else { return Ok(None) };
        // As in Blender, edit tools work on a selection, even where the op
        // would take the whole mesh.
        let (picked, mode) = self.edit.picked();
        if picked.is_empty() || mode != Some(self.ui.mode) {
            return Err(t("Select vertices, edges or faces first.").to_owned());
        }
        let by_id = has_property(&schema, "ids");
        if !by_id {
            return Err(t("Waits for ops to take picked elements (selectors by element id).").to_owned());
        }
        let kind = match self.ui.mode {
            SelectMode::Vert => "verts",
            SelectMode::Edge => "edges",
            _ => "faces",
        };
        Ok(Some(json!({ kind: { "ids": picked } })))
    }

    /// Runs an element op on the edited mesh and its selected elements.
    pub fn run_element_op(&mut self, op: &str) -> Result<Value, String> {
        let select = self.element_op_state(op)?;
        let Some(id) = self.edit.object.and_then(|k| self.doc().object(k)).map(|o| o.public_id()) else {
            return Err(t("nothing to edit").to_owned());
        };
        let mut params = json!({ "target": id });
        if let Some(s) = select {
            params["select"] = s;
        }
        self.run_element_call(op, params)
    }

    /// Runs one element op call on the edited mesh, then selects what it
    /// made, as Blender does: the extruded caps, the inset faces, the split
    /// faces or the cut edges (from the groups the op created).
    fn run_element_call(&mut self, op: &str, params: Value) -> Result<Value, String> {
        let Some(k) = self.edit.object else { return Err(t("nothing to edit").to_owned()) };
        let before: BTreeSet<String> = self.edit.mesh().map(|m| m.groups().map(|(n, _)| n.to_owned()).collect()).unwrap_or_default();
        let r = self.execute(&[OpCall::new(op, params)]);
        match &r {
            Ok(_) => self.reselect_created(k, &before),
            Err(e) => self.set_status(e.clone()),
        }
        r
    }

    /// Selects the elements of the first new group named `*.cap`,
    /// `*.inner`, `*.faces` or `*.edges`, converted to the current mode.
    ///
    /// Complexity: O(groups + group size) plus a conversion as
    /// [`elements::flush`].
    fn reselect_created(&mut self, k: ObjectKey, before: &BTreeSet<String>) {
        let doc = std::sync::Arc::clone(self.session.doc());
        let Some(mesh) = doc.object(k).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m)).cloned() else { return };
        let mode = self.ui.mode;
        self.edit.sync(Some(k), Some(&mesh), mode);
        let created: Vec<(&str, &p3d_mesh::Group)> = mesh.groups().filter(|(n, _)| !before.contains(*n)).collect();
        for suffix in [".cap", ".inner", ".faces", ".edges"] {
            if let Some((_, g)) = created.iter().find(|(n, _)| n.ends_with(suffix)) {
                let kind = match g.kind() {
                    p3d_mesh::ElementKind::Face => SelectMode::Face,
                    p3d_mesh::ElementKind::Edge => SelectMode::Edge,
                    p3d_mesh::ElementKind::Vert => SelectMode::Vert,
                };
                self.edit.apply_box(g.iter().collect(), kind, elements::BoxOp::Set);
                self.edit.sync(Some(k), Some(&mesh), mode);
                return;
            }
        }
    }

    /// The knife: cuts the faces of the edited mesh that the screen line
    /// `a`-`b` (points) crosses, along the plane through the line and the
    /// view direction (`edit.plane_cut` on those faces). Faces turned away
    /// are left alone unless x-ray is on.
    ///
    /// Complexity: O(face corners) to find the crossed faces, plus the op.
    pub fn knife_cut(&mut self, a: egui::Pos2, b: egui::Pos2) -> Result<Value, String> {
        let k = self.edit.object.ok_or(t("nothing to edit"))?;
        let (_, mesh, model) = self.viewport.part_of(k).ok_or(t("nothing to edit"))?;
        let id = self.doc().object(k).map(|o| o.public_id()).unwrap_or_default();
        let (cam, r) = (self.viewport.camera, self.viewport.rect);
        let (oa, da) = cam.ray(a.x - r.left(), a.y - r.top(), r.width(), r.height());
        let (ob, db) = cam.ray(b.x - r.left(), b.y - r.top(), r.width(), r.height());
        let normal_w = ((ob + db) - (oa + da)).cross(da).normalize_or_zero();
        if normal_w == glam::Vec3::ZERO {
            return Err(t("draw a longer line to cut").to_owned());
        }
        let inv = model.inverse();
        let point = inv.transform_point3(oa + da);
        let normal = model.transpose().transform_vector3(normal_w).normalize_or_zero();
        let faces = elements::faces_crossed(&mesh, model, &cam, r, a, b, self.ui.view.xray);
        if faces.is_empty() {
            return Err(t("the line crosses no face").to_owned());
        }
        let params = json!({
            "target": id,
            "select": { "faces": { "ids": faces } },
            "point": point.to_array(),
            "normal": normal.to_array(),
        });
        self.run_element_call("edit.plane_cut", params)
    }

    /// A box selection: elements of the edited mesh in edit mode (hidden
    /// ones only in x-ray), objects in Object mode.
    ///
    /// Complexity: as [`elements::box_pick`] or [`viewport::ViewportState::objects_in`].
    pub fn box_select(&mut self, area: egui::Rect, op: elements::BoxOp) {
        use elements::BoxOp;
        if self.ui.mode.is_edit() {
            let Some(k) = self.edit.object else { return };
            let (Some((_, mesh, model)), Some(bvh)) = (self.viewport.part_of(k), self.edit.bvh()) else { return };
            let ids = elements::box_pick(&mesh, &bvh, model, &self.viewport.camera, self.viewport.rect, area, self.ui.mode, self.ui.view.xray);
            self.edit.apply_box(ids, self.ui.mode, op);
            return;
        }
        let hits = self.viewport.objects_in(area);
        match op {
            BoxOp::Set => self.selection = hits,
            BoxOp::Add => self.selection.extend(hits),
            BoxOp::Sub => self.selection.retain(|k| !hits.contains(k)),
        }
        if self.active.is_none_or(|a| !self.selection.contains(&a)) {
            self.active = self.selection.first().copied();
        }
    }

    /// Moves the 3D cursor: to `position` ([x, y, z] meters), or `to` the
    /// world origin, the selection's center (objects, or the selected
    /// elements in Edit Mode) or the active object.
    ///
    /// Complexity: O(selected objects), or O(selected vertices) in Edit Mode.
    pub fn place_cursor(&mut self, params: &Value) -> Result<Value, String> {
        let doc = std::sync::Arc::clone(self.session.doc());
        let at: glam::Vec3 = if let Some(p) = params.get("position") {
            glam::Vec3::from_array(serde_json::from_value::<[f32; 3]>(p.clone()).map_err(|e| format!("position: {e}"))?)
        } else {
            match params.get("to").and_then(Value::as_str) {
                Some("origin") | None => glam::Vec3::ZERO,
                Some("active") => {
                    let k = self.active_object().ok_or(t("nothing is active"))?;
                    glam::Vec3::from(doc.world_affine(k).map(|a| a.translation).unwrap_or_default())
                }
                Some("selected") => self.selection_center(&doc).ok_or(t("nothing selected"))?,
                Some(other) => return Err(format!("{} `{other}`: origin, selected or active", t("unknown cursor target"))),
            }
        };
        self.ui.transform.cursor = at.to_array();
        if let Some(r) = params.get("rotation") {
            self.ui.transform.cursor_rotation = serde_json::from_value(r.clone()).map_err(|e| format!("rotation: {e}"))?;
        }
        Ok(json!({ "cursor": self.ui.transform.cursor, "rotation": self.ui.transform.cursor_rotation }))
    }

    /// The center of the selection in world space: the selected elements'
    /// vertices in Edit Mode, else the selected objects' bounds.
    ///
    /// Complexity: O(selected objects) or O(selected vertices).
    pub fn selection_center(&self, doc: &p3d_scene::Document) -> Option<glam::Vec3> {
        if self.ui.mode.is_edit()
            && let (Some(k), Some(m), Some(f)) = (self.edit.object, self.edit.mesh(), self.edit.flushed())
            && !f.verts.is_empty()
        {
            let model = glam::Mat4::from(doc.world_affine(k)?);
            let sum: glam::Vec3 = f.verts.iter().filter_map(|&v| m.positions.get(v as usize)).map(|p| model.transform_point3(*p)).sum();
            return Some(sum / f.verts.len() as f32);
        }
        let (mut lo, mut hi) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
        for &k in &self.selection {
            let model = glam::Mat4::from(doc.world_affine(k)?);
            let (a, b) = match doc.object_mesh(k).and_then(|m| m.bounds()) {
                Some((a, b)) => (model.transform_point3(a), model.transform_point3(b)),
                None => (model.transform_point3(glam::Vec3::ZERO), model.transform_point3(glam::Vec3::ZERO)),
            };
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        }
        lo.cmple(hi).all().then(|| (lo + hi) * 0.5)
    }

    /// Switches between Object mode and the element modes. Edit mode needs
    /// an active mesh object.
    pub fn set_mode(&mut self, mode: SelectMode) -> Result<Value, String> {
        let was_edit = self.ui.mode.is_edit();
        if mode.is_edit() {
            let mesh = self.active_object().is_some_and(|k| self.doc().object_mesh(k).is_some());
            if !mesh && !was_edit {
                return Err(t("Select a mesh object to edit it").to_owned());
            }
            self.ui.element_mode = mode;
        }
        self.ui.mode = mode;
        if !mode.is_edit() && self.ui.tool == Tool::Knife {
            self.ui.tool = Tool::Select;
        }
        // Follow the edited mesh now, so commands right after see it.
        if mode.is_edit() {
            let doc = std::sync::Arc::clone(self.session.doc());
            let editing = self.active_object().or(self.edit.object);
            let mesh = editing.and_then(|k| doc.object(k)?.mesh_key()).and_then(|m| doc.mesh_arc(m));
            self.edit.sync(mesh.and(editing), mesh, mode);
            // Right after a boolean, Edit Mode starts with the faces it cut.
            if !was_edit
                && let Some((revision, k, name)) = self.edit_focus.take()
                && revision == self.session.revision()
                && editing == Some(k)
            {
                return self.select_group(k, &name);
            }
        }
        Ok(json!({ "mode": mode }))
    }

    /// Edits object `k` with its group `name` selected, in the group's
    /// element mode (faces, edges or vertices).
    ///
    /// Complexity: O(g log g) for a group of g elements, plus a mode change.
    pub fn select_group(&mut self, k: ObjectKey, name: &str) -> Result<Value, String> {
        let doc = std::sync::Arc::clone(self.session.doc());
        let mesh = doc.object(k).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m)).cloned().ok_or(t("not a mesh object"))?;
        let g = mesh.group(name).ok_or_else(|| format!("{} `{name}`", t("no group")))?;
        let mode = match g.kind() {
            p3d_mesh::ElementKind::Face => SelectMode::Face,
            p3d_mesh::ElementKind::Edge => SelectMode::Edge,
            p3d_mesh::ElementKind::Vert => SelectMode::Vert,
        };
        let ids: Vec<u32> = g.iter().collect();
        let count = ids.len();
        self.selection = [k].into_iter().collect();
        self.active = Some(k);
        self.ui.mode = mode;
        self.ui.element_mode = mode;
        if self.ui.tool == Tool::Knife {
            self.ui.tool = Tool::Select;
        }
        self.edit.sync(Some(k), Some(&mesh), mode);
        self.edit.apply_box(ids, mode, elements::BoxOp::Set);
        self.set_status(format!("{} {name}: {count} {}", t("Selected"), t("elements")));
        Ok(json!({ "mode": mode, "group": name, "elements": count }))
    }

    /// The target (the active mesh) and the tools (the other selected
    /// meshes) of an op that combines objects, such as a boolean, or why
    /// the selection does not give them.
    ///
    /// Complexity: O(selected objects).
    pub fn tools_on_active(&self) -> Result<(ObjectKey, Vec<ObjectKey>), String> {
        if self.ui.mode.is_edit() {
            return Err(t("Works on whole objects: switch to Object Mode (Tab).").to_owned());
        }
        let doc = self.doc();
        let is_mesh = |k: &ObjectKey| doc.object(*k).is_some_and(|o| o.mesh_key().is_some());
        if self.selection.iter().filter(|k| is_mesh(k)).count() < 2 {
            return Err(t("Select two or more meshes: the active one (selected last) keeps the result, the others act on it.").to_owned());
        }
        let target = self.active_object().filter(is_mesh).ok_or(t("Make a mesh active (click it last): it keeps the result."))?;
        let tools = self.selection.iter().copied().filter(|k| *k != target && is_mesh(k)).collect();
        Ok((target, tools))
    }

    /// The call of an op that takes the active mesh as its `target` and the
    /// other selected meshes as its `tool`, with `preset` parameters.
    ///
    /// Complexity: O(selected objects).
    pub fn tools_on_active_call(&self, op: &str, preset: &Value) -> Result<(ObjectKey, OpCall), String> {
        let (target, tools) = self.tools_on_active()?;
        let doc = self.doc();
        let id = |k: ObjectKey| doc.object(k).map(|o| o.public_id());
        let mut args = preset.as_object().cloned().unwrap_or_default();
        args.insert("target".into(), json!(id(target)));
        args.insert("tool".into(), json!(tools.into_iter().filter_map(id).collect::<Vec<_>>()));
        Ok((target, OpCall::new(op, Value::Object(args))))
    }

    /// Runs an op on the active mesh with the other selected meshes as its
    /// tools, as Blender's booleans do (one undo step). The target stays
    /// selected; entering Edit Mode next selects the faces a cut made.
    pub fn run_tools_on_active(&mut self, op: &str, preset: &Value) -> Result<Value, String> {
        let (target, call) = self.tools_on_active_call(op, preset)?;
        let r = self.execute(&[call])?;
        self.selection = [target].into_iter().collect();
        self.active = Some(target);
        Ok(r)
    }

    /// The contextual actions for the current selection and mode, generated
    /// again only when the document, the selection or the mode changed.
    ///
    /// Complexity: O(selected) to check; a rebuild as [`actions::for_context`].
    pub fn contextual_actions(&mut self) -> std::sync::Arc<Vec<actions::Action>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.scene_key().hash(&mut h);
        self.selection.hash(&mut h);
        (self.ui.mode as u8).hash(&mut h);
        self.edit.version.hash(&mut h);
        let key = h.finish();
        if let Some((k, a)) = &self.actions
            && *k == key
        {
            return std::sync::Arc::clone(a);
        }
        let a = std::sync::Arc::new(actions::for_context(self));
        self.actions = Some((key, std::sync::Arc::clone(&a)));
        a
    }

    /// Object and element counts, selected and visible.
    ///
    /// Complexity: O(selected + hidden) per call; a recount (O(faces)) only
    /// after an edit or a selection change.
    pub fn stats(&mut self) -> stats::Stats {
        let key = self.scene_key();
        let doc = std::sync::Arc::clone(self.session.doc());
        self.stats.get(key, &doc, &self.selection, &self.hidden)
    }

    pub fn bottom_panel_visible(&self) -> bool {
        self.ui.docking.is_visible(docking::Panel::History) || self.ui.docking.is_visible(docking::Panel::Console)
    }

    /// Whether the document changed since it was opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.session.revision() != self.saved_revision
    }

    /// The scene cache key: the load generation and the document revision.
    pub fn scene_key(&self) -> u64 {
        (self.generation << 40) ^ self.session.revision()
    }

    pub fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| t("Untitled").to_owned(), |n| n.to_string_lossy().into_owned());
        format!("{name}{}", if self.is_dirty() { " *" } else { "" })
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.log(s.clone());
        self.status = Some((s, Instant::now()));
    }

    /// The status message while it is fresh.
    pub fn status_message(&self) -> Option<&str> {
        if self.stable {
            return None;
        }
        self.status.as_ref().filter(|(_, at)| at.elapsed() < STATUS_TIME).map(|(s, _)| s.as_str())
    }

    pub fn log(&mut self, line: impl Into<String>) {
        if self.console.len() >= CONSOLE_LINES {
            self.console.drain(..CONSOLE_LINES / 4);
        }
        self.console.push(line.into());
    }

    /// Preferences kept across restarts (JSON for the host's storage).
    pub fn prefs_json(&self) -> String {
        json!({ "ui": self.ui }).to_string()
    }

    /// Restores [`PixelApp::prefs_json`]; malformed values are ignored.
    pub fn load_prefs(&mut self, json: &str) {
        let Ok(v) = serde_json::from_str::<Value>(json) else { return };
        if let Some(ui) = v.get("ui").and_then(|u| serde_json::from_value::<UiState>(u.clone()).ok())
            && ui.docking.validate().is_ok()
        {
            let start = self.ui.start_page;
            self.ui = ui;
            self.ui.start_page = start;
        }
    }

    // ---- documents ----------------------------------------------------------------

    /// Gives the session the current folders: the project's and the picked ones.
    ///
    /// Complexity: O(folders) file system calls.
    fn sync_file_access(&mut self) {
        self.session.set_file_access(self.files.access());
    }

    /// Allows the folder of a file the user chose (in a dialog or by
    /// dropping it on the window) for file ops, for the rest of the run.
    pub fn allow_picked(&mut self, file: &std::path::Path) {
        if self.files.allow_picked_file(file) {
            self.sync_file_access();
        }
    }

    /// Allows a folder the user named when starting the app
    /// (`pixel3d --allow-dir`) for file ops, for the rest of the run.
    pub fn allow_folder(&mut self, dir: &std::path::Path) -> Result<(), String> {
        if !dir.is_dir() {
            return Err(format!("{} {}", dir.display(), t("is not a folder")));
        }
        if self.files.allow_folder(dir) {
            self.sync_file_access();
        }
        Ok(())
    }

    /// The open document's file changed: its folder is the project folder.
    fn set_project_file(&mut self, file: Option<&std::path::Path>) {
        if self.files.set_project_file(file) {
            self.sync_file_access();
        }
    }

    fn loaded(&mut self, doc: Document, path: Option<PathBuf>) {
        self.session.load(doc);
        self.generation += 1;
        self.saved_revision = self.session.revision();
        self.set_project_file(path.as_deref());
        self.path = path;
        self.selection.clear();
        self.active = None;
        self.hidden.clear();
        self.viewport.hidden_changed();
        self.last_op = None;
        self.result = None;
        // Jobs started on the old document must not land in the new one.
        self.jobs.clear();
        self.sdf = sdf::SdfEditor::default();
        self.spec = spec::SpecEditor::default();
        self.outline = outline::OutlineEditor::default();
        self.job_card = jobcard::JobCard::default();
        self.coalesce = None;
        self.health = health::HealthState::default();
        self.viewport.frame_pending = true;
        self.ui.start_page = false;
    }

    pub fn new_document(&mut self) {
        self.loaded(Document::new(), None);
        self.set_status(t("New document"));
    }

    /// Opens a `.p3d` file.
    pub fn open_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        match p3d_io::p3d::load(path) {
            Ok(doc) => {
                let n = doc.object_count();
                self.loaded(doc, Some(path.to_owned()));
                self.ui.remember(path);
                self.set_status(format!("{} {} ({n} {})", t("Opened"), path.display(), t("objects")));
                Ok(())
            }
            Err(e) => {
                let msg = format!("{} {}: {e}", t("Cannot open"), path.display());
                self.set_status(msg.clone());
                Err(msg)
            }
        }
    }

    /// Asks the user for a file with a host dialog; the folder of the
    /// chosen file is allowed for file ops from now on.
    fn pick(&mut self, dialog: impl FnOnce(&Services) -> Option<PathBuf>) -> Option<PathBuf> {
        let chosen = dialog(&self.services)?;
        self.allow_picked(&chosen);
        Some(chosen)
    }

    /// Saves to `path`, or to the current file.
    pub fn save_to(&mut self, path: Option<PathBuf>) -> Result<Value, String> {
        let path = match path.or_else(|| self.path.clone()) {
            Some(p) => p,
            None => return Err(t("no file name: use Save As").to_owned()),
        };
        let path = if path.extension().is_none() { path.with_extension("p3d") } else { path };
        p3d_io::p3d::save(self.session.doc(), &path).map_err(|e| format!("{} {}: {e}", t("Cannot save"), path.display()))?;
        self.saved_revision = self.session.revision();
        self.set_project_file(Some(&path));
        self.path = Some(path.clone());
        self.ui.remember(&path);
        self.set_status(format!("{} {}", t("Saved"), path.display()));
        Ok(json!({ "path": path.display().to_string() }))
    }

    // ---- commands -----------------------------------------------------------------------

    /// Runs a command: a `ui.*`, `file.*`, `edit.*` or `view.*` command, or
    /// any registered op id (one undoable transaction with `params` as args).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let r = self.run_inner(id, &params);
        if let Err(e) = &r {
            self.set_status(e.clone());
        }
        r
    }

    fn run_inner(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let str_param = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_owned);
        match id {
            "file.new" => {
                self.new_document();
                Ok(json!({}))
            }
            "file.open" => {
                let path = match str_param("path") {
                    Some(p) => Some(PathBuf::from(p)),
                    None => self.pick(|s| s.pick_open.as_ref().and_then(|f| f())),
                };
                match path {
                    Some(p) => self.open_path(&p).map(|()| json!({ "path": p.display().to_string() })),
                    None => Ok(json!({ "cancelled": true })),
                }
            }
            "file.save" | "file.save_as" => {
                let mut path = str_param("path").map(PathBuf::from);
                if path.is_none() && (id == "file.save_as" || self.path.is_none()) {
                    let suggested = self
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map_or_else(|| "untitled.p3d".to_owned(), |n| n.to_string_lossy().into_owned());
                    path = self.pick(|s| s.pick_save.as_ref().and_then(|f| f(&suggested)));
                    if path.is_none() {
                        return Ok(json!({ "cancelled": true }));
                    }
                }
                self.save_to(path)
            }
            "file.import" | "io.import" if params.get("path").is_none() => {
                // Without a path, ask in a dialog; the chosen folder is
                // allowed from then on.
                let Some(path) = self.pick(|s| s.pick_import.as_ref().and_then(|f| f())) else {
                    return Ok(json!({ "cancelled": true }));
                };
                self.import_files(&[path], params)
            }
            "file.import" => {
                let path = str_param("path").ok_or("path must be a string")?;
                let mut rest = params.clone();
                if let Some(m) = rest.as_object_mut() {
                    m.remove("path");
                }
                self.import_files(&[PathBuf::from(path)], &rest)
            }
            "file.sample" => self.sample(),
            // A new SDF object (Add > SDF Shape): the starter shapes unless
            // `shapes` is given; the SDF op without shapes does the same.
            "shapes.new" => {
                let mut p = params.as_object().cloned().unwrap_or_default();
                if !p.contains_key("shapes") {
                    p.extend(sdf::starter().as_object().cloned().unwrap_or_default());
                }
                let op = sdf::op_id(self.session.registry());
                self.execute(&[OpCall::new(op, Value::Object(p))])
            }
            op if sdf::is_sdf_op(op) && params.get("shapes").is_none() => self.run_inner("shapes.new", params),
            // Long ops on a mesh (remesh): the run card, then a background run.
            "job.open" => {
                let op = str_param("op").ok_or("op is required: remesh.quad or sdf.from_mesh")?;
                self.open_job_card(&op)
            }
            "job.run" => {
                jobcard::merge(&mut self.job_card.params, params);
                self.run_job_card()
            }
            "job.cancel" => {
                self.cancel_job_card();
                Ok(json!({ "cancelling": self.job_card.busy() }))
            }
            "job.close" => {
                self.cancel_job_card();
                self.job_card.op = None;
                Ok(json!({}))
            }
            op if params.get("target").is_none() && jobcard::runs_in_card(self.session.registry(), op) => self.open_job_card(op),
            "view.poles" => {
                self.ui.view.poles = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.poles);
                Ok(json!({ "poles": self.ui.view.poles }))
            }
            "mesh.poles" => self.pole_counts(params.get("target").and_then(Value::as_str)),
            shapes_cmd if shapes_cmd.starts_with("shapes.") => self.sdf_command(shapes_cmd, params),
            // The outline editor: points, views, holes and fields; Add > Shell and Silhouettes.
            outline_cmd if outline_cmd.starts_with("outline.") => self.outline_command(outline_cmd, params),
            // The Spec tab: starters, parts, the raw source, landmarks.
            "spec.starter" | "spec.select_part" | "spec.json" | "spec.pick_landmark" | "spec.set_landmark" | "landmarks.toggle"
            | "view.landmarks" => self.spec_command(id, params),
            // A patch of the edited spec (the op itself when a target is given).
            "spec.patch" if params.get("target").is_none() => match (params.get("merge"), params.get("ops")) {
                (Some(m), _) => self.spec_patch(m.clone(), t("Edit spec")),
                (None, Some(ops)) => {
                    let target = self
                        .spec
                        .root
                        .and_then(|k| self.doc().object(k))
                        .map(|o| o.public_id())
                        .ok_or(t("select a spec root or one of its parts"))?;
                    self.execute(&[OpCall::new("spec.patch", json!({ "target": target, "ops": ops }))])
                }
                _ => Err(t("merge or ops is required").to_owned()),
            },
            "edit.undo" => {
                let label = self.session.undo().map_err(|e| e.message)?;
                self.prune_selection();
                self.set_status(format!("{} {label}", t("Undid")));
                Ok(json!({ "undone": label }))
            }
            "edit.redo" => {
                let label = self.session.redo().map_err(|e| e.message)?;
                self.prune_selection();
                self.set_status(format!("{} {label}", t("Redid")));
                Ok(json!({ "redone": label }))
            }
            "ui.transform" => self.ui.transform.apply(params),
            "view.cursor" => self.place_cursor(params),
            "edit.select" => {
                let target = params.get("target").and_then(Value::as_str).ok_or("target is required")?;
                let k = self.doc().resolve(target).map_err(|e| e.to_string())?;
                let add = params.get("add").and_then(Value::as_bool).unwrap_or(false);
                self.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add }]);
                Ok(json!({ "selected": self.selection.len() }))
            }
            "edit.element_op" => {
                let op = params.get("op").and_then(Value::as_str).ok_or("op is required")?.to_owned();
                let r = self.run_element_op(&op)?;
                // Rip Region: the split part follows the pointer next.
                if params.get("then").and_then(Value::as_str) == Some("move") {
                    self.ui.tool = Tool::Move;
                }
                Ok(r)
            }
            "edit.join" => {
                // Into the active object, as Blender's Ctrl+J.
                let targets = self.selected_targets();
                if targets.len() < 2 {
                    return Err(t("select two or more meshes, or two or more curves, to join").to_owned());
                }
                let into = self.active_object().and_then(|k| self.doc().object(k)).map(|o| o.public_id());
                self.execute(&[OpCall::new("scene.join", json!({ "targets": targets, "into": into }))])
            }
            "edit.new_collection" => {
                let members = self.selected_targets();
                outliner::new_collection(self, &members)
            }
            "edit.lod_group" => outliner::lod_group_from_selection(self),
            "edit.select_all" if self.ui.mode != SelectMode::Object && self.edit.object.is_some() => {
                self.edit.select_all(self.ui.mode);
                Ok(json!({ "elements": self.edit.picked().0.len() }))
            }
            "edit.select_none" if self.ui.mode != SelectMode::Object && self.edit.object.is_some() => {
                self.edit.clear();
                Ok(json!({ "elements": 0 }))
            }
            "edit.select_all" => {
                self.selection = self.doc().objects().map(|(k, _)| k).collect();
                Ok(json!({ "selected": self.selection.len() }))
            }
            "edit.select_none" => {
                self.selection.clear();
                Ok(json!({ "selected": 0 }))
            }
            "edit.delete" if self.ui.mode != SelectMode::Object => {
                // Ops take named groups, not picked ids (project spec 4.3);
                // deleting picked elements waits for an op that can.
                Err(t("Deleting selected vertices, edges or faces is not available yet; switch to Object mode to delete objects.").to_owned())
            }
            "edit.delete" => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Err(t("nothing selected").to_owned());
                }
                let calls: Vec<OpCall> = targets.iter().map(|t| OpCall::new("scene.delete", json!({ "target": t, "recursive": true }))).collect();
                let r = self.execute(&calls);
                self.prune_selection();
                r
            }
            "edit.duplicate" => {
                let targets = self.selected_targets();
                if targets.is_empty() {
                    return Err(t("nothing selected").to_owned());
                }
                let calls: Vec<OpCall> = targets.iter().map(|t| OpCall::new("scene.duplicate", json!({ "target": t }))).collect();
                self.execute(&calls)
            }
            "view.frame" => {
                let sel = self.selection.clone();
                self.viewport.frame(&sel);
                Ok(json!({}))
            }
            "view.ortho" => {
                self.ui.view.ortho = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.ortho);
                Ok(json!({ "ortho": self.ui.view.ortho }))
            }
            "view.grid" => {
                self.ui.view.grid = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.grid);
                Ok(json!({ "grid": self.ui.view.grid }))
            }
            "view.xray" => {
                self.ui.view.xray = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.xray);
                if let Some(a) = params.get("alpha").and_then(Value::as_f64) {
                    if !(0.0..=1.0).contains(&a) {
                        return Err(format!("alpha must be between 0 and 1, got {a}"));
                    }
                    self.ui.view.xray_alpha = a as f32;
                }
                Ok(json!({ "xray": self.ui.view.xray, "alpha": self.ui.view.xray_alpha }))
            }
            "view.stats" => {
                self.ui.view.stats = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.stats);
                Ok(json!({ "stats": self.ui.view.stats }))
            }
            "view.shading" => {
                let mode = str_param("mode").ok_or("mode is required: solid, solid_wire, wire or matcap")?;
                p3d_render::Shading::parse(&mode).ok_or_else(|| format!("unknown shading `{mode}`: use solid, solid_wire, wire or matcap"))?;
                self.ui.view.shading = mode.clone();
                Ok(json!({ "shading": mode }))
            }
            "view.set" => {
                let name = str_param("view").ok_or("view is required")?;
                let view = ViewKind::parse(&name).ok_or_else(|| format!("unknown view `{name}`"))?;
                self.viewport.set_view(view);
                Ok(json!({ "view": name }))
            }
            "ui.theme" => {
                let name = str_param("theme").ok_or("theme is required: system, light or dark")?;
                self.ui.theme = theme::ThemePref::parse(&name).ok_or_else(|| format!("unknown theme `{name}`: use system, light or dark"))?;
                Ok(json!({ "theme": self.ui.theme.as_str() }))
            }
            "ui.dock" => self.ui.docking.command(params),
            "ui.dock.reset" => {
                // The current workspace's preset.
                self.ui.docking = docking::preset(self.ui.workspace);
                Ok(json!({}))
            }
            "ui.start" => {
                self.ui.start_page = params.get("on").and_then(Value::as_bool).unwrap_or(true);
                Ok(json!({ "startPage": self.ui.start_page }))
            }
            "ui.bottom" => {
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!self.bottom_panel_visible());
                // Close the console first: History restores into its group.
                let order = if on { [docking::Panel::History, docking::Panel::Console] } else { [docking::Panel::Console, docking::Panel::History] };
                for p in order {
                    self.ui.docking.set_visible(p, on)?;
                }
                Ok(json!({ "bottom": on }))
            }
            "ui.workspace" => {
                let ws =
                    serde_json::from_value::<WorkspaceKind>(params.get("workspace").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                if ws != self.ui.workspace {
                    // Keep this workspace's layout; bring back the other's, or its preset.
                    let current = std::mem::take(&mut self.ui.docking);
                    self.ui.layouts.insert(self.ui.workspace, current);
                    self.ui.docking = self.ui.layouts.remove(&ws).unwrap_or_else(|| docking::preset(ws));
                    self.ui.workspace = ws;
                }
                Ok(json!({ "workspace": ws }))
            }
            "view.snap" => {
                self.ui.view.snap = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.view.snap);
                Ok(json!({ "snap": self.ui.view.snap }))
            }
            "selection.run" => {
                let op = str_param("op").ok_or("op is required: an op that takes a target")?;
                self.run_on_selection(&op)
            }
            "selection.action" => {
                // The contextual action of an op: its inputs come from the
                // selection as the tool settings row would pass them.
                let op = str_param("op").ok_or("op is required")?;
                let action = actions::for_context(self).into_iter().find(|a| a.op == op);
                match action {
                    Some(a) => actions::run(self, &a),
                    None => Err(actions::selection_hint(self, &op)),
                }
            }
            // A boolean on the selection: the active mesh is the target, the
            // other selected meshes are the tools (`operation`: difference,
            // union or intersect; also `keep_tool` and `weld`).
            "selection.boolean" => self.run_tools_on_active("edit.boolean", params),
            "edit.boolean" if params.get("target").is_none() => self.run_tools_on_active("edit.boolean", params),
            "edit.select_group" => {
                let target = str_param("target").ok_or("target is required: the mesh object")?;
                let group = str_param("group").ok_or("group is required: a group name, for example boolean_1.cut")?;
                let k = self.doc().resolve(&target).map_err(|e| e.to_string())?;
                self.select_group(k, &group)
            }
            "view.modal" => {
                // G, R, S: a transform that follows the pointer until a click.
                let kind = match params.get("kind").and_then(Value::as_str) {
                    Some("move") | None => gizmo::Kind::Move,
                    Some("rotate") => gizmo::Kind::Rotate,
                    Some("scale") => gizmo::Kind::Scale,
                    Some(other) => return Err(format!("{} `{other}`: move, rotate or scale", t("unknown transform"))),
                };
                let has = if self.ui.mode.is_edit() { self.edit.flushed().is_some_and(|f| !f.verts.is_empty()) } else { !self.selection.is_empty() };
                if !has {
                    return Err(t("nothing selected").to_owned());
                }
                self.gizmo.pending = Some(kind);
                Ok(json!({ "modal": params.get("kind").cloned().unwrap_or(json!("move")) }))
            }
            "view.benchmark" => {
                // Orbits the camera every frame for `seconds` and measures.
                let secs = params.get("seconds").and_then(Value::as_f64).unwrap_or(5.0).clamp(0.5, 120.0);
                let now = std::time::Instant::now();
                self.bench = Some((now, now + std::time::Duration::from_secs_f64(secs), 0));
                self.bench_result = None;
                Ok(json!({ "seconds": secs }))
            }
            "app.control" => {
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(self.remote.addr().is_none());
                if on {
                    // The default port lets `pixel3d-cli mcp --connect` find the
                    // app; another free one when a second app holds it.
                    let r = match params.get("port").and_then(Value::as_u64) {
                        Some(port) => self.start_control(u16::try_from(port).map_err(|_| t("port must be 0 to 65535"))?),
                        None => self.start_control(p3d_control::DEFAULT_PORT).or_else(|_| self.start_control(0)),
                    };
                    r.map(|a| json!({ "control": a.to_string() }))
                } else {
                    self.stop_control();
                    Ok(json!({ "control": null }))
                }
            }
            "ui.palette" => Ok(palette::toggle(self, params.get("on").and_then(Value::as_bool))),
            "ui.rail_labels" => {
                self.ui.rail_labels = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.rail_labels);
                Ok(json!({ "railLabels": self.ui.rail_labels }))
            }
            "ui.tool" => {
                let tool = serde_json::from_value::<Tool>(params.get("tool").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                if tool == Tool::Knife && !self.ui.mode.is_edit() {
                    return Err(t("The knife works in Edit Mode (Tab).").to_owned());
                }
                self.ui.tool = tool;
                Ok(json!({ "tool": tool }))
            }
            "ui.mode" => {
                let raw = params.get("mode").cloned().unwrap_or(Value::Null);
                let mode = if raw.as_str() == Some("edit") {
                    self.ui.element_mode
                } else {
                    serde_json::from_value::<SelectMode>(raw).map_err(|_| t("mode is object, edit, vert, edge or face").to_owned())?
                };
                self.set_mode(mode)
            }
            "ui.edit" => {
                // Tab: between Object mode and the last element mode.
                let mode = if self.ui.mode.is_edit() { SelectMode::Object } else { self.ui.element_mode };
                self.set_mode(mode)
            }
            "app.quit" => {
                self.quit_requested = true;
                Ok(json!({}))
            }
            op if self.session.registry().get(op).is_ok() => {
                let calls = [OpCall::new(op, params.clone())];
                self.execute(&calls)
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// Runs an op that takes a `target` on every selected object, as one
    /// transaction (one undo step).
    ///
    /// Complexity: O(selected objects) calls.
    pub fn run_on_selection(&mut self, op: &str) -> Result<Value, String> {
        let targets = self.selected_targets();
        if targets.is_empty() {
            let e = t("nothing selected").to_owned();
            self.set_status(e.clone());
            return Err(e);
        }
        let calls: Vec<OpCall> = targets.iter().map(|id| OpCall::new(op, json!({ "target": id }))).collect();
        let r = self.execute(&calls);
        if let Err(e) = &r {
            self.set_status(e.clone());
        }
        r
    }

    /// Runs op calls as one transaction; created objects become the selection.
    pub fn execute(&mut self, calls: &[OpCall]) -> Result<Value, String> {
        self.execute_labeled(calls, None)
    }

    /// [`PixelApp::execute`] with an undo label (the op labels when `None`).
    ///
    /// Reports (ops that are not undoable, such as `cleanup.report`) change
    /// nothing, so they run without adding an undo step.
    pub fn execute_labeled(&mut self, calls: &[OpCall], label: Option<&str>) -> Result<Value, String> {
        let registry = std::sync::Arc::clone(self.session.registry());
        let read_only =
            !calls.is_empty() && calls.iter().all(|c| registry.get(&c.op).is_ok_and(|o| !o.meta().flags.contains(p3d_ops::OpFlags::UNDOABLE)));
        let opts = BatchOptions { label: label.map(str::to_owned), dry_run: read_only, ..BatchOptions::default() };
        match self.session.execute(calls, &opts) {
            Ok(r) => Ok(self.committed(&r)),
            Err(e) => Err(e.error.to_string()),
        }
    }

    /// Takes in a finished batch: logs the summaries and warnings, keeps
    /// the report for the result card, makes a single op adjustable, and
    /// selects the objects it created.
    ///
    /// Complexity: O(results + warnings + created objects).
    pub(crate) fn committed(&mut self, r: &p3d_engine::BatchResult) -> Value {
        let mut created = Vec::new();
        for res in &r.results {
            self.log(res.output.summary.clone());
            for w in &res.output.warnings {
                self.log(format!("  {}: {}", t("warning"), w.message));
                if let Some(f) = &w.fix {
                    self.log(format!("    {}: {} {}", t("fix"), f.op, f.args));
                }
            }
            created.extend(res.output.created.iter().copied());
        }
        let result = results::LastResult::from_batch(r, self.session.registry());
        let warnings = result.warning_count();
        if let Some(last) = r.results.last() {
            let status = match warnings {
                0 => last.output.summary.clone(),
                1 => format!("{} ({})", last.output.summary, t("1 warning")),
                n => format!("{} ({n} {})", last.output.summary, t("warnings")),
            };
            self.status = Some((status, Instant::now()));
        }
        if !r.dry_run {
            self.last_op = match r.results.as_slice() {
                [one] => Some(LastOp { op: one.op.clone(), params: one.params.clone(), revision: r.revision, error: None }),
                _ => None,
            };
            if !created.is_empty() {
                self.active = created.first().copied();
                self.selection = created.into_iter().collect();
            }
            self.note_cut_group(r);
        }
        self.result = Some(result);
        self.ui.start_page = false;
        let mut out = json!({ "revision": r.revision, "summaries": r.results.iter().map(|x| x.output.summary.clone()).collect::<Vec<_>>() });
        let all: Vec<&p3d_ops::Warning> = r.results.iter().flat_map(|x| &x.output.warnings).collect();
        if !all.is_empty() {
            out["warnings"] = json!(all);
        }
        out
    }

    /// Remembers the faces a cut made (a group named `*.cut`, as booleans
    /// make), so Edit Mode can start with them selected.
    ///
    /// Complexity: O(groups the batch made).
    fn note_cut_group(&mut self, r: &p3d_engine::BatchResult) {
        self.edit_focus =
            r.results.iter().flat_map(|x| &x.output.groups).find(|g| g.name.ends_with(".cut")).map(|g| (r.revision, g.object, g.name.clone()));
    }

    /// Starts op calls in the background as one transaction (imports, which
    /// may take a while); [`PixelApp::poll_jobs`] commits the result.
    pub fn start_job(&mut self, calls: Vec<OpCall>, label: &str) {
        let job = self.session.spawn_job(calls, label);
        self.jobs.push(RunningJob { job, label: label.to_owned() });
        self.set_status(format!("{label}..."));
    }

    /// Commits the background jobs that finished; returns whether any are
    /// still running. Call once per frame.
    ///
    /// Complexity: O(jobs) plus the work of committing finished ones.
    pub fn poll_jobs(&mut self) -> bool {
        let mut i = 0;
        while i < self.jobs.len() {
            match self.session.poll_job(&mut self.jobs[i].job) {
                p3d_engine::JobStatus::Pending(_) => i += 1,
                p3d_engine::JobStatus::Done(r) => {
                    self.jobs.remove(i);
                    self.committed(&r);
                    self.prune_selection();
                }
                p3d_engine::JobStatus::Failed(e) => {
                    let job = self.jobs.remove(i);
                    self.set_status(format!("{}: {}", job.label, e.error));
                }
            }
        }
        !self.jobs.is_empty()
    }

    /// Imports model files (glTF, GLB, OBJ, FBX) as one undoable step in
    /// the background. `args` holds the `io.import` parameters other than
    /// the path (name, parent, weld).
    pub fn import_files(&mut self, files: &[PathBuf], args: &Value) -> Result<Value, String> {
        if files.is_empty() {
            return Err(t("no file to import").to_owned());
        }
        let calls: Vec<OpCall> = files
            .iter()
            .map(|f| {
                let mut a = args.as_object().cloned().unwrap_or_default();
                a.insert("path".into(), json!(f.display().to_string()));
                OpCall::new("io.import", Value::Object(a))
            })
            .collect();
        let name = |f: &PathBuf| f.file_name().map_or_else(|| f.display().to_string(), |n| n.to_string_lossy().into_owned());
        let label = match files {
            [one] => format!("{} {}", t("Import"), name(one)),
            many => format!("{} {} {}", t("Import"), many.len(), t("files")),
        };
        self.start_job(calls, &label);
        Ok(json!({ "importing": files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>() }))
    }

    /// Files dropped on the window: a `.p3d` scene opens, model files are
    /// imported into the scene (their folders become allowed, as when picked
    /// in a dialog), anything else is refused with a message.
    pub fn drop_files(&mut self, files: &[PathBuf]) {
        let scene = files.iter().find(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("p3d")));
        let models: Vec<PathBuf> = files.iter().filter(|f| is_model_file(f)).cloned().collect();
        let other = files.iter().find(|f| Some(*f) != scene && !is_model_file(f));
        if let Some(s) = scene {
            let _ = self.open_path(s);
        }
        for m in &models {
            self.allow_picked(m);
        }
        if !models.is_empty() {
            let _ = self.import_files(&models, &Value::Null);
        }
        if let Some(f) = other {
            self.set_status(format!("{} {}: {}", t("Cannot use"), f.display(), t("drop a .p3d scene, or .glb, .gltf, .obj or .fbx models")));
        }
    }

    /// Re-runs the last single-op transaction with new parameters, in place
    /// of the old run (still one undo step). Refused when anything changed
    /// since; a failing run leaves the old result.
    pub fn adjust_last(&mut self, params: Value) -> Result<(), String> {
        let Some(last) = self.last_op.clone() else { return Err(t("nothing to adjust").to_owned()) };
        if self.session.revision() != last.revision {
            return Err(t("the scene changed since that operation").to_owned());
        }
        let label = self.session.undo().map_err(|e| e.message)?;
        let opts = BatchOptions { label: Some(label), ..BatchOptions::default() };
        match self.session.execute(&[OpCall::new(&last.op, params)], &opts) {
            Ok(r) => {
                if let Some(one) = r.results.first() {
                    self.status = Some((one.output.summary.clone(), Instant::now()));
                    if let Some(&k) = one.output.created.first() {
                        self.active = Some(k);
                        self.selection = one.output.created.iter().copied().collect();
                    }
                    self.last_op = Some(LastOp { op: last.op, params: one.params.clone(), revision: r.revision, error: None });
                }
                self.result = Some(results::LastResult::from_batch(&r, self.session.registry()));
                self.note_cut_group(&r);
                self.prune_selection();
                Ok(())
            }
            Err(e) => {
                let _ = self.session.redo();
                let msg = e.error.to_string();
                if let Some(l) = &mut self.last_op {
                    l.error = Some(msg.clone());
                }
                Err(msg)
            }
        }
    }

    /// Runs calls as one transaction that replaces the previous one of the
    /// same drag (one drag, one undo step). `dragging` false ends the drag.
    pub fn execute_coalesced(&mut self, calls: &[OpCall], dragging: bool) -> Result<Value, String> {
        if let Some(rev) = self.coalesce.take()
            && rev == self.session.revision()
        {
            let _ = self.session.undo();
        }
        let r = self.execute(calls);
        match &r {
            Ok(_) if dragging => self.coalesce = Some(self.session.revision()),
            Ok(_) => {}
            Err(e) => self.set_status(e.clone()),
        }
        r
    }

    /// Ends a coalesced drag: the next edit is a new undo step.
    pub fn end_drag(&mut self) {
        self.coalesce = None;
    }

    /// Hides or shows objects in the viewport only.
    pub fn set_hidden(&mut self, key: ObjectKey, hidden: bool) {
        if hidden {
            self.hidden.insert(key);
            self.selection.remove(&key);
        } else {
            self.hidden.remove(&key);
        }
        self.viewport.hidden_changed();
    }

    /// The object Properties shows: the active one if selected, else the
    /// only selected one.
    pub fn active_object(&self) -> Option<ObjectKey> {
        match self.active {
            Some(a) if self.selection.contains(&a) && self.doc().object(a).is_some() => Some(a),
            _ if self.selection.len() == 1 => self.selection.first().copied().filter(|k| self.doc().object(*k).is_some()),
            _ => None,
        }
    }

    /// A small sample scene built with ops (undoable like any edit).
    fn sample(&mut self) -> Result<Value, String> {
        let calls = [
            OpCall::new(
                "prim.box",
                json!({ "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6, "bevel": { "width": "8mm", "segments": 2 } }),
            ),
            OpCall::new("prim.cylinder", json!({ "name": "barrel", "radius": 0.28, "height": 0.9, "position": [1.1, 0, 0] })),
            OpCall::new("prim.quad_sphere", json!({ "name": "ball", "radius": 0.22, "position": [-1.0, 0, 0.3] })),
            OpCall::new("prim.plane", json!({ "name": "floor", "width": 4, "depth": 3, "advanced": { "segments": [4, 3] } })),
        ];
        let r = self.execute_labeled(&calls, Some(t("Open the sample scene")));
        self.selection.clear();
        self.viewport.frame_pending = true;
        r
    }

    /// Ids of selected objects that still exist, without those whose
    /// ancestor is selected too (an op on the parent covers them).
    ///
    /// Complexity: O(s x depth) for s selected objects.
    pub fn selected_targets(&self) -> Vec<String> {
        let doc = self.doc();
        let mut keys: Vec<ObjectKey> = self.selection.iter().copied().filter(|k| doc.object(*k).is_some()).collect();
        keys.retain(|k| {
            let mut p = doc.object(*k).and_then(|o| o.parent());
            while let Some(q) = p {
                if self.selection.contains(&q) {
                    return false;
                }
                p = doc.object(q).and_then(|o| o.parent());
            }
            true
        });
        keys.iter().filter_map(|k| doc.object(*k).map(|o| o.public_id())).collect()
    }

    /// Drops selected objects that no longer exist.
    pub fn prune_selection(&mut self) {
        let doc = std::sync::Arc::clone(self.session.doc());
        self.selection.retain(|k| doc.object(*k).is_some());
    }

    pub fn handle_viewport(&mut self, events: Vec<ViewportEvent>) {
        for e in events {
            match e {
                ViewportEvent::Picked { object, add } => match (object, add) {
                    (Some(k), true) => {
                        if self.selection.remove(&k) {
                            self.active = self.selection.last().copied();
                        } else {
                            self.selection.insert(k);
                            self.active = Some(k);
                        }
                    }
                    (Some(k), false) => {
                        self.selection.clear();
                        self.selection.insert(k);
                        self.active = Some(k);
                    }
                    (None, false) => self.selection.clear(),
                    (None, true) => {}
                },
                ViewportEvent::BoxSelect { area, op } => self.box_select(area, op),
                ViewportEvent::MissedAt { at, add } => {
                    // Curves have no surface in the id pass: try them by distance.
                    let key = self.scene_key();
                    let doc = std::sync::Arc::clone(self.session.doc());
                    let lines = self.curves.get(key, &doc, &self.hidden);
                    let curve = curves::pick(&self.viewport.camera, self.viewport.rect, lines, at);
                    self.handle_viewport(vec![ViewportEvent::Picked { object: curve, add }]);
                }
                ViewportEvent::KnifeCut { a, b } => {
                    if let Err(e) = self.knife_cut(a, b) {
                        self.set_status(e);
                    }
                }
                ViewportEvent::PlaceCursor { at } => {
                    let p = self.viewport.surface_at(at).unwrap_or_else(|| self.viewport.ground_at(at));
                    self.ui.transform.cursor = p.to_array();
                }
                ViewportEvent::ElementClick { at, add } => {
                    let Some(k) = self.edit.object else { continue };
                    let (Some((_, mesh, model)), Some(bvh)) = (self.viewport.part_of(k), self.edit.bvh()) else { continue };
                    let hit = elements::pick(&mesh, &bvh, model, &self.viewport.camera, self.viewport.rect, at, self.ui.mode);
                    self.edit.click(hit, add);
                }
            }
        }
    }

    // ---- frame ------------------------------------------------------------------------------

    /// Takes keys egui would otherwise use for focus before egui sees them:
    /// Tab toggles Edit Mode (as in Blender) instead of moving the keyboard
    /// focus through the menus, unless a text field is being edited. Call
    /// from the host's raw input hook.
    pub fn filter_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.feed_remote_input(raw);
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (mut toggle, mut snap) = (false, false);
        raw.events.retain(|e| match e {
            egui::Event::Key { key: egui::Key::Tab, pressed, modifiers, .. } if modifiers.is_none() => {
                toggle |= *pressed;
                false
            }
            // Shift+Tab: snapping, as in Blender (not focus backwards).
            egui::Event::Key { key: egui::Key::Tab, pressed, modifiers, .. } if *modifiers == egui::Modifiers::SHIFT => {
                snap |= *pressed;
                false
            }
            _ => true,
        });
        if toggle {
            let _ = self.run("ui.edit", Value::Null);
        }
        if snap {
            let _ = self.run("view.snap", Value::Null);
        }
    }

    /// Per-frame logic before layout: theme, shortcuts, screenshots, dropped files.
    pub fn logic(&mut self, ctx: &egui::Context) {
        let shown = self.ui.theme.resolve(ctx.system_theme());
        let follow = self.ui.theme == theme::ThemePref::System;
        theme::set_active(shown);
        if self.shown_theme != Some((shown, follow)) {
            theme::apply(ctx, shown, follow);
            self.shown_theme = Some((shown, follow));
        }
        chrome::shortcuts(self, ctx);
        self.drain_control(ctx);
        if let Some((start, end, frames)) = self.bench {
            let now = std::time::Instant::now();
            if now >= end {
                let secs = now.duration_since(start).as_secs_f64();
                let tris: usize = self.stats().total.tris;
                self.bench_result = Some(json!({ "frames": frames, "seconds": secs, "fps": f64::from(frames) / secs, "triangles": tris }));
                self.bench = None;
                self.set_status(format!("{} {:.1} fps", t("Benchmark:"), f64::from(frames) / secs));
            } else {
                self.viewport.camera.orbit(0.01, 0.0);
                self.bench = Some((start, end, frames + 1));
                ctx.request_repaint();
            }
        }
        self.shots.tick(ctx);
        if self.quit_after_shot
            && let Some(done) = &self.shots.done
        {
            if let Err(e) = done {
                tracing::error!(error = %e, "screenshot failed");
            }
            self.quit_requested = true;
        }
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.drop_files(&dropped);
        }
        if self.poll_jobs() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.spec_tick(ctx);
        self.sdf_tick(ctx);
        self.outline_tick(ctx);
        self.job_card_tick(ctx);
        if let Some((_, at)) = &self.status {
            let left = STATUS_TIME.saturating_sub(at.elapsed());
            if !left.is_zero() {
                ctx.request_repaint_after(left);
            }
        }
    }

    /// Lays out the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t0 = Instant::now();
        control::begin_frame(ui.ctx());
        let tk = theme::Tokens::get();
        if !self.ui.start_page {
            let (key, seed) = (self.scene_key(), self.session.seed());
            let (doc, registry) = (std::sync::Arc::clone(self.session.doc()), std::sync::Arc::clone(self.session.registry()));
            self.health.update(key, &doc, &registry, seed, ui.ctx());
        }
        chrome::top_bar(self, ui);
        if self.ui.show_toolbar {
            chrome::tool_settings(self, ui);
        }
        if self.ui.show_status_bar {
            chrome::status_bar(self, ui);
        }
        if !self.ui.start_page {
            chrome::tool_sidebar(self, ui);
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tk.border)).show(ui, |ui| {
            if self.ui.start_page {
                chrome::start_page(self, ui);
            } else {
                self.dock(ui);
            }
        });
        palette::show(self, ui);
        chrome::drop_overlay(ui);
        self.frame_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn dock(&mut self, ui: &mut egui::Ui) {
        let workspace = self.ui.docking.clone();
        let mut viewport_events = Vec::new();
        let mut nav = None;
        let actions = docking::show(ui, &workspace, |ui, panel| match panel {
            docking::Panel::Viewport => {
                // While an Edit Mode gizmo drag runs, the viewport shows its
                // preview document.
                let preview = self
                    .gizmo
                    .preview
                    .clone()
                    .map(|(d, n)| (d, n | (1 << 63)))
                    .or_else(|| self.action_preview.doc.clone().map(|(d, n)| (d, n | (1 << 62))))
                    .or_else(|| self.sdf.preview_doc().map(|(d, n)| (d, n | (1 << 61))))
                    .or_else(|| self.outline.preview_doc().map(|(d, n)| (d, n | (1 << 60))));
                let (doc, key) = match preview {
                    Some((d, n)) => (d, n),
                    None => (std::sync::Arc::clone(self.session.doc()), self.scene_key()),
                };
                let style = theme::Tokens::get().view_style();
                let sel = self.selection.clone();
                self.viewport.scene(&doc, key, &self.hidden);
                self.viewport.set_selection(&sel);
                // The element modes edit the active mesh object; its overlay
                // is rebuilt only when the selection or the mesh changes.
                let element_mode = self.ui.mode != SelectMode::Object;
                let editing = if element_mode { self.active_object() } else { self.edit.object };
                let part = editing.and_then(|k| self.viewport.part_of(k));
                // The preview mesh keeps the element ids: no resync then.
                let previewing = self.gizmo.preview.is_some() || self.action_preview.doc.is_some();
                if !previewing {
                    self.edit.sync(part.as_ref().and(editing), part.as_ref().map(|p| &p.1), self.ui.mode);
                }
                self.viewport.edit_overlay = match (&part, element_mode) {
                    (Some((i, mesh, _)), true) => self.edit.overlay(*i, self.ui.mode, previewing.then_some(mesh)).map(|(o, k, _)| (o, k)),
                    // Object Mode: the faces of the shape row under the pointer.
                    (_, false) => self.spec_highlight().or_else(|| self.sdf_highlight()),
                    _ => None,
                };
                self.viewport.pick_elements = element_mode && self.edit.object.is_some();
                self.viewport.knife = element_mode && self.ui.tool == Tool::Knife;
                self.viewport.suppress = self.gizmo.modal();
                let view_rect = chrome::viewport_header(self, ui, ui.max_rect());
                viewport_events = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(view_rect), |ui| {
                        self.viewport.show(ui, &doc, key, &self.hidden, &self.ui.view, style)
                    })
                    .inner;
                chrome::viewport_overlay(self, ui);
                sdf::overlay(self, ui, &mut viewport_events);
                spec::overlay(self, ui, &mut viewport_events);
                spec::provenance_tip(self, ui);
                outline::overlay(self, ui);
                poles::overlay(self, ui);
                let gizmo_rect = self.viewport.rect;
                gizmo::show(self, ui, gizmo_rect);
                quick::show(self, ui, gizmo_rect);
                jobcard::show(self, ui, gizmo_rect);
                health::badges(self, ui);
                nav = navgizmo::show(ui, view_rect, &mut self.viewport, self.ui.view.ortho);
            }
            other => {
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| match other {
                    docking::Panel::Outliner => outliner::show(self, ui),
                    docking::Panel::Properties => inspector::show(self, ui),
                    docking::Panel::Health => health::show(self, ui),
                    docking::Panel::History => panels::history(self, ui),
                    docking::Panel::Console | docking::Panel::Viewport => panels::console(self, ui),
                    docking::Panel::UvEditor => panels::uv_editor(self, ui),
                    docking::Panel::Timeline => panels::timeline(self, ui),
                    docking::Panel::Bake => panels::bake(self, ui),
                    docking::Panel::Export => panels::export(self, ui),
                });
            }
        });
        self.handle_viewport(viewport_events);
        match nav {
            Some(navgizmo::NavCommand::ToggleOrtho) => {
                let _ = self.run("view.ortho", Value::Null);
            }
            Some(navgizmo::NavCommand::Frame) => {
                let _ = self.run("view.frame", Value::Null);
            }
            None => {}
        }
        for action in actions {
            if let Err(e) = self.ui.docking.apply(action) {
                self.set_status(format!("{}: {e}", t("Panel move was not applied")));
            }
        }
    }
}

/// Whether some object schema in `schema` (at any depth, `$defs` included)
/// has a property named `name`.
///
/// Complexity: O(schema size).
fn has_property(schema: &Value, name: &str) -> bool {
    match schema {
        Value::Object(map) => {
            map.get("properties").and_then(Value::as_object).is_some_and(|p| p.contains_key(name)) || map.values().any(|v| has_property(v, name))
        }
        Value::Array(items) => items.iter().any(|v| has_property(v, name)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box in Edit Mode, face select, with the viewport scene built.
    fn editing_box() -> (PixelApp, ObjectKey) {
        let mut a = PixelApp::new(Session::new(), Services::default());
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        let k = a.doc().resolve("crate").unwrap();
        a.run("ui.mode", json!({ "mode": "face" })).unwrap();
        let doc = std::sync::Arc::clone(a.session.doc());
        a.viewport.scene(&doc, a.scene_key(), &BTreeSet::new());
        (a, k)
    }

    #[test]
    fn element_ops_run_on_the_picked_elements_and_select_the_result() {
        let (mut a, k) = editing_box();
        assert!(a.element_op_state("edit.extrude").is_err(), "nothing picked yet");
        let face = a.doc().object_mesh(k).and_then(|m| m.faces().next()).unwrap();
        a.edit.click(Some(face), false);
        assert_eq!(a.element_op_state("edit.extrude"), Ok(Some(json!({ "faces": { "ids": [face] } }))));
        let faces = a.doc().object_mesh(k).map(|m| m.face_count()).unwrap();
        a.run("edit.element_op", json!({ "op": "edit.extrude" })).unwrap();
        assert_eq!(a.doc().object_mesh(k).map(|m| m.face_count()), Some(faces + 4), "four side walls");
        // The extruded cap is selected, as in Blender.
        let (picked, mode) = a.edit.picked();
        let cap =
            a.doc().object_mesh(k).and_then(|m| m.groups().find(|(n, _)| n.ends_with(".cap")).map(|(_, g)| g.iter().collect::<BTreeSet<u32>>()));
        assert_eq!((Some(picked.clone()), mode), (cap, Some(SelectMode::Face)));
        // Rip: split the selection, then move it.
        a.run("edit.element_op", json!({ "op": "edit.split", "then": "move" })).unwrap();
        assert_eq!(a.ui.tool, Tool::Move);
        assert!(!a.edit.picked().0.is_empty());
    }

    #[test]
    fn the_knife_cuts_the_faces_its_line_crosses() {
        let (mut a, k) = editing_box();
        a.viewport.rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        a.viewport.camera.set_view(p3d_render::ViewKind::Front);
        let (lo, hi) = a.doc().object_mesh(k).and_then(|m| m.bounds()).unwrap();
        a.viewport.camera.frame(lo, hi, 1.0);
        a.run("ui.mode", json!({ "mode": "edge" })).unwrap();
        let faces = a.doc().object_mesh(k).map(|m| m.face_count()).unwrap();
        // A vertical line through the middle of the front face, past its edges.
        a.knife_cut(egui::pos2(200.0, 20.0), egui::pos2(200.0, 380.0)).unwrap();
        assert_eq!(a.doc().object_mesh(k).map(|m| m.face_count()), Some(faces + 1), "the front face is cut in two");
        assert!(!a.edit.picked().0.is_empty(), "the cut edges are selected");
        assert!(a.knife_cut(egui::pos2(1.0, 1.0), egui::pos2(3.0, 2.0)).is_err(), "a line that crosses nothing");
    }

    fn app() -> PixelApp {
        PixelApp::new(Session::new(), Services::default())
    }

    #[test]
    fn workspaces_switch_layouts_and_keep_their_own() {
        let mut a = app();
        let model = a.ui.docking.clone();
        a.run("ui.workspace", json!({ "workspace": "uv" })).unwrap();
        assert!(a.ui.docking.is_visible(docking::Panel::UvEditor), "the UV preset shows the UV editor");
        assert!(a.ui.docking.validate().is_ok());
        // A change in the UV workspace stays with it.
        a.ui.docking.set_visible(docking::Panel::Outliner, false).unwrap();
        a.run("ui.workspace", json!({ "workspace": "model" })).unwrap();
        assert_eq!(a.ui.docking, model);
        a.run("ui.workspace", json!({ "workspace": "uv" })).unwrap();
        assert!(!a.ui.docking.is_visible(docking::Panel::Outliner));
        // Saved with the preferences and restored.
        let mut b = app();
        b.load_prefs(&a.prefs_json());
        assert_eq!((b.ui.workspace, &b.ui.docking), (WorkspaceKind::Uv, &a.ui.docking));
        // Reset brings back this workspace's preset.
        a.run("ui.dock.reset", Value::Null).unwrap();
        assert_eq!(a.ui.docking, docking::preset(WorkspaceKind::Uv));
        for ws in WorkspaceKind::ALL {
            assert!(docking::preset(ws).validate().is_ok(), "{ws:?}");
        }
    }

    #[test]
    fn tab_toggles_edit_mode_instead_of_moving_focus() {
        let mut a = app();
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        let ctx = egui::Context::default();
        let tab = |pressed| egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        let mut raw = egui::RawInput { events: vec![tab(true), tab(false), egui::Event::Text("a".into())], ..Default::default() };
        a.filter_input(&ctx, &mut raw);
        assert_eq!(raw.events, vec![egui::Event::Text("a".into())], "egui never sees Tab");
        assert!(a.ui.mode.is_edit());
        let mut raw = egui::RawInput { events: vec![tab(true)], ..Default::default() };
        a.filter_input(&ctx, &mut raw);
        assert_eq!(a.ui.mode, SelectMode::Object);
    }

    #[test]
    fn ops_run_as_undoable_commands_and_select_what_they_create() {
        let mut a = app();
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        assert_eq!(a.doc().object_count(), 1);
        assert_eq!(a.selection.len(), 1, "the new box is selected");
        assert!(a.is_dirty());
        a.run("edit.duplicate", Value::Null).unwrap();
        assert_eq!(a.doc().object_count(), 2);
        a.run("edit.undo", Value::Null).unwrap();
        assert_eq!(a.doc().object_count(), 1);
        a.run("edit.redo", Value::Null).unwrap();
        assert_eq!(a.doc().object_count(), 2);
        a.run("edit.select_all", Value::Null).unwrap();
        a.run("edit.delete", Value::Null).unwrap();
        assert_eq!(a.doc().object_count(), 0);
        assert!(a.selection.is_empty());
        assert!(a.run("edit.delete", Value::Null).is_err());
        assert!(a.run("prim.box", json!({ "widht": 1 })).unwrap_err().contains("widht"));
        assert!(a.run("no.such", Value::Null).is_err());
    }

    #[test]
    fn save_and_open_round_trip() {
        let dir = std::env::temp_dir().join(format!("p3d-ui-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("scene.p3d");
        let mut a = app();
        a.run("file.sample", Value::Null).unwrap();
        assert_eq!(a.doc().object_count(), 4);
        a.run("file.save", json!({ "path": path.display().to_string() })).unwrap();
        assert!(!a.is_dirty());
        let mut b = app();
        b.run("file.open", json!({ "path": path.display().to_string() })).unwrap();
        assert_eq!(b.doc().object_count(), 4);
        assert!(!b.is_dirty() && !b.ui.start_page);
        assert!(b.title().starts_with("scene.p3d"));
        assert_ne!(a.scene_key() >> 40, b.scene_key() >> 40, "a load starts a new cache generation");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Commits background jobs until none run (a few seconds at most).
    fn finish_jobs(a: &mut PixelApp) {
        for _ in 0..2000 {
            if !a.poll_jobs() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("a job did not finish");
    }

    #[test]
    fn imports_run_in_the_background_and_offer_their_fixes() {
        let dir = files::tests::temp_dir("import");
        // A box without its top: cleanup.auto would fill the hole.
        let bin = dir.join("bin.obj");
        let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
                   f 1 4 3 2\nf 1 2 6 5\nf 3 4 8 7\nf 1 5 8 4\nf 2 3 7 6\n";
        std::fs::write(&bin, obj).unwrap();
        let picked = bin.clone();
        let services = Services { pick_import: Some(Box::new(move || Some(picked.clone()))), ..Services::default() };
        let mut a = PixelApp::new(Session::new(), services);
        // File > Import...: the dialog's folder is allowed, the import runs as a job.
        assert!(a.run("file.import", Value::Null).unwrap()["importing"].is_array());
        assert_eq!(a.running_jobs(), 1);
        finish_jobs(&mut a);
        assert_eq!((a.doc().object_count(), a.selection.len(), a.session.history().len()), (1, 1, 1), "one undoable step, selected");
        assert_eq!(a.last_op.as_ref().map(|l| l.op.as_str()), Some("io.import"), "adjustable like any op");
        let result = a.result.clone().unwrap();
        let fix = result.entries[0].warnings.iter().find_map(|w| w.fix.clone()).expect("a warning with a fix");
        assert_eq!(fix.op, "cleanup.auto");
        let k = *a.selection.first().unwrap();
        let faces = a.doc().object_mesh(k).unwrap().face_count();
        results::run_fix(&mut a, OpCall::new(&fix.op, fix.args)).unwrap();
        assert_eq!(a.doc().object_mesh(k).unwrap().face_count(), faces + 1, "the fix closed the hole");
        // Dropping files: models import, other files are refused.
        std::fs::write(dir.join("notes.txt"), "hello").unwrap();
        a.drop_files(&[dir.join("notes.txt")]);
        assert!(a.status.as_ref().is_some_and(|(s, _)| s.contains("Cannot use")));
        a.drop_files(std::slice::from_ref(&bin));
        finish_jobs(&mut a);
        assert_eq!(a.doc().object_count(), 2);
        // A folder nobody picked stays closed, through the dialog-free path too.
        let other = files::tests::temp_dir("import-other");
        std::fs::write(other.join("bin.obj"), obj).unwrap();
        a.run("file.import", json!({ "path": other.join("bin.obj").display().to_string() })).unwrap();
        finish_jobs(&mut a);
        assert!(a.status.as_ref().is_some_and(|(s, _)| s.contains("outside the allowed folders")), "{:?}", a.status);
        assert_eq!(a.doc().object_count(), 2);
        for d in [dir, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// A box with a thin cylinder through it; the cylinder is selected
    /// first, the box last (active).
    fn crate_and_drill() -> (PixelApp, ObjectKey, ObjectKey) {
        let mut a = app();
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        a.run("prim.cylinder", json!({ "name": "drill", "radius": 0.2, "height": 2.0 })).unwrap();
        let (c, d) = (a.doc().resolve("crate").unwrap(), a.doc().resolve("drill").unwrap());
        a.selection = [c, d].into_iter().collect();
        a.active = Some(c);
        (a, c, d)
    }

    #[test]
    fn booleans_cut_the_active_mesh_with_the_other_selected_ones() {
        let (mut a, c, d) = crate_and_drill();
        a.selection = [d].into_iter().collect();
        let refused = a.run("selection.boolean", json!({ "operation": "difference" })).unwrap_err();
        assert!(refused.contains("two or more meshes"), "{refused}");
        a.selection = [c, d].into_iter().collect();
        a.active = Some(c);
        let steps = a.session.history().len();
        a.run("selection.boolean", json!({ "operation": "difference" })).unwrap();
        assert_eq!((a.doc().object_count(), a.session.history().len()), (1, steps + 1), "the drill is used up, one undo step");
        assert_eq!(a.selection, [c].into_iter().collect(), "the crate stays selected");
        // Adjust Last Operation: keep the drill; still one undo step.
        let mut params = a.last_op.as_ref().map(|l| l.params.clone()).unwrap();
        assert_eq!(a.last_op.as_ref().map(|l| l.op.as_str()), Some("edit.boolean"));
        params["keep_tool"] = json!(true);
        quick::adjust(&mut a, params).unwrap();
        assert_eq!((a.doc().object_count(), a.session.history().len()), (2, steps + 1));
        // Tab: Edit Mode starts with the faces the cut made.
        a.run("ui.edit", Value::Null).unwrap();
        assert_eq!(a.ui.mode, SelectMode::Face);
        let mesh = a.doc().object_mesh(c).unwrap();
        let (name, cut) = mesh.groups().find(|(n, _)| n.ends_with(".cut")).map(|(n, g)| (n.to_owned(), g.iter().collect::<BTreeSet<u32>>())).unwrap();
        assert!(!cut.is_empty());
        assert_eq!(a.edit.picked(), (&cut, Some(SelectMode::Face)));
        // The group can be picked again by name (the Properties group list).
        a.run("ui.mode", json!({ "mode": "object" })).unwrap();
        a.run("edit.select_group", json!({ "target": "crate", "group": name })).unwrap();
        assert_eq!(a.edit.picked().0, &cut);
        // Without a target the op itself takes the selection too (palette, command line).
        a.run("ui.mode", json!({ "mode": "object" })).unwrap();
        a.selection = [c, d].into_iter().collect();
        a.active = Some(c);
        a.run("edit.boolean", json!({ "operation": "union" })).unwrap();
        assert_eq!(a.doc().object_count(), 1);
    }

    #[test]
    fn boolean_keys_run_on_the_selection() {
        let ctx = egui::Context::default();
        let press = |a: &mut PixelApp, key: egui::Key, modifiers: egui::Modifiers| {
            let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            ctx.begin_pass(egui::RawInput { events: vec![ev], ..Default::default() });
            chrome::shortcuts(a, &ctx);
            ctx.end_pass().textures_delta.clear();
        };
        let cmd = egui::Modifiers::COMMAND;
        for (key, modifiers, op) in [
            (egui::Key::Minus, cmd, "difference"),
            (egui::Key::Plus, cmd, "union"),
            (egui::Key::Equals, cmd, "union"),
            (egui::Key::Num8, cmd | egui::Modifiers::SHIFT, "intersect"),
        ] {
            let (mut a, _, _) = crate_and_drill();
            press(&mut a, key, modifiers);
            let ran = a.last_op.as_ref().map(|l| (l.op.clone(), l.params["operation"].clone()));
            assert_eq!(ran, Some(("edit.boolean".to_owned(), json!(op))), "{key:?}");
            assert_eq!(a.doc().object_count(), 1);
        }
    }

    #[test]
    fn view_and_ui_commands_validate_their_parameters() {
        let mut a = app();
        assert!(a.run("view.shading", json!({ "mode": "toon" })).is_err());
        a.run("view.shading", json!({ "mode": "wire" })).unwrap();
        assert_eq!(a.ui.view.shading(), p3d_render::Shading::Wire);
        a.run("view.grid", Value::Null).unwrap();
        assert!(!a.ui.view.grid);
        a.run("ui.theme", json!({ "theme": "light" })).unwrap();
        assert!(a.run("ui.theme", json!({ "theme": "purple" })).is_err());
        a.run("ui.tool", json!({ "tool": "rotate" })).unwrap();
        assert_eq!(a.ui.tool, Tool::Rotate);
        // Preferences survive a restart; the start page choice does not.
        let mut b = app();
        b.load_prefs(&a.prefs_json());
        assert_eq!(b.ui.theme, theme::ThemePref::Light);
        assert!(b.ui.start_page);
        b.load_prefs("not json");
        assert_eq!(b.ui.theme, theme::ThemePref::Light);
    }
}
