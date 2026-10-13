//! Parametric feature templates (project spec 17.5, ADR 0012): details an
//! agent describes with semantic parameters instead of drawing, such as
//! ears, noses, eyes, horns, hands, bolts and vents. Organic features give
//! SDF items (they blend into SDF parents); designed ones give meshes whose
//! edge flow comes from their construction. Landmarks travel as one-vertex
//! groups `lm.<name>`.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.

use std::collections::BTreeMap;

use glam::Vec3;
use p3d_mesh::{ElementKind, Mesh};
use p3d_sdf::Item;
use serde_json::Value;

mod cable;
mod ear;
mod eye;
mod hand;
mod hard;
mod head;
mod horn;
mod nose;
mod outline;
mod tube;

/// What kind of value a parameter takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    /// Meters; accepts unit strings such as `"8cm"`.
    Length,
    /// Radians inside; accepts degrees as numbers or strings such as `"25deg"`.
    Angle,
    /// A plain number (shares, ratios, strengths).
    Ratio,
    /// A whole number.
    Count,
    /// One of the listed words.
    Choice(&'static [&'static str]),
}

/// One parameter of a feature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamInfo {
    pub name: &'static str,
    pub doc: &'static str,
    pub kind: ParamKind,
    /// Default value (for choices, the index of the default word; angles in
    /// degrees).
    pub default: f64,
    /// Valid range (angles in degrees).
    pub min: f64,
    pub max: f64,
}

/// A feature's description for agents and the GUI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeatureInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub params: &'static [ParamInfo],
    /// Landmarks with what they mark.
    pub landmarks: &'static [(&'static str, &'static str)],
    /// Typical triangle count at default settings.
    pub budget: u32,
    pub aliases: &'static [&'static str],
    /// Whether the output is SDF items (blendable) rather than a mesh.
    pub sdf: bool,
}

impl ParamInfo {
    /// A value of this parameter for people and agents (`45 mm`, `20 deg`,
    /// `elf`).
    pub fn text(&self, v: f64) -> String {
        match self.kind {
            ParamKind::Length => p3d_math::format_length(v),
            ParamKind::Angle => format!("{} deg", trim(v)),
            ParamKind::Choice(words) => words.get(v as usize).copied().unwrap_or("").to_owned(),
            ParamKind::Ratio | ParamKind::Count => trim(v),
        }
    }

    /// The parameter as JSON: name, kind, default and range (or choices).
    pub fn to_json(&self) -> Value {
        let mut o = serde_json::json!({ "name": self.name, "doc": self.doc, "default": self.text(self.default) });
        match self.kind {
            ParamKind::Choice(words) => o["choices"] = serde_json::json!(words),
            k => {
                o["kind"] = serde_json::json!(kind_word(k));
                o["min"] = serde_json::json!(self.text(self.min));
                o["max"] = serde_json::json!(self.text(self.max));
            }
        }
        o
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

impl FeatureInfo {
    /// The feature in full as text: summary, parameters with defaults and
    /// ranges, landmarks and how to use it.
    ///
    /// Complexity: O(parameters + landmarks).
    pub fn describe(&self) -> String {
        let mut t = format!("{} ({}): {}\n", self.id, self.label, self.summary);
        t.push_str(&format!(
            "output: {}, about {} tris\nparameters:\n",
            if self.sdf { "SDF shapes (blend into SDF parts)" } else { "designed mesh" },
            self.budget
        ));
        for p in self.params {
            match p.kind {
                ParamKind::Choice(words) => t.push_str(&format!("  {}: {} (default {}). {}\n", p.name, words.join(" | "), p.text(p.default), p.doc)),
                _ => t.push_str(&format!(
                    "  {}: {} (default {}, {} to {}). {}\n",
                    p.name,
                    kind_word(p.kind),
                    p.text(p.default),
                    p.text(p.min),
                    p.text(p.max),
                    p.doc
                )),
            }
        }
        t.push_str("landmarks:\n");
        for (n, doc) in self.landmarks {
            t.push_str(&format!("  {n}: {doc}\n"));
        }
        t.push_str(&format!(
            "use: op gen.feature {{\"feature\": \"{id}\", \"params\": {{...}}}}, or a spec part {{\"feature\": \"{id}\", \"params\": {{...}}}}; landmarks are vertex groups lm.<name>\n",
            id = self.id
        ));
        t
    }

    /// The feature as JSON for clients and the GUI.
    ///
    /// Complexity: O(parameters + landmarks).
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "label": self.label,
            "summary": self.summary,
            "sdf": self.sdf,
            "budget": self.budget,
            "aliases": self.aliases,
            "params": self.params.iter().map(ParamInfo::to_json).collect::<Vec<_>>(),
            "landmarks": self.landmarks.iter().map(|(n, d)| serde_json::json!({ "name": n, "doc": d })).collect::<Vec<_>>(),
        })
    }
}

fn kind_word(k: ParamKind) -> &'static str {
    match k {
        ParamKind::Length => "length",
        ParamKind::Angle => "angle",
        ParamKind::Count => "count",
        _ => "number",
    }
}

/// Resolved parameter values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Params {
    values: BTreeMap<&'static str, f64>,
    choices: BTreeMap<&'static str, &'static str>,
}

impl Params {
    /// A numeric parameter (lengths in meters, angles in radians).
    pub fn get(&self, name: &str) -> f64 {
        self.values.get(name).copied().unwrap_or(0.0)
    }

    pub fn f(&self, name: &str) -> f32 {
        self.get(name) as f32
    }

    pub fn count(&self, name: &str) -> usize {
        self.get(name).round().max(0.0) as usize
    }

    pub fn choice(&self, name: &str) -> &'static str {
        self.choices.get(name).copied().unwrap_or("")
    }
}

/// A problem with a feature or its parameters, at a JSON pointer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct FeatureError {
    pub path: String,
    pub message: String,
    pub suggestions: Vec<String>,
}

impl FeatureError {
    fn new(path: &str, message: impl Into<String>) -> FeatureError {
        FeatureError { path: path.to_owned(), message: message.into(), suggestions: Vec::new() }
    }
}

/// What a feature builds.
#[derive(Debug, Clone)]
// Built once per call and moved, never stored in bulk: boxing buys nothing.
#[allow(clippy::large_enum_variant)]
pub enum Shape {
    Mesh(Mesh),
    Sdf(Vec<Item>),
}

/// A built feature: its shape and named landmarks (feature space).
#[derive(Debug, Clone)]
pub struct Built {
    pub shape: Shape,
    pub landmarks: BTreeMap<String, Vec3>,
}

/// A feature template.
pub trait Feature: Send + Sync {
    fn info(&self) -> FeatureInfo;
    /// Builds from resolved parameters.
    fn build(&self, p: &Params) -> Result<Built, FeatureError>;
}

/// Every built-in feature, in id order.
pub fn library() -> Vec<Box<dyn Feature>> {
    let mut v: Vec<Box<dyn Feature>> = vec![
        Box::new(ear::Ear),
        Box::new(eye::Eye),
        Box::new(horn::Horn),
        Box::new(nose::Nose),
        Box::new(hard::Bolt),
        Box::new(hard::Rivet),
        Box::new(hard::VentGrille),
        Box::new(hard::PanelInset),
        Box::new(hard::Molding),
        Box::new(head::HeadBase),
        Box::new(hand::Hand),
        Box::new(cable::Cable),
    ];
    v.sort_by_key(|f| f.info().id);
    v
}

/// The feature with this id, or an error with suggestions.
///
/// Complexity: O(features).
pub fn get(id: &str) -> Result<Box<dyn Feature>, FeatureError> {
    let lib = library();
    let ids: Vec<&'static str> = lib.iter().map(|f| f.info().id).collect();
    lib.into_iter().find(|f| f.info().id == id).ok_or_else(|| {
        let mut e = FeatureError::new("/feature", format!("there is no feature `{id}`"));
        e.suggestions = p3d_scene_suggest(id, &ids);
        e
    })
}

/// Close names, for suggestions.
fn p3d_scene_suggest(name: &str, candidates: &[&str]) -> Vec<String> {
    p3d_scene::name::suggest(name, candidates.iter().copied(), 3)
}

/// Features whose id, label, summary or aliases contain every word of the
/// query.
///
/// Complexity: O(features x words).
pub fn search(query: &str) -> Vec<FeatureInfo> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    library()
        .into_iter()
        .map(|f| f.info())
        .filter(|i| {
            let hay = format!("{} {} {} {}", i.id, i.label, i.summary, i.aliases.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Resolves JSON parameters against a feature's description: defaults for
/// missing ones, unit strings parsed, ranges checked.
///
/// Complexity: O(parameters).
pub fn resolve(info: &FeatureInfo, v: &Value) -> Result<Params, FeatureError> {
    let empty = serde_json::Map::new();
    let obj = match v {
        Value::Object(m) => m,
        Value::Null => &empty,
        _ => return Err(FeatureError::new("/params", "give the parameters as an object")),
    };
    for k in obj.keys() {
        if !info.params.iter().any(|p| p.name == k) {
            let mut e = FeatureError::new(&format!("/params/{k}"), format!("`{}` has no parameter `{k}`", info.id));
            e.suggestions = p3d_scene_suggest(k, &info.params.iter().map(|p| p.name).collect::<Vec<_>>());
            return Err(e);
        }
    }
    let mut out = Params::default();
    for p in info.params {
        let path = format!("/params/{}", p.name);
        let given = obj.get(p.name);
        if let ParamKind::Choice(words) = p.kind {
            let w = match given {
                None => words.get(p.default as usize).copied().unwrap_or(""),
                Some(Value::String(s)) => words.iter().copied().find(|w| w == s).ok_or_else(|| {
                    let mut e = FeatureError::new(&path, format!("`{s}` is not one of {}", words.join(", ")));
                    e.suggestions = p3d_scene_suggest(s, words);
                    e
                })?,
                Some(_) => return Err(FeatureError::new(&path, format!("give one of {}", words.join(", ")))),
            };
            out.choices.insert(p.name, w);
            continue;
        }
        let raw = match given {
            None => p.default,
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            Some(Value::Bool(b)) => f64::from(u8::from(*b)),
            Some(Value::String(s)) => match p.kind {
                ParamKind::Length => p3d_math::parse_length(s).map_err(|e| FeatureError::new(&path, e.to_string()))?,
                ParamKind::Angle => p3d_math::parse_angle(s).map_err(|e| FeatureError::new(&path, e.to_string()))?.to_degrees(),
                _ => s.parse().map_err(|_| FeatureError::new(&path, format!("`{s}` is not a number")))?,
            },
            Some(_) => return Err(FeatureError::new(&path, "give a number")),
        };
        if !raw.is_finite() || raw < p.min || raw > p.max {
            let unit = match p.kind {
                ParamKind::Length => " m",
                ParamKind::Angle => " degrees",
                _ => "",
            };
            return Err(FeatureError::new(&path, format!("{} must be between {}{unit} and {}{unit}", p.name, p.min, p.max)));
        }
        let value = match p.kind {
            ParamKind::Angle => raw.to_radians(),
            ParamKind::Count => raw.round(),
            _ => raw,
        };
        out.values.insert(p.name, value);
    }
    Ok(out)
}

/// Builds a feature from JSON parameters.
///
/// Complexity: the feature's own.
pub fn build(id: &str, params: &Value) -> Result<Built, FeatureError> {
    let f = get(id)?;
    let p = resolve(&f.info(), params)?;
    f.build(&p)
}

/// Moves landmarks onto the surface of SDF items by Newton steps along
/// the gradient (points already on it stay).
///
/// Complexity: O(landmarks x items).
pub(crate) fn onto_surface(items: &[Item], marks: &mut BTreeMap<String, Vec3>) {
    let Ok(sdf) = p3d_sdf::Sdf::new(items.to_vec()) else { return };
    let (lo, hi) = sdf.bounds();
    let eps = (hi - lo).max_element() * 1e-4;
    for p in marks.values_mut() {
        for _ in 0..16 {
            let d = sdf.eval(*p);
            let g = sdf.gradient(*p, eps).normalize_or_zero();
            if d.abs() < eps || g == Vec3::ZERO {
                break;
            }
            *p -= g * d;
        }
    }
}

/// Puts each landmark on its nearest vertex as a one-vertex group
/// `lm.<name>`.
///
/// Complexity: O(V) per landmark.
pub fn bake_landmarks(mesh: &mut Mesh, landmarks: &BTreeMap<String, Vec3>) {
    for (name, p) in landmarks {
        let v =
            mesh.verts().min_by(|&a, &b| mesh.positions[a as usize].distance_squared(*p).total_cmp(&mesh.positions[b as usize].distance_squared(*p)));
        if let Some(v) = v {
            // A fresh vertex group of one vertex cannot fail on a live mesh.
            let _ = mesh.set_group(&format!("lm.{name}"), ElementKind::Vert, vec![v]);
        }
    }
}

/// The landmarks baked into a mesh, by name.
///
/// Complexity: O(groups).
pub fn baked_landmarks(mesh: &Mesh) -> BTreeMap<String, Vec3> {
    mesh.groups()
        .filter(|(n, g)| n.starts_with("lm.") && g.kind() == ElementKind::Vert)
        .filter_map(|(n, g)| g.iter().next().map(|v| (n["lm.".len()..].to_owned(), mesh.positions[v as usize])))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn parameters_resolve_with_units_defaults_and_errors() {
        let ear = get("ear").unwrap().info();
        let p = resolve(&ear, &json!({ "length": "9cm", "tilt_back": "20deg", "variant": "elf" })).unwrap();
        assert!((p.get("length") - 0.09).abs() < 1e-9);
        assert!((p.get("tilt_back") - 20f64.to_radians()).abs() < 1e-9);
        assert_eq!(p.choice("variant"), "elf");
        let e = resolve(&ear, &json!({ "lenght": 0.1 })).unwrap_err();
        assert_eq!(e.path, "/params/lenght");
        assert!(e.suggestions.contains(&"length".to_owned()));
        let e = resolve(&ear, &json!({ "variant": "elff" })).unwrap_err();
        assert!(e.suggestions.contains(&"elf".to_owned()));
        let e = resolve(&ear, &json!({ "length": 50 })).unwrap_err();
        assert!(e.message.contains("between"), "{e}");
        assert!(get("eer").err().is_some_and(|e| e.suggestions.contains(&"ear".to_owned())));
        assert!(!search("pointed ear").is_empty());
    }
}
