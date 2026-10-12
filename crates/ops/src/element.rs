//! Element transforms: move, rotate and scale selected vertices (with
//! proportional editing), shrink and fatten along normals, edge slide.

use glam::{EulerRot, Mat3, Quat, Vec3};
use p3d_geo::edit::{Falloff, KernelError, Pivot, Proportional, TransformSpec, edge_slide, shrink_fatten, transform_verts};
use p3d_mesh::{ElementKind, Mesh};
use p3d_query::{Filter, Selector, select as evaluate};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length, vec3};
use crate::tx::Tx;

const EDIT: OpFlags = OpFlags::UNDOABLE.union(OpFlags::PREVIEWABLE);

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

fn kernel(e: KernelError) -> OpError {
    match e {
        KernelError::OutOfRange { param, message, hint } => OpError::out_of_range(&format!("/{param}"), message).with_hint(hint),
        KernelError::Empty(m) => OpError::new(ErrorCode::EmptySelection, m).at("/select"),
        KernelError::Topology(t) => OpError::new(ErrorCode::Topology, t.to_string()),
    }
}

/// Vertices picked by a selector (edges and faces select their vertices).
///
/// Complexity: O(n) plus the selected elements' vertices.
fn selected_verts(mesh: &Mesh, select: &Selector) -> Result<Vec<u32>, OpError> {
    let sel = evaluate(mesh, select).map_err(|e| OpError::from_query(e, "/select"))?;
    let mut out: Vec<u32> = match sel.kind {
        ElementKind::Vert => sel.iter().collect(),
        ElementKind::Edge => sel.iter().flat_map(|e| mesh.edge_verts(e)).collect(),
        ElementKind::Face => sel.iter().flat_map(|f| mesh.face_verts(f).collect::<Vec<_>>()).collect(),
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(OpError::new(ErrorCode::EmptySelection, "the selector matched nothing")
            .at("/select")
            .with_hint("query shows what a selector picks"));
    }
    Ok(out)
}

// ---- edit.transform ---------------------------------------------------------------------

/// What rotations and scales of `edit.transform` turn around.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PivotParam {
    /// The mean of the selected vertices.
    #[default]
    Median,
    /// The center of their bounding box.
    BoundsCenter,
    /// Each connected piece of the selection around its own median.
    Individual,
    /// `pivot_point` (the 3D cursor or the active element).
    Point,
}

/// Proportional editing falloff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FalloffParam {
    #[default]
    Smooth,
    Sphere,
    Root,
    InverseSquare,
    Sharp,
    Linear,
    Constant,
    Random,
}

/// Proportional editing settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProportionalParams {
    /// Unselected vertices closer than this to the selection follow, less the farther they are.
    pub radius: Length,
    /// How the influence fades: `smooth` (default), `sphere`, `root`, `inverse_square`, `sharp`, `linear`, `constant`, `random`.
    #[serde(default)]
    pub falloff: FalloffParam,
    /// Measure the distance along edges (only connected geometry follows).
    #[serde(default)]
    pub connected_only: bool,
}

/// Parameters of `edit.transform`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransformElementsParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to transform (edges and faces select their vertices).
    pub select: Selector,
    /// Offset along the orientation axes.
    #[serde(default)]
    pub translate: [Length; 3],
    /// Rotation around the orientation axes (degrees, applied X, then Y, then Z).
    #[serde(default)]
    pub rotate: [Angle; 3],
    /// Scale along the orientation axes (1 changes nothing, 0 flattens).
    #[serde(default = "ones")]
    pub scale: [f32; 3],
    /// `median` (default), `bounds_center`, `individual` or `point`.
    #[serde(default)]
    pub pivot: PivotParam,
    /// Pivot for `pivot: "point"`, in object space.
    #[serde(default)]
    pub pivot_point: [Length; 3],
    /// Orientation axes in object space as three columns (X, Y, Z); the object's own axes when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axes: Option<[[f32; 3]; 3]>,
    /// Proportional editing: nearby unselected vertices follow with a falloff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proportional: Option<ProportionalParams>,
}

fn ones() -> [f32; 3] {
    [1.0; 3]
}

impl Default for TransformElementsParams {
    fn default() -> Self {
        TransformElementsParams {
            target: String::new(),
            select: Selector::Verts(Filter::default()),
            translate: [Length(0.0); 3],
            rotate: [Angle(0.0); 3],
            scale: ones(),
            pivot: PivotParam::Median,
            pivot_point: [Length(0.0); 3],
            axes: None,
            proportional: None,
        }
    }
}

/// `edit.transform`.
pub struct TransformElements;

impl Op for TransformElements {
    const ID: &'static str = "edit.transform";
    type Params = TransformElementsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Transform elements",
            category: Category::Edit,
            summary: "Moves, rotates and scales selected vertices, edges or faces of a mesh, optionally with proportional editing.",
            docs: "Works in the object's space: `translate`, `rotate` (degrees) and `scale` apply along `axes` (the object's axes by \
                   default; pass a face normal frame or the view for other orientations) around `pivot`. With `proportional`, \
                   unselected vertices within `radius` follow by a falloff of their distance to the selection, which reshapes \
                   surfaces smoothly.\n\n\
                   **Use for** tweaking shapes after modelling ops: lift a face, flare a rim (scale a loop), lean a part, pull a \
                   bump with proportional editing. To move whole objects use `scene.set_transform`.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}, "translate": [0, "10cm", 0]}"#,
                result: "crate (o:1): transformed 4 vertices",
            }],
            aliases: &["move vertices", "g", "grab", "rotate", "r", "scale", "s", "proportional editing", "soft selection", "polyMoveVertex"],
            complexity: "O(k log k) for k selected vertices; with proportional editing O(n log k), or O(m log m) along edges",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TransformElementsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !p.scale.iter().all(|s| s.is_finite() && s.abs() <= 1e6) {
            return Err(OpError::out_of_range("/scale", "scale factors must be finite"));
        }
        if let Some(a) = p.axes
            && !(a.iter().flatten().all(|x| x.is_finite()) && Mat3::from_cols_array_2d(&a).determinant().abs() > 1e-6)
        {
            return Err(
                OpError::invalid("/axes", "axes must be three finite, independent directions").with_hint("for example [[1,0,0],[0,1,0],[0,0,1]]")
            );
        }
        if let Some(pr) = &p.proportional {
            crate::common::check_length("/proportional/radius", pr.radius, 1e-6, p3d_geo::prim::MAX_SIZE)?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &TransformElementsParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let verts = selected_verts(tx.mesh(key, "/target")?, &p.select)?;
        // Orthonormalize the frame so translations and rotations stay true.
        let axes = p.axes.map_or(Mat3::IDENTITY, |a| {
            let m = Mat3::from_cols_array_2d(&a);
            let x = m.x_axis.normalize_or_zero();
            let z = x.cross(m.y_axis).normalize_or_zero();
            Mat3::from_cols(x, z.cross(x), z)
        });
        let r = p.rotate;
        let spec = TransformSpec {
            translate: vec3(&p.translate),
            rotate: Quat::from_euler(EulerRot::ZYX, r[2].radians() as f32, r[1].radians() as f32, r[0].radians() as f32),
            scale: Vec3::from(p.scale),
            pivot: match p.pivot {
                PivotParam::Median => Pivot::Median,
                PivotParam::BoundsCenter => Pivot::BoundsCenter,
                PivotParam::Individual => Pivot::Individual,
                PivotParam::Point => Pivot::Point(vec3(&p.pivot_point)),
            },
            axes,
            proportional: p.proportional.as_ref().map(|pr| Proportional {
                radius: pr.radius.0 as f32,
                falloff: match pr.falloff {
                    FalloffParam::Smooth => Falloff::Smooth,
                    FalloffParam::Sphere => Falloff::Sphere,
                    FalloffParam::Root => Falloff::Root,
                    FalloffParam::InverseSquare => Falloff::InverseSquare,
                    FalloffParam::Sharp => Falloff::Sharp,
                    FalloffParam::Linear => Falloff::Linear,
                    FalloffParam::Constant => Falloff::Constant,
                    FalloffParam::Random => Falloff::Random,
                },
                connected_only: pr.connected_only,
                seed: ctx.seed,
            }),
        };
        let res = transform_verts(tx.mesh_mut(key, "/target")?, &verts, &spec).map_err(kernel)?;
        let follow = if res.influenced > 0 { format!(" (+{} by proportional editing)", thousands(res.influenced)) } else { String::new() };
        let mut out = OpOutput::summary(format!("{}: transformed {} vertices{follow}", label(tx.doc(), key), thousands(res.selected)));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- edit.shrink_fatten --------------------------------------------------------------------

/// Parameters of `edit.shrink_fatten`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShrinkFattenParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to move (edges and faces select their vertices).
    pub select: Selector,
    /// Offset along the normals; negative shrinks.
    #[serde(default = "one_cm")]
    pub distance: Length,
    /// Stretch the offset at creases so faces move by the full distance.
    #[serde(default = "yes")]
    pub even: bool,
}

fn one_cm() -> Length {
    Length(0.01)
}

fn yes() -> bool {
    true
}

impl Default for ShrinkFattenParams {
    fn default() -> Self {
        ShrinkFattenParams { target: String::new(), select: Selector::Verts(Filter::default()), distance: one_cm(), even: true }
    }
}

/// `edit.shrink_fatten`.
pub struct ShrinkFatten;

impl Op for ShrinkFatten {
    const ID: &'static str = "edit.shrink_fatten";
    type Params = ShrinkFattenParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Shrink/Fatten",
            category: Category::Edit,
            summary: "Moves vertices along their normals: positive fattens (outward), negative shrinks.",
            docs: "Each selected vertex moves along its area-weighted normal by `distance`; `even` keeps faces moving by the full \
                   distance at creases. Unlike `edit.extrude` no faces are added.\n\n\
                   **Use for** inflating or thinning parts (a puffier cushion, a thinner blade), offsetting a region slightly.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {}}, "distance": "1cm"}"#,
                result: "crate (o:1): moved 8 vertices 10 mm along their normals",
            }],
            aliases: &["shrink fatten", "alt s", "push pull", "offset along normals", "inflate"],
            complexity: "O(k log k + sum of valences) for k vertices",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ShrinkFattenParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.distance.0.is_finite() && p.distance.0.abs() <= p3d_geo::prim::MAX_SIZE) {
            return Err(OpError::out_of_range("/distance", "distance must be finite"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &ShrinkFattenParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let verts = selected_verts(tx.mesh(key, "/target")?, &p.select)?;
        let n = shrink_fatten(tx.mesh_mut(key, "/target")?, &verts, p.distance.0 as f32, p.even).map_err(kernel)?;
        let mut out = OpOutput::summary(format!(
            "{}: moved {} vertices {} along their normals",
            label(tx.doc(), key),
            thousands(n),
            p3d_math::format_length(p.distance.0)
        ));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- edit.edge_slide ---------------------------------------------------------------------

/// Parameters of `edit.edge_slide`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeSlideParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Edges to slide (typically an edge loop).
    pub select: Selector,
    /// How far, from -1 to 1: 1 reaches the neighbouring vertices on one side, -1 the other side.
    #[serde(default = "half")]
    pub factor: f32,
}

fn half() -> f32 {
    0.5
}

impl Default for EdgeSlideParams {
    fn default() -> Self {
        EdgeSlideParams { target: String::new(), select: Selector::Edges(Filter::default()), factor: 0.5 }
    }
}

/// `edit.edge_slide`.
pub struct EdgeSlide;

impl Op for EdgeSlide {
    const ID: &'static str = "edit.edge_slide";
    type Params = EdgeSlideParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Edge slide",
            category: Category::Edit,
            summary: "Slides edges (an edge loop) along the surface toward the neighbouring edges on one side.",
            docs: "Each vertex of the selected edges moves along its sideways edge toward the next vertex: `factor` 1 reaches it on \
                   one side, -1 on the other. The surface keeps its shape because vertices only travel along existing edges.\n\n\
                   **Use for** placing a support loop close to a hard edge (after `edit.loop_cut`), adjusting edge flow.",
            examples: &[OpExample {
                args: r#"{"target": "duct", "select": {"edges": {"in_box": {"min": [-1, 0.39, -1], "max": [1, 0.41, 1]}}}, "factor": 0.25}"#,
                result: "duct (o:7): slid 8 vertices of 8 edges by 0.25",
            }],
            aliases: &["edge slide", "gg", "slide edge loop", "polySlideEdge"],
            complexity: "O(e log e + sum of valences) for e edges",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &EdgeSlideParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.factor.is_finite() && (-1.0..=1.0).contains(&p.factor)) {
            return Err(OpError::out_of_range("/factor", format!("factor {} is outside -1 to 1", p.factor)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &EdgeSlideParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let sel = evaluate(mesh, &p.select).map_err(|e| OpError::from_query(e, "/select"))?;
        if sel.kind != ElementKind::Edge {
            return Err(OpError::invalid("/select", format!("edit.edge_slide works on edges, but the selector picks {}", sel.kind.as_str())));
        }
        let edges: Vec<u32> = sel.iter().collect();
        let n = edge_slide(tx.mesh_mut(key, "/target")?, &edges, p.factor).map_err(kernel)?;
        let mut out = OpOutput::summary(format!(
            "{}: slid {} vertices of {} edges by {}",
            label(tx.doc(), key),
            thousands(n),
            thousands(edges.len()),
            p.factor
        ));
        out.changed.push(key);
        Ok(out)
    }
}
