//! Shapes drawn as 2D outlines: `gen.shell` (a curved sheet from one
//! outline: ears, leaves, wings, fins, capes) and `sdf.silhouettes` (a
//! solid from two or three orthographic outlines, like a blueprint).
//! Outlines are SVG path data in meters or point lists. With `target`
//! either rebuilds a mesh object in place, so its history holds the
//! outlines to edit and run again.

use glam::{DVec2, Vec2};
use p3d_geo::generate::svg::parse_path;
use p3d_geo::generate::{ShellSpec, shell};
use p3d_mesh::Mesh;
use p3d_scene::Document;
use p3d_sdf::{MeshSpec, OutlineField, View, mesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_length, check_new_object, create_mesh_object, dims, label, mesh_groups, mesh_target, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length};
use crate::prim::created_summary;
use crate::tx::Tx;

/// An outline: SVG path data (`"M 0 0 C 0.02 0.03 ..."`, meters, y up) or
/// points `[[x, y], ...]` forming one closed contour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OutlineParam {
    Path(String),
    Points(Vec<[Length; 2]>),
}

impl Default for OutlineParam {
    fn default() -> Self {
        OutlineParam::Points(Vec::new())
    }
}

impl OutlineParam {
    /// The closed contours, curves flattened at `step`.
    ///
    /// Complexity: O(path length + points).
    pub fn contours(&self, path: &str, step: f64) -> Result<Vec<Vec<DVec2>>, OpError> {
        match self {
            OutlineParam::Path(d) => parse_path(d, step).map_err(|e| OpError::invalid(path, e.to_string())),
            OutlineParam::Points(p) if p.len() >= 3 => Ok(vec![p.iter().map(|q| DVec2::new(q[0].0, q[1].0)).collect()]),
            OutlineParam::Points(_) => {
                Err(OpError::invalid(path, "an outline needs at least three points").with_hint("[[0, 0], [0.04, 0.05], [0, 0.1]] or SVG path data"))
            }
        }
    }
}

/// Size of a set of contours (longest side of their bounds).
fn extent(contours: &[Vec<DVec2>]) -> f64 {
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in contours.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (hi - lo).max_element().max(0.0)
}

const MIN_DETAIL: f64 = 1e-4;
const MAX_DETAIL: f64 = 1.0;

/// Checks where the mesh goes: a new object, or `target` rebuilt in place.
fn check_place(doc: &Document, target: Option<&str>, name: Option<&str>, parent: Option<&str>, position: &[Length; 3]) -> Result<(), OpError> {
    match target {
        Some(t) => {
            if name.is_some() || parent.is_some() || position.iter().any(|l| l.0 != 0.0) {
                return Err(OpError::invalid("/target", "name, parent and position are for a new object; leave them out to rebuild the target"));
            }
            mesh_target(doc, t).map(|_| ())
        }
        None => check_new_object(doc, name, parent).map(|_| ()),
    }
}

/// Puts the mesh in a new object, or in `target` (which keeps its name,
/// transform, parent and children).
///
/// Complexity: O(V + F) of the mesh for the summary.
fn place(
    tx: &mut Tx,
    target: Option<&str>,
    name: Option<&str>,
    parent: Option<&str>,
    position: &[Length; 3],
    kind: &str,
    m: Mesh,
) -> Result<OpOutput, OpError> {
    let Some(t) = target else {
        let key = create_mesh_object(tx, name, kind, parent, position, m)?;
        return Ok(created_summary(tx, key, kind));
    };
    let key = mesh_target(tx.doc(), t)?;
    *tx.mesh_mut(key, "/target")? = m;
    let doc = tx.doc();
    let (dims, faces, tris) = doc.object_mesh(key).map_or_else(|| ("?".into(), 0, 0), |m| (dims(m), m.face_count(), m.triangle_count()));
    let mut out = OpOutput::summary(format!(
        "rebuilt {} {kind} {dims}: {} faces ({} tris; budget {})",
        label(doc, key),
        thousands(faces),
        thousands(tris),
        thousands(doc.settings.tri_budget() as usize)
    ));
    out.changed.push(key);
    out.groups = mesh_groups(doc, key);
    Ok(out)
}

// ---- gen.shell -------------------------------------------------------------------

/// Parameters of `gen.shell`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellParams {
    /// Object name; a free name such as `shell_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The outline in the shell's plane: x across, y along its length with the base at the lowest y.
    pub outline: OutlineParam,
    /// Outlines cut out of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<OutlineParam>,
    /// Thickness; 0 (the default) makes a one-sided card facing +Z.
    #[serde(default)]
    pub thickness: Length,
    /// Edges curl toward +Z by this share of the half width (negative curls back).
    #[serde(default)]
    pub cup: f64,
    /// The tip lifts toward +Z by this share of the length.
    #[serde(default)]
    pub bend: f64,
    /// Turn about the length axis at the tip.
    #[serde(default)]
    pub twist: Angle,
    /// Edge length of the sheet; a 16th of the outline's longer side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Length>,
    /// Origin position in the parent's space (the outline's origin).
    #[serde(default)]
    pub position: [Length; 3],
    /// Mesh object to rebuild in place from the outline (it keeps its name, transform, parent and children) instead of making a new object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// `gen.shell`.
pub struct GenShell;

impl GenShell {
    fn spec(p: &ShellParams) -> Result<ShellSpec, OpError> {
        let rough = p.outline.contours("/outline", 1e-3)?;
        let size = extent(&rough);
        let spacing = p.detail.map_or((size / 16.0).clamp(MIN_DETAIL, MAX_DETAIL), |d| d.0);
        let mut contours = p.outline.contours("/outline", spacing / 2.0)?;
        for (i, h) in p.holes.iter().enumerate() {
            contours.extend(h.contours(&format!("/holes/{i}"), spacing / 2.0)?);
        }
        Ok(ShellSpec { contours, spacing, thickness: p.thickness.0, cup: p.cup, bend: p.bend, twist: p.twist.0 })
    }
}

impl Op for GenShell {
    const ID: &'static str = "gen.shell";
    type Params = ShellParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Shell",
            category: Category::Gen,
            summary: "Builds a curved sheet from one outline (cup, bend, twist), as a card or a closed shell: ears, leaves, petals, wings, fins.",
            docs: "The outline lies in the XY plane: x across, y along the length, base at the lowest y (where the shell \
                   attaches). It is filled with an even, mostly quad sheet, then `cup` curls its edges (a share of the half \
                   width), `bend` lifts its tip (a share of the length) and `twist` turns it toward the tip. With `thickness` it \
                   becomes a closed solid with groups `front`, `back` and `rim`; without, a one-sided card facing +Z (group \
                   `front`) for alpha-textured foliage, feathers and hair. Outlines are SVG path data in meters (curves welcome) \
                   or point lists; `holes` cut it. With `target` the outline rebuilds that mesh object in place (it keeps its \
                   name, transform, parent and children), so its history holds the outline to edit and run again.\n\n\
                   **Use for** thin organic parts an agent can describe with an outline and three numbers: an elf ear with \
                   `cup` 0.4 and a little `bend`, leaf cards, membrane wings, fins, cloth flaps.",
            examples: &[OpExample {
                args: r#"{"name": "leaf", "outline": "M 0 0 C 0.03 0.02 0.025 0.07 0 0.1 C -0.025 0.07 -0.03 0.02 0 0 Z", "cup": 0.3, "bend": 0.2, "detail": "5mm"}"#,
                result: "created leaf (o:4) shell 41.33 mm x 0.1 m x 20 mm: 198 faces (314 tris; budget 6,000)",
            }],
            aliases: &["leaf", "card", "ear", "wing", "fin", "petal", "membrane", "sheet", "cloth flap", "solidify outline"],
            complexity: "O(n log n) in sheet points (about the outline's area / detail^2)",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ShellParams) -> Result<(), OpError> {
        check_place(doc, p.target.as_deref(), p.name.as_deref(), p.parent.as_deref(), &p.position)?;
        if let Some(d) = p.detail {
            check_length("/detail", d, MIN_DETAIL, MAX_DETAIL)?;
        }
        if p.thickness.0 != 0.0 {
            check_length("/thickness", p.thickness, 1e-5, 1.0)?;
        }
        Self::spec(p).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &ShellParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = Self::spec(p)?;
        let m = shell(&spec).map_err(|e| OpError::invalid("/outline", e.to_string()))?;
        place(tx, p.target.as_deref(), p.name.as_deref(), p.parent.as_deref(), &p.position, "shell", m)
    }
}

// ---- sdf.silhouettes -------------------------------------------------------------

/// Parameters of `sdf.silhouettes`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SilhouettesParams {
    /// Object name; a free name such as `silhouettes_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Outline seen from the front (+Z): x right, y up.
    pub front: OutlineParam,
    /// Outline seen from the side (+X): z right (front), y up; give side, top or both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<OutlineParam>,
    /// Outline seen from the top (+Y): x right, z up (front).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<OutlineParam>,
    /// Rounds every edge by this radius.
    #[serde(default)]
    pub round: Length,
    /// Voxel size, about the edge length; a 64th of the largest side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Length>,
    /// Origin position in the parent's space.
    #[serde(default)]
    pub position: [Length; 3],
    /// Mesh object to rebuild in place from the outlines (it keeps its name, transform, parent and children) instead of making a new object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// `sdf.silhouettes`.
pub struct SdfSilhouettes;

impl SdfSilhouettes {
    fn field(p: &SilhouettesParams) -> Result<(OutlineField, f32), OpError> {
        let mut views = Vec::new();
        let mut size = 0.0f64;
        let front = Some(p.front.clone());
        for (view, key, o) in [(View::Front, "front", &front), (View::Side, "side", &p.side), (View::Top, "top", &p.top)] {
            if let Some(o) = o {
                let rough = o.contours(&format!("/{key}"), 1e-3)?;
                size = size.max(extent(&rough));
                views.push((view, key, o));
            }
        }
        let voxel = p.detail.map_or((size / 64.0).clamp(MIN_DETAIL, MAX_DETAIL), |d| d.0);
        let mut outlines = Vec::with_capacity(views.len());
        for (view, key, o) in views {
            let c = o.contours(&format!("/{key}"), voxel)?;
            outlines.push((view, c.into_iter().map(|c| c.into_iter().map(|q| Vec2::new(q.x as f32, q.y as f32)).collect()).collect()));
        }
        let field = OutlineField::new(&outlines, p.round.0 as f32).map_err(|e| OpError::invalid("/front", e.to_string()))?;
        Ok((field, voxel as f32))
    }
}

impl Op for SdfSilhouettes {
    const ID: &'static str = "sdf.silhouettes";
    type Params = SilhouettesParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Silhouettes",
            category: Category::Sdf,
            summary: "Builds a solid from two or three orthographic outlines (front, side, top), like a blueprint, with rounded edges.",
            docs: "Each outline is extruded straight through the part and the extrusions are intersected (the visual hull), \
                   then every edge is rounded by `round`. Draw what the part looks like from the front and from the side (and \
                   the top when needed) as SVG path data in meters or point lists: an axe head, a helmet, a sword guard, a \
                   creature's body plan. The result is an even closed quad mesh; refine it with `remesh.quad`, details with \
                   other parts. With `target` the outlines rebuild that mesh object in place (it keeps its name, transform, \
                   parent and children), so its history holds the outlines to edit and run again.\n\n\
                   **Use for** turning concept outlines into a blockout without guessing depth.",
            examples: &[OpExample {
                args: r#"{"name": "axe_head", "front": "M -0.02 0 L 0.12 -0.06 C 0.16 0 0.16 0.08 0.12 0.14 L -0.02 0.08 Z", "side": [[-0.015, -0.06], [0.015, -0.06], [0.015, 0.14], [-0.015, 0.14]], "round": "3mm", "detail": "4mm"}"#,
                result: "created axe_head (o:4) silhouettes 0.1704 m x 0.1983 m x 30.71 mm: 4,240 faces (8,480 tris; budget 6,000)",
            }],
            aliases: &["blueprint", "visual hull", "front and side view", "outline extrude", "boolean of extrusions"],
            complexity: "O(active voxels x outline edges)",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::LONG_RUNNING),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SilhouettesParams) -> Result<(), OpError> {
        check_place(doc, p.target.as_deref(), p.name.as_deref(), p.parent.as_deref(), &p.position)?;
        if let Some(d) = p.detail {
            check_length("/detail", d, MIN_DETAIL, MAX_DETAIL)?;
        }
        if p.round.0 != 0.0 {
            check_length("/round", p.round, 1e-5, 1.0)?;
        }
        Self::field(p).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SilhouettesParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let (field, voxel) = Self::field(p)?;
        ctx.check_cancel()?;
        let r = mesh(&field, &MeshSpec { voxel }).map_err(|e| OpError::new(ErrorCode::Topology, e.to_string()).at("/detail"))?;
        place(tx, p.target.as_deref(), p.name.as_deref(), p.parent.as_deref(), &p.position, "silhouettes", r.mesh)
    }
}
