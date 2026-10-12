//! `prim.*`: primitives with clean quad topology and semantic groups.

use glam::DVec3;
use p3d_geo::prim::{
    self as kernel, BoxSpec, CapKind, ConeSpec, CylinderSpec, MAX_BEVEL_SEGMENTS, MAX_SEGMENTS, MAX_SIZE, MIN_SIZE, PlaneSpec, PrimError,
    QuadSphereSpec,
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
    /// Rarely needed settings.
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
    /// Radius in the XZ plane.
    pub radius: Length,
    /// Size along Y.
    pub height: Length,
    /// Segments around; when omitted, derived from the circumference and the
    /// asset class's target edge length (8 to 64, a multiple of 4 for quad-grid caps).
    pub segments: Option<u32>,
    /// Cap fill: `quad_grid` (default, quads only), `ngon`, `fan` or `none`.
    pub cap: CapParam,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    /// Rarely needed settings.
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
                   **Use for** posts, pipes, wheels, barrels (or `gen.lathe` for curved profiles).",
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
            check_round_segments("/segments", s, p.cap)?;
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

// ---- prim.cone ------------------------------------------------------------------------

/// Advanced cone settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ConeAdvanced {
    /// Segments along the height.
    pub height_segments: u32,
}

impl Default for ConeAdvanced {
    fn default() -> Self {
        ConeAdvanced { height_segments: 1 }
    }
}

/// Parameters of `prim.cone`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct ConeParams {
    /// Object name; a free name such as `cone_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Radius at the bottom; 0 makes a point there.
    pub radius_bottom: Length,
    /// Radius at the top; 0 (default) makes a pointed cone, any other value a frustum.
    pub radius_top: Length,
    /// Size along Y.
    pub height: Length,
    /// Segments around; when omitted, derived from the larger circumference and
    /// the asset class's target edge length (8 to 64, a multiple of 4 for quad-grid caps).
    pub segments: Option<u32>,
    /// Cap fill of ends with a radius: `quad_grid` (default, quads only), `ngon`, `fan` or `none`.
    pub cap: CapParam,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    /// Rarely needed settings.
    pub advanced: ConeAdvanced,
}

impl Default for ConeParams {
    fn default() -> Self {
        ConeParams {
            name: None,
            parent: None,
            radius_bottom: Length(0.25),
            radius_top: Length(0.0),
            height: Length(0.5),
            segments: None,
            cap: CapParam::QuadGrid,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
            advanced: ConeAdvanced::default(),
        }
    }
}

/// `prim.cone`.
pub struct Cone;

/// Checks a radius that may be 0 (a point) or a real size.
fn check_radius(path: &str, r: Length) -> Result<(), OpError> {
    if r.0 == 0.0 { Ok(()) } else { check_length(path, r, MIN_SIZE, MAX_SIZE) }
}

/// Checks a segment count for a cap kind; quad-grid caps need a multiple of 4.
fn check_round_segments(path: &str, s: u32, cap: CapParam) -> Result<(), OpError> {
    check_count(path, s, 3, MAX_SEGMENTS)?;
    if cap == CapParam::QuadGrid && !s.is_multiple_of(4) {
        let lower = (s / 4).max(1) * 4;
        return Err(OpError::invalid(path, format!("quad_grid caps need a multiple of 4 segments, got {s}"))
            .with_suggestions(vec![lower.to_string(), (lower + 4).to_string()])
            .with_hint("or use cap \"ngon\" or \"fan\""));
    }
    Ok(())
}

impl Op for Cone {
    const ID: &'static str = "prim.cone";
    type Params = ConeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Cone",
            category: Category::Prim,
            summary: "Creates a pointed cone or a frustum (truncated cone) along Y with groups side, cap_top, cap_bottom.",
            docs: "Creates a cone standing on the ground by default. `radius_top: 0` (default) gives a point with one ring of triangles \
                   around it; any other top radius gives a frustum with two caps. A zero `radius_bottom` points the cone down.\n\n\
                   **Caps** work as on `prim.cylinder`: `quad_grid` (default, quads only, multiple of 4 segments), `ngon`, `fan`, `none`. \
                   Ends with radius 0 never get a cap. Rim edges are sharp.\n\n\
                   **Use for** spikes, horns (blockout), lamp shades, funnels, bucket and pot bodies (frustums), tree crowns in low poly.",
            examples: &[
                OpExample {
                    args: r#"{"name": "spike", "radius_bottom": "3cm", "height": "12cm", "segments": 8}"#,
                    result: "created spike (o:1) cone 60 mm x 0.12 m x 60 mm: 20 faces (32 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"name": "bucket", "radius_bottom": 0.12, "radius_top": 0.15, "height": 0.3, "cap": "ngon"}"#,
                    result: "created bucket (o:1) cone 0.3 x 0.3 x 0.3 m: 26 faces (92 tris; budget 6,000)",
                },
            ],
            aliases: &["add cone", "frustum", "truncated cone", "polyCone", "spike"],
            complexity: "O(segments x height segments + segments^2 / 16) with quad-grid caps",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ConeParams) -> Result<(), OpError> {
        check_radius("/radius_bottom", p.radius_bottom)?;
        check_radius("/radius_top", p.radius_top)?;
        if p.radius_bottom.0 == 0.0 && p.radius_top.0 == 0.0 {
            return Err(OpError::out_of_range("/radius_top", "both radii are 0; at most one end can be a point")
                .with_hint("give radius_bottom or radius_top a size"));
        }
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        check_count("/advanced/height_segments", p.advanced.height_segments, 1, MAX_SEGMENTS)?;
        if let Some(s) = p.segments {
            check_round_segments("/segments", s, p.cap)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &ConeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let multiple = if p.cap == CapParam::QuadGrid { 4 } else { 1 };
        let r = p.radius_bottom.0.max(p.radius_top.0);
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * r, multiple));
        let spec = ConeSpec {
            radius_bottom: p.radius_bottom.0,
            radius_top: p.radius_top.0,
            height: p.height.0,
            segments,
            height_segments: p.advanced.height_segments,
            cap: p.cap.into(),
        };
        let mesh = kernel::cone(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "cone", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "cone"))
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
    /// Sphere radius.
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

// ---- prim.uv_sphere ----------------------------------------------------------------

/// Parameters of `prim.uv_sphere`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct UvSphereParams {
    /// Object name; a free name such as `sphere_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Sphere radius.
    pub radius: Length,
    /// Segments around Y; derived from the circumference when omitted (8 to 64, even).
    pub segments: Option<u32>,
    /// Bands from pole to pole; half the segments when omitted.
    pub rings: Option<u32>,
    /// Origin at the bottom (rests on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
}

impl Default for UvSphereParams {
    fn default() -> Self {
        UvSphereParams {
            name: None,
            parent: None,
            radius: Length(0.25),
            segments: None,
            rings: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
        }
    }
}

/// `prim.uv_sphere`.
pub struct UvSphere;

impl Op for UvSphere {
    const ID: &'static str = "prim.uv_sphere";
    type Params = UvSphereParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "UV sphere",
            category: Category::Prim,
            summary: "Creates a latitude-longitude sphere with quads between parallels and triangle fans at both poles.",
            docs: "Meridians and parallels like a globe: quads everywhere except one fan of triangles at each pole. Parallels give clean \
                   horizontal loops to cut or scale (domes, onion shapes, eyes with a pupil at a pole).\n\n\
                   **Prefer** `prim.quad_sphere` for general round shapes (no poles, even quads, better for subdivision and UVs); use \
                   this one when you need the pole and loop structure.\n\n\
                   **Segments** default to the circumference over the asset class's target edge length; **rings** to half the segments.",
            examples: &[OpExample {
                args: r#"{"name": "eye", "radius": "12mm", "segments": 16, "rings": 8}"#,
                result: "created eye (o:1) sphere 24 x 24 x 24 mm: 128 faces (224 tris; budget 6,000)",
            }],
            aliases: &["globe sphere", "latlong sphere", "polySphere", "add uv sphere"],
            complexity: "O(segments x rings)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &UvSphereParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, MAX_SEGMENTS)?;
        }
        if let Some(n) = p.rings {
            check_count("/rings", n, 2, MAX_SEGMENTS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &UvSphereParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * p.radius.0, 2));
        let rings = p.rings.unwrap_or((segments / 2).max(2));
        let spec = kernel::UvSphereSpec { radius: p.radius.0, segments, rings };
        let mesh = kernel::uv_sphere(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "sphere", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "sphere"))
    }
}

// ---- prim.ico_sphere ----------------------------------------------------------------

/// Parameters of `prim.ico_sphere`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct IcoSphereParams {
    /// Object name; a free name such as `sphere_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Sphere radius.
    pub radius: Length,
    /// Subdivision level (0 to 8): 20 x 4^level triangles. Derived from the
    /// asset class's target edge length when omitted (1 to 5).
    pub subdivisions: Option<u32>,
    /// Origin at the bottom (rests on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
}

impl Default for IcoSphereParams {
    fn default() -> Self {
        IcoSphereParams {
            name: None,
            parent: None,
            radius: Length(0.25),
            subdivisions: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
        }
    }
}

/// `prim.ico_sphere`.
pub struct IcoSphere;

impl Op for IcoSphere {
    const ID: &'static str = "prim.ico_sphere";
    type Params = IcoSphereParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Ico sphere",
            category: Category::Prim,
            summary: "Creates a geodesic sphere of nearly equal triangles (a subdivided icosahedron).",
            docs: "Triangles of almost equal size and shape, with no poles: twelve valence-5 vertices and valence 6 elsewhere. A vertex \
                   sits at the top and the bottom.\n\n\
                   **Use for** rocks and organic blobs that get displaced (`deform.displace`), low-poly faceted gems and planets, and \
                   collision-friendly spheres. **Not for** shapes that will be subdivided or edge-looped (use `prim.quad_sphere`).\n\n\
                   **Subdivisions:** level 1 has 80 triangles, 2 has 320, 3 has 1,280. When omitted, the level follows the asset \
                   class's target edge length.",
            examples: &[OpExample {
                args: r#"{"name": "rock", "radius": 0.4, "subdivisions": 2}"#,
                result: "created rock (o:1) sphere 0.8 x 0.8 x 0.8 m: 320 faces (320 tris; budget 6,000)",
            }],
            aliases: &["icosphere", "geodesic sphere", "add ico sphere", "polyPlatonicSolid", "geosphere"],
            complexity: "O(4^subdivisions)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &IcoSphereParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        if let Some(l) = p.subdivisions {
            check_count("/subdivisions", l, 0, kernel::MAX_ICO_SUBDIVISIONS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &IcoSphereParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        // An icosahedron's edge is about 1.05 r; each level halves it.
        let subdivisions = p.subdivisions.unwrap_or_else(|| {
            let edge = f64::from(tx.doc().settings.target_edge_length());
            (1.05 * p.radius.0 / edge).log2().round().clamp(1.0, 5.0) as u32
        });
        let spec = kernel::IcoSphereSpec { radius: p.radius.0, subdivisions };
        let mesh = kernel::ico_sphere(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "sphere", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "sphere"))
    }
}

// ---- prim.torus ------------------------------------------------------------------------

/// Parameters of `prim.torus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TorusParams {
    /// Object name; a free name such as `torus_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Distance from the center to the middle of the ring.
    pub major_radius: Length,
    /// Radius of the ring's cross-section (half its thickness); less than the major radius.
    pub minor_radius: Length,
    /// Segments around Y; derived from the outer circumference when omitted (8 to 64).
    pub segments: Option<u32>,
    /// Segments around the cross-section; derived from its circumference when omitted (8 to 64, even).
    pub sides: Option<u32>,
    /// Origin at the bottom (rests on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
}

impl Default for TorusParams {
    fn default() -> Self {
        TorusParams {
            name: None,
            parent: None,
            major_radius: Length(0.3),
            minor_radius: Length(0.05),
            segments: None,
            sides: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
        }
    }
}

/// `prim.torus`.
pub struct Torus;

impl Op for Torus {
    const ID: &'static str = "prim.torus";
    type Params = TorusParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Torus",
            category: Category::Prim,
            summary: "Creates a torus (ring, donut) lying flat in the XZ plane, all quads.",
            docs: "A ring around the Y axis: `major_radius` to the middle of the ring, `minor_radius` for its thickness. All quads, every \
                   vertex valence 4, no groups or sharp edges. Rotate it with `scene.set_transform` to stand it up.\n\n\
                   **Use for** barrel and bucket hoops (lay one per hoop), rings, handles, tires (blockout), pipe flanges, rope rings.\n\n\
                   **Segments** and **sides** default to the circumferences over the asset class's target edge length.",
            examples: &[OpExample {
                args: r#"{"name": "hoop", "major_radius": 0.31, "minor_radius": "8mm", "segments": 32, "sides": 8}"#,
                result: "created hoop (o:1) torus 0.636 m x 16 mm x 0.636 m: 256 faces (512 tris; budget 6,000)",
            }],
            aliases: &["donut", "ring", "add torus", "polyTorus", "hoop"],
            complexity: "O(segments x sides)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TorusParams) -> Result<(), OpError> {
        check_length("/major_radius", p.major_radius, MIN_SIZE, MAX_SIZE)?;
        check_length("/minor_radius", p.minor_radius, MIN_SIZE, MAX_SIZE)?;
        if p.minor_radius.0 >= p.major_radius.0 * (1.0 - 1e-6) {
            return Err(OpError::out_of_range(
                "/minor_radius",
                format!(
                    "minor radius {} must be less than the major radius {}",
                    p3d_math::format_length(p.minor_radius.0),
                    p3d_math::format_length(p.major_radius.0)
                ),
            )
            .with_hint("a thicker ring needs a larger major_radius; for a disc with a hole use prim.tube"));
        }
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, MAX_SEGMENTS)?;
        }
        if let Some(s) = p.sides {
            check_count("/sides", s, 3, MAX_SEGMENTS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &TorusParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let settings = &tx.doc().settings;
        let tau = std::f64::consts::TAU;
        let segments = p.segments.unwrap_or_else(|| settings.round_segments(tau * (p.major_radius.0 + p.minor_radius.0), 1));
        let sides = p.sides.unwrap_or_else(|| settings.round_segments(tau * p.minor_radius.0, 2));
        let spec = kernel::TorusSpec { major_radius: p.major_radius.0, minor_radius: p.minor_radius.0, segments, sides };
        let mesh = kernel::torus(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "torus", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "torus"))
    }
}

// ---- prim.tube ---------------------------------------------------------------------------

/// Advanced tube settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TubeAdvanced {
    /// Segments along the height.
    pub height_segments: u32,
}

impl Default for TubeAdvanced {
    fn default() -> Self {
        TubeAdvanced { height_segments: 1 }
    }
}

/// Parameters of `prim.tube`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TubeParams {
    /// Object name; a free name such as `tube_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Outer radius.
    pub radius: Length,
    /// Radius of the hole; less than `radius` (wall thickness = radius - inner_radius).
    pub inner_radius: Length,
    /// Size along Y.
    pub height: Length,
    /// Segments around; derived from the outer circumference when omitted (8 to 64).
    pub segments: Option<u32>,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    /// Rarely needed settings.
    pub advanced: TubeAdvanced,
}

impl Default for TubeParams {
    fn default() -> Self {
        TubeParams {
            name: None,
            parent: None,
            radius: Length(0.25),
            inner_radius: Length(0.2),
            height: Length(1.0),
            segments: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
            advanced: TubeAdvanced::default(),
        }
    }
}

/// `prim.tube`.
pub struct Tube;

impl Op for Tube {
    const ID: &'static str = "prim.tube";
    type Params = TubeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Tube",
            category: Category::Prim,
            summary: "Creates a hollow cylinder (pipe with wall thickness) with groups outer, inner, cap_top, cap_bottom.",
            docs: "A cylinder with a coaxial hole: outer and inner walls joined by ring-shaped caps, all quads, the four rims sharp. The \
                   inner wall faces the axis.\n\n\
                   **Use for** pipes and pipe sections with visible thickness, collars, washers (short tubes), sleeves, cups and \
                   mugs (then `edit.fill` a bottom), round frames.\n\n\
                   **Not for** thin bent pipes along a path (use `gen.pipe`) or rings with a round cross-section (use `prim.torus`).",
            examples: &[OpExample {
                args: r#"{"name": "pipe", "radius": "5cm", "inner_radius": "4cm", "height": 1.2, "segments": 16}"#,
                result: "created pipe (o:1) tube 0.1 x 1.2 x 0.1 m: 64 faces (128 tris; budget 6,000)",
            }],
            aliases: &["pipe", "hollow cylinder", "add tube", "polyPipe", "ring cylinder", "washer"],
            complexity: "O(segments x height segments)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TubeParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        check_length("/inner_radius", p.inner_radius, MIN_SIZE, MAX_SIZE)?;
        if p.inner_radius.0 >= p.radius.0 * (1.0 - 1e-6) {
            return Err(OpError::out_of_range(
                "/inner_radius",
                format!(
                    "inner radius {} must be less than the radius {}",
                    p3d_math::format_length(p.inner_radius.0),
                    p3d_math::format_length(p.radius.0)
                ),
            )
            .with_hint("the wall thickness is radius - inner_radius"));
        }
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        check_count("/advanced/height_segments", p.advanced.height_segments, 1, MAX_SEGMENTS)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, MAX_SEGMENTS)?;
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &TubeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * p.radius.0, 1));
        let spec = kernel::TubeSpec {
            outer_radius: p.radius.0,
            inner_radius: p.inner_radius.0,
            height: p.height.0,
            segments,
            height_segments: p.advanced.height_segments,
        };
        let mesh = kernel::tube(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "tube", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "tube"))
    }
}

// ---- prim.capsule ------------------------------------------------------------------------

/// Advanced capsule settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CapsuleAdvanced {
    /// Segments along the straight part.
    pub height_segments: u32,
}

impl Default for CapsuleAdvanced {
    fn default() -> Self {
        CapsuleAdvanced { height_segments: 1 }
    }
}

/// Parameters of `prim.capsule`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CapsuleParams {
    /// Object name; a free name such as `capsule_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Radius of the body and the round ends.
    pub radius: Length,
    /// Total height including both round ends; more than twice the radius.
    pub height: Length,
    /// Segments around; a multiple of 8, derived from the circumference when omitted (8 to 64).
    pub segments: Option<u32>,
    /// Origin at the bottom (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
    /// Rarely needed settings.
    pub advanced: CapsuleAdvanced,
}

impl Default for CapsuleParams {
    fn default() -> Self {
        CapsuleParams {
            name: None,
            parent: None,
            radius: Length(0.15),
            height: Length(0.8),
            segments: None,
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
            advanced: CapsuleAdvanced::default(),
        }
    }
}

/// `prim.capsule`.
pub struct Capsule;

impl Op for Capsule {
    const ID: &'static str = "prim.capsule";
    type Params = CapsuleParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Capsule",
            category: Category::Prim,
            summary: "Creates a capsule (a cylinder with round ends) of quads only, with groups side, cap_top, cap_bottom.",
            docs: "A cylinder whose ends are half spheres. The ends are quads without pinched poles (like `prim.quad_sphere`), so the \
                   capsule subdivides and deforms cleanly. `height` is the total height including both ends.\n\n\
                   **Use for** limbs and fingers in blockouts, pills, rounded handles, character collision proxies, bollards.\n\n\
                   **Segments** must be a multiple of 8; when omitted they follow the asset class's target edge length.",
            examples: &[OpExample {
                args: r#"{"name": "arm", "radius": "4cm", "height": "30cm", "segments": 8}"#,
                result: "created arm (o:1) capsule 80 mm x 0.3 m x 80 mm: 32 faces (64 tris; budget 6,000)",
            }],
            aliases: &["pill", "add capsule", "polyCapsule", "rounded cylinder"],
            complexity: "O(segments x height segments + segments^2)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CapsuleParams) -> Result<(), OpError> {
        check_length("/radius", p.radius, MIN_SIZE, MAX_SIZE)?;
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        if p.height.0 - 2.0 * p.radius.0 < MIN_SIZE {
            return Err(OpError::out_of_range(
                "/height",
                format!(
                    "height {} must be more than twice the radius ({})",
                    p3d_math::format_length(p.height.0),
                    p3d_math::format_length(2.0 * p.radius.0)
                ),
            )
            .with_hint("height is the total height including both round ends; use prim.quad_sphere for a ball"));
        }
        check_count("/advanced/height_segments", p.advanced.height_segments, 1, MAX_SEGMENTS)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 8, MAX_SEGMENTS)?;
            if !s.is_multiple_of(8) {
                let lower = (s / 8).max(1) * 8;
                return Err(OpError::invalid("/segments", format!("capsules need a multiple of 8 segments, got {s}"))
                    .with_suggestions(vec![lower.to_string(), (lower + 8).to_string()]));
            }
        }
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &CapsuleParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * p.radius.0, 8));
        let spec = kernel::CapsuleSpec { radius: p.radius.0, height: p.height.0, segments, height_segments: p.advanced.height_segments };
        let mesh = kernel::capsule(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "capsule", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "capsule"))
    }
}

// ---- prim.wedge ------------------------------------------------------------------------

/// Parameters of `prim.wedge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct WedgeParams {
    /// Object name; a free name such as `wedge_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Size along X (left to right).
    pub width: Length,
    /// Height of the back (the slope's top edge).
    pub height: Length,
    /// Size along Z (back to front); the slope runs down toward +Z.
    pub depth: Length,
    /// Origin at the bottom center (stands on the ground) or the center.
    pub origin: OriginParam,
    /// Origin position in the parent's space.
    pub position: [Length; 3],
}

impl Default for WedgeParams {
    fn default() -> Self {
        WedgeParams {
            name: None,
            parent: None,
            width: Length(1.0),
            height: Length(0.5),
            depth: Length(1.0),
            origin: OriginParam::Bottom,
            position: [Length(0.0); 3],
        }
    }
}

/// `prim.wedge`.
pub struct Wedge;

impl Op for Wedge {
    const ID: &'static str = "prim.wedge";
    type Params = WedgeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Wedge",
            category: Category::Prim,
            summary: "Creates a wedge (ramp) whose slope runs down toward the front, with groups bottom, back, slope, left, right.",
            docs: "A triangular prism lying on a rectangle: the back is a full-height rectangle, the slope falls to the front bottom edge, \
                   and the left and right ends are triangles. Every edge is sharp.\n\n\
                   **Use for** ramps, roof halves, door stops, wedge-shaped trim and blockouts. Rotate with `scene.set_transform` \
                   (for example 180 degrees around Y to face the slope backward).",
            examples: &[OpExample {
                args: r#"{"name": "ramp", "width": 2, "height": 0.5, "depth": 3}"#,
                result: "created ramp (o:1) wedge 2 x 0.5 x 3 m: 5 faces (8 tris; budget 6,000)",
            }],
            aliases: &["ramp", "add wedge", "triangular prism", "prism", "slope"],
            complexity: "O(1)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &WedgeParams) -> Result<(), OpError> {
        check_length("/width", p.width, MIN_SIZE, MAX_SIZE)?;
        check_length("/height", p.height, MIN_SIZE, MAX_SIZE)?;
        check_length("/depth", p.depth, MIN_SIZE, MAX_SIZE)?;
        check_placement(doc, p.name.as_deref(), p.parent.as_deref())
    }

    fn apply(tx: &mut Tx, p: &WedgeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = kernel::WedgeSpec { size: DVec3::new(p.width.0, p.height.0, p.depth.0) };
        let mesh = kernel::wedge(&spec, p.origin.into()).map_err(kernel_error)?;
        let key = create_mesh_object(tx, p.name.as_deref(), "wedge", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "wedge"))
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
    /// Rarely needed settings.
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
