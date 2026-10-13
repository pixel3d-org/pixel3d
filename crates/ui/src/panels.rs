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
