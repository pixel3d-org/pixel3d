//! Spec ops: `spec.set`, `spec.patch` and `spec.compile`. Each is one
//! undoable transaction that edits a spec's source and recompiles it.

use std::fmt::Write as _;

use glam::Vec3;
use p3d_ops::params::Length;
use p3d_ops::{Category, ErrorCode, Op, OpCtx, OpError, OpExample, OpFlags, OpMeta, OpOutput, Tx, Warning};
use p3d_scene::{Document, ObjectKey, ObjectKind, SpecRecord};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::compile::{CompileReport, PartStatus, compile};
use crate::model::parse;
use crate::patch::{PatchOp, apply as apply_patch, merge};

/// Parts listed one per line in a summary before the rest are counted.
const MAX_LISTED: usize = 12;

fn label(doc: &Document, key: ObjectKey) -> String {
    doc.object(key).map_or_else(|| "?".into(), |o| format!("{} ({})", o.name(), o.public_id()))
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn mm(x: f32) -> String {
    format!("{}", (f64::from(x) * 1000.0).round() as i64)
}

/// The compile report as an op summary: a head line, then one line per part.
fn summarize(doc: &Document, root: ObjectKey, r: &CompileReport) -> OpOutput {
    let unchanged = r.parts.iter().filter(|p| p.status == PartStatus::Unchanged).count();
    let blended = r.parts.iter().filter(|p| matches!(p.status, PartStatus::Blended(_))).count();
    let mut s = format!("spec {}: {} parts, {} built, {} unchanged", label(doc, root), r.parts.len(), r.built(), unchanged);
    if blended > 0 {
        let _ = write!(s, ", {blended} blended");
    }
    if !r.removed.is_empty() {
        let _ = write!(s, "; removed {}", r.removed.join(", "));
    }
    for p in r.parts.iter().take(MAX_LISTED) {
        match &p.status {
            PartStatus::Blended(into) => {
                let _ = write!(s, "\n  {} ({}) blended into {}", p.name, p.kind, into);
            }
            _ => {
                let c = p.center;
                let _ = write!(
                    s,
                    "\n  {} ({}) {} x {} x {} mm at ({}, {}, {}) mm, {} faces",
                    p.name,
                    p.kind,
                    mm(p.size.x),
                    mm(p.size.y),
                    mm(p.size.z),
                    mm(c.x),
                    mm(c.y),
                    mm(c.z),
                    thousands(p.faces)
                );
            }
        }
    }
    if r.parts.len() > MAX_LISTED {
        let _ = write!(s, "\n  ... and {} more", r.parts.len() - MAX_LISTED);
    }
    let mut out = OpOutput::summary(s);
    out.created.extend(r.created.iter().copied());
    out.changed.push(root);
    out.changed.extend(r.changed.iter().copied());
    out.deleted.extend(r.deleted.iter().copied());
    out.warnings.extend(r.warnings.iter().map(|w| Warning { message: w.clone(), fix: None }));
    out
}

fn spec_root(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::new(ErrorCode::NotFound, e.to_string()).at("/target"))?;
    if doc.spec(key).is_none() {
        return Err(OpError::new(ErrorCode::InvalidParam, format!("`{target}` has no spec")).at("/target").with_hint("create one with spec.set"));
    }
    Ok(key)
}

fn check_spec(source: &Value, base: &str) -> Result<(), OpError> {
    let spec = parse(source).map_err(|e| OpError::invalid(&format!("{base}{}", e.path), e.message))?;
    for (name, part) in &spec.parts {
        part.kind(&format!("/parts/{name}")).map_err(|e| OpError::invalid(&format!("{base}{}", e.path), e.message))?;
    }
    Ok(())
}

// ---- spec.set ------------------------------------------------------------------

/// Parameters of `spec.set`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpecSetParams {
    /// Name of the spec's root object; it holds one object per part. An
    /// existing spec root of that name gets the new spec.
    pub name: String,
    /// The Model Spec: `{"parts": {"head": {"sdf": [...]}, "ear_l": {...}}}` (see the spec guide).
    pub spec: Value,
    /// Parent object of a new root; top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Position of a new root in its parent's space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
}

/// `spec.set`.
pub struct SpecSet;

impl Op for SpecSet {
    const ID: &'static str = "spec.set";
    type Params = SpecSetParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set spec",
            category: Category::Spec,
            summary: "Creates or replaces a Model Spec (named parts, shapes, anchors, joins) and compiles it into one object per part.",
            docs: "The spec is the model's source; meshes are build artifacts. Each part has one shape (`sdf`, `primitive`, \
                   `lathe`, `sweep`, `loft`, `extrude`, `pipe`, `mirror` or `instance`), a placement (`transform`, or an `anchor` \
                   on another part's surface: a landmark such as `top` or `ear_root_l`, a `param`, a `ray` or `nearest`), a `join` \
                   (`separate`, `blend` for SDF parts, `decal`) and optional `edits` (ops run on the part). Parts compile in \
                   dependency order; unchanged parts are reused, so later patches are fast. Every face records the spec node \
                   that made it.\n\n\
                   **Use for** building a model from parts and relations instead of op sequences; patch it with `spec.patch`.",
            examples: &[OpExample {
                args: r#"{"name": "lamp", "spec": {"parts": {"base": {"primitive": {"cylinder": {"radius": 0.1, "height": 0.02}}}, "pole": {"primitive": {"cylinder": {"radius": 0.01, "height": 0.4}}, "anchor": {"on": "base", "landmark": "top"}}}}}"#,
                result: "spec lamp (o:1): 2 parts, 2 built, 0 unchanged\n  base (cylinder) 200 x 20 x 200 mm at (0, 10, 0) mm, 112 faces\n  pole (cylinder) 20 x 400 x 20 mm at (0, 220, 0) mm, 32 faces",
            }],
            aliases: &["model spec", "spec", "blueprint", "declarative model", "parts"],
            complexity: "the changed parts' build costs, plus O(mesh size) per part for fingerprints",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SpecSetParams) -> Result<(), OpError> {
        p3d_scene::name::check_name(&p.name).map_err(|e| OpError::invalid("/name", e.to_string()))?;
        if let Some(parent) = &p.parent {
            doc.resolve(parent).map_err(|e| OpError::new(ErrorCode::NotFound, e.to_string()).at("/parent"))?;
        }
        check_spec(&p.spec, "/spec")
    }

    fn apply(tx: &mut Tx, p: &SpecSetParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let parent = p.parent.as_deref().map(|t| tx.resolve(t, "/parent")).transpose()?;
        let mut created = None;
        let root = match tx.doc().child_named(parent, &p.name) {
            Some(k) if tx.doc().spec(k).is_some() => k,
            Some(_) => {
                return Err(OpError::new(ErrorCode::InvalidParam, format!("`{}` exists and is not a spec root", p.name))
                    .at("/name")
                    .with_hint("pick another name"));
            }
            None => {
                let k = tx.doc_mut().create_object(&p.name, parent, ObjectKind::Empty).map_err(|e| OpError::invalid("/name", e.to_string()))?;
                if let (Some(pos), Some(o)) = (p.position, tx.doc_mut().object_mut(k)) {
                    o.transform.translation = Vec3::new(pos[0].0 as f32, pos[1].0 as f32, pos[2].0 as f32);
                }
                created = Some(k);
                k
            }
        };
        let parts = tx.doc().spec(root).map(|r| r.parts.clone()).unwrap_or_default();
        tx.doc_mut().set_spec(root, Some(SpecRecord { source: p.spec.clone(), parts })).map_err(|e| OpError::internal(e.to_string()))?;
        let report = compile(tx, root, ctx, false)?;
        let mut out = summarize(tx.doc(), root, &report);
        out.created.extend(created);
        Ok(out)
    }
}

// ---- spec.patch ----------------------------------------------------------------

/// Parameters of `spec.patch`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpecPatchParams {
    /// The spec's root object (name, path or id).
    pub target: String,
    /// JSON Merge Patch: objects merge, `null` removes a key, other values
    /// replace (`{"parts": {"ear_l": {"blend": "8mm"}}}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<Value>,
    /// JSON Patch operations, for one array element or a move
    /// (`[{"op": "replace", "path": "/parts/head/sdf/1/radii/0", "value": 0.06}]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<PatchOp>>,
}

/// `spec.patch`.
pub struct SpecPatch;

impl SpecPatch {
    fn patched(doc: &Document, p: &SpecPatchParams) -> Result<(ObjectKey, Value), OpError> {
        let root = spec_root(doc, &p.target)?;
        let mut source = doc.spec(root).map(|r| r.source.clone()).unwrap_or_default();
        match (&p.merge, &p.ops) {
            (Some(m), None) => merge(&mut source, m),
            (None, Some(ops)) => apply_patch(&mut source, ops).map_err(|e| OpError::invalid(&e.path, e.message))?,
            _ => return Err(OpError::invalid("/merge", "give either merge (a merge patch) or ops (JSON Patch)")),
        }
        Ok((root, source))
    }
}

impl Op for SpecPatch {
    const ID: &'static str = "spec.patch";
    type Params = SpecPatchParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Patch spec",
            category: Category::Spec,
            summary: "Changes a Model Spec with a merge patch or JSON Patch and recompiles only the parts it affects.",
            docs: "A merge patch is the quick way: `{\"parts\": {\"ear_l\": {\"blend\": \"8mm\"}}}` changes one value, `null` \
                   removes a key or a part. JSON Patch (`ops`) reaches into arrays, such as one SDF shape. Parts whose shape, \
                   edits and blended children are unchanged keep their meshes; a part that only moved keeps its mesh too. Errors \
                   point at the spec path (`/spec/parts/ear_l/anchor/landmark`).\n\n\
                   **Use for** every change to a spec-driven model: the spec stays the source, so recompiles never lose work.",
            examples: &[OpExample {
                args: r#"{"target": "lamp", "merge": {"parts": {"pole": {"primitive": {"cylinder": {"radius": 0.015, "height": 0.5}}}}}}"#,
                result: "spec lamp (o:1): 2 parts, 1 built, 1 unchanged\n  base (cylinder) 200 x 20 x 200 mm at (0, 10, 0) mm, 112 faces\n  pole (cylinder) 30 x 500 x 30 mm at (0, 270, 0) mm, 32 faces",
            }],
            aliases: &["edit spec", "change spec", "update spec", "patch"],
            complexity: "the changed parts' build costs, plus O(mesh size) per part for fingerprints",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SpecPatchParams) -> Result<(), OpError> {
        let (_, source) = Self::patched(doc, p)?;
        check_spec(&source, "/spec")
    }

    fn apply(tx: &mut Tx, p: &SpecPatchParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let (root, source) = Self::patched(tx.doc(), p)?;
        let parts = tx.doc().spec(root).map(|r| r.parts.clone()).unwrap_or_default();
        tx.doc_mut().set_spec(root, Some(SpecRecord { source, parts })).map_err(|e| OpError::internal(e.to_string()))?;
        let report = compile(tx, root, ctx, false)?;
        Ok(summarize(tx.doc(), root, &report))
    }
}

// ---- spec.compile --------------------------------------------------------------

/// Parameters of `spec.compile`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpecCompileParams {
    /// The spec's root object (name, path or id).
    pub target: String,
    /// Rebuild every part, even unchanged ones.
    #[serde(default)]
    pub force: bool,
}

/// `spec.compile`.
pub struct SpecCompile;

impl Op for SpecCompile {
    const ID: &'static str = "spec.compile";
    type Params = SpecCompileParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Compile spec",
            category: Category::Spec,
            summary: "Recompiles a Model Spec: rebuilds changed parts (or all with force) and restores part objects.",
            docs: "`spec.set` and `spec.patch` compile already. Run this after editing part meshes by hand (the edits are \
                   replaced; keep manual work in a part's `edits`), after deleting part objects, or with `force` after an \
                   upgrade.",
            examples: &[OpExample {
                args: r#"{"target": "lamp", "force": true}"#,
                result: "spec lamp (o:1): 2 parts, 2 built, 0 unchanged\n  base (cylinder) 200 x 20 x 200 mm at (0, 10, 0) mm, 112 faces\n  pole (cylinder) 20 x 400 x 20 mm at (0, 220, 0) mm, 32 faces",
            }],
            aliases: &["rebuild spec", "recompile", "build spec"],
            complexity: "the rebuilt parts' build costs",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SpecCompileParams) -> Result<(), OpError> {
        spec_root(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SpecCompileParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let root = spec_root(tx.doc(), &p.target)?;
        let report = compile(tx, root, ctx, p.force)?;
        Ok(summarize(tx.doc(), root, &report))
    }
}
