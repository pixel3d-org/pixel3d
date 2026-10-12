//! The menu tree, shared by the in-window menu bar and the native macOS
//! menu. Menus are at most two levels deep (project spec 20.7); every item
//! is a command, so menus, shortcuts, the palette and agents all run the
//! same code.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

use crate::PixelApp;
use crate::icons::Icon;

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
    MenuItem { label, command, params, shortcut, icon: None, checked: None }
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
                icon(Icon::Save, keyed("Save", "file.save", key(cmd, Key::S))),
                keyed("Save As...", "file.save_as", key(shift_cmd, Key::S)),
                None,
                icon(Icon::Menu, item("Start page", "ui.start")),
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
                icon(Icon::Cylinder, item("Cylinder", "prim.cylinder")),
                icon(Icon::Sphere, item("Sphere", "prim.quad_sphere")),
                icon(Icon::Plane, item("Plane", "prim.plane")),
                None,
                icon(Icon::Empty, item("Empty", "scene.create_empty")),
            ],
        },
        Menu {
            label: "Mesh",
            entries: vec![
                icon(Icon::Fill, with("Fill holes", "selection.run", || json!({ "op": "edit.fill" }))),
                icon(Icon::Normals, with("Recalculate normals", "selection.run", || json!({ "op": "edit.recalc_normals" }))),
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
                if it.is_header() || matches!(it.command, "file.open" | "file.save" | "file.save_as" | "app.quit") {
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
}
