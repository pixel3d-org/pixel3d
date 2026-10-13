//! The Outline tab of Properties (and the outline part of the Spec tab):
//! the view buttons, the canvas of the selected view with its point
//! buttons, thumbnails of the other views for silhouettes, and the shape's
//! fields as sliders (cup, bend, twist, thickness and detail for shells;
//! round and detail for silhouettes) that preview while dragged.

use egui::{RichText, vec2};
use serde_json::{Value, json};

use super::{Apply, Kind, LayerKey, ViewId, canvas};
use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::widgets::{prop_row, section as panel};

/// A command the tab asks for this frame.
type Command = (&'static str, Value);

/// The tab body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let Some(kind) = app.outline.kind else {
        ui.label(RichText::new(t("Select an object made by Shell or Silhouettes (Add > Shell, Add > Silhouettes).")).color(tk.text_dim));
        return;
    };
    let view = app.outline.view.unwrap_or(kind.views()[0]);
    let mut command: Option<Command> = None;
    let title = match kind {
        Kind::Shell => t("Outline"),
        Kind::Silhouettes => t("Silhouettes"),
    };
    panel(ui, "outline-canvas", title, true, |ui| {
        // The views (silhouettes): selected, present or missing.
        if kind == Kind::Silhouettes {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for v in kind.views() {
                    let present = app.outline.layers_in(*v).next().is_some();
                    let text = if present { v.label().to_owned() } else { format!("+ {}", v.label()) };
                    let r = ui.selectable_label(view == *v, text);
                    crate::control::record_widget(ui, &r, v.label(), "tab", Some(view == *v));
                    if r.clicked() {
                        command = Some(if present {
                            ("outline.select_view", json!({ "view": v.name() }))
                        } else {
                            ("outline.add", json!({ "layer": v.name() }))
                        });
                    }
                }
            });
            ui.add_space(4.0);
        }
        let w = ui.available_width();
        canvas::show(app, ui, view, vec2(w, (w * 0.9).clamp(160.0, 420.0)), true);
        ui.add_space(4.0);
        command = command.take().or_else(|| point_buttons(app, ui, kind, view));
        // The other views, small: a click opens one.
        if kind == Kind::Silhouettes {
            let others: Vec<ViewId> = kind.views().iter().copied().filter(|v| *v != view && app.outline.layers_in(*v).next().is_some()).collect();
            if !others.is_empty() {
                ui.horizontal(|ui| {
                    let size = ((ui.available_width() - 6.0) / 2.0).min(140.0);
                    for v in others {
                        if canvas::show(app, ui, v, vec2(size, size * 0.75), false).clicked() {
                            command = Some(("outline.select_view", json!({ "view": v.name() })));
                        }
                    }
                });
            }
        }
        let note = |ui: &mut egui::Ui, text: &str, color| {
            ui.add(egui::Label::new(RichText::new(text).font(theme::small()).color(color)).wrap());
        };
        note(
            ui,
            t(
                "Drag points and handles. Double-click a segment to add a point, a point to make it smooth or a corner. Points snap to the dashed extents of the other views, and to the grid with Snap on or Cmd held.",
            ),
            tk.text_faint,
        );
        if !app.outline.later.is_empty() {
            note(ui, &format!("{} {}", t("Editing the outline rebuilds the mesh and drops:"), app.outline.later.join(", ")), tk.health_warn);
        }
        if let Some(e) = &app.outline.error {
            note(ui, e, tk.health_error);
        }
        if app.outline.busy() {
            note(ui, t("Rebuilding..."), tk.text_faint);
        }
    });
    fields(app, ui, kind);
    if let Some((id, params)) = command
        && let Err(e) = app.outline_command(id, &params)
    {
        app.set_status(e);
    }
}

/// The selected point's buttons, and adding or removing holes.
fn point_buttons(app: &PixelApp, ui: &mut egui::Ui, kind: Kind, view: ViewId) -> Option<Command> {
    let e = &app.outline;
    let sel = e.selected.filter(|s| e.layers.get(s.layer).is_some_and(|l| l.key.view() == view));
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        let mut button = |ui: &mut egui::Ui, label: &str, tip: &str, enabled: bool, cmd: Command| {
            let r = ui.add_enabled(enabled, egui::Button::new(RichText::new(label).font(theme::small())));
            crate::control::record_widget(ui, &r, label, "button", None);
            if r.on_hover_text(tip).clicked() {
                out = Some(cmd);
            }
        };
        let smooth = sel.and_then(|s| e.node(s)).is_some_and(|n| n.smooth);
        let at = |s: super::Sel| json!({ "layer": e.layers[s.layer].key.name(), "contour": s.contour, "index": s.node });
        let (s_params, seg_params) = match sel {
            Some(s) => (at(s), json!({ "layer": e.layers[s.layer].key.name(), "contour": s.contour, "segment": s.node })),
            None => (Value::Null, Value::Null),
        };
        button(ui, t("Add point"), t("Add a point halfway to the next one"), sel.is_some(), ("outline.insert_point", seg_params));
        button(
            ui,
            t("Delete point"),
            t("Delete the selected point (Delete over the canvas)"),
            sel.is_some(),
            ("outline.delete_point", s_params.clone()),
        );
        let toggle = if smooth { t("Make corner") } else { t("Make smooth") };
        button(ui, toggle, t("Turn the selected point smooth (with handles) or into a corner"), sel.is_some(), ("outline.toggle_smooth", s_params));
        if kind == Kind::Shell {
            button(ui, t("Add hole"), t("Cut a hole out of the shell"), true, ("outline.add", json!({ "layer": "hole" })));
            if let Some(s) = sel
                && let LayerKey::Hole(_) = e.layers[s.layer].key
            {
                let name = e.layers[s.layer].key.name();
                button(ui, t("Remove hole"), t("Remove the selected point's hole"), true, ("outline.remove", json!({ "layer": name })));
            }
        } else if view != ViewId::Front && e.layers_in(view).next().is_some() {
            button(
                ui,
                t("Remove view"),
                t("Build from the other views only"),
                e.layers.len() > 2,
                ("outline.remove", json!({ "layer": view.name() })),
            );
        }
    });
    out
}

/// A length value in meters (`"3mm"` read too).
fn length(v: Option<&Value>) -> Option<f64> {
    serde_json::from_value::<p3d_ops::params::Length>(v?.clone()).ok().map(|l| l.0)
}

/// An angle value in degrees.
fn degrees(v: Option<&Value>) -> Option<f64> {
    serde_json::from_value::<p3d_ops::params::Angle>(v?.clone()).ok().map(|a| a.0.to_degrees())
}

/// One slider row; returns the new value and whether it is still dragged
/// when it changed.
fn slider(ui: &mut egui::Ui, label: &str, desc: &str, value: f64, range: std::ops::RangeInclusive<f64>, unit: &'static str) -> Option<(f64, bool)> {
    let mut v = value;
    let r = prop_row(ui, label, desc, |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 64.0).max(60.0);
        let s = egui::Slider::new(&mut v, range).clamping(egui::SliderClamping::Never);
        let s = match unit {
            "mm" => s
                .custom_formatter(|v, _| format!("{:.1} mm", v * 1000.0))
                .custom_parser(|s| s.trim().trim_end_matches("mm").trim().parse::<f64>().ok().map(|v| v / 1000.0)),
            "deg" => s.suffix("\u{b0}").fixed_decimals(0),
            _ => s.fixed_decimals(2),
        };
        ui.add(s)
    });
    crate::control::record_widget(ui, &r, label, "slider", None);
    if r.changed() {
        return Some((v, r.dragged()));
    }
    if r.drag_stopped() {
        return Some((v, false));
    }
    None
}

/// The shape's fields as sliders: a drag previews, letting go rebuilds.
fn fields(app: &mut PixelApp, ui: &mut egui::Ui, kind: Kind) {
    let tk = Tokens::get();
    let f = app.outline.fields.clone();
    let mut set: Option<(&'static str, Value, bool)> = None;
    panel(ui, "outline-fields", t("Shape"), true, |ui| {
        if kind == Kind::Shell {
            let cup = f.get("cup").and_then(Value::as_f64).unwrap_or(0.0);
            if let Some((v, d)) =
                slider(ui, t("Cup"), t("Edges curl toward +Z by this share of the half width (negative curls back)"), cup, -1.0..=1.0, "")
            {
                set = Some(("cup", json!(v), d));
            }
            let bend = f.get("bend").and_then(Value::as_f64).unwrap_or(0.0);
            if let Some((v, d)) = slider(ui, t("Bend"), t("The tip lifts toward +Z by this share of the length"), bend, -1.0..=1.0, "") {
                set = Some(("bend", json!(v), d));
            }
            let twist = degrees(f.get("twist")).unwrap_or(0.0);
            if let Some((v, d)) = slider(ui, t("Twist"), t("Turn about the length axis at the tip"), twist, -180.0..=180.0, "deg") {
                set = Some(("twist", json!(v), d));
            }
            let thick = length(f.get("thickness")).unwrap_or(0.0);
            if let Some((v, d)) = slider(ui, t("Thickness"), t("0 makes a one-sided card facing +Z"), thick, 0.0..=0.02, "mm") {
                set = Some(("thickness", json!(v.max(0.0)), d));
            }
        } else {
            let round = length(f.get("round")).unwrap_or(0.0);
            if let Some((v, d)) = slider(ui, t("Round"), t("Rounds every edge by this radius"), round, 0.0..=0.02, "mm") {
                set = Some(("round", json!(v.max(0.0)), d));
            }
        }
        // Detail: automatic, or a set length.
        let detail = length(f.get("detail"));
        let auto_value = match kind {
            Kind::Shell => app.outline.size() / 16.0,
            Kind::Silhouettes => app.outline.size() / 64.0,
        }
        .clamp(1e-4, 1.0);
        let mut auto = detail.is_none();
        let r = prop_row(ui, t("Auto detail"), t("Pick the detail from the size"), |ui| ui.checkbox(&mut auto, ""));
        crate::control::record_widget(ui, &r, t("Auto detail"), "checkbox", Some(auto));
        if r.changed() {
            set = Some(("detail", if auto { Value::Null } else { json!(auto_value) }, false));
        }
        if let Some(d) = detail
            && let Some((v, dragging)) = slider(ui, t("Detail"), t("Edge length of the mesh: smaller is finer"), d, 0.0005..=0.02, "mm")
        {
            set = Some(("detail", json!(v.clamp(1e-4, 1.0)), dragging));
        }
        if kind == Kind::Silhouettes {
            ui.add(
                egui::Label::new(RichText::new(t("While dragging, the preview is three times coarser.")).font(theme::small()).color(tk.text_faint))
                    .wrap(),
            );
        }
    });
    if let Some((key, v, dragging)) = set
        && (dragging || app.outline.live || f.get(key) != Some(&v))
    {
        if v.is_null() {
            app.outline.fields.remove(key);
        } else {
            app.outline.fields.insert(key.to_owned(), v);
        }
        app.apply_outline(if dragging { Apply::Live } else { Apply::Commit(t("Shape settings")) });
    }
}
