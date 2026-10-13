//! The command palette (Cmd+K or Ctrl+K, and F3 as in Blender): a search
//! over every command and op by label, alias and description, run with
//! Enter or a click. Arrow keys move the highlight; Esc or a click outside
//! closes it.

use egui::{Align2, Color32, Key, Modifiers, Rect, Sense, pos2, vec2};
use serde_json::Value;

use crate::PixelApp;
use crate::commands::{self, Command};
use crate::i18n::t;
use crate::theme::{self, Tokens};
use crate::widgets;

const WIDTH: f32 = 580.0;
const ROW: f32 = 42.0;
const VISIBLE_ROWS: f32 = 9.0;
/// Results shown at most (the best ones).
const MAX_RESULTS: usize = 200;

/// An open palette: the query, the ranked results and the highlight.
pub struct Palette {
    index: Vec<Command>,
    query: String,
    results: Vec<usize>,
    ranked_for: Option<String>,
    selected: usize,
    focus: bool,
    /// Scroll the highlight into view (after a key press).
    reveal: bool,
}

impl Palette {
    /// Opens over the commands of `app`.
    ///
    /// Complexity: O(menu entries + ops).
    pub fn new(app: &PixelApp) -> Self {
        Palette { index: commands::index(app), query: String::new(), results: Vec::new(), ranked_for: None, selected: 0, focus: true, reveal: false }
    }
}

/// Draws the palette when open and runs the chosen command.
pub fn show(app: &mut PixelApp, ui: &egui::Ui) {
    if app.palette.is_none() {
        return;
    }
    let tk = Tokens::get();
    let id = egui::Id::new("p3d_palette");
    let frame = egui::Frame::NONE
        .fill(tk.popup)
        .stroke(egui::Stroke::new(1.0, tk.popup_border))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(8))
        .shadow(ui.visuals().popup_shadow);
    let dark = theme::active() == egui::Theme::Dark;
    let backdrop = Color32::from_black_alpha(if dark { 110 } else { 50 });
    let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, 84.0));
    let mut chosen: Option<Command> = None;
    let modal = egui::Modal::new(id).area(area).frame(frame).backdrop_color(backdrop).show(ui.ctx(), |ui| {
        let Some(p) = app.palette.as_mut() else { return };
        ui.set_width(WIDTH);
        let (up, down, enter) = ui.input_mut(|i| {
            (i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::Enter))
        });
        let field = widgets::search_field(ui, &mut p.query, t("Search commands and operations"));
        if std::mem::take(&mut p.focus) {
            field.request_focus();
        }
        if p.ranked_for.as_deref() != Some(p.query.as_str()) {
            p.results = commands::rank(&p.index, &p.query);
            p.results.truncate(MAX_RESULTS);
            p.ranked_for = Some(p.query.clone());
            p.selected = 0;
        }
        let n = p.results.len();
        if n > 0 && (up || down) {
            p.selected = if down { (p.selected + 1) % n } else { (p.selected + n - 1) % n };
            p.reveal = true;
        }
        if enter {
            chosen = p.results.get(p.selected).and_then(|&i| p.index.get(i)).cloned();
        }
        ui.add_space(6.0);
        if n == 0 {
            let (r, _) = ui.allocate_exact_size(vec2(WIDTH, 60.0), Sense::hover());
            ui.painter().text(r.center(), Align2::CENTER_CENTER, t("No command matches."), theme::body(), tk.text_dim);
        }
        egui::ScrollArea::vertical().max_height(ROW * VISIBLE_ROWS).auto_shrink([false, true]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let reveal = std::mem::take(&mut p.reveal);
            for (row, &ci) in p.results.iter().enumerate() {
                let Some(c) = p.index.get(ci) else { continue };
                let resp = result_row(ui, c, row == p.selected);
                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    p.selected = row;
                }
                if row == p.selected && reveal {
                    resp.scroll_to_me(None);
                }
                if resp.clicked() {
                    chosen = Some(c.clone());
                }
            }
        });
        footer(ui);
    });
    if modal.should_close() || chosen.is_some() {
        app.palette = None;
    }
    if let Some(cmd) = chosen
        && let Err(e) = commands::run(app, &cmd)
    {
        app.set_status(e);
    }
}

/// One result: icon, label and description, and the shortcut on the right.
fn result_row(ui: &mut egui::Ui, c: &Command, selected: bool) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let p = ui.painter();
    let on = ui.ctx().animate_bool_with_time(resp.id.with("on"), selected, 0.08);
    if on > 0.0 {
        p.rect_filled(rect, 8.0, tk.control.gamma_multiply(on));
    }
    let icon_box = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(28.0, 28.0));
    p.rect_filled(icon_box, 7.0, widgets::mix(tk.chrome, tk.control_hover, on));
    match c.icon {
        Some(icon) => crate::icons::paint_colored(p, icon_box.shrink(6.0), icon, widgets::mix(tk.icon, tk.text, on), tk.accent),
        None => crate::icons::paint_colored(p, icon_box.shrink(7.0), crate::icons::Icon::Command, tk.text_faint, tk.accent),
    }
    let x = icon_box.right() + 12.0;
    let mut right = rect.right() - 12.0;
    if let Some(s) = &c.shortcut {
        let g = p.layout_no_wrap(crate::menus::shortcut_text(ui.ctx(), s), theme::small(), tk.text_faint);
        right -= g.size().x;
        p.galley(pos2(right, rect.center().y - g.size().y / 2.0), g, tk.text_faint);
        right -= 12.0;
    } else {
        let g = p.layout_no_wrap(c.id.to_owned(), theme::mono(), tk.text_faint);
        if g.size().x < 180.0 {
            right -= g.size().x;
            p.galley(pos2(right, rect.center().y - g.size().y / 2.0), g, tk.text_faint);
            right -= 12.0;
        }
    }
    let width = (right - x).max(20.0);
    let line = |text: &str, font: egui::FontId, color: Color32, y: f32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
        let g = p.layout_job(job);
        p.galley(pos2(x, y - g.size().y / 2.0), g, color);
    };
    if c.desc.is_empty() {
        line(&c.label, theme::body(), tk.text, rect.center().y);
    } else {
        line(&c.label, theme::body(), tk.text, rect.center().y - 8.0);
        line(&c.desc, theme::small(), tk.text_dim, rect.center().y + 8.0);
    }
    crate::control::record_widget(ui, &resp, &c.label, "option", Some(selected));
    resp
}

/// Key hints under the results.
fn footer(ui: &mut egui::Ui) {
    let tk = Tokens::get();
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, tk.card_border));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        for (keys, what) in [("Up+Down", "to move"), ("Enter", "to run"), ("Esc", "to close")] {
            widgets::keycaps(ui, keys);
            ui.label(egui::RichText::new(t(what)).font(theme::small()).color(tk.text_faint));
            ui.add_space(10.0);
        }
    });
}

/// Opens or closes the palette (the `ui.palette` command).
pub fn toggle(app: &mut PixelApp, on: Option<bool>) -> Value {
    let open = on.unwrap_or(app.palette.is_none());
    app.palette = if open { Some(Palette::new(app)) } else { None };
    serde_json::json!({ "palette": open })
}
