//! The command line at the bottom of the Console (project spec 20.2): type
//! an op id, a label or another tool's name (`add cube`, `polyCube`), with
//! parameters as `name=value` pairs or a JSON object. Suggestions from the
//! command index show above it: Tab completes, Up and Down walk the
//! suggestions (or the history when there are none), Enter runs, Esc
//! clears. Every line and its result go to the Console.

use egui::{Key, Modifiers, Sense, pos2, vec2};
use serde_json::Value;

use crate::PixelApp;
use crate::commands::{self, Command};
use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::widgets;

const SUGGESTIONS: usize = 6;
const HISTORY: usize = 100;

/// The command line's text, history and suggestions.
#[derive(Default)]
pub struct CmdLine {
    pub text: String,
    history: Vec<String>,
    /// Position while walking the history (newest is `history.len() - 1`).
    browse: Option<usize>,
    index: Option<(usize, Vec<Command>)>,
    selected: usize,
    focused: bool,
}

/// The command words typed so far (without the parameters).
fn words(text: &str) -> String {
    commands::parse_line(text).map(|(w, _)| w).unwrap_or_default()
}

/// Draws the command line in the rest of `ui` and runs entered lines.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let ops = app.session.registry().len();
    if app.cmdline.index.as_ref().is_none_or(|(n, _)| *n != ops) {
        let index = commands::index(app);
        app.cmdline.index = Some((ops, index));
    }
    let mut run: Option<String> = None;
    let line = &mut app.cmdline;
    let Some((_, index)) = &line.index else { return };
    let typed = words(&line.text);
    let suggestions: Vec<usize> =
        if line.focused && !typed.is_empty() { commands::rank(index, &typed).into_iter().take(SUGGESTIONS).collect() } else { Vec::new() };
    if line.selected >= suggestions.len() {
        line.selected = 0;
    }
    if line.focused {
        let (tab, up, down, enter, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if tab && let Some(c) = suggestions.get(line.selected).and_then(|&i| index.get(i)) {
            // Keep the parameters typed after the command words.
            let rest = line.text.trim_start().strip_prefix(typed.as_str()).unwrap_or("").trim_start().to_owned();
            line.text = if rest.is_empty() { format!("{} ", c.id) } else { format!("{} {rest}", c.id) };
        }
        if !suggestions.is_empty() && (up || down) {
            let n = suggestions.len();
            line.selected = if down { (line.selected + 1) % n } else { (line.selected + n - 1) % n };
        } else if up && !line.history.is_empty() {
            let at = line.browse.map_or(line.history.len() - 1, |b| b.saturating_sub(1));
            line.browse = Some(at);
            line.text = line.history[at].clone();
        } else if down && let Some(b) = line.browse {
            line.browse = (b + 1 < line.history.len()).then_some(b + 1);
            line.text = line.browse.map(|b| line.history[b].clone()).unwrap_or_default();
        }
        if esc {
            line.text.clear();
            line.browse = None;
        }
        if enter && !line.text.trim().is_empty() {
            // A suggestion picked with the arrows replaces the typed words.
            if line.selected > 0
                && let Some(c) = suggestions.get(line.selected).and_then(|&i| index.get(i))
            {
                let rest = line.text.trim_start().strip_prefix(typed.as_str()).unwrap_or("").to_owned();
                line.text = format!("{}{rest}", c.id);
            }
            run = Some(std::mem::take(&mut line.text));
            line.browse = None;
        }
    }
    let field =
        widgets::icon_field(ui, &mut line.text, t("Type a command, for example: add cube width=2"), crate::icons::Icon::Command, theme::mono());
    if run.is_some() {
        field.request_focus();
    }
    line.focused = field.has_focus();
    if !suggestions.is_empty() && line.focused {
        suggestions_popup(ui, field.rect, index, &suggestions, line.selected);
    }
    crate::control::record_widget(ui, &field, "Command line", "textbox", None);
    if let Some(text) = run {
        let line = &mut app.cmdline;
        if line.history.last() != Some(&text) {
            line.history.push(text.clone());
            if line.history.len() > HISTORY {
                line.history.remove(0);
            }
        }
        run_line(app, &text);
    }
}

/// Suggestions in a popup just above the field.
fn suggestions_popup(ui: &egui::Ui, field: egui::Rect, index: &[Command], list: &[usize], selected: usize) {
    let tk = Tokens::get();
    let h = list.len() as f32 * 28.0 + 10.0;
    egui::Area::new(ui.id().with("cmdline-suggestions"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(field.left(), field.top() - h - 6.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::NONE
                .fill(tk.popup)
                .stroke(egui::Stroke::new(1.0, tk.popup_border))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(5))
                .shadow(ui.visuals().popup_shadow)
                .show(ui, |ui| {
                    ui.set_width(field.width().min(520.0) - 10.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (row, &i) in list.iter().enumerate() {
                        let Some(c) = index.get(i) else { continue };
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                        if row == selected {
                            ui.painter().rect_filled(r, 6.0, tk.control);
                        }
                        let id = ui.painter().layout_no_wrap(c.id.to_owned(), theme::mono(), tk.text);
                        let idw = id.size().x;
                        ui.painter().galley(pos2(r.left() + 8.0, r.center().y - id.size().y / 2.0), id, tk.text);
                        let mut job = egui::text::LayoutJob::simple_singleline(format!("{}  {}", c.label, c.desc), theme::small(), tk.text_dim);
                        job.wrap = egui::text::TextWrapping {
                            max_width: (r.width() - idw - 30.0).max(10.0),
                            max_rows: 1,
                            break_anywhere: true,
                            overflow_character: Some('\u{2026}'),
                        };
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(r.left() + idw + 22.0, r.center().y - g.size().y / 2.0), g, tk.text_dim);
                    }
                });
        });
}

/// Runs one command line and writes it and its result to the Console.
///
/// Complexity: as [`commands::resolve`] plus the command.
pub fn run_line(app: &mut PixelApp, text: &str) {
    app.log(format!("> {text}"));
    let index = match app.cmdline.index.take() {
        Some(i) => i,
        None => (app.session.registry().len(), commands::index(app)),
    };
    let outcome = commands::parse_line(text).and_then(|(words, params)| {
        let cmd = commands::resolve(&index.1, &words)?.clone();
        commands::run_with(app, &cmd, params).map(|v| (cmd.id, v))
    });
    app.cmdline.index = Some(index);
    match outcome {
        Ok((id, v)) => app.log(format!("  {id}: {}", summary(&v))),
        Err(e) => app.log(format!("  {}: {e}", t("error"))),
    }
}

/// A result as one short line.
fn summary(v: &Value) -> String {
    let s = match v {
        Value::Null => t("done").to_owned(),
        other => other.to_string(),
    };
    if s.chars().count() > 160 { format!("{}\u{2026}", s.chars().take(160).collect::<String>()) } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_run_and_log() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        run_line(&mut app, "polyCube name=crate width=2");
        assert!(app.doc().resolve("crate").is_ok(), "{:?}", app.console);
        assert!(app.console.iter().any(|l| l.starts_with("> polyCube")));
        run_line(&mut app, "prim.box {\"width\": -1}");
        assert!(app.console.last().is_some_and(|l| l.contains(t("error"))), "{:?}", app.console.last());
        run_line(&mut app, "frame");
        assert!(app.console.last().is_some_and(|l| l.contains("view.frame")), "{:?}", app.console.last());
    }
}
