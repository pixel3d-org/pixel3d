//! `prim.*`: primitives with clean quad topology and semantic groups.

use glam::DVec3;
use p3d_geo::prim::{
    self as kernel, BoxSpec, CapKind, CylinderSpec, MAX_BEVEL_SEGMENTS, MAX_SEGMENTS, MAX_SIZE, MIN_SIZE, PlaneSpec, PrimError, QuadSphereSpec,
};
use p3d_scene::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_count, check_length, check_new_object, create_mesh_object, dims, label, mesh_groups, thousands};
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Length, OriginParam};
use crate::tx::Tx;

const FLAGS: OpFlags = OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS);

fn kernel_error(e: PrimError) -> OpError {
    match e {
        PrimError::InvalidParam { param, value, reason } => OpError::out_of_range("", format!("{param} is {value}: {reason}")),
        PrimError::Build(b) => OpError::internal(format!("generated mesh is invalid: {b}")),
    }
}

fn created_summary(tx: &Tx, key: p3d_scene::ObjectKey, what: &str) -> OpOutput {
    let doc = tx.doc();
    let (dims_s, faces, tris) = doc.object_mesh(key).map_or_else(|| ("?".into(), 0, 0), |m| (dims(m), m.face_count(), m.triangle_count()));
    let budget = doc.settings.tri_budget() as usize;
    let mut out = OpOutput::summary(format!(
        "created {} {what} {dims_s}: {} faces ({} tris; budget {})",
        label(doc, key),
        thousands(faces),
        thousands(tris),
        thousands(budget)
    ));
    out.created.push(key);
    out.groups = mesh_groups(doc, key);
    out
}

fn check_placement(doc: &Document, name: Option<&str>, parent: Option<&str>) -> Result<(), OpError> {
    check_new_object(doc, name, parent).map(|_| ())
}

// ---- prim.box ------------------------------------------------------------------

/// Bevel of a box's edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BevelParams {
    /// Bevel radius; less than half the smallest box size.
    pub width: Length,
    /// Segments across the bevel: 1 is a flat chamfer (one triangle per
    /// corner), 2 or more rounds it (even counts give quads only).
    #[serde(default = "one")]
    pub segments: u32,
}

fn one() -> u32 {
    1
}

/// Advanced box settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct BoxAdvanced {
    /// Quads along X, Y and Z on each flat face.
    pub segments: [u32; 3],
}

impl Default for BoxAdvanced {
    fn default() -> Self {
        BoxAdvanced { segments: [1; 3] }
    }
}

/// Parameters of `prim.box`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct BoxParams {
    /// Object name; a free name such as `box_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Size along X (left to right).
    pub width: Length,
    /// Size along Y (bottom to top).
    pub height: Length,
    /// Size along Z (back to front).
    pub depth: Length,
    /// Rounded or chamfered edges; sharp when omitted.
    pub bevel: Option<BevelParams>,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    pub advanced: BoxAdvanced,
}

impl Default for BoxParams {
    fn default() -> Self {
        BoxParams {
            name: None,
            parent: None,
            width: Length(1.0),
            height: Length(1.0),
            depth: Length(1.0),
            bevel: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
            advanced: BoxAdvanced::default(),
        }
    }
}

/// `prim.box`.
pub struct BoxOp;

impl Op for BoxOp {
    const ID: &'static str = "prim.box";
    type Params = BoxParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Box",
            category: Category::Prim,
            summary: "Creates a box with optional rounded or chamfered edges and groups top, bottom, front, back, left, right.",
            docs: "Creates a mesh box at real size. The origin sits at the bottom center by default so the box stands on the ground.\n\n\
                   **Use for** crates, cabinets, walls, panels, blockouts. Add `bevel` for hard-surface props: 1 segment (chamfer) for small \
                   props, 2 to 3 for hero assets; widths of 3 to 10 mm read well on 0.5 to 1 m props.\n\n\
                   **Groups:** `top`, `bottom`, `front` (+Z), `back`, `left` (-X), `right` (+X), and `bevel` when beveled. \
                   Edges between groups are marked sharp.\n\n\
                   **Not for** round shapes (use `prim.cylinder` or `prim.quad_sphere`).",
            examples: &[
                OpExample {
                    args: r#"{"name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6, "bevel": {"width": "8mm", "segments": 2}}"#,
                    result: "created crate (o:1) box 0.6 x 0.6 x 0.6 m: 54 faces (108 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"width": "4m", "height": "3m", "depth": "20cm"}"#,
                    result: "created box (o:1) box 4 x 3 x 0.2 m: 6 faces (12 tris; budget 6,000)",
                },
            ],
            aliases: &["cube", "add cube", "rounded box", "chamfer box", "polyCube"],
            complexity: "O(faces): segments products plus bevel segments times edge segments",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &BoxParams) -> Result<(), OpError> {
        check_length("/width", p.width, MIN_SIZE, MAX_SIZE)?;
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        check_length("/depth", p.depth, MIN_SIZE, MAX_SIZE)?;
        for (i, &s) in p.advanced.segments.iter().enumerate() {
            check_count(&format!("/advanced/segments/{i}"), s, 1, MAX_SEGMENTS)?;
        }
        if let Some(b) = &p.bevel {
            let max = p.width.0.min(p.height.0).min(p.depth.0) / 2.0;
            if !(b.width.0.is_finite() && b.width.0 >= 0.0 && b.width.0 < max * (1.0 - 1e-6)) {
                return Err(OpError::out_of_range(
                    "/bevel/width",
                    format!("bevel width {} must be less than half the smallest size", p3d_math::format_length(b.width.0)),
                )
                .with_hint(format!("use less than {}", p3d_math::format_length(max))));
            }
            check_count("/bevel/segments", b.segments, 1, MAX_BEVEL_SEGMENTS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &BoxParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = BoxSpec {
            size: DVec3::new(p.width.0, p.height.0, p.depth.0),
            segments: p.advanced.segments,
            bevel_width: p.bevel.as_ref().map_or(0.0, |b| b.width.0),
            bevel_segments: p.bevel.as_ref().map_or(0, |b| b.segments),
        };
        let mesh = kernel::cuboid(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "box", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "box"))
    }
}

// ---- prim.cylinder ---------------------------------------------------------------

/// How cylinder caps are filled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapParam {
    /// Quads only (segments must be a multiple of 4); best for games and subdivision.
    #[default]
    QuadGrid,
    /// One polygon per cap.
    Ngon,
    /// Triangles around a center vertex.
    Fan,
    /// Open ends.
    None,
}

impl From<CapParam> for CapKind {
    fn from(c: CapParam) -> Self {
        match c {
            CapParam::QuadGrid => CapKind::QuadGrid,
            CapParam::Ngon => CapKind::Ngon,
            CapParam::Fan => CapKind::Fan,
            CapParam::None => CapKind::None,
        }
    }
}

/// Advanced cylinder settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CylinderAdvanced {
    /// Segments along the height.
    pub height_segments: u32,
}

impl Default for CylinderAdvanced {
    fn default() -> Self {
        CylinderAdvanced { height_segments: 1 }
    }
}

/// Parameters of `prim.cylinder`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CylinderParams {
    /// Object name; a free name such as `cylinder_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    pub radius: Length,
    /// Size along Y.
    pub height: Length,
    /// Segments around; when omitted, derived from the circumference and the
    /// asset class's target edge length (8 to 64, a multiple of 4 for quad-grid caps).
    pub segments: Option<u32>,
    pub cap: CapParam,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    pub advanced: CylinderAdvanced,
}

impl Default for CylinderParams {
    fn default() -> Self {
        CylinderParams {
            name: None,
            parent: None,
            radius: Length(0.25),
            height: Length(1.0),
            segments: None,
            cap: CapParam::QuadGrid,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
            advanced: CylinderAdvanced::default(),
        }
    }
}

fn cylinder_segments(doc: &Document, p: &CylinderParams) -> u32 {
    let multiple = if p.cap == CapParam::QuadGrid { 4 } else { 1 };
    p.segments.unwrap_or_else(|| doc.settings.round_segments(std::f64::consts::TAU * p.radius.0, multiple))
}

/// `prim.cylinder`.
pub struct Cylinder;

impl Op for Cylinder {
    const ID: &'static str = "prim.cylinder";
    type Params = CylinderParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Cylinder",
            category: Category::Prim,
            summary: "Creates a cylinder along Y with quad-grid, n-gon, fan or open caps and groups side, cap_top, cap_bottom.",
            docs: "Creates a cylinder standing on the ground by default. Ring vertex 0 faces +Z, so the mesh is mirror-symmetric left to right.\n\n\
                   **Caps:** `quad_grid` (default) is quads only and needs a multiple of 4 segments; `ngon` is one polygon (fine for flat \
                   low-poly caps); `fan` is triangles around a center; `none` leaves the ends open (pipes).\n\n\
                   **Segments** default to the circumference divided by the asset class's target edge length, clamped to 8 to 64.\n\n\
                   **Use for** posts, pipes, wheels, barrels (or `gen.lathe` for curved profiles, Phase 3).",
            examples: &[
                OpExample {
                    args: r#"{"name": "post", "radius": "5cm", "height": 2}"#,
                    result: "created post (o:1) cylinder 0.1 x 2 x 0.1 m: 32 faces (64 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"radius": 0.3, "height": 0.9, "segments": 24, "cap": "ngon"}"#,
                    result: "created cylinder (o:1) cylinder 0.6 x 0.9 x 0.6 m: 26 faces (92 tris; budget 6,000)",
                },
            ],
            aliases: &["add cylinder", "tube solid", "polyCylinder", "pillar"],
            complexity: "O(segments x height segments + segments^2 / 16) with quad-grid caps",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CylinderParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        check_count("/advanced/height_segments", p.advanced.height_segments, 1, MAX_SEGMENTS)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, MAX_SEGMENTS)?;
            if p.cap == CapParam::QuadGrid && !s.is_multiple_of(4) {
                let lower = (s / 4).max(1) * 4;
                return Err(OpError::invalid("/segments", format!("quad_grid caps need a multiple of 4 segments, got {s}"))
                    .with_suggestions(vec![lower.to_string(), (lower + 4).to_string()])
                    .with_hint("or use cap \"ngon\" or \"fan\""));
            }
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &CylinderParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = CylinderSpec {
            radius: p.radius.0,
            height: p.height.0,
            segments: cylinder_segments(tx.doc(), p),
            height_segments: p.advanced.height_segments,
            cap: p.cap.into(),
        };
        let mesh = kernel::cylinder(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "cylinder", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "cylinder"))
    }
}

// ---- prim.quad_sphere --------------------------------------------------------------

/// Parameters of `prim.quad_sphere`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct QuadSphereParams {
    /// Object name; a free name such as `sphere_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    pub radius: Length,
    /// Quads along each edge of the underlying cube (the equator has 4x as
    /// many); derived from the circumference when omitted.
    pub segments: Option<u32>,
    /// Origin at the bottom (rests on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
}

impl Default for QuadSphereParams {
    fn default() -> Self {
        QuadSphereParams { name: None, parent: None, radius: Length(0.25), segments: None, origin: OriginParam::Bottom, position: [Length(0.0); 3] }
    }
}

/// `prim.quad_sphere`.
pub struct QuadSphere;

impl Op for QuadSphere {
    const ID: &'static str = "prim.quad_sphere";
    type Params = QuadSphereParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Quad sphere",
            category: Category::Prim,
            summary: "Creates a sphere of even quads with no poles (a projected subdivided cube).",
            docs: "The default sphere: even quads, eight valence-3 vertices, no pinched poles, so it subdivides and deforms cleanly.\n\n\
                   **Segments** count quads along each cube edge; the equator gets four times as many edges. When omitted they follow \
                   the asset class's target edge length.\n\n**Use for** balls, heads (blockout), domes, knobs.",
            examples: &[OpExample {
                args: r#"{"name": "ball", "radius": "11cm", "segments": 6}"#,
                result: "created ball (o:1) sphere 0.22 x 0.22 x 0.22 m: 216 faces (432 tris; budget 6,000)",
            }],
            aliases: &["uv sphere", "add sphere", "cube sphere", "spherified cube", "polySphere", "ball"],
            complexity: "O(segments^2)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &QuadSphereParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 1, MAX_SEGMENTS / 4)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &QuadSphereParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * p.radius.0, 4) / 4);
        let mesh = kernel::quad_sphere(&QuadSphereSpec { radius: p.radius.0, segments }, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "sphere", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "sphere"))
    }
}

// ---- prim.plane ----------------------------------------------------------------------

/// Advanced plane settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct PlaneAdvanced {
    /// Quads along X and Z.
    pub segments: [u32; 2],
}

impl Default for PlaneAdvanced {
    fn default() -> Self {
        PlaneAdvanced { segments: [1; 2] }
    }
}

/// Parameters of `prim.plane`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct PlaneParams {
    /// Object name; a free name such as `plane_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Size along X.
    pub width: Length,
    /// Size along Z.
    pub depth: Length,
    /// Center position in the parent's space.
    pub position: [Length; 3],
    pub advanced: PlaneAdvanced,
}

impl Default for PlaneParams {
    fn default() -> Self {
        PlaneParams {
            name: None,
            parent: None,
            width: Length(1.0),
            depth: Length(1.0),
            position: [Length(0.0); 3],
            advanced: PlaneAdvanced::default(),
        }
    }
}

/// `prim.plane`.
pub struct Plane;

impl Op for Plane {
    const ID: &'static str = "prim.plane";
    type Params = PlaneParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Plane",
            category: Category::Prim,
            summary: "Creates a flat grid of quads in the XZ plane facing up (+Y).",
            docs: "A grid lying on the ground, facing up. **Use for** floors, cards, decals, terrain bases (with `advanced.segments`). \
                   **Not for** walls with thickness (use `prim.box`).",
            examples: &[OpExample {
                args: r#"{"name": "floor", "width": 4, "depth": 4, "advanced": {"segments": [4, 4]}}"#,
                result: "created floor (o:1) plane 4 x 0 x 4 m: 16 faces (32 tris; budget 6,000)",
            }],
            aliases: &["grid", "add plane", "quad", "polyPlane"],
            complexity: "O(segments_x x segments_z)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &PlaneParams) -> Result<(), OpError> {
        check_length("/width", p.width, MIN_SIZE, MAX_SIZE)?;
        check_length("/depth", p.depth, MIN_SIZE, MAX_SIZE)?;
        for (i, &s) in p.advanced.segments.iter().enumerate() {
            check_count(&format!("/advanced/segments/{i}"), s, 1, MAX_SEGMENTS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &PlaneParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = PlaneSpec { size_x: p.width.0, size_z: p.depth.0, segments_x: p.advanced.segments[0], segments_z: p.advanced.segments[1] };
        let mesh = kernel::plane(&spec).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "plane", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "plane"))
    }
}
