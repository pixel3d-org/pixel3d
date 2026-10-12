//! Window chrome, laid out like Blender's: a top bar with the menus and the
//! title, a tool settings row under it, a vertical tool column on the left,
//! a header strip on top of the viewport (selection modes and shading), the
//! status bar, the Start page, viewport overlays and keyboard shortcuts.

use egui::{Align2, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{PixelApp, SelectMode, Tool};

/// Height of the top bar; on macOS the traffic lights sit in it.
const TOP_BAR: f32 = 28.0;
/// Where top bar content starts right of the macOS traffic lights.
const MAC_CONTENT_LEFT: f32 = 78.0;
/// The tool settings row under the top bar.
const TOOL_SETTINGS: f32 = 30.0;
/// The tool column: icons only, or icons with labels.
const SIDEBAR: f32 = 40.0;
const SIDEBAR_LABELS: f32 = 168.0;
const TOOL_BUTTON: f32 = 32.0;
/// The header strip on top of the viewport.
pub const VIEW_HEADER: f32 = 28.0;

/// One button: icon, command, label, parameters and whether it shows as on.
struct Item {
    icon: Icon,
    command: &'static str,
    label: &'static str,
    /// Shortcut and one line on what it does, for the tooltip.
    hint: &'static str,
    params: fn() -> Value,
    selected: fn(&PixelApp) -> bool,
    /// Needs a selected object.
    needs_selection: bool,
}

fn no_params() -> Value {
    Value::Null
}

fn never(_: &PixelApp) -> bool {
    false
}

macro_rules! item {
    ($icon:ident, $cmd:literal, $label:literal, $hint:literal) => {
        Item { icon: Icon::$icon, command: $cmd, label: $label, hint: $hint, params: no_params, selected: never, needs_selection: false }
    };
    ($icon:ident, $cmd:literal, $label:literal, $hint:literal, $params:expr, $selected:expr) => {
        Item { icon: Icon::$icon, command: $cmd, label: $label, hint: $hint, params: $params, selected: $selected, needs_selection: false }
    };
}

fn on_selection(mut it: Item) -> Item {
    it.needs_selection = true;
    it
}

/// The tool column, in groups.
fn sidebar_groups() -> Vec<Vec<Item>> {
    vec![
        vec![
            item!(Select, "ui.tool", "Select", "Q: click to select, Shift+click to add", || json!({ "tool": "select" }), |a| a.ui.tool
                == Tool::Select),
            item!(Move, "ui.tool", "Move", "W: move the selection", || json!({ "tool": "move" }), |a| a.ui.tool == Tool::Move),
            item!(Rotate, "ui.tool", "Rotate", "E: rotate the selection", || json!({ "tool": "rotate" }), |a| a.ui.tool == Tool::Rotate),
            item!(Scale, "ui.tool", "Scale", "R: scale the selection", || json!({ "tool": "scale" }), |a| a.ui.tool == Tool::Scale),
        ],
        vec![
            item!(Box, "prim.box", "Box", "Add a box at the origin"),
            item!(Cylinder, "prim.cylinder", "Cylinder", "Add a cylinder at the origin"),
            item!(Sphere, "prim.quad_sphere", "Sphere", "Add a quad sphere at the origin"),
            item!(Plane, "prim.plane", "Plane", "Add a flat grid at the origin"),
            item!(Empty, "scene.create_empty", "Empty", "Add an empty (a transform only)"),
        ],
    ]
}

/// Actions on the selected objects, shown in the tool settings row while
/// something is selected (the contextual toolbar).
fn object_actions() -> Vec<Item> {
    vec![
        on_selection(item!(Frame, "scene.apply_transform", "Apply transform", "Bake the object's transform into its mesh")),
        on_selection(item!(Mesh, "edit.fill", "Fill holes", "Close holes with one face each")),
        on_selection(item!(Health, "edit.recalc_normals", "Recalculate normals", "Make faces point outward")),
        item!(Plus, "edit.duplicate", "Duplicate", "Ctrl+D: copy the selected objects"),
        item!(Close, "edit.delete", "Delete", "Delete: remove the selected objects"),
    ]
}

/// Buttons of the tool settings row (file and history).
fn settings_items() -> Vec<Item> {
    vec![
        item!(New, "file.new", "New", "Ctrl+N: start an empty scene"),
        item!(Open, "file.open", "Open", "Ctrl+O: open a .p3d file"),
        item!(Save, "file.save", "Save", "Ctrl+S: save to the current file"),
        item!(Undo, "edit.undo", "Undo", "Ctrl+Z: step back one change"),
        item!(Redo, "edit.redo", "Redo", "Ctrl+Shift+Z: step forward again"),
    ]
}

/// The viewport header: selection modes on the left, view toggles and
/// shading on the right.
fn header_modes() -> Vec<Item> {
    vec![
        item!(ModeObject, "ui.mode", "Object mode", "1: select whole objects", || json!({ "mode": "object" }), |a| a.ui.mode == SelectMode::Object),
        item!(ModeVert, "ui.mode", "Vertex mode", "2: select vertices", || json!({ "mode": "vert" }), |a| a.ui.mode == SelectMode::Vert),
        item!(ModeEdge, "ui.mode", "Edge mode", "3: select edges", || json!({ "mode": "edge" }), |a| a.ui.mode == SelectMode::Edge),
        item!(ModeFace, "ui.mode", "Face mode", "4: select faces", || json!({ "mode": "face" }), |a| a.ui.mode == SelectMode::Face),
    ]
}

fn header_view() -> Vec<Vec<Item>> {
    vec![
        vec![
            item!(Frame, "view.frame", "Frame selection", "F: fit the selection, or everything, in the view"),
            item!(Grid, "view.grid", "Ground grid", "Show or hide the ground grid", no_params, |a| a.ui.view.grid),
            item!(Orthographic, "view.ortho", "Orthographic", "5: switch between perspective and orthographic", no_params, |a| a.ui.view.ortho),
        ],
        vec![
            item!(ShadeWire, "view.shading", "Wireframe", "Edges only", || json!({ "mode": "wire" }), |a| a.ui.view.shading == "wire"),
            item!(ShadeSolid, "view.shading", "Solid", "Lit surfaces", || json!({ "mode": "solid" }), |a| a.ui.view.shading == "solid"),
            item!(ShadeSolidWire, "view.shading", "Solid with edges", "Lit surfaces and polygon edges", || json!({ "mode": "solid_wire" }), |a| a
                .ui
                .view
                .shading
                == "solid_wire"),
            item!(ShadeMatcap, "view.shading", "Clay", "Gray clay that shows form", || json!({ "mode": "matcap" }), |a| a.ui.view.shading
                == "matcap"),
        ],
    ]
}

/// Draws one icon button in `rect`; returns whether it was clicked.
fn button_at(ui: &mut egui::Ui, rect: Rect, it: &Item, app: &PixelApp, enabled: bool) -> bool {
    let selected = (it.selected)(app);
    let tooltip = format!("{}\n{}", t(it.label), t(it.hint));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.add_enabled_ui(enabled, |ui| icons::button(ui, it.icon, rect.width(), &tooltip, selected).clicked()).inner
    })
    .inner
}

/// The top bar: menus on the left (after the traffic lights on macOS), the
/// document title in the middle. Dragging it moves the window.
pub fn top_bar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    egui::Panel::top("p3d_top_bar").exact_size(TOP_BAR).frame(egui::Frame::NONE.fill(tk.chrome_dark)).show(ui, |ui| {
        let r = ui.max_rect();
        let left = r.left() + if app.integrated_titlebar { MAC_CONTENT_LEFT } else { 6.0 };
        // Window dragging first, so the menus on top of it keep their clicks.
        if app.integrated_titlebar {
            let resp = ui.interact(r, ui.id().with("titledrag"), Sense::CLICK | Sense::DRAG);
            if resp.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if resp.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
        }
        let menus_rect = Rect::from_min_max(pos2(left, r.top()), pos2(r.right(), r.bottom()));
        let menus_right = ui
            .scope_builder(egui::UiBuilder::new().max_rect(menus_rect).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                menu_bar(app, ui)
            })
            .inner;
        let title = if app.ui.start_page { t("Start").to_owned() } else { app.title() };
        let galley = ui.painter().layout_no_wrap(format!("pixel3d  {title}"), theme::body(), tk.text_dim);
        let x = (r.center().x - galley.size().x / 2.0).max(menus_right + 24.0);
        if x + galley.size().x < r.right() - 8.0 {
            ui.painter().galley(pos2(x, r.center().y - galley.size().y / 2.0), galley, tk.text_dim);
        }
    });
}

/// The menus (also the macOS native menu's contents).
/// Returns the right edge of the menus.
fn menu_bar(app: &mut PixelApp, ui: &mut egui::Ui) -> f32 {
    let mut clicked: Option<(&'static str, Value)> = None;
    let mut right = ui.max_rect().left();
    egui::MenuBar::new().ui(ui, |ui| {
        for menu in crate::menus::menus() {
            let button = ui.menu_button(t(menu.label), |ui| {
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
            right = right.max(button.response.rect.right());
        }
    });
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
    right
}

/// The tool settings row: file and history buttons, then the active tool
/// and what it does.
pub fn tool_settings(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    egui::Panel::top("p3d_tool_settings").exact_size(TOOL_SETTINGS).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, tk.border));
        let size = 24.0;
        let mut x = r.left() + 8.0;
        let mut clicked = None;
        for (i, it) in settings_items().iter().enumerate() {
            if i == 3 {
                ui.painter().vline(x + 3.0, r.y_range().shrink(8.0), Stroke::new(1.0, tk.border));
                x += 8.0;
            }
            let rect = Rect::from_min_size(pos2(x, r.center().y - size / 2.0), vec2(size, size));
            if button_at(ui, rect, it, app, true) {
                clicked = Some((it.command, (it.params)()));
            }
            x += size + 2.0;
        }
        x += 10.0;
        ui.painter().vline(x, r.y_range().shrink(8.0), Stroke::new(1.0, tk.border));
        x += 12.0;
        let (name, what) = match app.ui.tool {
            Tool::Select => (t("Select"), t("Click an object to select it; Shift+click adds or removes.")),
            Tool::Move => (t("Move"), t("Drag the gizmo arrows to move the selection.")),
            Tool::Rotate => (t("Rotate"), t("Drag the gizmo rings to rotate the selection.")),
            Tool::Scale => (t("Scale"), t("Drag the gizmo handles to scale the selection.")),
        };
        let g = ui.painter().layout_no_wrap(name.to_owned(), egui::FontId::proportional(13.0), tk.text);
        let w = g.size().x;
        ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, tk.text);
        x += w + 14.0;
        let mut action = None;
        if app.selected_targets().is_empty() {
            ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, what, theme::small(), tk.text_dim);
        } else {
            // The contextual toolbar: actions on the selection.
            for it in object_actions() {
                let label = t(it.label);
                let text_w = ui.painter().layout_no_wrap(label.to_owned(), theme::body(), tk.text).size().x;
                let rect = Rect::from_min_size(pos2(x, r.center().y - 11.0), vec2(16.0 + 6.0 + text_w + 14.0, 22.0));
                if rect.right() > r.right() - 8.0 {
                    break;
                }
                let resp = ui.interact(rect, ui.id().with(("action", it.command)), Sense::click());
                let fill = if resp.hovered() { tk.control_hover } else { tk.control.gamma_multiply(0.55) };
                ui.painter().rect_filled(rect, 4.0, fill);
                icons::paint(ui.painter(), Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 8.0), vec2(16.0, 16.0)), it.icon, false);
                ui.painter().text(pos2(rect.left() + 28.0, rect.center().y), Align2::LEFT_CENTER, label, theme::body(), tk.text);
                icons::describe_control(ui, &resp, label, None);
                if resp.on_hover_text(t(it.hint)).clicked() {
                    action = Some(it);
                }
                x = rect.right() + 6.0;
            }
        }
        if let Some((cmd, params)) = clicked {
            let _ = app.run(cmd, params);
        }
        if let Some(it) = action {
            if it.needs_selection {
                let _ = app.run_on_selection(it.command);
            } else {
                let _ = app.run(it.command, (it.params)());
            }
        }
    });
}

/// The tool column on the left: icon buttons in groups, like Blender's
/// toolbar. Right-click switches between icons only and icons with labels.
pub fn tool_sidebar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let width = if app.ui.sidebar_labels { SIDEBAR_LABELS } else { SIDEBAR };
    let has_selection = !app.selected_targets().is_empty();
    egui::Panel::left("p3d_tools").exact_size(width).resizable(false).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, tk.border));
        let back = ui.interact(r, ui.id().with("toolbar-bg"), Sense::click());
        back.context_menu(|ui| {
            let label = if app.ui.sidebar_labels { t("Icons only") } else { t("Show labels") };
            if ui.button(label).clicked() {
                app.ui.sidebar_labels = !app.ui.sidebar_labels;
                ui.close();
            }
        });
        let mut y = r.top() + 6.0;
        let mut clicked = None;
        let groups = sidebar_groups();
        for (gi, group) in groups.iter().enumerate() {
            if gi > 0 {
                y += 5.0;
                ui.painter().hline(r.left() + 8.0..=r.right() - 8.0, y, Stroke::new(1.0, tk.border));
                y += 6.0;
            }
            for it in group {
                let enabled = !it.needs_selection || has_selection;
                let rect = Rect::from_min_size(pos2(r.left() + 4.0, y), vec2(TOOL_BUTTON, TOOL_BUTTON));
                if button_at(ui, rect, it, app, enabled) {
                    clicked = Some((it, (it.params)()));
                }
                if app.ui.sidebar_labels {
                    let lr = Rect::from_min_max(pos2(rect.right() + 6.0, rect.top()), pos2(r.right() - 6.0, rect.bottom()));
                    let resp = ui.interact(lr, ui.id().with(("toollabel", it.label, it.icon as u8)), Sense::click());
                    let color = if enabled { if (it.selected)(app) { tk.text } else { tk.text_dim } } else { tk.text_faint };
                    ui.painter().text(pos2(lr.left(), lr.center().y), Align2::LEFT_CENTER, t(it.label), theme::body(), color);
                    if enabled && resp.clicked() {
                        clicked = Some((it, (it.params)()));
                    }
                }
                y += TOOL_BUTTON + 2.0;
            }
        }
        if let Some((it, params)) = clicked {
            let params = if it.needs_selection {
                match app.selected_targets().first() {
                    Some(id) => json!({ "target": id }),
                    None => params,
                }
            } else {
                params
            };
            let _ = app.run(it.command, params);
        }
    });
}

/// The viewport header strip at the top of `rect`; returns the area left
/// for the 3D view.
pub fn viewport_header(app: &mut PixelApp, ui: &mut egui::Ui, rect: Rect) -> Rect {
    let tk = Tokens::get();
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), VIEW_HEADER));
    ui.painter().rect_filled(header, 0.0, tk.chrome_dark);
    let size = 22.0;
    let y = header.center().y - size / 2.0;
    let mut clicked = None;
    let mut x = header.left() + 6.0;
    for it in header_modes() {
        let r = Rect::from_min_size(pos2(x, y), vec2(size, size));
        if button_at(ui, r, &it, app, true) {
            clicked = Some((it.command, (it.params)()));
        }
        x += size + 1.0;
    }
    ui.painter().text(pos2(x + 10.0, header.center().y), Align2::LEFT_CENTER, app.ui.mode.label(), theme::small(), tk.text_dim);
    let mut rx = header.right() - 6.0;
    for (gi, group) in header_view().iter().enumerate().rev() {
        for it in group.iter().rev() {
            rx -= size;
            let r = Rect::from_min_size(pos2(rx, y), vec2(size, size));
            if button_at(ui, r, it, app, true) {
                clicked = Some((it.command, (it.params)()));
            }
            rx -= 1.0;
        }
        if gi > 0 {
            rx -= 6.0;
            ui.painter().vline(rx, header.y_range().shrink(7.0), Stroke::new(1.0, tk.border));
            rx -= 6.0;
        }
    }
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
    Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max)
}

/// Status bar: mode and selection, the last message, triangles against the
/// budget, units, frame rate and the bottom panel toggle.
pub fn status_bar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let mut toggle_bottom = false;
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
        let mut rx = r.right() - 6.0;
        let br = Rect::from_min_size(pos2(rx - 20.0, y - 10.0), vec2(20.0, 20.0));
        let shown = app.bottom_panel_visible();
        let tip = if shown { t("Hide History and Console") } else { t("Show History and Console") };
        let resp = ui.scope_builder(egui::UiBuilder::new().max_rect(br), |ui| icons::button(ui, Icon::History, 20.0, tip, shown)).inner;
        toggle_bottom = resp.clicked();
        rx -= 32.0;
        let doc = app.doc();
        let tris: usize = doc.objects().filter_map(|(k, _)| doc.object_mesh(k)).map(|m| m.triangle_count()).sum();
        let budget = doc.settings.tri_budget() as usize;
        let right = [
            format!("{:.0} {}", app.viewport.fps, t("fps")),
            t("meters").to_owned(),
            format!("{} / {} {}", thousands(tris), thousands(budget), t("tris")),
        ];
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
    if toggle_bottom {
        let _ = app.run("ui.bottom", Value::Null);
    }
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

/// Overlays drawn over the viewport image: the view name.
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

    #[test]
    fn every_chrome_command_is_known() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", Value::Null).unwrap();
        let items: Vec<Item> = sidebar_groups()
            .into_iter()
            .flatten()
            .chain(header_modes())
            .chain(header_view().into_iter().flatten())
            .chain(settings_items().into_iter().filter(|it| !it.command.starts_with("file.")))
            .chain(object_actions())
            .collect();
        for it in items {
            let params = if it.needs_selection { json!({ "target": app.selected_targets()[0] }) } else { (it.params)() };
            // Ops may refuse a fresh box (no holes to fill, nothing to apply);
            // the command itself must exist.
            if let Err(e) = app.run(it.command, params) {
                assert!(!e.starts_with(t("unknown command")), "{}: {e}", it.command);
            }
            // Keep one object selected for the selection tools.
            if app.selected_targets().is_empty() {
                app.run("edit.select_all", Value::Null).unwrap();
            }
        }
    }
}
