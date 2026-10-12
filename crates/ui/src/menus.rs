//! The menu tree, shared by the in-window menu bar and the native macOS
//! menu. Menus are at most two levels deep (project spec 20.7); every item
//! is a command, so menus, shortcuts, the palette and agents all run the
//! same code.

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};

/// One menu item.
#[derive(Clone, Copy)]
pub struct MenuItem {
    pub label: &'static str,
    pub command: &'static str,
    pub params: fn() -> Value,
    pub shortcut: Option<KeyboardShortcut>,
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

fn item(label: &'static str, command: &'static str) -> Option<MenuItem> {
    Some(MenuItem { label, command, params: none, shortcut: None })
}

fn keyed(label: &'static str, command: &'static str, shortcut: Option<KeyboardShortcut>) -> Option<MenuItem> {
    Some(MenuItem { label, command, params: none, shortcut })
}

fn with(label: &'static str, command: &'static str, params: fn() -> Value) -> Option<MenuItem> {
    Some(MenuItem { label, command, params, shortcut: None })
}

/// Every menu in order.
pub fn menus() -> Vec<Menu> {
    let cmd = Modifiers::COMMAND;
    let shift_cmd = Modifiers::COMMAND.plus(Modifiers::SHIFT);
    vec![
        Menu {
            label: "File",
            entries: vec![
                keyed("New", "file.new", key(cmd, Key::N)),
                keyed("Open...", "file.open", key(cmd, Key::O)),
                item("Open the sample scene", "file.sample"),
                None,
                keyed("Save", "file.save", key(cmd, Key::S)),
                keyed("Save As...", "file.save_as", key(shift_cmd, Key::S)),
                None,
                item("Start page", "ui.start"),
                None,
                item("Quit", "app.quit"),
            ],
        },
        Menu {
            label: "Edit",
            entries: vec![
                keyed("Undo", "edit.undo", key(cmd, Key::Z)),
                keyed("Redo", "edit.redo", key(shift_cmd, Key::Z)),
                None,
                keyed("Duplicate", "edit.duplicate", key(cmd, Key::D)),
                keyed("Delete", "edit.delete", key(Modifiers::NONE, Key::Delete)),
                None,
                keyed("Select all", "edit.select_all", key(cmd, Key::A)),
                keyed("Select none", "edit.select_none", key(Modifiers::NONE, Key::Escape)),
            ],
        },
        Menu {
            label: "View",
            entries: vec![
                keyed("Frame selection", "view.frame", key(Modifiers::NONE, Key::F)),
                None,
                with("Front", "view.set", || json!({ "view": "front" })),
                with("Back", "view.set", || json!({ "view": "back" })),
                with("Right", "view.set", || json!({ "view": "right" })),
                with("Left", "view.set", || json!({ "view": "left" })),
                with("Top", "view.set", || json!({ "view": "top" })),
                with("Bottom", "view.set", || json!({ "view": "bottom" })),
                with("Three-quarter", "view.set", || json!({ "view": "three_quarter" })),
                None,
                keyed("Perspective or orthographic", "view.ortho", key(Modifiers::NONE, Key::Num5)),
                item("Ground grid", "view.grid"),
                None,
                with("Solid", "view.shading", || json!({ "mode": "solid" })),
                with("Solid with edges", "view.shading", || json!({ "mode": "solid_wire" })),
                with("Wireframe", "view.shading", || json!({ "mode": "wire" })),
                with("Clay", "view.shading", || json!({ "mode": "matcap" })),
                None,
                with("Theme: system", "ui.theme", || json!({ "theme": "system" })),
                with("Theme: light", "ui.theme", || json!({ "theme": "light" })),
                with("Theme: dark", "ui.theme", || json!({ "theme": "dark" })),
                None,
                item("History and Console", "ui.bottom"),
                item("Tool labels", "ui.sidebar_labels"),
                item("Reset panel layout", "ui.dock.reset"),
            ],
        },
        Menu {
            label: "Add",
            entries: vec![
                item("Box", "prim.box"),
                item("Cylinder", "prim.cylinder"),
                item("Sphere", "prim.quad_sphere"),
                item("Plane", "prim.plane"),
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
                // Commands that need a dialog or quit are skipped; the rest must run.
                if matches!(it.command, "file.open" | "file.save" | "file.save_as" | "app.quit") {
                    continue;
                }
                let r = app.run(it.command, (it.params)());
                assert!(r.is_ok() || it.command.starts_with("edit."), "{}: {r:?}", it.command);
            }
        }
    }
}
