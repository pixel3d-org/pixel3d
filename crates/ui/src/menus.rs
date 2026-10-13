//! The menu tree, shared by the in-window menu bar and the native macOS
//! menu. Menus are at most two levels deep (project spec 20.7); every item
//! is a command, so menus, shortcuts, the palette and agents all run the
//! same code.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::PixelApp;
use crate::icons::Icon;

/// Whether an item can run now, or why not.
pub type Ready = fn(&PixelApp) -> Result<(), String>;

/// One menu item. An item without a command is a section header.
#[derive(Clone, Copy)]
pub struct MenuItem {
    pub label: &'static str,
    pub command: &'static str,
    pub params: fn() -> Value,
    pub shortcut: Option<KeyboardShortcut>,
    pub icon: Option<Icon>,
    /// Whether the item shows a check mark (toggles and choices).
    pub checked: Option<fn(&PixelApp) -> bool>,
    /// Whether the item can run now, or why not (shown on hover).
    pub enabled: Option<Ready>,
}

impl MenuItem {
    /// A section header: a small title over the items that follow.
    pub fn is_header(&self) -> bool {
        self.command.is_empty()
    }
}

/// A top-level menu; `None` entries are separators.
pub struct Menu {
    pub label: &'static str,
    pub entries: Vec<Option<MenuItem>>,
}

fn none() -> Value {
    Value::Null
}

const fn key(modifiers: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(modifiers, key))
}

fn entry(label: &'static str, command: &'static str, params: fn() -> Value, shortcut: Option<KeyboardShortcut>) -> MenuItem {
    MenuItem { label, command, params, shortcut, icon: None, checked: None, enabled: None }
}

/// Cmd+* (Ctrl+* off macOS) as egui sees it: `*` is Shift+8 on most
/// keyboards and egui has no key of its own for it.
pub const ASTERISK: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Num8);

/// A shortcut as menus and the palette write it, with Cmd+Shift+8 as
/// `Ctrl+*` (`Cmd*` on macOS).
pub fn shortcut_text(ctx: &egui::Context, s: &KeyboardShortcut) -> String {
    if *s == ASTERISK {
        let mut text = ctx.format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Num8));
        text.pop();
        text.push('*');
        return text;
    }
    ctx.format_shortcut(s)
}

/// The shapes Add > SDF Shape starts with: a body and a head blended.
fn sdf_start() -> Value {
    json!({ "shapes": [
        { "name": "body", "ellipsoid": { "at": [0, 0.35, 0], "radii": [0.3, 0.35, 0.25] } },
        { "name": "head", "smooth_union": 0.08, "sphere": { "at": [0, 0.82, 0], "radius": 0.18 } }
    ] })
}

/// Whether a boolean can run on the selection, or why not.
fn boolean_ready(app: &PixelApp) -> Result<(), String> {
    app.tools_on_active().map(|_| ())
}

/// Whether a mesh is selected (cleanup works on meshes).
///
/// Complexity: O(selected objects).
fn mesh_selected(app: &PixelApp) -> Result<(), String> {
    let doc = app.doc();
    if app.selection.iter().any(|k| doc.object(*k).is_some_and(|o| o.mesh_key().is_some())) {
        Ok(())
    } else {
        Err(crate::i18n::t("Select a mesh first.").to_owned())
    }
}

fn when(state: Ready, it: Option<MenuItem>) -> Option<MenuItem> {
    it.map(|it| MenuItem { enabled: Some(state), ..it })
}

fn item(label: &'static str, command: &'static str) -> Option<MenuItem> {
    Some(entry(label, command, none, None))
}

fn keyed(label: &'static str, command: &'static str, shortcut: Option<KeyboardShortcut>) -> Option<MenuItem> {
    Some(entry(label, command, none, shortcut))
}

fn with(label: &'static str, command: &'static str, params: fn() -> Value) -> Option<MenuItem> {
    Some(entry(label, command, params, None))
}

fn header(label: &'static str) -> Option<MenuItem> {
    Some(entry(label, "", none, None))
}

fn icon(icon: Icon, it: Option<MenuItem>) -> Option<MenuItem> {
    it.map(|it| MenuItem { icon: Some(icon), ..it })
}

fn checked(state: fn(&PixelApp) -> bool, it: Option<MenuItem>) -> Option<MenuItem> {
    it.map(|it| MenuItem { checked: Some(state), ..it })
}

/// Every menu in order.
pub fn menus() -> Vec<Menu> {
    let cmd = Modifiers::COMMAND;
    let shift_cmd = Modifiers::COMMAND.plus(Modifiers::SHIFT);
    vec![
        Menu {
            label: "File",
            entries: vec![
                icon(Icon::New, keyed("New", "file.new", key(cmd, Key::N))),
                icon(Icon::Open, keyed("Open...", "file.open", key(cmd, Key::O))),
                icon(Icon::Box, item("Open the sample scene", "file.sample")),
                None,
                icon(Icon::Import, item("Import...", "file.import")),
                None,
                icon(Icon::Save, keyed("Save", "file.save", key(cmd, Key::S))),
                keyed("Save As...", "file.save_as", key(shift_cmd, Key::S)),
                None,
                icon(Icon::Menu, item("Start page", "ui.start")),
                checked(|a| a.remote.addr().is_some(), icon(Icon::Command, item("Allow agents to connect", "app.control"))),
                None,
                icon(Icon::Close, item("Quit", "app.quit")),
            ],
        },
        Menu {
            label: "Edit",
            entries: vec![
                icon(Icon::Undo, keyed("Undo", "edit.undo", key(cmd, Key::Z))),
                icon(Icon::Redo, keyed("Redo", "edit.redo", key(shift_cmd, Key::Z))),
                None,
                icon(Icon::Copy, keyed("Duplicate", "edit.duplicate", key(cmd, Key::D))),
                icon(Icon::Trash, keyed("Delete", "edit.delete", key(Modifiers::NONE, Key::Delete))),
                None,
                icon(Icon::Select, keyed("Select all", "edit.select_all", key(cmd, Key::A))),
                keyed("Select none", "edit.select_none", key(Modifiers::NONE, Key::Escape)),
                None,
                icon(Icon::Search, keyed("Command palette...", "ui.palette", key(cmd, Key::K))),
            ],
        },
        Menu {
            label: "View",
            entries: vec![
                icon(Icon::Frame, keyed("Frame selection", "view.frame", key(Modifiers::NONE, Key::F))),
                None,
                header("Viewpoint"),
                with("Front", "view.set", || json!({ "view": "front" })),
                with("Back", "view.set", || json!({ "view": "back" })),
                with("Right", "view.set", || json!({ "view": "right" })),
                with("Left", "view.set", || json!({ "view": "left" })),
                with("Top", "view.set", || json!({ "view": "top" })),
                with("Bottom", "view.set", || json!({ "view": "bottom" })),
                with("Three-quarter", "view.set", || json!({ "view": "three_quarter" })),
                None,
                checked(|a| a.ui.view.ortho, icon(Icon::Orthographic, keyed("Orthographic", "view.ortho", key(Modifiers::NONE, Key::Num5)))),
                checked(|a| a.ui.view.grid, icon(Icon::Grid, item("Ground grid", "view.grid"))),
                checked(|a| a.ui.view.xray, icon(Icon::XRay, keyed("X-ray", "view.xray", key(Modifiers::ALT, Key::Z)))),
                checked(|a| a.ui.view.stats, icon(Icon::Info, item("Statistics", "view.stats"))),
                None,
                header("3D Cursor"),
                icon(Icon::Cursor, with("Cursor to World Origin", "view.cursor", || json!({ "to": "origin" }))),
                with("Cursor to Selected", "view.cursor", || json!({ "to": "selected" })),
                with("Cursor to Active", "view.cursor", || json!({ "to": "active" })),
                None,
                header("Shading"),
                checked(|a| a.ui.view.shading == "solid", icon(Icon::ShadeSolid, with("Solid", "view.shading", || json!({ "mode": "solid" })))),
                checked(
                    |a| a.ui.view.shading == "solid_wire",
                    icon(Icon::ShadeSolidWire, with("Solid with edges", "view.shading", || json!({ "mode": "solid_wire" }))),
                ),
                checked(|a| a.ui.view.shading == "wire", icon(Icon::ShadeWire, with("Wireframe", "view.shading", || json!({ "mode": "wire" })))),
                checked(|a| a.ui.view.shading == "matcap", icon(Icon::ShadeMatcap, with("Clay", "view.shading", || json!({ "mode": "matcap" })))),
                None,
                header("Theme"),
                checked(|a| a.ui.theme == crate::theme::ThemePref::System, with("System", "ui.theme", || json!({ "theme": "system" }))),
                checked(|a| a.ui.theme == crate::theme::ThemePref::Light, with("Light", "ui.theme", || json!({ "theme": "light" }))),
                checked(|a| a.ui.theme == crate::theme::ThemePref::Dark, with("Dark", "ui.theme", || json!({ "theme": "dark" }))),
                None,
                checked(PixelApp::bottom_panel_visible, icon(Icon::History, item("History and Console", "ui.bottom"))),
                checked(|a| a.ui.rail_labels, item("Tool labels", "ui.rail_labels")),
                item("Reset panel layout", "ui.dock.reset"),
            ],
        },
        Menu {
            label: "Add",
            entries: vec![
                header("Mesh"),
                icon(Icon::Box, item("Box", "prim.box")),
                icon(Icon::Plane, item("Plane", "prim.plane")),
                icon(Icon::Cylinder, item("Cylinder", "prim.cylinder")),
                icon(Icon::Cone, item("Cone", "prim.cone")),
                icon(Icon::Tube, item("Tube", "prim.tube")),
                icon(Icon::Sphere, item("Quad sphere", "prim.quad_sphere")),
                icon(Icon::Sphere, item("UV sphere", "prim.uv_sphere")),
                icon(Icon::IcoSphere, item("Ico sphere", "prim.ico_sphere")),
                icon(Icon::Torus, item("Torus", "prim.torus")),
                icon(Icon::Capsule, item("Capsule", "prim.capsule")),
                icon(Icon::Wedge, item("Wedge", "prim.wedge")),
                None,
                icon(Icon::Empty, item("Empty", "scene.create_empty")),
                None,
                header("Curve"),
                icon(Icon::Curve, with("Polyline", "curve.polyline", || json!({ "points": [[0, 0, 0], [1, 0, 0], [1, 0, -1]] }))),
                icon(Icon::Curve, with("Bezier", "curve.bezier", || json!({ "points": [[-0.5, 0, 0], [0, 0.5, 0], [0.5, 0, 0]] }))),
                icon(Icon::Curve, item("Circle", "curve.circle")),
                icon(Icon::Curve, item("Rectangle", "curve.rectangle")),
                None,
                header("Generate"),
                icon(
                    Icon::Extrude,
                    with(
                        "Extruded shape",
                        "gen.extrude_shape",
                        || json!({ "outline": [[-0.5, -0.2], [0.1, -0.2], [0.1, -0.4], [0.5, 0.0], [0.1, 0.4], [0.1, 0.2], [-0.5, 0.2]], "depth": "8cm" }),
                    ),
                ),
                icon(
                    Icon::Sweep,
                    with(
                        "Sweep",
                        "gen.sweep",
                        || json!({ "path": [[-0.5, 0, 0], [0, 0, -0.4], [0.5, 0, 0]], "smooth_path": true, "profile": { "rectangle": ["8cm", "4cm"] } }),
                    ),
                ),
                icon(
                    Icon::Pipe,
                    with("Pipe", "gen.pipe", || json!({ "path": [[0, 0, 0], [0, 0.5, 0], [0.5, 0.5, 0], [0.5, 0.5, -0.5]], "radius": "3cm" })),
                ),
                icon(
                    Icon::Loft,
                    with(
                        "Loft",
                        "gen.loft",
                        || json!({ "points": 16, "sections": [[[-0.2, 0, -0.2], [0.2, 0, -0.2], [0.2, 0, 0.2], [-0.2, 0, 0.2]], [[0.15, 0.5, 0], [0, 0.5, -0.15], [-0.15, 0.5, 0], [0, 0.5, 0.15]]] }),
                    ),
                ),
                // Blended shapes to sculpt in the Shapes tab.
                icon(Icon::Shapes, with("SDF shape", crate::sdf::OP, sdf_start)),
                None,
                header("Generate (lathe)"),
                icon(Icon::Tube, with("Barrel", "gen.lathe", || json!({ "profile": "barrel" }))),
                icon(Icon::Tube, with("Vase", "gen.lathe", || json!({ "profile": "vase" }))),
                icon(Icon::Tube, with("Bottle", "gen.lathe", || json!({ "profile": "bottle" }))),
                icon(Icon::Tube, with("Bowl", "gen.lathe", || json!({ "profile": "bowl" }))),
                icon(Icon::Tube, with("Column", "gen.lathe", || json!({ "profile": "column" }))),
            ],
        },
        Menu {
            label: "Object",
            entries: vec![
                icon(Icon::Apply, with("Apply transform", "selection.run", || json!({ "op": "scene.apply_transform" }))),
                icon(Icon::Copy, keyed("Duplicate", "edit.duplicate", key(Modifiers::COMMAND, Key::D))),
                None,
                header("Organize"),
                icon(Icon::Collection, item("New collection from selection", "edit.new_collection")),
                icon(Icon::LodGroup, item("LOD group from selection", "edit.lod_group")),
                icon(Icon::Merge, keyed("Join", "edit.join", key(Modifiers::COMMAND, Key::J))),
                with("Move out of parent", "selection.run", || json!({ "op": "scene.unparent" })),
                None,
                header("Generate from selection"),
                icon(Icon::Array, with("Array", "selection.action", || json!({ "op": "gen.array" }))),
                icon(Icon::Symmetrize, with("Mirror", "selection.action", || json!({ "op": "gen.mirror" }))),
                icon(Icon::Scatter, with("Scatter onto active", "selection.action", || json!({ "op": "gen.scatter" }))),
                icon(Icon::Sweep, with("Sweep along curve", "selection.action", || json!({ "op": "gen.sweep" }))),
                icon(Icon::Pipe, with("Pipe along curve", "selection.action", || json!({ "op": "gen.pipe" }))),
                icon(Icon::Loft, with("Loft through curves", "selection.action", || json!({ "op": "gen.loft" }))),
                None,
                header("Boolean"),
                when(
                    boolean_ready,
                    icon(
                        Icon::BoolDifference,
                        Some(entry("Difference", "selection.boolean", || json!({ "operation": "difference" }), key(cmd, Key::Minus))),
                    ),
                ),
                when(
                    boolean_ready,
                    icon(Icon::BoolUnion, Some(entry("Union", "selection.boolean", || json!({ "operation": "union" }), key(cmd, Key::Plus)))),
                ),
                when(
                    boolean_ready,
                    icon(Icon::BoolIntersect, Some(entry("Intersect", "selection.boolean", || json!({ "operation": "intersect" }), Some(ASTERISK)))),
                ),
                None,
                icon(Icon::Trash, keyed("Delete", "edit.delete", key(Modifiers::NONE, Key::Delete))),
            ],
        },
        Menu {
            label: "Mesh",
            entries: vec![
                icon(Icon::Fill, with("Fill holes", "selection.run", || json!({ "op": "edit.fill" }))),
                icon(Icon::Normals, with("Recalculate normals", "selection.run", || json!({ "op": "edit.recalc_normals" }))),
                None,
                header("Clean Up"),
                when(mesh_selected, icon(Icon::CleanUp, with("Auto Clean", "selection.run", || json!({ "op": "cleanup.auto" })))),
                when(mesh_selected, icon(Icon::Report, with("Report", "selection.run", || json!({ "op": "cleanup.report" })))),
                None,
                icon(Icon::Apply, with("Apply transform", "selection.run", || json!({ "op": "scene.apply_transform" }))),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_command_is_known() {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        for menu in menus() {
            for it in menu.entries.into_iter().flatten() {
                // Headers, commands that need a dialog and quit are skipped;
                // the rest must run.
                if it.is_header() || matches!(it.command, "file.open" | "file.import" | "file.save" | "file.save_as" | "app.quit") {
                    continue;
                }
                // Ops may refuse the scene (nothing selected, no holes); the
                // command itself must exist.
                if let Err(e) = app.run(it.command, (it.params)()) {
                    assert!(!e.starts_with(crate::i18n::t("unknown command")), "{}: {e}", it.command);
                }
                if let Some(state) = it.checked {
                    let _ = state(&app);
                }
            }
        }
    }

    #[test]
    fn boolean_items_say_why_they_cannot_run() {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let object = menus().into_iter().find(|m| m.label == "Object").unwrap();
        let booleans: Vec<MenuItem> = object.entries.into_iter().flatten().filter(|it| it.command == "selection.boolean").collect();
        assert_eq!(booleans.iter().map(|it| it.label).collect::<Vec<_>>(), ["Difference", "Union", "Intersect"]);
        let state = |app: &crate::PixelApp| booleans.iter().map(|it| (it.enabled.unwrap())(app)).collect::<Vec<_>>();
        app.run("prim.box", serde_json::json!({ "name": "crate" })).unwrap();
        assert!(state(&app).iter().all(|s| s.as_ref().is_err_and(|e| e.contains("two or more meshes"))));
        app.run("prim.box", serde_json::json!({ "name": "lid", "position": [0, 0.5, 0] })).unwrap();
        app.run("edit.select_all", Value::Null).unwrap();
        assert!(state(&app).iter().all(Result::is_ok));
        app.run("ui.mode", serde_json::json!({ "mode": "face" })).unwrap();
        assert!(state(&app).iter().all(|s| s.as_ref().is_err_and(|e| e.contains("Object Mode"))));
        // Cmd+* is written as such.
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let minus = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);
            assert!(shortcut_text(ui.ctx(), &ASTERISK).ends_with('*'));
            assert_eq!(shortcut_text(ui.ctx(), &minus), ui.ctx().format_shortcut(&minus));
        });
        out.textures_delta.clear();
    }

    #[test]
    fn clean_up_items_clean_and_report() {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let mesh = menus().into_iter().find(|m| m.label == "Mesh").unwrap();
        let items: Vec<MenuItem> = mesh.entries.into_iter().flatten().filter(|it| matches!(it.label, "Auto Clean" | "Report")).collect();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|it| (it.enabled.unwrap())(&app).is_err()), "nothing selected");
        app.run("prim.box", serde_json::json!({ "name": "crate" })).unwrap();
        assert!(items.iter().all(|it| (it.enabled.unwrap())(&app).is_ok()));
        let steps = app.session.history().len();
        for it in items.iter().rev() {
            app.run(it.command, (it.params)()).unwrap();
        }
        // The report adds no undo step and reads as lines; the clean box stays clean.
        assert_eq!(app.session.history().len(), steps + 1);
        assert!(app.result.as_ref().is_some_and(|r| !r.read_only));
        app.run(items[1].command, (items[1].params)()).unwrap();
        let r = app.result.clone().unwrap();
        let (_, lines) = crate::results::readable(&r.entries[0].summary, app.session.registry());
        assert!(r.read_only && lines.iter().any(|l| l == "Clean up would change nothing"), "{lines:?}");
    }

    #[test]
    fn every_add_item_creates_an_object() {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let add = menus().into_iter().find(|m| m.label == "Add").unwrap();
        for it in add.entries.into_iter().flatten().filter(|it| !it.is_header()) {
            let before = app.doc().object_count();
            app.run(it.command, (it.params)()).unwrap_or_else(|e| panic!("{}: {e}", it.label));
            assert!(app.doc().object_count() > before, "{}", it.label);
        }
    }
}
