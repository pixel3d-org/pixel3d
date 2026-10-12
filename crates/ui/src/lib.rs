//! Desktop user interface built on egui.
//!
//! Layer L5 (frontends). See `docs/architecture.md`.
//!
//! A thin shell over [`p3d_engine::Session`] built with cadcraft's
//! techniques (project spec 20): panels read the session and change things
//! only through ops and `ui.*` commands ([`PixelApp::run`]), so undo, MCP
//! and the control server see the same actions. Nothing here owns document
//! data.

pub mod chrome;
pub mod cmdline;
pub mod commands;
pub mod control;
pub mod docking;
pub mod health;
pub mod i18n;
mod icon_data;
pub mod icons;
pub mod inspector;
pub mod menus;
pub mod navgizmo;
pub mod outliner;
pub mod palette;
pub mod panels;
pub mod schema_form;
pub mod screenshot;
pub mod stats;
pub mod theme;
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

    /// Why the workspace cannot be used yet, if it cannot.
    pub fn unavailable(self) -> Option<&'static str> {
        match self {
            WorkspaceKind::Model => None,
            WorkspaceKind::Uv => Some(t("Needs the UV editor, which is not built yet.")),
            WorkspaceKind::Bake => Some(t("Needs baking, which is not built yet.")),
            WorkspaceKind::Rig => Some(t("Needs rigging, which is not built yet.")),
            WorkspaceKind::Animate => Some(t("Needs animation, which is not built yet.")),
            WorkspaceKind::Export => Some(t("Needs the exporters, which are not built yet.")),
        }
    }
}

/// Persisted UI state (serde, camelCase, so automation can read and set it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub docking: docking::Workspace,
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
    /// Recently opened or saved files, newest first.
    pub recent: Vec<String>,
}

/// Recent files kept.
const RECENT: usize = 10;

impl Default for UiState {
    fn default() -> Self {
        UiState {
            docking: docking::Workspace::default(),
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
            recent: Vec::new(),
        }
    }
}

impl UiState {
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
    /// The revision of an edit still being dragged: the next change of the
    /// same drag replaces it, so one drag is one undo step.
    coalesce: Option<u64>,
    pub services: Services,
    /// The file the document was opened from or saved to.
    pub path: Option<PathBuf>,
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
    /// The command palette while open.
    pub palette: Option<palette::Palette>,
    /// The Console's command line.
    pub cmdline: cmdline::CmdLine,
    /// Quit once the pending screenshot is saved (`--screenshot`).
    pub quit_after_shot: bool,
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
            coalesce: None,
            services,
            path: None,
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
            palette: None,
            cmdline: cmdline::CmdLine::default(),
            quit_after_shot: false,
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
    }

    /// Whether the History or Console panel is open.
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

    fn loaded(&mut self, doc: Document, path: Option<PathBuf>) {
        self.session.load(doc);
        self.generation += 1;
        self.saved_revision = self.session.revision();
        self.path = path;
        self.selection.clear();
        self.active = None;
        self.hidden.clear();
        self.viewport.hidden_changed();
        self.last_op = None;
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

    /// Saves to `path`, or to the current file.
    pub fn save_to(&mut self, path: Option<PathBuf>) -> Result<Value, String> {
        let path = match path.or_else(|| self.path.clone()) {
            Some(p) => p,
            None => return Err(t("no file name: use Save As").to_owned()),
        };
        let path = if path.extension().is_none() { path.with_extension("p3d") } else { path };
        p3d_io::p3d::save(self.session.doc(), &path).map_err(|e| format!("{} {}: {e}", t("Cannot save"), path.display()))?;
        self.saved_revision = self.session.revision();
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
                let path = str_param("path").map(PathBuf::from).or_else(|| self.services.pick_open.as_ref().and_then(|f| f()));
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
                    path = self.services.pick_save.as_ref().and_then(|f| f(&suggested));
                    if path.is_none() {
                        return Ok(json!({ "cancelled": true }));
                    }
                }
                self.save_to(path)
            }
            "file.sample" => self.sample(),
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
            "edit.select_all" => {
                self.selection = self.doc().objects().map(|(k, _)| k).collect();
                Ok(json!({ "selected": self.selection.len() }))
            }
            "edit.select_none" => {
                self.selection.clear();
                Ok(json!({ "selected": 0 }))
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
                self.ui.docking = docking::Workspace::default();
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
                if let Some(why) = ws.unavailable() {
                    return Err(format!("{}: {why}", ws.label()));
                }
                self.ui.workspace = ws;
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
            "ui.palette" => Ok(palette::toggle(self, params.get("on").and_then(Value::as_bool))),
            "ui.rail_labels" => {
                self.ui.rail_labels = params.get("on").and_then(Value::as_bool).unwrap_or(!self.ui.rail_labels);
                Ok(json!({ "railLabels": self.ui.rail_labels }))
            }
            "ui.tool" => {
                let tool = serde_json::from_value::<Tool>(params.get("tool").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                self.ui.tool = tool;
                Ok(json!({ "tool": tool }))
            }
            "ui.mode" => {
                let mode = serde_json::from_value::<SelectMode>(params.get("mode").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
                self.ui.mode = mode;
                Ok(json!({ "mode": mode }))
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
    pub fn execute_labeled(&mut self, calls: &[OpCall], label: Option<&str>) -> Result<Value, String> {
        let opts = BatchOptions { label: label.map(str::to_owned), ..BatchOptions::default() };
        match self.session.execute(calls, &opts) {
            Ok(r) => {
                let mut created = Vec::new();
                for res in &r.results {
                    self.log(res.output.summary.clone());
                    created.extend(res.output.created.iter().copied());
                }
                if let Some(last) = r.results.last() {
                    self.status = Some((last.output.summary.clone(), Instant::now()));
                }
                self.last_op = match r.results.as_slice() {
                    [one] => Some(LastOp { op: one.op.clone(), params: one.params.clone(), revision: r.revision, error: None }),
                    _ => None,
                };
                if !created.is_empty() {
                    self.active = created.first().copied();
                    self.selection = created.into_iter().collect();
                }
                self.ui.start_page = false;
                Ok(json!({ "revision": r.revision, "summaries": r.results.iter().map(|x| x.output.summary.clone()).collect::<Vec<_>>() }))
            }
            Err(e) => Err(e.error.to_string()),
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
            }
        }
    }

    // ---- frame ------------------------------------------------------------------------------

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
        self.shots.tick(ctx);
        if self.quit_after_shot
            && let Some(done) = &self.shots.done
        {
            if let Err(e) = done {
                tracing::error!(error = %e, "screenshot failed");
            }
            self.quit_requested = true;
        }
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let p = f.path().to_path_buf();
            if !p.as_os_str().is_empty() {
                let _ = self.open_path(&p);
            }
        }
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
        self.frame_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn dock(&mut self, ui: &mut egui::Ui) {
        let workspace = self.ui.docking.clone();
        let mut viewport_events = Vec::new();
        let mut nav = None;
        let actions = docking::show(ui, &workspace, |ui, panel| match panel {
            docking::Panel::Viewport => {
                let doc = std::sync::Arc::clone(self.session.doc());
                let key = self.scene_key();
                let style = theme::Tokens::get().view_style();
                let sel = self.selection.clone();
                self.viewport.scene(&doc, key, &self.hidden);
                self.viewport.set_selection(&sel);
                let view_rect = chrome::viewport_header(self, ui, ui.max_rect());
                viewport_events = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(view_rect), |ui| {
                        self.viewport.show(ui, &doc, key, &self.hidden, &self.ui.view, style)
                    })
                    .inner;
                chrome::viewport_overlay(self, ui);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PixelApp {
        PixelApp::new(Session::new(), Services::default())
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
