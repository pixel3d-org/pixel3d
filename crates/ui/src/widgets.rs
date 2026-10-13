//! Shared widgets: property cards that open and close with an eased
//! animation, two-column property rows, the tool rail item, and tooltips
//! with a title, a description and the shortcut as key caps. Every state
//! change (hover, selection, opening) fades instead of snapping.

use egui::collapsing_header::CollapsingState;
use egui::{Align2, Color32, FontId, Pos2, Rect, RectAlign, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::theme::{self, Tokens};

/// Height of a section header and of a property row.
pub const HEADER: f32 = 30.0;
pub const ROW: f32 = 22.0;

/// `a` blended toward `b` by `t` in 0..=1.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// What a tooltip says: a title, a line on what it does, and the keyboard
/// shortcut written as `Cmd+Shift+Z` (shown with the platform's key names).
#[derive(Clone, Copy, Debug, Default)]
pub struct Tip<'a> {
    pub title: &'a str,
    pub desc: &'a str,
    pub keys: &'a str,
}

impl<'a> Tip<'a> {
    pub fn new(title: &'a str) -> Self {
        Tip { title, desc: "", keys: "" }
    }

    pub fn desc(mut self, desc: &'a str) -> Self {
        self.desc = desc;
        self
    }

    pub fn keys(mut self, keys: &'a str) -> Self {
        self.keys = keys;
        self
    }
}

/// Where a tooltip goes when the preferred side has no room.
const TIP_SIDES: [RectAlign; 4] = [RectAlign::RIGHT, RectAlign::BOTTOM, RectAlign::TOP, RectAlign::LEFT];

/// Shows `tip` next to `resp` (on `side`) while it is hovered.
pub fn tooltip(resp: &egui::Response, tip: Tip<'_>, side: RectAlign) {
    if tip.title.is_empty() {
        return;
    }
    let tk = Tokens::get();
    let mut tt = if resp.enabled() { egui::Tooltip::for_enabled(resp) } else { egui::Tooltip::for_disabled(resp) };
    let frame = egui::Frame::NONE
        .fill(tk.popup)
        .stroke(Stroke::new(1.0, tk.popup_border))
        .corner_radius(8)
        .inner_margin(egui::Margin { left: 10, right: 10, top: 7, bottom: 8 })
        .shadow(resp.ctx.global_style().visuals.popup_shadow);
    tt.popup = tt.popup.align(side).align_alternatives(&TIP_SIDES).gap(8.0).frame(frame);
    tt.show(|ui| tip_body(ui, tip));
}

fn tip_body(ui: &mut egui::Ui, tip: Tip<'_>) {
    let tk = Tokens::get();
    ui.set_max_width(260.0);
    ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tip.title).font(theme::body()).color(tk.text));
        if !tip.keys.is_empty() {
            ui.add_space(10.0);
            keycaps(ui, tip.keys);
        }
    });
    if !tip.desc.is_empty() {
        ui.add(egui::Label::new(RichText::new(tip.desc).font(theme::small()).color(tk.text_dim)).wrap());
    }
}

/// The platform name of a key in `Cmd+Shift+Z` notation, in words as egui
/// writes shortcuts (the bundled fonts have no key symbols).
fn key_name(key: &str, mac: bool) -> &str {
    match (key, mac) {
        ("Cmd", false) => "Ctrl",
        ("Alt", true) => "Option",
        _ => key,
    }
}

/// `keys` as key caps, for example `Cmd+Shift+Z`.
pub fn keycaps(ui: &mut egui::Ui, keys: &str) {
    let tk = Tokens::get();
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    ui.spacing_mut().item_spacing.x = 3.0;
    for key in keys.split('+') {
        let g = ui.painter().layout_no_wrap(key_name(key.trim(), mac).to_owned(), FontId::proportional(10.5), tk.text_dim);
        let (r, _) = ui.allocate_exact_size(vec2((g.size().x + 9.0).max(18.0), 18.0), Sense::hover());
        ui.painter().rect(r, 4.0, tk.keycap, Stroke::new(1.0, tk.popup_border), StrokeKind::Inside);
        ui.painter().galley(r.center() - g.size() / 2.0, g, tk.text_dim);
    }
}

/// A chevron that turns from pointing right (0) to pointing down (1).
pub fn chevron(painter: &egui::Painter, center: Pos2, open: f32, color: Color32) {
    let (s, c) = (open * std::f32::consts::FRAC_PI_2).sin_cos();
    let rot = |x: f32, y: f32| center + vec2(x * c - y * s, x * s + y * c);
    painter.line(vec![rot(-1.6, -3.4), rot(1.8, 0.0), rot(-1.6, 3.4)], Stroke::new(1.5, color));
}

/// A property card: a header that toggles the body, which opens and closes
/// with an eased height animation. The open state persists per `id`.
pub fn section(ui: &mut egui::Ui, id: &str, title: &str, default_open: bool, body: impl FnOnce(&mut egui::Ui)) {
    let tk = Tokens::get();
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), ui.id().with(("section", id)), default_open);
    let bg = ui.painter().add(egui::Shape::Noop);
    let width = ui.available_width();
    let card = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(width, HEADER), Sense::click());
            if resp.clicked() {
                state.toggle(ui);
            }
            let open = state.openness(ui.ctx());
            let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
            if hot > 0.0 {
                let bottom = (8.0 * (1.0 - open)).round() as u8;
                let radius = egui::CornerRadius { nw: 8, ne: 8, sw: bottom, se: bottom };
                ui.painter().rect_filled(rect.shrink(1.0), radius, tk.card_hover.gamma_multiply(hot));
            }
            chevron(ui.painter(), pos2(rect.left() + 15.0, rect.center().y), open, mix(tk.text_faint, tk.text_dim, hot));
            ui.painter().text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, title, theme::body(), tk.text);
            if open > 0.0 {
                let line = tk.card_border.gamma_multiply(open);
                ui.painter().hline(rect.left() + 1.0..=rect.right() - 1.0, rect.bottom() - 0.5, Stroke::new(1.0, line));
            }
            crate::icons::describe_control(ui, &resp, title, Some(state.is_open()));
            state.show_body_unindented(ui, |ui| {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 12, top: 9, bottom: 11 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 4.0;
                    body(ui);
                });
            });
        })
        .response
        .rect;
    ui.painter().set(bg, egui::epaint::RectShape::new(card, 8, tk.card, Stroke::new(1.0, tk.card_border), StrokeKind::Inside));
    ui.add_space(8.0);
}

/// Width of the label column in a row `available` wide.
pub fn label_width(available: f32) -> f32 {
    (available * 0.36).clamp(72.0, 120.0)
}

/// A property row: the label in a fixed left column, then `add`'s widget.
pub fn prop_row<R>(ui: &mut egui::Ui, label: &str, desc: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w = label_width(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, ROW), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(vec2(w, ROW));
            let l = ui.add(egui::Label::new(RichText::new(label).color(tk.text_dim)).truncate());
            if !desc.is_empty() {
                l.on_hover_text(desc);
            }
        });
        add(ui)
    })
    .inner
}

/// A read-only property: a dim label and the value as plain text (editable
/// values get input fields, so the two never look alike).
pub fn info(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    let tk = Tokens::get();
    let value = value.into();
    prop_row(ui, label, "", |ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(20.0), ROW), Sense::hover());
        let mut job = egui::text::LayoutJob::simple_singleline(value.clone(), theme::body(), tk.text);
        job.wrap =
            egui::text::TextWrapping { max_width: rect.width().max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
        let g = ui.painter().layout_job(job);
        let cut = g.elided;
        ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, tk.text);
        if cut {
            resp.on_hover_text(value);
        }
    });
}

/// A tool rail item: an icon pill with a small label under it (when
/// `label` is set). Selected items fill with the accent.
pub fn rail_item(ui: &mut egui::Ui, rect: Rect, icon: crate::icons::Icon, label: Option<&str>, tip: Tip<'_>, selected: bool) -> egui::Response {
    let tk = Tokens::get();
    let resp = ui.interact(rect, ui.id().with(("rail", tip.title, icon as u8)), Sense::click());
    let enabled = ui.is_enabled();
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), enabled && resp.hovered(), 0.12);
    let on = ui.ctx().animate_bool_with_time(resp.id.with("on"), selected, 0.14);
    let pill_center = if label.is_some() { pos2(rect.center().x, rect.top() + 15.0) } else { rect.center() };
    let pill = Rect::from_center_size(pill_center, vec2(34.0, 28.0));
    let mut fill = mix(Color32::TRANSPARENT, tk.control, hot);
    fill = mix(fill, tk.accent, on);
    if resp.is_pointer_button_down_on() {
        fill = mix(fill, tk.control_hover, 0.6);
    }
    let p = ui.painter();
    if fill.a() > 0 {
        p.rect_filled(pill, 8.0, fill);
    }
    let base = if enabled { mix(tk.icon, tk.text, hot) } else { tk.text_faint.gamma_multiply(0.6) };
    let ir = Rect::from_center_size(pill.center(), vec2(18.0, 18.0));
    crate::icons::paint_colored(p, ir, icon, mix(base, tk.on_accent, on), mix(tk.accent, tk.on_accent, on));
    if let Some(text) = label {
        let color = if !enabled { tk.text_faint.gamma_multiply(0.6) } else { mix(mix(tk.text_faint, tk.text_dim, hot), tk.text, on) };
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(9.5), color);
        job.wrap =
            egui::text::TextWrapping { max_width: rect.width() - 2.0, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
        let g = p.layout_job(job);
        p.galley(pos2(rect.center().x - g.size().x / 2.0, pill.bottom() + 3.0), g, color);
    }
    crate::icons::describe_control(ui, &resp, tip.title, Some(selected));
    tooltip(&resp, tip, RectAlign::RIGHT);
    resp
}

/// Height of search fields.
pub const SEARCH: f32 = 30.0;

/// A search field: a rounded box with the search icon and the text (or the
/// placeholder) centered vertically, an accent edge while focused, and a
/// clear button once there is text.
pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    icon_field(ui, text, hint, crate::icons::Icon::Search, theme::body())
}

/// A text field like [`search_field`] with another icon and font.
pub fn icon_field(ui: &mut egui::Ui, text: &mut String, hint: &str, icon: crate::icons::Icon, font: FontId) -> egui::Response {
    let tk = Tokens::get();
    let bg = ui.painter().add(egui::Shape::Noop);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), SEARCH), Sense::hover());
    let clear = Rect::from_center_size(pos2(rect.right() - 15.0, rect.center().y), vec2(18.0, 18.0));
    let inner = Rect::from_min_max(
        pos2(rect.left() + 32.0, rect.top()),
        pos2(if text.is_empty() { rect.right() - 8.0 } else { clear.left() - 4.0 }, rect.bottom()),
    );
    let edit = egui::TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .hint_text(RichText::new(hint).color(tk.text_faint).font(font.clone()))
        .font(font)
        .vertical_align(egui::Align::Center)
        .min_size(inner.size())
        .desired_width(inner.width())
        .margin(egui::Margin::ZERO);
    let resp = ui.put(inner, edit);
    let focus = resp.has_focus();
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
    let lit = ui.ctx().animate_bool_with_time(resp.id.with("focus"), focus, 0.12);
    let edge = mix(mix(tk.card_border, tk.text_faint, hot * 0.6), tk.accent, lit);
    ui.painter().set(bg, egui::epaint::RectShape::new(rect, 8, tk.field, Stroke::new(1.0, edge), StrokeKind::Inside));
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(15.0, 15.0));
    crate::icons::paint_colored(ui.painter(), icon_rect, icon, mix(tk.text_faint, tk.text_dim, hot.max(lit)), tk.accent);
    if !text.is_empty() {
        let c = ui.interact(clear, resp.id.with("clear"), Sense::click());
        let color = if c.hovered() { tk.text } else { tk.text_faint };
        crate::icons::paint_colored(ui.painter(), clear.shrink(3.0), crate::icons::Icon::Close, color, tk.accent);
        crate::icons::describe_control(ui, &c, crate::i18n::t("Clear the search"), None);
        if c.clicked() {
            text.clear();
        }
    }
    resp
}

/// A vertical strip of tab icons; returns the clicked tab index.
pub fn tab_strip(ui: &mut egui::Ui, rect: Rect, tabs: &[(crate::icons::Icon, &str)], current: usize) -> Option<usize> {
    let tk = Tokens::get();
    ui.painter().rect_filled(rect, 0.0, tk.chrome);
    ui.painter().vline(rect.right() - 0.5, rect.y_range(), Stroke::new(1.0, tk.border));
    let mut clicked = None;
    for (i, (icon, label)) in tabs.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 4.0, rect.top() + 6.0 + i as f32 * 32.0), vec2(rect.width() - 8.0, 28.0));
        let hit = ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                crate::icons::button_with(ui, *icon, r.size(), Tip::new(label), i == current, crate::icons::Look::Solid, RectAlign::LEFT).clicked()
            })
            .inner;
        if hit {
            clicked = Some(i);
        }
    }
    clicked
}

/// The frame of menus, dropdowns and popovers.
pub fn popup_frame(ui: &egui::Ui) -> egui::Frame {
    let tk = Tokens::get();
    egui::Frame::NONE
        .fill(tk.popup)
        .stroke(Stroke::new(1.0, tk.popup_border))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(5))
        .shadow(ui.visuals().popup_shadow)
}

/// One dropdown row: an icon column (when `icon_column`), the label, the
/// shortcut, and a check mark when `on` is `Some(true)`.
pub fn menu_row(
    ui: &mut egui::Ui,
    icon: Option<crate::icons::Icon>,
    label: &str,
    shortcut: Option<&str>,
    on: Option<bool>,
    icon_column: bool,
) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.08);
    let checked = on == Some(true);
    let p = ui.painter();
    if hot > 0.0 {
        p.rect_filled(rect, 6.0, tk.control.gamma_multiply(hot));
    }
    let mut x = rect.left() + 10.0;
    if icon_column {
        if let Some(icon) = icon {
            let ir = Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
            let color = if checked { tk.icon_accent } else { mix(tk.icon, tk.text, hot) };
            crate::icons::paint_colored(p, ir, icon, color, tk.accent);
        }
        x += 28.0;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, theme::body(), tk.text);
    let mut right = rect.right() - 10.0;
    if checked {
        let c = pos2(right - 5.0, rect.center().y);
        p.line(vec![c + vec2(-4.5, 0.0), c + vec2(-1.5, 3.0), c + vec2(4.5, -3.5)], Stroke::new(1.6, tk.icon_accent));
        right -= 18.0;
    }
    if let Some(s) = shortcut {
        p.text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, s, theme::small(), tk.text_faint);
    }
    crate::icons::describe_control(ui, &resp, label, on);
    crate::control::record_widget(ui, &resp, label, "menuitem", on);
    resp
}

/// A small section title inside a dropdown.
pub fn menu_header(ui: &mut egui::Ui, label: &str) {
    let tk = Tokens::get();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 10.0, r.center().y + 2.0), Align2::LEFT_CENTER, label, theme::small(), tk.text_faint);
}

/// A thin line between groups of dropdown rows.
pub fn menu_separator(ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(r.left() + 8.0..=r.right() - 8.0, r.center().y, Stroke::new(1.0, tk.card_border));
}
