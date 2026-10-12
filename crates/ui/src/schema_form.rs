//! Forms generated from op parameter schemas (the JSON Schema schemars
//! derives from each op's `Params`): one schema-to-widget mapping for every
//! op, built-in or from an addon (project spec 20.6). Lengths get a meter
//! field, angles a degree field, enums a dropdown, optional values a
//! checkbox, nested settings a collapsible section and position-like
//! triples three X, Y, Z fields.

use egui::{Color32, DragValue, RichText};
use serde_json::{Map, Value, json};

use crate::theme::{self, Tokens};

/// What a form did this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormOutput {
    /// A value changed.
    pub changed: bool,
    /// A value is being dragged (changes will keep coming).
    pub dragging: bool,
}

impl FormOutput {
    fn merge(&mut self, o: FormOutput) {
        self.changed |= o.changed;
        self.dragging |= o.dragging;
    }
}

/// The widget a schema maps to.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Length,
    Angle,
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Integer {
        min: Option<f64>,
        max: Option<f64>,
    },
    Bool,
    /// Allowed values with their descriptions.
    Enum(Vec<(String, String)>),
    Text,
    /// Three values of one kind (positions, rotations, segment counts).
    Triple(Box<Kind>),
    /// An object with properties (resolved schema).
    Object(Value),
    /// A value that may be null (auto, off).
    Optional(Box<Kind>),
    /// Anything else, shown as JSON.
    Other,
}

/// Follows `#/$defs/Name` references.
fn resolve<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    let mut s = schema;
    for _ in 0..8 {
        match s.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")) {
            Some(name) => match root.get("$defs").and_then(|d| d.get(name)) {
                Some(next) => s = next,
                None => break,
            },
            None => break,
        }
    }
    s
}

fn ref_name(schema: &Value) -> Option<&str> {
    schema.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/"))
}

/// The widget kind of a property schema.
pub fn kind_of(schema: &Value, root: &Value) -> Kind {
    match ref_name(schema) {
        Some("Length") => return Kind::Length,
        Some("Angle") => return Kind::Angle,
        _ => {}
    }
    if let Some(any) = schema.get("anyOf").and_then(Value::as_array) {
        let non_null: Vec<&Value> = any.iter().filter(|s| s.get("type").and_then(Value::as_str) != Some("null")).collect();
        if non_null.len() == 1 && any.len() == 2 {
            return Kind::Optional(Box::new(kind_of(non_null[0], root)));
        }
        // Untagged alternatives (a uniform scale or three factors): the
        // triple covers both.
        if non_null.len() == any.len()
            && let Some(triple) = non_null.iter().map(|s| kind_of(s, root)).find(|k| matches!(k, Kind::Triple(_)))
        {
            return triple;
        }
    }
    let s = resolve(schema, root);
    if !std::ptr::eq(s, schema) {
        return kind_of(s, root);
    }
    if let Some(one) = s.get("oneOf").and_then(Value::as_array)
        && one.iter().all(|o| o.get("const").is_some())
    {
        return Kind::Enum(
            one.iter()
                .filter_map(|o| Some((o.get("const")?.as_str()?.to_owned(), o.get("description").and_then(Value::as_str).unwrap_or("").to_owned())))
                .collect(),
        );
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        return Kind::Enum(e.iter().filter_map(|v| v.as_str().map(|x| (x.to_owned(), String::new()))).collect());
    }
    let min = s.get("minimum").and_then(Value::as_f64);
    let max = s.get("maximum").and_then(Value::as_f64);
    match s.get("type") {
        Some(Value::Array(types)) => {
            let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            let inner: Vec<&str> = names.iter().copied().filter(|t| *t != "null").collect();
            match (inner.as_slice(), names.contains(&"null")) {
                (["string"], true) => Kind::Optional(Box::new(Kind::Text)),
                (["integer"], true) => Kind::Optional(Box::new(Kind::Integer { min, max })),
                (["number"], true) => Kind::Optional(Box::new(Kind::Number { min, max })),
                (["boolean"], true) => Kind::Optional(Box::new(Kind::Bool)),
                ([single], true) => {
                    // The same schema without null, for arrays and objects.
                    let mut inner = s.clone();
                    if let Some(o) = inner.as_object_mut() {
                        o.insert("type".into(), json!(single));
                    }
                    Kind::Optional(Box::new(kind_of(&inner, root)))
                }
                _ => Kind::Other,
            }
        }
        Some(Value::String(t)) => match t.as_str() {
            "integer" => Kind::Integer { min, max },
            "number" => Kind::Number { min, max },
            "boolean" => Kind::Bool,
            "string" => Kind::Text,
            "array" => {
                let n = (s.get("minItems").and_then(Value::as_u64), s.get("maxItems").and_then(Value::as_u64));
                match (n, s.get("items")) {
                    ((Some(3), Some(3)), Some(items)) => Kind::Triple(Box::new(kind_of(items, root))),
                    _ => Kind::Other,
                }
            }
            "object" if s.get("properties").is_some() => Kind::Object(s.clone()),
            _ => Kind::Other,
        },
        _ => Kind::Other,
    }
}

/// `height_segments` as "Height segments".
pub fn humanize(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Where a field goes: sizes first, then counts and choices, placement,
/// names, and rarely needed settings last.
fn order_key(name: &str, kind: &Kind) -> (u8, usize, String) {
    const PREFERRED: &[&str] = &[
        "target",
        "width",
        "height",
        "depth",
        "radius",
        "inner_radius",
        "size",
        "segments",
        "cap",
        "bevel",
        "origin",
        "position",
        "rotation",
        "scale",
        "relative",
    ];
    if name == "advanced" {
        return (9, 0, String::new());
    }
    if let Some(i) = PREFERRED.iter().position(|p| *p == name) {
        return (0, i, String::new());
    }
    let group = match kind {
        Kind::Length | Kind::Angle | Kind::Number { .. } | Kind::Integer { .. } => 1,
        Kind::Enum(_) | Kind::Bool => 2,
        Kind::Optional(_) | Kind::Object(_) | Kind::Triple(_) => 3,
        Kind::Text | Kind::Other => 4,
    };
    (group, 0, name.to_owned())
}

/// A starting value for a kind (when an optional value is switched on).
fn default_for(kind: &Kind, schema: &Value, root: &Value) -> Value {
    if let Some(d) = schema.get("default").filter(|d| !d.is_null()) {
        return d.clone();
    }
    match kind {
        Kind::Length => json!(0.01),
        Kind::Angle | Kind::Number { .. } => json!(0.0),
        Kind::Integer { min, .. } => json!(min.unwrap_or(1.0).max(1.0) as i64),
        Kind::Bool => json!(false),
        Kind::Enum(options) => options.first().map_or(Value::Null, |(v, _)| json!(v)),
        Kind::Text => json!(""),
        Kind::Triple(inner) => {
            json!([default_for(inner, &Value::Null, root), default_for(inner, &Value::Null, root), default_for(inner, &Value::Null, root)])
        }
        Kind::Object(s) => {
            let mut m = Map::new();
            if let Some(props) = s.get("properties").and_then(Value::as_object) {
                for (k, ps) in props {
                    m.insert(k.clone(), default_for(&kind_of(ps, root), ps, root));
                }
            }
            Value::Object(m)
        }
        Kind::Optional(_) | Kind::Other => Value::Null,
    }
}

/// How a form presents values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormOptions {
    /// Show optional values as always set (object properties edited through
    /// an op whose fields are optional, such as a transform).
    pub optionals_always_on: bool,
}

/// Lays out a form for `value` (an object) with the properties of `schema`;
/// `root` holds the `$defs`. Properties named in `hide` are skipped.
///
/// Complexity: O(properties) per frame.
pub fn form(ui: &mut egui::Ui, id: egui::Id, schema: &Value, root: &Value, value: &mut Value, hide: &[&str]) -> FormOutput {
    form_with(ui, id, schema, root, value, hide, FormOptions::default())
}

/// [`form`] with presentation options.
///
/// Complexity: O(properties) per frame.
pub fn form_with(ui: &mut egui::Ui, id: egui::Id, schema: &Value, root: &Value, value: &mut Value, hide: &[&str], opts: FormOptions) -> FormOutput {
    let mut out = FormOutput::default();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else { return out };
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    let mut fields: Vec<(&String, &Value, Kind)> =
        props.iter().filter(|(k, _)| !hide.contains(&k.as_str())).map(|(k, s)| (k, s, kind_of(s, root))).collect();
    fields.sort_by_key(|(k, _, kind)| order_key(k, kind));
    for (name, prop, kind) in fields {
        let desc = prop.get("description").and_then(Value::as_str).unwrap_or("").to_owned();
        let Some(obj) = value.as_object_mut() else { return out };
        let v = obj.entry(name.clone()).or_insert(Value::Null);
        out.merge(field(ui, id.with(name.as_str()), &humanize(name), &desc, &kind, prop, root, v, opts));
    }
    out
}

/// A row: label on the left (right-aligned), widget on the right.
fn row<R>(ui: &mut egui::Ui, label: &str, desc: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let tk = Tokens::get();
    ui.horizontal(|ui| {
        let w = (ui.available_width() * 0.4).clamp(70.0, 130.0);
        ui.allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let l = ui.add(egui::Label::new(RichText::new(label).color(tk.text_dim)).truncate());
            if !desc.is_empty() {
                l.on_hover_text(desc);
            }
        });
        add(ui)
    })
    .inner
}

fn drag(ui: &mut egui::Ui, v: &mut f64, kind: &Kind, width: f32) -> egui::Response {
    let length_speed = (v.abs() * 0.005).max(0.001);
    let mut d = match kind {
        Kind::Length => DragValue::new(v).speed(length_speed).suffix(" m").max_decimals(4),
        Kind::Angle => DragValue::new(v).speed(0.5).suffix("\u{b0}").max_decimals(2),
        Kind::Integer { .. } => DragValue::new(v).speed(0.1).fixed_decimals(0),
        _ => DragValue::new(v).speed(0.01).max_decimals(4),
    };
    if let Kind::Integer { min, max } | Kind::Number { min, max } = kind {
        d = d.range(min.unwrap_or(f64::NEG_INFINITY)..=max.unwrap_or(f64::INFINITY));
    }
    ui.add_sized([width, 20.0], d)
}

#[allow(clippy::too_many_arguments)] // One recursive step of the mapping; each argument is distinct.
fn field(
    ui: &mut egui::Ui,
    id: egui::Id,
    label: &str,
    desc: &str,
    kind: &Kind,
    schema: &Value,
    root: &Value,
    v: &mut Value,
    opts: FormOptions,
) -> FormOutput {
    let mut out = FormOutput::default();
    let tk = Tokens::get();
    if let (Kind::Optional(inner), true) = (kind, opts.optionals_always_on) {
        if v.is_null() {
            *v = default_for(inner, schema, root);
        }
        return field(ui, id, label, desc, inner, schema, root, v, opts);
    }
    match kind {
        Kind::Length | Kind::Angle | Kind::Number { .. } | Kind::Integer { .. } => {
            let mut x = v.as_f64().unwrap_or(0.0);
            let r = row(ui, label, desc, |ui| drag(ui, &mut x, kind, ui.available_width().max(60.0)));
            if r.changed() {
                *v = if matches!(kind, Kind::Integer { .. }) { json!(x.round() as i64) } else { json!(x) };
                out.changed = true;
            }
            out.dragging = r.dragged();
        }
        Kind::Bool => {
            let mut b = v.as_bool().unwrap_or(false);
            if row(ui, label, desc, |ui| ui.checkbox(&mut b, "")).changed() {
                *v = json!(b);
                out.changed = true;
            }
        }
        Kind::Enum(options) => {
            let current = v.as_str().unwrap_or("").to_owned();
            let mut chosen = current.clone();
            row(ui, label, desc, |ui| {
                egui::ComboBox::from_id_salt(id).width(ui.available_width().max(80.0)).selected_text(humanize(&current)).show_ui(ui, |ui| {
                    for (opt, d) in options {
                        let r = ui.selectable_value(&mut chosen, opt.clone(), humanize(opt));
                        if !d.is_empty() {
                            r.on_hover_text(d);
                        }
                    }
                });
            });
            if chosen != current {
                *v = json!(chosen);
                out.changed = true;
            }
        }
        Kind::Text => {
            let mut s = v.as_str().unwrap_or("").to_owned();
            let r = row(ui, label, desc, |ui| ui.add_sized([ui.available_width().max(80.0), 20.0], egui::TextEdit::singleline(&mut s)));
            if r.changed() {
                *v = json!(s);
                out.changed = true;
            }
        }
        Kind::Triple(inner) => {
            // -0 shows as 0; float noise below a micro unit as 0.
            let clean = |x: f64| if x.abs() < 1e-9 { 0.0 } else { x };
            let mut xs: Vec<f64> = v.as_array().map(|a| a.iter().map(|x| clean(x.as_f64().unwrap_or(0.0))).collect()).unwrap_or_default();
            xs.resize(3, 0.0);
            let axes = [tk.axis_x, tk.axis_y, tk.axis_z];
            let mut changed = false;
            let mut dragging = false;
            // Stacked like Blender: the label on the X row, one field per axis.
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, x) in xs.iter_mut().enumerate() {
                let r = row(ui, if i == 0 { label } else { "" }, desc, |ui| {
                    ui.label(RichText::new(["X", "Y", "Z"][i]).color(axes[i]).font(theme::small()));
                    drag(ui, x, inner, ui.available_width().max(40.0))
                });
                changed |= r.changed();
                dragging |= r.dragged();
            }
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.add_space(3.0);
            if changed {
                let ints = matches!(**inner, Kind::Integer { .. });
                *v = Value::Array(xs.iter().map(|x| if ints { json!(x.round() as i64) } else { json!(x) }).collect());
                out.changed = true;
            }
            out.dragging = dragging;
        }
        Kind::Optional(inner) => {
            let mut on = !v.is_null();
            let toggle_label = match **inner {
                Kind::Integer { .. } | Kind::Number { .. } | Kind::Length => "Auto",
                _ => "",
            };
            // Numbers: a checked "Auto" means null; other values: checked means set.
            let auto = !toggle_label.is_empty();
            let mut check = if auto { !on } else { on };
            match &**inner {
                Kind::Object(s) => {
                    let toggled = ui.horizontal(|ui| ui.checkbox(&mut check, RichText::new(label).strong()).on_hover_text(desc).changed()).inner;
                    if toggled {
                        on = check;
                        *v = if on { default_for(inner, &Value::Null, root) } else { Value::Null };
                        out.changed = true;
                    }
                    if on {
                        ui.indent(id.with("sub"), |ui| out.merge(form_with(ui, id.with("form"), s, root, v, &[], opts)));
                    }
                }
                Kind::Text => {
                    let mut s = v.as_str().unwrap_or("").to_owned();
                    let r = row(ui, label, desc, |ui| {
                        ui.add_sized(
                            [ui.available_width().max(80.0), 20.0],
                            egui::TextEdit::singleline(&mut s).hint_text(crate::i18n::t("automatic")),
                        )
                    });
                    if r.changed() {
                        *v = if s.is_empty() { Value::Null } else { json!(s) };
                        out.changed = true;
                    }
                }
                Kind::Triple(_) | Kind::Enum(_) | Kind::Bool | Kind::Optional(_) | Kind::Other => {
                    // Set or leave unset (the op keeps the current value).
                    let toggled = ui.horizontal(|ui| ui.checkbox(&mut check, RichText::new(label)).on_hover_text(desc).changed()).inner;
                    if toggled {
                        on = check;
                        *v = if on { default_for(inner, &Value::Null, root) } else { Value::Null };
                        out.changed = true;
                    }
                    if on {
                        out.merge(field(ui, id.with("set"), "", desc, inner, schema, root, v, opts));
                    }
                }
                _ => {
                    let mut x = v.as_f64().unwrap_or_else(|| default_for(inner, schema, root).as_f64().unwrap_or(8.0));
                    let (toggled, r) = row(ui, label, desc, |ui| {
                        let t = ui.checkbox(&mut check, crate::i18n::t(toggle_label)).changed();
                        let r =
                            ui.add_enabled_ui(if auto { !check } else { check }, |ui| drag(ui, &mut x, inner, ui.available_width().max(50.0))).inner;
                        (t, r)
                    });
                    on = if auto { !check } else { check };
                    if toggled || (on && r.changed()) {
                        let ints = matches!(**inner, Kind::Integer { .. });
                        *v = if !on {
                            Value::Null
                        } else if ints {
                            json!(x.round() as i64)
                        } else {
                            json!(x)
                        };
                        out.changed = true;
                    }
                    out.dragging = r.dragged();
                }
            }
        }
        Kind::Object(s) => {
            egui::CollapsingHeader::new(RichText::new(label).color(tk.text_dim)).id_salt(id).default_open(false).show(ui, |ui| {
                out.merge(form_with(ui, id.with("form"), s, root, v, &[], opts));
            });
        }
        Kind::Other => {
            row(ui, label, desc, |ui| ui.label(RichText::new(v.to_string()).font(theme::mono()).color(Color32::GRAY)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(id: &str) -> Value {
        let r = p3d_ops::Registry::builtin();
        r.get(id).map(|o| o.params_schema()).unwrap_or(Value::Null)
    }

    #[test]
    fn op_schemas_map_to_widgets() {
        let s = schema("prim.box");
        let p = &s["properties"];
        assert_eq!(kind_of(&p["width"], &s), Kind::Length);
        assert!(matches!(kind_of(&p["origin"], &s), Kind::Enum(o) if o.iter().any(|(v, _)| v == "bottom")));
        assert!(matches!(kind_of(&p["bevel"], &s), Kind::Optional(k) if matches!(*k, Kind::Object(_))));
        assert!(matches!(kind_of(&p["position"], &s), Kind::Triple(k) if *k == Kind::Length));
        assert!(matches!(kind_of(&p["name"], &s), Kind::Optional(k) if *k == Kind::Text));
        assert!(matches!(kind_of(&p["advanced"], &s), Kind::Object(_)));
        let c = schema("prim.cylinder");
        assert!(matches!(kind_of(&c["properties"]["segments"], &c), Kind::Optional(k) if matches!(*k, Kind::Integer { .. })));
        let t = schema("scene.set_transform");
        assert!(matches!(kind_of(&t["properties"]["rotation"], &t), Kind::Optional(k) if matches!(*k, Kind::Triple(_))));
    }

    #[test]
    fn switching_on_an_optional_object_fills_its_defaults() {
        let s = schema("prim.box");
        let kind = kind_of(&s["properties"]["bevel"], &s);
        let Kind::Optional(inner) = kind else { return };
        let v = default_for(&inner, &Value::Null, &s);
        assert_eq!(v["segments"], json!(1));
        assert!(v["width"].as_f64().is_some_and(|w| w > 0.0));
    }

    #[test]
    fn fields_are_ordered_for_people() {
        let s = schema("prim.box");
        let props = s["properties"].as_object().cloned().unwrap_or_default();
        let mut names: Vec<(String, Kind)> = props.iter().map(|(k, v)| (k.clone(), kind_of(v, &s))).collect();
        names.sort_by_key(|(k, kind)| order_key(k, kind));
        let order: Vec<&str> = names.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order.first(), Some(&"width"));
        assert_eq!(order.last(), Some(&"advanced"));
        assert_eq!(humanize("height_segments"), "Height segments");
    }

    #[test]
    fn forms_lay_out_every_builtin_op() {
        let ctx = egui::Context::default();
        let r = p3d_ops::Registry::builtin();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for op in r.iter() {
                let s = op.params_schema();
                let mut v = op.default_params();
                let out = form(ui, egui::Id::new(op.id()), &s, &s, &mut v, &[]);
                assert!(!out.changed, "{} changed without input", op.id());
            }
        });
        out.textures_delta.clear();
    }
}
