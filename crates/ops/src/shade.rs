//! `shade.*`: hard and smooth edges, custom normals and subdivision creases.

use p3d_geo::shade::{Weighting, sharp_by_angle, weighted_normals};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};
use p3d_query::{Filter, Selector, select as evaluate};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{degrees, label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::Angle;
use crate::tx::Tx;

const SHADE: OpFlags = OpFlags::UNDOABLE;

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Edges picked by a selector: selected edges, the edges of selected faces,
/// or edges between two selected vertices. Every edge without a selector.
///
/// Complexity: O(n) for the selector plus the selected elements.
fn selected_edges(mesh: &Mesh, select: Option<&Selector>) -> Result<Vec<u32>, OpError> {
    let Some(s) = select else { return Ok(mesh.edges().collect()) };
    let sel = evaluate(mesh, s).map_err(|e| OpError::from_query(e, "/select"))?;
    let mut out: Vec<u32> = match sel.kind {
        ElementKind::Edge => sel.iter().collect(),
        ElementKind::Face => sel.iter().flat_map(|f| mesh.face_halfedges(f).map(|h| h >> 1).collect::<Vec<_>>()).collect(),
        ElementKind::Vert => mesh.edges().filter(|&e| mesh.edge_verts(e).iter().all(|&v| sel.bits.contains(v as usize))).collect(),
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(OpError::new(ErrorCode::EmptySelection, "the selector matched no edges")
            .at("/select")
            .with_hint("query shows what a selector picks"));
    }
    Ok(out)
}

/// Faces picked by an optional selector (all without one).
fn selected_faces(mesh: &Mesh, select: Option<&Selector>) -> Result<Vec<u32>, OpError> {
    let Some(s) = select else { return Ok(mesh.faces().collect()) };
    let sel = evaluate(mesh, s).map_err(|e| OpError::from_query(e, "/select"))?;
    if sel.kind != ElementKind::Face {
        return Err(OpError::invalid("/select", format!("this op works on faces, but the selector picks {}", sel.kind.as_str())));
    }
    if sel.is_empty() {
        return Err(OpError::new(ErrorCode::EmptySelection, "the selector matched no faces").at("/select"));
    }
    Ok(sel.iter().collect())
}

fn sharp_count(mesh: &Mesh) -> usize {
    mesh.edges().filter(|&e| mesh.edge_flags(e).contains(EdgeFlags::SHARP)).count()
}

/// Sets or clears the sharp flag on edges; returns how many changed.
fn set_sharp(mesh: &mut Mesh, edges: &[u32], on: bool) -> usize {
    let mut changed = 0;
    for &e in edges {
        if mesh.edge_flags(e).contains(EdgeFlags::SHARP) != on {
            mesh.set_edge_flag(e, EdgeFlags::SHARP, on);
            changed += 1;
        }
    }
    changed
}

// ---- shade.smooth_by_angle -------------------------------------------------------------

/// Parameters of `shade.smooth_by_angle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmoothByAngleParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Edges whose faces meet at more than this many degrees become sharp.
    #[serde(default = "thirty")]
    pub angle: Angle,
    /// Keep edges that are sharp already (marked by hand or by primitives).
    #[serde(default = "yes")]
    pub keep_sharp: bool,
}

fn thirty() -> Angle {
    Angle::from_degrees(30.0)
}

fn yes() -> bool {
    true
}

impl Default for SmoothByAngleParams {
    fn default() -> Self {
        SmoothByAngleParams { target: String::new(), angle: thirty(), keep_sharp: true }
    }
}

/// `shade.smooth_by_angle`.
pub struct SmoothByAngle;

impl Op for SmoothByAngle {
    const ID: &'static str = "shade.smooth_by_angle";
    type Params = SmoothByAngleParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Smooth by angle",
            category: Category::Shade,
            summary: "Shades smoothly across gentle edges and keeps edges sharper than an angle hard.",
            docs: "Edges whose faces meet at more than `angle` are marked sharp (normals split there), the others smooth. Sharp edges \
                   already marked stay unless `keep_sharp` is false. Custom normals are dropped so the result shows.\n\n\
                   **Use for** the standard look of game assets after modelling: 30 degrees suits most props; 45 to 60 for organic \
                   shapes with creases. For low-poly hard surface follow with `shade.weighted_normals`.",
            examples: &[OpExample { args: r#"{"target": "post", "angle": 30}"#, result: "post (o:3): 24 sharp edges at 30 degrees (+8, -0)" }],
            aliases: &["auto smooth", "shade auto smooth", "smooth by angle", "soften harden edges", "polySoftEdge"],
            complexity: "O(E + F + C)",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SmoothByAngleParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(0.0..=180.0).contains(&p.angle.degrees()) {
            return Err(OpError::out_of_range("/angle", format!("{} degrees is outside 0 to 180", p.angle.degrees())));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SmoothByAngleParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = sharp_by_angle(mesh, p.angle.degrees() as f32, p.keep_sharp);
        let mut out = OpOutput::summary(format!(
            "{}: {} sharp edges at {} degrees (+{}, -{})",
            label(tx.doc(), key),
            thousands(r.sharp),
            degrees(p.angle),
            thousands(r.marked),
            thousands(r.cleared)
        ));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- shade.mark_sharp / shade.clear_sharp -----------------------------------------------

/// Parameters of `shade.mark_sharp` and `shade.clear_sharp`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharpParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Edges (faces select their edges, vertices the edges between them); every edge when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
}

/// `shade.mark_sharp`.
pub struct MarkSharp;

impl Op for MarkSharp {
    const ID: &'static str = "shade.mark_sharp";
    type Params = SharpParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Mark sharp",
            category: Category::Shade,
            summary: "Marks edges sharp: shading breaks there (hard edges).",
            docs: "Sharp edges split vertex normals, so the faces on both sides shade separately (and export as split vertices). \
                   Custom normals, if set, are kept: rerun `shade.weighted_normals` afterwards.\n\n\
                   **Use for** panel lines, silhouettes of hard-surface parts, UV and bevel helpers ({\"edges\": {\"sharp\": true}} \
                   selects them later).",
            examples: &[OpExample {
                args: r#"{"target": "post", "select": {"edges": {"sharper_than_deg": 10}}}"#,
                result: "post (o:3): marked 8 edges sharp (24 sharp in all)",
            }],
            aliases: &["mark sharp", "harden edges", "hard edge", "polySoftEdge 0"],
            complexity: "O(n) for the selector plus O(E) to count",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SharpParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SharpParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let edges = selected_edges(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let n = set_sharp(mesh, &edges, true);
        let total = sharp_count(mesh);
        let mut out = OpOutput::summary(format!(
            "{}: marked {} edge{} sharp ({} sharp in all)",
            label(tx.doc(), key),
            thousands(n),
            plural(n),
            thousands(total)
        ));
        out.changed.push(key);
        Ok(out)
    }
}

/// `shade.clear_sharp`.
pub struct ClearSharp;

impl Op for ClearSharp {
    const ID: &'static str = "shade.clear_sharp";
    type Params = SharpParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Clear sharp",
            category: Category::Shade,
            summary: "Clears the sharp mark from edges so shading flows smoothly across them.",
            docs: "The opposite of `shade.mark_sharp`. Without a selection every edge becomes smooth (a fully smooth-shaded object).",
            examples: &[OpExample { args: r#"{"target": "post"}"#, result: "post (o:3): cleared 16 sharp edges (0 sharp in all)" }],
            aliases: &["clear sharp", "soften edges", "soft edge", "polySoftEdge 180"],
            complexity: "O(n) for the selector plus O(E) to count",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SharpParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SharpParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let edges = selected_edges(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let n = set_sharp(mesh, &edges, false);
        let total = sharp_count(mesh);
        let mut out = OpOutput::summary(format!(
            "{}: cleared {} sharp edge{} ({} sharp in all)",
            label(tx.doc(), key),
            thousands(n),
            plural(n),
            thousands(total)
        ));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- shade.set_flat / shade.set_smooth ---------------------------------------------------

/// Parameters of `shade.set_flat` and `shade.set_smooth`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FacesShadeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to shade; every face when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
}

/// `shade.set_flat`.
pub struct SetFlat;

impl Op for SetFlat {
    const ID: &'static str = "shade.set_flat";
    type Params = FacesShadeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Shade flat",
            category: Category::Shade,
            summary: "Shades faces flat (faceted) by marking all their edges sharp.",
            docs: "Every edge of the selected faces becomes sharp, so each face shows its own normal; custom normals are dropped. \
                   **Use for** faceted low-poly styles (gems, rocks, stylized props).",
            examples: &[OpExample { args: r#"{"target": "post"}"#, result: "post (o:3): 32 faces shade flat (64 edges sharp)" }],
            aliases: &["shade flat", "flat shading", "faceted", "harden normals"],
            complexity: "O(n) for the selector plus O(C) for the faces' corners",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FacesShadeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &FacesShadeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let faces = selected_faces(mesh, p.select.as_ref())?;
        let mut edges: Vec<u32> = faces.iter().flat_map(|&f| mesh.face_halfedges(f).map(|h| h >> 1).collect::<Vec<_>>()).collect();
        edges.sort_unstable();
        edges.dedup();
        let mesh = tx.mesh_mut(key, "/target")?;
        set_sharp(mesh, &edges, true);
        mesh.corner_normal = None;
        let mut out = OpOutput::summary(format!(
            "{}: {} faces shade flat ({} edges sharp)",
            label(tx.doc(), key),
            thousands(faces.len()),
            thousands(edges.len())
        ));
        out.changed.push(key);
        Ok(out)
    }
}

/// `shade.set_smooth`.
pub struct SetSmooth;

impl Op for SetSmooth {
    const ID: &'static str = "shade.set_smooth";
    type Params = FacesShadeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Shade smooth",
            category: Category::Shade,
            summary: "Shades faces smoothly by clearing the sharp mark on the edges between them.",
            docs: "Edges between two selected faces become smooth; edges on the selection's border keep their state. Custom normals \
                   are dropped. **Use for** organic shapes and round parts; for game props prefer `shade.smooth_by_angle`, which \
                   keeps real corners hard.",
            examples: &[OpExample { args: r#"{"target": "post"}"#, result: "post (o:3): 32 faces shade smooth (16 edges cleared)" }],
            aliases: &["shade smooth", "smooth shading", "soften normals"],
            complexity: "O(n) for the selector plus O(C) for the faces' corners",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FacesShadeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &FacesShadeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let faces = selected_faces(mesh, p.select.as_ref())?;
        let sel: std::collections::BTreeSet<u32> = faces.iter().copied().collect();
        let mut edges: Vec<u32> = faces
            .iter()
            .flat_map(|&f| mesh.face_halfedges(f).filter(|&h| mesh.face(h ^ 1).is_some_and(|g| sel.contains(&g))).map(|h| h >> 1).collect::<Vec<_>>())
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mesh = tx.mesh_mut(key, "/target")?;
        let n = set_sharp(mesh, &edges, false);
        mesh.corner_normal = None;
        let mut out =
            OpOutput::summary(format!("{}: {} faces shade smooth ({} edges cleared)", label(tx.doc(), key), thousands(faces.len()), thousands(n)));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- shade.weighted_normals ---------------------------------------------------------------

/// How `shade.weighted_normals` weighs faces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeightingParam {
    /// Large faces dominate: flat panels stay flat next to bevels.
    #[default]
    Area,
    /// By corner angle (like plain smooth shading).
    Angle,
    /// By area and angle.
    AreaAngle,
}

/// Parameters of `shade.weighted_normals`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WeightedNormalsParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// `area` (default), `angle` or `area_angle`.
    #[serde(default)]
    pub weighting: WeightingParam,
}

/// `shade.weighted_normals`.
pub struct WeightedNormals;

impl Op for WeightedNormals {
    const ID: &'static str = "shade.weighted_normals";
    type Params = WeightedNormalsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Weighted normals",
            category: Category::Shade,
            summary: "Sets custom normals weighted by face area so large flat faces stay flat-shaded next to small bevel faces.",
            docs: "Each vertex normal is the area-weighted average of its faces (within runs not split by sharp edges), stored as \
                   custom split normals and exported as they are. Large faces dominate, so a beveled box shades like a crisp box \
                   with soft highlights on the bevels: the standard look of low-poly hard-surface game assets.\n\n\
                   **Use after** the last geometry edit (`edit.bevel`, `prim.box` with `bevel`) and after setting sharp edges; edits \
                   after it carry the old normals along, so rerun it last.",
            examples: &[OpExample { args: r#"{"target": "crate"}"#, result: "crate (o:1): weighted normals on 24 corners (area)" }],
            aliases: &["weighted normal", "weighted normal modifier", "face weighted normals", "fwvn"],
            complexity: "O(V + F + C)",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &WeightedNormalsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &WeightedNormalsParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let (w, name) = match p.weighting {
            WeightingParam::Area => (Weighting::Area, "area"),
            WeightingParam::Angle => (Weighting::Angle, "angle"),
            WeightingParam::AreaAngle => (Weighting::AreaAngle, "area and angle"),
        };
        let mesh = tx.mesh_mut(key, "/target")?;
        let n = weighted_normals(mesh, w);
        let mut out = OpOutput::summary(format!("{}: weighted normals on {} corners ({name})", label(tx.doc(), key), thousands(n)));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- shade.crease ----------------------------------------------------------------------------

/// Parameters of `shade.crease`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreaseParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Edges to crease (faces select their edges, vertices the edges between them).
    pub select: Selector,
    /// Crease weight from 0 (smooth) to 1 (stays sharp under subdivision).
    #[serde(default = "full")]
    pub weight: f32,
}

fn full() -> f32 {
    1.0
}

impl Default for CreaseParams {
    fn default() -> Self {
        CreaseParams { target: String::new(), select: Selector::Edges(Filter::default()), weight: 1.0 }
    }
}

/// `shade.crease`.
pub struct Crease;

impl Op for Crease {
    const ID: &'static str = "shade.crease";
    type Params = CreaseParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Crease",
            category: Category::Shade,
            summary: "Sets how sharp edges stay under `subd.catmull_clark`: 0 rounds them, 1 keeps them sharp.",
            docs: "Crease weights only affect subdivision: a weight of 1 keeps the edge sharp at every level, 0.5 keeps it sharp for \
                   about five levels' worth of rounding less (semi-sharp creases), 0 rounds it fully.\n\n\
                   **Use for** holding hard edges on subdivided models without extra support loops.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}, "weight": 1}"#,
                result: "crate (o:1): crease 1 on 4 edges",
            }],
            aliases: &["crease", "edge crease", "subdivision crease", "polyCrease", "shift e"],
            complexity: "O(n) for the selector",
            flags: SHADE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CreaseParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.weight.is_finite() && (0.0..=1.0).contains(&p.weight)) {
            return Err(OpError::out_of_range("/weight", format!("crease weight {} is outside 0 to 1", p.weight)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &CreaseParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let edges = selected_edges(tx.mesh(key, "/target")?, Some(&p.select))?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let slots = mesh.edge_slots();
        let col = mesh.edge_crease.get_or_insert_with(|| vec![0.0; slots]);
        for &e in &edges {
            col[e as usize] = p.weight;
        }
        mesh.revision += 1;
        let mut out =
            OpOutput::summary(format!("{}: crease {} on {} edge{}", label(tx.doc(), key), p.weight, thousands(edges.len()), plural(edges.len())));
        out.changed.push(key);
        Ok(out)
    }
}
