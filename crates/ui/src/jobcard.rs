//! The run card for ops that take a while on a mesh (quad remesh, voxel
//! remesh, any op flagged long running that takes a `target`): a card over
//! the viewport with the op's settings from its schema, an estimate of the
//! result before running (faces from the surface area over the edge
//! length squared), and a run in the background with progress and Cancel.
//! The run goes through the normal op path as one undo step; Run again
//! replaces the card's last run while nothing else changed since.

use egui::{Align2, RichText, vec2};
use p3d_engine::{JobStatus, OpCall};
use p3d_ops::OpFlags;
use p3d_scene::ObjectKey;
use serde_json::{Map, Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::schema_form;
use crate::theme::{self, Tokens};
use crate::widgets;

/// The card between frames.
#[derive(Default)]
pub struct JobCard {
    /// The op the card runs, and the object it runs on.
    pub op: Option<String>,
    pub target: Option<ObjectKey>,
    /// The op's parameters besides the target.
    pub params: Value,
    running: Option<p3d_engine::Job>,
    /// The revision the card's last run produced (Run again replaces it).
    last: Option<u64>,
    /// The last run was undone to be replaced: redo it if the new run fails.
    replaced: bool,
    cancelling: bool,
    pub summary: Option<String>,
    pub error: Option<String>,
    /// Surface area and largest side of a mesh, by mesh, for the estimate.
    measured: Option<(usize, f64, f64, usize)>,
}

impl JobCard {
    pub fn is_open(&self) -> bool {
        self.op.is_some()
    }

    pub fn busy(&self) -> bool {
        self.running.is_some()
    }

    /// How far the run got, in [0, 1].
    pub fn progress(&self) -> Option<f32> {
        self.running.as_ref().map(|j| j.handle().progress())
    }
}

/// Whether an op runs in the card: long running, on a required `target`
/// (ops that may also make a new object, such as `sdf.silhouettes`, run
/// as usual).
pub fn runs_in_card(registry: &p3d_ops::Registry, op: &str) -> bool {
    registry.get(op).is_ok_and(|o| {
        let schema = o.params_schema();
        let required = schema.get("required").and_then(Value::as_array).is_some_and(|r| r.iter().any(|k| k == "target"));
        o.meta().flags.contains(OpFlags::LONG_RUNNING) && required
    })
}

/// A mesh's surface area, largest side and open edges.
///
/// Complexity: O(corners + edges).
fn measure(mesh: &p3d_mesh::Mesh) -> (f64, f64, usize) {
    let a: f64 = mesh.faces().map(|f| f64::from(mesh.face_area(f))).sum();
    let s = mesh.bounds().map_or(1.0, |(lo, hi)| f64::from((hi - lo).max_element()));
    let o = mesh.edges().filter(|&e| mesh.is_boundary_edge(e)).count();
    (a, s, o)
}

/// The length an op's result is built at: the set edge or detail, else the
/// op's own default from the mesh's largest side.
fn length_used(op: &str, params: &Value, side: f64) -> (f64, bool) {
    let key = if op == "remesh.quad" { "edge" } else { "detail" };
    match params.get(key).and_then(Value::as_f64) {
        Some(l) => (l, false),
        None if op == "remesh.quad" => (side / 40.0, true),
        None => ((side / 60.0).clamp(1e-4, 0.5), true),
    }
}

impl PixelApp {
    /// Opens the card for `op` on the active mesh object.
    pub fn open_job_card(&mut self, op: &str) -> Result<Value, String> {
        let registry = std::sync::Arc::clone(self.session.registry());
        if !runs_in_card(&registry, op) {
            return Err(format!("`{op}` {}", t("does not run in the background on a mesh")));
        }
        let doc = self.doc();
        let k = self.active_object().filter(|k| doc.object_mesh(*k).is_some()).ok_or(t("select a mesh first"))?;
        let mut params = registry.get(op).map(|o| o.default_params()).unwrap_or(Value::Null);
        if let Some(m) = params.as_object_mut() {
            m.remove("target");
        }
        let keep_running = self.job_card.running.take();
        self.job_card = crate::jobcard::JobCard { op: Some(op.to_owned()), target: Some(k), params, running: keep_running, ..Default::default() };
        Ok(json!({ "op": op, "target": self.doc().object(k).map(|o| o.public_id()) }))
    }

    /// Runs the card's op in the background (replacing the card's last run
    /// when nothing changed since).
    pub fn run_job_card(&mut self) -> Result<Value, String> {
        if self.job_card.running.is_some() {
            return Err(t("a run is going on").to_owned());
        }
        let op = self.job_card.op.clone().ok_or(t("no run card is open"))?;
        let k = self.job_card.target.ok_or(t("no object"))?;
        let id = self.doc().object(k).map(|o| o.public_id()).ok_or(t("the object is gone"))?;
        let mut args = self.job_card.params.as_object().cloned().unwrap_or_default();
        args.insert("target".into(), json!(id));
        args.retain(|_, v| !v.is_null());
        let label = self.session.registry().get(&op).map_or_else(|_| op.clone(), |o| o.meta().label.to_owned());
        self.job_card.replaced = false;
        if self.job_card.last == Some(self.session.revision()) && self.session.undo().is_ok() {
            self.job_card.replaced = true;
        }
        let job = self.session.spawn_job(vec![OpCall::new(&op, Value::Object(args))], &label);
        self.job_card.running = Some(job);
        self.job_card.cancelling = false;
        self.job_card.error = None;
        self.job_card.summary = None;
        self.set_status(format!("{label}..."));
        Ok(json!({ "running": op }))
    }

    /// Asks the running op to stop at its next check.
    pub fn cancel_job_card(&mut self) {
        if let Some(j) = &self.job_card.running {
            j.handle().cancel();
            self.job_card.cancelling = true;
        }
    }

    /// Takes in a finished run. Call once per frame.
    pub(crate) fn job_card_tick(&mut self, ctx: &egui::Context) {
        let Some(mut job) = self.job_card.running.take() else { return };
        match self.session.poll_job(&mut job) {
            JobStatus::Pending(_) => {
                self.job_card.running = Some(job);
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            JobStatus::Done(r) => {
                self.committed(&r);
                self.prune_selection();
                self.job_card.last = Some(r.revision);
                self.job_card.summary = r.results.last().map(|x| x.output.summary.clone());
            }
            JobStatus::Failed(e) => {
                if self.job_card.replaced {
                    let _ = self.session.redo();
                }
                let msg = if e.error.code == p3d_ops::ErrorCode::Cancelled { t("Cancelled.").to_owned() } else { e.error.to_string() };
                self.set_status(msg.clone());
                self.job_card.error = Some(msg);
            }
        }
    }

    /// The faces the card's run is expected to make (area over the length
    /// squared), the length it uses and whether that is automatic, the
    /// faces now, and open edges (which the remesh ops refuse).
    ///
    /// Complexity: O(corners + edges) of the mesh once per mesh, then O(1).
    pub fn job_card_estimate(&mut self) -> Option<(u64, f64, bool, usize, usize)> {
        let k = self.job_card.target?;
        let doc = std::sync::Arc::clone(self.session.doc());
        let mesh = doc.object(k).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m))?;
        let key = std::sync::Arc::as_ptr(mesh) as usize;
        let measured = match self.job_card.measured {
            Some((m, a, s, o)) if m == key => (a, s, o),
            _ => {
                let (a, s, o) = measure(mesh);
                self.job_card.measured = Some((key, a, s, o));
                (a, s, o)
            }
        };
        self.estimate_from(measured, mesh.face_count())
    }

    /// The estimate from a mesh's area, largest side and open edges.
    fn estimate_from(&self, (area, side, open): (f64, f64, usize), faces_now: usize) -> Option<(u64, f64, bool, usize, usize)> {
        let op = self.job_card.op.as_deref()?;
        let (len, auto) = length_used(op, &self.job_card.params, side);
        let faces = if len > 0.0 { (area / (len * len)).round() as u64 } else { 0 };
        Some((faces, len, auto, faces_now, open))
    }

    /// The card's state for agents (`ui.inspect`).
    ///
    /// Complexity: O(corners + edges) of the target mesh.
    pub(crate) fn job_card_state(&self) -> Value {
        let doc = self.doc();
        let mesh = self.job_card.target.and_then(|k| doc.object(k)).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m));
        let estimate = mesh.and_then(|m| self.estimate_from(measure(m), m.face_count()));
        let c = &self.job_card;
        json!({
            "op": c.op,
            "target": c.target.and_then(|k| self.doc().object(k)).map(|o| o.public_id()),
            "params": c.params,
            "running": c.running.is_some(),
            "progress": c.progress(),
            "estimate": estimate.map(|(f, l, auto, now, open)| json!({ "faces": f, "length": l, "auto": auto, "facesNow": now, "openEdges": open })),
            "summary": c.summary,
            "error": c.error,
        })
    }
}

/// Draws the card over the viewport `rect` while it is open.
pub fn show(app: &mut PixelApp, ui: &egui::Ui, rect: egui::Rect) {
    let Some(op) = app.job_card.op.clone() else { return };
    if !rect.is_positive() {
        return;
    }
    let registry = std::sync::Arc::clone(app.session.registry());
    let Ok(o) = registry.get(&op) else { return };
    let tk = Tokens::get();
    let schema = o.params_schema();
    let estimate = app.job_card_estimate();
    let object = app.job_card.target.and_then(|k| app.doc().object(k)).map(|o| o.name().to_owned()).unwrap_or_default();
    let title = format!("{}: {object}", o.meta().label);
    let mut params = app.job_card.params.clone();
    let (mut run, mut cancel, mut close) = (false, false, false);
    let running = app.job_card.running.is_some();
    let again = app.job_card.last == Some(app.session.revision());
    let shown = egui::Area::new(egui::Id::new("p3d_job_card"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(rect.left_bottom() + vec2(12.0, -12.0))
        .show(ui.ctx(), |ui| {
            ui.set_width(290.0);
            ui.set_max_height((rect.height() * 0.7).max(160.0));
            widgets::section(ui, "job-card", &title, true, |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    let out = schema_form::form(ui, egui::Id::new(("job-card", op.as_str())), &schema, &schema, &mut params, &["target"]);
                    if out.changed {
                        app.job_card.params = params.clone();
                    }
                });
                if let Some((faces, len, auto, now, open)) = estimate {
                    let length = p3d_math::units::format_length(len);
                    let line =
                        if auto { format!("{} {length} ({})", t("Edge about"), t("automatic")) } else { format!("{} {length}", t("Edge about")) };
                    ui.label(RichText::new(line).font(theme::small()).color(tk.text_dim));
                    let expect = format!(
                        "{} {} {} ({} {})",
                        t("About"),
                        crate::chrome::thousands(faces as usize),
                        t("faces"),
                        t("now"),
                        crate::chrome::thousands(now)
                    );
                    ui.label(RichText::new(expect).color(tk.text));
                    if open > 0 {
                        let warn = format!(
                            "{} {} {}",
                            t("The mesh has"),
                            crate::chrome::thousands(open),
                            t("open edges; close it first (Mesh > Clean Up > Auto Clean).")
                        );
                        ui.add(egui::Label::new(RichText::new(warn).font(theme::small()).color(tk.health_warn)).wrap());
                    }
                }
                if let Some(p) = app.job_card.progress() {
                    let text = if app.job_card.cancelling { t("Cancelling...").to_owned() } else { format!("{} {:.0}%", t("Running"), p * 100.0) };
                    ui.add(egui::ProgressBar::new(p).text(text).animate(true));
                }
                if let Some(s) = &app.job_card.summary {
                    ui.add(egui::Label::new(RichText::new(s).font(theme::small()).color(tk.text_dim)).wrap());
                }
                if let Some(e) = &app.job_card.error {
                    ui.add(egui::Label::new(RichText::new(e).font(theme::small()).color(tk.health_error)).wrap());
                }
                ui.horizontal(|ui| {
                    if running {
                        let b = ui.add_enabled(!app.job_card.cancelling, egui::Button::new(t("Cancel")));
                        crate::control::record_widget(ui, &b, t("Cancel run"), "button", None);
                        cancel = b.clicked();
                    } else {
                        let closed = estimate.is_some_and(|e| e.4 == 0);
                        let label = if again { t("Run again") } else { t("Run") };
                        let b = ui.add_enabled(closed, egui::Button::new(label));
                        crate::control::record_widget(ui, &b, label, "button", None);
                        let tip = if again {
                            t("Replace the last run with these settings (still one undo step)")
                        } else {
                            t("Run in the background; it is one undo step")
                        };
                        run = b.on_hover_text(tip).clicked();
                    }
                    let c = ui.button(t("Close"));
                    crate::control::record_widget(ui, &c, t("Close run card"), "button", None);
                    close = c.clicked();
                });
            });
        });
    let _ = shown;
    if run && let Err(e) = app.run_job_card() {
        app.job_card.error = Some(e);
    }
    if cancel {
        app.cancel_job_card();
    }
    if close {
        app.cancel_job_card();
        app.job_card.op = None;
    }
}

/// The card's command parameters merged into its settings (`job.run`).
pub fn merge(params: &mut Value, more: &Value) {
    let Some(more) = more.as_object() else { return };
    if !params.is_object() {
        *params = Value::Object(Map::new());
    }
    if let Some(p) = params.as_object_mut() {
        for (k, v) in more {
            if k != "target" {
                p.insert(k.clone(), v.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use std::time::Duration;

    fn finish(a: &mut PixelApp) {
        let ctx = egui::Context::default();
        for _ in 0..4000 {
            a.job_card_tick(&ctx);
            if !a.job_card.busy() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the run did not finish");
    }

    #[test]
    fn only_ops_on_a_required_target_run_in_the_card() {
        let session = p3d_engine::Session::new();
        let r = session.registry();
        assert!(runs_in_card(r, "remesh.quad") && runs_in_card(r, "sdf.from_mesh"));
        // Long running, but it makes new objects; its target is optional.
        assert!(!runs_in_card(r, "sdf.silhouettes"));
        assert!(!runs_in_card(r, "prim.box"));
    }

    #[test]
    fn remeshes_run_in_the_background_as_one_step() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.torus", json!({ "name": "ring", "major_radius": 0.3, "minor_radius": 0.1 })).unwrap();
        let steps = a.session.history().len();
        a.run("job.open", json!({ "op": "remesh.quad" })).unwrap();
        assert_eq!(a.job_card.params["iterations"], json!(12), "the op's defaults");
        // The estimate before running: area over the edge length squared.
        assert!(a.job_card_estimate().is_some_and(|e| e.2), "automatic edge at first");
        a.job_card.params["edge"] = json!(0.04);
        let (faces, len, auto, _, open) = a.job_card_estimate().unwrap();
        assert!((len - 0.04).abs() < 1e-9 && !auto && open == 0);
        let area = 4.0 * std::f64::consts::PI.powi(2) * 0.3 * 0.1;
        assert!((faces as f64 - area / 0.0016).abs() < area / 0.0016 * 0.05, "{faces}");
        // Run, then run again with another edge: still one undo step.
        a.run("job.run", Value::Null).unwrap();
        assert!(a.job_card.busy());
        finish(&mut a);
        assert_eq!(a.session.history().len(), steps + 1);
        let first = a.doc().object_mesh(a.doc().resolve("ring").unwrap()).unwrap().face_count();
        assert!(a.job_card.summary.as_deref().is_some_and(|s| s.contains("poles")));
        a.run("job.run", json!({ "edge": 0.06 })).unwrap();
        finish(&mut a);
        let second = a.doc().object_mesh(a.doc().resolve("ring").unwrap()).unwrap().face_count();
        assert_eq!(a.session.history().len(), steps + 1, "the second run replaced the first");
        assert!(second < first, "a longer edge, fewer faces: {second} against {first}");
        // Voxel remesh too.
        a.run("job.open", json!({ "op": "sdf.from_mesh" })).unwrap();
        a.run("job.run", json!({ "detail": 0.05 })).unwrap();
        finish(&mut a);
        assert_eq!(a.session.history().len(), steps + 2);
    }

    #[test]
    fn open_meshes_and_cancels_leave_the_scene_as_it_was() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.plane", json!({ "name": "floor" })).unwrap();
        a.run("job.open", json!({ "op": "remesh.quad" })).unwrap();
        assert!(a.job_card_estimate().is_some_and(|e| e.4 > 0), "open edges are reported before running");
        let (steps, revision) = (a.session.history().len(), a.session.revision());
        a.run("job.run", Value::Null).unwrap();
        finish(&mut a);
        assert!(a.job_card.error.as_deref().is_some_and(|e| e.contains("open edges")));
        assert_eq!((a.session.history().len(), a.session.revision()), (steps, revision), "a refused run changes nothing");
        a.run("prim.torus", json!({ "name": "ring" })).unwrap();
        a.run("job.open", json!({ "op": "remesh.quad" })).unwrap();
        let (steps, revision) = (a.session.history().len(), a.session.revision());
        a.run("job.run", json!({ "edge": 0.01 })).unwrap();
        a.run("job.cancel", Value::Null).unwrap();
        finish(&mut a);
        // The run stops at its next check, unless it was already done.
        match a.job_card.error.as_deref() {
            Some(e) => {
                assert!(e.contains("Cancelled"), "{e}");
                assert_eq!(a.session.revision(), revision, "a cancelled run changes nothing");
            }
            None => assert_eq!(a.session.history().len(), steps + 1),
        }
        assert!(a.run("job.open", json!({ "op": "prim.box" })).is_err(), "only long ops on a mesh");
    }
}
