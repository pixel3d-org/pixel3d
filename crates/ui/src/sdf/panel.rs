//! The Shapes tab of Properties: the ordered shape list of the edited SDF
//! object (one row per shape: drag handle, visibility, primitive, name,
//! join, blend size and mirror), the list's buttons (add, duplicate,
//! delete, wrap into a group), and the selected shape's fields as forms
//! from the op schema. Hovering a row lights its faces in the viewport.

use egui::{Align, Layout, RichText, Sense, Stroke, vec2};
use serde_json::{Map, Value, json};

use super::Apply;
use super::shapes::{self, Path};
use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::schema_form;
use crate::theme::{self, Tokens};
use crate::widgets::section as panel;

/// Width of a row's join, blend and mirror controls.
const JOIN_W: f32 = 50.0;
const BLEND_W: f32 = 50.0;
const MIRROR_W: f32 = 22.0;

/// The icon of a primitive.
pub fn prim_icon(prim: &str) -> Icon {
    match prim {
        "box" => Icon::Box,
        "capsule" => Icon::Capsule,
        "cone" => Icon::Cone,
        "torus" => Icon::Torus,
        "cylinder" => Icon::Cylinder,
        "group" => Icon::Collection,
        _ => Icon::Sphere,
    }
}

/// A join's short name in a row and what it does.
fn join_text(join: &str) -> (&'static str, &'static str) {
    match join {
        "smooth_union" => (t("Blend"), t("Join with a rounded fillet of the blend size")),
        "subtract" => (t("Cut"), t("Cut this shape out of the shapes before it")),
        "smooth_subtract" => (t("Soft cut"), t("Cut with a rounded fillet of the blend size")),
        "intersect" => (t("Keep"), t("Keep only what this shape and the shapes before it share")),
        "smooth_intersect" => (t("Soft keep"), t("Keep the shared part, with a rounded fillet")),
        _ => (t("Union"), t("Add this shape to the shapes before it")),
    }
}

/// The schema `$defs` entry of a primitive's fields.
fn prim_def(prim: &str) -> &'static str {
    match prim {
        "ellipsoid" => "EllipsoidPrim",
        "box" => "BoxPrim",
        "capsule" => "CapsulePrim",
        "cone" => "ConePrim",
        "torus" => "TorusPrim",
        "cylinder" => "CylinderPrim",
        _ => "SpherePrim",
    }
}

/// The modifier fields, written against the op schema's `$defs`; zeros
/// mean off and are left out of the shape.
fn modifier_schema() -> Value {
    let len = |d: &str| json!({ "$ref": "#/$defs/Length", "description": d });
    let ang = |d: &str| json!({ "$ref": "#/$defs/Angle", "description": d });
    json!({ "type": "object", "properties": {
        "rotate": { "type": "array", "items": { "$ref": "#/$defs/Angle" }, "minItems": 3, "maxItems": 3,
                    "description": t("Turn around the shape's center: degrees about X, then Y, then Z") },
        "round": len(t("Grow the shape with rounded edges (0 is off)")),
        "shell": len(t("Hollow the shape into a skin this thick (0 is off)")),
        "twist": ang(t("Turn around the shape's Y axis, degrees per meter (0 is off)")),
        "bend": ang(t("Bend the shape's Y axis toward +X, degrees per meter (0 is off)")),
        "noise": { "anyOf": [{ "$ref": "#/$defs/NoiseParam" }, { "type": "null" }], "description": t("Seeded bumps on the surface") },
    } })
}

const MODIFIERS: [&str; 6] = ["rotate", "round", "shell", "twist", "bend", "noise"];

/// What the tab asks for this frame.
#[derive(Default)]
struct Out {
    command: Option<(&'static str, Value)>,
    edited: Option<(Vec<Value>, bool, String)>,
    select: Option<Path>,
    hovered: Option<Path>,
    dragging: bool,
    /// A new detail (meters, `None` for automatic) and whether it is dragged.
    detail: Option<(Option<f64>, bool)>,
}

/// The Shapes tab body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    if app.sdf.object.is_none() {
        ui.label(RichText::new(t("Select an object made from SDF shapes (Add > SDF Shape).")).color(tk.text_dim));
        return;
    }
    let list = app.sdf.shapes.clone();
    let selected = app.sdf.selected.clone().filter(|p| shapes::get(&list, p).is_some());
    let mut out = Out::default();
    panel(ui, "sdf-shapes", t("Shapes"), true, |ui| {
        toolbar(ui, selected.is_some(), &mut out);
        ui.add_space(2.0);
        let width = ui.available_width();
        for (path, shape) in shapes::flatten(&list) {
            row(ui, &list, &path, shape, selected.as_ref() == Some(&path), width, &mut out);
        }
        // Dropping below the last row moves a shape to the end.
        let (end, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
        let end = ui.interact(end, ui.id().with("sdf-end"), Sense::hover());
        if end.dnd_hover_payload::<Path>().is_some() {
            ui.painter().hline(end.rect.x_range(), end.rect.top() + 1.0, Stroke::new(2.0, tk.accent));
        }
        if let Some(from) = end.dnd_release_payload::<Path>() {
            out.command = Some(("shapes.move", json!({ "from": *from, "before": [list.len()] })));
        }
    });
    if let Some(path) = &selected {
        shape_fields(app, ui, &list, path, &mut out);
    }
    mesh_fields(app, ui, &mut out);
    // Apply what was asked, in one place.
    app.sdf.hover_next = out.hovered;
    if let Some(p) = out.select {
        app.sdf.selected = Some(p);
    }
    if let Some((id, params)) = out.command {
        if let Err(e) = app.sdf_command(id, &params) {
            app.set_status(e);
        }
    } else if let Some((new, dragging, label)) = out.edited {
        app.set_sdf_shapes(new, if dragging { Apply::Live } else { Apply::Commit(&label) });
    } else if let Some((detail, dragging)) = out.detail {
        app.sdf.detail = detail.map(|d| d.clamp(super::MIN_DETAIL, super::MAX_DETAIL));
        app.sdf.changed();
        app.apply_sdf(if dragging { Apply::Live } else { Apply::Commit(t("Detail")) });
    } else if app.sdf.live && !app.sdf_dragging() && !out.dragging {
        // A field drag ended: rebuild at full detail.
        app.apply_sdf(Apply::Commit(t("Edit shapes")));
    }
}

/// Add (a menu of primitives), duplicate, delete and group.
fn toolbar(ui: &mut egui::Ui, has_selection: bool, out: &mut Out) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let add = icons::button(ui, Icon::Plus, 24.0, t("Add a shape after the selected one"), false);
        egui::Popup::menu(&add).frame(crate::widgets::popup_frame(ui)).gap(4.0).align(egui::RectAlign::BOTTOM_START).width(180.0).show(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for prim in shapes::PRIMS.iter().filter(|p| **p != "group") {
                if crate::widgets::menu_row(ui, Some(prim_icon(prim)), &schema_form::humanize(prim), None, None, true).clicked() {
                    out.command = Some(("shapes.add", json!({ "prim": prim })));
                }
            }
        });
        ui.add_enabled_ui(has_selection, |ui| {
            if icons::button(ui, Icon::Copy, 24.0, t("Duplicate the selected shape"), false).clicked() {
                out.command = Some(("shapes.duplicate", Value::Null));
            }
            if icons::button(ui, Icon::Trash, 24.0, t("Delete the selected shape"), false).clicked() {
                out.command = Some(("shapes.delete", Value::Null));
            }
            if icons::button(ui, Icon::Collection, 24.0, t("Wrap the selected shape into a group"), false).clicked() {
                out.command = Some(("shapes.group", Value::Null));
            }
        });
    });
}

/// One row of the list.
#[allow(clippy::too_many_arguments)] // the row, its shape and its place in the list
fn row(ui: &mut egui::Ui, list: &[Value], path: &Path, shape: &Value, selected: bool, width: f32, out: &mut Out) {
    let tk = Tokens::get();
    let id = ui.id().with(("sdf-row", path));
    let label = shapes::label(list, path);
    let hidden = shapes::is_hidden(shape);
    let prim = shapes::prim_key(shape).unwrap_or("sphere");
    let bg = ui.painter().add(egui::Shape::Noop);
    let resp = ui
        .horizontal(|ui| {
            ui.set_min_height(24.0);
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.add_space((path.len() - 1) as f32 * 10.0);
            let tip = if hidden { format!("{} {label}", t("Show")) } else { format!("{} {label}", t("Hide")) };
            if icons::button(ui, if hidden { Icon::EyeOff } else { Icon::Eye }, 16.0, &tip, false).clicked() {
                out.command = Some(("shapes.hide", json!({ "path": path })));
            }
            // The primitive's icon is the handle that drags the row elsewhere.
            let grip = ui
                .dnd_drag_source(id.with("grip"), path.clone(), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::hover());
                    icons::paint(ui.painter(), r.shrink2(vec2(0.0, 2.0)), prim_icon(prim), hidden);
                })
                .response;
            grip.on_hover_text(format!("{}: {}", schema_form::humanize(prim), t("drag to reorder")));
            let controls = JOIN_W + BLEND_W + MIRROR_W + 9.0;
            // From the list's width, so a wide row never widens the next ones.
            let used = (path.len() - 1) as f32 * 10.0 + 16.0 + 16.0 + 6.0;
            let name_w = (width - used - controls).max(36.0);
            let color = if hidden { tk.text_faint } else { tk.text };
            let name = ui
                .allocate_ui_with_layout(vec2(name_w, 20.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.set_min_size(vec2(name_w, 20.0));
                    ui.add(egui::Label::new(RichText::new(&label).color(color)).truncate().sense(Sense::click()))
                })
                .inner;
            crate::control::record_widget(ui, &name, &format!("{} {label}", t("Shape")), "listitem", Some(selected));
            if name.clicked() {
                out.select = Some(path.clone());
            }
            // The join: a compact button with a menu of the six joins.
            let (join, blend) = shapes::join_of(shape);
            let (short, what) = join_text(join);
            let jb = ui.add_sized([JOIN_W, 20.0], egui::Button::new(RichText::new(short).font(theme::small())));
            crate::control::record_widget(ui, &jb, &format!("{} {label}", t("Join of")), "button", None);
            let jb = jb.on_hover_text(what);
            egui::Popup::menu(&jb).frame(crate::widgets::popup_frame(ui)).gap(4.0).width(150.0).show(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for (j, _) in shapes::JOINS {
                    let (s, d) = join_text(j);
                    let r = crate::widgets::menu_row(ui, None, s, None, Some(j == join), false).on_hover_text(d);
                    if r.clicked() && j != join {
                        out.command = Some(("shapes.join", json!({ "path": path, "join": j })));
                    }
                }
            });
            match blend {
                Some(mut b) => {
                    let drag = egui::DragValue::new(&mut b)
                        .speed(0.0005)
                        .range(0.0001..=1.0)
                        .custom_formatter(|v, _| p3d_math::units::format_length(v))
                        .custom_parser(|s| p3d_math::units::parse_length(s).ok());
                    let r = ui.scope(|ui| {
                        ui.style_mut().override_font_id = Some(theme::small());
                        ui.add_sized([BLEND_W, 20.0], drag)
                    });
                    let r = r.inner;
                    let r = r.on_hover_text(t("Blend size: the fillet where this shape joins"));
                    out.dragging |= r.dragged();
                    if r.changed() {
                        let mut next = list.to_vec();
                        if let Some(s) = shapes::get_mut(&mut next, path) {
                            shapes::set_join(s, join, b);
                        }
                        out.edited = Some((next, r.dragged(), format!("{} {label}", t("Blend"))));
                    }
                }
                None => {
                    ui.allocate_exact_size(vec2(BLEND_W, 20.0), Sense::hover());
                }
            }
            // Mirror: a click steps through none, X, Y and Z.
            let mirror = shapes::mirror_of(shape);
            let shown = if mirror.is_empty() { "-".to_owned() } else { mirror.to_uppercase().replace(", ", "") };
            let next = match shapes::MIRRORS.iter().position(|a| *a == mirror) {
                Some(i) => shapes::MIRRORS[(i + 1) % shapes::MIRRORS.len()],
                None => "",
            };
            let mb = ui.add_sized([MIRROR_W, 20.0], egui::Button::new(RichText::new(shown).font(theme::small())).selected(!mirror.is_empty()));
            crate::control::record_widget(ui, &mb, &format!("{} {label}", t("Mirror")), "button", Some(!mirror.is_empty()));
            let tip = if next.is_empty() {
                t("Mirror: click to stop mirroring").to_owned()
            } else {
                format!("{} {}", t("Mirror: click to mirror across"), next.to_uppercase())
            };
            if mb.on_hover_text(tip).clicked() {
                out.command = Some(("shapes.mirror", json!({ "path": path, "axis": next })));
            }
        })
        .response;
    let over = ui.rect_contains_pointer(resp.rect);
    if over {
        out.hovered = Some(path.clone());
    }
    let fill = if selected {
        tk.accent_soft
    } else if over {
        tk.control.gamma_multiply(0.5)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().set(bg, egui::epaint::RectShape::filled(resp.rect.expand2(vec2(2.0, 1.0)), 5.0, fill));
    if resp.dnd_hover_payload::<Path>().is_some_and(|from| *from != *path) {
        ui.painter().hline(resp.rect.x_range(), resp.rect.top() - 1.0, Stroke::new(2.0, tk.accent));
    }
    if let Some(from) = resp.dnd_release_payload::<Path>()
        && *from != *path
    {
        out.command = Some(("shapes.move", json!({ "from": *from, "before": path })));
    }
}

/// The selected shape: its name, primitive fields and modifiers.
fn shape_fields(app: &PixelApp, ui: &mut egui::Ui, list: &[Value], path: &Path, out: &mut Out) {
    let Some(shape) = shapes::get(list, path) else { return };
    let prim = shapes::prim_key(shape).unwrap_or("sphere");
    let label = shapes::label(list, path);
    let Ok(op) = app.session.registry().get(super::op_id(app.session.registry())) else { return };
    let root = op.params_schema();
    let title = format!("{}: {label}", schema_form::humanize(prim));
    let base = egui::Id::new(("sdf-fields", path));
    let mut next = list.to_vec();
    let mut changed: Option<(bool, String)> = None;
    panel(ui, "sdf-shape", &title, true, |ui| {
        // Name: a new name renames the face group, so it applies on Enter.
        let name_id = base.with("name");
        let mut name: String = ui.data_mut(|d| d.get_temp(name_id)).unwrap_or_else(|| shapes::name_of(shape).unwrap_or("").to_owned());
        let r = crate::widgets::prop_row(ui, t("Name"), t("Its faces form a group of this name"), |ui| {
            ui.add_sized([ui.available_width(), crate::widgets::ROW], egui::TextEdit::singleline(&mut name).hint_text(&label))
        });
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
        }
        if r.lost_focus() {
            ui.data_mut(|d| d.remove::<String>(name_id));
            let name = name.trim();
            if name != shapes::name_of(shape).unwrap_or("")
                && let Some(s) = shapes::get_mut(&mut next, path).and_then(Value::as_object_mut)
            {
                if name.is_empty() {
                    s.remove("name");
                } else {
                    s.insert("name".into(), json!(name));
                }
                changed = Some((false, t("Rename shape").to_owned()));
            }
        }
        // The primitive's own fields, from its schema.
        if prim == "group" {
            let schema = json!({ "type": "object", "properties": { "at": { "type": "array", "items": { "$ref": "#/$defs/Length" }, "minItems": 3, "maxItems": 3, "description": t("The group's center for rotate") } } });
            let mut v = json!({ "at": shape.get("at").cloned().unwrap_or(json!([0.0, 0.0, 0.0])) });
            let o = schema_form::form(ui, base.with("group"), &schema, &root, &mut v, &[]);
            out.dragging |= o.dragging;
            if o.changed {
                next = list.to_vec();
                if let Some(s) = shapes::get_mut(&mut next, path) {
                    s["at"] = v["at"].clone();
                }
                changed = Some((o.dragging, format!("{} {label}", t("Edit"))));
            }
        } else if let Some(schema) = root.get("$defs").and_then(|d| d.get(prim_def(prim))) {
            let mut v = shape.get(prim).cloned().unwrap_or(Value::Null);
            let o = schema_form::form(ui, base.with("prim"), schema, &root, &mut v, &[]);
            out.dragging |= o.dragging;
            if o.changed {
                next = list.to_vec();
                if let Some(s) = shapes::get_mut(&mut next, path) {
                    s[prim] = v;
                }
                changed = Some((o.dragging, format!("{} {label}", t("Edit"))));
            }
        }
        ui.add_space(4.0);
        crate::widgets::menu_header(ui, t("Modifiers"));
        let mut m = Map::new();
        for key in MODIFIERS {
            let v = shape.get(key).cloned().unwrap_or(Value::Null);
            let v = match (key, v) {
                ("rotate", Value::Null) => json!([0.0, 0.0, 0.0]),
                ("noise", v) => v,
                (_, Value::Null) => json!(0.0),
                (_, v) => v,
            };
            m.insert(key.to_owned(), v);
        }
        let mut v = Value::Object(m);
        let o = schema_form::form(ui, base.with("modifiers"), &modifier_schema(), &root, &mut v, &[]);
        out.dragging |= o.dragging;
        if o.changed {
            next = list.to_vec();
            if let Some(s) = shapes::get_mut(&mut next, path).and_then(Value::as_object_mut) {
                for key in MODIFIERS {
                    let x = v.get(key).cloned().unwrap_or(Value::Null);
                    let off = x.is_null() || x.as_f64() == Some(0.0) || x.as_array().is_some_and(|a| a.iter().all(|c| c.as_f64() == Some(0.0)));
                    if off {
                        s.remove(key);
                    } else {
                        s.insert(key.to_owned(), x);
                    }
                }
            }
            changed = Some((o.dragging, format!("{} {label}", t("Edit"))));
        }
    });
    if let Some((dragging, label)) = changed {
        out.edited = Some((next, dragging, label));
    }
}

/// The mesh settings: the detail, what a rebuild drops, and errors.
fn mesh_fields(app: &PixelApp, ui: &mut egui::Ui, out: &mut Out) {
    let tk = Tokens::get();
    let Ok(op) = app.session.registry().get(super::op_id(app.session.registry())) else { return };
    let root = op.params_schema();
    panel(ui, "sdf-mesh", t("Mesh"), true, |ui| {
        let schema = json!({ "type": "object", "properties": { "detail": {
            "anyOf": [{ "$ref": "#/$defs/Length" }, { "type": "null" }],
            "description": t("Voxel size, about the edge length of the mesh: smaller is finer. Auto fits about 64 across the largest side.")
        } } });
        let mut v = json!({ "detail": app.sdf.detail });
        let o = schema_form::form(ui, egui::Id::new("sdf-detail"), &schema, &root, &mut v, &[]);
        out.dragging |= o.dragging;
        if o.changed {
            out.detail = Some((v["detail"].as_f64(), o.dragging));
        }
        if !app.sdf.later.is_empty() {
            let text = format!("{} {}", t("Editing the shapes rebuilds the mesh and drops:"), app.sdf.later.join(", "));
            ui.add(egui::Label::new(RichText::new(text).font(theme::small()).color(tk.health_warn)).wrap());
        }
        if let Some(e) = &app.sdf.error {
            ui.add(egui::Label::new(RichText::new(e).font(theme::small()).color(tk.health_error)).wrap());
        }
        let note = if app.sdf.busy() { t("Rebuilding...") } else { t("Drag a value for a quick preview; the mesh rebuilds when you let go.") };
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.add(egui::Label::new(RichText::new(note).font(theme::small()).color(tk.text_faint)).wrap());
        });
    });
}
