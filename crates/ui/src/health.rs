//! The Health panel: the same diagnostics agents get (project spec 17.10,
//! 20.6), as a traffic-light list. Each finding shows its part, what is
//! wrong and the likely cause; **Fix** applies the first fix the engine
//! predicted, as one undo step. Checks run on a background thread whenever
//! the document changes, so editing never waits for them.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Align2, Color32, Rect, RichText, Sense, pos2, vec2};
use p3d_perceive::diagnose::{Finding, Severity};
use p3d_scene::Document;

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

/// The Health panel body.
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
    let mut fix: Option<String> = None;
    let mut select: Option<String> = None;
    egui::ScrollArea::vertical().id_salt("health").auto_shrink([false, false]).show(ui, |ui| {
        for f in &app.health.findings {
            let frame = egui::Frame::NONE.fill(tk.chrome_dark.gamma_multiply(0.6)).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 4));
            let resp = frame
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, color(Some(f.severity)));
                        ui.label(RichText::new(format!("{} {}", f.id, f.code)).font(theme::mono()).color(tk.text));
                        ui.label(RichText::new(&f.part).color(tk.text_dim));
                    });
                    ui.label(RichText::new(&f.message).color(tk.text));
                    if let Some(c) = &f.cause {
                        ui.label(RichText::new(format!("{}: {c}", t("cause"))).font(theme::small()).color(tk.text_faint));
                    }
                    match f.fixes.first() {
                        Some(x) => {
                            ui.horizontal(|ui| {
                                let b = ui.add_enabled(current, egui::Button::new(format!("{}: {}", t("Fix"), x.summary)));
                                let tip = x.predicted.clone().unwrap_or_else(|| t("The result is predicted when the check finishes.").to_owned());
                                crate::control::record_widget(ui, &b, &format!("Fix {}", f.id), "button", None);
                                if b.on_hover_text(tip).clicked() {
                                    fix = Some(f.id.clone());
                                }
                            });
                        }
                        None => {
                            if let Some(h) = &f.hint {
                                ui.label(RichText::new(h).font(theme::small()).color(tk.text_faint));
                            }
                        }
                    }
                })
                .response;
            if resp.interact(Sense::click()).clicked() {
                select = Some(f.part.clone());
            }
            ui.add_space(4.0);
        }
    });
    if let Some(part) = select
        && let Ok(k) = app.doc().resolve(&part)
    {
        app.handle_viewport(vec![crate::viewport::ViewportEvent::Picked { object: Some(k), add: false }]);
    }
    if let Some(id) = fix {
        match p3d_engine::predict::apply_fix(&mut app.session, None, &id, 0) {
            Ok((f, r)) => {
                let summary = r.results.first().map(|x| x.output.summary.clone()).unwrap_or_default();
                app.set_status(format!("{} {} {}: {summary}", t("Fixed"), f.id, f.code));
                app.last_op = None;
                app.prune_selection();
            }
            Err(e) => app.set_status(e.to_string()),
        }
    }
}

/// Severity badges at the findings' positions in the viewport.
pub fn badges(app: &PixelApp, ui: &egui::Ui) {
    let rect = app.viewport.rect;
    if !rect.is_positive() || app.health.key != Some(app.scene_key()) {
        return;
    }
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height());
    let p = ui.painter_at(rect);
    for f in app.health.findings.iter().take(64) {
        let c = vp.project_point3(f.at);
        if !(0.0..=1.0).contains(&c.z) || c.x.abs() > 1.0 || c.y.abs() > 1.0 {
            continue;
        }
        let at = pos2(rect.left() + (c.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - c.y * 0.5) * rect.height());
        let col = color(Some(f.severity));
        p.circle_filled(at, 7.0, col);
        p.text(at, Align2::CENTER_CENTER, "!", egui::FontId::proportional(10.0), Color32::from_rgb(0x12, 0x14, 0x18));
        let label = Rect::from_min_size(at + vec2(10.0, -8.0), vec2(0.0, 16.0));
        p.text(label.left_center(), Align2::LEFT_CENTER, &f.id, theme::small(), col);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checks_run_in_the_background_and_fixes_apply() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        // A box with its bottom deleted: an open boundary to find and fill.
        app.run("prim.box", json!({ "name": "bin" })).unwrap();
        app.run("edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "bottom" } } })).unwrap();
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
        let open = app.health.findings.iter().find(|f| f.code == "OPEN_BOUNDARY").cloned();
        let Some(f) = open else { panic!("no open boundary found: {:?}", app.health.findings) };
        assert!(!f.fixes.is_empty());
        assert_eq!(app.health.worst().map(|s| s <= Severity::Warn), Some(true));
        let before = app.session.history().len();
        p3d_engine::predict::apply_fix(&mut app.session, None, &f.id, 0).unwrap();
        assert_eq!(app.session.history().len(), before + 1, "one undo step");
    }
}
