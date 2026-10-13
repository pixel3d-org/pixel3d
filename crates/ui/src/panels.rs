//! Dock panel bodies: Outliner, Properties, History and Console.
//! Panels read the session and act only through [`PixelApp::run`].

use egui::{Align2, Sense, pos2, vec2};
use serde_json::Value;

use crate::PixelApp;
use crate::i18n::t;

use crate::theme::{self, Tokens};

const ROW: f32 = 22.0;

/// History: the undo list, newest last (the current state). Clicking an
/// entry undoes back to the state after it; redo brings later ones back.
pub fn history(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let entries = app.session.history();
    let mut undo_to: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).stick_to_bottom(true).show_rows(
        ui,
        ROW,
        entries.len() + 1,
        |ui, range| {
            for i in range {
                let label = if i == 0 { t("Opened or new").to_owned() } else { entries[i - 1].label.clone() };
                let is_current = i == entries.len();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                if is_current {
                    ui.painter().rect_filled(rect, 5.0, tk.accent.gamma_multiply(0.14));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, tk.control.gamma_multiply(0.4));
                }
                ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, &label, theme::body(), tk.text);
                crate::control::record_widget(ui, &resp, &label, "listitem", Some(is_current));
                if resp.clicked() {
                    undo_to = Some(i);
                }
            }
        },
    );
    if let Some(i) = undo_to {
        // Entry i (1-based) is the state after transaction i; undo the ones after it.
        let steps = entries.len().saturating_sub(i);
        for _ in 0..steps {
            if app.run("edit.undo", Value::Null).is_err() {
                break;
            }
        }
    }
}

/// Console: op results and messages, newest last.
pub fn console(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let full = ui.available_rect_before_wrap();
    let input_h = crate::widgets::SEARCH + 8.0;
    let list = egui::Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - input_h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        let lines = &app.console;
        egui::ScrollArea::vertical().id_salt("console").auto_shrink([false, false]).stick_to_bottom(true).show_rows(
            ui,
            16.0,
            lines.len(),
            |ui, range| {
                for line in &lines[range] {
                    // Typed commands stand out; errors in the error color.
                    let color = if line.starts_with("> ") {
                        tk.text
                    } else if line.trim_start().starts_with(t("error")) {
                        tk.health_error
                    } else {
                        tk.text_dim
                    };
                    ui.label(egui::RichText::new(line).font(theme::mono()).color(color));
                }
            },
        );
    });
    let input = egui::Rect::from_min_max(pos2(full.left(), full.bottom() - input_h + 6.0), full.max);
    ui.scope_builder(egui::UiBuilder::new().max_rect(input), |ui| crate::cmdline::show(app, ui));
}

/// A note about what an editor will hold once its tools exist.
fn coming(ui: &mut egui::Ui, text: &str) {
    let tk = Tokens::get();
    ui.add_space(4.0);
    ui.add(egui::Label::new(egui::RichText::new(text).font(theme::small()).color(tk.text_faint)).wrap());
}

/// The UV editor: the 0 to 1 UV square and the active mesh's UV layers.
/// Unwrapping, packing and editing arrive with the UV tools.
pub fn uv_editor(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let mesh = app.active_object().and_then(|k| app.doc().object_mesh(k).map(|m| m.uv_layer_count()));
    let full = ui.available_rect_before_wrap();
    let side = (full.width().min(full.height()) - 60.0).max(80.0);
    let square = egui::Rect::from_center_size(full.center() - vec2(0.0, 14.0), vec2(side, side));
    let p = ui.painter();
    p.rect_filled(square, 2.0, tk.canvas);
    for i in 0..=10 {
        let f = i as f32 / 10.0;
        let c = if i % 5 == 0 { tk.grid_major } else { tk.grid_minor };
        p.vline(square.left() + f * side, square.y_range(), egui::Stroke::new(1.0, c));
        p.hline(square.x_range(), square.top() + f * side, egui::Stroke::new(1.0, c));
    }
    p.text(square.left_bottom() + vec2(0.0, 6.0), Align2::LEFT_TOP, "0, 0", theme::small(), tk.text_faint);
    p.text(square.right_top() - vec2(0.0, 6.0), Align2::RIGHT_BOTTOM, "1, 1", theme::small(), tk.text_faint);
    let text = match mesh {
        Some(n) => format!("{n} {}", t("UV layers on the active mesh. Unwrapping, packing and editing UVs arrive with the UV tools.")),
        None => t("Select a mesh to see its UVs. Unwrapping, packing and editing UVs arrive with the UV tools.").to_owned(),
    };
    p.text(pos2(full.left() + 4.0, full.bottom() - 14.0), Align2::LEFT_CENTER, text, theme::small(), tk.text_faint);
    ui.allocate_rect(full, Sense::hover());
}

/// The timeline: a frame ruler with the playhead at frame 0. Keys, clips
/// and playback arrive with animation.
pub fn timeline(_app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let full = ui.available_rect_before_wrap();
    let ruler = egui::Rect::from_min_size(full.min + vec2(0.0, 4.0), vec2(full.width(), 22.0));
    let p = ui.painter();
    p.rect_filled(ruler, 4.0, tk.chrome_dark);
    let frames = 250.0;
    for f in (0..=250).step_by(10) {
        let x = ruler.left() + 8.0 + (ruler.width() - 16.0) * f as f32 / frames;
        let major = f % 50 == 0;
        p.vline(x, (ruler.bottom() - if major { 10.0 } else { 5.0 })..=ruler.bottom(), egui::Stroke::new(1.0, tk.text_faint));
        if major {
            p.text(pos2(x + 3.0, ruler.top() + 2.0), Align2::LEFT_TOP, f.to_string(), theme::small(), tk.text_dim);
        }
    }
    let head = ruler.left() + 8.0;
    p.vline(head, ruler.top()..=full.bottom(), egui::Stroke::new(1.5, tk.accent));
    p.text(
        pos2(full.left() + 4.0, ruler.bottom() + 16.0),
        Align2::LEFT_CENTER,
        t("Keys, clips and playback arrive with animation."),
        theme::small(),
        tk.text_faint,
    );
    ui.allocate_rect(full, Sense::hover());
}

/// Bake: the high and low poly pairs found by their tags. Baking normal,
/// AO and curvature maps arrives with the bake tools.
pub fn bake(app: &mut PixelApp, ui: &mut egui::Ui) {
    let doc = app.doc();
    let tagged =
        |tag: &str| -> Vec<String> { doc.objects().filter(|(_, o)| o.tags.iter().any(|x| x == tag)).map(|(_, o)| o.name().to_owned()).collect() };
    let (high, low) = (tagged("high"), tagged("low"));
    crate::widgets::info(ui, t("High poly"), if high.is_empty() { t("none tagged `high`").to_owned() } else { high.join(", ") });
    crate::widgets::info(ui, t("Low poly"), if low.is_empty() { t("none tagged `low`").to_owned() } else { low.join(", ") });
    coming(ui, t("Baking normal, ambient occlusion and curvature maps from the high to the low poly arrives with the bake tools."));
}

/// Export: what an export would check now: the asset class, triangles
/// against the budget and the health of the scene. The exporters for
/// Unity, Unreal and Godot arrive with their profiles.
pub fn export(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let budget_tris = app.stats().budget_tris;
    let doc = app.doc();
    let budget = doc.settings.tri_budget() as usize;
    crate::widgets::info(ui, t("Asset class"), doc.settings.asset_class.as_str());
    crate::widgets::info(ui, t("Triangles"), format!("{} / {}", crate::chrome::thousands(budget_tris), crate::chrome::thousands(budget)));
    let worst = app.health.worst();
    let health = match worst {
        None if app.health.key.is_some() => t("no problems").to_owned(),
        None => t("checking").to_owned(),
        Some(s) => format!(
            "{} {}",
            app.health.findings.len(),
            if s == p3d_perceive::diagnose::Severity::Error { t("findings, errors first") } else { t("findings") }
        ),
    };
    ui.horizontal(|ui| {
        crate::widgets::info(ui, t("Health"), health);
    });
    if budget_tris > budget {
        ui.label(egui::RichText::new(t("Over the triangle budget of the asset class.")).font(theme::small()).color(tk.health_warn));
    }
    coming(ui, t("Exporting to Unity, Unreal and Godot (FBX, glTF) arrives with the engine profiles."));
}
