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
    (op $icon:ident, $op:literal, $label:literal, $keys:literal, $desc:literal, then $then:literal) => {
        Item {
            icon: Icon::$icon,
            command: "edit.element_op",
            label: $label,
            keys: $keys,
            desc: $desc,
            params: || json!({ "op": $op, "then": $then }),
            selected: never,
            needs_selection: false,
        }
    };
    (op $icon:ident, $op:literal, $label:literal, $keys:literal, $desc:literal) => {
        Item {
            icon: Icon::$icon,
            command: "edit.element_op",
            label: $label,
            keys: $keys,
            desc: $desc,
            params: || json!({ "op": $op }),
            selected: never,
            needs_selection: false,
        }
    };
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

/// The tool rail, in groups.
fn sidebar_groups(edit: bool) -> Vec<Vec<Item>> {
    let tools = vec![
        item!(Select, "ui.tool", "Select", "Q", "Click to select, Shift+click to add or remove", || json!({ "tool": "select" }), |a| a.ui.tool
            == Tool::Select),
        item!(Move, "ui.tool", "Move", "W", "Move the selection with the gizmo (G moves with the pointer)", || json!({ "tool": "move" }), |a| a
            .ui
            .tool
            == Tool::Move),
        item!(
            Rotate,
            "ui.tool",
            "Rotate",
            "E",
            "Rotate the selection with the gizmo (R turns with the pointer)",
            || json!({ "tool": "rotate" }),
            |a| a.ui.tool == Tool::Rotate
        ),
        item!(Scale, "ui.tool", "Scale", "T", "Scale the selection with the gizmo (S scales with the pointer)", || json!({ "tool": "scale" }), |a| a
            .ui
            .tool
            == Tool::Scale),
    ];
    if edit {
        return vec![tools, edit_tools(), edit_extras()];
    }
    vec![
        tools,
        vec![
            item!(Box, "prim.box", "Box", "", "Add a box at the origin"),
            item!(Cylinder, "prim.cylinder", "Cylinder", "", "Add a cylinder at the origin"),
            item!(Sphere, "prim.quad_sphere", "Sphere", "", "Add a quad sphere at the origin"),
            item!(Plane, "prim.plane", "Plane", "", "Add a flat grid at the origin"),
            item!(Empty, "scene.create_empty", "Empty", "", "Add an empty: a transform without a mesh"),
        ],
    ]
}

/// Edit mode: tools that reshape the selected elements (Blender's toolbar).
fn edit_tools() -> Vec<Item> {
    vec![
        item!(op Extrude, "edit.extrude", "Extrude", "E", "Pull the selected faces out along their normals, with new side walls"),
        item!(op Inset, "edit.inset", "Inset", "I", "Shrink the selected faces inward, with a ring of new faces"),
        item!(op Bevel, "edit.bevel", "Bevel", "Ctrl+B", "Round off the selected edges or vertices"),
        item!(op LoopCut, "edit.loop_cut", "Loop Cut", "Ctrl+R", "Cut a loop across the ring of quads of each selected edge"),
        item!(
            Knife,
            "ui.tool",
            "Knife",
            "K",
            "Drag a line across faces to cut them (x-ray cuts through to the back)",
            || json!({ "tool": "knife" }),
            |a| a.ui.tool == Tool::Knife
        ),
        item!(op Smooth, "edit.smooth", "Smooth", "", "Relax the selected vertices (the whole mesh when none)"),
        item!(op EdgeSlide, "edit.edge_slide", "Edge Slide", "", "Slide the selected edges along their neighbors"),
        item!(op ShrinkFatten, "edit.shrink_fatten", "Shrink/Fatten", "Alt+S", "Move the selected vertices along their normals"),
        item!(op Rip, "edit.split", "Rip Region", "V", "Tear the selected faces free, then move them with the gizmo", then "move"),
    ]
}

/// Edit mode: more mesh operations on the selection.
fn edit_extras() -> Vec<Item> {
    vec![
        item!(op Poke, "edit.poke", "Poke", "", "Add a vertex in the middle of each selected face"),
        item!(op Triangulate, "edit.triangulate", "Triangulate", "Ctrl+T", "Split the selected faces into triangles"),
        item!(op Merge, "edit.merge", "Merge", "M", "Merge the selected vertices into one"),
        item!(op Dissolve, "edit.dissolve", "Dissolve", "", "Remove the selected elements and join the faces around them"),
        item!(op Subdivide, "edit.subdivide", "Subdivide", "", "Split the selected faces into smaller ones"),
        item!(op Bridge, "edit.bridge", "Bridge", "", "Connect two edge loops with faces"),
        item!(op Fill, "edit.fill", "Fill", "", "Close holes with faces"),
        item!(op Split, "edit.split", "Split", "Y", "Detach the selected faces from their neighbors"),
        item!(op Symmetrize, "edit.symmetrize", "Symmetrize", "", "Mirror one half of the mesh onto the other"),
    ]
}

/// App commands that close the contextual toolbar in Object Mode (the ops
/// before them are generated from the registry, `actions.rs`).
fn object_actions() -> Vec<Item> {
    vec![
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

/// The viewport header: the element select modes (edit mode) after the
/// mode menu, view toggles and shading on the right.
fn header_modes() -> Vec<Item> {
    vec![
        item!(ModeVert, "ui.mode", "Vertex select", "1", "Select vertices", || json!({ "mode": "vert" }), |a| a.ui.mode == SelectMode::Vert),
        item!(ModeEdge, "ui.mode", "Edge select", "2", "Select edges", || json!({ "mode": "edge" }), |a| a.ui.mode == SelectMode::Edge),
        item!(ModeFace, "ui.mode", "Face select", "3", "Select faces", || json!({ "mode": "face" }), |a| a.ui.mode == SelectMode::Face),
    ]
}

fn header_view() -> Vec<Vec<Item>> {
    vec![
        vec![
            item!(Frame, "view.frame", "Frame selection", "F", "Fit the selection, or everything, in the view"),
            item!(Grid, "view.grid", "Ground grid", "", "Show or hide the ground grid", no_params, |a| a.ui.view.grid),
            item!(Orthographic, "view.ortho", "Orthographic", "5", "Switch between perspective and orthographic", no_params, |a| a.ui.view.ortho),
        ],
        vec![item!(XRay, "view.xray", "X-ray", "Alt+Z", "See through surfaces to the edges and objects behind them", no_params, |a| a.ui.view.xray)],
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
        let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.1);
        if current {
            ui.painter().rect_filled(rect, 6.0, tk.tab_active);
        } else if hot > 0.0 {
            ui.painter().rect_filled(rect, 6.0, tk.control.gamma_multiply(0.6 * hot));
        }
        let color = if current { tk.text } else { crate::widgets::mix(tk.text_dim, tk.text, hot) };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, ws.label(), theme::body(), color);
        icons::describe_control(ui, &resp, ws.label(), Some(current));
        crate::widgets::tooltip(&resp, Tip::new(ws.label()).desc(ws.describe()), egui::RectAlign::BOTTOM);
        if resp.clicked() {
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
    ui.spacing_mut().item_spacing.y = 1.0;
    let icon_column = menu.entries.iter().flatten().any(|it| it.icon.is_some());
    let mut clicked = None;
    for entry in &menu.entries {
        match entry {
            None => crate::widgets::menu_separator(ui),
            Some(it) if it.is_header() => crate::widgets::menu_header(ui, t(it.label)),
            Some(it) => {
                if menu_item(ui, app, it, icon_column).clicked() {
                    clicked = Some((it.command, (it.params)()));
                }
            }
        }
    }
    clicked
}

/// One dropdown row: icon, label, shortcut and a check mark when on. An
/// item that cannot run now is dimmed and says why on hover.
fn menu_item(ui: &mut egui::Ui, app: &PixelApp, it: &crate::menus::MenuItem, icon_column: bool) -> egui::Response {
    let on = it.checked.map(|f| f(app));
    let keys = it.shortcut.as_ref().map(|s| crate::menus::shortcut_text(ui.ctx(), s));
    let state = it.enabled.map_or(Ok(()), |f| f(app));
    let resp = ui.add_enabled_ui(state.is_ok(), |ui| crate::widgets::menu_row(ui, it.icon, t(it.label), keys.as_deref(), on, icon_column)).inner;
    match state {
        Err(why) => resp.on_disabled_hover_text(why),
        Ok(()) => resp,
    }
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

/// An outlined chip for a toolbar item.
fn chip(ui: &mut egui::Ui, rect: Rect, it: &Item) -> egui::Response {
    chip_with(ui, rect, it.command, it.icon, t(it.label), it.tip())
}

/// An outlined chip with an icon and a label; returns the response.
fn chip_with(ui: &mut egui::Ui, rect: Rect, key: &str, icon: Icon, label: &str, tip: Tip<'_>) -> egui::Response {
    let tk = Tokens::get();
    let resp = ui.interact(rect, ui.id().with(("chip", key)), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect(rect, 7.0, crate::widgets::mix(egui::Color32::TRANSPARENT, tk.control, hot), Stroke::new(1.0, tk.control), egui::StrokeKind::Inside);
    let color = crate::widgets::mix(tk.icon, tk.text, hot);
    icons::paint_colored(p, Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 7.5), vec2(15.0, 15.0)), icon, color, tk.accent);
    p.text(pos2(rect.left() + 29.0, rect.center().y), Align2::LEFT_CENTER, label, theme::body(), crate::widgets::mix(tk.text_dim, tk.text, hot));
    icons::describe_control(ui, &resp, label, None);
    crate::widgets::tooltip(&resp, tip, egui::RectAlign::BOTTOM);
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
            Tool::Knife => (t("Knife"), t("Drag a line across faces to cut them; with x-ray the cut goes through to the back.")),
        };
        let g = ui.painter().layout_no_wrap(name.to_owned(), egui::FontId::proportional(13.0), tk.text);
        let w = g.size().x;
        ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, tk.text);
        x += w + 16.0;
        let mut action = None;
        if let Some(text) = crate::gizmo::status_text(app) {
            // A transform in progress: its value and the keys that steer it.
            let g = ui.painter().layout_no_wrap(text, theme::mono(), tk.icon_accent);
            let w = g.size().x;
            ui.painter().galley(pos2(x, cy - g.size().y / 2.0), g, tk.icon_accent);
            let hint = t("X Y Z constrain, Shift+X plane, type a value, Enter confirms, Esc cancels");
            ui.painter().text(pos2(x + w + 18.0, cy), Align2::LEFT_CENTER, hint, theme::small(), tk.text_dim);
        } else {
            let acts = app.contextual_actions();
            let edit = app.ui.mode.is_edit();
            // Duplicate and Delete close the row in Object Mode.
            let tail: Vec<Item> = if edit || app.selected_targets().is_empty() { Vec::new() } else { object_actions() };
            if acts.is_empty() && tail.is_empty() {
                let hint = if edit { t("Pick vertices, edges or faces to see what can run on them.") } else { what };
                ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, hint, theme::small(), tk.text_dim);
            } else {
                let measure = ui.painter().clone();
                let width_of = |label: &str| measure.layout_no_wrap(label.to_owned(), theme::body(), tk.text).size().x + 39.0;
                let tail_w: f32 = tail.iter().map(|it| width_of(t(it.label)) + 6.0).sum();
                let more_w = 74.0;
                let limit = r.right() - 8.0 - tail_w - more_w;
                let mut hovered: Option<crate::actions::Action> = None;
                let mut run: Option<crate::actions::Action> = None;
                let mut shown = 0;
                for a in acts.iter() {
                    let w = width_of(&a.label);
                    if x + w > limit {
                        break;
                    }
                    let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(w, 26.0));
                    let tip = Tip::new(&a.label).desc(&a.summary);
                    let resp = chip_with(ui, rect, a.op, a.icon, &a.label, tip);
                    if resp.hovered() {
                        hovered = Some(a.clone());
                    }
                    if resp.clicked() {
                        run = Some(a.clone());
                    }
                    x += w + 6.0;
                    shown += 1;
                }
                if shown < acts.len() {
                    // The rest in a menu, grouped by category.
                    let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(more_w - 6.0, 26.0));
                    let tip = Tip::new(t("More actions")).desc(t("Everything that can run on the selection"));
                    let resp = chip_with(ui, rect, "more-actions", Icon::Menu, t("More"), tip);
                    egui::Popup::menu(&resp).frame(crate::widgets::popup_frame(ui)).gap(4.0).align(egui::RectAlign::BOTTOM_START).width(260.0).show(
                        |ui| {
                            egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                let mut group = None;
                                for a in acts.iter().skip(shown) {
                                    let g = crate::actions::category_label(a.category);
                                    if group != Some(g) {
                                        crate::widgets::menu_header(ui, t(g));
                                        group = Some(g);
                                    }
                                    let row = crate::widgets::menu_row(ui, Some(a.icon), &a.label, None, None, true);
                                    let row = row.on_hover_text(&a.summary);
                                    if row.hovered() {
                                        hovered = Some(a.clone());
                                    }
                                    if row.clicked() {
                                        run = Some(a.clone());
                                    }
                                }
                            });
                        },
                    );
                    x = rect.right() + 6.0;
                }
                for it in tail {
                    let w = width_of(t(it.label));
                    let rect = Rect::from_min_size(pos2(x, cy - 13.0), vec2(w, 26.0));
                    if chip(ui, rect, &it).clicked() {
                        action = Some(it);
                    }
                    x = rect.right() + 6.0;
                }
                // Hovering an action previews it; a click runs it.
                crate::actions::hover(app, hovered.as_ref(), ui.ctx());
                if let Some(a) = run
                    && let Err(e) = crate::actions::run(app, &a)
                {
                    app.set_status(e);
                }
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
/// groups; in Edit Mode the mesh tools follow the transform tools. It
/// scrolls when it is taller than the window. Right-click hides or shows
/// the labels.
pub fn tool_sidebar(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let labels = app.ui.rail_labels;
    let width = if labels { RAIL } else { RAIL_ICONS };
    let item_h = if labels { 48.0 } else { 36.0 };
    let has_selection = !app.selected_targets().is_empty();
    let edit = app.ui.mode.is_edit();
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
        let mut clicked = None;
        egui::ScrollArea::vertical().id_salt("rail").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(6.0);
            for (gi, group) in sidebar_groups(edit).iter().enumerate() {
                if gi > 0 {
                    let (sep, _) = ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                    ui.painter().hline(sep.center().x - 12.0..=sep.center().x + 12.0, sep.center().y, Stroke::new(1.0, tk.control));
                }
                for it in group {
                    // Element ops say why they cannot run yet.
                    let state = (it.command == "edit.element_op")
                        .then(|| (it.params)().get("op").and_then(Value::as_str).map(|op| app.element_op_state(op).map(|_| ())))
                        .flatten();
                    let reason = state.as_ref().and_then(|s| s.as_ref().err().cloned());
                    let enabled = (!it.needs_selection || has_selection) && reason.is_none();
                    let (rect, _) = ui.allocate_exact_size(vec2(width, item_h), Sense::hover());
                    let rect = rect.shrink2(vec2(2.0, 0.0));
                    let tip = Tip::new(t(it.label)).desc(reason.as_deref().unwrap_or(t(it.desc))).keys(it.keys);
                    let resp = ui
                        .add_enabled_ui(enabled, |ui| {
                            crate::widgets::rail_item(ui, rect, it.icon, labels.then(|| t(it.label)), tip, (it.selected)(app))
                        })
                        .inner;
                    if resp.clicked() {
                        clicked = Some((it.command, it.needs_selection, (it.params)()));
                    }
                }
            }
            ui.add_space(8.0);
        });
        if let Some((command, needs_selection, params)) = clicked {
            let params = match (needs_selection, app.selected_targets().first()) {
                (true, Some(id)) => json!({ "target": id }),
                _ => params,
            };
            if let Err(e) = app.run(command, params) {
                app.set_status(e);
            }
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
    let (mut left_end, mut clicked) = mode_menu(app, ui, header.left() + 8.0, cy);
    if app.ui.mode.is_edit() {
        let modes = header_modes();
        clicked = segment(ui, app, &modes, left_end + 8.0, cy).or(clicked);
        left_end += 8.0 + segment_width(modes.len());
    }
    let mut rx = header.right() - 8.0;
    for group in header_view().iter().rev() {
        rx -= segment_width(group.len());
        clicked = segment(ui, app, group, rx, cy).or(clicked);
        rx -= 6.0;
    }
    // Orientation, pivot, snapping and proportional editing in the middle.
    if let Some(c) = crate::transform_bar::show(app, ui, header, left_end, rx) {
        clicked = Some(c);
    }
    if let Some((cmd, params)) = clicked {
        let _ = app.run(cmd, params);
    }
    Rect::from_min_max(pos2(rect.left(), header.bottom()), rect.max)
}

/// The interaction mode menu (Object Mode or Edit Mode, Tab toggles) from
/// `left`; returns its right edge and a chosen command.
fn mode_menu(app: &PixelApp, ui: &mut egui::Ui, left: f32, cy: f32) -> (f32, Option<(&'static str, Value)>) {
    let tk = Tokens::get();
    let edit = app.ui.mode.is_edit();
    let label = if edit { t("Edit Mode") } else { t("Object Mode") };
    let g = ui.painter().layout_no_wrap(label.to_owned(), theme::body(), tk.text);
    let rect = Rect::from_min_size(pos2(left, cy - 13.0), vec2(8.0 + 16.0 + 7.0 + g.size().x + 8.0 + 12.0 + 8.0, 26.0));
    let resp = ui.interact(rect, ui.id().with("mode-menu"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered() || open, 0.1);
    let p = ui.painter();
    p.rect_filled(rect, 8.0, crate::widgets::mix(tk.chrome_dark, tk.control, hot));
    let icon = if edit { app.ui.mode_icon() } else { Icon::ModeObject };
    icons::paint_colored(p, Rect::from_min_size(pos2(rect.left() + 8.0, cy - 8.0), vec2(16.0, 16.0)), icon, tk.icon, tk.accent);
    p.galley(pos2(rect.left() + 31.0, cy - g.size().y / 2.0), g, tk.text);
    crate::widgets::chevron(p, pos2(rect.right() - 14.0, cy), 1.0, tk.text_dim);
    icons::describe_control(ui, &resp, label, Some(open));
    crate::widgets::tooltip(
        &resp,
        Tip::new(t("Interaction mode")).desc(t("Object Mode works on whole objects; Edit Mode on vertices, edges and faces")).keys("Tab"),
        egui::RectAlign::BOTTOM,
    );
    let mut chosen = None;
    let frame = egui::Frame::NONE
        .fill(tk.popup)
        .stroke(Stroke::new(1.0, tk.popup_border))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(5))
        .shadow(ui.visuals().popup_shadow);
    egui::Popup::menu(&resp).frame(frame).gap(4.0).align(egui::RectAlign::BOTTOM_START).width(200.0).show(|ui| {
        let entries = [
            crate::menus::MenuItem {
                label: "Object Mode",
                command: "ui.mode",
                params: || json!({ "mode": "object" }),
                shortcut: None,
                icon: Some(Icon::ModeObject),
                checked: Some(|a| !a.ui.mode.is_edit()),
                enabled: None,
            },
            crate::menus::MenuItem {
                label: "Edit Mode",
                command: "ui.mode",
                params: || json!({ "mode": "edit" }),
                shortcut: Some(KeyboardShortcut::new(Modifiers::NONE, Key::Tab)),
                icon: Some(Icon::ModeVert),
                checked: Some(|a| a.ui.mode.is_edit()),
                enabled: None,
            },
        ];
        for it in &entries {
            if menu_item(ui, app, it, true).clicked() {
                chosen = Some((it.command, (it.params)()));
            }
        }
    });
    (rect.right(), chosen)
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
        let tris = app.stats().budget_tris;
        let doc = app.doc();
        let budget = doc.settings.tri_budget() as usize;
        let jobs = app.running_jobs();
        // Right to left: fps, jobs, class and profile, snap, units, triangles.
        let class = format!("{}, {}", doc.settings.asset_class.as_str(), t("no profile"));
        let items: [(String, Option<bool>, &str); 6] = [
            (if app.stable { String::new() } else { format!("{:.0} {}", app.viewport.fps, t("fps")) }, None, ""),
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

/// The 3D cursor: a red and white ring with a dark crosshair, as Blender
/// draws it.
fn draw_cursor(app: &PixelApp, p: &egui::Painter, rect: Rect) {
    let tk = Tokens::get();
    let vp = app.viewport.camera.view_proj(rect.width() / rect.height().max(1.0));
    let clip = vp * app.ui.transform.cursor().extend(1.0);
    if clip.w <= 1e-6 {
        return;
    }
    let n = clip.truncate() / clip.w;
    let at = pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height());
    if !rect.contains(at) {
        return;
    }
    let (r, tau) = (10.0, std::f32::consts::TAU);
    for i in 0..8 {
        let a0 = i as f32 * tau / 8.0;
        let pts: Vec<egui::Pos2> = (0..=6).map(|k| at + egui::Vec2::angled(a0 + tau / 8.0 * k as f32 / 6.0) * r).collect();
        let color = if i % 2 == 0 { tk.axis_x } else { egui::Color32::WHITE };
        p.add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    }
    let dark = egui::Color32::from_black_alpha(210);
    for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
        p.line_segment([at + d * 5.0, at + d * 17.0], Stroke::new(1.2, dark));
    }
}

/// Overlays drawn over the viewport image: the view name and, when on,
/// the scene statistics under it (selected against visible totals).
pub fn viewport_overlay(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let rect = app.viewport.rect;
    if !rect.is_positive() {
        return;
    }
    let p = ui.painter_at(rect);
    // Curve objects: polylines over the scene (they have no faces).
    let key = app.scene_key();
    let doc = std::sync::Arc::clone(app.session.doc());
    let lines = app.curves.get(key, &doc, &app.hidden);
    crate::curves::draw(&p, &app.viewport.camera, rect, lines, &app.selection, tk.icon, tk.accent);
    draw_cursor(app, &p, rect);
    if app.ui.view.stats {
        let s = app.stats();
        let any = s.selected.objects > 0;
        let pair = |sel: usize, total: usize| if any { format!("{} / {}", thousands(sel), thousands(total)) } else { thousands(total) };
        let both = |sel: usize, total: usize| format!("{} / {}", thousands(sel), thousands(total));
        // In element modes the counts are the edited mesh's, as in Blender.
        let edited = (app.ui.mode != crate::SelectMode::Object).then(|| app.edit.mesh().zip(app.edit.flushed())).flatten();
        let rows = match edited {
            Some((m, f)) => {
                let tris: usize = f.faces.iter().map(|&x| m.face_len(x).saturating_sub(2)).sum();
                [
                    (t("Objects"), both(s.selected.objects, s.total.objects)),
                    (t("Vertices"), both(f.verts.len(), m.vert_count())),
                    (t("Edges"), both(f.edges.len(), m.edge_count())),
                    (t("Faces"), both(f.faces.len(), m.face_count())),
                    (t("Triangles"), both(tris, app.edit.object.and_then(|k| app.stats.tris_of(k)).unwrap_or(0))),
                ]
            }
            None => [
                (t("Objects"), both(s.selected.objects, s.total.objects)),
                (t("Vertices"), pair(s.selected.verts, s.total.verts)),
                (t("Edges"), pair(s.selected.edges, s.total.edges)),
                (t("Faces"), pair(s.selected.faces, s.total.faces)),
                (t("Triangles"), pair(s.selected.tris, s.total.tris)),
            ],
        };
        // A soft shadow keeps the text readable over geometry.
        let shadow = if theme::active() == egui::Theme::Dark { egui::Color32::from_black_alpha(150) } else { egui::Color32::from_white_alpha(170) };
        let label_w = rows.iter().map(|(l, _)| p.layout_no_wrap((*l).to_owned(), theme::small(), tk.text_dim).size().x).fold(0.0, f32::max);
        let mut y = rect.top() + 28.0;
        for (label, value) in rows {
            let at = pos2(rect.left() + 10.0, y);
            let vx = at.x + label_w + 14.0;
            p.text(at + vec2(1.0, 1.0), Align2::LEFT_TOP, label, theme::small(), shadow);
            p.text(at, Align2::LEFT_TOP, label, theme::small(), tk.text_dim);
            p.text(pos2(vx + 1.0, y + 1.0), Align2::LEFT_TOP, &value, theme::small(), shadow);
            p.text(pos2(vx, y), Align2::LEFT_TOP, &value, theme::small(), tk.text);
            y += 16.0;
        }
    }
    let label = format!(
        "{}, {}{}",
        if app.ui.view.ortho { t("Orthographic") } else { t("Perspective") },
        match app.ui.view.shading() {
            p3d_render::Shading::Solid => t("Solid"),
            p3d_render::Shading::SolidWire => t("Solid with edges"),
            p3d_render::Shading::Wire => t("Wireframe"),
            p3d_render::Shading::Matcap => t("Clay"),
        },
        if app.ui.view.xray { format!(", {}", t("X-ray")) } else { String::new() }
    );
    p.text(rect.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, label, theme::small(), tk.text_dim);
}

/// While files are dragged over the window: what dropping them does.
pub fn drop_overlay(ui: &egui::Ui) {
    let hovering = ui.ctx().input(|i| i.raw.hovered_files.len());
    if hovering == 0 {
        return;
    }
    let tk = Tokens::get();
    let screen = ui.ctx().content_rect();
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("p3d_drop_overlay")));
    p.rect_filled(screen, 0.0, tk.border.gamma_multiply(0.7));
    let card = Rect::from_center_size(screen.center(), vec2(420.0, 96.0));
    p.rect(card, 12.0, tk.popup, Stroke::new(1.5, tk.accent), egui::StrokeKind::Inside);
    icons::paint_colored(&p, Rect::from_center_size(pos2(card.left() + 44.0, card.center().y), vec2(30.0, 30.0)), Icon::Import, tk.icon, tk.accent);
    p.text(pos2(card.left() + 78.0, card.center().y - 11.0), Align2::LEFT_CENTER, t("Drop to import"), egui::FontId::proportional(16.0), tk.text);
    p.text(
        pos2(card.left() + 78.0, card.center().y + 13.0),
        Align2::LEFT_CENTER,
        t(".glb, .gltf, .obj and .fbx join the scene; a .p3d opens"),
        theme::small(),
        tk.text_dim,
    );
}

/// A shortcut, its command and the command parameters.
type Binding = (KeyboardShortcut, &'static str, fn() -> Value);

/// Keyboard shortcuts. Nothing fires while a text field has the keyboard.
pub fn shortcuts(app: &mut PixelApp, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    // A modal transform reads the keys itself (X, Y, Z, typed values).
    if app.gizmo.modal() {
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
        (KeyboardShortcut::new(cmd, Key::J), "edit.join", || Value::Null),
        (KeyboardShortcut::new(cmd, Key::A), "edit.select_all", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Escape), "edit.select_none", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Delete), "edit.delete", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Backspace), "edit.delete", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F), "view.frame", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Q), "ui.tool", || json!({ "tool": "select" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::W), "ui.tool", || json!({ "tool": "move" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::E), "ui.tool", || json!({ "tool": "rotate" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::T), "ui.tool", || json!({ "tool": "scale" })),
        // Blender's modal transforms.
        (KeyboardShortcut::new(Modifiers::NONE, Key::G), "view.modal", || json!({ "kind": "move" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::R), "view.modal", || json!({ "kind": "rotate" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::S), "view.modal", || json!({ "kind": "scale" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num1), "ui.mode", || json!({ "mode": "vert" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num2), "ui.mode", || json!({ "mode": "edge" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num3), "ui.mode", || json!({ "mode": "face" })),
        (KeyboardShortcut::new(Modifiers::NONE, Key::Num5), "view.ortho", || Value::Null),
        (KeyboardShortcut::new(Modifiers::ALT, Key::Z), "view.xray", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::O), "ui.transform", || json!({ "proportional": "toggle" })),
        (KeyboardShortcut::new(cmd, Key::K), "ui.palette", || Value::Null),
        (KeyboardShortcut::new(Modifiers::NONE, Key::F3), "ui.palette", || Value::Null),
        // Booleans on the selection (Blender's Bool Tool keys; the numpad
        // keys arrive as the same keys). They take Cmd+Minus and Cmd+Plus
        // from egui's interface zoom.
        (crate::menus::ASTERISK, "selection.boolean", || json!({ "operation": "intersect" })),
        (KeyboardShortcut::new(cmd, Key::Minus), "selection.boolean", || json!({ "operation": "difference" })),
        (KeyboardShortcut::new(cmd, Key::Plus), "selection.boolean", || json!({ "operation": "union" })),
        (KeyboardShortcut::new(cmd, Key::Equals), "selection.boolean", || json!({ "operation": "union" })),
    ];
    // Edit Mode keys come first (E extrudes there, as in Blender).
    if app.ui.mode.is_edit() {
        let edit: &[(KeyboardShortcut, &str)] = &[
            (KeyboardShortcut::new(Modifiers::NONE, Key::E), "edit.extrude"),
            (KeyboardShortcut::new(Modifiers::NONE, Key::I), "edit.inset"),
            (KeyboardShortcut::new(cmd, Key::B), "edit.bevel"),
            (KeyboardShortcut::new(cmd, Key::R), "edit.loop_cut"),
            (KeyboardShortcut::new(Modifiers::ALT, Key::S), "edit.shrink_fatten"),
            (KeyboardShortcut::new(Modifiers::NONE, Key::V), "edit.split"),
            (KeyboardShortcut::new(Modifiers::NONE, Key::M), "edit.merge"),
            (KeyboardShortcut::new(cmd, Key::T), "edit.triangulate"),
            (KeyboardShortcut::new(Modifiers::NONE, Key::Y), "edit.split"),
        ];
        for (shortcut, op) in edit {
            if ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                let _ = app.run("edit.element_op", json!({ "op": op }));
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::NONE, Key::K))) {
            let _ = app.run("ui.tool", json!({ "tool": "knife" }));
        }
    }
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
        let items: Vec<Item> = sidebar_groups(true)
            .into_iter()
            .chain(sidebar_groups(false))
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
