//! The Outliner, laid out like Blender's: a search field, then the object
//! tree with disclosure arrows, type icons, rounded rows, the selection
//! highlighted (the active object brighter) and an eye per row that hides
//! the object in the viewport. Double-click renames in place; right-click
//! opens the object's actions.

use std::collections::BTreeSet;

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use p3d_engine::OpCall;
use p3d_scene::ObjectKey;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::viewport::ViewportEvent;

const ROW: f32 = 28.0;
const INDENT: f32 = 16.0;

/// One visible row.
struct Row {
    key: ObjectKey,
    depth: usize,
    has_children: bool,
}

/// Rows in hierarchy order; children under expanded parents only. With a
/// filter, matching objects in a flat list.
///
/// Complexity: O(objects).
fn rows(app: &PixelApp, collapsed: &BTreeSet<ObjectKey>, filter: &str) -> Vec<Row> {
    let doc = app.doc();
    let mut out = Vec::new();
    let needle = filter.trim().to_lowercase();
    let mut stack: Vec<(ObjectKey, usize)> = doc.roots().collect::<Vec<_>>().into_iter().rev().map(|k| (k, 0)).collect();
    while let Some((k, depth)) = stack.pop() {
        let Some(o) = doc.object(k) else { continue };
        if needle.is_empty() {
            out.push(Row { key: k, depth, has_children: !o.children().is_empty() });
        } else if o.name().to_lowercase().contains(&needle) {
            out.push(Row { key: k, depth: 0, has_children: false });
        }
        if !needle.is_empty() || !collapsed.contains(&k) {
            for &c in o.children().iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    out
}

/// What a row asked for this frame.
enum RowAction {
    Select { key: ObjectKey, add: bool },
    Toggle(ObjectKey),
    Hide(ObjectKey, bool),
    Rename(ObjectKey),
    Run(&'static str, ObjectKey),
}

/// The Outliner body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let state = ui.id().with("outliner");
    let (mut collapsed, mut filter, mut renaming): (BTreeSet<ObjectKey>, String, Option<(ObjectKey, String)>) =
        ui.data_mut(|d| d.get_temp(state).unwrap_or_default());

    // Search.
    crate::widgets::search_field(ui, &mut filter, t("Search objects"));
    ui.add_space(6.0);

    let list = rows(app, &collapsed, &filter);
    if list.is_empty() {
        ui.add_space(8.0);
        let msg = if filter.trim().is_empty() {
            t("The scene is empty. Add a shape with the tools on the left.")
        } else {
            t("No object matches the search.")
        };
        ui.label(egui::RichText::new(msg).color(tk.text_dim));
        ui.data_mut(|d| d.insert_temp(state, (collapsed, filter, renaming)));
        return;
    }
    let active = app.active_object();
    let mut action: Option<RowAction> = None;
    let mut commit_rename: Option<(ObjectKey, String)> = None;
    egui::ScrollArea::vertical().id_salt("outliner-rows").auto_shrink([false, false]).show_rows(ui, ROW, list.len(), |ui, range| {
        for i in range {
            let row = &list[i];
            let doc = app.doc();
            let Some(obj) = doc.object(row.key) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let selected = app.selection.contains(&row.key);
            let hidden = app.hidden.contains(&row.key);
            // Rounded rows: a light accent tint for the selection, stronger
            // for the active object, an eased tint on hover.
            let pill = rect.shrink2(vec2(2.0, 1.0));
            let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
            if selected {
                let is_active = active == Some(row.key);
                ui.painter().rect_filled(pill, 5.0, tk.accent.gamma_multiply(if is_active { 0.2 } else { 0.1 } + 0.05 * hot));
            } else if hot > 0.0 {
                ui.painter().rect_filled(pill, 5.0, tk.control.gamma_multiply(0.8 * hot));
            }
            let x0 = rect.left() + 4.0 + row.depth as f32 * INDENT;
            // Tree lines toward the parent.
            if row.depth > 0 {
                let lx = x0 - INDENT / 2.0;
                ui.painter().vline(lx, rect.top()..=rect.center().y, Stroke::new(1.0, tk.border));
                ui.painter().hline(lx..=x0 + 2.0, rect.center().y, Stroke::new(1.0, tk.border));
            }
            if row.has_children {
                let cr = Rect::from_min_size(pos2(x0, rect.center().y - 7.0), vec2(14.0, 14.0));
                let c = ui.interact(cr, ui.id().with(("expand", row.key)), Sense::click());
                icons::paint(ui.painter(), cr, if collapsed.contains(&row.key) { Icon::ChevronRight } else { Icon::ChevronDown }, false);
                if c.clicked() {
                    action = Some(RowAction::Toggle(row.key));
                }
            }
            let ir = Rect::from_min_size(pos2(x0 + 16.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            icons::paint(ui.painter(), ir, if obj.mesh_key().is_some() { Icon::Mesh } else { Icon::Empty }, hidden);
            let name_x = ir.right() + 6.0;
            // Eye toggle on the right.
            let eye = Rect::from_min_size(pos2(rect.right() - 24.0, rect.center().y - 9.0), vec2(18.0, 18.0));
            let er = ui.interact(eye, ui.id().with(("eye", row.key)), Sense::click());
            icons::paint(ui.painter(), eye.shrink(1.0), if hidden { Icon::EyeOff } else { Icon::Eye }, !er.hovered() && hidden);
            icons::describe_control(ui, &er, if hidden { t("Show in viewport") } else { t("Hide in viewport") }, Some(!hidden));
            if er.on_hover_text(if hidden { t("Show in viewport") } else { t("Hide in viewport") }).clicked() {
                action = Some(RowAction::Hide(row.key, !hidden));
            }
            let text_right = eye.left() - 6.0;
            match &mut renaming {
                Some((k, text)) if *k == row.key => {
                    let er = Rect::from_min_max(pos2(name_x, rect.top() + 1.0), pos2(text_right, rect.bottom() - 1.0));
                    let te = ui.put(er, egui::TextEdit::singleline(text).margin(egui::Margin::symmetric(2, 1)));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    if te.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            commit_rename = Some((row.key, String::new()));
                        } else {
                            commit_rename = Some((row.key, text.clone()));
                        }
                    }
                }
                _ => {
                    let color = if hidden { tk.text_faint } else { tk.text };
                    let mut job = egui::text::LayoutJob::simple_singleline(obj.name().to_owned(), theme::body(), color);
                    job.wrap = egui::text::TextWrapping {
                        max_width: (text_right - name_x - 40.0).max(20.0),
                        max_rows: 1,
                        break_anywhere: true,
                        overflow_character: Some('\u{2026}'),
                    };
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, color);
                    if let Some(m) = doc.object_mesh(row.key) {
                        ui.painter().text(
                            pos2(text_right, rect.center().y),
                            Align2::RIGHT_CENTER,
                            crate::chrome::thousands(m.triangle_count()),
                            theme::small(),
                            tk.text_faint,
                        );
                    }
                }
            }
            crate::control::record_widget(ui, &resp, obj.name(), "treeitem", Some(selected));
            let key = row.key;
            resp.context_menu(|ui| {
                for (label, what) in [
                    (t("Rename"), "rename"),
                    (t("Duplicate"), "scene.duplicate"),
                    (t("Frame in view"), "frame"),
                    (if hidden { t("Show in viewport") } else { t("Hide in viewport") }, "hide"),
                    (t("Delete"), "scene.delete"),
                ] {
                    if ui.button(label).clicked() {
                        action = Some(match what {
                            "rename" => RowAction::Rename(key),
                            "hide" => RowAction::Hide(key, !hidden),
                            other => RowAction::Run(other, key),
                        });
                        ui.close();
                    }
                }
            });
            let resp = resp.on_hover_text(format!("{} ({})", doc.path(key).unwrap_or_default(), obj.public_id()));
            if resp.double_clicked() {
                action = Some(RowAction::Rename(key));
            } else if resp.clicked() {
                let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                action = Some(RowAction::Select { key, add });
            }
        }
    });
    if let Some((k, name)) = commit_rename {
        renaming = None;
        let current = app.doc().object(k).map(|o| (o.name().to_owned(), o.public_id()));
        if let Some((old, id)) = current
            && !name.trim().is_empty()
            && name.trim() != old
        {
            let _ = app.execute(&[OpCall::new("scene.rename", json!({ "target": id, "name": name.trim() }))]);
        }
    }
    match action {
        Some(RowAction::Select { key, add }) => app.handle_viewport(vec![ViewportEvent::Picked { object: Some(key), add }]),
        Some(RowAction::Toggle(k)) => {
            if !collapsed.remove(&k) {
                collapsed.insert(k);
            }
        }
        Some(RowAction::Hide(k, on)) => app.set_hidden(k, on),
        Some(RowAction::Rename(k)) => renaming = app.doc().object(k).map(|o| (k, o.name().to_owned())),
        Some(RowAction::Run(cmd, k)) => {
            app.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
            match cmd {
                "frame" => {
                    let _ = app.run("view.frame", Value::Null);
                }
                "scene.duplicate" => {
                    let _ = app.run("edit.duplicate", Value::Null);
                }
                _ => {
                    let _ = app.run("edit.delete", Value::Null);
                }
            }
        }
        None => {}
    }
    ui.data_mut(|d| d.insert_temp(state, (collapsed, filter, renaming)));
}
