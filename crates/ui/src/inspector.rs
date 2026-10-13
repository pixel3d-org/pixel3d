//! The Properties editor, laid out like Blender's: a column of tab icons
//! on the left (Tool, Scene, Object, Mesh) and collapsible panels on the
//! right. Values are edited through ops only: the object transform through
//! `scene.set_transform` (one undo step per drag), the last operation's
//! parameters by re-running it, both with forms generated from the ops'
//! parameter schemas.

use egui::{Rect, RichText, pos2, vec2};
use p3d_engine::OpCall;
use p3d_scene::ObjectKey;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::schema_form;
use crate::theme::{self, Tokens};
use crate::widgets::{info, section as panel};
use crate::{PixelApp, chrome};

/// The Properties tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tab {
    /// The active tool and the last operation.
    Tool,
    Scene,
    #[default]
    Object,
    Mesh,
    /// The shapes of an SDF object (its editor).
    Shapes,
}

impl Tab {
    fn icon(self) -> Icon {
        match self {
            Tab::Tool => Icon::Tool,
            Tab::Scene => Icon::Scene,
            Tab::Object => Icon::ModeObject,
            Tab::Mesh => Icon::Mesh,
            Tab::Shapes => Icon::Shapes,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Tool => t("Tool and last operation"),
            Tab::Scene => t("Scene"),
            Tab::Object => t("Object"),
            Tab::Mesh => t("Mesh data"),
            Tab::Shapes => t("Shapes (SDF)"),
        }
    }
}

const TAB_STRIP: f32 = 36.0;

fn fmt(x: f32) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The Properties editor body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let full = ui.max_rect();
    let strip = Rect::from_min_size(full.min, vec2(TAB_STRIP, full.height()));
    let active = app.active_object();
    let has_mesh = active.is_some_and(|k| app.doc().object_mesh(k).is_some());
    let tabs: Vec<Tab> = [Tab::Tool, Tab::Scene, Tab::Object, Tab::Mesh, Tab::Shapes]
        .into_iter()
        .filter(|t| match t {
            Tab::Object => active.is_some(),
            Tab::Mesh => has_mesh,
            Tab::Shapes => app.sdf.object.is_some() && app.sdf.object == active,
            _ => true,
        })
        .collect();
    let mut tab = app.ui.inspector_tab;
    if !tabs.contains(&tab) {
        tab = if active.is_some() { Tab::Object } else { Tab::Scene };
    }
    let labels: Vec<(Icon, &str)> = tabs.iter().map(|tb| (tb.icon(), tb.label())).collect();
    let current = tabs.iter().position(|tb| *tb == tab).unwrap_or(0);
    if let Some(i) = crate::widgets::tab_strip(ui, strip, &labels, current)
        && let Some(tb) = tabs.get(i)
    {
        tab = *tb;
        app.ui.inspector_tab = *tb;
    }
    ui.painter().rect_filled(Rect::from_min_max(pos2(strip.right(), full.top()), full.max), 0.0, tk.panel);
    let body = Rect::from_min_max(pos2(strip.right() + 8.0, full.top()), pos2(full.right() - 8.0, full.bottom()));
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            match (tab, active) {
                (Tab::Tool, _) => tool_tab(app, ui),
                (Tab::Object, Some(k)) => object_tab(app, ui, k),
                (Tab::Mesh, Some(k)) => mesh_tab(app, ui, k),
                (Tab::Shapes, Some(_)) => crate::sdf::panel::show(app, ui),
                _ => scene_tab(app, ui),
            }
        });
    });
}

fn tool_tab(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    panel(ui, "tool", t("Active tool"), true, |ui| {
        let name = match app.ui.tool {
            crate::Tool::Select => t("Select"),
            crate::Tool::Move => t("Move"),
            crate::Tool::Rotate => t("Rotate"),
            crate::Tool::Scale => t("Scale"),
            crate::Tool::Knife => t("Knife"),
        };
        info(ui, t("Tool"), name);
        info(ui, t("Mode"), app.ui.mode.label());
    });
    // A report (an op that changed nothing): its lines and fixes.
    let registry = std::sync::Arc::clone(app.session.registry());
    let revision = app.session.revision();
    if let Some(r) = app.result.clone().filter(|r| r.read_only && r.revision == revision) {
        let mut fix = None;
        panel(ui, "report", &r.title(&registry), true, |ui| fix = crate::results::report_ui(ui, &r, &registry, true));
        if let Some(f) = fix {
            let _ = crate::results::run_fix(app, f);
            return;
        }
    }
    let Some(last) = app.last_op.clone() else {
        panel(ui, "last", t("Last operation"), true, |ui| {
            ui.label(RichText::new(t("Run an operation, then adjust its settings here.")).color(tk.text_dim));
        });
        return;
    };
    let Ok(op) = registry.get(&last.op) else { return };
    let title = format!("{}: {}", t("Adjust"), op.meta().label);
    let current = app.session.revision() == last.revision;
    let mut params = last.params.clone();
    let mut out = schema_form::FormOutput::default();
    // The op's warnings, each fix a button.
    let result = app.result.clone().filter(|r| !r.read_only && r.revision == last.revision && r.warning_count() > 0);
    let mut fix = None;
    panel(ui, "last", &title, true, |ui| {
        let schema = op.params_schema();
        ui.add_enabled_ui(current, |ui| {
            out = schema_form::form(ui, egui::Id::new(("adjust", last.op.as_str())), &schema, &schema, &mut params, &[]);
        });
        if !current {
            ui.label(RichText::new(t("The scene changed since; run the operation again to adjust it.")).font(theme::small()).color(tk.text_dim));
        }
        if let Some(e) = &last.error {
            ui.label(RichText::new(e).font(theme::small()).color(tk.health_error));
        }
        if let Some(r) = &result {
            ui.add_space(4.0);
            fix = crate::results::warnings_ui(ui, r, &registry, current);
        }
    });
    if out.changed && current {
        let _ = app.adjust_last(params);
    }
    if let Some(f) = fix {
        let _ = crate::results::run_fix(app, f);
    }
}

fn scene_tab(app: &mut PixelApp, ui: &mut egui::Ui) {
    let doc = std::sync::Arc::clone(app.session.doc());
    panel(ui, "scene", t("Scene"), true, |ui| {
        info(ui, t("Objects"), doc.object_count().to_string());
        info(ui, t("Asset class"), doc.settings.asset_class.as_str());
        info(ui, t("Budget"), format!("{} {}", chrome::thousands(doc.settings.tri_budget() as usize), t("tris")));
        info(ui, t("Units"), t("meters"));
        info(ui, t("Revision"), doc.revision.to_string());
        info(ui, t("File"), app.path.as_ref().map_or_else(|| t("not saved yet").to_owned(), |p| p.display().to_string()));
    });
    let n = app.selection.len();
    // Counted once per document state and selection, not per frame.
    let tris = app.stats().selected.tris;
    if n > 1 {
        panel(ui, "selection", t("Selection"), true, |ui| {
            info(ui, t("Objects"), n.to_string());
            info(ui, t("Triangles"), chrome::thousands(tris));
        });
    }
}

fn object_tab(app: &mut PixelApp, ui: &mut egui::Ui, key: ObjectKey) {
    let tk = Tokens::get();
    let doc = std::sync::Arc::clone(app.session.doc());
    let Some(obj) = doc.object(key) else { return };
    let id = obj.public_id();
    // Name, editable in place.
    let name_id = ui.id().with(("name", key));
    let mut name: String = ui.data_mut(|d| d.get_temp(name_id)).unwrap_or_else(|| obj.name().to_owned());
    let resp = ui
        .horizontal(|ui| {
            icons::paint(
                ui.painter(),
                Rect::from_min_size(ui.cursor().min + vec2(0.0, 2.0), vec2(18.0, 18.0)),
                match crate::outliner::kind_of(obj) {
                    crate::outliner::Kind::Collection => Icon::Collection,
                    crate::outliner::Kind::LodGroup => Icon::LodGroup,
                    crate::outliner::Kind::Mesh => Icon::Mesh,
                    crate::outliner::Kind::Curve => Icon::Curve,
                    _ => Icon::Empty,
                },
                false,
            );
            ui.add_space(22.0);
            ui.add_sized([ui.available_width(), 22.0], egui::TextEdit::singleline(&mut name))
        })
        .inner;
    if resp.changed() {
        ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
    }
    if resp.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(name_id));
        if name != obj.name() && !name.trim().is_empty() {
            let _ = app.execute(&[OpCall::new("scene.rename", json!({ "target": id, "name": name.trim() }))]);
        }
    }
    ui.add_space(6.0);

    // Transform: the set_transform form, with the object's current values.
    let registry = std::sync::Arc::clone(app.session.registry());
    let tr = &obj.transform;
    let rot = p3d_math::transform::rotation_to_euler_deg(tr.rotation);
    let mut values = json!({
        "target": id,
        "position": [tr.translation.x, tr.translation.y, tr.translation.z],
        "rotation": rot,
        "scale": [tr.scale.x, tr.scale.y, tr.scale.z],
    });
    let mut out = schema_form::FormOutput::default();
    panel(ui, "transform", t("Transform"), true, |ui| {
        if let Ok(op) = registry.get("scene.set_transform") {
            let schema = op.params_schema();
            out = schema_form::form_with(
                ui,
                egui::Id::new(("transform", key)),
                &schema,
                &schema,
                &mut values,
                &["target", "relative"],
                schema_form::FormOptions { optionals_always_on: true },
            );
        }
    });
    if out.changed {
        let _ = app.execute_coalesced(&[OpCall::new("scene.set_transform", values)], out.dragging);
    } else if !out.dragging {
        app.end_drag();
    }

    panel(ui, "relations", t("Relations"), false, |ui| {
        let parent = obj.parent().and_then(|p| doc.path(p)).unwrap_or_else(|| t("none").to_owned());
        info(ui, t("Parent"), parent);
        info(ui, t("Children"), obj.children().len().to_string());
        info(ui, t("Path"), doc.path(key).unwrap_or_default());
    });
    panel(ui, "info", t("Info"), false, |ui| {
        info(ui, t("Id"), id.clone());
        info(ui, t("Kind"), obj.kind().as_str());
        let tags = if obj.tags.is_empty() { t("none").to_owned() } else { obj.tags.join(", ") };
        info(ui, t("Tags"), tags);
        if !obj.visible {
            ui.label(RichText::new(t("Hidden in exports and renders.")).font(theme::small()).color(tk.text_dim));
        }
    });
}

fn mesh_tab(app: &mut PixelApp, ui: &mut egui::Ui, key: ObjectKey) {
    let tk = Tokens::get();
    let doc = std::sync::Arc::clone(app.session.doc());
    let Some(m) = doc.object_mesh(key) else { return };
    app.stats();
    let tris = app.stats.tris_of(key).unwrap_or_else(|| m.triangle_count());
    panel(ui, "mesh", t("Mesh"), true, |ui| {
        info(ui, t("Vertices"), chrome::thousands(m.vert_count()));
        info(ui, t("Edges"), chrome::thousands(m.edge_count()));
        info(ui, t("Faces"), chrome::thousands(m.face_count()));
        info(ui, t("Triangles"), chrome::thousands(tris));
        if let Some((lo, hi)) = m.bounds() {
            let d = hi - lo;
            info(ui, t("Size"), format!("{} x {} x {} m", fmt(d.x), fmt(d.y), fmt(d.z)));
        }
        info(ui, t("UV layers"), m.uv_layer_count().to_string());
    });
    // The group a boolean just cut stands out; clicking a group edits it.
    let fresh = app.edit_focus.as_ref().filter(|(rev, k, _)| *rev == app.session.revision() && *k == key).map(|(_, _, n)| n.clone());
    let mut pick: Option<String> = None;
    panel(ui, "groups", t("Groups"), true, |ui| {
        if m.group_count() == 0 {
            ui.label(RichText::new(t("No groups.")).color(tk.text_dim));
        }
        for (name, g) in m.groups() {
            let new = fresh.as_deref() == Some(name);
            let bg = ui.painter().add(egui::Shape::Noop);
            let row = ui
                .horizontal(|ui| {
                    ui.label(RichText::new(g.kind().as_str()).font(theme::small()).color(tk.text_faint));
                    ui.add(egui::Label::new(RichText::new(name).color(if new { tk.icon_accent } else { tk.text })).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(chrome::thousands(g.count())).color(tk.text_dim));
                    });
                })
                .response
                .interact(egui::Sense::click());
            if row.hovered() {
                ui.painter().set(bg, egui::epaint::RectShape::filled(row.rect.expand(2.0), 4.0, tk.control));
            }
            crate::control::record_widget(ui, &row, &format!("{} {name}", t("Select group")), "listitem", Some(new));
            let tip = if new { t("Made by the last operation. Click to select it in Edit Mode.") } else { t("Click to select it in Edit Mode.") };
            if row.on_hover_text(tip).clicked() {
                pick = Some(name.to_owned());
            }
        }
    });
    if let Some(name) = pick {
        let _ = app.select_group(key, &name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn adjusting_the_last_operation_replaces_it() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate", "width": 1 })).unwrap();
        let steps = app.session.history().len();
        let mut params = app.last_op.clone().map(|l| l.params).unwrap_or_default();
        params["width"] = json!(2.0);
        app.adjust_last(params.clone()).unwrap();
        assert_eq!(app.session.history().len(), steps, "still one undo step");
        let k = app.doc().resolve("crate").unwrap();
        let (lo, hi) = app.doc().object_mesh(k).and_then(|m| m.bounds()).unwrap();
        assert!(((hi - lo).x - 2.0).abs() < 1e-5);
        // A refused value keeps the old result and reports the error.
        params["width"] = json!(-1.0);
        assert!(app.adjust_last(params).is_err());
        assert!(app.last_op.as_ref().is_some_and(|l| l.error.is_some()));
        assert_eq!(app.doc().object_count(), 1);
        // After another edit, adjusting is refused.
        app.run("prim.box", Value::Null).unwrap();
        app.run("edit.select_all", Value::Null).unwrap();
        app.run("edit.duplicate", Value::Null).unwrap();
        app.run("edit.undo", Value::Null).unwrap();
        assert!(app.adjust_last(json!({})).is_err());
    }

    #[test]
    fn a_drag_is_one_undo_step() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        let steps = app.session.history().len();
        for x in [0.1, 0.2, 0.3] {
            app.execute_coalesced(&[OpCall::new("scene.set_transform", json!({ "target": "crate", "position": [x, 0, 0] }))], true).unwrap();
        }
        app.end_drag();
        assert_eq!(app.session.history().len(), steps + 1);
        let k = app.doc().resolve("crate").unwrap();
        assert!((app.doc().object(k).map(|o| o.transform.translation.x).unwrap_or(0.0) - 0.3).abs() < 1e-6);
        // The next edit is a new step.
        app.execute_coalesced(&[OpCall::new("scene.set_transform", json!({ "target": "crate", "position": [1, 0, 0] }))], false).unwrap();
        assert_eq!(app.session.history().len(), steps + 2);
    }

    #[test]
    fn hidden_objects_leave_the_selection() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        let k = app.doc().resolve("crate").unwrap();
        assert!(app.selection.contains(&k));
        app.set_hidden(k, true);
        assert!(!app.selection.contains(&k) && app.hidden.contains(&k));
        app.set_hidden(k, false);
        assert!(app.hidden.is_empty());
    }
}
