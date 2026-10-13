//! Full diagnostics (project spec 17.10): the mesh-level findings of
//! `p3d-perceive` plus the ones that need the Model Spec's meaning.
//!
//! - **Intent:** failing expectations (`EXPECTATION_FAILED`), brief items
//!   (`BRIEF_UNMET`), category proportions out of their style's range
//!   (`PROPORTIONS`) and broken mirror symmetry (`ASYMMETRY`).
//! - **Edge flow at landmarks:** too few loops across joints
//!   (`JOINT_LOOPS`) and missing eye and mouth loops on `humanoid_head`
//!   parts (`FACE_LOOPS`).
//! - **Spec-first fixes and causes:** placement fixes on spec parts become
//!   spec patches of the anchor offset or position (an op on a compiled
//!   object would be undone by the next compile), placement findings name
//!   the anchor as their cause, and every finding on a spec part names the
//!   landmark it is at.

use std::collections::{BTreeMap, HashMap};

use glam::{EulerRot, Quat, Vec3};
use p3d_ops::params::Length;
use p3d_perceive::diagnose::flow::{DEFORMING, MIN_JOINT_LOOPS, RING_FLOW, joint_loops, remesh_fix, ring_flow};
use p3d_perceive::diagnose::source::{SpecPart, part_object, patch_fix, set_ops, shape_source, spec_part};
use p3d_perceive::diagnose::{Draft, Finding, Fix, Severity, drafts as mesh_drafts, rank, tagged};
use p3d_perceive::fmt::mm;
use p3d_perceive::view::View;
use p3d_scene::{Document, ObjectKey};

use serde_json::{Value, json};

use crate::brief::{ItemKind, Status, expand, parse_brief};
use crate::compile::{LandmarkPoint, part_landmarks};
use crate::model::{Landmarks, ModelSpec, parse};

/// Mirror pairs whose sizes or places differ by more than this share are
/// asymmetric.
pub const ASYMMETRY: f32 = 0.03;
/// A landmark names a finding's place when it is within this share of its
/// part's largest extent.
const LANDMARK_REACH: f32 = 0.2;
/// Landmark names that mark joints.
pub const JOINT_WORDS: [&str; 10] = ["shoulder", "elbow", "wrist", "hip", "knee", "ankle", "neck", "waist", "knuckle", "joint"];
/// Loop radius around each face landmark of a head, as a share of its
/// height (as the compiler's remesh lays them).
const FACE_LOOPS: [(&str, f32); 3] = [("eye_l", 0.15), ("eye_r", 0.15), ("mouth", 0.18)];

/// A spec root with something in view.
struct Root {
    key: ObjectKey,
    path: String,
    spec: ModelSpec,
}

/// Lookups shared by the checks.
struct Ctx<'v, 'a> {
    view: &'v View<'a>,
    index: HashMap<ObjectKey, usize>,
    roots: Vec<Root>,
    landmarks: BTreeMap<(usize, String), Vec<LandmarkPoint>>,
}

impl<'v, 'a> Ctx<'v, 'a> {
    /// Complexity: O(parts + specs).
    fn new(view: &'v View<'a>) -> Ctx<'v, 'a> {
        let doc = view.doc;
        let index: HashMap<ObjectKey, usize> = view.parts.iter().enumerate().map(|(i, p)| (p.key, i)).collect();
        let mut keys: Vec<ObjectKey> = view.parts.iter().filter_map(|p| spec_part(doc, p.key).map(|s| s.root)).collect();
        keys.extend(view.root.filter(|&r| doc.spec(r).is_some()));
        let mut roots: Vec<Root> = Vec::new();
        for k in keys {
            if roots.iter().any(|r| r.key == k) {
                continue;
            }
            let Some(spec) = doc.spec(k).and_then(|r| parse(&r.source).ok()) else { continue };
            roots.push(Root { key: k, path: doc.path(k).unwrap_or_default(), spec });
        }
        roots.sort_by(|a, b| a.path.cmp(&b.path));
        Ctx { view, index, roots, landmarks: BTreeMap::new() }
    }

    fn doc(&self) -> &'a Document {
        self.view.doc
    }

    /// The view index of a spec part's object.
    fn part(&self, root: ObjectKey, name: &str) -> Option<usize> {
        part_object(self.doc(), root, name).and_then(|k| self.index.get(&k).copied())
    }

    /// The landmarks of a view part compiled from a spec (cached).
    ///
    /// Complexity: [`part_landmarks`] on first use per part.
    fn landmarks(&mut self, i: usize) -> &[LandmarkPoint] {
        let doc = self.doc();
        let Some(sp) = spec_part(doc, self.view.parts[i].key) else { return &[] };
        let key = (i, sp.name.to_owned());
        let root = sp.root;
        self.landmarks.entry(key).or_insert_with_key(|(_, name)| part_landmarks(doc, root, name).unwrap_or_default())
    }
}

fn length(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| serde_json::from_value::<Length>(v.clone()).ok()).map(|l| l.0)
}

fn vec3(v: Option<&Value>) -> Option<Vec3> {
    let a = v?.as_array()?;
    Some(Vec3::new(length(a.first())? as f32, length(a.get(1))? as f32, length(a.get(2))? as f32))
}

fn r4(x: f32) -> f64 {
    (f64::from(x) * 1e4).round() / 1e4
}

fn r4v(v: Vec3) -> [f64; 3] {
    v.to_array().map(r4)
}

// ---- placement fixes as spec patches -------------------------------------------

/// The world move `delta` of a spec part as a spec patch: its anchor offset
/// (in the anchor's frame), its root-space position, or, for a mirror, its
/// source's reflected move.
///
/// Complexity: O(1).
fn spec_move(doc: &Document, sp: &SpecPart, delta: Vec3, summary: &str) -> Option<(Fix, String)> {
    let root_world = doc.world_affine(sp.root)?;
    let mut d = root_world.inverse().transform_vector3(delta);
    let mut sp = *sp;
    if let Some(src) = sp.get("mirror").and_then(Value::as_str) {
        let axis = match sp.get("plane").and_then(Value::as_str) {
            Some("y") => 1,
            Some("z") => 2,
            _ => 0,
        };
        d[axis] = -d[axis];
        let k = part_object(doc, sp.root, src)?;
        sp = spec_part(doc, k)?;
    }
    let obj = doc.object(part_object(doc, sp.root, sp.name)?)?;
    if let Some(anchor) = sp.get("anchor") {
        let frame = match anchor.get("orient").and_then(Value::as_str) {
            Some("normal") => {
                let r = sp.get("transform/rotate").and_then(Value::as_array).map_or(Quat::IDENTITY, |a| {
                    let deg = |k: usize| a.get(k).and_then(Value::as_f64).unwrap_or(0.0).to_radians() as f32;
                    Quat::from_euler(EulerRot::XYZ, deg(0), deg(1), deg(2))
                });
                obj.transform.rotation * r.inverse()
            }
            _ => {
                let on = anchor.get("on").and_then(Value::as_str)?;
                doc.object(part_object(doc, sp.root, on)?)?.transform.rotation
            }
        };
        let old = vec3(anchor.get("offset")).unwrap_or(Vec3::ZERO);
        let new = old + frame.inverse() * d;
        let fix = patch_fix(doc, sp.root, set_ops(&sp, "anchor/offset", json!(r4v(new))), format!("{summary} (anchor offset)"));
        return Some((fix, sp.path("anchor")));
    }
    let old = vec3(sp.get("transform/position")).unwrap_or(obj.transform.translation);
    let fix = patch_fix(doc, sp.root, set_ops(&sp, "transform/position", json!(r4v(old + d))), format!("{summary} (position)"));
    Some((fix, sp.path("transform/position")))
}

/// Placement findings on spec parts: the fix moves the part in its spec
/// and the cause is its placement.
///
/// Complexity: O(drafts).
fn spec_placement(view: &View, drafts: &mut [Draft]) {
    for d in drafts.iter_mut().filter(|d| matches!(d.code, "FLOATING_PART" | "PENETRATION" | "Z_FIGHTING")) {
        let part = &view.parts[d.part];
        let Some(sp) = spec_part(view.doc, part.key) else { continue };
        let Some(fix) = d.fixes.first() else { continue };
        let Some(rel) = fix.args.get("position").and_then(Value::as_array) else { continue };
        let local = Vec3::new(
            rel.first().and_then(Value::as_f64).unwrap_or(0.0) as f32,
            rel.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32,
            rel.get(2).and_then(Value::as_f64).unwrap_or(0.0) as f32,
        );
        // Back to world: the op's move is in the object's parent's space.
        let parent = view.doc.object(part.key).and_then(|o| o.parent());
        let delta = parent.and_then(|p| view.doc.world_affine(p)).map_or(local, |m| m.transform_vector3(local));
        let summary = fix.summary.clone();
        if let Some((patch, cause)) = spec_move(view.doc, &sp, delta, &summary) {
            d.fixes[0] = patch;
            let what = sp.get("anchor").map_or_else(
                || "its position".to_owned(),
                |a| {
                    let on = a.get("on").and_then(Value::as_str).unwrap_or("?");
                    match a.get("landmark").and_then(Value::as_str) {
                        Some(l) => format!("its anchor on {on} at {l}"),
                        None => format!("its anchor on {on}"),
                    }
                },
            );
            d.cause = Some(format!("{cause} places it ({what})"));
            d.cause_path = Some(cause);
        }
    }
}

// ---- intent ----------------------------------------------------------------------

/// Failing expectations of the document, on their first part in view; a fix
/// of a finding on the same part is offered too.
///
/// Complexity: the expectation checks.
fn expectations(ctx: &Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    let known: Vec<(usize, Fix)> = out.iter().filter_map(|d| Some((d.part, d.fixes.first()?.clone()))).collect();
    for e in doc.expectations() {
        let Some(i) = e.check.objects().into_iter().filter_map(|id| doc.object_by_id(id)).find_map(|k| ctx.index.get(&k).copied()) else {
            continue;
        };
        let o = p3d_perceive::expect::check(doc, e);
        if o.pass {
            continue;
        }
        let part = &ctx.view.parts[i];
        let mut d = Draft::new("EXPECTATION_FAILED", Severity::Error, i, part.center(), format!("test {} {} fails: {}", o.name, o.label, o.message));
        d.cause = Some(format!("expectation {} ({})", o.name, o.label));
        if let Some((_, f)) = known.iter().find(|(p, _)| *p == i) {
            d.fixes.push(f.clone());
        } else {
            d.hint = Some("expect check shows the measurements; perceive relations or shape show why".into());
        }
        out.push(d);
    }
}

/// The part carrying a spec's category landmarks, else `head`, else the
/// first part in view.
///
/// Complexity: O(parts x log parts).
fn main_part(ctx: &Ctx, root: &Root) -> Option<usize> {
    let named = root
        .spec
        .parts
        .iter()
        .find(|(_, p)| matches!(&p.landmarks, Some(Landmarks::Category(_))))
        .map(|(n, _)| n.as_str())
        .or_else(|| root.spec.parts.contains_key("head").then_some("head"));
    named
        .and_then(|n| ctx.part(root.key, n))
        .or_else(|| root.spec.parts.keys().find_map(|n| ctx.part(root.key, n).filter(|&i| !tagged(ctx.view, &ctx.view.parts[i], "strands"))))
}

/// A spec patch scaling a part along root axes by `factor` (multiplying
/// its own scale).
fn scale_fix(doc: &Document, sp: &SpecPart, factor: Vec3, summary: String) -> Fix {
    let old = match sp.get("transform/scale") {
        Some(Value::Array(a)) => Vec3::new(
            a.first().and_then(Value::as_f64).unwrap_or(1.0) as f32,
            a.get(1).and_then(Value::as_f64).unwrap_or(1.0) as f32,
            a.get(2).and_then(Value::as_f64).unwrap_or(1.0) as f32,
        ),
        Some(v) => Vec3::splat(v.as_f64().unwrap_or(1.0) as f32),
        None => Vec3::ONE,
    };
    let new = (old * factor).to_array().map(|x| (f64::from(x) * 1e3).round() / 1e3);
    patch_fix(doc, sp.root, set_ops(sp, "transform/scale", json!(new)), summary)
}

/// A value inside a range, a quarter in from the bound it is past.
fn target_in(v: f64, min: f64, max: f64) -> f64 {
    let q = 0.25 * (max - min);
    if v < min { min + q } else { max - q }
}

/// Category proportions of each spec root out of their style's range.
///
/// Complexity: one landmark query plus O(V) bounds per root.
fn proportions(ctx: &Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    for root in &ctx.roots {
        let Ok((_, Value::Array(rows))) = crate::brief::proportions(doc, root.key) else { continue };
        let main_name = rows.iter().find_map(|r| r.get("relative_to").and_then(Value::as_str));
        let Some(i) = main_name.and_then(|n| ctx.part(root.key, n)) else { continue };
        let Some(sp) = spec_part(doc, ctx.view.parts[i].key) else { continue };
        for r in &rows {
            if r.get("status").and_then(Value::as_str) != Some("fail") {
                continue;
            }
            let (Some(name), Some(v), Some(min), Some(max)) = (
                r.get("name").and_then(Value::as_str),
                r.get("value").and_then(Value::as_str).and_then(|s| s.parse::<f64>().ok()),
                r.get("min").and_then(Value::as_f64),
                r.get("max").and_then(Value::as_f64),
            ) else {
                continue;
            };
            let part = &ctx.view.parts[i];
            let bound = if v < min { min } else { max };
            let mut d = Draft::new(
                "PROPORTIONS",
                Severity::Warn,
                i,
                part.center(),
                format!("{name} is {v:.2}, outside {min} to {max} for its category and style"),
            )
            .measure(v as f32, bound as f32, "ratio");
            let target = target_in(v, min, max);
            let f = (target / v.max(1e-6)) as f32;
            let axis = match name {
                "width_to_height" => Some((Vec3::new(f, 1.0, 1.0), "width")),
                "depth_to_height" => Some((Vec3::new(1.0, 1.0, f), "depth")),
                _ => None,
            };
            d.cause = Some(format!("{} shapes it", sp.path("")));
            d.cause_path = Some(sp.path(""));
            match axis {
                Some((factor, what)) => d.fixes.push(scale_fix(doc, &sp, factor, format!("scale its {what} by {f:.2} to {target:.2}"))),
                None => d.hint = Some("move the eye anchors (or the landmarks they sit on) toward the range".into()),
            }
            out.push(d);
        }
    }
}

/// Failing brief items (proportions have their own code), on the part they
/// are about.
///
/// Complexity: the brief's coverage checks per root.
fn brief_items(ctx: &Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    let known: Vec<(usize, Fix)> = out.iter().filter_map(|d| Some((d.part, d.fixes.first()?.clone()))).collect();
    for root in &ctx.roots {
        let Some(name) = root.spec.brief.as_deref() else { continue };
        let Some(brief) = doc.brief(name).and_then(|b| parse_brief(b).ok()) else { continue };
        let Ok(c) = crate::brief::coverage(doc, name) else { continue };
        if c.spec.as_deref() != Some(root.path.as_str()) {
            continue;
        }
        let Some(main) = main_part(ctx, root) else { continue };
        for (item, res) in expand(&brief).iter().zip(&c.items) {
            if res.status != Status::Fail || matches!(item.kind, ItemKind::Proportion { .. }) {
                continue;
            }
            let feature_part = |f: &str| root.spec.parts.keys().filter(|n| n.starts_with(f)).find_map(|n| ctx.part(root.key, n));
            let i = match &item.kind {
                ItemKind::Ratio { feature, .. } | ItemKind::Feature { feature, .. } => feature_part(feature).unwrap_or(main),
                ItemKind::NotIntersecting { a, .. } => ctx.part(root.key, a).unwrap_or(main),
                _ => main,
            };
            let part = &ctx.view.parts[i];
            let mut d = Draft::new("BRIEF_UNMET", Severity::Warn, i, part.center(), format!("brief {name}: {} fails ({})", item.label, res.detail));
            d.cause = Some(format!("brief {name} asks for {}", item.label));
            match &item.kind {
                ItemKind::Size { axis, min, max } => {
                    let (lo, hi) = root_bounds(ctx, root);
                    let v = f64::from(hi[*axis] - lo[*axis]);
                    if v > 0.0 {
                        let f = target_in(v, *min, *max) / v;
                        d.measured = Some(v as f32 * 1e3);
                        d.threshold = Some(if v < *min { *min as f32 * 1e3 } else { *max as f32 * 1e3 });
                        d.unit = Some("mm");
                        d.fixes.push(Fix::op(
                            "scene.set_transform",
                            json!({ "target": root.path, "scale": (f * 1e3).round() / 1e3, "relative": true }),
                            format!("scale the whole asset by {f:.2}"),
                        ));
                    }
                }
                ItemKind::Ratio { feature, min, max, .. } => {
                    if let Ok(r) = res.detail.parse::<f64>() {
                        let f = (target_in(r, *min, *max) / r.max(1e-6)) as f32;
                        d.measured = Some(r as f32);
                        d.threshold = Some(if r < *min { *min as f32 } else { *max as f32 });
                        d.unit = Some("ratio");
                        // Mirrors follow their sources.
                        let sources: Vec<SpecPart> = root
                            .spec
                            .parts
                            .iter()
                            .filter(|(n, p)| n.starts_with(feature.as_str()) && p.mirror.is_none())
                            .filter_map(|(n, _)| spec_part(doc, part_object(doc, root.key, n)?))
                            .collect();
                        let mut ops = Vec::new();
                        for s in &sources {
                            if let Some(o) = scale_fix(doc, s, Vec3::splat(f), String::new()).args.get("ops").and_then(Value::as_array) {
                                ops.extend(o.iter().cloned());
                            }
                        }
                        if !ops.is_empty() {
                            d.fixes.push(patch_fix(doc, root.key, ops, format!("scale the {feature} parts by {f:.2}")));
                        }
                    }
                }
                ItemKind::Feature { feature, .. } => {
                    d.hint = Some(format!("add a part named {feature}_... (features lists templates; spec docs show anchors)"));
                }
                _ => {
                    if let Some((_, f)) = known.iter().find(|(p, _)| *p == i) {
                        d.fixes.push(f.clone());
                    } else {
                        d.hint = Some(format!("perceive brief {} shows the measurements", root.path));
                    }
                }
            }
            out.push(d);
        }
    }
}

/// World bounds of a spec root's part objects in view.
///
/// Complexity: O(parts x log parts).
fn root_bounds(ctx: &Ctx, root: &Root) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for n in root.spec.parts.keys() {
        if let Some(i) = ctx.part(root.key, n) {
            let p = &ctx.view.parts[i];
            if p3d_perceive::diagnose::decal(ctx.view, p) {
                continue;
            }
            lo = lo.min(p.lo);
            hi = hi.max(p.hi);
        }
    }
    (lo, hi)
}

/// The other side's name of a left or right part name.
fn mirror_name(name: &str) -> Option<String> {
    for (l, r) in [("_l", "_r"), ("_left", "_right")] {
        if let Some(stem) = name.strip_suffix(l) {
            return Some(format!("{stem}{r}"));
        }
    }
    name.strip_prefix("left_").map(|s| format!("right_{s}"))
}

/// The left part's JSON as the right part would read if it mirrored it:
/// side words swapped, x offsets and y and z turns negated.
///
/// Complexity: O(size of the value).
fn mirrored(v: &Value, key: &str) -> Value {
    match v {
        Value::String(s) => Value::String(mirror_name(s).unwrap_or_else(|| s.clone())),
        Value::Array(a) if matches!(key, "offset" | "position" | "nearest" | "through") && a.len() == 3 => {
            let mut a = a.clone();
            if let Some(x) = a[0].as_f64() {
                a[0] = json!(-x);
            }
            Value::Array(a)
        }
        Value::Array(a) if key == "rotate" && a.len() == 3 => {
            let mut a = a.clone();
            for v in a.iter_mut().skip(1) {
                if let Some(x) = v.as_f64() {
                    *v = json!(-x);
                }
            }
            Value::Array(a)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| mirrored(x, key)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), mirrored(x, k))).collect()),
        other => other.clone(),
    }
}

/// The first leaf (by sorted keys) where `b` differs from `a`, as a pointer
/// below the part, with `a`'s value (null when `a` lacks it).
///
/// Complexity: O(size of the values).
fn first_diff(a: &Value, b: &Value, path: &str) -> Option<(String, Value)> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|k| first_diff(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), &format!("{path}/{k}")))
        }
        (Value::Number(x), Value::Number(y)) => {
            let (p, q) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            ((p - q).abs() > 1e-9).then(|| (path.to_owned(), a.clone()))
        }
        _ => (a != b).then(|| (path.to_owned(), a.clone())),
    }
}

/// Size and place of a part in its root's space.
///
/// Complexity: O(V) over the part's vertices.
fn root_box(ctx: &Ctx, root: ObjectKey, i: usize) -> Option<(Vec3, Vec3)> {
    let inv = ctx.doc().world_affine(root)?.inverse();
    let p = &ctx.view.parts[i];
    let m = p.mesh;
    let to = inv * p.world;
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for v in m.verts() {
        let q = to.transform_point3(m.positions[v as usize]);
        lo = lo.min(q);
        hi = hi.max(q);
    }
    (lo.x <= hi.x).then_some((hi - lo, (lo + hi) / 2.0))
}

/// Left and right parts written out twice that drifted apart: their sizes
/// or their places differ by more than [`ASYMMETRY`] from mirror images
/// across the root's x plane. (A `mirror` part is an exact copy: the
/// compiler ignores edits and deform on it.)
///
/// Complexity: O(V) per pair for root-space bounds, plus the size of the
/// two parts' JSON.
fn asymmetry(ctx: &Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    for root in &ctx.roots {
        let Some(source) = doc.spec(root.key).map(|r| &r.source) else { continue };
        for (left, part) in &root.spec.parts {
            let Some(right) = mirror_name(left) else { continue };
            let Some(other) = root.spec.parts.get(&right) else { continue };
            // A real mirror, or strands (each side grows from its own seed).
            if part.mirror.is_some() || other.mirror.is_some() || part.strands.is_some() || other.strands.is_some() {
                continue;
            }
            let (Some(ri), Some(li)) = (ctx.part(root.key, &right), ctx.part(root.key, left)) else { continue };
            let (Some((rs, rc)), Some((ls, lc))) = (root_box(ctx, root.key, ri), root_box(ctx, root.key, li)) else { continue };
            let size_diff = (rs - ls).abs() / rs.max(ls).max(Vec3::splat(1e-6));
            let k = size_diff.max_position();
            let off = rc.distance(Vec3::new(-lc.x, lc.y, lc.z));
            let pos_diff = off / rs.max(ls).max_element().max(1e-6);
            let worst = size_diff[k].max(pos_diff);
            if worst <= ASYMMETRY {
                continue;
            }
            let Some(sp) = spec_part(doc, ctx.view.parts[ri].key) else { continue };
            let message = if size_diff[k] >= pos_diff {
                let axis_word = ["width", "height", "depth"][k];
                format!("{left} and {right} differ by {}% in {axis_word} (mirror images expected)", (size_diff[k] * 100.0).round() as i32)
            } else {
                format!("{right} sits {} mm off the mirror of {left}'s place", mm(off))
            };
            let mut d =
                Draft::new("ASYMMETRY", Severity::Warn, ri, ctx.view.parts[ri].center(), message).measure(worst * 100.0, ASYMMETRY * 100.0, "%");
            d.other = Some(li);
            let left_json = source.pointer(&format!("/parts/{left}")).cloned().unwrap_or(Value::Null);
            let right_json = sp.json;
            match first_diff(&mirrored(&left_json, ""), right_json, "") {
                Some((path, value)) => {
                    let have = right_json.pointer(&path).cloned().unwrap_or(Value::Null);
                    let at = format!("{}{path}", sp.path(""));
                    d.cause = Some(format!("{at} = {have} overrides the mirror ({left} has {value})"));
                    d.cause_path = Some(at.clone());
                    let op = if value.is_null() { json!({ "op": "remove", "path": at }) } else { json!({ "op": "add", "path": at, "value": value }) };
                    d.fixes.push(patch_fix(doc, root.key, vec![op], format!("match {left}: {path} = {value}")));
                }
                None => {
                    d.cause = Some(format!("{} is written out by hand instead of mirroring {left}", sp.path("")));
                    d.cause_path = Some(sp.path(""));
                }
            }
            // Making it a real mirror keeps them equal from now on.
            let mut m = serde_json::Map::new();
            m.insert("mirror".into(), json!(left));
            for k in ["tags", "material", "join", "blend", "importance"] {
                if let Some(v) = right_json.get(k) {
                    m.insert(k.into(), v.clone());
                }
            }
            d.fixes.push(patch_fix(
                doc,
                root.key,
                vec![json!({ "op": "add", "path": sp.path(""), "value": Value::Object(m) })],
                format!("make {right} a mirror of {left}"),
            ));
            out.push(d);
        }
    }
}

// ---- edge flow at landmarks ---------------------------------------------------------

/// Whether a landmark name marks a joint.
pub fn is_joint(name: &str) -> bool {
    JOINT_WORDS.iter().any(|w| name.contains(w))
}

/// Joints of deforming spec parts with too few loops across them.
///
/// Complexity: per deforming part, its landmarks plus O(log n + k) per joint.
fn joints(ctx: &mut Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    for i in 0..ctx.view.parts.len() {
        let part = &ctx.view.parts[i];
        if !tagged(ctx.view, part, DEFORMING) || tagged(ctx.view, part, "strands") {
            continue;
        }
        let Some(sp) = spec_part(doc, part.key) else { continue };
        let marks: Vec<LandmarkPoint> = ctx.landmarks(i).iter().filter(|l| is_joint(&l.name)).cloned().collect();
        for l in marks {
            let part = &ctx.view.parts[i];
            let Some(j) = joint_loops(part, l.point) else { continue };
            if j.loops >= MIN_JOINT_LOOPS {
                continue;
            }
            let mut d = Draft::new(
                "JOINT_LOOPS",
                Severity::Warn,
                i,
                l.point,
                format!(
                    "{} loop{} across {} within its {} mm width (needs {} or more to bend cleanly)",
                    j.loops,
                    if (j.loops - 1.0).abs() < 1e-6 { "" } else { "s" },
                    l.name,
                    mm(2.0 * j.radius),
                    MIN_JOINT_LOOPS
                ),
            )
            .measure(j.loops, MIN_JOINT_LOOPS, "count");
            d.landmark = Some(format!("{}.{}", sp.name, l.name));
            // Spacing that puts a little more than the minimum across.
            let want = 2.0 * j.radius / (MIN_JOINT_LOOPS + 0.5);
            let factor = (j.spacing / want).max(1.0);
            // Shape parameters live on the part a mirror or instance copies.
            let sp = shape_source(doc, sp);
            let prim = sp.json.get("primitive").and_then(Value::as_object).and_then(|m| m.iter().next());
            match prim {
                Some((kind, args)) if matches!(kind.as_str(), "cylinder" | "capsule" | "tube") => {
                    let cur = args.pointer("/advanced/height_segments").and_then(Value::as_u64).unwrap_or(1);
                    let new = ((cur as f32 * factor).ceil() as u64).clamp(cur + 1, 4096);
                    let rest = format!("primitive/{kind}/advanced/height_segments");
                    d.cause = Some(format!("{} = {cur}: too few rings along the limb", sp.path(&rest)));
                    d.cause_path = Some(sp.path(&rest));
                    d.fixes.push(patch_fix(doc, sp.root, set_ops(&sp, &rest, json!(new)), format!("{new} rings along the limb")));
                }
                _ if sp.get("sdf").is_some() => {
                    let detail = length(sp.get("detail"));
                    d.cause = Some(match detail {
                        Some(v) => format!("{} = {} mm voxels are too coarse here", sp.path("detail"), mm(v as f32)),
                        None => format!("{} (default) voxels are too coarse here", sp.path("detail")),
                    });
                    d.cause_path = Some(sp.path("detail"));
                    d.fixes.push(patch_fix(doc, sp.root, set_ops(&sp, "detail", json!(r4(want))), format!("voxels of {} mm", mm(want))));
                }
                _ => {
                    d.fixes.push(remesh_fix(ctx.view, i, Some(&sp), json!({ "edge": r4(want) }), "rebuild it with even quads across the joint"));
                }
            }
            out.push(d);
        }
    }
}

/// Whether a spec part is a humanoid head: it carries the `humanoid_head`
/// landmarks or is a `head_base` feature.
fn humanoid_head(spec: &ModelSpec, name: &str) -> bool {
    let Some(p) = spec.parts.get(name) else { return false };
    match p.landmarks.as_ref() {
        Some(Landmarks::Category(c)) if c == "category" => spec.category.as_deref() == Some("humanoid_head"),
        Some(Landmarks::Category(c)) => c == "humanoid_head",
        _ => p.feature.as_deref() == Some("head_base"),
    }
}

/// `humanoid_head` parts without edge loops circling the eyes and mouth.
///
/// Complexity: per head, its landmarks plus O(E) per face landmark.
fn face_loops(ctx: &mut Ctx, out: &mut Vec<Draft>) {
    let doc = ctx.doc();
    for i in 0..ctx.view.parts.len() {
        let part = &ctx.view.parts[i];
        let Some(sp) = spec_part(doc, part.key) else { continue };
        let Some(root) = ctx.roots.iter().find(|r| r.key == sp.root) else { continue };
        if !humanoid_head(&root.spec, sp.name) {
            continue;
        }
        let deforming = tagged(ctx.view, part, DEFORMING);
        let height = part.size().y;
        let marks: Vec<LandmarkPoint> = ctx.landmarks(i).to_vec();
        let part = &ctx.view.parts[i];
        let (mut missing, mut loops, mut worst, mut edge) = (Vec::new(), Vec::new(), 0.0f32, f32::INFINITY);
        // The remesh runs in the part's own space.
        let to_part = part.world.inverse();
        let local_height = part.mesh.bounds().map_or(height, |(lo, hi)| hi.y - lo.y);
        for (name, share) in FACE_LOOPS {
            let Some(l) = marks.iter().find(|l| l.name == name) else { continue };
            let flow = ring_flow(part, l.point, height * share).unwrap_or(1.0);
            if flow > RING_FLOW {
                missing.push(l);
                worst = worst.max(flow);
            }
            loops.push(json!({ "at": r4v(to_part.transform_point3(l.point)), "radius": r4(local_height * share) }));
            edge = edge.min(local_height * share / 10.0);
        }
        let Some(first) = missing.first() else { continue };
        let names: Vec<&str> = missing.iter().map(|l| l.name.as_str()).collect();
        let mut d = Draft::new(
            "FACE_LOOPS",
            if deforming { Severity::Warn } else { Severity::Info },
            i,
            first.point,
            format!(
                "no edge loops circle {} (flow {:.2} off circular, limit {RING_FLOW}): the face will not open, close or smile cleanly",
                names.join(", "),
                worst
            ),
        )
        .measure(worst, RING_FLOW, "ratio");
        d.landmark = Some(format!("{}.{}", sp.name, first.name));
        d.fixes.push(remesh_fix(
            ctx.view,
            i,
            Some(&sp),
            // Loops form when the ring spans enough edges: about ten per radius.
            json!({ "edge": r4(edge), "loops": loops, "align": "y" }),
            "rebuild it as quads with loops around the eyes and mouth",
        ));
        out.push(d);
    }
}

// ---- landmarks at findings ------------------------------------------------------------

/// Names the landmark at each draft's place: for relation findings the
/// nearest landmark of the other part (where it sits), else of its own
/// part, within a fifth of that part's size; a part's own center gives way
/// to the landmark it is anchored at.
///
/// Complexity: [`part_landmarks`] once per spec part with findings, plus
/// O(landmarks) per draft.
fn name_places(ctx: &mut Ctx, drafts: &mut [Draft]) {
    for d in drafts.iter_mut().filter(|d| d.landmark.is_none()) {
        let mut best: Option<(f32, String)> = None;
        for i in d.other.into_iter().chain(std::iter::once(d.part)) {
            // Strands have no landmarks of their own worth naming.
            if best.is_some() || tagged(ctx.view, &ctx.view.parts[i], "strands") {
                continue;
            }
            let reach = LANDMARK_REACH * ctx.view.parts[i].size().max_element();
            let Some(sp) = spec_part(ctx.doc(), ctx.view.parts[i].key) else { continue };
            let name = sp.name.to_owned();
            for l in ctx.landmarks(i) {
                let dist = l.point.distance(d.at);
                if dist <= reach && best.as_ref().is_none_or(|(b, _)| dist < *b) {
                    best = Some((dist, format!("{name}.{}", l.name)));
                }
            }
        }
        // A small part's own center says less than where it is anchored.
        if best.as_ref().is_none_or(|(_, n)| n.ends_with(".center"))
            && let Some(sp) = spec_part(ctx.doc(), ctx.view.parts[d.part].key)
            && let Some(a) = sp.get("anchor")
            && let (Some(on), Some(l)) = (a.get("on").and_then(Value::as_str), a.get("landmark").and_then(Value::as_str))
        {
            best = Some((0.0, format!("{on}.{l}")));
        }
        d.landmark = best.map(|(_, n)| n);
    }
}

/// Every draft of a view: the mesh-level ones and the spec-level ones,
/// with spec-first fixes and named places, unranked.
///
/// Complexity: [`p3d_perceive::diagnose::drafts`] plus the spec checks
/// (landmark queries per spec part with findings, coverage per brief).
pub fn drafts(view: &View) -> Vec<Draft> {
    let mut out = mesh_drafts(view);
    let mut ctx = Ctx::new(view);
    spec_placement(view, &mut out);
    if !ctx.roots.is_empty() {
        joints(&mut ctx, &mut out);
        face_loops(&mut ctx, &mut out);
        asymmetry(&ctx, &mut out);
        proportions(&ctx, &mut out);
        brief_items(&ctx, &mut out);
    }
    expectations(&ctx, &mut out);
    if !ctx.roots.is_empty() {
        name_places(&mut ctx, &mut out);
    }
    out
}

/// Every finding of a view, ranked and numbered (project spec 17.10).
///
/// Complexity: [`drafts`] plus [`rank`].
pub fn diagnose(view: &View) -> Vec<Finding> {
    rank(view, drafts(view))
}

/// The `diagnose` report of a view (fixes without predictions).
///
/// Complexity: [`diagnose`].
pub fn report(view: &View) -> p3d_perceive::Report {
    let f = diagnose(view);
    p3d_perceive::Report::new("diagnose", p3d_perceive::diagnose::findings_text(&view.title(), &f), &f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_json_swaps_sides_and_flips_x() {
        let left = json!({ "anchor": { "on": "head", "landmark": "ear_root_l", "offset": [0.01, 0.0, 0.002] }, "params": { "length": 0.06 } });
        let want = mirrored(&left, "");
        assert_eq!(want["anchor"]["landmark"], "ear_root_r");
        assert_eq!(want["anchor"]["offset"], json!([-0.01, 0.0, 0.002]));
        let right = json!({ "anchor": { "on": "head", "landmark": "ear_root_r", "offset": [-0.01, 0.0, 0.002] }, "params": { "length": 0.07 } });
        assert_eq!(first_diff(&want, &right, ""), Some(("/params/length".into(), json!(0.06))));
    }

    #[test]
    fn joint_words_and_side_names() {
        assert!(is_joint("elbow_l") && is_joint("knee") && !is_joint("eye_l"));
        assert_eq!(mirror_name("ear_l").as_deref(), Some("ear_r"));
        assert_eq!(mirror_name("left_hand").as_deref(), Some("right_hand"));
        assert_eq!(mirror_name("head"), None);
        assert!((target_in(0.5, 0.68, 0.85) - 0.7225).abs() < 1e-9);
    }
}
