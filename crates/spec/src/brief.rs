//! Briefs: the user's request as structure (project spec 17.2). A brief
//! expands into checks: its size and budget, its category's defaults
//! (symmetry, parts attached, closed surfaces, proportions for the style),
//! its must-have features with their traits and its must-nots. Specs answer
//! a brief by naming it (`"brief": "elf_ranger_head"`), and every report
//! carries the coverage line `brief 9/10; failing: ...`. Checks name parts,
//! so a brief can come before its spec; until then its items are pending.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use glam::Vec3;
use p3d_ops::params::Length;
use p3d_ops::{Category, ErrorCode, Op, OpCtx, OpError, OpExample, OpFlags, OpMeta, OpOutput, Tx};
use p3d_scene::{Check, Document, Expectation, ObjectKey, ObjectRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SpecError;
use crate::compile::part_landmarks;
use crate::model::{Join, Landmarks, ModelSpec, parse};

/// Relative tolerance of a size given as one value (`"0.26m"`).
const SIZE_TOLERANCE: f64 = 0.1;
/// Largest gap an anchored part may leave to its parent.
const ATTACH_GAP: f64 = 0.002;

/// A brief.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Asset category, for example `humanoid_head`: default checks and
    /// landmarks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `realistic` or `stylized` (proportion ranges).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_class: Option<String>,
    /// Engine profile: `unreal`, `unity`, `godot` or `gltf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_have: Vec<MustHave>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not: Vec<String>,
    /// Reference images and outlines (kept; outlines become silhouette
    /// targets later).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Value>,
    /// Visual DNA the asset should have (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dna: Option<Value>,
}

/// A size: one value (within 10%) or `[min, max]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SizeRange {
    About(Length),
    Range([Length; 2]),
}

impl SizeRange {
    fn bounds(self) -> (f64, f64) {
        match self {
            SizeRange::About(v) => (v.0 * (1.0 - SIZE_TOLERANCE), v.0 * (1.0 + SIZE_TOLERANCE)),
            SizeRange::Range([a, b]) => (a.0.min(b.0), a.0.max(b.0)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizeRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<SizeRange>,
}

/// A count: at most this many, or `[min, max]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CountRange {
    Max(u64),
    Range([u64; 2]),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tris: Option<CountRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materials: Option<u32>,
    /// Texture size, for example `2k`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub textures: Option<String>,
}

/// A feature the asset must have.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MustHave {
    /// The feature (`ear`, `hair`, `horn`): parts whose name starts with it
    /// or that carry it as a tag.
    pub feature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Traits: `length_ratio_to_<part>: [min, max]` is measured; other
    /// traits wait for the feature library.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub traits: BTreeMap<String, Value>,
}

/// Parses a brief, with errors at JSON pointer paths.
///
/// Complexity: O(size of the brief).
pub fn parse_brief(source: &Value) -> Result<Brief, SpecError> {
    serde_path_to_error::deserialize(source).map_err(|e| {
        let path = crate::model::pointer(&e.path().to_string());
        SpecError::new(&path, e.inner().to_string())
    })
}

/// One check a brief expands into.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    /// World size of the asset along an axis (0 x, 1 y, 2 z), meters.
    Size {
        axis: usize,
        min: f64,
        max: f64,
    },
    Budget {
        min: Option<u64>,
        max: u64,
    },
    /// The asset mirrors itself across x.
    Symmetric,
    /// Every separate part anchored on another touches it.
    Attached,
    /// The main part is closed.
    Watertight,
    /// Parts of a feature exist (at least `count`).
    Feature {
        feature: String,
        count: Option<u32>,
    },
    /// Longest extent of each feature part over the height of `to`.
    Ratio {
        feature: String,
        to: String,
        min: f64,
        max: f64,
    },
    /// A category proportion of the main part.
    Proportion {
        name: &'static str,
        min: f64,
        max: f64,
    },
    NotIntersecting {
        a: String,
        b: String,
    },
    /// Intent that cannot be measured yet.
    Pending {
        reason: String,
    },
}

/// A labelled check.
#[derive(Debug, Clone, PartialEq)]
pub struct BriefItem {
    pub label: String,
    pub kind: ItemKind,
}

/// Category proportion references: name, then (min, max) for realistic and
/// for stylized.
type Proportions = &'static [(&'static str, (f64, f64), (f64, f64))];

fn proportions(category: &str) -> Proportions {
    match category {
        // Width and depth over height of the skull (ears excluded), and the
        // eye line as a fraction of the height from the chin.
        "humanoid_head" => {
            &[("width_to_height", (0.68, 0.85), (0.62, 1.0)), ("depth_to_height", (0.8, 1.05), (0.7, 1.15)), ("eye_line", (0.42, 0.55), (0.35, 0.62))]
        }
        _ => &[],
    }
}

/// The checks of a brief.
///
/// Complexity: O(items).
pub fn expand(b: &Brief) -> Vec<BriefItem> {
    let mut out = Vec::new();
    if let Some(s) = &b.size {
        for (axis, name, r) in [(0, "width", s.width), (1, "height", s.height), (2, "depth", s.depth)] {
            if let Some(r) = r {
                let (min, max) = r.bounds();
                out.push(BriefItem { label: format!("{name} {} to {} mm", mm(min), mm(max)), kind: ItemKind::Size { axis, min, max } });
            }
        }
    }
    if let Some(budget) = &b.budget {
        if let Some(t) = budget.tris {
            let (min, max) = match t {
                CountRange::Max(m) => (None, m),
                CountRange::Range([a, b]) => (Some(a.min(b)), a.max(b)),
            };
            let label = match min {
                Some(lo) => format!("budget {} to {} tris", thousands(lo), thousands(max)),
                None => format!("budget at most {} tris", thousands(max)),
            };
            out.push(BriefItem { label, kind: ItemKind::Budget { min, max } });
        }
        if budget.materials.is_some() || budget.textures.is_some() {
            out.push(BriefItem { label: "materials and textures".into(), kind: ItemKind::Pending { reason: "materials come with Phase 6".into() } });
        }
    }
    let category = b.category.as_deref().unwrap_or("");
    if category == "humanoid_head" {
        out.push(BriefItem { label: "symmetric across x".into(), kind: ItemKind::Symmetric });
        out.push(BriefItem { label: "head closed".into(), kind: ItemKind::Watertight });
    }
    out.push(BriefItem { label: "parts attached to what they sit on".into(), kind: ItemKind::Attached });
    let stylized = b.style.as_deref() == Some("stylized");
    for &(name, real, styl) in proportions(category) {
        let (min, max) = if stylized { styl } else { real };
        out.push(BriefItem { label: format!("{name} {min} to {max}"), kind: ItemKind::Proportion { name, min, max } });
    }
    for m in &b.must_have {
        let label = match m.count {
            Some(n) => format!("{n} {}", m.feature),
            None => m.feature.clone(),
        };
        out.push(BriefItem { label, kind: ItemKind::Feature { feature: m.feature.clone(), count: m.count } });
        for (t, v) in &m.traits {
            let range = v.as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
            match (t.strip_prefix("length_ratio_to_"), range) {
                (Some(to), Some((a, b))) => out.push(BriefItem {
                    label: format!("{} length {a} to {b} of {to}", m.feature),
                    kind: ItemKind::Ratio { feature: m.feature.clone(), to: to.to_owned(), min: a.min(b), max: a.max(b) },
                }),
                _ => out.push(BriefItem {
                    label: format!("{} {t} {}", m.feature, v.as_str().map_or_else(|| v.to_string(), str::to_owned)),
                    kind: ItemKind::Pending { reason: format!("`{t}` is measured by the {} feature (Phase 3c)", m.feature) },
                }),
            }
        }
    }
    for rule in &b.must_not {
        let kind = match rule.split_once("_penetrating_") {
            Some((a, b)) => ItemKind::NotIntersecting { a: a.to_owned(), b: b.to_owned() },
            None if rule.contains("seam") => ItemKind::Pending { reason: "UV seams come with Phase 4".into() },
            None => ItemKind::Pending { reason: "no check for this rule yet".into() },
        };
        out.push(BriefItem { label: format!("no {}", rule.replace('_', " ")), kind });
    }
    out
}

/// The verdict on one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemResult {
    pub label: String,
    pub status: Status,
    /// Measured values, and why it fails or waits.
    pub detail: String,
}

/// How well the model answers a brief.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Coverage {
    pub brief: String,
    /// The spec root measured, if a spec answers the brief.
    pub spec: Option<String>,
    pub items: Vec<ItemResult>,
}

impl Coverage {
    /// `brief elf 9/12; failing: ear length 0.31 (0.35 to 0.6 of head); 2 pending`.
    pub fn line(&self) -> String {
        let n = |s| self.items.iter().filter(|i| i.status == s).count();
        let (pass, pending) = (n(Status::Pass), n(Status::Pending));
        let mut t = format!("brief {} {pass}/{}", self.brief, self.items.len() - pending);
        let failing: Vec<String> =
            self.items.iter().filter(|i| i.status == Status::Fail).take(3).map(|i| format!("{} ({})", i.label, i.detail)).collect();
        if !failing.is_empty() {
            let _ = write!(t, "; failing: {}", failing.join(", "));
        }
        if pending > 0 {
            let _ = write!(t, "; {pending} pending");
        }
        if self.spec.is_none() {
            t.push_str("; no spec answers it yet");
        }
        t
    }
}

fn mm(m: f64) -> String {
    format!("{}", (m * 1000.0).round() as i64)
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// World bounds of a set of objects' meshes.
///
/// Complexity: O(vertices).
fn bounds(doc: &Document, keys: &[ObjectKey]) -> Option<(Vec3, Vec3)> {
    let mut acc: Option<(Vec3, Vec3)> = None;
    for &k in keys {
        let (Some(m), Some(w)) = (doc.object_mesh(k), doc.world_affine(k)) else { continue };
        for v in m.verts() {
            let p = w.transform_point3(m.positions[v as usize]);
            acc = Some(acc.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
        }
    }
    acc
}

/// What a spec root offers the checks.
struct Asset<'a> {
    doc: &'a Document,
    root: ObjectKey,
    spec: ModelSpec,
    /// Separate (non-decal) part objects by part name.
    objects: BTreeMap<String, ObjectKey>,
}

impl Asset<'_> {
    fn reference(&self, key: ObjectKey) -> ObjectRef {
        let o = self.doc.object(key);
        ObjectRef { name: self.doc.path(key).unwrap_or_default(), id: o.map_or(0, |o| o.id()) }
    }

    fn check(&self, check: Check) -> (Status, String) {
        let o = p3d_perceive::expect::check(self.doc, &Expectation { name: String::new(), check });
        (if o.pass { Status::Pass } else { Status::Fail }, o.message)
    }

    /// Parts of a feature: by name prefix or tag.
    fn feature_parts(&self, feature: &str) -> Vec<&str> {
        self.spec.parts.iter().filter(|(n, p)| n.starts_with(feature) || p.tags.iter().any(|t| t == feature)).map(|(n, _)| n.as_str()).collect()
    }

    /// World bounds of a part: its object, or its faces in the mesh it is
    /// blended into.
    fn part_bounds(&self, part: &str) -> Option<(Vec3, Vec3)> {
        if let Some(&k) = self.objects.get(part) {
            return bounds(self.doc, &[k]);
        }
        let prefix = format!("{part}.");
        for &k in self.objects.values() {
            let (Some(m), Some(w)) = (self.doc.object_mesh(k), self.doc.world_affine(k)) else { continue };
            let mut acc: Option<(Vec3, Vec3)> = None;
            for (_, g) in m.groups().filter(|(n, g)| n.starts_with(&prefix) && g.kind() == p3d_mesh::ElementKind::Face) {
                for f in g.iter() {
                    for v in m.face_verts(f) {
                        let p = w.transform_point3(m.positions[v as usize]);
                        acc = Some(acc.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
                    }
                }
            }
            if acc.is_some() {
                return acc;
            }
        }
        None
    }

    /// The part carrying the category's landmarks (else `head`).
    fn main_part(&self) -> Option<&str> {
        self.spec
            .parts
            .iter()
            .find(|(_, p)| matches!(&p.landmarks, Some(Landmarks::Category(_))))
            .map(|(n, _)| n.as_str())
            .or_else(|| self.spec.parts.contains_key("head").then_some("head"))
    }

    fn evaluate(&self, item: &BriefItem) -> (Status, String) {
        let all: Vec<ObjectKey> = self.objects.values().copied().collect();
        match &item.kind {
            ItemKind::Pending { reason } => (Status::Pending, reason.clone()),
            ItemKind::Size { axis, min, max } => match bounds(self.doc, &all) {
                Some((lo, hi)) => {
                    let v = f64::from(hi[*axis] - lo[*axis]);
                    (if (*min..=*max).contains(&v) { Status::Pass } else { Status::Fail }, format!("{} mm", mm(v)))
                }
                None => (Status::Fail, "nothing built".into()),
            },
            ItemKind::Budget { min, max } => {
                let tris: u64 = all.iter().filter_map(|&k| self.doc.object_mesh(k)).map(|m| m.triangle_count() as u64).sum();
                let ok = tris <= *max && min.is_none_or(|m| tris >= m);
                (if ok { Status::Pass } else { Status::Fail }, format!("{} tris", thousands(tris)))
            }
            ItemKind::Symmetric => self.check(Check::Symmetric { a: self.reference(self.root), b: None, axis: 'x', at: None, tolerance: 0.002 }),
            ItemKind::Watertight => match self.main_part().and_then(|p| self.objects.get(p)) {
                Some(&k) => self.check(Check::Watertight { target: self.reference(k) }),
                None => (Status::Fail, "no main part".into()),
            },
            ItemKind::Attached => {
                let mut failing = Vec::new();
                let mut n = 0;
                for (name, part) in &self.spec.parts {
                    let (Some(&k), Some(on)) = (self.objects.get(name), part.anchor.as_ref().and_then(|a| a.on.as_ref())) else { continue };
                    let Some(&to) = self.objects.get(on) else { continue };
                    n += 1;
                    let (s, msg) =
                        self.check(Check::Attached { part: self.reference(k), to: self.reference(to), max_gap: ATTACH_GAP, max_penetration: None });
                    if s == Status::Fail {
                        failing.push(format!("{name}: {msg}"));
                    }
                }
                if failing.is_empty() {
                    (Status::Pass, format!("{n} anchored part{} touch", if n == 1 { "" } else { "s" }))
                } else {
                    (Status::Fail, failing.join("; "))
                }
            }
            ItemKind::Feature { feature, count } => {
                let n = self.feature_parts(feature).len() as u32;
                let ok = n > 0 && count.is_none_or(|c| n >= c);
                (if ok { Status::Pass } else { Status::Fail }, format!("{n} found"))
            }
            ItemKind::Ratio { feature, to, min, max } => {
                let Some((lo, hi)) = self.part_bounds(to) else { return (Status::Fail, format!("no part `{to}`")) };
                let base = f64::from(hi.y - lo.y).max(1e-9);
                // The part furthest outside the range speaks for all.
                let mut worst: Option<(f64, f64)> = None;
                for p in self.feature_parts(feature) {
                    if let Some((a, b)) = self.part_bounds(p) {
                        let r = f64::from((b - a).max_element()) / base;
                        let off = if r < *min {
                            min - r
                        } else if r > *max {
                            r - max
                        } else {
                            0.0
                        };
                        if worst.is_none_or(|(w, _)| off > w) {
                            worst = Some((off, r));
                        }
                    }
                }
                match worst {
                    Some((_, r)) => (if (*min..=*max).contains(&r) { Status::Pass } else { Status::Fail }, format!("{r:.2}")),
                    None => (Status::Fail, format!("no {feature} parts")),
                }
            }
            ItemKind::Proportion { name, min, max } => {
                let Some(main) = self.main_part() else { return (Status::Fail, "no main part".into()) };
                if !self.objects.contains_key(main) {
                    return (Status::Fail, format!("`{main}` has no object"));
                }
                let (lo, hi) = self.skull_bounds(main);
                let size = hi - lo;
                let v = match *name {
                    "width_to_height" => f64::from(size.x / size.y.max(1e-9)),
                    "depth_to_height" => f64::from(size.z / size.y.max(1e-9)),
                    "eye_line" => {
                        let eye =
                            part_landmarks(self.doc, self.root, main).ok().and_then(|l| l.into_iter().find(|m| m.name == "eye_l")).map(|m| m.point.y);
                        match eye {
                            Some(y) => f64::from((y - lo.y) / size.y.max(1e-9)),
                            None => return (Status::Pending, "no eye_l landmark".into()),
                        }
                    }
                    _ => return (Status::Pending, "unknown proportion".into()),
                };
                (if (*min..=*max).contains(&v) { Status::Pass } else { Status::Fail }, format!("{v:.2}"))
            }
            ItemKind::NotIntersecting { a, b } => match (self.objects.get(a), self.objects.get(b)) {
                (Some(&x), Some(&y)) => self.check(Check::NotIntersecting { parts: vec![self.reference(x), self.reference(y)] }),
                _ => (Status::Pending, format!("waits for parts `{a}` and `{b}`")),
            },
        }
    }

    /// Bounds of the main part without the faces blended in from other
    /// parts (ears do not widen the skull).
    fn skull_bounds(&self, main: &str) -> (Vec3, Vec3) {
        let Some(&k) = self.objects.get(main) else { return (Vec3::ZERO, Vec3::ZERO) };
        let (Some(m), Some(w)) = (self.doc.object_mesh(k), self.doc.world_affine(k)) else { return (Vec3::ZERO, Vec3::ZERO) };
        let blended: Vec<String> =
            self.spec.parts.iter().filter(|(_, p)| p.join() == Join::Blend || p.mirror.is_some()).map(|(n, _)| format!("{n}.")).collect();
        let mut skip = vec![false; m.face_slots()];
        for (name, g) in m.groups() {
            if g.kind() == p3d_mesh::ElementKind::Face && blended.iter().any(|b| name.starts_with(b.as_str())) {
                for f in g.iter() {
                    skip[f as usize] = true;
                }
            }
        }
        let mut acc: Option<(Vec3, Vec3)> = None;
        for f in m.faces().filter(|&f| !skip[f as usize]) {
            for v in m.face_verts(f) {
                let p = w.transform_point3(m.positions[v as usize]);
                acc = Some(acc.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
            }
        }
        acc.unwrap_or((Vec3::ZERO, Vec3::ZERO))
    }
}

/// Coverage of a brief by the spec that names it (the first by id).
///
/// Complexity: the checks: bounded samples for symmetry, one relation per
/// anchored part, O(V) for sizes and budgets.
pub fn coverage(doc: &Document, name: &str) -> Result<Coverage, OpError> {
    let source = doc.brief(name).ok_or_else(|| {
        let mut e = OpError::new(ErrorCode::NotFound, format!("no brief `{name}`")).at("/name");
        e.suggestions = p3d_scene::name::suggest(name, doc.briefs().map(|(n, _)| n), 3);
        e
    })?;
    let brief = parse_brief(source).map_err(|e| OpError::invalid(&e.path, e.message))?;
    let items = expand(&brief);
    let mut roots: Vec<(u32, ObjectKey, ModelSpec)> = doc
        .spec_roots()
        .filter_map(|k| {
            let spec = parse(&doc.spec(k)?.source).ok()?;
            (spec.brief.as_deref() == Some(name)).then(|| (doc.object(k).map_or(0, |o| o.id()), k, spec))
        })
        .collect();
    roots.sort_by_key(|r| r.0);
    let Some((_, root, spec)) = roots.into_iter().next() else {
        let items = items.into_iter().map(|i| ItemResult { label: i.label, status: Status::Pending, detail: "no spec yet".into() }).collect();
        return Ok(Coverage { brief: name.to_owned(), spec: None, items });
    };
    let record = doc.spec(root).cloned().unwrap_or_default();
    let objects: BTreeMap<String, ObjectKey> = record
        .parts
        .iter()
        .filter(|(n, _)| spec.parts.get(*n).is_some_and(|p| p.join() != Join::Decal))
        .filter_map(|(n, r)| Some((n.clone(), doc.object_by_id(r.object?)?)))
        .collect();
    let asset = Asset { doc, root, spec, objects };
    let items = items
        .iter()
        .map(|i| {
            let (status, detail) = asset.evaluate(i);
            ItemResult { label: i.label.clone(), status, detail }
        })
        .collect();
    Ok(Coverage { brief: name.to_owned(), spec: doc.path(root), items })
}

// ---- brief.set -----------------------------------------------------------------

/// Parameters of `brief.set`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BriefSetParams {
    /// Name of the brief; a spec answers it with `"brief": "<name>"`.
    pub name: String,
    /// The brief: title, category, style, size, budget, must_have, must_not, references (see the spec guide).
    pub brief: Value,
}

/// `brief.set`.
pub struct BriefSet;

impl Op for BriefSet {
    const ID: &'static str = "brief.set";
    type Params = BriefSetParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set brief",
            category: Category::Spec,
            summary: "Records what the user asked for (category, style, size, budget, must-haves) as checks the model is measured against.",
            docs: "A brief expands into checks: its size (one value means within 10%) and triangle budget, its category's \
                   defaults (a humanoid head is symmetric and closed, anchored parts touch what they sit on, proportions within the \
                   style's range), each `must_have` feature (parts named after it or tagged with it) with measurable traits \
                   (`length_ratio_to_<part>`), and each `must_not` (`<a>_penetrating_<b>`). A spec answers the brief by naming it; \
                   every report then carries the coverage line. Items that need later phases are listed as pending.\n\n\
                   **Use for** turning the user's words into tests before modelling.",
            examples: &[OpExample {
                args: r#"{"name": "elf", "brief": {"title": "Elf ranger head", "category": "humanoid_head", "style": "stylized", "size": {"height": "0.26m"}, "budget": {"tris": [8000, 14000]}, "must_have": [{"feature": "ear", "count": 2, "traits": {"length_ratio_to_head": [0.35, 0.6], "pointiness": "high"}}]}}"#,
                result: "brief elf: 10 checks: height 234 to 286 mm; budget 8,000 to 14,000 tris; symmetric across x; head closed; parts attached to what they sit on; width_to_height 0.62 to 1; depth_to_height 0.7 to 1.15; eye_line 0.35 to 0.62; 2 ear; ear length 0.35 to 0.6 of head (1 pending)",
            }],
            aliases: &["brief", "requirements", "intent", "goals", "acceptance"],
            complexity: "O(size of the brief)",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(_doc: &Document, p: &BriefSetParams) -> Result<(), OpError> {
        p3d_scene::name::check_name(&p.name).map_err(|e| OpError::invalid("/name", e.to_string()))?;
        parse_brief(&p.brief).map(|_| ()).map_err(|e| OpError::invalid(&format!("/brief{}", e.path), e.message))
    }

    fn apply(tx: &mut Tx, p: &BriefSetParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let brief = parse_brief(&p.brief).map_err(|e| OpError::invalid(&format!("/brief{}", e.path), e.message))?;
        tx.doc_mut().set_brief(&p.name, p.brief.clone());
        let items = expand(&brief);
        let pending = items.iter().filter(|i| matches!(i.kind, ItemKind::Pending { .. })).count();
        let labels: Vec<&str> = items.iter().filter(|i| !matches!(i.kind, ItemKind::Pending { .. })).map(|i| i.label.as_str()).collect();
        let mut s = format!("brief {}: {} checks: {}", p.name, labels.len(), labels.join("; "));
        if pending > 0 {
            let _ = write!(s, " ({pending} pending)");
        }
        Ok(OpOutput::summary(s))
    }
}
