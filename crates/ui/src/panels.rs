//! Dock panel bodies: Tool Sets, Outliner, Properties, History and Console.
//! Panels read the session and act only through [`PixelApp::run`].

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use p3d_scene::ObjectKey;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

const ROW: f32 = 22.0;

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).font(theme::small()).color(Tokens::get().text_faint));
    ui.add_space(2.0);
}

/// One Tool Sets entry: an op run on the selection (or creating an object).
struct ToolEntry {
    icon: Icon,
    label: &'static str,
    op: &'static str,
    /// Runs on the first selected object (`{"target": id}`).
    needs_target: bool,
}

const CREATE: &[ToolEntry] = &[
    ToolEntry { icon: Icon::Box, label: "Box", op: "prim.box", needs_target: false },
    ToolEntry { icon: Icon::Cylinder, label: "Cylinder", op: "prim.cylinder", needs_target: false },
    ToolEntry { icon: Icon::Sphere, label: "Sphere", op: "prim.quad_sphere", needs_target: false },
    ToolEntry { icon: Icon::Plane, label: "Plane", op: "prim.plane", needs_target: false },
];

const MESH: &[ToolEntry] = &[
    ToolEntry { icon: Icon::Mesh, label: "Fill holes", op: "edit.fill", needs_target: true },
    ToolEntry { icon: Icon::Health, label: "Recalculate normals", op: "edit.recalc_normals", needs_target: true },
];

const OBJECT: &[ToolEntry] = &[
    ToolEntry { icon: Icon::Empty, label: "Empty", op: "scene.create_empty", needs_target: false },
    ToolEntry { icon: Icon::Frame, label: "Apply transform", op: "scene.apply_transform", needs_target: true },
];

/// Tool Sets: tools grouped by what they do.
pub fn tool_sets(app: &mut PixelApp, ui: &mut egui::Ui) {
    let target = app.selected_targets().into_iter().next();
    let mut run: Option<(&'static str, Value)> = None;
    egui::ScrollArea::vertical().id_salt("toolsets").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        for (title, entries) in [("Create", CREATE), ("Object", OBJECT), ("Mesh", MESH)] {
            section(ui, t(title));
            for e in entries {
                let enabled = !e.needs_target || target.is_some();
                let resp = ui
                    .add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            let b = icons::button(ui, e.icon, 24.0, t(e.label), false);
                            let l = ui.add(egui::Label::new(t(e.label)).sense(Sense::click()));
                            b.clicked() || l.clicked()
                        })
                        .inner
                    })
                    .inner;
                if resp {
                    let params = match (&target, e.needs_target) {
                        (Some(id), true) => json!({ "target": id }),
                        _ => Value::Null,
                    };
                    run = Some((e.op, params));
                }
            }
        }
        if target.is_none() {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(t("Select an object to use the object and mesh tools.")).font(theme::small()).color(Tokens::get().text_dim));
        }
    });
    if let Some((op, params)) = run {
        let _ = app.run(op, params);
    }
}

/// One visible Outliner row.
struct Row {
    key: ObjectKey,
    depth: usize,
    has_children: bool,
}

/// Objects in hierarchy order, children under expanded parents only.
///
/// Complexity: O(visible objects).
fn rows(app: &PixelApp, collapsed: &std::collections::BTreeSet<ObjectKey>) -> Vec<Row> {
    let doc = app.doc();
    let mut out = Vec::new();
    let mut stack: Vec<(ObjectKey, usize)> = doc.roots().collect::<Vec<_>>().into_iter().rev().map(|k| (k, 0)).collect();
    while let Some((k, depth)) = stack.pop() {
        let Some(o) = doc.object(k) else { continue };
        out.push(Row { key: k, depth, has_children: !o.children().is_empty() });
        if !collapsed.contains(&k) {
            for &c in o.children().iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    out
}

/// Outliner: the object tree; click selects, Shift or Cmd click toggles.
pub fn outliner(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let id = ui.id().with("outliner-collapsed");
    let mut collapsed: std::collections::BTreeSet<ObjectKey> = ui.data_mut(|d| d.get_temp(id).unwrap_or_default());
    let rows = rows(app, &collapsed);
    if rows.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(t("The scene is empty. Add a shape from Tool Sets.")).color(tk.text_dim));
        return;
    }
    let mut clicked: Option<(ObjectKey, bool)> = None;
    let mut toggle: Option<ObjectKey> = None;
    egui::ScrollArea::vertical().id_salt("outliner").auto_shrink([false, false]).show_rows(ui, ROW, rows.len(), |ui, range| {
        for row in &rows[range] {
            let doc = app.doc();
            let Some(obj) = doc.object(row.key) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let selected = app.selection.contains(&row.key);
            if selected {
                ui.painter().rect_filled(rect, 2.0, tk.select_bg.gamma_multiply(0.45));
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 2.0, tk.control.gamma_multiply(0.4));
            }
            let x = rect.left() + 4.0 + row.depth as f32 * 14.0;
            if row.has_children {
                let cr = Rect::from_min_size(pos2(x, rect.center().y - 7.0), vec2(14.0, 14.0));
                let c = ui.interact(cr, ui.id().with(("expand", row.key)), Sense::click());
                icons::paint(ui.painter(), cr, if collapsed.contains(&row.key) { Icon::ChevronRight } else { Icon::ChevronDown }, false);
                if c.clicked() {
                    toggle = Some(row.key);
                }
            }
            let ir = Rect::from_min_size(pos2(x + 16.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            icons::paint(ui.painter(), ir, if obj.mesh_key().is_some() { Icon::Mesh } else { Icon::Empty }, !obj.visible);
            let color = if obj.visible { tk.text } else { tk.text_faint };
            ui.painter().text(pos2(ir.right() + 6.0, rect.center().y), Align2::LEFT_CENTER, obj.name(), theme::body(), color);
            if let Some(m) = doc.object_mesh(row.key) {
                let tris = crate::chrome::thousands(m.triangle_count());
                ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, tris, theme::small(), tk.text_faint);
            }
            crate::control::record_widget(ui, &resp, obj.name(), "treeitem", Some(selected));
            let resp = resp.on_hover_text(format!("{} ({})", doc.path(row.key).unwrap_or_default(), obj.public_id()));
            if resp.clicked() {
                let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                clicked = Some((row.key, add));
            }
        }
    });
    if let Some(k) = toggle {
        if !collapsed.remove(&k) {
            collapsed.insert(k);
        }
        ui.data_mut(|d| d.insert_temp(id, collapsed));
    }
    if let Some((k, add)) = clicked {
        app.handle_viewport(vec![crate::viewport::ViewportEvent::Picked { object: Some(k), add }]);
    }
}

fn kv(ui: &mut egui::Ui, key: &str, value: impl Into<String>) {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 18.0], egui::Label::new(egui::RichText::new(key).color(tk.text_dim)).truncate());
        ui.label(value.into());
    });
}

fn vec3(v: glam::Vec3) -> String {
    format!("{} {} {}", fmt(v.x), fmt(v.y), fmt(v.z))
}

fn fmt(x: f32) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// Properties of the selection, or of the document when nothing is selected.
pub fn properties(app: &mut PixelApp, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().id_salt("properties").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let doc = std::sync::Arc::clone(app.session.doc());
        let selected: Vec<ObjectKey> = app.selection.iter().copied().filter(|k| doc.object(*k).is_some()).collect();
        match selected.as_slice() {
            [] => {
                section(ui, t("Scene"));
                kv(ui, t("Objects"), doc.object_count().to_string());
                kv(ui, t("Asset class"), doc.settings.asset_class.as_str());
                kv(ui, t("Budget"), format!("{} {}", crate::chrome::thousands(doc.settings.tri_budget() as usize), t("tris")));
                kv(ui, t("Revision"), doc.revision.to_string());
            }
            [k] => {
                let Some(o) = doc.object(*k) else { return };
                section(ui, t("Object"));
                kv(ui, t("Name"), o.name());
                kv(ui, t("Id"), o.public_id());
                kv(ui, t("Path"), doc.path(*k).unwrap_or_default());
                kv(ui, t("Kind"), o.kind().as_str());
                section(ui, t("Transform"));
                let tr = &o.transform;
                kv(ui, t("Position"), format!("{} m", vec3(tr.translation)));
                let (x, y, z) = tr.rotation.to_euler(glam::EulerRot::ZYX);
                kv(ui, t("Rotation"), format!("{} deg", vec3(glam::Vec3::new(z, y, x) * (180.0 / std::f32::consts::PI))));
                kv(ui, t("Scale"), vec3(tr.scale));
                if let Some(m) = doc.object_mesh(*k) {
                    section(ui, t("Mesh"));
                    kv(ui, t("Vertices"), crate::chrome::thousands(m.vert_count()));
                    kv(ui, t("Faces"), crate::chrome::thousands(m.face_count()));
                    kv(ui, t("Triangles"), crate::chrome::thousands(m.triangle_count()));
                    if let Some((lo, hi)) = m.bounds() {
                        kv(ui, t("Size"), format!("{} m", vec3(hi - lo)));
                    }
                    kv(ui, t("Groups"), m.group_count().to_string());
                }
            }
            many => {
                section(ui, t("Selection"));
                kv(ui, t("Objects"), many.len().to_string());
                let tris: usize = many.iter().filter_map(|k| doc.object_mesh(*k)).map(|m| m.triangle_count()).sum();
                kv(ui, t("Triangles"), crate::chrome::thousands(tris));
            }
        }
    });
}

/// History: the undo list, newest last (the current state). Clicking an
/// entry undoes back to the state after it; redo brings later ones back.
pub fn history(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let entries = app.session.history();
    let mut undo_to: Option<usize> = None;
    egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).stick_to_bottom(true).show_rows(
        ui,
        ROW,
        entries.len() + 1,
        |ui, range| {
            for i in range {
                let label = if i == 0 { t("Opened or new").to_owned() } else { entries[i - 1].label.clone() };
                let is_current = i == entries.len();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                if is_current {
                    ui.painter().rect_filled(rect, 2.0, tk.select_bg.gamma_multiply(0.45));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 2.0, tk.control.gamma_multiply(0.4));
                }
                ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, &label, theme::body(), tk.text);
                if is_current {
                    ui.painter().vline(rect.left() + 2.0, rect.y_range().shrink(3.0), Stroke::new(2.0, tk.accent));
                }
                crate::control::record_widget(ui, &resp, &label, "listitem", Some(is_current));
                if resp.clicked() {
                    undo_to = Some(i);
                }
            }
        },
    );
    if let Some(i) = undo_to {
        // Entry i (1-based) is the state after transaction i; undo the ones after it.
        let steps = entries.len().saturating_sub(i);
        for _ in 0..steps {
            if app.run("edit.undo", Value::Null).is_err() {
                break;
            }
        }
    }
}

/// Console: op results and messages, newest last.
pub fn console(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let lines = &app.console;
    egui::ScrollArea::vertical().id_salt("console").auto_shrink([false, false]).stick_to_bottom(true).show_rows(
        ui,
        16.0,
        lines.len(),
        |ui, range| {
            for line in &lines[range] {
                ui.label(egui::RichText::new(line).font(theme::mono()).color(tk.text_dim));
            }
        },
    );
}
