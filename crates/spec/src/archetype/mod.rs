//! Archetypes (ADR 0011, ADR 0016): genome templates that expand into a
//! named skeleton, generated parts and default genes when a spec is parsed.
//!
//! A spec's `archetypes` block names instances (`{"dwarf": {"kind":
//! "dwarf", "params": {...}}}`). Each instance's generator places joints
//! from its parameters, applies the instance's joint edits, then writes
//! ordinary spec parts anchored on those joints. The spec's own parts of
//! the same name are layered on top, so agents patch generated parts like
//! written ones. Expansion is pure data: no geometry, O(parts).

mod humanoid;
mod weapon;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use glam::{EulerRot, Quat, Vec3};
use p3d_features::{ParamInfo, ParamKind, Params};
use serde_json::{Map, Value, json};

use crate::SpecError;
use crate::model::{ArchetypeUse, JointEdit};

/// The skeleton name generated parts use for their own instance; expansion
/// replaces it with the instance's name.
pub const SELF: &str = "@";

/// Shape keys of a part: a written one that differs from the generated
/// part's replaces the generated shape.
const SHAPE_KEYS: [&str; 14] =
    ["sdf", "primitive", "lathe", "sweep", "loft", "extrude", "pipe", "shell", "silhouettes", "ornament", "feature", "strands", "mirror", "instance"];

/// A described archetype.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArchetypeInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
    pub params: &'static [ParamInfo],
    /// Joints and what they mark.
    pub joints: &'static [(&'static str, &'static str)],
    /// Generated parts and what they are.
    pub parts: &'static [(&'static str, &'static str)],
    /// The joint `at` puts on another instance's joint.
    pub root: &'static str,
    pub aliases: &'static [&'static str],
}

impl ArchetypeInfo {
    /// The archetype in full as text: summary, parameters, joints, parts
    /// and how to use it.
    ///
    /// Complexity: O(parameters + joints + parts).
    pub fn describe(&self) -> String {
        let mut t = format!("{} ({}): {}\nparameters:\n", self.id, self.label, self.summary);
        for p in self.params {
            match p.kind {
                ParamKind::Choice(words) => t.push_str(&format!("  {}: {} (default {}). {}\n", p.name, words.join(" | "), p.text(p.default), p.doc)),
                _ => t.push_str(&format!("  {}: default {}, {} to {}. {}\n", p.name, p.text(p.default), p.text(p.min), p.text(p.max), p.doc)),
            }
        }
        t.push_str(&format!("joints (landmarks of the instance's skeleton; `at` places joint {}):\n", self.root));
        for (n, doc) in self.joints {
            t.push_str(&format!("  {n}: {doc}\n"));
        }
        t.push_str("parts (patch them by name; `omit` leaves them out):\n");
        for (n, doc) in self.parts {
            t.push_str(&format!("  {n}: {doc}\n"));
        }
        t.push_str(&format!(
            "use: spec {{\"archetypes\": {{\"<name>\": {{\"kind\": \"{}\", \"params\": {{...}}}}}}}}; anchor parts on joints with {{\"on\": \"<name>\", \"landmark\": \"<joint>\"}}\n",
            self.id
        ));
        t
    }

    /// The archetype as JSON for clients and the GUI.
    ///
    /// Complexity: O(parameters + joints + parts).
    pub fn to_json(&self) -> Value {
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(n, d)| json!({ "name": n, "doc": d })).collect::<Vec<_>>();
        json!({
            "id": self.id,
            "label": self.label,
            "summary": self.summary,
            "root": self.root,
            "aliases": self.aliases,
            "params": self.params.iter().map(ParamInfo::to_json).collect::<Vec<_>>(),
            "joints": pairs(self.joints),
            "parts": pairs(self.parts),
        })
    }
}

/// One joint while an archetype is generated (the archetype's own space
/// until placed).
#[derive(Debug, Clone, PartialEq)]
pub struct RigJoint {
    pub name: String,
    pub at: Vec3,
    /// +Y of the joint's frame.
    pub axis: Vec3,
    /// +X of the joint's frame (perpendicular to `axis`).
    pub side: Vec3,
    pub parent: Option<String>,
}

/// Joints in order, parents before children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rig {
    pub joints: Vec<RigJoint>,
}

impl Rig {
    /// Adds a joint with an upright frame.
    pub fn add(&mut self, name: &str, at: Vec3, parent: Option<&str>) {
        self.joints.push(RigJoint { name: name.to_owned(), at, axis: Vec3::Y, side: Vec3::X, parent: parent.map(str::to_owned) });
    }

    /// Complexity: O(joints).
    pub fn get(&self, name: &str) -> Option<&RigJoint> {
        self.joints.iter().find(|j| j.name == name)
    }

    /// A joint's position (the origin for an unknown name).
    ///
    /// Complexity: O(joints).
    pub fn at(&self, name: &str) -> Vec3 {
        self.get(name).map_or(Vec3::ZERO, |j| j.at)
    }

    /// Sets a joint's frame: +Y along `axis`, +X along `side` made
    /// perpendicular (any perpendicular when they are parallel).
    ///
    /// Complexity: O(joints).
    pub fn frame(&mut self, name: &str, axis: Vec3, side: Vec3) {
        if let Some(j) = self.joints.iter_mut().find(|j| j.name == name) {
            let y = axis.try_normalize().unwrap_or(Vec3::Y);
            let x = (side - y * side.dot(y)).try_normalize().unwrap_or_else(|| y.any_orthonormal_vector());
            j.axis = y;
            j.side = x;
        }
    }

    /// Points a joint's +Y at another joint, keeping +X near `side`.
    ///
    /// Complexity: O(joints).
    pub fn aim(&mut self, name: &str, toward: &str, side: Vec3) {
        let axis = self.at(toward) - self.at(name);
        self.frame(name, axis, side);
    }

    /// The joint's rotation (columns side, axis, side x axis).
    ///
    /// Complexity: O(joints).
    pub fn rotation(&self, name: &str) -> Quat {
        self.get(name).map_or(Quat::IDENTITY, |j| Quat::from_mat3(&glam::Mat3::from_cols(j.side, j.axis, j.side.cross(j.axis))).normalize())
    }

    /// A point in a joint's frame (for parts anchored there with `orient:
    /// "normal"`).
    ///
    /// Complexity: O(joints).
    pub fn local(&self, name: &str, p: Vec3) -> Vec3 {
        self.rotation(name).inverse() * (p - self.at(name))
    }
}

/// What a generator writes besides the joints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Generated {
    /// Chains of joints.
    pub curves: Vec<(String, Vec<String>)>,
    /// Parts by local name; references to the instance's skeleton use
    /// [`SELF`].
    pub parts: Vec<(String, Value)>,
    /// Default genes, under the spec's own `dna`.
    pub dna: Value,
}

/// An archetype generator.
pub trait Archetype: Send + Sync {
    fn info(&self) -> ArchetypeInfo;
    /// Joint positions and parents from the parameters.
    fn joints(&self, p: &Params) -> Rig;
    /// Joint frames, curves, parts and genes from the (possibly edited)
    /// joints.
    fn build(&self, p: &Params, rig: &mut Rig) -> Generated;
}

/// Every archetype, in id order.
pub fn library() -> Vec<Box<dyn Archetype>> {
    let mut v: Vec<Box<dyn Archetype>> = Vec::new();
    v.extend(humanoid::all());
    v.extend(weapon::all());
    v.sort_by_key(|a| a.info().id);
    v
}

/// The archetype with this id (or alias), or an error with suggestions.
///
/// Complexity: O(archetypes).
pub fn get(id: &str) -> Result<Box<dyn Archetype>, (String, Vec<String>)> {
    let lib = library();
    let ids: Vec<&'static str> = lib.iter().map(|a| a.info().id).collect();
    let found = lib.into_iter().find(|a| a.info().id == id || a.info().aliases.contains(&id));
    found.ok_or_else(|| (format!("there is no archetype `{id}`"), p3d_scene::name::suggest(id, ids.iter().copied(), 3)))
}

/// Archetypes whose id, label, summary or aliases contain every word of
/// the query.
///
/// Complexity: O(archetypes x words).
pub fn search(query: &str) -> Vec<ArchetypeInfo> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    library()
        .into_iter()
        .map(|a| a.info())
        .filter(|i| {
            let hay = format!("{} {} {} {}", i.id, i.label, i.summary, i.aliases.join(" ")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// What one instance made: its kind, joint count and generated part names.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceReport {
    pub name: String,
    pub kind: String,
    pub joints: usize,
    pub parts: Vec<String>,
}

/// The spec with every archetype instance expanded; the source itself when
/// it has none.
///
/// Complexity: O(size of the expanded spec).
pub fn expand(source: &Value) -> Result<Cow<'_, Value>, SpecError> {
    Ok(match expand_report(source)? {
        Some((v, _)) => Cow::Owned(v),
        None => Cow::Borrowed(source),
    })
}

/// A JSON number rounded to 0.01 mm (or 1e-5 of a unit vector), so
/// generated specs read cleanly and hash the same everywhere.
pub fn num(x: f32) -> Value {
    json!((f64::from(x) * 1e5).round() / 1e5)
}

/// A point as JSON.
pub fn vec(v: Vec3) -> Value {
    Value::Array(vec![num(v.x), num(v.y), num(v.z)])
}

/// The order instances expand in: each after the instance its `at` names.
///
/// Complexity: O(instances).
fn order(uses: &BTreeMap<String, ArchetypeUse>) -> Result<Vec<String>, SpecError> {
    fn visit(n: &str, uses: &BTreeMap<String, ArchetypeUse>, state: &mut BTreeMap<String, u8>, out: &mut Vec<String>) -> Result<(), SpecError> {
        match state.get(n) {
            Some(2) => return Ok(()),
            Some(_) => return Err(SpecError::new(&format!("/archetypes/{n}/at"), "instances sit on each other in a loop")),
            None => {}
        }
        state.insert(n.to_owned(), 1);
        if let Some(at) = uses.get(n).and_then(|u| u.at.as_deref())
            && let Some((inst, _)) = at.split_once('.')
            && uses.contains_key(inst)
        {
            visit(inst, uses, state, out)?;
        }
        state.insert(n.to_owned(), 2);
        out.push(n.to_owned());
        Ok(())
    }
    let mut state = BTreeMap::new();
    let mut out = Vec::with_capacity(uses.len());
    for n in uses.keys() {
        visit(n, uses, &mut state, &mut out)?;
    }
    Ok(out)
}

/// Applies joint edits: `at` moves a joint to a point, `by` by an offset;
/// either way the joints below it move with it.
///
/// Complexity: O(edits x joints).
fn edit_joints(rig: &mut Rig, edits: &BTreeMap<String, JointEdit>, path: &str) -> Result<(), SpecError> {
    let v3 = |p: &[p3d_ops::params::Length; 3]| Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32);
    for (name, e) in edits {
        let jpath = format!("{path}/joints/{name}");
        let Some(j) = rig.get(name) else {
            let mut msg = format!("the skeleton has no joint `{name}`");
            let near = p3d_scene::name::suggest(name, rig.joints.iter().map(|j| j.name.as_str()), 3);
            if !near.is_empty() {
                msg.push_str(&format!("; did you mean {}?", near.join(", ")));
            }
            return Err(SpecError::new(&jpath, msg));
        };
        let delta = match (&e.at, &e.by) {
            (Some(a), None) => v3(a) - j.at,
            (None, Some(b)) => v3(b),
            _ => return Err(SpecError::new(&jpath, "give one of at (a position) or by (an offset)")),
        };
        if !delta.is_finite() {
            return Err(SpecError::new(&jpath, "the move is not finite"));
        }
        let mut moved: BTreeSet<String> = BTreeSet::new();
        moved.insert(name.clone());
        for j in &mut rig.joints {
            if j.name == *name || j.parent.as_ref().is_some_and(|p| moved.contains(p)) {
                moved.insert(j.name.clone());
                j.at += delta;
            }
        }
    }
    Ok(())
}

/// A generated part with a written part of the same name on top: a
/// different shape key replaces the generated shape, then the written part
/// merges in (objects merge, values replace).
///
/// Complexity: O(size of both parts).
fn layered(mut generated: Value, written: &Value) -> Value {
    let replaces = SHAPE_KEYS.iter().any(|k| match (written.get(*k), generated.get(*k)) {
        (Some(_), None) => true,
        (Some(w), Some(g)) => matches!(*k, "feature" | "strands" | "mirror" | "instance") && w != g,
        _ => false,
    });
    if replaces && let Value::Object(m) = &mut generated {
        for k in SHAPE_KEYS {
            m.remove(k);
        }
        m.remove("params");
    }
    crate::patch::merge(&mut generated, written);
    generated
}

/// Renames references inside a generated part: the instance's skeleton and
/// the instance's other parts (with the prefix).
///
/// Complexity: O(1) per reference.
fn rename_refs(part: &mut Value, instance: &str, prefix: &str, local: &BTreeSet<String>) {
    let rename = |v: &mut Value| {
        if let Value::String(s) = v {
            if s == SELF {
                *s = instance.to_owned();
            } else if local.contains(s.as_str()) {
                *s = format!("{prefix}{s}");
            }
        }
    };
    if let Some(on) = part.pointer_mut("/anchor/on") {
        rename(on);
    }
    for k in ["into", "mirror", "instance"] {
        if let Some(v) = part.get_mut(k) {
            rename(v);
        }
    }
    if part.get("strands").is_some()
        && let Some(on) = part.pointer_mut("/params/on")
    {
        rename(on);
    }
}

/// The rotation and translation that put an instance in the root's space.
fn placement(u: &ArchetypeUse, root: Option<&RigJoint>, placed: &BTreeMap<String, Rig>, path: &str) -> Result<(Quat, Vec3), SpecError> {
    if let Some(at) = &u.at {
        let apath = format!("{path}/at");
        let (inst, joint) =
            at.split_once('.').ok_or_else(|| SpecError::new(&apath, format!("`{at}` is not instance.joint, such as dwarf.grip_r")))?;
        let rig = placed.get(inst).ok_or_else(|| SpecError::new(&apath, format!("there is no archetype instance `{inst}`")))?;
        let t = rig.get(joint).ok_or_else(|| {
            let near = p3d_scene::name::suggest(joint, rig.joints.iter().map(|j| j.name.as_str()), 3);
            let mut m = format!("`{inst}` has no joint `{joint}`");
            if !near.is_empty() {
                m.push_str(&format!("; did you mean {}?", near.join(", ")));
            }
            SpecError::new(&apath, m)
        })?;
        let tq = rig.rotation(&t.name);
        let (rp, rq) = match root {
            Some(r) => (r.at, Quat::from_mat3(&glam::Mat3::from_cols(r.side, r.axis, r.side.cross(r.axis))).normalize()),
            None => (Vec3::ZERO, Quat::IDENTITY),
        };
        let rot = (tq * rq.inverse()).normalize();
        return Ok((rot, t.at - rot * rp));
    }
    if let Some(t) = &u.transform {
        if t.scale.is_some() {
            return Err(SpecError::new(&format!("{path}/transform/scale"), "archetypes size through their parameters (height, length, span)"));
        }
        let rot = t.rotate.map_or(Quat::IDENTITY, |r| Quat::from_euler(EulerRot::XYZ, r[0].0 as f32, r[1].0 as f32, r[2].0 as f32));
        let pos = t.position.map_or(Vec3::ZERO, |p| Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32));
        return Ok((rot, pos));
    }
    Ok((Quat::IDENTITY, Vec3::ZERO))
}

/// A placed skeleton as spec JSON.
fn skeleton_json(rig: &Rig, curves: &[(String, Vec<String>)]) -> Value {
    let mut joints = Map::new();
    for j in &rig.joints {
        let mut o = json!({ "at": vec(j.at), "axis": vec(j.axis), "side": vec(j.side) });
        if let Some(p) = &j.parent {
            o["parent"] = json!(p);
        }
        joints.insert(j.name.clone(), o);
    }
    let curves: Map<String, Value> = curves.iter().map(|(n, c)| (n.clone(), json!(c))).collect();
    json!({ "joints": joints, "curves": curves })
}

/// Expands every archetype instance: the expanded spec and what each
/// instance made, or none when the spec has no instances.
///
/// Complexity: O(size of the expanded spec + instances x joints).
pub fn expand_report(source: &Value) -> Result<Option<(Value, Vec<InstanceReport>)>, SpecError> {
    let Some(arch) = source.get("archetypes") else { return Ok(None) };
    let Value::Object(map) = arch else {
        return Err(SpecError::new("/archetypes", "give named instances, for example {\"dwarf\": {\"kind\": \"dwarf\"}}"));
    };
    if map.is_empty() {
        return Ok(None);
    }
    let mut uses = BTreeMap::new();
    for (name, v) in map {
        let path = format!("/archetypes/{name}");
        p3d_scene::name::check_name(name).map_err(|e| SpecError::new(&path, format!("`{name}` is not a valid instance name: {e}")))?;
        let u: ArchetypeUse = serde_path_to_error::deserialize(v)
            .map_err(|e| SpecError::new(&format!("{path}{}", crate::model::pointer(&e.path().to_string())), e.inner().to_string()))?;
        uses.insert(name.clone(), u);
    }
    let written: Map<String, Value> = source.get("parts").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut parts = written.clone();
    let mut skeletons: Map<String, Value> = source.get("skeletons").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut placed: BTreeMap<String, Rig> = BTreeMap::new();
    let mut made_by: BTreeMap<String, String> = BTreeMap::new();
    let mut genes: Option<Value> = None;
    let mut reports = Vec::with_capacity(uses.len());
    for name in order(&uses)? {
        let u = &uses[&name];
        let path = format!("/archetypes/{name}");
        let arch = get(&u.kind).map_err(|(m, s)| {
            let mut m = m;
            if !s.is_empty() {
                m.push_str(&format!("; did you mean {}?", s.join(", ")));
            }
            SpecError::new(&format!("{path}/kind"), m)
        })?;
        let info = arch.info();
        let p = p3d_features::resolve_params(info.id, info.params, &u.params).map_err(|e| {
            let mut m = e.message;
            if !e.suggestions.is_empty() {
                m.push_str(&format!("; did you mean {}?", e.suggestions.join(", ")));
            }
            SpecError::new(&format!("{path}{}", e.path), m)
        })?;
        let mut rig = arch.joints(&p);
        edit_joints(&mut rig, &u.joints, &path)?;
        let generated = arch.build(&p, &mut rig);
        let (rot, shift) = placement(u, rig.get(info.root), &placed, &path)?;
        for j in &mut rig.joints {
            j.at = rot * j.at + shift;
            j.axis = rot * j.axis;
            j.side = rot * j.side;
        }
        if skeletons.contains_key(&name) {
            return Err(SpecError::new(
                &format!("/skeletons/{name}"),
                format!("`{name}` is an archetype instance, which makes this skeleton; rename one"),
            ));
        }
        skeletons.insert(name.clone(), skeleton_json(&rig, &generated.curves));
        let prefix = u.prefix.clone().unwrap_or_default();
        let local: BTreeSet<String> = generated.parts.iter().map(|(n, _)| n.clone()).collect();
        for (i, o) in u.omit.iter().enumerate() {
            if !local.contains(o) {
                let near = p3d_scene::name::suggest(o, local.iter().map(String::as_str), 3);
                let mut m = format!("`{}` makes no part `{o}`", info.id);
                if !near.is_empty() {
                    m.push_str(&format!("; did you mean {}?", near.join(", ")));
                }
                return Err(SpecError::new(&format!("{path}/omit/{i}"), m));
            }
        }
        let mut listed = Vec::new();
        for (pname, mut pv) in generated.parts {
            if u.omit.contains(&pname) {
                continue;
            }
            rename_refs(&mut pv, &name, &prefix, &local);
            let full = format!("{prefix}{pname}");
            if let Some(other) = made_by.get(&full) {
                return Err(SpecError::new(&path, format!("`{other}` and `{name}` both make part `{full}`; give one a prefix such as \"{name}_\"")));
            }
            if let Some(w) = written.get(&full) {
                pv = layered(pv, w);
            }
            parts.insert(full.clone(), pv);
            made_by.insert(full.clone(), name.clone());
            listed.push(full);
        }
        // The first instance placed on its own sets the genes.
        if genes.is_none() && u.at.is_none() && !generated.dna.is_null() {
            genes = Some(generated.dna);
        }
        reports.push(InstanceReport { name: name.clone(), kind: info.id.to_owned(), joints: rig.joints.len(), parts: listed });
        placed.insert(name, rig);
    }
    let mut out = source.clone();
    if let Value::Object(o) = &mut out {
        o.insert("parts".into(), Value::Object(parts));
        o.insert("skeletons".into(), Value::Object(skeletons));
        if let Some(mut g) = genes {
            if let Some(d) = source.get("dna") {
                crate::patch::merge(&mut g, d);
            }
            o.insert("dna".into(), g);
        }
    }
    Ok(Some((out, reports)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn written_parts_layer_over_generated_ones() {
        let g = json!({ "feature": "head_base", "params": { "brow": 0.4, "jaw": 1.0 }, "anchor": { "on": "@", "landmark": "head" } });
        let merged = layered(g.clone(), &json!({ "params": { "brow": 0.9 } }));
        assert_eq!(merged["params"], json!({ "brow": 0.9, "jaw": 1.0 }));
        assert_eq!(merged["feature"], "head_base");
        let replaced = layered(g, &json!({ "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 } }] }));
        assert!(replaced.get("feature").is_none() && replaced.get("params").is_none() && replaced.get("anchor").is_some(), "{replaced}");
    }

    #[test]
    fn joint_edits_carry_the_joints_below() {
        let mut rig = Rig::default();
        rig.add("a", Vec3::ZERO, None);
        rig.add("b", Vec3::Y, Some("a"));
        rig.add("c", Vec3::X, None);
        let mut edits = BTreeMap::new();
        edits.insert(
            "a".to_owned(),
            JointEdit { by: Some([p3d_ops::params::Length(0.0), p3d_ops::params::Length(0.0), p3d_ops::params::Length(1.0)]), at: None },
        );
        edit_joints(&mut rig, &edits, "").unwrap();
        assert_eq!(rig.at("b"), Vec3::new(0.0, 1.0, 1.0));
        assert_eq!(rig.at("c"), Vec3::X);
        edits.clear();
        edits.insert("x".to_owned(), JointEdit::default());
        assert_eq!(edit_joints(&mut rig, &edits, "/archetypes/d").unwrap_err().path, "/archetypes/d/joints/x");
    }
}
