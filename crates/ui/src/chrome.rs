//! Window chrome, laid out like Blender's: a top bar with the menus and the
//! title, a tool settings row under it, a vertical tool column on the left,
//! a header strip on top of the viewport (selection modes and shading), the
//! status bar, the Start page, viewport overlays and keyboard shortcuts.

use egui::{Align2, Key, KeyboardShortcut, Modifiers, Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::icons::{self, Icon, Look};
use crate::theme::{self, Tokens};
use crate::widgets::Tip;
use crate::{PixelApp, SelectMode, Tool};

/// Height of the top bar; on macOS the traffic lights sit in it.
const TOP_BAR: f32 = 36.0;
/// Where top bar content starts right of the macOS traffic lights.
const MAC_CONTENT_LEFT: f32 = 78.0;
/// The tool settings row under the top bar.
const TOOL_SETTINGS: f32 = 36.0;
/// The tool rail: with small labels under the icons, or icons only.
const RAIL: f32 = 58.0;
const RAIL_ICONS: f32 = 46.0;
/// The header strip on top of the viewport.
pub const VIEW_HEADER: f32 = 34.0;

/// One button: icon, command, label, shortcut, description, parameters and
/// whether it shows as on.
struct Item {
    icon: Icon,
    command: &'static str,
    label: &'static str,
    /// The shortcut in `Cmd+Shift+Z` notation, or empty.
    keys: &'static str,
    /// One line on what it does, for the tooltip.
    desc: &'static str,
    params: fn() -> Value,
    selected: fn(&PixelApp) -> bool,
    /// Needs a selected object.
    needs_selection: bool,
}

impl Item {
    fn tip(&self) -> Tip<'static> {
        Tip::new(t(self.label)).desc(t(self.desc)).keys(self.keys)
    }
}

fn no_params() -> Value {
    Value::Null
}

fn never(_: &PixelApp) -> bool {
    false
}

macro_rules! item {
    ($icon:ident, $cmd:literal, $label:literal, $keys:literal, $desc:literal) => {
        Item { icon: Icon::$icon, command: $cmd, label: $label, keys: $keys, desc: $desc, params: no_params, selected: never, needs_selection: false }
    };
    ($icon:ident, $cmd:literal, $label:literal, $keys:literal, $desc:literal, $params:expr, $selected:expr) => {
        Item {
            icon: Icon::$icon,
            command: $cmd,
            label: $label,
            keys: $keys,
            desc: $desc,
            params: $params,
            selected: $selected,
            needs_selection: false,
        }
    };
}

fn on_selection(mut it: Item) -> Item {
    it.needs_selection = true;
    it
}

/// The tool rail, in groups.
fn sidebar_groups() -> Vec<Vec<Item>> {
    vec![
        vec![
            item!(Select, "ui.tool", "Select", "Q", "Click to select, Shift+click to add or remove", || json!({ "tool": "select" }), |a| a.ui.tool
                == Tool::Select),
            item!(Move, "ui.tool", "Move", "W", "Move the selection with the gizmo", || json!({ "tool": "move" }), |a| a.ui.tool == Tool::Move),
            item!(Rotate, "ui.tool", "Rotate", "E", "Rotate the selection with the gizmo", || json!({ "tool": "rotate" }), |a| a.ui.tool
                == Tool::Rotate),
            item!(Scale, "ui.tool", "Scale", "R", "Scale the selection with the gizmo", || json!({ "tool": "scale" }), |a| a.ui.tool == Tool::Scale),
        ],
        vec![
            item!(Box, "prim.box", "Box", "", "Add a box at the origin"),
            item!(Cylinder, "prim.cylinder", "Cylinder", "", "Add a cylinder at the origin"),
            item!(Sphere, "prim.quad_sphere", "Sphere", "", "Add a quad sphere at the origin"),
            item!(Plane, "prim.plane", "Plane", "", "Add a flat grid at the origin"),
            item!(Empty, "scene.create_empty", "Empty", "", "Add an empty: a transform without a mesh"),
        ],
    ]
}

/// Actions on the selected objects, shown in the tool settings row while
/// something is selected (the contextual toolbar).
fn object_actions() -> Vec<Item> {
    vec![
        on_selection(item!(Apply, "scene.apply_transform", "Apply transform", "", "Bake the object's transform into its mesh")),
        on_selection(item!(Fill, "edit.fill", "Fill holes", "", "Close each hole with one face")),
        on_selection(item!(Normals, "edit.recalc_normals", "Recalculate normals", "", "Make every face point outward")),
        item!(Copy, "edit.duplicate", "Duplicate", "Cmd+D", "Copy the selected objects"),
        item!(Trash, "edit.delete", "Delete", "Delete", "Remove the selected objects"),
    ]
}

/// Buttons of the tool settings row (file and history).
fn settings_items() -> Vec<Item> {
    vec![
        item!(New, "file.new", "New", "Cmd+N", "Start an empty scene"),
        item!(Open, "file.open", "Open", "Cmd+O", "Open a .p3d file"),
        item!(Save, "file.save", "Save", "Cmd+S", "Save to the current file"),
        item!(Undo, "edit.undo", "Undo", "Cmd+Z", "Step back one change"),
        item!(Redo, "edit.redo", "Redo", "Cmd+Shift+Z", "Step forward again"),
    ]
}

/// The viewport header: selection modes on the left, view toggles and
/// shading on the right.
fn header_modes() -> Vec<Item> {
    vec![
        item!(ModeObject, "ui.mode", "Object mode", "1", "Select whole objects", || json!({ "mode": "object" }), |a| a.ui.mode == SelectMode::Object),
        item!(ModeVert, "ui.mode", "Vertex mode", "2", "Select vertices", || json!({ "mode": "vert" }), |a| a.ui.mode == SelectMode::Vert),
        item!(ModeEdge, "ui.mode", "Edge mode", "3", "Select edges", || json!({ "mode": "edge" }), |a| a.ui.mode == SelectMode::Edge),
        item!(ModeFace, "ui.mode", "Face mode", "4", "Select faces", || json!({ "mode": "face" }), |a| a.ui.mode == SelectMode::Face),
    ]
}

fn header_view() -> Vec<Vec<Item>> {
    vec![
        vec![
            item!(Frame, "view.frame", "Frame selection", "F", "Fit the selection, or everything, in the view"),
            item!(Grid, "view.grid", "Ground grid", "", "Show or hide the ground grid", no_params, |a| a.ui.view.grid),
            item!(Orthographic, "view.ortho", "Orthographic", "5", "Switch between perspective and orthographic", no_params, |a| a.ui.view.ortho),
        ],
        vec![
            item!(ShadeWire, "view.shading", "Wireframe", "", "Edges only", || json!({ "mode": "wire" }), |a| a.ui.view.shading == "wire"),
            item!(ShadeSolid, "view.shading", "Solid", "", "Lit surfaces", || json!({ "mode": "solid" }), |a| a.ui.view.shading == "solid"),
            item!(
                ShadeSolidWire,
                "view.shading",
                "Solid with edges",
                "",
                "Lit surfaces and polygon edges",
                || json!({ "mode": "solid_wire" }),
                |a| a.ui.view.shading == "solid_wire"
            ),
            item!(ShadeMatcap, "view.shading", "Clay", "", "Gray clay that shows form", || json!({ "mode": "matcap" }), |a| a.ui.view.shading
                == "matcap"),
        ],
    ]
}

/// Draws one icon button in `rect`; returns whether it was clicked.
fn button_at(ui: &mut egui::Ui, rect: Rect, it: &Item, app: &PixelApp, enabled: bool) -> bool {
    let selected = (it.selected)(app);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            icons::button_with(ui, it.icon, rect.size(), it.tip(), selected, Look::Segment, egui::RectAlign::BOTTOM).clicked()
        })
        .inner
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
        let menus_right = menu_bar(app, ui, left, r);
        let tabs_left = workspace_tabs(app, ui, r);
        let title = if app.ui.start_page { t("Start").to_owned() } else { app.title() };
        let galley = ui.painter().layout_no_wrap(format!("pixel3d  {title}"), theme::body(), tk.text_dim);
        let x = (r.center().x - galley.size().x / 2.0).max(menus_right + 24.0);
        if x + galley.size().x < tabs_left - 16.0 {
            ui.painter().galley(pos2(x, r.center().y - galley.size().y / 2.0), galley, tk.text_dim);
        }
    });
}

/// The workspace tabs at the right end of the top bar; returns their left
/// edge. Workspaces whose editors do not exist yet are disabled.
fn workspace_tabs(app: &mut PixelApp, ui: &mut egui::Ui, r: Rect) -> f32 {
    let tk = Tokens::get();
    let widths: Vec<f32> =
        crate::WorkspaceKind::ALL.iter().map(|w| ui.painter().layout_no_wrap(w.label().to_owned(), theme::body(), tk.text).size().x + 20.0).collect();
    let total: f32 = widths.iter().sum();
    let mut x = r.right() - 8.0 - total;
    let left = x;
    let mut chosen = None;
    for (ws, w) in crate::WorkspaceKind::ALL.iter().zip(widths) {
        let rect = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(w, 26.0));
        let resp = ui.interact(rect, ui.id().with(("workspace", *ws)), Sense::click());
        let current = app.ui.workspace == *ws;
        let enabled = ws.unavailable().is_none();
        if current {
            ui.painter().rect_filled(rect, 6.0, tk.tab_active);
        } else if resp.hovered() && enabled {
            ui.painter().rect_filled(rect, 6.0, tk.control.gamma_multiply(0.6));
        }
        let color = if current {
            tk.text
        } else if enabled {
            tk.text_dim
        } else {
            tk.text_faint.gamma_multiply(0.7)
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, ws.label(), theme::body(), color);
        icons::describe_control(ui, &resp, ws.label(), Some(current));
        let tip = ws.unavailable().map_or_else(|| format!("{} {}", ws.label(), t("workspace")), str::to_owned);
        if resp.on_hover_text(tip).clicked() && enabled {
            chosen = Some(*ws);
        }
        x += w;
    }
    if let Some(ws) = chosen {
        let _ = app.run("ui.workspace", json!({ "workspace": ws }));
    }
    left
}

/// The menus (also the macOS native menu's contents): title buttons from
/// `left` that open dropdowns; while one is open, hovering another title
/// switches to it. Returns the right edge of the titles.
fn menu_bar(app: &mut PixelApp, ui: &mut egui::Ui, left: f32, bar: Rect) -> f32 {
    let tk = Tokens::get();
    let menus = crate::menus::menus();
    let mut x = left;
    let mut titles = Vec::with_capacity(menus.len());
    for menu in &menus {
        let g = ui.painter().layout_no_wrap(t(menu.label).to_owned(), theme::body(), tk.text);
        let rect = Rect::from_min_size(pos2(x, bar.center().y - 13.0), vec2(g.size().x + 20.0, 26.0));
        let resp = ui.interact(rect, ui.id().with(("menu", menu.label)), Sense::click());
        x = rect.right() + 1.0;
        titles.push((resp, g));
    }
    let ids: Vec<egui::Id> = titles.iter().map(|(r, _)| egui::Popup::default_response_id(r)).collect();
    let any_open = ids.iter().any(|id| egui::Popup::is_id_open(ui.ctx(), *id));
    let mut clicked: Option<(&'static str, Value)> = None;
    for ((menu, (resp, g)), id) in menus.iter().zip(titles).zip(ids) {
        if any_open && resp.hovered() && !egui::Popup::is_id_open(ui.ctx(), id) {
            egui::Popup::open_id(ui.ctx(), id);
        }
        let open = egui::Popup::is_id_open(ui.ctx(), id);
        let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered() || open, 0.1);
        if hot > 0.0 {
            ui.painter().rect_filled(resp.rect, 6.0, tk.control.gamma_multiply(hot));
        }
        let color = crate::widgets::mix(tk.text_dim, tk.text, hot);
        ui.painter().galley(resp.rect.center() - g.size() / 2.0, g, color);
        icons::describe_control(ui, &resp, t(menu.label), Some(open));
        let frame = egui::Frame::NONE
            .fill(tk.popup)
            .stroke(Stroke::new(1.0, tk.popup_border))
            .corner_radius(10)
            .inner_margin(egui::Margin::same(5))
            .shadow(ui.visuals().popup_shadow);
        egui::Popup::menu(&resp).frame(frame).gap(4.0).align(egui::RectAlign::BOTTOM_START).width(236.0).show(|ui| {
            if let Some(c) = menu_body(app, ui, menu) {
                clicked = Some(c);
            }
        });
    }
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
    x
}

/// The items of one dropdown; returns the clicked item's command.
fn menu_body(app: &PixelApp, ui: &mut egui::Ui, menu: &crate::menus::Menu) -> Option<(&'static str, Value)> {
    let tk = Tokens::get();
    ui.spacing_mut().item_spacing.y = 1.0;
    let icon_column = menu.entries.iter().flatten().any(|it| it.icon.is_some());
    let mut clicked = None;
    for entry in &menu.entries {
        let w = ui.available_width();
        match entry {
            None => {
                let (r, _) = ui.allocate_exact_size(vec2(w, 9.0), Sense::hover());
                ui.painter().hline(r.left() + 8.0..=r.right() - 8.0, r.center().y, Stroke::new(1.0, tk.card_border));
            }
            Some(it) if it.is_header() => {
                let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                ui.painter().text(pos2(r.left() + 10.0, r.center().y + 2.0), Align2::LEFT_CENTER, t(it.label), theme::small(), tk.text_faint);
            }
            Some(it) => {
                if menu_item(ui, app, it, icon_column).clicked() {
                    clicked = Some((it.command, (it.params)()));
                }
            }
        }
    }
    clicked
}

/// One dropdown row: icon, label, shortcut and a check mark when on.
fn menu_item(ui: &mut egui::Ui, app: &PixelApp, it: &crate::menus::MenuItem, icon_column: bool) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.08);
    let p = ui.painter();
    if hot > 0.0 {
        p.rect_filled(rect, 6.0, tk.control.gamma_multiply(hot));
    }
    let on = it.checked.is_some_and(|f| f(app));
    let mut x = rect.left() + 10.0;
    if icon_column {
        if let Some(icon) = it.icon {
            let ir = Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(16.0, 16.0));
            let color = if on { tk.icon_accent } else { crate::widgets::mix(tk.icon, tk.text, hot) };
            icons::paint_colored(p, ir, icon, color, tk.accent);
        }
        x += 28.0;
    }
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, t(it.label), theme::body(), tk.text);
    let mut right = rect.right() - 10.0;
    if on {
        let c = pos2(right - 5.0, rect.center().y);
        p.line(vec![c + vec2(-4.5, 0.0), c + vec2(-1.5, 3.0), c + vec2(4.5, -3.5)], Stroke::new(1.6, tk.icon_accent));
        right -= 18.0;
    }
    if let Some(s) = &it.shortcut {
        p.text(pos2(right, rect.center().y), Align2::RIGHT_CENTER, ui.ctx().format_shortcut(s), theme::small(), tk.text_faint);
    }
    icons::describe_control(ui, &resp, t(it.label), it.checked.map(|_| on));
    crate::control::record_widget(ui, &resp, it.label, "menuitem", it.checked.map(|_| on));
    resp
}

/// Size of a button in a segmented group, and the group's inner padding.
const SEGMENT: f32 = 26.0;
const SEGMENT_PAD: f32 = 2.0;

/// Width of a segmented group of `n` buttons.
fn segment_width(n: usize) -> f32 {
    n as f32 * SEGMENT + n.saturating_sub(1) as f32 + SEGMENT_PAD * 2.0
}

/// A segmented group of icon buttons on an inset track, from `left`,
/// centered on `cy`. Returns the clicked item's command and parameters.
fn segment(ui: &mut egui::Ui, app: &PixelApp, items: &[Item], left: f32, cy: f32) -> Option<(&'static str, Value)> {
    let tk = Tokens::get();
    let track = Rect::from_min_size(pos2(left, cy - SEGMENT / 2.0 - SEGMENT_PAD), vec2(segment_width(items.len()), SEGMENT + SEGMENT_PAD * 2.0));
    ui.painter().rect_filled(track, 8.0, tk.chrome_dark);
    let mut clicked = None;
    let mut x = track.left() + SEGMENT_PAD;
    for it in items {
        let r = Rect::from_min_size(pos2(x, track.top() + SEGMENT_PAD), vec2(SEGMENT, SEGMENT));
        if button_at(ui, r, it, app, true) {
            clicked = Some((it.command, (it.params)()));
        }
        x += SEGMENT + 1.0;
    }
    clicked
}

/// An outlined chip with an icon and a label; returns the response.
fn chip(ui: &mut egui::Ui, rect: Rect, it: &Item) -> egui::Response {
    let tk = Tokens::get();
    let resp = ui.interact(rect, ui.id().with(("chip", it.command)), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect(rect, 7.0, crate::widgets::mix(egui::Color32::TRANSPARENT, tk.control, hot), Stroke::new(1.0, tk.control), egui::StrokeKind::Inside);
    let color = crate::widgets::mix(tk.icon, tk.text, hot);
    icons::paint_colored(p, Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 7.5), vec2(15.0, 15.0)), it.icon, color, tk.accent);
    p.text(
        pos2(rect.left() + 29.0, rect.center().y),
        Align2::LEFT_CENTER,
        t(it.label),
        theme::body(),
        crate::widgets::mix(tk.text_dim, tk.text, hot),
    );
    icons::describe_control(ui, &resp, t(it.label), None);
    crate::widgets::tooltip(&resp, it.tip(), egui::RectAlign::BOTTOM);
    resp
}

/// The tool settings row: file and history buttons, then the active tool
/// and either what it does or actions on the selection.
pub fn tool_settings(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    egui::Panel::top("p3d_tool_settings").exact_size(TOOL_SETTINGS).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, tk.border));
        let cy = r.center().y;
        let items = settings_items();
        let (file, history) = items.split_at(3);
        let mut x = r.left() + 8.0;
        let mut clicked = segment(ui, app, file, x, cy);
        x += segment_width(file.len()) + 6.0;
        clicked = segment(ui, app, history, x, cy).or(clicked);
        x += segment_width(history.len()) + 14.0;
        ui.painter().vline(x, r.y_range().shrink(10.0), Stroke::new(1.0, tk.control));
        x += 14.0;
        let (name, what) = match app.ui.tool {
            Tool::Select => (t("Select"), t("Click an object to select it; Shift+click adds or removes.")),
            Tool::Move => (t("Move"), t("Drag the gizmo arrows to move the selection.")),
            Tool::Rotate => (t("Rotate"), t("Drag the gizmo rings to rotate the selection.")),
            Tool::Scale => (t("Scale"), t("Drag the gizmo handles to scale the selection.")),
        };
        let g = ui.painter().layout_no_wrap(name.to_owned(), egui::FontId::proportional(13.0), tk.text);
        let w = g.size().x;
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, tk.text);
        x += w + 16.0;
        let mut action = None;
        if app.selected_targets().is_empty() {
            ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, what, theme::small(), tk.text_dim);
        } else {
            for it in object_actions() {
                let text_w = ui.painter().layout_no_wrap(t(it.label).to_owned(), theme::body(), tk.text).size().x;
                let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(29.0 + text_w + 10.0, 26.0));
                if rect.right() > r.right() - 8.0 {
                    break;
                }
                if chip(ui, rect, &it).clicked() {
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

/// The tool rail on the left: icon pills with small labels under them, in
/// groups. Right-click hides or shows the labels.
pub fn tool_sidebar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let labels = app.ui.rail_labels;
    let width = if labels { RAIL } else { RAIL_ICONS };
    let item_h = if labels { 48.0 } else { 36.0 };
    let has_selection = !app.selected_targets().is_empty();
    egui::Panel::left("p3d_tools").exact_size(width).resizable(false).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let r = ui.max_rect();
        ui.painter().vline(r.right() - 0.5, r.y_range(), Stroke::new(1.0, tk.border));
        let back = ui.interact(r, ui.id().with("toolbar-bg"), Sense::click());
        back.context_menu(|ui| {
            let label = if labels { t("Icons only") } else { t("Show labels") };
            if ui.button(label).clicked() {
                let _ = app.run("ui.rail_labels", Value::Null);
                ui.close();
            }
        });
        let mut y = r.top() + 8.0;
        let mut clicked = None;
        for (gi, group) in sidebar_groups().iter().enumerate() {
            if gi > 0 {
                y += 4.0;
                ui.painter().hline(r.center().x - 12.0..=r.center().x + 12.0, y, Stroke::new(1.0, tk.control));
                y += 7.0;
            }
            for it in group {
                let enabled = !it.needs_selection || has_selection;
                let rect = Rect::from_min_size(pos2(r.left() + 2.0, y), vec2(width - 4.0, item_h));
                let resp = ui
                    .add_enabled_ui(enabled, |ui| {
                        crate::widgets::rail_item(ui, rect, it.icon, labels.then(|| t(it.label)), it.tip(), (it.selected)(app))
                    })
                    .inner;
                if resp.clicked() {
                    clicked = Some((it.command, it.needs_selection, (it.params)()));
                }
                y += item_h + 2.0;
            }
        }
        if let Some((command, needs_selection, params)) = clicked {
            let params = match (needs_selection, app.selected_targets().first()) {
                (true, Some(id)) => json!({ "target": id }),
                _ => params,
            };
            let _ = app.run(command, params);
        }
    });
}

/// The viewport header strip at the top of `rect`; returns the area left
/// for the 3D view.
pub fn viewport_header(app: &mut PixelApp, ui: &mut egui::Ui, rect: Rect) -> Rect {
    let tk = Tokens::get();
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), VIEW_HEADER));
    ui.painter().rect_filled(header, 0.0, tk.chrome);
    ui.painter().hline(header.x_range(), header.bottom() - 0.5, Stroke::new(1.0, tk.border));
    let cy = header.center().y;
    let modes = header_modes();
    let mut clicked = segment(ui, app, &modes, header.left() + 8.0, cy);
    let label_x = header.left() + 8.0 + segment_width(modes.len()) + 10.0;
    ui.painter().text(pos2(label_x, cy), Align2::LEFT_CENTER, app.ui.mode.label(), theme::small(), tk.text_dim);
    let mut rx = header.right() - 8.0;
    for group in header_view().iter().rev() {
        rx -= segment_width(group.len());
        clicked = segment(ui, app, group, rx, cy).or(clicked);
        rx -= 6.0;
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
    let mut toggle_snap = false;
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
        let jobs = app.running_jobs();
        // Right to left: fps, jobs, class and profile, snap, units, triangles.
        let class = format!("{}, {}", doc.settings.asset_class.as_str(), t("no profile"));
        let items: [(String, Option<bool>, &str); 6] = [
            (format!("{:.0} {}", app.viewport.fps, t("fps")), None, ""),
            (if jobs == 0 { t("no jobs").to_owned() } else { format!("{jobs} {}", t("jobs")) }, None, t("Background work (health checks)")),
            (class, None, t("Asset class and engine profile")),
            (t("Snap").to_owned(), Some(app.ui.view.snap), t("Snap transform drags to the grid and 15 degree steps")),
            (t("meters").to_owned(), None, t("Display units")),
            (format!("{} / {} {}", thousands(tris), thousands(budget), t("tris")), None, t("Triangles against the asset class budget")),
        ];
        let mut snap_clicked = false;
        for (i, (s, toggle, tip)) in items.iter().enumerate() {
            let color = match (i, toggle) {
                (5, _) if tris > budget => tk.health_warn,
                (_, Some(true)) => tk.text,
                _ => tk.text_dim,
            };
            let g = p.layout_no_wrap(s.clone(), theme::small(), color);
            let w = g.size().x;
            rx -= w;
            let rect = Rect::from_min_size(pos2(rx - 5.0, r.top() + 3.0), vec2(w + 10.0, r.height() - 6.0));
            if let Some(on) = toggle {
                let resp = ui.interact(rect, ui.id().with(("status", i)), Sense::click());
                if *on {
                    p.rect_filled(rect, 3.0, tk.toggle_on.gamma_multiply(0.35));
                } else if resp.hovered() {
                    p.rect_filled(rect, 3.0, tk.control_hover.gamma_multiply(0.5));
                }
                icons::describe_control(ui, &resp, s, Some(*on));
                snap_clicked |= resp.on_hover_text(*tip).clicked();
            } else if !tip.is_empty() {
                ui.interact(rect, ui.id().with(("status", i)), Sense::hover()).on_hover_text(*tip);
            }
            p.galley(pos2(rx, y - g.size().y / 2.0), g, color);
            rx -= 18.0;
        }
        if snap_clicked {
            toggle_snap = true;
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
    if toggle_snap {
        let _ = app.run("view.snap", Value::Null);
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
