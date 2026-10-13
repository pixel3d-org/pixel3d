//! Visual DNA critique on a spec (ADR 0011): each critique's remedy becomes
//! a spec patch (a merge patch or JSON Patch for `spec.patch`), and the
//! first few are predicted by applying the patch to a copy of the document
//! and measuring again.

use std::time::Instant;

use glam::{EulerRot, Quat, Vec3};
use p3d_ops::{OpCtx, OpError, Registry, Tx};
use p3d_perceive::dna::{Critique, DnaSpec, Remedy, evaluate, targets};
use p3d_perceive::{Report, View};
use p3d_scene::{Document, ObjectKey};
use serde_json::{Value, json};

use crate::model::{ModelSpec, Part, Scale, parse};

/// Critiques predicted per call; later ones say how to predict them.
pub const MAX_PREDICTIONS: usize = 4;

/// A critique with the spec patch that answers it and its predicted result.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Fix {
    pub code: &'static str,
    pub subject: String,
    pub parts: Vec<String>,
    /// `spec.patch` arguments without the target: `{"merge": ...}` or
    /// `{"ops": [...]}`.
    pub patch: Value,
    /// `resolved`, or what remains, with the genes on target before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicted: Option<String>,
}

fn n3(v: Vec3) -> Value {
    let r = |x: f32| (f64::from(x) * 1e5).round() / 1e5;
    json!([r(v.x), r(v.y), r(v.z)])
}

/// The spec part name of a view label (`elf/ear_l` or `ear_l`).
fn name_of(label: &str) -> &str {
    label.rsplit('/').next().unwrap_or(label)
}

/// What a spec patch needs to know about the spec on a root.
struct Ctx<'a> {
    doc: &'a Document,
    root: ObjectKey,
    spec: ModelSpec,
    source: Value,
    /// Generated parts by instance name.
    made: Vec<crate::archetype::InstanceReport>,
}

impl Ctx<'_> {
    fn part(&self, name: &str) -> Option<&Part> {
        self.spec.parts.get(name)
    }

    /// The part a move or scale should go to: the source of a mirror.
    fn editable<'b>(&'b self, name: &'b str) -> (&'b str, bool) {
        match self.part(name).and_then(|p| p.mirror.as_deref()) {
            Some(src) => (src, true),
            None => (name, false),
        }
    }

    /// The archetype instance that generated a part.
    fn instance_of(&self, part: &str) -> Option<&crate::archetype::InstanceReport> {
        self.made.iter().find(|i| i.parts.iter().any(|p| p == part))
    }

    /// The part object's world rotation.
    fn rotation(&self, part: &str) -> Quat {
        let k = self.doc.object(self.root).and_then(|o| o.children().iter().copied().find(|&k| self.doc.object(k).is_some_and(|c| c.name() == part)));
        k.and_then(|k| self.doc.world_affine(k)).map_or(Quat::IDENTITY, |a| a.to_scale_rotation_translation().1)
    }

    fn origin(&self, part: &str) -> Option<Vec3> {
        let k = self.doc.object(self.root)?.children().iter().copied().find(|&k| self.doc.object(k).is_some_and(|c| c.name() == part))?;
        Some(self.doc.world_affine(k)?.translation.into())
    }

    fn root_rotation(&self) -> Quat {
        self.doc.world_affine(self.root).map_or(Quat::IDENTITY, |a| a.to_scale_rotation_translation().1)
    }

    /// The part's own `transform.rotate` as a rotation.
    fn own_rotation(&self, part: &str) -> Quat {
        self.part(part)
            .and_then(|p| p.transform.as_ref())
            .and_then(|t| t.rotate)
            .map_or(Quat::IDENTITY, |r| Quat::from_euler(EulerRot::XYZ, r[0].0 as f32, r[1].0 as f32, r[2].0 as f32))
    }

    /// A merge patch moving a part by a world offset: its anchor offset
    /// (in the anchor's frame) or its position (in the root's space).
    fn move_patch(&self, name: &str, by: Vec3) -> Value {
        let (target, mirrored) = self.editable(name);
        let by = if mirrored { by * Vec3::new(-1.0, 1.0, 1.0) } else { by };
        let Some(p) = self.part(target) else { return Value::Null };
        if let Some(a) = &p.anchor {
            let frame = self.rotation(target) * self.own_rotation(target).inverse();
            let old = a.offset.as_ref().map_or(Vec3::ZERO, |o| Vec3::new(o[0].0 as f32, o[1].0 as f32, o[2].0 as f32));
            return json!({ "parts": { target: { "anchor": { "offset": n3(old + frame.inverse() * by) } } } });
        }
        let old =
            p.transform.as_ref().and_then(|t| t.position.as_ref()).map_or(Vec3::ZERO, |o| Vec3::new(o[0].0 as f32, o[1].0 as f32, o[2].0 as f32));
        json!({ "parts": { target: { "transform": { "position": n3(old + self.root_rotation().inverse() * by) } } } })
    }

    fn scale_patch(&self, name: &str, factor: f32) -> Value {
        let (target, _) = self.editable(name);
        let old = self.part(target).and_then(|p| p.transform.as_ref()).and_then(|t| t.scale).map_or([1.0; 3], Scale::vec);
        let s: Vec<f64> = old.iter().map(|x| (x * f64::from(factor) * 1e4).round() / 1e4).collect();
        let scale = if s.iter().all(|x| (x - s[0]).abs() < 1e-9) { json!(s[0]) } else { json!(s) };
        json!({ "parts": { target: { "transform": { "scale": scale } } } })
    }

    /// An archetype parameter change for a generated part, when its
    /// instance has the parameter.
    fn param_patch(&self, instance: &str, param: &str, value: Value) -> Option<Value> {
        let kind = self.spec.archetypes.get(instance)?.kind.clone();
        let arch = crate::archetype::get(&kind).ok()?;
        arch.info().params.iter().any(|p| p.name == param).then(|| json!({ "archetypes": { instance: { "params": { param: value } } } }))
    }

    fn current_param(&self, instance: &str, param: &str) -> Option<f64> {
        let given = self.source.pointer(&format!("/archetypes/{instance}/params/{param}")).and_then(Value::as_f64);
        given.or_else(|| {
            let kind = self.spec.archetypes.get(instance)?.kind.clone();
            let arch = crate::archetype::get(&kind).ok()?;
            arch.info().params.iter().find(|p| p.name == param).map(|p| p.default)
        })
    }

    /// The spec patch for a remedy, or none.
    fn patch(&self, r: &Remedy) -> Option<Value> {
        Some(match r {
            Remedy::Scale { part, factor } => json!({ "merge": self.scale_patch(name_of(part), *factor) }),
            Remedy::Move { part, by } => {
                let m = self.move_patch(name_of(part), *by);
                if m.is_null() {
                    return None;
                }
                json!({ "merge": m })
            }
            Remedy::Bend { part, by, along } => {
                let name = name_of(part);
                // A generated blade bends through its archetype's curve.
                if let Some(i) = self.instance_of(name)
                    && let Some(p) = self.param_patch(&i.name, "curve", json!(12))
                {
                    return Some(json!({ "merge": p }));
                }
                let (target, _) = self.editable(name);
                let k = self.doc.object(self.root)?.children().iter().copied().find(|&k| self.doc.object(k).is_some_and(|c| c.name() == target))?;
                let (lo, hi) = self.doc.object_mesh(k)?.bounds()?;
                let to_part = self.doc.world_affine(k)?.inverse();
                let c = (lo + hi) * 0.5;
                let half = (hi - lo).max_element() * 0.5;
                let ax = to_part.transform_vector3(*along).normalize_or(Vec3::Y);
                json!({ "merge": { "parts": { target: { "deform": {
                    "handles": [{ "move": n3(c), "by": n3(to_part.transform_vector3(*by)) }],
                    "pins": [n3(c - ax * half), n3(c + ax * half)],
                } } } } })
            }
            Remedy::Space { parts, .. } => {
                let names: Vec<&str> = parts.iter().map(|p| name_of(p)).collect();
                let (first, last) = (self.origin(names[0])?, self.origin(names[names.len() - 1])?);
                let inv = self.doc.world_affine(self.root)?.inverse();
                json!({ "ops": [{ "op": "add", "path": "/constraints/-", "value": { "distribute": {
                    "parts": names, "from": n3(inv.transform_point3(first)), "to": n3(inv.transform_point3(last)), "spacing": "varied"
                } } }] })
            }
            Remedy::Proportion { gene, measured, target } => {
                if let Some((a, _)) = gene.split_once("_to_") {
                    return Some(json!({ "merge": self.scale_patch(a, target / measured.max(1e-6)) }));
                }
                // A humanoid measure: the archetype parameter of that name.
                let inst = self.spec.archetypes.keys().find(|i| self.param_patch(i, gene, json!(0)).is_some())?.clone();
                let now = self.current_param(&inst, gene)?;
                let value = (now * f64::from(*target / measured.max(1e-6)) * 100.0).round() / 100.0;
                json!({ "merge": self.param_patch(&inst, gene, json!(value))? })
            }
            Remedy::Remove { parts } => {
                let mut merge = json!({});
                for p in parts {
                    let name = name_of(p);
                    match self.instance_of(name) {
                        Some(i) => {
                            let mut omit: Vec<Value> =
                                self.source.pointer(&format!("/archetypes/{}/omit", i.name)).and_then(Value::as_array).cloned().unwrap_or_default();
                            if let Some(Value::Array(more)) = merge.pointer(&format!("/archetypes/{}/omit", i.name)) {
                                omit = more.clone();
                            }
                            omit.push(json!(name));
                            crate::patch::merge(&mut merge, &json!({ "archetypes": { i.name.clone(): { "omit": omit } } }));
                        }
                        None => crate::patch::merge(&mut merge, &json!({ "parts": { name: Value::Null } })),
                    }
                }
                json!({ "merge": merge })
            }
        })
    }
}

/// Genes on target and critique codes of a document's spec root.
fn score(doc: &Document, root: &str, dna: &DnaSpec) -> Option<(usize, usize, Vec<Critique>)> {
    let view = View::new(doc, Some(root)).ok()?;
    let r = p3d_perceive::dna::report(&view, dna).ok()?;
    let on = r.data.get("on_target")?.as_u64()? as usize;
    let genes = r.data.get("genes")?.as_u64()? as usize;
    let t = targets(dna).ok()?;
    Some((on, genes, evaluate(&view, &t).1))
}

/// The `dna` report of a spec root with a fix (spec patch) per critique
/// and the predicted result of the first [`MAX_PREDICTIONS`].
///
/// Complexity: one measurement, plus per predicted fix a recompile of the
/// patched parts and one more measurement.
pub fn report(doc: &Document, root: ObjectKey, dna: &DnaSpec, registry: &Registry, seed: u64) -> Result<Report, OpError> {
    let path = doc.path(root).unwrap_or_default();
    let view = View::new(doc, Some(&path)).map_err(|e| OpError::invalid("/target", e.message))?;
    let mut report = p3d_perceive::dna::report(&view, dna).map_err(|(p, m)| OpError::invalid(&format!("/dna{p}"), m))?;
    let t = targets(dna).map_err(|(p, m)| OpError::invalid(&format!("/dna{p}"), m))?;
    let (_, critiques) = evaluate(&view, &t);
    let Some(record) = doc.spec(root) else { return Ok(report) };
    let source = record.source.clone();
    let Ok(spec) = parse(&source) else { return Ok(report) };
    let made = crate::archetype::expand_report(&source).ok().flatten().map(|(_, r)| r).unwrap_or_default();
    let ctx = Ctx { doc, root, spec, source, made };
    let before = score(doc, &path, dna);
    let mut fixes = Vec::new();
    let mut budget = MAX_PREDICTIONS;
    for c in &critiques {
        let Some(patch) = c.remedy.as_ref().and_then(|r| ctx.patch(r)) else { continue };
        let predicted = if budget > 0 {
            budget -= 1;
            Some(predict(doc, &path, dna, &patch, c, before.as_ref(), registry, seed))
        } else {
            Some("not predicted (ask for the dna of this part alone)".to_owned())
        };
        fixes.push(Fix { code: c.code, subject: c.subject.clone(), parts: c.parts.clone(), patch, predicted });
    }
    if !fixes.is_empty() {
        report.text.push_str("fixes (spec patch, predicted):\n");
        for f in &fixes {
            report.text.push_str(&format!(
                "  {} {}: {} -> {}\n",
                f.code,
                f.subject,
                serde_json::to_string(&f.patch).unwrap_or_default(),
                f.predicted.as_deref().unwrap_or("")
            ));
        }
    }
    if let Value::Object(o) = &mut report.data {
        o.insert("fixes".into(), serde_json::to_value(&fixes).unwrap_or(Value::Null));
    }
    Ok(report)
}

/// Applies a patch to a copy of the document and measures again.
#[allow(clippy::too_many_arguments)]
fn predict(
    doc: &Document,
    root: &str,
    dna: &DnaSpec,
    patch: &Value,
    c: &Critique,
    before: Option<&(usize, usize, Vec<Critique>)>,
    registry: &Registry,
    seed: u64,
) -> String {
    let start = Instant::now();
    let mut args = patch.clone();
    args["target"] = json!(root);
    let mut tx = Tx::new(doc.clone());
    let ran = registry.get("spec.patch").and_then(|op| op.run(&mut tx, &args, &OpCtx { seed, registry: Some(registry), ..OpCtx::default() }));
    if let Err(e) = ran {
        return format!("fails: {} {}", e.code.as_str(), e.message);
    }
    // The patch may carry new DNA genes: measure against the patched spec's.
    let root_key = tx.doc().resolve(root).ok();
    let dna = root_key.and_then(|k| crate::dna_of(tx.doc(), k)).unwrap_or_else(|| dna.clone());
    let Some((on, genes, after)) = score(tx.doc(), root, &dna) else { return "cannot measure the result".into() };
    let same = |d: &Critique| d.code == c.code && d.subject == c.subject;
    let mut out = match after.iter().find(|d| same(d)) {
        None => "resolved".to_owned(),
        Some(d) => format!("remains: {}", d.message),
    };
    if let Some((on0, genes0, crit0)) = before {
        out.push_str(&format!("; dna {on0}/{genes0} -> {on}/{genes}"));
        let new: Vec<&str> =
            after.iter().filter(|d| !crit0.iter().any(|e| e.code == d.code && e.subject == d.subject)).map(|d| d.code).take(2).collect();
        if !new.is_empty() {
            out.push_str(&format!("; new {}", new.join(", ")));
        }
    }
    tracing::debug!(code = c.code, ms = start.elapsed().as_millis() as u64, "dna fix predicted");
    out
}
