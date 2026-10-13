//! The Strands panel: hair, fur, eyebrows, beards and grass grown on the
//! selected mesh with `gen.strands`. Pick a kind (its preset sets the rest),
//! then the region: all of it, face groups, a loop of the mesh's `lm.*`
//! landmarks (clicked in order on their dots, or typed), or a loop of
//! points clicked on the surface (object space). The op's other parameters
//! come from its schema through the shared form. Grow runs the op through
//! the normal path; while that is the last step, edits grow it again in its
//! place (one undo step). The summary's coverage and largest bald spot
//! show in the panel, and the bald spot is marked in the viewport.

use std::collections::BTreeSet;

use egui::{Pos2, RichText, Stroke, pos2, vec2};
use glam::{Affine3A, Mat4, Vec3};
use p3d_engine::OpCall;
use p3d_scene::{Document, ObjectKey};
use serde_json::{Map, Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

/// The kinds, as the op names them, with their labels.
pub const KINDS: [(&str, &str); 5] = [("hair", "Hair"), ("fur", "Fur"), ("eyebrows", "Eyebrows"), ("beard", "Beard"), ("grass", "Grass")];

/// Where strands grow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    All,
    Groups,
    Landmarks,
    Points,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::All => "all",
            Mode::Groups => "groups",
            Mode::Landmarks => "landmarks",
            Mode::Points => "points",
        }
    }

    fn parse(s: &str) -> Option<Mode> {
        [Mode::All, Mode::Groups, Mode::Landmarks, Mode::Points].into_iter().find(|m| m.name() == s)
    }
}

/// What the last grow reported.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Covered share of the region (0 to 1).
    pub coverage: Option<f64>,
    /// The largest bald spot: its center in the object's space and its
    /// area (m2).
    pub bald: Option<(Vec3, f64)>,
}

/// Reads coverage and the bald spot from a `gen.strands` summary
/// (`coverage 87% (largest bald spot 12 cm2 at (10, 20, 30) mm)`).
///
/// Complexity: O(summary length).
pub fn parse_report(summary: &str) -> Report {
    let after = |key: &str| summary.find(key).map(|i| &summary[i + key.len()..]);
    let coverage = after("coverage ").and_then(|s| s.split('%').next()).and_then(|n| n.trim().parse::<f64>().ok()).map(|p| p / 100.0);
    let bald = after("largest bald spot ").and_then(|s| {
        let area = s.split(" cm2").next()?.trim().parse::<f64>().ok()? / 1e4;
        let coords = s.split("at (").nth(1)?.split(')').next()?;
        let v: Vec<f32> = coords.split(',').filter_map(|c| c.trim().parse::<f32>().ok()).collect();
        (v.len() == 3).then(|| (Vec3::new(v[0], v[1], v[2]) / 1000.0, area))
    });
    Report { coverage, bald }
}

/// The panel between frames.
pub struct StrandsTool {
    pub kind: &'static str,
    pub mode: Mode,
    pub groups: BTreeSet<String>,
    /// The landmark loop, by name (without `lm.`).
    pub landmarks: Vec<String>,
    /// The point loop, in the object's space.
    pub points: Vec<Vec3>,
    /// Clicks in the viewport add to the loop.
    pub picking: bool,
    /// The op's other parameters as the user set them (the preset fills
    /// the rest).
    pub params: Map<String, Value>,
    /// The last grow (its revision), the object it grew on, and its report.
    pub grown: Option<u64>,
    pub on: Option<ObjectKey>,
    pub report: Option<Report>,
    pub error: Option<String>,
    /// Loop points and the bald spot on screen last frame (for agents).
    pub screen: Vec<(String, Pos2)>,
}

impl Default for StrandsTool {
    fn default() -> Self {
        StrandsTool {
            kind: "hair",
            mode: Mode::All,
            groups: BTreeSet::new(),
            landmarks: Vec::new(),
            points: Vec::new(),
            picking: false,
            params: Map::new(),
            grown: None,
            on: None,
            report: None,
            error: None,
            screen: Vec::new(),
        }
    }
}

/// Whether an object is strands (tagged by `gen.strands`).
pub fn is_strands(doc: &Document, k: ObjectKey) -> bool {
    doc.object(k).is_some_and(|o| o.tags.iter().any(|t| t == "strands"))
}

/// What a strands object is, from the op that grew it: `hair (cards)`.
///
/// Complexity: O(history length).
pub fn describe(doc: &Document, k: ObjectKey) -> Option<String> {
    let n = doc.history(k)?.nodes.iter().rev().find(|n| n.op == "gen.strands")?;
    let kind = n.params.get("kind").and_then(Value::as_str).unwrap_or("hair");
    Some(match n.params.get("output").and_then(Value::as_str) {
        Some(out) => format!("{kind} {} ({out})", t("strands")),
        None => format!("{kind} {}", t("strands")),
    })
}

/// The `lm.*` landmarks of a mesh object, in world space.
///
/// Complexity: O(groups).
fn landmarks_of(doc: &Document, k: ObjectKey) -> Vec<(String, Vec3)> {
    let (Some(mesh), Some(w)) = (doc.object_mesh(k), doc.world_affine(k)) else { return Vec::new() };
    p3d_features::baked_landmarks(mesh).into_iter().map(|(n, p)| (n, w.transform_point3(p))).collect()
}

impl PixelApp {
    /// The mesh strands grow on: the active mesh (a strands object stands
    /// for the object it grew on).
    pub(crate) fn strands_target(&self) -> Option<ObjectKey> {
        let doc = self.doc();
        let k = self.active_object()?;
        let k = if is_strands(doc, k) { doc.object(k)?.parent()? } else { k };
        doc.object_mesh(k).map(|_| k)
    }

    /// The region as `gen.strands` takes it.
    fn strands_region(&self) -> Result<Value, String> {
        let s = &self.strands;
        match s.mode {
            // Left out: the op's default is the whole surface (it reads `"all"`
            // as no region at all).
            Mode::All => Ok(Value::Null),
            Mode::Groups if s.groups.is_empty() => Err(t("pick at least one face group").to_owned()),
            Mode::Groups => Ok(json!({ "groups": s.groups.iter().collect::<Vec<_>>() })),
            Mode::Landmarks if s.landmarks.len() < 3 => Err(t("a landmark loop needs three or more landmarks").to_owned()),
            Mode::Landmarks => Ok(json!(s.landmarks)),
            Mode::Points if s.points.len() < 3 => Err(t("a point loop needs three or more points").to_owned()),
            Mode::Points => Ok(json!(s.points.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>())),
        }
    }

    /// The op call: kind, region and the set parameters.
    fn strands_call(&self) -> Result<OpCall, String> {
        let on = self.strands_target().ok_or(t("select a mesh to grow strands on"))?;
        let id = self.doc().object(on).map(|o| o.public_id()).ok_or(t("the mesh is gone"))?;
        let mut args = self.strands.params.clone();
        args.retain(|_, v| !v.is_null());
        args.insert("on".into(), json!(id));
        args.insert("kind".into(), json!(self.strands.kind));
        let region = self.strands_region()?;
        if !region.is_null() {
            args.insert("region".into(), region);
        }
        Ok(OpCall::new("gen.strands", Value::Object(args)))
    }

    /// The last grow's summary, read for its report.
    fn strands_take_report(&mut self) {
        let summary = self.result.as_ref().and_then(|r| r.entries.first()).filter(|e| e.op == "gen.strands").map(|e| e.summary.clone());
        self.strands.report = summary.as_deref().map(parse_report);
    }

    /// Grows strands on the selected mesh (one undo step).
    pub fn strands_grow(&mut self) -> Result<Value, String> {
        let call = self.strands_call().inspect_err(|e| self.strands.error = Some(e.clone()))?;
        let on = self.strands_target();
        match self.execute(&[call]) {
            Ok(r) => {
                self.strands.grown = Some(self.session.revision());
                self.strands.on = on;
                self.strands.error = None;
                self.strands.picking = false;
                self.strands_take_report();
                Ok(r)
            }
            Err(e) => {
                self.strands.error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Something changed: while the last grow is the last step, it grows
    /// again with the new settings in its place.
    fn strands_changed(&mut self) {
        let live = self.strands.grown == Some(self.session.revision())
            && self.last_op.as_ref().is_some_and(|l| l.op == "gen.strands" && l.revision == self.session.revision());
        if !live {
            self.strands.grown = None;
            return;
        }
        // The grow selected its strands object: grow on what it grew on.
        let Some(on) = self.strands.on.and_then(|k| self.doc().object(k).map(|o| o.public_id())) else { return };
        let mut args = self.strands.params.clone();
        args.retain(|_, v| !v.is_null());
        args.insert("on".into(), json!(on));
        args.insert("kind".into(), json!(self.strands.kind));
        match self.strands_region() {
            Ok(r) if r.is_null() => {}
            Ok(r) => {
                args.insert("region".into(), r);
            }
            Err(e) => {
                self.strands.error = Some(e);
                return;
            }
        }
        match self.adjust_last(Value::Object(args)) {
            Ok(()) => {
                self.strands.grown = Some(self.session.revision());
                self.strands.error = None;
                self.strands_take_report();
            }
            Err(e) => self.strands.error = Some(e),
        }
    }

    /// Opens the panel on the selected mesh with a kind.
    pub fn strands_start(&mut self, kind: Option<&str>) -> Result<Value, String> {
        if let Some(k) = kind {
            self.strands.kind = KINDS.iter().find(|(id, _)| *id == k).map(|(id, _)| *id).ok_or_else(|| format!("{} `{k}`", t("no strands kind")))?;
            self.strands.params.clear();
        }
        self.ui.docking.set_visible(crate::docking::Panel::Strands, true)?;
        self.strands.grown = None;
        Ok(json!({ "kind": self.strands.kind, "on": self.strands_target().and_then(|k| self.doc().object(k)).map(|o| o.public_id()) }))
    }

    /// Adds a clicked point to the loop: a landmark dot (landmark mode) or
    /// the surface under `at` (point mode).
    fn strands_click(&mut self, at: Pos2) {
        let Some(on) = self.strands_target() else { return };
        let rect = self.viewport.rect;
        let vp = self.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
        match self.strands.mode {
            Mode::Landmarks => {
                let near = landmarks_of(self.doc(), on)
                    .into_iter()
                    .filter_map(|(n, p)| project(vp, rect, p).map(|s| (n, s.distance(at))))
                    .filter(|(_, d)| *d <= 10.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((n, _)) = near
                    && self.strands.landmarks.last() != Some(&n)
                {
                    self.strands.landmarks.push(n);
                }
            }
            Mode::Points => {
                if let Some((k, _, p)) = self.viewport.hit_at(at)
                    && k == on
                {
                    let local = self.doc().world_affine(on).unwrap_or(Affine3A::IDENTITY).inverse().transform_point3(p);
                    self.strands.points.push(local);
                }
            }
            _ => {}
        }
    }

    /// Runs a `strands.*` command.
    pub(crate) fn strands_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let s = |k: &str| params.get(k).and_then(Value::as_str);
        match id {
            "strands.open" => {
                let shown = self.ui.docking.is_visible(crate::docking::Panel::Strands);
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!shown);
                self.ui.docking.set_visible(crate::docking::Panel::Strands, on)?;
                Ok(json!({ "open": on }))
            }
            "strands.start" => self.strands_start(s("kind")),
            "strands.kind" => {
                let k = s("kind").ok_or(t("kind is hair, fur, eyebrows, beard or grass"))?;
                self.strands.kind =
                    KINDS.iter().find(|(id, _)| *id == k).map(|(id, _)| *id).ok_or_else(|| format!("{} `{k}`", t("no strands kind")))?;
                self.strands.params.clear();
                self.strands_changed();
                Ok(json!({ "kind": self.strands.kind }))
            }
            "strands.region" => {
                if let Some(m) = s("mode") {
                    self.strands.mode = Mode::parse(m).ok_or(t("mode is all, groups, landmarks or points"))?;
                }
                if let Some(g) = params.get("groups").and_then(Value::as_array) {
                    self.strands.groups = g.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                    self.strands.mode = Mode::Groups;
                }
                match params.get("landmarks") {
                    Some(Value::String(text)) => {
                        self.strands.landmarks = text.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).collect();
                        self.strands.mode = Mode::Landmarks;
                    }
                    Some(Value::Array(names)) => {
                        self.strands.landmarks = names.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                        self.strands.mode = Mode::Landmarks;
                    }
                    _ => {}
                }
                if let Some(pts) = params.get("points").and_then(Value::as_array) {
                    self.strands.points = pts
                        .iter()
                        .filter_map(|p| {
                            let a = p.as_array()?;
                            Some(Vec3::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32))
                        })
                        .collect();
                    self.strands.mode = Mode::Points;
                }
                self.strands_changed();
                Ok(json!({ "mode": self.strands.mode.name(), "region": self.strands_region().unwrap_or(Value::Null) }))
            }
            "strands.pick" => {
                self.strands.picking = params.get("on").and_then(Value::as_bool).unwrap_or(!self.strands.picking);
                Ok(json!({ "picking": self.strands.picking }))
            }
            "strands.clear_loop" => {
                self.strands.landmarks.clear();
                self.strands.points.clear();
                Ok(json!({ "cleared": true }))
            }
            "strands.set" => {
                let values = params.get("params").unwrap_or(params).as_object().ok_or(t("give the parameters as an object"))?;
                for (k, v) in values {
                    if ["on", "kind", "region"].contains(&k.as_str()) {
                        return Err(format!("`{k}` {}", t("is set with its own command")));
                    }
                    self.strands.params.insert(k.clone(), v.clone());
                }
                self.strands_changed();
                Ok(Value::Object(self.strands.params.clone()))
            }
            "strands.grow" => self.strands_grow(),
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// The panel's state for agents (`ui.inspect`).
    pub(crate) fn strands_state(&self) -> Value {
        let s = &self.strands;
        let doc = self.doc();
        json!({
            "open": self.ui.docking.is_visible(crate::docking::Panel::Strands),
            "on": self.strands_target().and_then(|k| doc.object(k)).map(|o| o.public_id()),
            "kind": s.kind,
            "mode": s.mode.name(),
            "region": self.strands_region().ok(),
            "picking": s.picking,
            "params": s.params,
            "live": s.grown == Some(self.session.revision()),
            "coverage": s.report.as_ref().and_then(|r| r.coverage),
            "baldSpot": s.report.as_ref().and_then(|r| r.bald).map(|(p, a)| json!({ "at": p.to_array(), "area": a })),
            "onScreen": s.screen.iter().map(|(n, at)| json!({ "name": n, "at": [at.x, at.y] })).collect::<Vec<_>>(),
            "error": s.error,
        })
    }
}

/// The Strands panel body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let Some(on) = app.strands_target() else {
        ui.label(RichText::new(t("Select a mesh to grow hair, fur, eyebrows, beards or grass on.")).color(tk.text_dim));
        return;
    };
    let doc = std::sync::Arc::clone(app.session.doc());
    let name = doc.object(on).map(|o| o.name().to_owned()).unwrap_or_default();
    let mut changed = false;
    egui::ScrollArea::vertical().id_salt("strands-panel").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(t("On")).color(tk.text_dim));
            ui.label(RichText::new(&name).strong().color(tk.text));
        });
        // The kind: its preset sets everything else.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (id, label) in KINDS {
                let on_kind = app.strands.kind == id;
                let r = ui.selectable_label(on_kind, t(label));
                crate::control::record_widget(ui, &r, t(label), "tab", Some(on_kind));
                if r.clicked() && !on_kind {
                    app.strands.kind = id;
                    app.strands.params.clear();
                    changed = true;
                }
            }
        });
        ui.add_space(4.0);
        // The region.
        ui.label(RichText::new(t("Where")).color(tk.text_dim));
        ui.horizontal_wrapped(|ui| {
            for (m, label) in
                [(Mode::All, t("All")), (Mode::Groups, t("Face groups")), (Mode::Landmarks, t("Landmark loop")), (Mode::Points, t("Point loop"))]
            {
                let r = ui.radio(app.strands.mode == m, label);
                crate::control::record_widget(ui, &r, label, "radio", Some(app.strands.mode == m));
                if r.clicked() && app.strands.mode != m {
                    app.strands.mode = m;
                    app.strands.picking = matches!(m, Mode::Landmarks | Mode::Points);
                    changed = true;
                }
            }
        });
        match app.strands.mode {
            Mode::All => {}
            Mode::Groups => {
                let groups: Vec<String> = doc
                    .object_mesh(on)
                    .map(|m| m.groups().filter(|(_, g)| g.kind() == p3d_mesh::ElementKind::Face).map(|(n, _)| n.to_owned()).collect())
                    .unwrap_or_default();
                if groups.is_empty() {
                    ui.label(RichText::new(t("This mesh has no face groups.")).font(theme::small()).color(tk.text_faint));
                }
                for g in groups {
                    let mut on_g = app.strands.groups.contains(&g);
                    if ui.checkbox(&mut on_g, &g).changed() {
                        if on_g {
                            app.strands.groups.insert(g);
                        } else {
                            app.strands.groups.remove(&g);
                        }
                        changed = true;
                    }
                }
            }
            Mode::Landmarks => {
                let mut text = app.strands.landmarks.join(", ");
                let r =
                    ui.add(egui::TextEdit::singleline(&mut text).hint_text("hairline_front, temple_l, nape, temple_r").desired_width(f32::INFINITY));
                crate::control::record_widget(ui, &r, t("Landmark loop"), "textbox", None);
                if r.changed() {
                    app.strands.landmarks = text.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).collect();
                }
                if r.lost_focus() {
                    changed = true;
                }
                loop_buttons(app, ui, &mut changed);
            }
            Mode::Points => {
                ui.label(
                    RichText::new(format!("{} {}", app.strands.points.len(), t("points (object space)"))).font(theme::small()).color(tk.text_dim),
                );
                loop_buttons(app, ui, &mut changed);
            }
        }
        ui.add_space(6.0);
        let grow = ui.add_sized([ui.available_width(), 26.0], egui::Button::new(RichText::new(t("Grow")).color(tk.on_accent)).fill(tk.accent));
        crate::control::record_widget(ui, &grow, t("Grow strands"), "button", None);
        if grow.on_hover_text(t("Grow the strands as a new object under the mesh (one undo step); then edits regrow them")).clicked()
            && let Err(e) = app.strands_grow()
        {
            app.set_status(e);
        }
        // What the grow reported.
        if let Some(r) = app.strands.report.clone() {
            let low = r.coverage.is_some_and(|c| c < 0.95);
            let mut line = r.coverage.map_or_else(String::new, |c| format!("{} {:.0}%", t("Coverage"), c * 100.0));
            if let Some((_, area)) = r.bald {
                line.push_str(&format!("; {} {:.0} cm2 ({})", t("largest bald spot"), area * 1e4, t("marked in the viewport")));
            }
            if !line.is_empty() {
                ui.add(egui::Label::new(RichText::new(line).font(theme::small()).color(if low { tk.health_warn } else { tk.health_ok })).wrap());
            }
            if low {
                ui.add(egui::Label::new(RichText::new(t("Raise clumps or width to cover it.")).font(theme::small()).color(tk.text_faint)).wrap());
            }
        }
        let live = app.strands.grown == Some(app.session.revision());
        let note = if live { t("Editing now regrows the strands in place.") } else { t("Grow adds them as a child of the mesh.") };
        ui.add(egui::Label::new(RichText::new(note).font(theme::small()).color(tk.text_faint)).wrap());
        if let Some(e) = &app.strands.error {
            ui.add(egui::Label::new(RichText::new(e).font(theme::small()).color(tk.health_error)).wrap());
        }
        ui.add_space(6.0);
        // The op's parameters from its schema (auto means the preset's).
        let registry = app.session.registry();
        let Ok(op) = registry.get("gen.strands") else { return };
        let schema = op.params_schema();
        let mut v = Value::Object(app.strands.params.clone());
        let hide = ["on", "kind", "region", "name", "inside", "anchor"];
        let o = crate::schema_form::form(ui, egui::Id::new("strands-params"), &schema, &schema, &mut v, &hide);
        if o.changed
            && let Value::Object(mut m) = v
        {
            m.retain(|_, x| !x.is_null());
            app.strands.params = m;
            changed = true;
        }
        // Inside (a point of the patch) and the ponytail's anchor: a landmark
        // name or coordinates.
        for (key, label, tip) in [
            ("inside", t("Inside"), t("A landmark inside a loop's patch, when it is not the smaller side")),
            ("anchor", t("Anchor"), t("Where a ponytail gathers: a landmark name")),
        ] {
            let mut text = app.strands.params.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
            let r = crate::widgets::prop_row(ui, label, tip, |ui| ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width())));
            if r.lost_focus() {
                if text.trim().is_empty() {
                    app.strands.params.remove(key);
                } else {
                    app.strands.params.insert(key.to_owned(), json!(text.trim()));
                }
                changed = true;
            }
        }
    });
    if changed {
        app.strands_changed();
    }
}

/// Pick, undo the last point, clear.
fn loop_buttons(app: &mut PixelApp, ui: &mut egui::Ui, changed: &mut bool) {
    ui.horizontal(|ui| {
        let r = ui.selectable_label(app.strands.picking, t("Pick in the viewport"));
        crate::control::record_widget(ui, &r, t("Pick in the viewport"), "button", Some(app.strands.picking));
        if r.on_hover_text(t("Click landmark dots (or the surface for points) in order around the patch")).clicked() {
            app.strands.picking = !app.strands.picking;
        }
        if ui.small_button(t("Undo point")).clicked() {
            match app.strands.mode {
                Mode::Landmarks => {
                    app.strands.landmarks.pop();
                }
                _ => {
                    app.strands.points.pop();
                }
            }
            *changed = true;
        }
        if ui.small_button(t("Clear")).clicked() {
            app.strands.landmarks.clear();
            app.strands.points.clear();
            *changed = true;
        }
    });
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

/// The loop being picked (landmark dots to click, the loop so far) and the
/// last grow's bald spot, in the viewport. While picking, clicks add to
/// the loop instead of selecting.
///
/// Complexity: O(landmarks + loop points) per frame.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui) {
    app.strands.screen.clear();
    let rect = app.viewport.rect;
    let open = app.ui.docking.is_visible(crate::docking::Panel::Strands);
    if !open || app.ui.mode.is_edit() || !rect.is_positive() {
        return;
    }
    let Some(on) = app.strands_target() else { return };
    let tk = Tokens::get();
    let doc = std::sync::Arc::clone(app.session.doc());
    let world = doc.world_affine(on).unwrap_or(Affine3A::IDENTITY);
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let p = ui.painter_at(rect);
    let picking = app.strands.picking && matches!(app.strands.mode, Mode::Landmarks | Mode::Points);
    // The landmark dots to click.
    let marks = landmarks_of(&doc, on);
    if picking && app.strands.mode == Mode::Landmarks {
        for (n, at) in &marks {
            let Some(s) = project(vp, rect, *at) else { continue };
            let used = app.strands.landmarks.contains(n);
            p.circle(s, 4.0, if used { tk.accent } else { tk.icon_accent }, Stroke::new(1.0, egui::Color32::from_black_alpha(160)));
            p.text(s + vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, n, theme::small(), tk.text);
            app.strands.screen.push((n.clone(), s));
        }
    }
    // The loop so far, closed and numbered.
    let pts: Vec<Vec3> = match app.strands.mode {
        Mode::Landmarks => app.strands.landmarks.iter().filter_map(|n| marks.iter().find(|(m, _)| m == n).map(|(_, p)| *p)).collect(),
        Mode::Points => app.strands.points.iter().map(|q| world.transform_point3(*q)).collect(),
        _ => Vec::new(),
    };
    let screen: Vec<Pos2> = pts.iter().filter_map(|q| project(vp, rect, *q)).collect();
    if screen.len() > 1 {
        let mut line = screen.clone();
        if screen.len() > 2 {
            line.push(screen[0]);
        }
        p.add(egui::Shape::line(line, Stroke::new(1.6, tk.accent)));
    }
    for (i, s) in screen.iter().enumerate() {
        p.circle_filled(*s, 3.0, tk.accent);
        p.text(*s + vec2(5.0, 5.0), egui::Align2::LEFT_TOP, format!("{}", i + 1), theme::small(), tk.accent);
    }
    // The bald spot of the last grow.
    if let (Some(r), Some(grown_on)) = (&app.strands.report, app.strands.on)
        && let Some((at, area)) = r.bald
        && let Some(w) = doc.world_affine(grown_on)
    {
        let c = w.transform_point3(at);
        let radius = (area / std::f64::consts::PI).sqrt() as f32;
        let right = app.viewport.camera.ray(rect.width(), rect.height() / 2.0, rect.width(), rect.height()).1;
        if let (Some(s), Some(e)) = (project(vp, rect, c), project(vp, rect, c + right.cross(Vec3::Y).normalize_or(Vec3::X) * radius)) {
            let rad = s.distance(e).max(6.0);
            p.circle_stroke(s, rad, Stroke::new(2.0, tk.health_error));
            p.text(
                s + vec2(rad + 4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{} {:.0} cm2", t("bald spot"), area * 1e4),
                theme::small(),
                tk.health_error,
            );
            app.strands.screen.push(("bald_spot".into(), s));
        }
    }
    // Clicks add to the loop while picking.
    if picking && let Some(at) = app.viewport.tool_input.clicked {
        app.strands_click(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use crate::viewport::ViewportEvent;

    fn head() -> PixelApp {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("gen.feature", json!({ "name": "head", "feature": "head_base", "detail": "5mm" })).unwrap();
        a
    }

    #[test]
    fn reports_read_coverage_and_the_bald_spot() {
        let r = parse_report(
            "created head/hair (o:2) hair 0.2 x 0.2 x 0.2 m: 1 faces; 40 cards, 900 tris, region 310 cm2, coverage 87% (largest bald spot 12 cm2 at (10, -20, 30) mm)",
        );
        assert_eq!(r.coverage, Some(0.87));
        let (at, area) = r.bald.unwrap();
        assert!(at.distance(Vec3::new(0.01, -0.02, 0.03)) < 1e-6 && (area - 0.0012).abs() < 1e-9);
        let full = parse_report("30 cards, 240 tris, region 3600 cm2, coverage 100%");
        assert_eq!((full.coverage, full.bald), (Some(1.0), None));
    }

    #[test]
    fn grow_hair_in_a_landmark_loop_then_edit_in_place() {
        let mut a = head();
        a.run("strands.start", json!({ "kind": "hair" })).unwrap();
        a.run("strands.region", json!({ "landmarks": "hairline_front, temple_l, nape, temple_r" })).unwrap();
        let steps = a.session.history().len();
        a.run("strands.grow", Value::Null).unwrap();
        assert_eq!(a.session.history().len(), steps + 1);
        let hair = a.doc().resolve("head/hair").unwrap();
        assert!(is_strands(a.doc(), hair));
        assert_eq!(describe(a.doc(), hair).as_deref(), Some("hair strands"));
        assert!(a.strands.report.as_ref().and_then(|r| r.coverage).is_some(), "{:?}", a.strands.report);
        // The grown hair is selected; the panel still grows on the head.
        assert_eq!(a.strands_target(), a.doc().resolve("head").ok());
        // An edit regrows in place: one step, one hair object.
        let tris = |a: &PixelApp| a.doc().object_mesh(a.doc().resolve("head/hair").unwrap()).map_or(0, |m| m.face_count());
        let before = tris(&a);
        a.run("strands.set", json!({ "clumps": 400 })).unwrap();
        assert_eq!(a.session.history().len(), steps + 1);
        assert!(tris(&a) > before, "{before} -> {}", tris(&a));
        assert_eq!(a.strands_state()["live"], json!(true));
        let args = &a.last_op.as_ref().unwrap().params;
        assert_eq!(args["region"], json!(["hairline_front", "temple_l", "nape", "temple_r"]));
    }

    #[test]
    fn loops_by_clicking_landmark_dots_and_the_surface() {
        let mut a = head();
        let doc = std::sync::Arc::clone(a.session.doc());
        a.viewport.scene(&doc, a.scene_key(), &BTreeSet::new());
        a.viewport.rect = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0));
        a.viewport.camera.set_view(p3d_render::ViewKind::Top);
        a.viewport.camera.frame(Vec3::splat(-0.15), Vec3::new(0.15, 0.3, 0.15), 1.0);
        let k = doc.resolve("head").unwrap();
        a.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
        a.run("strands.region", json!({ "mode": "landmarks" })).unwrap();
        let vp = a.viewport.camera.view_proj(1.0);
        for name in ["hairline_front", "temple_l", "nape"] {
            let at = landmarks_of(&doc, k).into_iter().find(|(n, _)| n == name).and_then(|(_, p)| project(vp, a.viewport.rect, p)).unwrap();
            a.strands_click(at);
        }
        assert_eq!(a.strands.landmarks, ["hairline_front", "temple_l", "nape"]);
        // Points: clicks on the surface, kept in the head's space.
        a.run("strands.region", json!({ "mode": "points" })).unwrap();
        a.strands_click(pos2(200.0, 200.0));
        assert_eq!(a.strands.points.len(), 1, "the top of the head is under the center");
        a.strands_click(pos2(2.0, 2.0));
        assert_eq!(a.strands.points.len(), 1, "a miss adds nothing");
        assert!(a.run("strands.grow", Value::Null).is_err(), "three points are needed");
    }

    #[test]
    fn groups_and_kinds() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.box", json!({ "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6 })).unwrap();
        a.run("strands.start", json!({ "kind": "grass" })).unwrap();
        a.run("strands.region", json!({ "groups": ["top"] })).unwrap();
        a.run("strands.set", json!({ "clumps": 30 })).unwrap();
        a.run("strands.grow", Value::Null).unwrap();
        let grass = a.doc().resolve("crate/grass").unwrap();
        assert!(is_strands(a.doc(), grass));
        let summary = a.result.as_ref().map(|r| r.entries[0].summary.clone()).unwrap_or_default();
        assert!(a.strands.report.as_ref().and_then(|r| r.coverage).is_some(), "{summary}");
        // All of it: the op's default region.
        a.run("strands.region", json!({ "mode": "all" })).unwrap();
        a.run("strands.kind", json!({ "kind": "fur" })).unwrap();
        assert_eq!(a.last_op.as_ref().unwrap().params["kind"], json!("fur"), "regrown in place as fur");
        assert!(a.run("strands.kind", json!({ "kind": "moss" })).is_err());
    }
}
