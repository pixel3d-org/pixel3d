//! Quick Properties: Blender's "Adjust Last Operation" panel, floating at
//! the bottom left of the viewport after an operation. Its parameters (the
//! target and the selection left out) are a form generated from the op's
//! schema; changing one runs the op again in place of the last one, so it
//! stays one undo step. It goes away once anything else changes the scene.

use egui::{Align2, Rect, vec2};

use crate::PixelApp;
use crate::i18n::t;
use crate::results;
use crate::schema_form;
use crate::theme::Tokens;
use crate::widgets;

/// Ops whose panel would only repeat what the gizmo or Properties show.
const SKIP: &[&str] = &["scene.set_transform", "scene.rename", "edit.transform", "gen.sdf", "sdf.mesh"];

/// Parameters the panel leaves out: what the op works on comes from the
/// selection (the target, the picked elements, a boolean's tool objects).
const HIDDEN: &[&str] = &["target", "select", "tool"];

/// Draws the panel over the viewport `rect` when the last op can be
/// adjusted, with its warnings and their fixes. A report (an op that
/// changed nothing) or a batch with warnings gets the same card without
/// the form; a report's card can be closed.
pub fn show(app: &mut PixelApp, ui: &egui::Ui, rect: Rect) {
    if !rect.is_positive() || app.gizmo.dragging() {
        return;
    }
    let revision = app.session.revision();
    let result = app.result.clone().filter(|r| r.revision == revision && r.worth_showing());
    // A report is newer than the op before it, so it takes the card.
    let last =
        app.last_op.clone().filter(|l| l.revision == revision && !SKIP.contains(&l.op.as_str()) && !result.as_ref().is_some_and(|r| r.read_only));
    if last.is_none() && result.is_none() {
        return;
    }
    let registry = std::sync::Arc::clone(app.session.registry());
    let op = last.as_ref().and_then(|l| registry.get(&l.op).ok());
    let tk = Tokens::get();
    let mut params = last.as_ref().map(|l| l.params.clone()).unwrap_or_default();
    let mut out = schema_form::FormOutput::default();
    let (mut fix, mut close) = (None, false);
    let title = match (op, &result) {
        (Some(op), _) => format!("{}: {}", t("Adjust"), op.meta().label),
        (None, Some(r)) => r.title(&registry),
        (None, None) => return,
    };
    let area = egui::Id::new("p3d_quick_properties");
    let shown = egui::Area::new(area)
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(rect.left_bottom() + vec2(12.0, -12.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::NONE
                .show(ui, |ui| {
                    ui.set_width(280.0);
                    // The area hands its content last frame's size; let the
                    // card grow past it (it is anchored at the bottom and moves
                    // up on the next frame).
                    let max_height = (rect.height() * 0.6).max(120.0);
                    ui.set_max_height(max_height);
                    let scroll = egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
                        // Results with warnings open by themselves.
                        let open = result.is_some();
                        widgets::section(ui, if open { "quick-result" } else { "quick-properties" }, &title, open, |ui| {
                            if let (Some(l), Some(op)) = (&last, op) {
                                let schema = op.params_schema();
                                out = schema_form::form(ui, egui::Id::new(("quick", l.op.as_str())), &schema, &schema, &mut params, HIDDEN);
                                if let Some(e) = &l.error {
                                    ui.label(egui::RichText::new(e).font(crate::theme::small()).color(tk.health_error));
                                }
                            }
                            if let Some(r) = &result {
                                fix = if r.read_only {
                                    results::report_ui(ui, r, &registry, true)
                                } else {
                                    results::warnings_ui(ui, r, &registry, true)
                                };
                                if r.read_only {
                                    let b = ui.button(egui::RichText::new(t("Close")).font(crate::theme::small()));
                                    crate::control::record_widget(ui, &b, t("Close report"), "button", None);
                                    close = b.clicked();
                                }
                            }
                        });
                    });
                    scroll.content_size
                })
                .inner
        });
    // The scroll area and the bottom-anchored card size themselves from the
    // last frame: draw again until the content stops changing size (a new
    // result, a card opening), or it would stay cut off until the next input.
    let size_key = area.with("sizes");
    let sizes = (shown.inner, shown.response.rect.size());
    if ui.ctx().data(|d| d.get_temp::<(egui::Vec2, egui::Vec2)>(size_key)) != Some(sizes) {
        ui.ctx().data_mut(|d| d.insert_temp(size_key, sizes));
        ui.ctx().request_repaint();
    }
    if out.changed {
        // Errors stay on the last op and show in the panel.
        let _ = adjust(app, params);
    }
    if let Some(f) = fix {
        let _ = results::run_fix(app, f);
    }
    if close {
        app.result = None;
    }
}

/// Runs the last op again with `params` (one undo step), keeping the picked
/// elements: running it again makes a new mesh with the same ids.
pub fn adjust(app: &mut PixelApp, params: serde_json::Value) -> Result<(), String> {
    let keep = app.edit.picked().0.clone();
    let mode = app.edit.picked().1;
    app.adjust_last(params)?;
    if let (Some(k), Some(mode)) = (app.edit.object, mode)
        && !keep.is_empty()
    {
        let doc = std::sync::Arc::clone(app.session.doc());
        if let Some(mesh) = doc.object(k).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m)) {
            app.edit.sync(Some(k), Some(mesh), app.ui.mode);
            app.edit.apply_box(keep.into_iter().collect(), mode, crate::elements::BoxOp::Set);
            app.edit.sync(Some(k), Some(mesh), app.ui.mode);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adjusting_an_element_op_keeps_the_selection_and_one_step() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        app.run("ui.mode", json!({ "mode": "face" })).unwrap();
        let k = app.doc().resolve("crate").unwrap();
        let face = app.doc().object_mesh(k).and_then(|m| m.faces().next()).unwrap();
        app.edit.click(Some(face), false);
        app.run("edit.element_op", json!({ "op": "edit.extrude" })).unwrap();
        let (picked, steps) = (app.edit.picked().0.clone(), app.session.history().len());
        let mut params = app.last_op.as_ref().unwrap().params.clone();
        params["distance"] = json!(0.25);
        adjust(&mut app, params).unwrap();
        assert_eq!(app.session.history().len(), steps, "still one undo step");
        assert_eq!(app.edit.picked().0, &picked, "the cap stays picked");
        let (lo, hi) = app.doc().object_mesh(k).and_then(|m| m.bounds()).unwrap();
        assert!((hi - lo).max_element() > 1.2, "extruded by 25 cm");
    }
}
