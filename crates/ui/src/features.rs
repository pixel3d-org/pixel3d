//! The Features panel: the feature template library (ears, eyes, noses,
//! horns, bolts, rivets, vents, panels, moldings) with search, the selected
//! template's parameters as widgets generated from its `ParamInfo`s (through
//! the shared schema form: lengths and angles as unit drag values within
//! their range, ratios as sliders, counts as whole numbers, choices as
//! combos), its landmarks and triangle budget, and Insert, which runs
//! `gen.feature` through the normal op path at the 3D cursor or under the
//! active object. While the inserted feature is the last operation, editing
//! a parameter runs it again in place of that step (one undo step).
//!
//! Also here: the landmark overlay of feature objects (View > Overlays >
//! Feature landmarks), the `lm.<name>` vertex groups as labelled dots.

use std::sync::Arc;

use egui::{RichText, Sense, Stroke, pos2, vec2};
use glam::{Mat4, Vec3};
use p3d_engine::OpCall;
use p3d_features::{FeatureInfo, ParamInfo, ParamKind};
use p3d_scene::ObjectKey;
use serde_json::{Map, Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::widgets;

/// A feature landmark: its object, name and world point.
pub type Landmark = (ObjectKey, String, Vec3);

/// Where Insert puts the feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Place {
    /// At the 3D cursor, top level.
    #[default]
    Cursor,
    /// A child of the active object, at the cursor.
    Parent,
}

/// The panel between frames.
#[derive(Default)]
pub struct FeatureBrowser {
    pub query: String,
    /// The selected template.
    pub selected: Option<&'static str>,
    /// Its parameter values as `gen.feature` takes them (lengths in meters,
    /// angles in degrees, choices as words).
    pub params: Map<String, Value>,
    pub place: Place,
    /// The last insert (its revision): edits re-run it while it is current.
    pub inserted: Option<u64>,
    pub error: Option<String>,
    /// Feature landmarks of the shown objects, cached per scene state.
    marks: Option<(u64, Arc<Vec<Landmark>>)>,
    /// On-screen landmark positions last frame, for agents.
    pub screen: Vec<(String, egui::Pos2)>,
}

/// The template with id `id`.
pub fn info(id: &str) -> Option<FeatureInfo> {
    p3d_features::get(id).ok().map(|f| f.info())
}

/// A parameter's default as `gen.feature` takes it.
fn default_value(p: &ParamInfo) -> Value {
    match p.kind {
        ParamKind::Choice(words) => json!(words.get(p.default as usize).copied().unwrap_or("")),
        ParamKind::Count => json!(p.default.round() as i64),
        _ => json!(p.default),
    }
}

/// Every default of a template.
pub fn defaults(info: &FeatureInfo) -> Map<String, Value> {
    info.params.iter().map(|p| (p.name.to_owned(), default_value(p))).collect()
}

/// A JSON Schema of a template's parameters for the shared schema form:
/// lengths and angles as unit references with their range, ratios as
/// sliders, counts as integers, choices as enums.
///
/// Complexity: O(parameters).
pub fn schema(info: &FeatureInfo) -> Value {
    let mut props = Map::new();
    for p in info.params {
        let s = match p.kind {
            ParamKind::Length => json!({ "$ref": "#/$defs/Length", "minimum": p.min, "maximum": p.max }),
            ParamKind::Angle => json!({ "$ref": "#/$defs/Angle", "minimum": p.min, "maximum": p.max }),
            ParamKind::Ratio => json!({ "type": "number", "minimum": p.min, "maximum": p.max, "x-slider": true }),
            ParamKind::Count => json!({ "type": "integer", "minimum": p.min, "maximum": p.max }),
            ParamKind::Choice(words) => json!({ "enum": words }),
        };
        let mut s = s;
        s["description"] =
            json!(format!("{} ({} {}; {} {} {} {})", p.doc, t("default"), p.text(p.default), t("from"), p.text(p.min), t("to"), p.text(p.max)));
        if let ParamKind::Choice(_) = p.kind {
            s["description"] = json!(format!("{} ({} {})", p.doc, t("default"), p.text(p.default)));
        }
        s["default"] = default_value(p);
        props.insert(p.name.to_owned(), s);
    }
    json!({ "type": "object", "properties": props })
}

/// Values kept within each parameter's range (counts whole).
pub(crate) fn clamp(info: &FeatureInfo, params: &mut Map<String, Value>) {
    for p in info.params {
        if let (Some(v), false) = (params.get(p.name).and_then(Value::as_f64), matches!(p.kind, ParamKind::Choice(_))) {
            let c = v.clamp(p.min, p.max);
            let c = if p.kind == ParamKind::Count { json!(c.round() as i64) } else { json!(c) };
            params.insert(p.name.to_owned(), c);
        }
    }
}

impl PixelApp {
    /// Selects a template, with its defaults.
    pub fn features_select(&mut self, id: &str) -> Result<Value, String> {
        let i = info(id).ok_or_else(|| format!("{} `{id}`", t("there is no feature")))?;
        self.features.selected = Some(i.id);
        self.features.params = defaults(&i);
        self.features.inserted = None;
        self.features.error = None;
        Ok(i.to_json())
    }

    /// The `gen.feature` call of the selected template and values, at the
    /// 3D cursor (in the active object's space when placed under it).
    fn features_call(&self) -> Option<OpCall> {
        let id = self.features.selected?;
        let doc = self.doc();
        let cursor = Vec3::from_array(self.ui.transform.cursor);
        let mut args = json!({ "feature": id, "params": Value::Object(self.features.params.clone()) });
        let parent = self.active_object().filter(|_| self.features.place == Place::Parent);
        let at = match parent.and_then(|k| doc.world_affine(k).map(|w| (k, w))) {
            Some((k, w)) => {
                args["parent"] = json!(doc.object(k)?.public_id());
                w.inverse().transform_point3(cursor)
            }
            None => cursor,
        };
        args["position"] = json!([at.x, at.y, at.z]);
        args["name"] = json!(crate::outliner::free_name(doc, id));
        Some(OpCall::new("gen.feature", args))
    }

    /// Inserts the selected template (one undo step).
    pub fn features_insert(&mut self) -> Result<Value, String> {
        let call = self.features_call().ok_or(t("pick a feature first"))?;
        let r = self.execute(&[call]);
        match &r {
            Ok(_) => {
                self.features.inserted = Some(self.session.revision());
                self.features.error = None;
            }
            Err(e) => self.features.error = Some(e.clone()),
        }
        r
    }

    /// The parameters changed: when the inserted feature is still the last
    /// operation, it runs again with them in place of that step.
    fn features_changed(&mut self) {
        let Some(info) = self.features.selected.and_then(info) else { return };
        clamp(&info, &mut self.features.params);
        let current = self.features.inserted == Some(self.session.revision())
            && self.last_op.as_ref().is_some_and(|l| l.op == "gen.feature" && l.revision == self.session.revision());
        if !current {
            self.features.inserted = None;
            return;
        }
        let Some(mut params) = self.last_op.as_ref().map(|l| l.params.clone()) else { return };
        params["params"] = Value::Object(self.features.params.clone());
        match self.adjust_last(params) {
            Ok(()) => {
                self.features.inserted = Some(self.session.revision());
                self.features.error = None;
            }
            Err(e) => self.features.error = Some(e),
        }
    }

    /// Runs a `features.*` command.
    pub(crate) fn features_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        match id {
            "features.open" => {
                let shown = self.ui.docking.is_visible(crate::docking::Panel::Features);
                let on = params.get("on").and_then(Value::as_bool).unwrap_or(!shown);
                self.ui.docking.set_visible(crate::docking::Panel::Features, on)?;
                if on {
                    self.ui.docking.command(&json!({ "operation": "activate", "panel": "features" }))?;
                }
                if let Some(f) = params.get("feature").and_then(Value::as_str) {
                    self.features_select(f)?;
                }
                Ok(json!({ "open": on }))
            }
            "features.search" => {
                self.features.query = params.get("query").and_then(Value::as_str).unwrap_or("").to_owned();
                let found: Vec<&str> = p3d_features::search(&self.features.query).iter().map(|i| i.id).collect();
                Ok(json!({ "found": found }))
            }
            "features.select" => {
                let f = params.get("feature").or_else(|| params.get("id")).and_then(Value::as_str).ok_or(t("feature is required"))?;
                self.features_select(f)
            }
            "features.set" => {
                let info = self.features.selected.and_then(info).ok_or(t("pick a feature first"))?;
                let values = params.get("params").unwrap_or(params).as_object().ok_or(t("give the parameters as an object"))?;
                for (k, v) in values {
                    if !info.params.iter().any(|p| p.name == k) {
                        return Err(format!("`{}` {} `{k}`", info.id, t("has no parameter")));
                    }
                    self.features.params.insert(k.clone(), v.clone());
                }
                self.features_changed();
                Ok(Value::Object(self.features.params.clone()))
            }
            "features.reset" => {
                let info = self.features.selected.and_then(info).ok_or(t("pick a feature first"))?;
                self.features.params = defaults(&info);
                self.features_changed();
                Ok(Value::Object(self.features.params.clone()))
            }
            "features.place" => {
                self.features.place = match params.get("place").and_then(Value::as_str) {
                    Some("parent") => Place::Parent,
                    _ => Place::Cursor,
                };
                Ok(json!({ "place": if self.features.place == Place::Parent { "parent" } else { "cursor" } }))
            }
            "features.insert" => {
                if let Some(f) = params.get("feature").and_then(Value::as_str) {
                    self.features_select(f)?;
                }
                if let Some(Value::Object(values)) = params.get("params") {
                    for (k, v) in values {
                        self.features.params.insert(k.clone(), v.clone());
                    }
                }
                let r = self.features_insert()?;
                // From the Add menu: the panel opens to tune it.
                if params.get("open").and_then(Value::as_bool) == Some(true) {
                    self.ui.docking.set_visible(crate::docking::Panel::Features, true)?;
                }
                Ok(r)
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// Feature landmarks of the selected objects in world space: their
    /// `lm.<name>` groups, and for selected Model Specs every feature part's
    /// (SDF features have no groups: their landmarks come from the spec).
    /// Found once per scene state and selection.
    ///
    /// Complexity: O(objects shown x groups) plus one landmark query per
    /// SDF feature part, when the scene or selection changed.
    pub fn feature_landmarks(&mut self) -> Arc<Vec<Landmark>> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.scene_key().hash(&mut h);
        self.selection.hash(&mut h);
        let key = h.finish();
        if let Some((k, m)) = &self.features.marks
            && *k == key
        {
            return Arc::clone(m);
        }
        let doc = Arc::clone(self.session.doc());
        // The selected objects, and every part object of selected specs.
        let mut objects: std::collections::BTreeSet<ObjectKey> = self.selection.clone();
        let roots: std::collections::BTreeSet<ObjectKey> =
            self.selection.iter().filter_map(|&k| crate::spec::spec_of(&doc, k).map(|s| s.0)).collect();
        let mut parts = Vec::new();
        for &root in &roots {
            for p in crate::spec::parts_of(&doc, root) {
                objects.extend(p.object);
                parts.push((root, p));
            }
        }
        let mut out = Vec::new();
        let mut baked = std::collections::BTreeSet::new();
        for &k in &objects {
            let (Some(mesh), Some(w)) = (doc.object_mesh(k), doc.world_affine(k)) else { continue };
            for (name, p) in p3d_features::baked_landmarks(mesh) {
                baked.insert(k);
                out.push((k, name, w.transform_point3(p)));
            }
        }
        // Feature parts without groups (SDF features): the spec's landmarks
        // of the part, the template's names only.
        for (root, p) in parts.into_iter().filter(|(_, p)| p.key == "feature" && p.object.is_none_or(|k| !baked.contains(&k))) {
            let id = crate::spec::part_source(&doc, root, &p.name).and_then(|s| s.get("feature").and_then(Value::as_str).map(str::to_owned));
            let Some(info) = id.as_deref().and_then(info) else { continue };
            let Ok(points) = p3d_spec::part_landmarks(&doc, root, &p.name) else { continue };
            for l in points.into_iter().filter(|l| info.landmarks.iter().any(|(n, _)| *n == l.name)) {
                out.push((p.object.unwrap_or(root), l.name, l.point));
            }
        }
        let m = Arc::new(out);
        self.features.marks = Some((key, Arc::clone(&m)));
        m
    }

    /// The panel's state for agents (`ui.inspect`).
    pub(crate) fn features_state(&self) -> Value {
        let f = &self.features;
        json!({
            "open": self.ui.docking.is_visible(crate::docking::Panel::Features),
            "query": f.query,
            "found": p3d_features::search(&f.query).iter().map(|i| i.id).collect::<Vec<_>>(),
            "selected": f.selected,
            "params": f.params,
            "place": if f.place == Place::Parent { "parent" } else { "cursor" },
            "live": f.inserted == Some(self.session.revision()),
            "error": f.error,
            "landmarksShown": self.ui.view.feature_landmarks,
            "landmarks": f.screen.iter().map(|(n, at)| json!({ "name": n, "at": [at.x, at.y] })).collect::<Vec<_>>(),
        })
    }
}

/// The Features panel body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let mut select = None;
    widgets::search_field(ui, &mut app.features.query, t("Search features (ear, bolt, vent...)"));
    ui.add_space(4.0);
    let found = p3d_features::search(&app.features.query);
    if found.is_empty() {
        ui.label(RichText::new(t("No feature matches the search.")).color(tk.text_dim));
    }
    let list_h = (ui.available_height() * 0.38).clamp(90.0, 260.0);
    egui::ScrollArea::vertical().id_salt("features-list").max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
        for i in &found {
            if feature_row(ui, i, app.features.selected == Some(i.id)).clicked() {
                select = Some(i.id);
            }
        }
    });
    if let Some(id) = select {
        let _ = app.features_select(id);
    }
    let Some(info) = app.features.selected.and_then(info) else {
        ui.add_space(6.0);
        ui.label(RichText::new(t("Pick a feature to see its parameters, landmarks and budget.")).font(theme::small()).color(tk.text_faint));
        return;
    };
    ui.add_space(6.0);
    ui.separator();
    egui::ScrollArea::vertical().id_salt("features-detail").auto_shrink([false, false]).show(ui, |ui| detail(app, ui, &info));
}

/// One template in the list: label, id, an `sdf` badge, the summary.
fn feature_row(ui: &mut egui::Ui, i: &FeatureInfo, selected: bool) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let pill = rect.shrink2(vec2(1.0, 1.0));
    if selected {
        ui.painter().rect_filled(pill, 5.0, tk.accent.gamma_multiply(0.22));
    } else if resp.hovered() {
        ui.painter().rect_filled(pill, 5.0, tk.control.gamma_multiply(0.8));
    }
    let p = ui.painter_at(rect);
    let title = p.layout_no_wrap(i.label.to_owned(), theme::body(), tk.text);
    let tw = title.size().x;
    p.galley(pos2(rect.left() + 8.0, rect.top() + 4.0), title, tk.text);
    let mut x = rect.left() + 8.0 + tw + 6.0;
    let id = p.layout_no_wrap(i.id.to_owned(), theme::small(), tk.text_faint);
    let iw = id.size().x;
    p.galley(pos2(x, rect.top() + 6.0), id, tk.text_faint);
    x += iw + 6.0;
    if i.sdf {
        let g = p.layout_no_wrap("sdf".to_owned(), egui::FontId::proportional(9.5), tk.icon_accent);
        let r = egui::Rect::from_min_size(pos2(x, rect.top() + 5.0), vec2(g.size().x + 10.0, 15.0));
        p.rect_filled(r, 7.5, tk.accent_soft);
        p.galley(r.center() - g.size() / 2.0, g, tk.icon_accent);
    }
    let mut job = egui::text::LayoutJob::simple_singleline(i.summary.to_owned(), theme::small(), tk.text_dim);
    job.wrap = egui::text::TextWrapping { max_width: rect.width() - 16.0, max_rows: 1, break_anywhere: false, overflow_character: Some('\u{2026}') };
    let g = p.layout_job(job);
    p.galley(pos2(rect.left() + 8.0, rect.bottom() - g.size().y - 4.0), g, tk.text_dim);
    crate::control::record_widget(ui, &resp, i.label, "option", Some(selected));
    resp.on_hover_text(i.summary)
}

/// The selected template: summary, parameters, landmarks, budget, Insert.
fn detail(app: &mut PixelApp, ui: &mut egui::Ui, info: &FeatureInfo) {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        ui.label(RichText::new(info.label).strong().color(tk.text));
        ui.label(RichText::new(info.id).font(theme::small()).color(tk.text_faint));
        if info.sdf {
            ui.label(RichText::new(t("SDF: blends into SDF parts")).font(theme::small()).color(tk.icon_accent));
        }
    });
    ui.add(egui::Label::new(RichText::new(info.summary).font(theme::small()).color(tk.text_dim)).wrap());
    ui.add_space(4.0);
    // Where it goes, and Insert (kept in view above the parameters).
    let active = app.active_object().and_then(|k| app.doc().object(k).map(|o| o.name().to_owned()));
    ui.horizontal(|ui| {
        let mut place = app.features.place;
        ui.radio_value(&mut place, Place::Cursor, t("At the 3D cursor"));
        ui.add_enabled_ui(active.is_some(), |ui| {
            let label = active.as_deref().map_or_else(|| t("Under the active object").to_owned(), |n| format!("{} {n}", t("Under")));
            ui.radio_value(&mut place, Place::Parent, label);
        });
        app.features.place = if active.is_none() { Place::Cursor } else { place };
    });
    let insert = ui.add_sized([ui.available_width(), 26.0], egui::Button::new(RichText::new(t("Insert")).color(tk.on_accent)).fill(tk.accent));
    crate::control::record_widget(ui, &insert, t("Insert feature"), "button", None);
    if insert.on_hover_text(t("Add the feature as a new object (one undo step); then parameter edits update it")).clicked()
        && let Err(e) = app.features_insert()
    {
        app.set_status(e);
    }
    let live = app.features.inserted == Some(app.session.revision());
    let note = if live { t("Editing a parameter now updates the inserted feature.") } else { t("Insert places it as a new object.") };
    ui.add(egui::Label::new(RichText::new(note).font(theme::small()).color(tk.text_faint)).wrap());
    if let Some(e) = &app.features.error {
        ui.add(egui::Label::new(RichText::new(e).font(theme::small()).color(tk.health_error)).wrap());
    }
    ui.add_space(6.0);
    // The parameters, through the shared schema form.
    let registry = app.session.registry();
    let root = registry.get(crate::sdf::op_id(registry)).map(|op| op.params_schema()).unwrap_or(Value::Null);
    let schema = schema(info);
    let mut v = Value::Object(app.features.params.clone());
    let o = crate::schema_form::form(ui, egui::Id::new(("feature-params", info.id)), &schema, &root, &mut v, &[]);
    if o.changed
        && let Value::Object(m) = v
    {
        app.features.params = m;
        app.features_changed();
    }
    if o.dragging {
        ui.ctx().request_repaint();
    }
    ui.horizontal(|ui| {
        let r = ui.small_button(t("Defaults"));
        crate::control::record_widget(ui, &r, t("Defaults"), "button", None);
        if r.on_hover_text(t("Every parameter back to its default")).clicked() {
            app.features.params = defaults(info);
            app.features_changed();
        }
    });
    ui.add_space(6.0);
    widgets::info(ui, t("Budget"), format!("{} {} {}", t("about"), crate::chrome::thousands(info.budget as usize), t("triangles")));
    ui.label(RichText::new(t("Landmarks")).color(tk.text_dim));
    for (name, doc) in info.landmarks {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(*name).font(theme::mono()).color(tk.text));
            ui.label(RichText::new(*doc).font(theme::small()).color(tk.text_faint));
        });
    }
}

/// Feature landmarks of the selected objects as labelled dots (View >
/// Overlays > Feature landmarks).
///
/// Complexity: O(landmarks) per frame; they are found once per scene state.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui) {
    app.features.screen.clear();
    let rect = app.viewport.rect;
    if !app.ui.view.feature_landmarks || app.ui.mode.is_edit() || !rect.is_positive() {
        return;
    }
    let marks = app.feature_landmarks();
    let tk = Tokens::get();
    let vp: Mat4 = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let p = ui.painter_at(rect);
    // Dots always; labels where they fit (no pile-ups on dense heads).
    let mut placed = crate::spec::Placed::default();
    for (_, name, at) in marks.iter() {
        let c = vp * at.extend(1.0);
        if c.w <= 1e-6 {
            continue;
        }
        let n = c.truncate() / c.w;
        let s = pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height());
        p.circle(s, 3.5, tk.icon_accent, Stroke::new(1.0, egui::Color32::from_black_alpha(160)));
        app.features.screen.push((name.clone(), s));
        let g = p.layout_no_wrap(name.clone(), theme::small(), tk.text);
        let size = g.size() + vec2(6.0, 2.0);
        let spots = [vec2(6.0, -size.y - 3.0), vec2(6.0, 3.0), vec2(-size.x - 6.0, -size.y - 3.0), vec2(-size.x - 6.0, 3.0)];
        let Some(label) = spots.iter().map(|d| egui::Rect::from_min_size(s + *d, size)).find(|r| placed.free(*r)) else { continue };
        placed.add(label);
        p.rect_filled(label, 3.0, tk.popup.gamma_multiply(0.8));
        p.galley(label.min + vec2(3.0, 1.0), g, tk.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn every_template_maps_to_a_form_with_its_defaults() {
        for f in p3d_features::library() {
            let i = f.info();
            let s = schema(&i);
            let root = p3d_ops::Registry::builtin().get("sdf.mesh").map(|o| o.params_schema()).unwrap_or(Value::Null);
            let props = s["properties"].as_object().unwrap();
            assert_eq!(props.len(), i.params.len(), "{}", i.id);
            for p in i.params {
                let kind = crate::schema_form::kind_of(&props[p.name], &root);
                let want = match p.kind {
                    ParamKind::Length => matches!(kind, crate::schema_form::Kind::Length),
                    ParamKind::Angle => matches!(kind, crate::schema_form::Kind::Angle),
                    ParamKind::Ratio => matches!(kind, crate::schema_form::Kind::Number { .. }),
                    ParamKind::Count => matches!(kind, crate::schema_form::Kind::Integer { .. }),
                    ParamKind::Choice(_) => matches!(kind, crate::schema_form::Kind::Enum(_)),
                };
                assert!(want, "{}.{}: {kind:?}", i.id, p.name);
            }
            // The defaults resolve as the op reads them.
            p3d_features::resolve(&i, &Value::Object(defaults(&i))).unwrap();
        }
    }

    #[test]
    fn insert_then_edit_updates_the_feature_in_one_step() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("view.cursor", json!({ "to": "origin" })).ok();
        a.ui.transform.cursor = [0.2, 0.1, 0.0];
        a.run("features.select", json!({ "feature": "ear" })).unwrap();
        let steps = a.session.history().len();
        a.run("features.insert", Value::Null).unwrap();
        assert_eq!(a.session.history().len(), steps + 1);
        let k = a.doc().resolve("ear").unwrap();
        assert!((a.doc().object(k).unwrap().transform.translation.x - 0.2).abs() < 1e-6, "at the cursor");
        let marks = p3d_features::baked_landmarks(a.doc().object_mesh(k).unwrap());
        assert!(marks.contains_key("tip"), "{marks:?}");
        // A parameter edit runs it again in place of the insert.
        let tall = |a: &PixelApp| {
            a.doc().object_mesh(a.doc().resolve("ear").unwrap()).and_then(|m| m.bounds()).map_or(0.0, |(lo, hi)| (hi - lo).max_element())
        };
        let before = tall(&a);
        a.run("features.set", json!({ "length": 0.12 })).unwrap();
        assert_eq!(a.session.history().len(), steps + 1, "still one step");
        assert!(tall(&a) > before + 0.01, "{before} -> {}", tall(&a));
        assert_eq!(a.features_state()["live"], json!(true));
        // Out of range values are kept in range.
        a.run("features.set", json!({ "length": 99 })).unwrap();
        let max = info("ear").unwrap().params.iter().find(|p| p.name == "length").unwrap().max;
        assert_eq!(a.features.params["length"], json!(max));
        // Under the active object: a child, at the cursor in its space.
        a.run("prim.box", json!({ "name": "head", "position": [0.2, 0, 0] })).unwrap();
        a.run("features.place", json!({ "place": "parent" })).unwrap();
        a.run("features.insert", json!({ "feature": "bolt" })).unwrap();
        let bolt = a.doc().resolve("head/bolt").unwrap();
        assert!(a.doc().object(bolt).unwrap().transform.translation.distance(Vec3::new(0.0, 0.1, 0.0)) < 1e-5);
    }

    #[test]
    fn landmarks_of_selected_features_show_as_dots() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("features.insert", json!({ "feature": "horn" })).unwrap();
        a.ui.view.feature_landmarks = true;
        let marks = a.feature_landmarks();
        assert!(!marks.is_empty() && marks.iter().all(|(_, n, _)| !n.starts_with("lm.")), "{marks:?}");
        assert_eq!(a.run("view.feature_landmarks", json!({ "on": false })).unwrap(), json!({ "featureLandmarks": false }));
    }
}
