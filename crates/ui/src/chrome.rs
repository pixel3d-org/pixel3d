//! Window chrome: title row and toolbar, menu bar, status bar, the Start
//! page, viewport overlays and keyboard shortcuts.

use egui::{Align2, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{PixelApp, SelectMode, Tool};

/// Height of the macOS title bar band (the traffic lights sit in it).
const MAC_TITLE_BAR: f32 = 28.0;
/// Where title-row content may start right of the traffic lights.
const MAC_CONTENT_LEFT: f32 = 78.0;
const TOOLBAR: f32 = 34.0;
const BUTTON: f32 = 26.0;

/// One toolbar button: icon, command, label and its parameters.
struct Item {
    icon: Icon,
    command: &'static str,
    label: &'static str,
    params: fn() -> Value,
    selected: fn(&PixelApp) -> bool,
}

fn no_params() -> Value {
    Value::Null
}

fn never(_: &PixelApp) -> bool {
    false
}

macro_rules! item {
    ($icon:ident, $cmd:literal, $label:literal) => {
        Item { icon: Icon::$icon, command: $cmd, label: $label, params: no_params, selected: never }
    };
    ($icon:ident, $cmd:literal, $label:literal, $params:expr, $selected:expr) => {
        Item { icon: Icon::$icon, command: $cmd, label: $label, params: $params, selected: $selected }
    };
}

fn toolbar_groups() -> Vec<Vec<Item>> {
    vec![
        vec![item!(New, "file.new", "New (Ctrl+N)"), item!(Open, "file.open", "Open (Ctrl+O)"), item!(Save, "file.save", "Save (Ctrl+S)")],
        vec![item!(Undo, "edit.undo", "Undo (Ctrl+Z)"), item!(Redo, "edit.redo", "Redo (Ctrl+Shift+Z)")],
        vec![
            item!(Select, "ui.tool", "Select (Q)", || json!({ "tool": "select" }), |a| a.ui.tool == Tool::Select),
            item!(Move, "ui.tool", "Move (W)", || json!({ "tool": "move" }), |a| a.ui.tool == Tool::Move),
            item!(Rotate, "ui.tool", "Rotate (E)", || json!({ "tool": "rotate" }), |a| a.ui.tool == Tool::Rotate),
            item!(Scale, "ui.tool", "Scale (R)", || json!({ "tool": "scale" }), |a| a.ui.tool == Tool::Scale),
        ],
        vec![
            item!(ModeObject, "ui.mode", "Object mode (1)", || json!({ "mode": "object" }), |a| a.ui.mode == SelectMode::Object),
            item!(ModeVert, "ui.mode", "Vertex mode (2)", || json!({ "mode": "vert" }), |a| a.ui.mode == SelectMode::Vert),
            item!(ModeEdge, "ui.mode", "Edge mode (3)", || json!({ "mode": "edge" }), |a| a.ui.mode == SelectMode::Edge),
            item!(ModeFace, "ui.mode", "Face mode (4)", || json!({ "mode": "face" }), |a| a.ui.mode == SelectMode::Face),
        ],
        vec![
            item!(Box, "prim.box", "Add a box"),
            item!(Cylinder, "prim.cylinder", "Add a cylinder"),
            item!(Sphere, "prim.quad_sphere", "Add a sphere"),
            item!(Plane, "prim.plane", "Add a plane"),
        ],
        vec![
            item!(Frame, "view.frame", "Frame selection (F)"),
            item!(Grid, "view.grid", "Ground grid", no_params, |a| a.ui.view.grid),
            item!(Orthographic, "view.ortho", "Orthographic view (5)", no_params, |a| a.ui.view.ortho),
        ],
        vec![
            item!(ShadeSolid, "view.shading", "Solid", || json!({ "mode": "solid" }), |a| a.ui.view.shading == "solid"),
            item!(ShadeSolidWire, "view.shading", "Solid with edges", || json!({ "mode": "solid_wire" }), |a| a.ui.view.shading == "solid_wire"),
            item!(ShadeWire, "view.shading", "Wireframe", || json!({ "mode": "wire" }), |a| a.ui.view.shading == "wire"),
            item!(ShadeMatcap, "view.shading", "Clay (matcap)", || json!({ "mode": "matcap" }), |a| a.ui.view.shading == "matcap"),
        ],
    ]
}

/// Title row (integrated macOS title bar) and the toolbar.
pub fn title_and_toolbar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let title_h = if app.integrated_titlebar { MAC_TITLE_BAR } else { 0.0 };
    let tb_h = if app.ui.show_toolbar { TOOLBAR } else { 0.0 };
    if title_h + tb_h <= 0.0 {
        return;
    }
    egui::Panel::top("p3d_title_toolbar").exact_size(title_h + tb_h).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        if title_h > 0.0 {
            let title = format!("pixel3d      {}", if app.ui.start_page { t("Start").to_owned() } else { app.title() });
            let galley = ui.painter().layout_no_wrap(title, egui::FontId::proportional(13.0), tk.text_dim);
            let x = (r.left() + (r.width() - galley.size().x) / 2.0).max(r.left() + MAC_CONTENT_LEFT);
            ui.painter().galley(pos2(x, r.top() + (title_h - galley.size().y) / 2.0), galley, tk.text_dim);
            let band = Rect::from_min_size(r.min, vec2(r.width(), title_h));
            let resp = ui.interact(band, ui.id().with("titledrag"), Sense::CLICK | Sense::DRAG);
            if resp.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if resp.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
        if tb_h > 0.0 {
            let row = Rect::from_min_size(pos2(r.left(), r.top() + title_h), vec2(r.width(), tb_h));
            ui.painter().hline(row.x_range(), row.bottom() - 0.5, Stroke::new(1.0, tk.border));
            toolbar(app, ui, row);
        }
    });
}

fn toolbar(app: &mut PixelApp, ui: &mut egui::Ui, row: Rect) {
    let tk = Tokens::get();
    let groups = toolbar_groups();
    let gap = 3.0;
    let group_gap = 16.0;
    let widths: Vec<f32> = groups.iter().map(|g| g.len() as f32 * (BUTTON + gap) - gap).collect();
    let left = row.left() + 10.0;
    let available = (row.right() - 10.0 - left).max(0.0);
    let mut clicked: Option<(&'static str, Value)> = None;
    let fit = craft_ui::layout::fit_toolbar(&widths, available, group_gap, 30.0);
    let mut x = left;
    let shown: Vec<usize> = fit.as_ref().map_or_else(|| (0..groups.len()).collect(), |f| f.shown.iter().map(|(i, _)| *i).collect());
    for gi in shown {
        let Some(group) = groups.get(gi) else { continue };
        for it in group {
            let rect = Rect::from_min_size(pos2(x, row.center().y - BUTTON / 2.0), vec2(BUTTON, BUTTON));
            let selected = (it.selected)(app);
            let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| icons::button(ui, it.icon, BUTTON, t(it.label), selected)).inner;
            if resp.clicked() {
                clicked = Some((it.command, (it.params)()));
            }
            x += BUTTON + gap;
        }
        x += group_gap - gap;
        if gi + 1 < groups.len() {
            ui.painter().vline(x - group_gap / 2.0, row.y_range().shrink(9.0), Stroke::new(1.0, tk.border));
        }
    }
    if let Some(f) = &fit
        && !f.overflow.is_empty()
    {
        let rect = Rect::from_min_size(pos2(x, row.center().y - BUTTON / 2.0), vec2(f.overflow_width, BUTTON));
        let resp = ui.interact(rect, ui.id().with("toolbar_more"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 3.0, tk.control_hover.gamma_multiply(0.6));
        }
        for dx in [-5.0, 0.0, 5.0] {
            ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.5, tk.text);
        }
        icons::describe_control(ui, &resp, t("More tools"), None);
        egui::Popup::menu(&resp).show(|ui| {
            for (n, gi) in f.overflow.iter().enumerate() {
                let Some(group) = groups.get(*gi) else { continue };
                if n > 0 {
                    ui.separator();
                }
                for it in group {
                    if ui.button(t(it.label)).clicked() {
                        clicked = Some((it.command, (it.params)()));
                        ui.close();
                    }
                }
            }
        });
    }
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
}

/// The in-window menu bar (macOS uses the native one).
pub fn menu_bar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let mut clicked: Option<(&'static str, Value)> = None;
    egui::Panel::top("p3d_menu").frame(egui::Frame::NONE.fill(tk.chrome).inner_margin(egui::Margin::symmetric(6, 2))).show(ui, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            for menu in crate::menus::menus() {
                ui.menu_button(t(menu.label), |ui| {
                    for entry in menu.entries {
                        match entry {
                            None => {
                                ui.separator();
                            }
                            Some(it) => {
                                let mut b = egui::Button::new(t(it.label));
                                if let Some(s) = &it.shortcut {
                                    b = b.shortcut_text(ui.ctx().format_shortcut(s));
                                }
                                let resp = ui.add(b);
                                crate::control::record_widget(ui, &resp, it.label, "menuitem", None);
                                if resp.clicked() {
                                    clicked = Some((it.command, (it.params)()));
                                    ui.close();
                                }
                            }
                        }
                    }
                });
            }
        });
    });
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
}

/// Status bar: mode and selection, the last message, triangles against the
/// budget, units and frame rate.
pub fn status_bar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    egui::Panel::bottom("p3d_status").exact_size(24.0).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        let p = ui.painter().clone();
        p.hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, tk.border));
        let y = r.center().y;
        let mut x = r.left() + 10.0;
        let text = |p: &egui::Painter, x: &mut f32, s: &str, color| {
            let g = p.layout_no_wrap(s.to_owned(), theme::small(), color);
            let w = g.size().x;
            p.galley(pos2(*x, y - g.size().y / 2.0), g, color);
            *x += w + 18.0;
        };
        text(&p, &mut x, app.ui.mode.label(), tk.text);
        let sel = app.selection.len();
        text(&p, &mut x, &format!("{sel} {}", t("selected")), tk.text_dim);
        // Right side, laid out from the right edge.
        let doc = app.doc();
        let tris: usize = doc.objects().filter_map(|(k, _)| doc.object_mesh(k)).map(|m| m.triangle_count()).sum();
        let budget = doc.settings.tri_budget() as usize;
        let right = [
            format!("{:.0} {}", app.viewport.fps, t("fps")),
            t("meters").to_owned(),
            format!("{} / {} {}", thousands(tris), thousands(budget), t("tris")),
        ];
        let mut rx = r.right() - 10.0;
        let over = tris > budget;
        for (i, s) in right.iter().enumerate() {
            let color = if i == 2 && over { tk.health_warn } else { tk.text_dim };
            let g = p.layout_no_wrap(s.clone(), theme::small(), color);
            rx -= g.size().x;
            p.galley(pos2(rx, y - g.size().y / 2.0), g, color);
            rx -= 18.0;
        }
        if let Some(msg) = app.status_message() {
            let width = (rx - x).max(0.0);
            if width > 20.0 {
                let mut job = egui::text::LayoutJob::simple_singleline(msg.to_owned(), theme::small(), tk.warn);
                job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
                let g = p.layout_job(job);
                p.galley(pos2(x, y - g.size().y / 2.0), g, tk.warn);
            }
        }
    });
}

/// `1234567` as `1,234,567`.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The Start page: new, open, sample and recent files.
pub fn start_page(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let r = ui.max_rect();
    let p = ui.painter().clone();
    let left = Rect::from_min_size(r.min, vec2(300.0, r.height()));
    p.rect_filled(left, 0.0, tk.chrome_dark);
    p.rect_filled(Rect::from_min_max(pos2(left.right(), r.top()), r.max), 0.0, tk.chrome);
    p.text(pos2(left.left() + 34.0, left.top() + 70.0), Align2::LEFT_CENTER, "pixel3d", egui::FontId::proportional(28.0), tk.text);
    p.text(
        pos2(left.left() + 34.0, left.top() + 98.0),
        Align2::LEFT_CENTER,
        format!("{} {}", t("Version"), env!("CARGO_PKG_VERSION")),
        theme::small(),
        tk.text_faint,
    );
    let mut action: Option<(&'static str, Value)> = None;
    let mut y = left.top() + 140.0;
    for (label, cmd) in [("New scene", "file.new"), ("Open...", "file.open"), ("Open the sample scene", "file.sample")] {
        let br = Rect::from_min_size(pos2(left.left() + 34.0, y), vec2(230.0, 34.0));
        let resp = ui.interact(br, ui.id().with(("start", cmd)), Sense::click());
        p.rect_stroke(br, 3.0, Stroke::new(1.0, if resp.hovered() { tk.text } else { tk.text_dim }), egui::StrokeKind::Inside);
        p.text(pos2(br.left() + 16.0, br.center().y), Align2::LEFT_CENTER, t(label), theme::body(), tk.text);
        icons::describe_control(ui, &resp, t(label), None);
        if resp.clicked() {
            action = Some((cmd, Value::Null));
        }
        y += 44.0;
    }
    let main = Rect::from_min_max(pos2(left.right() + 36.0, r.top() + 40.0), r.max);
    p.text(main.min, Align2::LEFT_TOP, t("Recent"), egui::FontId::proportional(22.0), tk.text);
    let recent = app.ui.recent.clone();
    if recent.is_empty() {
        p.text(main.center(), Align2::CENTER_CENTER, t("There is nothing here yet."), egui::FontId::proportional(18.0), tk.text);
        p.text(
            main.center() + vec2(0.0, 26.0),
            Align2::CENTER_CENTER,
            t("Start a new scene, open a file, or open the sample scene."),
            theme::body(),
            tk.text_dim,
        );
    }
    let mut ry = main.top() + 44.0;
    for path in recent {
        let rr = Rect::from_min_size(pos2(main.left(), ry), vec2((main.width() - 40.0).max(100.0), 28.0));
        let resp = ui.interact(rr, ui.id().with(("recent", &path)), Sense::click());
        if resp.hovered() {
            p.rect_filled(rr, 3.0, tk.control.gamma_multiply(0.5));
        }
        p.text(pos2(rr.left() + 8.0, rr.center().y), Align2::LEFT_CENTER, &path, theme::body(), tk.text);
        icons::describe_control(ui, &resp, &path, None);
        if resp.clicked() {
            let _ = app.open_path(std::path::Path::new(&path));
        }
        ry += 30.0;
    }
    if let Some((cmd, params)) = action {
        let _ = app.run(cmd, params);
    }
}

/// Overlays drawn over the viewport image: the view name and the axis triad.
pub fn viewport_overlay(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let rect = app.viewport.rect;
    if !rect.is_positive() {
        return;
    }
    let p = ui.painter_at(rect);
    let label = format!(
        "{}, {}",
        if app.ui.view.ortho { t("Orthographic") } else { t("Perspective") },
        match app.ui.view.shading() {
            p3d_render::Shading::Solid => t("Solid"),
            p3d_render::Shading::SolidWire => t("Solid with edges"),
            p3d_render::Shading::Wire => t("Wireframe"),
            p3d_render::Shading::Matcap => t("Clay"),
        }
    );
    p.text(rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, label, theme::small(), tk.text_dim);
    // Axis triad, bottom left: world axes as seen by the camera.
    let view = app.viewport.camera.view();
    let o = pos2(rect.left() + 38.0, rect.bottom() - 38.0);
    let mut axes = [(glam::Vec3::X, tk.axis_x, "X"), (glam::Vec3::Y, tk.axis_y, "Y"), (glam::Vec3::Z, tk.axis_z, "Z")];
    // Draw the axis pointing away from the camera first.
    axes.sort_by(|a, b| view.transform_vector3(a.0).z.total_cmp(&view.transform_vector3(b.0).z));
    for (axis, color, name) in axes {
        let v = view.transform_vector3(axis);
        let end = o + vec2(v.x, -v.y) * 24.0;
        p.line_segment([o, end], Stroke::new(2.0, color));
        p.circle_filled(end, 7.0, color);
        p.text(end, Align2::CENTER_CENTER, name, egui::FontId::proportional(9.5), egui::Color32::WHITE);
    }
}

/// A shortcut, its command and the command parameters.
type Binding = (KeyboardShortcut, &'static str, fn() -> Value);

/// Keyboard shortcuts. Nothing fires while a text field has the keyboard.
pub fn shortcuts(app: &mut PixelApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let cmd = Modifiers::COMMAND;
    let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
    let table: &[Binding] = &[
        (KeyboardShortcut::new(shift_cmd, Key::Z), "edit.redo", || Value::Null),
        (KeyboardShortcut::new(shift_cmd, Key::S), "file.save_as", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::Z), "edit.undo", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::Y), "edit.redo", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::S), "file.save", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::O), "file.open", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::N), "file.new", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::D), "edit.duplicate", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::A), "edit.select_all", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Escape), "edit.select_none", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Delete), "edit.delete", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Backspace), "edit.delete", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F), "view.frame", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Q), "ui.tool", || json!({ "tool": "select" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::W), "ui.tool", || json!({ "tool": "move" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::E), "ui.tool", || json!({ "tool": "rotate" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::R), "ui.tool", || json!({ "tool": "scale" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num1), "ui.mode", || json!({ "mode": "object" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num2), "ui.mode", || json!({ "mode": "vert" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num3), "ui.mode", || json!({ "mode": "edge" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num4), "ui.mode", || json!({ "mode": "face" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num5), "view.ortho", || Value::Null),
    ];
    for (shortcut, command, params) in table {
        if ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
            let _ = app.run(command, params());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(6000), "6,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
