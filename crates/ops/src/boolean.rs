//! `edit.boolean`: union, difference and intersection of mesh objects with
//! the cleanup game assets need (kernel in `p3d_geo::boolean`, ADR 0008).

use glam::Affine3A;
use p3d_geo::boolean::{BoolOp, BooleanError, BooleanSpec, ManifoldEngine, boolean};
use p3d_mesh::{ElementKind, Mesh};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_length, group_label, label, mesh_target, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, GroupInfo, Op, OpCall, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::Length;
use crate::tx::Tx;

/// Below this many faces a target counts as low-poly for the curved-cut warning.
const LOW_POLY_FACES: usize = 5_000;

/// What the boolean keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoolOpParam {
    /// Cut the tools out of the target.
    #[default]
    Difference,
    /// Add the tools to the target.
    Union,
    /// Keep only what the target and the tools share.
    Intersect,
}

/// One object or several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ToolsParam {
    One(String),
    Many(Vec<String>),
}

impl ToolsParam {
    fn list(&self) -> Vec<&str> {
        match self {
            ToolsParam::One(s) => vec![s.as_str()],
            ToolsParam::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }

    fn path(&self, i: usize) -> String {
        match self {
            ToolsParam::One(_) => "/tool".to_owned(),
            ToolsParam::Many(_) => format!("/tool/{i}"),
        }
    }
}

impl Default for ToolsParam {
    fn default() -> Self {
        ToolsParam::One(String::new())
    }
}

/// Parameters of `edit.boolean`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BooleanParams {
    /// Mesh object that receives the result (name, path or id).
    pub target: String,
    /// The mesh object to cut with, add or intersect, or a list of them (applied in order).
    pub tool: ToolsParam,
    /// `difference` (default: cut the tools out), `union` or `intersect`.
    #[serde(default)]
    pub operation: BoolOpParam,
    /// Keep the tool objects; by default they are deleted (their children stay).
    #[serde(default)]
    pub keep_tool: bool,
    /// Vertices the cut makes closer than this merge, so no sliver edges remain; 0 keeps them.
    #[serde(default = "weld")]
    pub weld: Length,
}

fn weld() -> Length {
    Length(1e-4)
}

impl Default for BooleanParams {
    fn default() -> Self {
        BooleanParams { target: String::new(), tool: ToolsParam::default(), operation: BoolOpParam::Difference, keep_tool: false, weld: weld() }
    }
}

/// `edit.boolean`.
pub struct Boolean;

/// The tool objects, checked: meshes, each once, none the target.
fn tool_keys(doc: &Document, p: &BooleanParams, target: ObjectKey) -> Result<Vec<ObjectKey>, OpError> {
    let list = p.tool.list();
    if list.is_empty() {
        return Err(OpError::invalid("/tool", "give at least one tool object"));
    }
    let mut keys: Vec<ObjectKey> = Vec::with_capacity(list.len());
    for (i, t) in list.iter().enumerate() {
        let path = p.tool.path(i);
        let k = mesh_target(doc, t).map_err(|e| e.at(&path))?;
        if k == target {
            return Err(OpError::invalid(&path, format!("{} is the target; pick another object to cut with", label(doc, k))));
        }
        if keys.contains(&k) {
            return Err(OpError::invalid(&path, format!("{} is listed twice", label(doc, k))));
        }
        keys.push(k);
    }
    Ok(keys)
}

/// Maps a kernel refusal to an op error.
fn boolean_error(e: BooleanError, doc: &Document, target: ObjectKey, tools: &[ObjectKey], p: &BooleanParams) -> OpError {
    match e {
        BooleanError::NotClosed { operand, open } => {
            let (key, path) = match operand {
                0 => (target, "/target".to_owned()),
                i => (tools.get(i - 1).copied().unwrap_or(target), p.tool.path(i - 1)),
            };
            OpError::new(
                ErrorCode::NonManifoldInput,
                format!("{} is not closed: {} open edge{}", label(doc, key), thousands(open), if open == 1 { "" } else { "s" }),
            )
            .at(&path)
            .with_hint("a boolean needs a closed volume; close the holes with edit.fill or cleanup.auto first")
        }
        BooleanError::TooLarge(n) => OpError::new(ErrorCode::BudgetExceeded, format!("{} triangles is more than one boolean takes", thousands(n)))
            .with_hint("decimate the objects or cut with fewer tools at once"),
        BooleanError::Empty => {
            let why = match p.operation {
                BoolOpParam::Intersect => "the objects do not overlap",
                _ => "the tools cover the whole target",
            };
            OpError::new(ErrorCode::Topology, format!("the result would be empty: {why}"))
                .at("/tool")
                .with_hint("move the tools or pick another operation")
        }
        BooleanError::Engine(m) => OpError::new(ErrorCode::Topology, format!("the boolean failed: {m}"))
            .with_hint("the objects may intersect themselves; run cleanup.auto on them, or move the tools slightly"),
    }
}

impl Op for Boolean {
    const ID: &'static str = "edit.boolean";
    type Params = BooleanParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Boolean",
            category: Category::Edit,
            summary: "Cuts other mesh objects out of a mesh, adds them to it, or keeps only the overlap, with clean polygons.",
            docs: "Combines the target with the tool objects as closed volumes (exact arithmetic, so touching and coplanar faces \
                   work). `difference` (default) cuts the tools out, `union` adds them, `intersect` keeps the overlap. The tools \
                   are deleted unless `keep_tool` is set.\n\n\
                   The result is cleaned up for game assets: faces split by the cut merge back into polygons, faces with holes \
                   are refilled with quads, cut points left on straight edges and sliver edges shorter than `weld` disappear. \
                   UVs and colors are interpolated, and materials, sharp and seam flags, creases and the target's groups carry \
                   over. Faces that came from the tools are `<label>.cut`, the edges where they meet the target `<label>.edges`. \
                   Both objects must be closed.\n\n\
                   **Use for** holes, slots, sockets and notches in hard-surface props, merging blockout parts into one shell, \
                   trimming a shape to a volume. Cut low-poly curved surfaces sparingly: the facets show along the cut.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "tool": "drill"}"#,
                result: "crate (o:1): difference with drill (o:11): 28 faces (8 cut); deleted drill; groups boolean_1.cut (8), boolean_1.edges (16)",
            }],
            aliases: &["boolean modifier", "bool tool", "csg", "subtract", "difference", "union", "intersect", "carve", "cut hole", "polyBoolOp"],
            complexity: "O((n + k) log n) for n triangles and k intersections, plus O(n log n) cleanup",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &BooleanParams) -> Result<(), OpError> {
        let target = mesh_target(doc, &p.target)?;
        tool_keys(doc, p, target)?;
        check_length("/weld", p.weld, 0.0, 1.0)
    }

    fn apply(tx: &mut Tx, p: &BooleanParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let tools = tool_keys(tx.doc(), p, key)?;
        let world = tx.doc().world_affine(key).ok_or_else(|| OpError::internal("object vanished"))?;
        if world.matrix3.determinant().abs() < f32::EPSILON {
            return Err(OpError::new(ErrorCode::Unsupported, format!("{} has a singular transform", label(tx.doc(), key))));
        }
        let inv = world.inverse();
        let mut moved: Vec<Mesh> = Vec::with_capacity(tools.len());
        for (i, &k) in tools.iter().enumerate() {
            let rel: Affine3A = inv * tx.doc().world_affine(k).ok_or_else(|| OpError::internal("object vanished"))?;
            let mut m = tx.mesh(k, &p.tool.path(i))?.clone();
            m.transform(rel);
            moved.push(m);
        }
        ctx.check_cancel()?;
        let spec = BooleanSpec {
            op: match p.operation {
                BoolOpParam::Difference => BoolOp::Difference,
                BoolOpParam::Union => BoolOp::Union,
                BoolOpParam::Intersect => BoolOp::Intersect,
            },
            weld: p.weld.0 as f32,
        };
        let refs: Vec<&Mesh> = moved.iter().collect();
        let target = tx.mesh(key, "/target")?;
        let glabel = group_label(target, ctx, "boolean");
        let before = target.face_count();
        let r = boolean(target, &refs, &spec, &ManifoldEngine).map_err(|e| boolean_error(e, tx.doc(), key, &tools, p))?;
        ctx.check_cancel()?;
        let names: Vec<String> = tools.iter().map(|&k| label(tx.doc(), k)).collect();
        let paths: Vec<String> = tools.iter().map(|&k| tx.doc().path(k).unwrap_or_default()).collect();
        let what = match p.operation {
            BoolOpParam::Difference => "difference with",
            BoolOpParam::Union => "union with",
            BoolOpParam::Intersect => "intersection with",
        };
        let mut out = OpOutput::default();
        if !r.touched {
            out.summary =
                format!("{}: {} miss{} it; nothing changed", label(tx.doc(), key), names.join(", "), if names.len() == 1 { "es" } else { "" });
            return Ok(out);
        }
        let curved = r.curved > 0 && before < LOW_POLY_FACES;
        let mesh = tx.mesh_mut(key, "/target")?;
        *mesh = r.mesh;
        let cut_name = format!("{glabel}.cut");
        let n = mesh.set_group(&cut_name, ElementKind::Face, r.cut.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
        out.groups.push(GroupInfo { object: key, name: cut_name, kind: ElementKind::Face, count: n as u32 });
        let edges_name = format!("{glabel}.edges");
        let n = mesh.set_group(&edges_name, ElementKind::Edge, r.edges.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
        out.groups.push(GroupInfo { object: key, name: edges_name, kind: ElementKind::Edge, count: n as u32 });
        let faces = mesh.face_count();
        out.changed.push(key);
        if curved {
            out.warnings.push(Warning {
                message: format!(
                    "the cut crosses {} curved face{} of a low-poly mesh; its facets show along the cut (more segments on the target, or a smaller cutter, hide them)",
                    r.curved,
                    if r.curved == 1 { "" } else { "s" }
                ),
                fix: None,
            });
        }
        if r.normals_cleared {
            out.warnings.push(Warning {
                message: "custom normals were cleared because they no longer fit the cut surface".into(),
                fix: Some(OpCall { op: "shade.weighted_normals".into(), args: serde_json::json!({ "target": p.target }) }),
            });
        }
        let deleted = if p.keep_tool {
            String::new()
        } else {
            for (i, &k) in tools.iter().enumerate() {
                let ids = tx.doc_mut().delete(k, false).map_err(|e| OpError::from_scene(e, &p.tool.path(i)))?;
                out.deleted.extend(ids);
            }
            format!("; deleted {}", paths.join(", "))
        };
        let groups: Vec<String> = out.groups.iter().map(|g| format!("{} ({})", g.name, g.count)).collect();
        out.summary = format!(
            "{}: {what} {}: {} faces ({} cut){deleted}; groups {}",
            label(tx.doc(), key),
            names.join(", "),
            thousands(faces),
            thousands(r.cut.len()),
            groups.join(", ")
        );
        Ok(out)
    }
}
