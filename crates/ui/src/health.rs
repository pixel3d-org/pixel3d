//! The Health panel: the same diagnostics agents get (project spec 17.10,
//! 20.6), as a findings view grouped by category (or code). It renders the
//! diagnose JSON as it comes, so new diagnostics need no UI code: code,
//! severity, part, measured against threshold, the cause (a spec path in it
//! opens that part in the Spec tab), any extra fields, and every fix with
//! its predicted result and an **Apply** button (one undo step). A click on
//! a finding's badge in the viewport frames it. Checks run on a background
//! thread whenever the document changes, so editing never waits for them.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Align2, Color32, Rect, RichText, Sense, pos2, vec2};
use p3d_perceive::diagnose::{Finding, Severity};
use p3d_scene::Document;
use serde_json::Value;

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

type Outcome = Result<Vec<Finding>, String>;

/// Findings for one document state, and the check in progress.
#[derive(Default)]
pub struct HealthState {
    /// The scene key the findings belong to.
    pub key: Option<u64>,
    pub findings: Vec<Finding>,
    pub error: Option<String>,
    running: Option<(u64, Receiver<Outcome>)>,
}

impl HealthState {
    /// Whether a check is running.
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// Starts a check when the document changed, and collects a finished one.
    ///
    /// Complexity: O(1) on the UI thread; the check itself is a full
    /// diagnose plus a few fix predictions on a worker thread.
    pub fn update(&mut self, key: u64, doc: &Arc<Document>, registry: &Arc<p3d_ops::Registry>, seed: u64, ctx: &egui::Context) {
        if let Some((k, rx)) = &self.running {
            match rx.try_recv() {
                Ok(outcome) => {
                    let k = *k;
                    self.running = None;
                    self.key = Some(k);
                    match outcome {
                        Ok(f) => {
                            self.findings = f;
                            self.error = None;
                        }
                        Err(e) => {
                            self.findings.clear();
                            self.error = Some(e);
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    return;
                }
                Err(TryRecvError::Disconnected) => self.running = None,
            }
        }
        if self.key == Some(key) || self.running.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (doc, registry, ctx2) = (Arc::clone(doc), Arc::clone(registry), ctx.clone());
        let spawned = std::thread::Builder::new().name("health check".into()).spawn(move || {
            let outcome = p3d_engine::predict::diagnose_predicted(&registry, &doc, None, seed).map_err(|e| e.to_string());
            // The receiver may be gone (app closed); nothing to do then.
            let _ = tx.send(outcome);
            ctx2.request_repaint();
        });
        match spawned {
            Ok(_) => self.running = Some((key, rx)),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// The worst severity, `None` when there are no findings.
    pub fn worst(&self) -> Option<Severity> {
        self.findings.iter().map(|f| f.severity).min()
    }
}

/// The color of a severity.
pub fn color(s: Option<Severity>) -> Color32 {
    let tk = Tokens::get();
    match s {
        Some(Severity::Error) => tk.health_error,
        Some(Severity::Warn) => tk.health_warn,
        Some(Severity::Info) => tk.accent,
        None => tk.health_ok,
    }
}

/// Fields every finding has; anything else in a finding's JSON is shown as
/// it comes (new diagnostics need no code here).
const KNOWN: [&str; 14] =
    ["id", "code", "severity", "part", "other", "at", "side", "measured", "threshold", "message", "cause", "fixes", "hint", "category"];

/// What the panel asks for this frame.
#[derive(Default)]
struct Out {
    fix: Option<(String, usize)>,
    select: Option<String>,
    spec_path: Option<(String, String)>,
    frame: Option<String>,
}

/// A finding's group: its `category` when the diagnostics give one, else
/// its code.
fn group_of(f: &Value) -> String {
    f.get("category").and_then(Value::as_str).or_else(|| f.get("code").and_then(Value::as_str)).unwrap_or("?").to_owned()
}

/// A number as short text (`0.0123` as `0.0123`, `18` as `18`).
fn num(v: &Value) -> String {
    match v.as_f64() {
        Some(x) => {
            let s = format!("{x:.4}");
            s.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        None => v.to_string(),
    }
}

/// The spec path (`/parts/ear_l/...`) in a text, if any.
fn spec_path_in(text: &str) -> Option<(String, String)> {
    let at = text.find("/parts/")?;
    let rest = &text[at..];
    let end = rest.find(|c: char| c.is_whitespace() || c == ')' || c == ',' || c == ';').unwrap_or(rest.len());
    let path = rest[..end].to_owned();
    let part = path.trim_start_matches("/parts/").split('/').next()?.to_owned();
    (!part.is_empty()).then_some((path, part))
}

/// The spec path a finding points at: in its cause first, then in any
/// other text it carries (diagnostics may add a dedicated field).
///
/// Complexity: O(size of the finding's JSON).
fn spec_path_of(f: &Value) -> Option<(String, String)> {
    fn search(v: &Value) -> Option<(String, String)> {
        match v {
            Value::String(s) => spec_path_in(s),
            Value::Object(m) => m.values().find_map(search),
            Value::Array(a) => a.iter().find_map(search),
            _ => None,
        }
    }
    let m = f.as_object()?;
    m.get("cause").and_then(search).or_else(|| m.iter().filter(|(k, _)| !matches!(k.as_str(), "cause" | "fixes")).find_map(|(_, v)| search(v)))
}

/// The Health panel body: findings grouped by category (or code), each
/// with its severity, part, measured value against its threshold, cause
/// (a spec path selects that part in the Spec tab), every fix with its
/// predicted result and an Apply button, and whatever else the diagnostics
/// report.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let current = app.health.key == Some(app.scene_key());
    let busy = app.health.busy();
    // Summary line with the traffic light.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 5.0, color(app.health.worst()));
        let n = app.health.findings.len();
        let errors = app.health.findings.iter().filter(|f| f.severity == Severity::Error).count();
        let text = if let Some(e) = &app.health.error {
            format!("{}: {e}", t("Check failed"))
        } else if n == 0 && app.health.key.is_some() {
            t("No problems found").to_owned()
        } else if app.health.key.is_none() {
            t("Checking...").to_owned()
        } else {
            format!("{n} {}, {errors} {}", t("findings"), t("errors"))
        };
        ui.label(RichText::new(text).color(tk.text));
        if busy || !current {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(t("updating")).font(theme::small()).color(tk.text_faint));
            });
        }
    });
    ui.add_space(4.0);
    // Every finding as the diagnostics give it, grouped.
    let findings: Vec<(Severity, Value)> = app.health.findings.iter().filter_map(|f| serde_json::to_value(f).ok().map(|v| (f.severity, v))).collect();
    let mut groups: BTreeMap<String, Vec<&(Severity, Value)>> = BTreeMap::new();
    for f in &findings {
        groups.entry(group_of(&f.1)).or_default().push(f);
    }
    let mut order: Vec<(&String, &Vec<&(Severity, Value)>)> = groups.iter().collect();
    order.sort_by_key(|(name, fs)| (fs.iter().map(|f| f.0).min(), (*name).clone()));
    let mut out = Out::default();
    egui::ScrollArea::vertical().id_salt("health").auto_shrink([false, false]).show(ui, |ui| {
        for (name, fs) in order {
            let worst = fs.iter().map(|f| f.0).min();
            let header = format!("{} ({})", name.replace('_', " ").to_lowercase(), fs.len());
            let id = ui.make_persistent_id(("health-group", name));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, color(worst));
                    ui.label(RichText::new(header).strong().color(tk.text));
                })
                .body(|ui| {
                    for (sev, f) in fs.iter().map(|x| (&x.0, &x.1)) {
                        finding(ui, current, *sev, f, &mut out);
                        ui.add_space(4.0);
                    }
                });
        }
    });
    apply(app, out);
}

/// One finding, rendered from its JSON.
fn finding(ui: &mut egui::Ui, current: bool, sev: Severity, f: &Value, out: &mut Out) {
    let tk = Tokens::get();
    let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let (id, code, part) = (s("id"), s("code"), s("part"));
    let frame = egui::Frame::NONE.fill(tk.chrome_dark.gamma_multiply(0.6)).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 4));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, color(Some(sev)));
            ui.label(RichText::new(format!("{id} {code}")).font(theme::mono()).color(tk.text));
            let p = ui.link(RichText::new(&part).color(tk.text_dim));
            crate::control::record_widget(ui, &p, &format!("{id} {part}"), "link", None);
            if p.on_hover_text(t("Select the part")).clicked() {
                out.select = Some(id.clone());
            }
            if let Some(o) = f.get("other").and_then(Value::as_str) {
                ui.label(RichText::new(format!("{} {o}", t("and"))).color(tk.text_dim));
            }
            let fr = ui.small_button(t("Frame"));
            crate::control::record_widget(ui, &fr, &format!("Frame {id}"), "button", None);
            if fr.on_hover_text(t("Frame where it is in the viewport")).clicked() {
                out.frame = Some(id.clone());
            }
        });
        ui.add(egui::Label::new(RichText::new(s("message")).color(tk.text)).wrap());
        // Measured against the threshold, when both are given.
        if let (Some(m), Some(th)) = (f.get("measured").filter(|v| !v.is_null()), f.get("threshold").filter(|v| !v.is_null())) {
            ui.label(RichText::new(format!("{} {}, {} {}", t("measured"), num(m), t("threshold"), num(th))).font(theme::small()).color(tk.text_dim));
        }
        // The cause: a spec path in it selects that part in the Spec tab.
        let cause = match f.get("cause") {
            Some(Value::String(c)) => c.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => crate::schema_form::readable(other, &Value::Null, &Value::Null),
        };
        let path = spec_path_of(f);
        if !cause.is_empty() || path.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{}:", t("cause"))).font(theme::small()).color(tk.text_faint));
                match path {
                    Some((path, part)) => {
                        let text = if cause.is_empty() { path.clone() } else { cause.clone() };
                        let l = ui.link(RichText::new(text).font(theme::small()));
                        crate::control::record_widget(ui, &l, &format!("{id} {path}"), "link", None);
                        if l.on_hover_text(t("Open this spec part in the Spec tab")).clicked() {
                            out.spec_path = Some((id.clone(), part));
                        }
                    }
                    None => {
                        ui.label(RichText::new(&cause).font(theme::small()).color(tk.text_faint));
                    }
                }
            });
        }
        // Whatever else the diagnostics report.
        if let Some(obj) = f.as_object() {
            for (k, v) in obj.iter().filter(|(k, v)| !KNOWN.contains(&k.as_str()) && !v.is_null()) {
                let text = match v {
                    Value::String(x) => x.clone(),
                    Value::Number(_) => num(v),
                    other => crate::schema_form::readable(other, &Value::Null, &Value::Null),
                };
                ui.add(egui::Label::new(RichText::new(format!("{}: {text}", k.replace('_', " "))).font(theme::small()).color(tk.text_dim)).wrap());
            }
        }
        // Every fix, with its predicted result.
        let fixes = f.get("fixes").and_then(Value::as_array).cloned().unwrap_or_default();
        for (i, x) in fixes.iter().enumerate() {
            let summary = x.get("summary").and_then(Value::as_str).unwrap_or("");
            ui.horizontal_wrapped(|ui| {
                let b = ui.add_enabled(current, egui::Button::new(t("Apply")));
                crate::control::record_widget(ui, &b, &format!("Fix {id}{}", if i == 0 { String::new() } else { format!(" {i}") }), "button", None);
                if b.on_hover_text(t("Run this fix (one undo step)")).clicked() {
                    out.fix = Some((id.clone(), i));
                }
                ui.label(RichText::new(summary).color(tk.text));
            });
            match x.get("predicted").and_then(Value::as_str) {
                Some(p) => {
                    ui.add(egui::Label::new(RichText::new(format!("{} {p}", t("predicted:"))).font(theme::small()).color(tk.text_faint)).wrap())
                }
                None => ui.label(RichText::new(t("The result is predicted when the check finishes.")).font(theme::small()).color(tk.text_faint)),
            };
        }
        if fixes.is_empty()
            && let Some(h) = f.get("hint").and_then(Value::as_str)
        {
            ui.add(egui::Label::new(RichText::new(h).font(theme::small()).color(tk.text_faint)).wrap());
        }
    });
}

/// Applies what the panel asked for.
fn apply(app: &mut PixelApp, out: Out) {
    if let Some(id) = out.select
        && let Err(e) = app.health_command("health.select", &serde_json::json!({ "id": id }))
    {
        app.set_status(e);
    }
    if let Some((id, part)) = out.spec_path
        && let Err(e) = app.health_select_spec_part(&id, &part)
    {
        app.set_status(e);
    }
    if let Some(id) = out.frame
        && let Err(e) = app.health_command("health.frame", &serde_json::json!({ "id": id }))
    {
        app.set_status(e);
    }
    if let Some((id, option)) = out.fix
        && let Err(e) = app.health_command("health.fix", &serde_json::json!({ "id": id, "option": option }))
    {
        app.set_status(e);
    }
}

impl PixelApp {
    fn health_finding(&self, id: &str) -> Result<p3d_perceive::diagnose::Finding, String> {
        self.health.findings.iter().find(|f| f.id == id).cloned().ok_or_else(|| format!("{} `{id}`", t("no finding")))
    }

    /// Opens the spec part `part` (named in a finding's cause) in the Spec
    /// tab: the spec of the finding's part, else the first spec with it.
    ///
    /// Complexity: O(spec roots).
    pub(crate) fn health_select_spec_part(&mut self, id: &str, part: &str) -> Result<Value, String> {
        let f = self.health_finding(id)?;
        let doc = self.doc();
        let root = doc
            .resolve(&f.part)
            .ok()
            .and_then(|k| crate::spec::spec_of(doc, k).map(|s| s.0))
            .filter(|r| crate::spec::part_source(doc, *r, part).is_some())
            .or_else(|| doc.spec_roots().find(|r| crate::spec::part_source(doc, *r, part).is_some()))
            .ok_or_else(|| format!("{} `{part}`", t("no spec has a part")))?;
        let root = doc.object(root).map(|o| o.public_id()).ok_or(t("the spec is gone"))?;
        self.run("spec.select_part", serde_json::json!({ "root": root, "part": part }))
    }

    /// Frames a world point with room around it (a finding's place).
    pub(crate) fn frame_point(&mut self, at: glam::Vec3, size: f32) {
        let r = glam::Vec3::splat(size.max(0.02));
        let rect = self.viewport.rect;
        let aspect = if rect.is_positive() { rect.width() / rect.height() } else { 16.0 / 9.0 };
        self.viewport.camera.frame(at - r, at + r, aspect);
    }

    /// Runs `health.open`, and `health.fix`, `health.frame`, `health.select`
    /// and `health.spec_part` on a finding by id.
    pub(crate) fn health_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        if id == "health.open" {
            let shown = self.ui.docking.is_visible(crate::docking::Panel::Health);
            let on = params.get("on").and_then(Value::as_bool).unwrap_or(!shown);
            self.ui.docking.set_visible(crate::docking::Panel::Health, on)?;
            if on {
                self.ui.docking.command(&serde_json::json!({ "operation": "activate", "panel": "health" }))?;
            }
            return Ok(serde_json::json!({ "open": on }));
        }
        let fid = params.get("id").and_then(Value::as_str).ok_or(t("id is required, for example F1"))?.to_owned();
        let f = self.health_finding(&fid)?;
        match id {
            "health.fix" => {
                let option = params.get("option").and_then(Value::as_u64).unwrap_or(0) as usize;
                let (f, r) = p3d_engine::predict::apply_fix(&mut self.session, None, &fid, option).map_err(|e| e.to_string())?;
                let summary = r.results.first().map(|x| x.output.summary.clone()).unwrap_or_default();
                self.set_status(format!("{} {} {}: {summary}", t("Fixed"), f.id, f.code));
                self.last_op = None;
                self.prune_selection();
                Ok(serde_json::json!({ "fixed": f.id, "summary": summary }))
            }
            "health.frame" => {
                // Room for the part around the point: half its size, so the place reads in context.
                let only = self.doc().resolve(&f.part).ok().map(|k| std::collections::BTreeSet::from([k]));
                let size = only.and_then(|s| self.viewport.bounds(Some(&s))).map_or(0.1, |(lo, hi)| (hi - lo).max_element() * 0.5);
                self.frame_point(f.at, size);
                Ok(serde_json::json!({ "framed": f.id, "at": f.at.to_array() }))
            }
            "health.spec_part" => {
                // The part named in the request, else the spec path in the cause.
                let part = match params.get("part").and_then(Value::as_str) {
                    Some(p) => p.to_owned(),
                    None => serde_json::to_value(&f).ok().as_ref().and_then(spec_path_of).map(|x| x.1).ok_or(t("the finding names no spec part"))?,
                };
                self.health_select_spec_part(&fid, &part)
            }
            "health.select" => {
                let k = self.doc().resolve(&f.part).map_err(|e| e.to_string())?;
                self.handle_viewport(vec![crate::viewport::ViewportEvent::Picked { object: Some(k), add: false }]);
                Ok(serde_json::json!({ "selected": f.part }))
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// The findings for agents (`ui.inspect`), as the diagnostics give them.
    pub(crate) fn health_state(&self) -> Value {
        let badges: Vec<Value> =
            badge_points(self).into_iter().map(|(i, at)| serde_json::json!({ "id": self.health.findings[i].id, "at": [at.x, at.y] })).collect();
        serde_json::json!({
            "current": self.health.key == Some(self.scene_key()),
            "busy": self.health.busy(),
            "worst": self.health.worst().map(Severity::as_str),
            "findings": self.health.findings,
            "badges": badges,
        })
    }
}

/// The findings' badge positions in the viewport (points), by finding
/// index, for findings in front of the camera (at most 64 badges).
///
/// Complexity: O(findings).
fn badge_points(app: &PixelApp) -> Vec<(usize, egui::Pos2)> {
    let rect = app.viewport.rect;
    if !rect.is_positive() || app.health.key != Some(app.scene_key()) {
        return Vec::new();
    }
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height());
    let mut out = Vec::new();
    for (i, f) in app.health.findings.iter().enumerate().take(64) {
        let c = vp.project_point3(f.at);
        if !(0.0..=1.0).contains(&c.z) || c.x.abs() > 1.0 || c.y.abs() > 1.0 {
            continue;
        }
        out.push((i, pos2(rect.left() + (c.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - c.y * 0.5) * rect.height())));
    }
    out
}

/// Severity badges at the findings' positions in the viewport; a click on
/// one frames it (and selects nothing).
///
/// Complexity: O(findings).
pub fn badges(app: &mut PixelApp, ui: &egui::Ui, events: &mut Vec<crate::viewport::ViewportEvent>) {
    let p = ui.painter_at(app.viewport.rect);
    let click = app.viewport.tool_input.clicked;
    let mut hit: Option<String> = None;
    for (i, at) in badge_points(app) {
        let f = &app.health.findings[i];
        let col = color(Some(f.severity));
        p.circle_filled(at, 7.0, col);
        p.text(at, Align2::CENTER_CENTER, "!", egui::FontId::proportional(10.0), Color32::from_rgb(0x12, 0x14, 0x18));
        let label = Rect::from_min_size(at + vec2(10.0, -8.0), vec2(0.0, 16.0));
        p.text(label.left_center(), Align2::LEFT_CENTER, &f.id, theme::small(), col);
        if click.is_some_and(|q| q.distance(at) <= 9.0) {
            hit = Some(f.id.clone());
        }
    }
    if let Some(id) = hit {
        app.viewport.cancel_pick();
        events.retain(|e| {
            !matches!(
                e,
                crate::viewport::ViewportEvent::Picked { .. }
                    | crate::viewport::ViewportEvent::MissedAt { .. }
                    | crate::viewport::ViewportEvent::ElementClick { .. }
            )
        });
        if let Err(e) = app.health_command("health.frame", &serde_json::json!({ "id": id })) {
            app.set_status(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A box with its bottom deleted: an open boundary to find and fill.
    fn bin() -> PixelApp {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "bin" })).unwrap();
        app.run("edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "bottom" } } })).unwrap();
        app
    }

    /// Runs the background check to the end.
    fn check(app: &mut PixelApp) {
        let ctx = egui::Context::default();
        let key = app.scene_key();
        let doc = Arc::clone(app.session.doc());
        let registry = Arc::clone(app.session.registry());
        for _ in 0..500 {
            app.health.update(key, &doc, &registry, 0, &ctx);
            if app.health.key == Some(key) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(app.health.key, Some(key), "the check finished");
    }

    fn open_boundary(app: &PixelApp) -> Finding {
        let open = app.health.findings.iter().find(|f| f.code == "OPEN_BOUNDARY").cloned();
        let Some(f) = open else { panic!("no open boundary found: {:?}", app.health.findings) };
        f
    }

    #[test]
    fn checks_run_in_the_background_and_fixes_apply() {
        let mut app = bin();
        check(&mut app);
        let f = open_boundary(&app);
        assert!(!f.fixes.is_empty());
        assert_eq!(app.health.worst().map(|s| s <= Severity::Warn), Some(true));
        // A fix the finding does not have is an error, not a guess.
        assert!(app.run("health.fix", json!({ "id": f.id, "option": f.fixes.len() })).is_err());
        let before = app.session.history().len();
        let r = app.run("health.fix", json!({ "id": f.id, "option": 0 })).unwrap();
        assert_eq!(r["fixed"], json!(f.id));
        assert_eq!(app.session.history().len(), before + 1, "one undo step");
    }

    #[test]
    fn findings_group_by_category_else_code() {
        assert_eq!(group_of(&json!({ "code": "OPEN_BOUNDARY" })), "OPEN_BOUNDARY");
        assert_eq!(group_of(&json!({ "code": "THIN_WALL", "category": "printability" })), "printability");
        assert_eq!((num(&json!(0.0012)), num(&json!(18.0)), num(&json!("2 mm"))), ("0.0012".into(), "18".into(), "\"2 mm\"".into()));
    }

    #[test]
    fn spec_paths_are_found_anywhere_in_a_finding() {
        let cause = json!({ "cause": "set by /parts/ear_l/params/size (12 mm)" });
        assert_eq!(spec_path_of(&cause), Some(("/parts/ear_l/params/size".into(), "ear_l".into())));
        // A dedicated field (new diagnostics) works the same.
        let field = json!({ "cause": "last changed by edit.extrude", "source": { "path": "/parts/nose" } });
        assert_eq!(spec_path_of(&field).map(|p| p.1), Some("nose".into()));
        // Fix arguments are not a cause.
        let fixes = json!({ "cause": "last changed by x", "fixes": [{ "args": { "path": "/parts/head" } }] });
        assert_eq!(spec_path_of(&fixes), None);
        assert_eq!(spec_path_in("/parts/"), None);
    }

    #[test]
    fn agents_frame_select_and_open_findings() {
        let mut app = bin();
        app.viewport.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        check(&mut app);
        let f = open_boundary(&app);
        // ui.inspect carries the findings as the diagnostics give them.
        let state = app.health_state();
        assert_eq!((state["current"].clone(), state["worst"].is_string()), (json!(true), true));
        assert!(state["findings"].as_array().is_some_and(|a| a.iter().any(|x| x["code"] == "OPEN_BOUNDARY")));
        // Framing centers the camera on the finding.
        app.run("health.frame", json!({ "id": f.id })).unwrap();
        assert!((app.viewport.camera.target - f.at).length() < 1e-3, "{} vs {}", app.viewport.camera.target, f.at);
        app.run("view.frame", Value::Null).unwrap();
        app.run("health.select", json!({ "id": f.id })).unwrap();
        let bin = app.doc().resolve("bin").unwrap();
        assert_eq!(app.selection.iter().copied().collect::<Vec<_>>(), vec![bin]);
        assert!(app.run("health.frame", json!({ "id": "F999" })).is_err());
        // A cause naming a spec part opens that part in the Spec tab.
        app.run("spec.starter", json!({ "name": "lamp" })).unwrap();
        let mut g = f.clone();
        g.id = "F99".into();
        g.cause = Some("radius set by /parts/shade/params/radius".into());
        app.health.findings.push(g);
        assert!(app.run("health.spec_part", json!({ "id": f.id })).is_err(), "no spec path in its cause");
        app.run("health.spec_part", json!({ "id": "F99" })).unwrap();
        assert_eq!(app.spec.part.as_deref(), Some("shade"));
        assert_eq!(app.spec.root, app.doc().resolve("lamp").ok());
        // The panel opens and closes from the View menu or the palette.
        app.run("health.open", json!({ "on": false })).unwrap();
        assert!(!app.ui.docking.is_visible(crate::docking::Panel::Health));
        app.run("health.open", Value::Null).unwrap();
        assert!(app.ui.docking.is_visible(crate::docking::Panel::Health));
    }
    #[test]
    fn a_click_on_a_badge_frames_its_finding_and_picks_nothing() {
        use crate::viewport::ViewportEvent;
        let mut app = bin();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        app.viewport.rect = rect;
        check(&mut app);
        let f = open_boundary(&app);
        let c = app.viewport.camera.view_proj(rect.width() / rect.height()).project_point3(f.at);
        let at = pos2((c.x * 0.5 + 0.5) * rect.width(), (0.5 - c.y * 0.5) * rect.height());
        let ctx = egui::Context::default();
        let frame = |app: &mut PixelApp, click: egui::Pos2| {
            app.viewport.tool_input.clicked = Some(click);
            let mut events = vec![ViewportEvent::Picked { object: None, add: false }];
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| badges(app, ui, &mut events));
            out.textures_delta.clear();
            events
        };
        // A click away from the badges is the viewport's.
        let before = app.viewport.camera.target;
        assert_eq!(frame(&mut app, at + vec2(40.0, 40.0)).len(), 1);
        assert_eq!(app.viewport.camera.target, before);
        // On a badge: framed, and the pick is dropped.
        assert!(frame(&mut app, at + vec2(3.0, -2.0)).is_empty());
        assert!((app.viewport.camera.target - f.at).length() < 1e-3);
    }
}
