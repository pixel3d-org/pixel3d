//! The Spec tab of Properties: the parts of the edited spec (kind and
//! join), the selected part's shape, anchor, join, transform and tags, the
//! Shapes editor for SDF parts, and the raw JSON source with errors at
//! their lines. Every change is a `spec.patch` merge patch of the part, run
//! in the background; a dragged value commits when the drag ends.

use std::sync::Arc;

use egui::{RichText, Sense, Stroke, pos2, vec2};
use p3d_scene::ObjectKey;
use serde_json::{Map, Value, json};

use super::{PartInfo, SpecIssue, part_source};
use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::schema_form::{self, Kind};
use crate::theme::{self, Tokens};
use crate::widgets::{info, prop_row, section as panel};

/// Height of a part row.
const ROW: f32 = 22.0;

/// What the tab asks for this frame.
#[derive(Default)]
struct Out {
    /// Select a part (`None` for the whole spec).
    select: Option<Option<String>>,
    /// A merge patch of the selected part, its undo label, and whether a
    /// value is being dragged.
    patch: Option<(Value, String, bool)>,
    /// Anchor the part on another part (keeping its landmark when the new
    /// parent has it).
    anchor_on: Option<String>,
    landmark: Option<String>,
    command: Option<(&'static str, Value)>,
    hovered: Option<(ObjectKey, String)>,
    dragging: bool,
    apply_json: bool,
    revert_json: bool,
}

/// The icon of a part row.
fn part_icon(p: &PartInfo) -> Icon {
    match p.key {
        "sdf" => Icon::Shapes,
        "primitive" => crate::sdf::panel::prim_icon(&p.kind),
        "mirror" => Icon::Symmetrize,
        "instance" => Icon::Copy,
        "lathe" => Icon::Tube,
        "sweep" => Icon::Sweep,
        "loft" => Icon::Loft,
        "extrude" => Icon::Extrude,
        "pipe" => Icon::Pipe,
        "feature" => Icon::Features,
        "shell" => Icon::Leaf,
        "silhouettes" => Icon::Blueprint,
        _ => Icon::Empty,
    }
}

/// The schema `$defs` (Length, Angle) the forms refer to.
fn defs(app: &PixelApp) -> Value {
    let registry = app.session.registry();
    registry.get(crate::sdf::op_id(registry)).map(|op| op.params_schema()).unwrap_or(Value::Null)
}

/// Lengths and angles written as text (`"2cm"`, `"90deg"`) as the numbers
/// forms edit (meters, degrees), and a uniform triple as three values.
fn normalize(kind: &Kind, x: &mut Value) {
    match kind {
        Kind::Length => {
            if let Ok(l) = serde_json::from_value::<p3d_ops::params::Length>(x.clone()) {
                *x = json!(l.0);
            }
        }
        Kind::Angle => {
            if let Ok(a) = serde_json::from_value::<p3d_ops::params::Angle>(x.clone())
                && let Ok(d) = serde_json::to_value(a)
            {
                *x = d;
            }
        }
        Kind::Optional(k) if !x.is_null() => normalize(k, x),
        Kind::Triple(k) => {
            if !x.is_array() && !x.is_null() {
                *x = json!([x.clone(), x.clone(), x.clone()]);
            }
            if let Some(a) = x.as_array_mut() {
                for e in a {
                    normalize(k, e);
                }
            }
        }
        _ => {}
    }
}

/// The values of `schema`'s properties in `src` (or their defaults),
/// normalized for a form.
///
/// Complexity: O(properties).
fn form_value(schema: &Value, root: &Value, src: &Value) -> Value {
    let mut out = Map::new();
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (k, s) in props {
            if let Some(v) = src.get(k).or_else(|| s.get("default")) {
                let mut v = v.clone();
                normalize(&schema_form::kind_of(s, root), &mut v);
                out.insert(k.clone(), v);
            }
        }
    }
    Value::Object(out)
}

#[cfg(test)]
pub(crate) fn form_value_for_tests(schema: &Value, root: &Value, src: &Value) -> Value {
    form_value(schema, root, src)
}

/// The keys whose values differ between two form values, as a merge patch.
///
/// Complexity: O(properties).
fn changed(before: &Value, after: &Value) -> Value {
    let mut patch = Map::new();
    if let Some(a) = after.as_object() {
        for (k, v) in a {
            if before.get(k).unwrap_or(&Value::Null) != v {
                patch.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(patch)
}

/// The Spec tab body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let Some(root) = app.spec.root else {
        ui.label(RichText::new(t("Select a spec root or one of its parts (Add > Spec).")).color(tk.text_dim));
        return;
    };
    let doc = Arc::clone(app.session.doc());
    let parts = app.spec_parts().get(&root).cloned().unwrap_or_default();
    let root_name = doc.object(root).map(|o| o.name().to_owned()).unwrap_or_default();
    let mut out = Out::default();
    let selected = app.spec.part.clone();
    panel(ui, "spec-parts", &format!("{} {root_name}", t("Spec")), true, |ui| {
        let whole = PartInfo {
            name: t("Whole spec").to_owned(),
            kind: format!("{} {}", parts.len(), t("parts")),
            key: "",
            join: "",
            on: None,
            object: None,
            blend_into: None,
        };
        if part_row(ui, &whole, Icon::Scene, selected.is_none(), 0).clicked() {
            out.select = Some(None);
        }
        for p in &parts {
            let depth = usize::from(p.on.is_some());
            let resp = part_row(ui, p, part_icon(p), selected.as_deref() == Some(p.name.as_str()), depth);
            if resp.hovered() && p.blend_into.is_some() {
                out.hovered = Some((root, p.name.clone()));
            }
            if resp.clicked() {
                out.select = Some(Some(p.name.clone()));
            }
        }
        ui.add_space(4.0);
        if app.spec.busy() {
            note(ui, t("Compiling..."), tk.text_faint);
        }
        if let Some(e) = &app.spec.error {
            issue(ui, e);
        }
    });
    match selected.as_deref().and_then(|s| parts.iter().find(|p| p.name == s)) {
        Some(p) => {
            let src = match &app.spec.draft {
                Some((r, n, v)) if *r == root && *n == p.name => v.clone(),
                _ => part_source(&doc, root, &p.name).unwrap_or(Value::Null),
            };
            part_fields(app, ui, p, &parts, &src, &mut out);
            if p.key == "sdf" {
                crate::sdf::panel::show(app, ui);
            } else if matches!(p.key, "shell" | "silhouettes") {
                crate::outline::panel::show(app, ui);
            }
        }
        None => whole_fields(app, ui, root),
    }
    json_view(app, ui, &mut out);
    apply(app, root, out);
}

/// A selectable part row: icon, name, kind and join.
fn part_row(ui: &mut egui::Ui, p: &PartInfo, icon: Icon, selected: bool, depth: usize) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let pill = rect.shrink2(vec2(1.0, 1.0));
    if selected {
        ui.painter().rect_filled(pill, 5.0, tk.accent.gamma_multiply(0.22));
    } else if resp.hovered() {
        ui.painter().rect_filled(pill, 5.0, tk.control.gamma_multiply(0.8));
    }
    let x0 = rect.left() + 6.0 + depth as f32 * 12.0;
    let ir = egui::Rect::from_min_size(pos2(x0, rect.center().y - 7.0), vec2(14.0, 14.0));
    icons::paint_colored(ui.painter(), ir, icon, tk.icon, tk.accent);
    // Right: the join; middle: the kind after the name.
    let mut right = rect.right() - 6.0;
    if !p.join.is_empty() && p.join != "separate" {
        let g = ui.painter().layout_no_wrap(t(p.join).to_owned(), egui::FontId::proportional(9.5), tk.icon_accent);
        let r = egui::Rect::from_min_size(pos2(right - g.size().x - 10.0, rect.center().y - 8.0), vec2(g.size().x + 10.0, 16.0));
        ui.painter().rect_filled(r, 8.0, tk.accent_soft);
        ui.painter().galley(r.center() - g.size() / 2.0, g, tk.icon_accent);
        right = r.left() - 6.0;
    }
    let name_x = ir.right() + 6.0;
    let name = ui.painter().layout_no_wrap(p.name.clone(), theme::body(), tk.text);
    let name_w = name.size().x.min((right - name_x).max(0.0));
    ui.painter().with_clip_rect(egui::Rect::from_x_y_ranges(name_x..=right, rect.y_range())).galley(
        pos2(name_x, rect.center().y - name.size().y / 2.0),
        name,
        tk.text,
    );
    let kind_x = name_x + name_w + 8.0;
    if kind_x < right {
        let g = ui.painter().layout_no_wrap(p.kind.clone(), theme::small(), tk.text_faint);
        ui.painter().with_clip_rect(egui::Rect::from_x_y_ranges(kind_x..=right, rect.y_range())).galley(
            pos2(kind_x, rect.center().y - g.size().y / 2.0),
            g,
            tk.text_faint,
        );
    }
    crate::control::record_widget(ui, &resp, &p.name, "treeitem", Some(selected));
    let tip = match (p.join, p.on.as_deref()) {
        ("blend", Some(on)) => format!("{}, {} {on}", p.kind, t("blended into")),
        ("weld", Some(on)) => format!("{}, {} {on}", p.kind, t("welded into")),
        (_, Some(on)) => format!("{}, {} {on}", p.kind, t("on")),
        _ => p.kind.clone(),
    };
    resp.on_hover_text(tip)
}

fn note(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).font(theme::small()).color(color)).wrap());
}

/// An error at its spec path.
fn issue(ui: &mut egui::Ui, e: &SpecIssue) {
    let tk = Tokens::get();
    let text = match (e.path.is_empty(), e.line) {
        (true, _) => e.message.clone(),
        (false, Some(l)) => format!("{} ({} {}): {}", e.path, t("line"), l + 1, e.message),
        (false, None) => format!("{}: {}", e.path, e.message),
    };
    note(ui, &text, tk.health_error);
}

/// The whole spec: what it is and what the tab can do.
fn whole_fields(app: &PixelApp, ui: &mut egui::Ui, root: ObjectKey) {
    let tk = Tokens::get();
    let Some(record) = app.doc().spec(root) else { return };
    let source = &record.source;
    panel(ui, "spec-whole", t("Spec"), true, |ui| {
        if let Some(c) = source.get("category").and_then(Value::as_str) {
            info(ui, t("Category"), c);
        }
        info(ui, t("Seed"), source.get("seed").and_then(Value::as_u64).unwrap_or(0).to_string());
        let n = |k: &str| source.get(k).and_then(Value::as_array).map_or(0, Vec::len);
        info(ui, t("Constraints"), n("constraints").to_string());
        info(ui, t("Expectations"), n("expect").to_string());
        note(ui, t("Pick a part to edit its shape, anchor and join. Edit the source below for anything else."), tk.text_faint);
    });
}

/// The selected part's fields.
fn part_fields(app: &mut PixelApp, ui: &mut egui::Ui, p: &PartInfo, parts: &[PartInfo], src: &Value, out: &mut Out) {
    let tk = Tokens::get();
    let root = defs(app);
    panel(ui, "spec-part", &format!("{} {}", t("Part"), p.name), true, |ui| {
        info(ui, t("Shape"), &p.kind);
        // A feature's parameters are its shape: first.
        if p.key == "feature" {
            shape_form(app, ui, p, src, out);
        }
        // Placement next: where the part sits is what specs are about.
        if p.key == "mirror" {
            info(ui, t("Join"), format!("{} ({})", t(p.join), t("like its source")));
            note(ui, t("A mirror is placed and joined like its source."), tk.text_faint);
        } else {
            anchor_fields(app, ui, p, parts, src, &root, out);
            join_fields(ui, src, &root, out);
        }
        if p.key != "feature" {
            shape_form(app, ui, p, src, out);
        }
        transform_fields(ui, src, &root, out);
        tags_field(ui, p, src, out);
    });
    deform_fields(ui, p, src, &root, out);
}

/// The part's handles and pins (its `deform` block): each handle's point
/// and offset, the pins, and the radius. Pulling handles in the viewport
/// (the Handles tool, H) adds to them.
fn deform_fields(ui: &mut egui::Ui, p: &PartInfo, src: &Value, root: &Value, out: &mut Out) {
    let tk = Tokens::get();
    let deform = src.get("deform").cloned().unwrap_or(Value::Null);
    let handles = deform.get("handles").and_then(Value::as_array).cloned().unwrap_or_default();
    let pins = deform.get("pins").and_then(Value::as_array).cloned().unwrap_or_default();
    // The block with one change; without handles it goes (pins alone do nothing).
    let with = |key: &str, v: Value| -> Value {
        let mut d = deform.as_object().cloned().unwrap_or_default();
        d.insert(key.to_owned(), v);
        if d.get("handles").and_then(Value::as_array).is_none_or(Vec::is_empty) { Value::Null } else { Value::Object(d) }
    };
    let point = |v: &Value| match v {
        Value::String(n) => n.clone(),
        v => v.as_array().map(|a| a.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
    };
    panel(ui, "spec-deform", &format!("{} ({})", t("Deform"), handles.len()), !handles.is_empty(), |ui| {
        if handles.is_empty() {
            note(
                ui,
                t("No handles yet. Take the Handles tool (H) and drag the part's surface: each pull is kept here, in part space."),
                tk.text_faint,
            );
            return;
        }
        let schema = json!({ "type": "object", "properties": { "by": { "type": "array", "items": { "$ref": "#/$defs/Length" }, "minItems": 3, "maxItems": 3,
            "description": t("How far the point moves, in part space") } } });
        for (i, h) in handles.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} {}: {}", t("Handle"), i + 1, point(h.get("move").unwrap_or(&Value::Null))))
                        .font(theme::small())
                        .color(tk.text_dim),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let r = ui.small_button(t("Remove"));
                    crate::control::record_widget(ui, &r, &format!("{} {}", t("Remove handle"), i + 1), "button", None);
                    if r.clicked() {
                        let mut list = handles.clone();
                        list.remove(i);
                        out.patch = Some((json!({ "deform": with("handles", Value::Array(list)) }), t("Delete handle").to_owned(), false));
                    }
                });
            });
            let before = form_value(&schema, root, h);
            let mut v = before.clone();
            let o = schema_form::form(ui, egui::Id::new(("spec-handle", &p.name, i)), &schema, root, &mut v, &[]);
            out.dragging |= o.dragging;
            if o.changed {
                let mut list = handles.clone();
                list[i]["by"] = v["by"].clone();
                out.patch = Some((json!({ "deform": with("handles", Value::Array(list)) }), t("Handle offset").to_owned(), o.dragging));
            }
        }
        for (i, q) in pins.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}: {}", t("Pin"), i + 1, point(q))).font(theme::small()).color(tk.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t("Remove")).clicked() {
                        let mut list = pins.clone();
                        list.remove(i);
                        out.patch = Some((json!({ "deform": with("pins", Value::Array(list)) }), t("Delete pin").to_owned(), false));
                    }
                });
            });
        }
        let rs = json!({ "type": "object", "properties": { "radius": { "anyOf": [{ "$ref": "#/$defs/Length" }, { "type": "null" }],
            "description": t("How far along the surface each handle reaches; 40% of the part's diagonal when off") } } });
        let before = form_value(&rs, root, &deform);
        let mut v = before.clone();
        let o = schema_form::form(ui, egui::Id::new(("spec-deform-radius", &p.name)), &rs, root, &mut v, &[]);
        out.dragging |= o.dragging;
        if o.changed {
            out.patch = Some((json!({ "deform": with("radius", v["radius"].clone()) }), t("Handle radius").to_owned(), o.dragging));
        }
    });
}

/// A feature part's parameters, with the widgets of the Features panel
/// (from the template's `ParamInfo`); an edit patches `params`.
fn feature_form(app: &PixelApp, ui: &mut egui::Ui, p: &PartInfo, src: &Value, out: &mut Out) {
    let tk = Tokens::get();
    let Some(id) = src.get("feature").and_then(Value::as_str) else { return };
    let Some(info) = crate::features::info(id) else {
        note(ui, &format!("{} `{id}`", t("there is no feature")), tk.health_error);
        return;
    };
    note(ui, info.summary, tk.text_faint);
    let root = defs(app);
    let schema = crate::features::schema(&info);
    let params = src.get("params").cloned().unwrap_or(Value::Null);
    let before = form_value(&schema, &root, &params);
    let mut v = before.clone();
    let o = schema_form::form(ui, egui::Id::new(("spec-feature", &p.name)), &schema, &root, &mut v, &[]);
    out.dragging |= o.dragging;
    if o.changed
        && let Value::Object(mut m) = v
    {
        crate::features::clamp(&info, &mut m);
        let fields = changed(&before, &Value::Object(m));
        out.patch = Some((json!({ "params": fields }), t("Feature parameters").to_owned(), o.dragging));
    }
}

/// The fields of a primitive or generator part, as the op's form.
fn shape_form(app: &PixelApp, ui: &mut egui::Ui, p: &PartInfo, src: &Value, out: &mut Out) {
    if p.key == "feature" {
        return feature_form(app, ui, p, src, out);
    }
    let (op, key, value) = match p.key {
        "primitive" => (format!("prim.{}", p.kind), p.kind.as_str(), src.get("primitive").and_then(|v| v.get(&p.kind))),
        "lathe" | "sweep" | "loft" | "extrude" | "pipe" => {
            let op = p3d_spec::model::GENERATORS.iter().find(|(k, _)| *k == p.key).map_or("", |(_, op)| op);
            (op.to_owned(), p.key, src.get(p.key))
        }
        _ => return,
    };
    let Ok(def) = app.session.registry().get(&op) else { return };
    let schema = def.params_schema();
    let Some(value) = value.filter(|v| v.is_object()) else { return };
    let before = form_value(&schema, &schema, value);
    let mut v = before.clone();
    let hide = ["name", "parent", "position", "rotation", "target", "advanced", "material"];
    let o = schema_form::form(ui, egui::Id::new(("spec-shape", &p.name)), &schema, &schema, &mut v, &hide);
    out.dragging |= o.dragging;
    if o.changed {
        let fields = changed(&before, &v);
        let patch = if p.key == "primitive" { json!({ "primitive": { key: fields } }) } else { json!({ key: fields }) };
        out.patch = Some((patch, t("Shape").to_owned(), o.dragging));
    }
}

/// On (a part picker), landmark (a dropdown of the parent's landmarks and
/// a viewport pick), offset and orient.
fn anchor_fields(app: &mut PixelApp, ui: &mut egui::Ui, p: &PartInfo, parts: &[PartInfo], src: &Value, root: &Value, out: &mut Out) {
    let anchor = src.get("anchor").cloned().unwrap_or(Value::Null);
    let on = anchor.get("on").and_then(Value::as_str).map(str::to_owned);
    prop_row(ui, t("On"), t("The part this one sits on; its landmarks place it"), |ui| {
        let text = on.clone().unwrap_or_else(|| t("nothing").to_owned());
        egui::ComboBox::from_id_salt("spec-on").selected_text(text).width(ui.available_width()).show_ui(ui, |ui| {
            if ui.selectable_label(on.is_none(), t("nothing")).clicked() {
                out.patch = Some((json!({ "anchor": null }), t("Anchor").to_owned(), false));
            }
            for q in parts.iter().filter(|q| q.name != p.name) {
                if ui.selectable_label(on.as_deref() == Some(q.name.as_str()), &q.name).clicked() && on.as_deref() != Some(q.name.as_str()) {
                    out.anchor_on = Some(q.name.clone());
                }
            }
        });
    });
    let Some(on) = on else { return };
    let landmarks = app.spec_landmarks_of(&on);
    let current = anchor.get("landmark").and_then(Value::as_str);
    let placed_by = ["param", "ray", "nearest"].into_iter().find(|k| anchor.get(*k).is_some());
    let picking = app.spec.picking;
    prop_row(ui, t("Landmark"), t("Where on the parent the part sits"), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let text = match (current, placed_by) {
                (Some(l), _) => l.to_owned(),
                (None, Some(k)) => format!("({k})"),
                _ => String::new(),
            };
            let w = (ui.available_width() - 26.0).max(40.0);
            egui::ComboBox::from_id_salt("spec-landmark").selected_text(text).width(w).height(320.0).show_ui(ui, |ui| {
                for l in landmarks.iter() {
                    if ui.selectable_label(current == Some(l.name.as_str()), &l.name).clicked() {
                        out.landmark = Some(l.name.clone());
                    }
                }
            });
            let b = icons::button(ui, Icon::Landmark, 22.0, t("Pick a landmark in the viewport"), picking);
            crate::control::record_widget(ui, &b, t("Pick landmark"), "button", Some(picking));
            if b.clicked() {
                out.command = Some(("spec.pick_landmark", json!({ "on": !picking })));
            }
        });
    });
    if picking {
        note(ui, &format!("{} {on}.", t("Click a landmark in the viewport to place the part on")), Tokens::get().accent);
    }
    let schema = json!({ "type": "object", "properties": {
        "offset": { "type": "array", "items": { "$ref": "#/$defs/Length" }, "minItems": 3, "maxItems": 3,
            "description": t("Moves the part from its anchor point, along its axes") },
        "orient": { "oneOf": [
            { "const": "parent", "description": t("Keep the parent's axes") },
            { "const": "normal", "description": t("Turn +Y along the surface normal") }],
            "description": t("How the part is turned on its parent") },
    } });
    let mut src_vals = form_value(&schema, root, &anchor);
    if src_vals.get("offset").is_none() {
        src_vals["offset"] = json!([0.0, 0.0, 0.0]);
    }
    if src_vals.get("orient").is_none() {
        src_vals["orient"] = json!("parent");
    }
    let mut v = src_vals.clone();
    let o = schema_form::form(ui, egui::Id::new(("spec-anchor", &p.name)), &schema, root, &mut v, &[]);
    out.dragging |= o.dragging;
    if o.changed {
        let mut fields = changed(&src_vals, &v);
        if fields.get("orient").and_then(Value::as_str) == Some("parent") {
            fields["orient"] = Value::Null;
        }
        out.patch = Some((json!({ "anchor": fields }), t("Anchor offset").to_owned(), o.dragging));
    }
}

/// The join and the blend size of a blend.
fn join_fields(ui: &mut egui::Ui, src: &Value, root: &Value, out: &mut Out) {
    let join = src.get("join").and_then(Value::as_str).unwrap_or("separate");
    let mut props = json!({
        "join": { "oneOf": [
            { "const": "separate", "description": t("Its own mesh object") },
            { "const": "blend", "description": t("Merged into the parent's surface with a fillet (SDF parts)") },
            { "const": "weld", "description": t("Unioned into the parent's mesh with a crisp seam (no object of its own)") },
            { "const": "decal", "description": t("Texture-only detail baked onto the parent later") }],
            "description": t("How the part joins the part it sits on") },
    });
    if join == "blend" {
        props["blend"] = json!({ "anyOf": [{ "$ref": "#/$defs/Length" }, { "type": "null" }], "description": t("Fillet size of the blend") });
    }
    let schema = json!({ "type": "object", "properties": props });
    let mut before = form_value(&schema, root, src);
    before["join"] = json!(join);
    let mut v = before.clone();
    let o = schema_form::form(ui, egui::Id::new("spec-join"), &schema, root, &mut v, &[]);
    out.dragging |= o.dragging;
    if o.changed {
        let mut fields = changed(&before, &v);
        match fields.get("join").and_then(Value::as_str) {
            Some("separate") => {
                fields["join"] = Value::Null;
                fields["blend"] = Value::Null;
            }
            Some("decal" | "weld") => fields["blend"] = Value::Null,
            _ => {}
        }
        out.patch = Some((fields, t("Join").to_owned(), o.dragging));
    }
}

/// Position (without an anchor), rotation and scale.
fn transform_fields(ui: &mut egui::Ui, src: &Value, root: &Value, out: &mut Out) {
    let anchored = src.get("anchor").is_some_and(|a| !a.is_null());
    let mut props = json!({
        "rotate": { "type": "array", "items": { "$ref": "#/$defs/Angle" }, "minItems": 3, "maxItems": 3,
            "description": t("Degrees about X, then Y, then Z") },
        "scale": { "type": "array", "items": { "type": "number", "minimum": 0.0 }, "minItems": 3, "maxItems": 3,
            "description": t("Scale along X, Y and Z") },
    });
    if !anchored {
        props["position"] = json!({ "type": "array", "items": { "$ref": "#/$defs/Length" }, "minItems": 3, "maxItems": 3,
            "description": t("Position in the root's space") });
    }
    let schema = json!({ "type": "object", "properties": props });
    let tr = src.get("transform").cloned().unwrap_or(Value::Null);
    let mut before = form_value(&schema, root, &tr);
    for (k, d) in [("rotate", json!([0.0, 0.0, 0.0])), ("scale", json!([1.0, 1.0, 1.0])), ("position", json!([0.0, 0.0, 0.0]))] {
        if before.get(k).is_none() && (k != "position" || !anchored) {
            before[k] = d;
        }
    }
    let mut v = before.clone();
    let o = schema_form::form(ui, egui::Id::new("spec-transform"), &schema, root, &mut v, &[]);
    out.dragging |= o.dragging;
    if o.changed {
        out.patch = Some((json!({ "transform": changed(&before, &v) }), t("Part transform").to_owned(), o.dragging));
    }
}

/// Tags, comma separated; applied when the field loses focus.
fn tags_field(ui: &mut egui::Ui, p: &PartInfo, src: &Value, out: &mut Out) {
    let tags: Vec<&str> = src.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let joined = tags.join(", ");
    let id = egui::Id::new(("spec-tags", &p.name));
    prop_row(ui, t("Tags"), t("Words agents and exporters filter by, comma separated"), |ui| {
        let mut text: String = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).filter(|(s, _)| *s == joined).map_or(joined.clone(), |(_, e)| e);
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width()).hint_text(t("none")));
        crate::control::record_widget(ui, &r, t("Tags"), "textbox", None);
        ui.data_mut(|d| d.insert_temp(id, (joined.clone(), text.clone())));
        if r.lost_focus() {
            let new: Vec<String> = text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
            if new.iter().map(String::as_str).ne(tags.iter().copied()) {
                let v = if new.is_empty() { Value::Null } else { json!(new) };
                out.patch = Some((json!({ "tags": v }), t("Tags").to_owned(), false));
            }
        }
    });
}

/// The raw source: a code editor with the error's line marked, Apply and
/// Revert.
fn json_view(app: &mut PixelApp, ui: &mut egui::Ui, out: &mut Out) {
    let tk = Tokens::get();
    let edited = app.spec.json_edited;
    panel(ui, "spec-json", t("Source (JSON)"), false, |ui| {
        let shown = app.spec.json_error.clone().or_else(|| app.spec.error.clone().filter(|_| !edited));
        let mark = shown.as_ref().and_then(|e| e.line);
        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = egui::text::LayoutJob::default();
            for (n, line) in buf.as_str().split_inclusive('\n').enumerate() {
                let mut fmt = egui::TextFormat::simple(theme::mono(), tk.text);
                if Some(n) == mark {
                    fmt.background = tk.health_error.gamma_multiply(0.22);
                    fmt.underline = Stroke::new(1.0, tk.health_error);
                }
                job.append(line, 0.0, fmt);
            }
            job.wrap.max_width = wrap;
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let r =
            ui.add(egui::TextEdit::multiline(&mut app.spec.json).code_editor().desired_width(f32::INFINITY).desired_rows(14).layouter(&mut layouter));
        crate::control::record_widget(ui, &r, t("Spec source"), "textbox", None);
        if r.changed() {
            app.spec.json_edited = true;
            app.spec.json_error = None;
        }
        if let Some(e) = &shown {
            issue(ui, e);
        }
        ui.horizontal(|ui| {
            let apply = ui.add_enabled(app.spec.json_edited, egui::Button::new(t("Apply")));
            crate::control::record_widget(ui, &apply, t("Apply source"), "button", None);
            if apply.on_hover_text(t("Check the source and rebuild the parts that changed")).clicked() {
                out.apply_json = true;
            }
            let revert = ui.add_enabled(app.spec.json_edited, egui::Button::new(t("Revert")));
            if revert.on_hover_text(t("Drop the edits and show the spec as it is")).clicked() {
                out.revert_json = true;
            }
        });
    });
}

/// Applies what the tab asked for, in one place.
fn apply(app: &mut PixelApp, root: ObjectKey, out: Out) {
    if out.hovered.is_some() {
        app.spec.hover_next = out.hovered;
    }
    if let Some(sel) = out.select
        && let Err(e) = app.spec_select_part(sel.as_deref())
    {
        app.set_status(e);
    }
    if let Some((id, params)) = out.command
        && let Err(e) = app.spec_command(id, &params)
    {
        app.set_status(e);
    }
    if out.apply_json {
        let text = app.spec.json.clone();
        if let Err(e) = app.spec_apply_json(&text) {
            app.set_status(e);
        }
    }
    if out.revert_json {
        app.spec.json_edited = false;
        app.spec.json_error = None;
        app.spec.json.clear();
        app.spec_follow();
    }
    if let Some(name) = out.landmark
        && let Err(e) = app.spec_set_landmark(&name)
    {
        app.set_status(e);
    }
    if let Some(on) = out.anchor_on {
        // Keep the landmark when the new parent has one of that name.
        let part = app.spec.part.clone().unwrap_or_default();
        let old = part_source(app.doc(), root, &part).and_then(|s| s.get("anchor")?.get("landmark")?.as_str().map(str::to_owned));
        let names = app.spec_landmarks_of(&on);
        let landmark = old.filter(|l| names.iter().any(|n| n.name == *l)).unwrap_or_else(|| "top".to_owned());
        let anchor = json!({ "on": on, "landmark": landmark, "param": null, "ray": null, "nearest": null });
        if let Err(e) = app.spec_patch_part(json!({ "anchor": anchor }), t("Anchor")) {
            app.set_status(e);
        }
    }
    let Some(part) = app.spec.part.clone() else { return };
    match out.patch {
        Some((fields, label, dragging)) => {
            // Show the change at once; the source follows when it commits.
            let mut draft = match app.spec.draft.take() {
                Some((r, p, v)) if r == root && p == part => v,
                _ => part_source(app.doc(), root, &part).unwrap_or(Value::Null),
            };
            p3d_spec::patch::merge(&mut draft, &fields);
            app.spec.draft = Some((root, part.clone(), draft));
            let mut pending = app.spec.pending.take().map_or(Value::Object(Map::new()), |(v, _)| v);
            accumulate(&mut pending, &fields);
            if dragging {
                // A live preview of the drag, compiled in the background.
                app.spec_preview(json!({ "parts": { part.as_str(): pending.clone() } }));
                app.spec.pending = Some((pending, label));
            } else if let Err(e) = app.spec_patch_part(pending, &label) {
                app.set_status(e);
            }
        }
        None if !out.dragging => {
            // A drag ended: commit what it changed.
            if let Some((pending, label)) = app.spec.pending.take()
                && let Err(e) = app.spec_patch_part(pending, &label)
            {
                app.set_status(e);
            }
        }
        None => {}
    }
}

/// Adds a merge patch to the ones gathered during a drag: objects merge
/// key by key, anything else (`null` too) replaces.
///
/// Complexity: O(size of `patch`).
fn accumulate(acc: &mut Value, patch: &Value) {
    match (acc.as_object_mut(), patch.as_object()) {
        (Some(a), Some(p)) => {
            for (k, v) in p {
                match a.get_mut(k) {
                    Some(x) if x.is_object() && v.is_object() => accumulate(x, v),
                    _ => {
                        a.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        _ => *acc = patch.clone(),
    }
}
