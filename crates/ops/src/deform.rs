//! `deform.*`: bend, twist, taper and noise displacement.

use p3d_geo::deform::{DeformAxis, DisplaceSpec, MAX_OCTAVES, bend, displace, taper, twist};
use p3d_geo::edit::KernelError;
use p3d_mesh::{ElementKind, Mesh};
use p3d_query::{Selector, select as evaluate};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{degrees, label, thousands};
use crate::edit::AxisParam;
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length};
use crate::tx::Tx;

const DEFORM: OpFlags = OpFlags::UNDOABLE;

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

fn axis(a: AxisParam) -> DeformAxis {
    match a {
        AxisParam::X => DeformAxis::X,
        AxisParam::Y => DeformAxis::Y,
        AxisParam::Z => DeformAxis::Z,
    }
}

fn axis_name(a: AxisParam) -> &'static str {
    match a {
        AxisParam::X => "X",
        AxisParam::Y => "Y",
        AxisParam::Z => "Z",
    }
}

/// Vertices picked by an optional selector (edges and faces select their
/// vertices); every vertex without one.
///
/// Complexity: O(n) plus the selected elements' vertices.
fn verts(mesh: &Mesh, select: Option<&Selector>) -> Result<Vec<u32>, OpError> {
    let Some(s) = select else { return Ok(mesh.verts().collect()) };
    let sel = evaluate(mesh, s).map_err(|e| OpError::from_query(e, "/select"))?;
    let mut out: Vec<u32> = match sel.kind {
        ElementKind::Vert => sel.iter().collect(),
        ElementKind::Edge => sel.iter().flat_map(|e| mesh.edge_verts(e)).collect(),
        ElementKind::Face => sel.iter().flat_map(|f| mesh.face_verts(f).collect::<Vec<_>>()).collect(),
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(OpError::new(ErrorCode::EmptySelection, "the selector matched nothing").at("/select"));
    }
    Ok(out)
}

fn kernel(e: KernelError) -> OpError {
    match e {
        KernelError::OutOfRange { param, message, hint } => OpError::out_of_range(&format!("/{param}"), message).with_hint(hint),
        KernelError::Empty(m) => OpError::new(ErrorCode::EmptySelection, m).at("/select"),
        KernelError::Topology(t) => OpError::new(ErrorCode::Topology, t.to_string()),
    }
}

fn finish(tx: &Tx, key: ObjectKey, text: String) -> OpOutput {
    let mut out = OpOutput::summary(format!("{}: {text}", label(tx.doc(), key)));
    out.changed.push(key);
    out
}

// ---- deform.bend ---------------------------------------------------------------------------

/// Parameters of `deform.bend`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BendParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to bend (edges and faces select theirs); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// The axis the shape runs along (it curls from its low end).
    #[serde(default = "y_axis")]
    pub axis: AxisParam,
    /// The axis it bends toward (positive side; a negative angle bends the other way).
    #[serde(default)]
    pub toward: AxisParam,
    /// Total turn over the whole length (degrees).
    #[serde(default = "ninety")]
    pub angle: Angle,
}

fn y_axis() -> AxisParam {
    AxisParam::Y
}

fn ninety() -> Angle {
    Angle::from_degrees(90.0)
}

impl Default for BendParams {
    fn default() -> Self {
        BendParams { target: String::new(), select: None, axis: AxisParam::Y, toward: AxisParam::X, angle: ninety() }
    }
}

/// `deform.bend`.
pub struct Bend;

impl Op for Bend {
    const ID: &'static str = "deform.bend";
    type Params = BendParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Bend",
            category: Category::Deform,
            summary: "Curls a shape along an axis onto a circular arc, turning by an angle over its length.",
            docs: "The stretch along `axis` (from its lowest affected vertex) is bent toward `toward` so that it turns by `angle` \
                   over its whole length; 180 makes a U, 360 a ring. Needs enough segments along the axis to curve smoothly \
                   (`edit.loop_cut` or primitives' height segments).\n\n\
                   **Use for** pipes and hoses, curved handles, horns (with `deform.taper`), banners, bent planks.",
            examples: &[OpExample {
                args: r#"{"target": "post", "angle": 45}"#,
                result: "post (o:3): bent 34 vertices along Y toward +X by 45 degrees",
            }],
            aliases: &["bend", "simple deform bend", "nonlinear bend", "curve deform"],
            complexity: "O(k log k) for k vertices",
            flags: DEFORM,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &BendParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if p.axis == p.toward {
            return Err(OpError::invalid("/toward", "the bend direction must differ from the axis").with_hint("for a Y axis bend toward x or z"));
        }
        if p.angle.degrees().abs() > 3600.0 {
            return Err(OpError::out_of_range("/angle", "bend angles are limited to 10 turns"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &BendParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let vs = verts(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let n = bend(tx.mesh_mut(key, "/target")?, &vs, axis(p.axis), axis(p.toward), p.angle.radians() as f32).map_err(kernel)?;
        let sign = if p.angle.0 < 0.0 { "-" } else { "+" };
        Ok(finish(
            tx,
            key,
            format!(
                "bent {} vertices along {} toward {sign}{} by {} degrees",
                thousands(n),
                axis_name(p.axis),
                axis_name(p.toward),
                degrees(Angle(p.angle.0.abs()))
            ),
        ))
    }
}

// ---- deform.twist ------------------------------------------------------------------------

/// Parameters of `deform.twist`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TwistParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to twist (edges and faces select theirs); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Axis to twist around (through the object origin).
    #[serde(default = "y_axis")]
    pub axis: AxisParam,
    /// Turn at the far end (degrees; the low end stays).
    #[serde(default = "ninety")]
    pub angle: Angle,
}

impl Default for TwistParams {
    fn default() -> Self {
        TwistParams { target: String::new(), select: None, axis: AxisParam::Y, angle: ninety() }
    }
}

/// `deform.twist`.
pub struct Twist;

impl Op for Twist {
    const ID: &'static str = "deform.twist";
    type Params = TwistParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Twist",
            category: Category::Deform,
            summary: "Twists a shape around an axis, from no turn at its low end to `angle` at its high end.",
            docs: "Vertices rotate around the axis line through the object origin, by an amount growing linearly along the axis. \
                   Needs segments along the axis to look smooth.\n\n**Use for** twisted columns, drill bits, ropes, horns, stylized \
                   tree trunks.",
            examples: &[OpExample { args: r#"{"target": "post", "angle": 180}"#, result: "post (o:3): twisted 34 vertices around Y by 180 degrees" }],
            aliases: &["twist", "simple deform twist", "nonlinear twist"],
            complexity: "O(k log k) for k vertices",
            flags: DEFORM,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TwistParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &TwistParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let vs = verts(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let n = twist(tx.mesh_mut(key, "/target")?, &vs, axis(p.axis), p.angle.radians() as f32).map_err(kernel)?;
        Ok(finish(tx, key, format!("twisted {} vertices around {} by {} degrees", thousands(n), axis_name(p.axis), degrees(p.angle))))
    }
}

// ---- deform.taper ------------------------------------------------------------------------

/// Parameters of `deform.taper`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaperParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to taper (edges and faces select theirs); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Axis the taper runs along (scaling toward the axis line through the origin).
    #[serde(default = "y_axis")]
    pub axis: AxisParam,
    /// Scale at the far end (0 makes a point, 1 changes nothing, 2 doubles).
    #[serde(default = "half")]
    pub factor: f32,
}

fn half() -> f32 {
    0.5
}

impl Default for TaperParams {
    fn default() -> Self {
        TaperParams { target: String::new(), select: None, axis: AxisParam::Y, factor: 0.5 }
    }
}

/// `deform.taper`.
pub struct Taper;

impl Op for Taper {
    const ID: &'static str = "deform.taper";
    type Params = TaperParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Taper",
            category: Category::Deform,
            summary: "Scales a shape toward its axis progressively, from full size at its low end to `factor` at its high end.",
            docs: "Coordinates across the axis scale linearly along it (around the axis line through the object origin).\n\n\
                   **Use for** horns and tusks (taper then bend), tree trunks, table legs, bottle necks, blade tips.",
            examples: &[OpExample { args: r#"{"target": "post", "factor": 0.25}"#, result: "post (o:3): tapered 34 vertices along Y to 0.25" }],
            aliases: &["taper", "simple deform taper", "flare", "nonlinear flare"],
            complexity: "O(k log k) for k vertices",
            flags: DEFORM,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TaperParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.factor.is_finite() && (0.0..=100.0).contains(&p.factor)) {
            return Err(OpError::out_of_range("/factor", format!("taper factor {} is outside 0 to 100", p.factor)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &TaperParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let vs = verts(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let n = taper(tx.mesh_mut(key, "/target")?, &vs, axis(p.axis), p.factor).map_err(kernel)?;
        Ok(finish(tx, key, format!("tapered {} vertices along {} to {}", thousands(n), axis_name(p.axis), p.factor)))
    }
}

// ---- deform.displace ---------------------------------------------------------------------

/// Parameters of `deform.displace`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplaceParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to displace (edges and faces select theirs); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Largest offset along the normals.
    #[serde(default = "one_cm")]
    pub strength: Length,
    /// Size of the bumps.
    #[serde(default = "ten_cm")]
    pub scale: Length,
    /// Noise seed: the same seed gives the same shape. The call's seed when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Noise layers (1 to 8): more adds finer detail.
    #[serde(default = "three")]
    pub octaves: u32,
}

fn one_cm() -> Length {
    Length(0.01)
}

fn ten_cm() -> Length {
    Length(0.1)
}

fn three() -> u32 {
    3
}

impl Default for DisplaceParams {
    fn default() -> Self {
        DisplaceParams { target: String::new(), select: None, strength: one_cm(), scale: ten_cm(), seed: None, octaves: 3 }
    }
}

/// `deform.displace`.
pub struct Displace;

impl Op for Displace {
    const ID: &'static str = "deform.displace";
    type Params = DisplaceParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Displace",
            category: Category::Deform,
            summary: "Moves vertices along their normals by seeded fractal noise (bumps, rocks, terrain, organic variation).",
            docs: "Each vertex moves along its normal by up to `strength` times smooth 3D noise with features about `scale` in size; \
                   `octaves` adds finer layers. The same seed always gives the same result. Needs a reasonably dense mesh (subdivide \
                   or use `prim.ico_sphere` / segmented primitives first).\n\n\
                   **Use for** rocks (ico sphere, strength 10 to 20% of the size), terrain, dents and organic irregularity. Follow \
                   with `edit.smooth` if it looks noisy.",
            examples: &[OpExample {
                args: r#"{"target": "tiles", "strength": "5cm", "scale": "50cm", "seed": 7}"#,
                result: "tiles (o:6): displaced 25 vertices by up to 50 mm (scale 0.5 m, seed 7)",
            }],
            aliases: &["displace", "noise", "displacement modifier", "randomize vertices", "polyNoise"],
            complexity: "O(F + C) for normals plus O(k x octaves) for k vertices",
            flags: DEFORM,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &DisplaceParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.strength.0.is_finite() && p.strength.0.abs() <= p3d_geo::prim::MAX_SIZE) {
            return Err(OpError::out_of_range("/strength", "strength must be finite"));
        }
        crate::common::check_length("/scale", p.scale, 1e-6, p3d_geo::prim::MAX_SIZE)?;
        crate::common::check_count("/octaves", p.octaves, 1, MAX_OCTAVES)
    }

    fn apply(tx: &mut Tx, p: &DisplaceParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let vs = verts(tx.mesh(key, "/target")?, p.select.as_ref())?;
        let seed = p.seed.unwrap_or(ctx.seed);
        let spec = DisplaceSpec { strength: p.strength.0 as f32, scale: p.scale.0 as f32, seed, octaves: p.octaves };
        let n = displace(tx.mesh_mut(key, "/target")?, &vs, &spec).map_err(kernel)?;
        Ok(finish(
            tx,
            key,
            format!(
                "displaced {} vertices by up to {} (scale {}, seed {seed})",
                thousands(n),
                p3d_math::format_length(p.strength.0.abs()),
                p3d_math::format_length(p.scale.0)
            ),
        ))
    }
}
