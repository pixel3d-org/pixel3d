//! macOS: the menu tree as the native system menu bar (cadcraft's
//! approach). Items run through the same command path as the in-window
//! menus. Shortcuts with the command key become menu key equivalents;
//! single-key shortcuts stay in the app so they never steal typing.

use std::collections::BTreeMap;
use std::str::FromStr;

use muda::accelerator::Accelerator;
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use p3d_ui::PixelApp;
use p3d_ui::i18n::t;
use p3d_ui::menus::{MenuItem as Entry, menus};

pub struct NativeMenu {
    _menu: Menu,
    /// Menu item id to its command.
    items: BTreeMap<String, Entry>,
}

/// The key equivalent of a shortcut that uses the command key.
fn accelerator(s: &egui::KeyboardShortcut) -> Option<Accelerator> {
    if !s.modifiers.command {
        return None;
    }
    let mut text = String::from("CMD+");
    if s.modifiers.shift {
        text.push_str("SHIFT+");
    }
    if s.modifiers.alt {
        text.push_str("ALT+");
    }
    text.push_str(s.logical_key.name());
    Accelerator::from_str(&text).ok()
}

impl NativeMenu {
    /// Builds the menu bar and hands it to the application. Call on the
    /// main thread once the app runs (the first frame).
    pub fn install() -> Self {
        let menu = Menu::new();
        let mut items = BTreeMap::new();
        let app_menu = Submenu::new("pixel3d", true);
        let _ = app_menu.append_items(&[
            &PredefinedMenuItem::about(Some(t("About pixel3d")), None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(Some(t("Services"))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some(t("Hide pixel3d"))),
            &PredefinedMenuItem::hide_others(Some(t("Hide Others"))),
            &PredefinedMenuItem::show_all(Some(t("Show All"))),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(Some(t("Quit pixel3d"))),
        ]);
        let _ = menu.append(&app_menu);
        for (m, top) in menus().into_iter().enumerate() {
            let sub = Submenu::new(t(top.label), true);
            for (i, entry) in top.entries.into_iter().enumerate() {
                match entry {
                    None => {
                        let _ = sub.append(&PredefinedMenuItem::separator());
                    }
                    Some(it) => {
                        // The app menu has Quit already.
                        if it.command == "app.quit" {
                            continue;
                        }
                        let id = format!("p3d-{m}-{i}");
                        let item = MenuItem::with_id(id.clone(), t(it.label), true, it.shortcut.as_ref().and_then(accelerator));
                        let _ = sub.append(&item);
                        items.insert(id, it);
                    }
                }
            }
            let _ = menu.append(&sub);
        }
        menu.init_for_nsapp();
        NativeMenu { _menu: menu, items }
    }

    /// Runs the commands of chosen menu items.
    pub fn poll(&self, app: &mut PixelApp, ctx: &egui::Context) {
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if let Some(it) = self.items.get(ev.id.as_ref()) {
                let _ = app.run(it.command, (it.params)());
                ctx.request_repaint();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, KeyboardShortcut, Modifiers};

    #[test]
    fn only_command_shortcuts_become_key_equivalents() {
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        assert_eq!(accelerator(&save_as), Accelerator::from_str("CMD+SHIFT+S").ok());
        assert!(accelerator(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)).is_some());
        assert!(accelerator(&KeyboardShortcut::new(Modifiers::NONE, Key::F)).is_none());
        assert!(accelerator(&KeyboardShortcut::new(Modifiers::NONE, Key::Delete)).is_none());
    }
}
