//! `gen.*`: generators that build meshes from profiles and paths.

use std::borrow::Cow;

use glam::{DVec2, Vec3};
use p3d_geo::curve::{self as curves, MAX_RESOLUTION};
use p3d_geo::edit::Axis;
use p3d_geo::generate::{LatheSpec, MAX_PROFILE_POINTS, lathe};
use p3d_geo::prim::{CapKind, MAX_SEGMENTS, PrimError};
use p3d_mesh::Mesh;
use p3d_scene::{Document, ObjectKey, Spline};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::common::{check_count, check_length, check_new_object, create_mesh_object};
use crate::curve::{curve_target, kernel_error as curve_error};
use crate::edit::AxisParam;
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length};
use crate::prim::{CapParam, created_summary};
use crate::tx::Tx;

const FLAGS: OpFlags = OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS);

/// Bezier segments per curved span in generators: smooth enough for game
/// props without wasting rows; raise it for hero assets.
pub const GEN_RESOLUTION: u32 = 6;

fn gen_error(e: PrimError) -> OpError {
    match e {
        PrimError::InvalidParam { param, value, reason } => OpError::out_of_range(&format!("/{param}"), format!("{value}: {reason}")),
        PrimError::Build(b) => OpError::internal(format!("generated mesh is invalid: {b}")),
    }
}

fn axis(a: AxisParam) -> Axis {
    match a {
        AxisParam::X => Axis::X,
        AxisParam::Y => Axis::Y,
        AxisParam::Z => Axis::Z,
    }
}

fn y_axis() -> AxisParam {
    AxisParam::Y
}

fn zero3() -> [Length; 3] {
    [Length(0.0); 3]
}

fn full_turn() -> Angle {
    Angle::from_degrees(360.0)
}

fn gen_resolution() -> u32 {
    GEN_RESOLUTION
}

/// Creates a generated mesh object. Built from a curve object (`source`)
/// with no explicit parent or position, it takes the curve's parent and
/// transform, so it lands where the curve is drawn.
fn create_gen(
    tx: &mut Tx,
    name: Option<&str>,
    default_name: &str,
    parent: Option<&str>,
    position: Option<&[Length; 3]>,
    source: Option<ObjectKey>,
    mesh: Mesh,
) -> Result<ObjectKey, OpError> {
    let place = match (source, parent, position) {
        (Some(src), None, None) => tx.doc().object(src).map(|o| (o.parent(), o.transform)),
        _ => None,
    };
    let Some((parent_key, transform)) = place else {
        return create_mesh_object(tx, name, default_name, parent, position.unwrap_or(&zero3()), mesh);
    };
    check_new_object(tx.doc(), name, None)?;
    let name = name.map_or_else(|| tx.doc().unique_name(default_name, parent_key), str::to_owned);
    let key = tx.doc_mut().create_mesh_object(&name, parent_key, mesh).map_err(|e| OpError::from_scene(e, "/name"))?;
    if let Some(o) = tx.doc_mut().object_mut(key) {
        o.transform = transform;
    }
    Ok(key)
}

// ---- profiles --------------------------------------------------------------------------

/// Built-in lathe profiles, normalized to a largest radius and a height of 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePreset {
    /// Bulging staves, flat ends (0.9 m tall, 0.31 m radius).
    Barrel,
    /// Round belly, narrow neck, flared lip (0.4 m, 0.12 m).
    Vase,
    /// Straight body, shoulders, neck and lip (0.3 m, 0.04 m).
    Bottle,
    /// Hollow bowl with a foot ring (0.08 m, 0.08 m).
    Bowl,
    /// Shaft with a stepped base and capital (3 m, 0.25 m).
    Column,
}

impl ProfilePreset {
    const NAMES: [&'static str; 5] = ["barrel", "vase", "bottle", "bowl", "column"];

    /// Normalized points, whether they are smooth, and the default height and radius.
    fn shape(self) -> (&'static [[f64; 2]], bool, f64, f64) {
        match self {
            ProfilePreset::Barrel => (&[[0.82, 0.0], [1.0, 0.5], [0.82, 1.0]], true, 0.9, 0.31),
            ProfilePreset::Vase => (&[[0.5, 0.0], [0.85, 0.12], [1.0, 0.35], [0.6, 0.72], [0.42, 0.88], [0.55, 1.0]], true, 0.4, 0.12),
            ProfilePreset::Bottle => (&[[1.0, 0.0], [1.0, 0.6], [0.9, 0.7], [0.38, 0.82], [0.34, 0.92], [0.38, 0.96], [0.34, 1.0]], true, 0.3, 0.04),
            ProfilePreset::Bowl => (&[[0.45, 0.0], [0.75, 0.2], [1.0, 1.0], [0.92, 1.0], [0.68, 0.28], [0.0, 0.12]], true, 0.08, 0.08),
            ProfilePreset::Column => (&[[1.0, 0.0], [1.0, 0.04], [0.8, 0.06], [0.8, 0.94], [1.0, 0.96], [1.0, 1.0]], false, 3.0, 0.25),
        }
    }
}

/// A profile: inline points, a preset name, or a curve object.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ProfileParam {
    Preset(ProfilePreset),
    Points(Vec<[Length; 2]>),
    Curve { curve: String },
}

impl Default for ProfileParam {
    fn default() -> Self {
        ProfileParam::Preset(ProfilePreset::Barrel)
    }
}

impl<'de> Deserialize<'de> for ProfileParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v {
            Value::String(s) => serde_json::from_value(Value::String(s.clone()))
                .map(ProfileParam::Preset)
                .map_err(|_| D::Error::custom(format!("unknown profile preset `{s}`; presets are {}", ProfilePreset::NAMES.join(", ")))),
            Value::Array(_) => serde_json::from_value(v)
                .map(ProfileParam::Points)
                .map_err(|e| D::Error::custom(format!("profile points are [radius, height] pairs: {e}"))),
            Value::Object(m) => match (m.len(), m.get("curve")) {
                (1, Some(Value::String(c))) => Ok(ProfileParam::Curve { curve: c.clone() }),
                _ => Err(D::Error::custom("a profile object is {\"curve\": \"<curve object>\"}")),
            },
            _ => Err(D::Error::custom("a profile is a preset name, a list of [radius, height] points or {\"curve\": \"<name>\"}")),
        }
    }
}

impl JsonSchema for ProfileParam {
    fn schema_name() -> Cow<'static, str> {
        "Profile".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let preset = g.subschema_for::<ProfilePreset>();
        let length = g.subschema_for::<Length>();
        json_schema!({
            "description": "A profile: a preset name, points [radius, height] from bottom to top (either order works), or {\"curve\": \"<curve object>\"} whose X is the radius and Y the height.",
            "anyOf": [
                preset,
                { "type": "array", "minItems": 2, "items": { "type": "array", "items": length, "minItems": 2, "maxItems": 2 } },
                { "type": "object", "properties": { "curve": { "type": "string" } }, "required": ["curve"], "additionalProperties": false }
            ]
        })
    }
}

/// A resolved 2D profile: points, the span of each segment, closed.
struct Profile {
    points: Vec<DVec2>,
    spans: Vec<u32>,
    closed: bool,
}

/// Samples points as a smooth Bezier (automatic handles) or keeps them as a
/// polyline; returns the points and each segment's span.
///
/// Complexity: O(points x resolution).
fn sample(points: &[Vec3], handles: Option<Vec<[Vec3; 2]>>, closed: bool, resolution: u32) -> Result<(Vec<DVec2>, Vec<u32>), OpError> {
    let (pts, spans) = match handles {
        Some(h) => curves::sample_bezier_spans(points, &h, closed, resolution).map_err(curve_error)?,
        None => (points.to_vec(), Vec::new()),
    };
    Ok((pts.iter().map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))).collect(), spans))
}

/// Resolves a lathe profile to `(radius, height)` points.
///
/// Complexity: O(points x resolution).
fn lathe_profile(doc: &Document, p: &LatheParams) -> Result<Profile, OpError> {
    let smooth_handles = |pts: &[Vec3], closed: bool| if p.smooth { Some(curves::auto_handles(pts, closed)) } else { None };
    match &p.profile {
        ProfileParam::Preset(preset) => {
            let (shape, smooth, h, r) = preset.shape();
            let (h, r) = (p.height.map_or(h, |x| x.0), p.radius.map_or(r, |x| x.0));
            let pts: Vec<Vec3> = shape.iter().map(|[x, y]| Vec3::new((x * r) as f32, (y * h) as f32, 0.0)).collect();
            let handles = if smooth { Some(curves::auto_handles(&pts, false)) } else { None };
            let (points, spans) = sample(&pts, handles, false, p.resolution)?;
            Ok(Profile { points, spans, closed: false })
        }
        ProfileParam::Points(raw) => {
            let pts: Vec<Vec3> = raw.iter().map(|[x, y]| Vec3::new(x.0 as f32, y.0 as f32, 0.0)).collect();
            let (points, spans) = sample(&pts, smooth_handles(&pts, p.closed), p.closed, p.resolution)?;
            Ok(Profile { points, spans, closed: p.closed })
        }
        ProfileParam::Curve { curve } => {
            let key = curve_target(doc, curve, "/profile/curve")?;
            let c = doc.object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?;
            let [s]: &[Spline; 1] = c
                .splines
                .as_slice()
                .try_into()
                .map_err(|_| OpError::invalid("/profile/curve", format!("the curve has {} splines; a lathe profile is one", c.splines.len())))?;
            let pts = s.positions();
            let extent = pts.iter().map(|q| q.length()).fold(0.0f32, f32::max).max(1e-3);
            if let Some(i) = pts.iter().position(|q| q.z.abs() > extent * 1e-4) {
                return Err(OpError::invalid("/profile/curve", format!("point {i} has z {}; a lathe profile lies in the XY plane", pts[i].z))
                    .with_hint("draw the profile with x as the radius and y as the height, z = 0"));
            }
            let handles = s.handles.as_ref().map(|h| h.iter().map(|[a, b]| [Vec3::from(*a), Vec3::from(*b)]).collect());
            let handles = handles.or_else(|| smooth_handles(&pts, s.closed));
            let (points, spans) = sample(&pts, handles, s.closed, p.resolution)?;
            Ok(Profile { points, spans, closed: s.closed })
        }
    }
}

// ---- gen.lathe -------------------------------------------------------------------------

/// Parameters of `gen.lathe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LatheParams {
    /// Object name; a free name such as `lathe_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The profile to revolve.
    pub profile: ProfileParam,
    /// A smooth Bezier through the points instead of straight segments (points and polyline curves).
    #[serde(default)]
    pub smooth: bool,
    /// Joins the last point back to the first (rings such as tires and washers; points only).
    #[serde(default)]
    pub closed: bool,
    /// Height of a preset profile (presets only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Length>,
    /// Largest radius of a preset profile (presets only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<Length>,
    /// Segments around the axis; adapts to the size and asset class when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<u32>,
    /// Revolution (degrees); partial turns start at the front and stay open.
    #[serde(default = "full_turn")]
    pub angle: Angle,
    /// How the open ends of a full turn are filled.
    #[serde(default)]
    pub cap: CapParam,
    /// Axis to revolve around; the profile's height runs along it.
    #[serde(default = "y_axis")]
    pub axis: AxisParam,
    /// Segments per curved span of a smooth profile.
    #[serde(default = "gen_resolution")]
    pub resolution: u32,
    /// Origin position in the parent's space. Built from a curve object, the result takes the curve's place
    /// (parent and transform) unless `parent` or `position` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
}

impl Default for LatheParams {
    fn default() -> Self {
        LatheParams {
            name: None,
            parent: None,
            profile: ProfileParam::default(),
            smooth: false,
            closed: false,
            height: None,
            radius: None,
            segments: None,
            angle: full_turn(),
            cap: CapParam::QuadGrid,
            axis: AxisParam::Y,
            resolution: GEN_RESOLUTION,
            position: None,
        }
    }
}

/// `gen.lathe`.
pub struct LatheOp;

impl Op for LatheOp {
    const ID: &'static str = "gen.lathe";
    type Params = LatheParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Lathe",
            category: Category::Gen,
            summary: "Revolves a profile around an axis into a round object with groups side, cap_top, cap_bottom and profile_<i>.",
            docs: "The best tool for round objects: barrels, vases, bottles, bowls, columns, wheels, knobs, lamp bases.\n\n\
                   **Profile:** a preset (`barrel`, `vase`, `bottle`, `bowl`, `column`, scaled by `height` and `radius`), points \
                   `[radius, height]` in meters (`smooth: true` for a curve through them), or `{\"curve\": \"name\"}` for a curve \
                   object drawn in its XY plane (x is the radius). Points may run up or down; faces always point out of the solid. \
                   Only the ends of an open profile may touch the axis (they close as poles). `closed: true` makes rings (tires, washers).\n\n\
                   **Groups:** `side` (all revolved faces), `profile_<i>` per profile span from the bottom (loop cuts, hoops, \
                   materials), `cap_top` and `cap_bottom` for filled ends. Cap rims and profile corners sharper than 30 degrees \
                   get sharp edges.\n\n\
                   **Caps** are `quad_grid` by default (needs a multiple of 4 segments), `ngon`, `fan` or `none`. Segments adapt \
                   to the largest radius and the asset class when omitted.",
            examples: &[
                OpExample {
                    args: r#"{"name": "barrel", "profile": "barrel", "segments": 24}"#,
                    result: "created barrel (o:4) lathe 0.62 x 0.9 x 0.62 m: 504 faces (1,008 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"name": "knob", "profile": [[0.02, 0], [0.03, 0.01], [0.025, 0.03], [0, 0.035]], "smooth": true, "segments": 16}"#,
                    result: "created knob (o:4) lathe 61.25 x 35 x 61.25 mm: 336 faces (656 tris; budget 6,000)",
                },
            ],
            aliases: &["lathe", "spin", "screw", "revolve", "surface of revolution", "turn"],
            complexity: "O(profile points x segments + segments^2) for quad-grid caps",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &LatheParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_count("/resolution", p.resolution, 1, MAX_RESOLUTION)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, MAX_SEGMENTS)?;
        }
        let preset = matches!(p.profile, ProfileParam::Preset(_));
        for (path, v) in [("/height", p.height), ("/radius", p.radius)] {
            match v {
                Some(_) if !preset => {
                    return Err(OpError::invalid(path, "height and radius scale presets only")
                        .with_hint("give profile points in meters instead, for example [[0.3, 0], [0.35, 0.45], [0.3, 0.9]]"));
                }
                Some(l) => check_length(path, l, 1e-4, 1e4)?,
                None => {}
            }
        }
        if p.closed && !matches!(p.profile, ProfileParam::Points(_)) {
            return Err(OpError::invalid("/closed", "closed applies to inline points; curves carry their own closed flag"));
        }
        if let ProfileParam::Points(pts) = &p.profile
            && !(2..=MAX_PROFILE_POINTS).contains(&pts.len())
        {
            return Err(OpError::invalid("/profile", format!("{} points; a profile needs 2 to {MAX_PROFILE_POINTS}", pts.len())));
        }
        if !(p.angle.degrees() > 0.0 && p.angle.degrees() <= 360.0) {
            return Err(OpError::out_of_range("/angle", "the angle must be above 0 and at most 360 degrees"));
        }
        if let ProfileParam::Curve { curve } = &p.profile {
            curve_target(doc, curve, "/profile/curve")?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &LatheParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let profile = lathe_profile(tx.doc(), p)?;
        let full = p.angle.degrees() >= 360.0 - 1e-9;
        let open_ends = !profile.closed && full && p.cap != CapParam::None;
        let segments = match p.segments {
            Some(s) => s,
            None => {
                let r = profile.points.iter().map(|q| q.x).fold(0.0, f64::max);
                let multiple = if open_ends && p.cap == CapParam::QuadGrid { 4 } else { 1 };
                tx.doc().settings.round_segments(std::f64::consts::TAU * r, multiple)
            }
        };
        let spec = LatheSpec {
            profile: &profile.points,
            spans: &profile.spans,
            closed: profile.closed,
            segments,
            angle: p.angle.radians(),
            cap: CapKind::from(p.cap),
            axis: axis(p.axis),
        };
        let mesh = lathe(&spec).map_err(gen_error)?;
        let source = match &p.profile {
            ProfileParam::Curve { curve } => Some(curve_target(tx.doc(), curve, "/profile/curve")?),
            _ => None,
        };
        let key = create_gen(tx, p.name.as_deref(), "lathe", p.parent.as_deref(), p.position.as_ref(), source, mesh)?;
        Ok(created_summary(tx, key, "lathe"))
    }
}

// ---- gen.extrude_shape -----------------------------------------------------------------

/// Which rims of an extruded shape get the bevel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BevelSides {
    #[default]
    Both,
    Front,
    Back,
}

/// Rim bevel of an extruded shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeBevel {
    /// Bevel width, inward from the outline and up from the cap.
    pub width: Length,
    /// 1 is a chamfer, 2 or more round it.
    #[serde(default = "two")]
    pub segments: u32,
    /// Rims to bevel.
    #[serde(default)]
    pub sides: BevelSides,
}

fn two() -> u32 {
    2
}

fn tenth() -> Length {
    Length(0.1)
}

/// Parameters of `gen.extrude_shape`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeShapeParams {
    /// Object name; a free name such as `shape_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Outline points `[x, y]` in the shape plane (closed automatically, either winding).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Vec<[Length; 2]>>,
    /// Holes inside the outline, as point lists like the outline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<[Length; 2]>>,
    /// A curve object of closed splines instead of `outline` (inner splines become holes); its plane is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
    /// Plane of `outline` points; the shape extrudes along its normal (+Z, +Y, +X).
    #[serde(default = "front_plane")]
    pub plane: crate::curve::PlaneParam,
    /// Thickness along the plane's normal (the back stays on the plane).
    #[serde(default = "tenth")]
    pub depth: Length,
    /// Rounded or chamfered rims; sharp when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bevel: Option<ShapeBevel>,
    /// Segments per curved span of Bezier splines.
    #[serde(default = "gen_resolution")]
    pub resolution: u32,
    /// Origin position in the parent's space. Built from a curve object, the result takes the curve's place
    /// (parent and transform) unless `parent` or `position` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
}

fn front_plane() -> crate::curve::PlaneParam {
    crate::curve::PlaneParam::Xy
}

impl Default for ExtrudeShapeParams {
    fn default() -> Self {
        let sq = |h: f64| vec![[Length(-h), Length(-h)], [Length(h), Length(-h)], [Length(h), Length(h)], [Length(-h), Length(h)]];
        ExtrudeShapeParams {
            name: None,
            parent: None,
            outline: Some(sq(0.5)),
            holes: Vec::new(),
            curve: None,
            plane: front_plane(),
            depth: tenth(),
            bevel: None,
            resolution: GEN_RESOLUTION,
            position: None,
        }
    }
}

/// Shape plane coordinates of a point: `(u, v)` and the height along the normal.
fn to_plane(plane: crate::curve::PlaneParam, p: Vec3) -> (DVec2, f64) {
    let p = p.as_dvec3();
    match plane {
        crate::curve::PlaneParam::Xy => (DVec2::new(p.x, p.y), p.z),
        crate::curve::PlaneParam::Xz => (DVec2::new(p.x, -p.z), p.y),
        crate::curve::PlaneParam::Yz => (DVec2::new(p.y, p.z), p.x),
    }
}

fn plane_axis(plane: crate::curve::PlaneParam) -> Axis {
    match plane {
        crate::curve::PlaneParam::Xy => Axis::Z,
        crate::curve::PlaneParam::Xz => Axis::Y,
        crate::curve::PlaneParam::Yz => Axis::X,
    }
}

/// Contours of a curve object of closed splines, the axis plane they lie
/// in and the plane's height.
///
/// Complexity: O(points x resolution).
fn curve_contours(doc: &Document, target: &str, resolution: u32) -> Result<(Vec<Vec<DVec2>>, crate::curve::PlaneParam, f64), OpError> {
    let key = curve_target(doc, target, "/curve")?;
    let c = doc.object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?;
    if let Some(i) = c.splines.iter().position(|s| !s.closed) {
        return Err(OpError::invalid("/curve", format!("spline {i} is open; a shape needs closed outlines"))
            .with_hint("make closed curves, for example curve.polyline with closed: true or curve.rectangle"));
    }
    curve_contours_any(&c.splines, resolution, "/curve")
}

/// `gen.extrude_shape`.
pub struct ExtrudeShapeOp;

impl Op for ExtrudeShapeOp {
    const ID: &'static str = "gen.extrude_shape";
    type Params = ExtrudeShapeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Extrude shape",
            category: Category::Gen,
            summary: "Turns a flat outline with holes into a solid of a given depth, with optional round or chamfered rims.",
            docs: "Give the outline as `outline` points (plus `holes`) in a `plane`, or as a `curve` object whose closed splines \
                   nest into holes and islands (its own plane is used). The back stays on the plane and the front moves `depth` \
                   along its normal: +Z for `xy` (signs, plaques, panels), +Y for `xz` (floor plans to walls and slabs), +X for `yz`.\n\n\
                   **Groups:** `front`, `back`, `side`, and `bevel` with a bevel. Caps are constrained Delaunay fills merged into \
                   quads; walls are quads. Side edges at outline corners and sharp rims are marked sharp; a round bevel (2 or more \
                   segments) blends smoothly. Bevels inset the outline, so keep them narrower than half the thinnest part.\n\n\
                   **Use for** signs, frames, gaskets, cookie-cutter shapes, logos, floor slabs, window panels, gears.",
            examples: &[
                OpExample {
                    args: r#"{"name": "frame", "outline": [[-0.3, -0.2], [0.3, -0.2], [0.3, 0.2], [-0.3, 0.2]], "holes": [[[-0.25, -0.15], [0.25, -0.15], [0.25, 0.15], [-0.25, 0.15]]], "depth": "2cm"}"#,
                    result: "created frame (o:4) shape 0.6 m x 0.4 m x 20 mm: 16 faces (32 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"name": "slab", "curve": "outline", "depth": "15cm", "bevel": {"width": "1cm", "segments": 2, "sides": "front"}}"#,
                    result: "created slab (o:4) shape 0.8 x 0.15 x 0.6 m: 14 faces (28 tris; budget 6,000)",
                },
            ],
            aliases: &["extrude shape", "solidify outline", "extrude curve", "linear extrude", "fill and extrude", "plaque"],
            complexity: "O(n log n) in outline points, plus O(n x bevel segments)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ExtrudeShapeParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_count("/resolution", p.resolution, 1, MAX_RESOLUTION)?;
        check_length("/depth", p.depth, 1e-5, 1e4)?;
        match (&p.outline, &p.curve) {
            (Some(o), None) => {
                if o.len() < 3 {
                    return Err(OpError::invalid("/outline", "an outline needs at least 3 points"));
                }
                if let Some(i) = p.holes.iter().position(|h| h.len() < 3) {
                    return Err(OpError::invalid(&format!("/holes/{i}"), "a hole needs at least 3 points"));
                }
            }
            (None, Some(c)) => {
                curve_target(doc, c, "/curve")?;
                if !p.holes.is_empty() {
                    return Err(OpError::invalid("/holes", "holes come from the curve's inner splines when curve is given"));
                }
            }
            _ => {
                return Err(
                    OpError::invalid("", "give exactly one of outline and curve").with_hint("for example {\"outline\": [[0, 0], [1, 0], [1, 1]]}")
                );
            }
        }
        if let Some(b) = &p.bevel {
            check_length("/bevel/width", b.width, 0.0, 1e3)?;
            check_count("/bevel/segments", b.segments, 1, p3d_geo::prim::MAX_BEVEL_SEGMENTS)?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &ExtrudeShapeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let (contours, plane, height, path) = match (&p.outline, &p.curve) {
            (Some(o), _) => {
                let pts = |c: &[[Length; 2]]| c.iter().map(|[x, y]| DVec2::new(x.0, y.0)).collect::<Vec<_>>();
                let mut cs = vec![pts(o)];
                cs.extend(p.holes.iter().map(|h| pts(h)));
                (cs, p.plane, 0.0, "/outline")
            }
            (None, Some(c)) => {
                let (cs, plane, h) = curve_contours(tx.doc(), c, p.resolution)?;
                (cs, plane, h, "/curve")
            }
            (None, None) => return Err(OpError::invalid("", "give exactly one of outline and curve")),
        };
        let (w, segs, sides) = p.bevel.as_ref().map_or((0.0, 1, BevelSides::Both), |b| (b.width.0, b.segments, b.sides));
        let spec = p3d_geo::generate::ShapeSpec {
            contours: &contours,
            depth: p.depth.0,
            bevel: w,
            bevel_segments: segs,
            bevel_front: sides != BevelSides::Back,
            bevel_back: sides != BevelSides::Front,
            axis: plane_axis(plane),
        };
        let mut mesh = p3d_geo::generate::extrude_shape(&spec).map_err(|e| match e {
            PrimError::InvalidParam { param: "shape", value, reason } => OpError::invalid(path, format!("{value} {reason}")),
            PrimError::InvalidParam { param: "bevel", value, reason } => OpError::out_of_range("/bevel/width", format!("{value}: {reason}")),
            e => gen_error(e),
        })?;
        if height != 0.0 {
            let shift = plane_axis(plane).unit() * height as f32;
            for q in &mut mesh.positions {
                *q += shift;
            }
        }
        let source = p.curve.as_deref().map(|c| curve_target(tx.doc(), c, "/curve")).transpose()?;
        let key = create_gen(tx, p.name.as_deref(), "shape", p.parent.as_deref(), p.position.as_ref(), source, mesh)?;
        Ok(created_summary(tx, key, "shape"))
    }
}

// ---- sections and paths ----------------------------------------------------------------

/// A sweep cross section: points, a curve object, a circle or a rectangle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SectionParam {
    Points(Vec<[Length; 2]>),
    Curve { curve: String },
    Circle { circle: Length },
    Rectangle { rectangle: [Length; 2] },
}

impl Default for SectionParam {
    fn default() -> Self {
        SectionParam::Rectangle { rectangle: [Length(0.05), Length(0.05)] }
    }
}

impl<'de> Deserialize<'de> for SectionParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let shape =
            "a cross section is a list of [x, y] points, {\"curve\": \"<name>\"}, {\"circle\": <radius>} or {\"rectangle\": [<width>, <height>]}";
        match v {
            Value::Array(_) => serde_json::from_value(v)
                .map(SectionParam::Points)
                .map_err(|e| D::Error::custom(format!("cross section points are [x, y] pairs: {e}"))),
            Value::Object(m) if m.len() == 1 => {
                let (k, val) = m.into_iter().next().ok_or_else(|| D::Error::custom(shape))?;
                match k.as_str() {
                    "curve" => serde_json::from_value(val).map(|curve| SectionParam::Curve { curve }).map_err(D::Error::custom),
                    "circle" => serde_json::from_value(val).map(|circle| SectionParam::Circle { circle }).map_err(D::Error::custom),
                    "rectangle" => serde_json::from_value(val).map(|rectangle| SectionParam::Rectangle { rectangle }).map_err(D::Error::custom),
                    _ => Err(D::Error::custom(shape)),
                }
            }
            _ => Err(D::Error::custom(shape)),
        }
    }
}

impl JsonSchema for SectionParam {
    fn schema_name() -> Cow<'static, str> {
        "Section".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let length = g.subschema_for::<Length>();
        json_schema!({
            "description": "A cross section seen looking along the path (x right, y up): points [x, y], {\"curve\": \"<curve object>\"} (its axis plane), {\"circle\": radius} or {\"rectangle\": [width, height]}.",
            "anyOf": [
                { "type": "array", "minItems": 2, "items": { "type": "array", "items": length, "minItems": 2, "maxItems": 2 } },
                { "type": "object", "properties": { "curve": { "type": "string" } }, "required": ["curve"], "additionalProperties": false },
                { "type": "object", "properties": { "circle": length }, "required": ["circle"], "additionalProperties": false },
                { "type": "object", "properties": { "rectangle": { "type": "array", "items": length, "minItems": 2, "maxItems": 2 } }, "required": ["rectangle"], "additionalProperties": false }
            ]
        })
    }
}

/// A path: points or a curve object.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PathParam {
    Points(Vec<[Length; 3]>),
    Curve { curve: String },
}

impl Default for PathParam {
    fn default() -> Self {
        PathParam::Points(vec![[Length(0.0); 3], [Length(0.0), Length(1.0), Length(0.0)]])
    }
}

impl<'de> Deserialize<'de> for PathParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        match v {
            Value::Array(_) => {
                serde_json::from_value(v).map(PathParam::Points).map_err(|e| D::Error::custom(format!("path points are [x, y, z]: {e}")))
            }
            Value::Object(m) => match (m.len(), m.get("curve")) {
                (1, Some(Value::String(c))) => Ok(PathParam::Curve { curve: c.clone() }),
                _ => Err(D::Error::custom("a path object is {\"curve\": \"<curve object>\"}")),
            },
            _ => Err(D::Error::custom("a path is a list of [x, y, z] points or {\"curve\": \"<name>\"}")),
        }
    }
}

impl JsonSchema for PathParam {
    fn schema_name() -> Cow<'static, str> {
        "Path".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let length = g.subschema_for::<Length>();
        json_schema!({
            "description": "A path: points [x, y, z] in object space, or {\"curve\": \"<curve object>\"} (one spline; the result lands where the curve is).",
            "anyOf": [
                { "type": "array", "minItems": 2, "items": { "type": "array", "items": length, "minItems": 3, "maxItems": 3 } },
                { "type": "object", "properties": { "curve": { "type": "string" } }, "required": ["curve"], "additionalProperties": false }
            ]
        })
    }
}

/// A resolved path: points, closed, and the curve object it came from.
struct Path3 {
    points: Vec<glam::DVec3>,
    closed: bool,
    source: Option<ObjectKey>,
}

/// Resolves a path, sampling Bezier spans (and smoothing inline points
/// when `smooth`).
///
/// Complexity: O(points x resolution).
fn resolve_path(doc: &Document, path: &PathParam, smooth: bool, closed: bool, resolution: u32) -> Result<Path3, OpError> {
    match path {
        PathParam::Points(raw) => {
            let pts: Vec<Vec3> = raw.iter().map(crate::params::vec3).collect();
            let sampled =
                if smooth { curves::sample_bezier(&pts, &curves::auto_handles(&pts, closed), closed, resolution).map_err(curve_error)? } else { pts };
            Ok(Path3 { points: sampled.iter().map(|p| p.as_dvec3()).collect(), closed, source: None })
        }
        PathParam::Curve { curve } => {
            let key = curve_target(doc, curve, "/path/curve")?;
            let c = doc.object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?;
            let [s]: &[Spline; 1] = c
                .splines
                .as_slice()
                .try_into()
                .map_err(|_| OpError::invalid("/path/curve", format!("the curve has {} splines; a path is one", c.splines.len())))?;
            let pts = crate::curve::polyline(s, resolution).map_err(curve_error)?;
            Ok(Path3 { points: pts.iter().map(|p| p.as_dvec3()).collect(), closed: s.closed, source: Some(key) })
        }
    }
}

// ---- gen.sweep ---------------------------------------------------------------------------

fn ones() -> [f64; 2] {
    [1.0, 1.0]
}

fn yes() -> bool {
    true
}

/// Parameters of `gen.sweep`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SweepParams {
    /// Object name; a free name such as `sweep_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The cross section.
    pub profile: SectionParam,
    /// Whether inline profile points close into a loop (open profiles make trims and ribbons).
    #[serde(default = "yes")]
    pub profile_closed: bool,
    /// The path the cross section follows.
    pub path: PathParam,
    /// Joins the last inline path point back to the first.
    #[serde(default)]
    pub path_closed: bool,
    /// A smooth Bezier through inline path points instead of straight segments.
    #[serde(default)]
    pub smooth_path: bool,
    /// Turn of the cross section from start to end (degrees); closed paths need whole turns.
    #[serde(default)]
    pub twist: Angle,
    /// Cross section scale at the start and the end (taper).
    #[serde(default = "ones")]
    pub scale: [f64; 2],
    /// Fills the ends of a closed cross section on an open path.
    #[serde(default = "yes")]
    pub caps: bool,
    /// Points around a circle cross section; adapts to its size when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<u32>,
    /// Direction the cross section's y leans toward at the start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<[f64; 3]>,
    /// Segments per curved span of Bezier curves.
    #[serde(default = "gen_resolution")]
    pub resolution: u32,
    /// Origin position in the parent's space. Built from a path curve, the result takes the curve's place
    /// (parent and transform) unless `parent` or `position` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
}

impl Default for SweepParams {
    fn default() -> Self {
        SweepParams {
            name: None,
            parent: None,
            profile: SectionParam::default(),
            profile_closed: true,
            path: PathParam::default(),
            path_closed: false,
            smooth_path: false,
            twist: Angle(0.0),
            scale: ones(),
            caps: true,
            segments: None,
            up: None,
            resolution: GEN_RESOLUTION,
            position: None,
        }
    }
}

/// A resolved cross section: points, spans for groups, closed, round.
struct Section {
    points: Vec<DVec2>,
    spans: Vec<u32>,
    closed: bool,
    round: bool,
}

/// Resolves a cross section.
///
/// Complexity: O(points x resolution).
fn resolve_section(doc: &Document, p: &SectionParam, closed: bool, segments: Option<u32>, resolution: u32) -> Result<Section, OpError> {
    match p {
        SectionParam::Points(raw) => {
            let points: Vec<DVec2> = raw.iter().map(|[x, y]| DVec2::new(x.0, y.0)).collect();
            let segs = if closed { points.len() } else { points.len().saturating_sub(1) };
            Ok(Section { points, spans: (0..segs as u32).collect(), closed, round: false })
        }
        SectionParam::Rectangle { rectangle: [w, h] } => {
            let (x, y) = (w.0 / 2.0, h.0 / 2.0);
            let points = vec![DVec2::new(-x, -y), DVec2::new(x, -y), DVec2::new(x, y), DVec2::new(-x, y)];
            Ok(Section { points, spans: vec![0, 1, 2, 3], closed: true, round: false })
        }
        SectionParam::Circle { circle } => {
            let s = segments.unwrap_or_else(|| doc.settings.round_segments(std::f64::consts::TAU * circle.0, 4).clamp(8, 32));
            Ok(Section { points: p3d_geo::generate::round_profile(circle.0, s), spans: Vec::new(), closed: true, round: s.is_multiple_of(4) })
        }
        SectionParam::Curve { curve } => {
            let key = curve_target(doc, curve, "/profile/curve")?;
            let c = doc.object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?;
            let [s]: &[Spline; 1] = c
                .splines
                .as_slice()
                .try_into()
                .map_err(|_| OpError::invalid("/profile/curve", format!("the curve has {} splines; a cross section is one", c.splines.len())))?;
            let (contours, _, _) = curve_contours_any(std::slice::from_ref(s), resolution, "/profile/curve")?;
            let pts = contours.into_iter().next().unwrap_or_default();
            Ok(Section { points: pts, spans: Vec::new(), closed: s.closed, round: false })
        }
    }
}

/// Splines sampled and mapped to the axis plane they lie in.
///
/// Complexity: O(points x resolution).
fn curve_contours_any(splines: &[Spline], resolution: u32, path: &str) -> Result<(Vec<Vec<DVec2>>, crate::curve::PlaneParam, f64), OpError> {
    let mut sampled = Vec::with_capacity(splines.len());
    for s in splines {
        sampled.push(crate::curve::polyline(s, resolution).map_err(curve_error)?);
    }
    let (lo, hi) = sampled.iter().flatten().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let size = hi - lo;
    let flat = |s: f32| s <= size.max_element() * 1e-5;
    let plane = match (flat(size.x), flat(size.y), flat(size.z)) {
        (false, false, true) => crate::curve::PlaneParam::Xy,
        (false, true, false) => crate::curve::PlaneParam::Xz,
        (true, false, false) => crate::curve::PlaneParam::Yz,
        _ => {
            return Err(OpError::invalid(path, "the curve does not lie flat in the XY, XZ or YZ plane")
                .with_hint("draw shapes with curve.* in one plane (the plane parameter)"));
        }
    };
    let height = to_plane(plane, (lo + hi) * 0.5).1;
    let contours = sampled.iter().map(|s| s.iter().map(|p| to_plane(plane, *p).0).collect()).collect();
    Ok((contours, plane, height))
}

fn sweep_error(e: PrimError) -> OpError {
    match e {
        PrimError::InvalidParam { param: "caps", value, reason } => OpError::invalid("/caps", format!("{value} {reason}")),
        PrimError::InvalidParam { param: "twist", value, reason } => OpError::out_of_range("/twist", format!("{value}: {reason}")),
        e => gen_error(e),
    }
}

fn check_path_points(path: &PathParam, closed: bool) -> Result<(), OpError> {
    if let PathParam::Points(p) = path {
        let min = if closed { 3 } else { 2 };
        if p.len() < min || p.len() > p3d_geo::generate::MAX_PATH_POINTS {
            return Err(OpError::invalid("/path", format!("{} points; a path needs at least {min}", p.len())));
        }
    }
    Ok(())
}

fn check_up(up: Option<[f64; 3]>) -> Result<Option<glam::DVec3>, OpError> {
    match up {
        None => Ok(None),
        Some(u) if u.iter().all(|x| x.is_finite()) && u.iter().any(|&x| x != 0.0) => Ok(Some(glam::DVec3::from(u))),
        Some(_) => Err(OpError::invalid("/up", "up must be a nonzero vector").with_hint("for example [0, 1, 0]")),
    }
}

/// `gen.sweep`.
pub struct SweepOp;

impl Op for SweepOp {
    const ID: &'static str = "gen.sweep";
    type Params = SweepParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Sweep",
            category: Category::Gen,
            summary: "Carries a cross section along a path into rails, trims, frames, handles and cables, with twist and taper.",
            docs: "The cross section is seen looking along the path (x right, y up) and rides on rotation-minimizing frames, so it \
                   never spins on its own; `twist` turns it on purpose and `scale` tapers it from start to end. Sharp path corners \
                   are mitered so the section keeps its thickness (a frame swept along a closed rectangle). `up` sets which way the \
                   section's y points at the start (+Y by default).\n\n\
                   **Cross sections:** `{\"rectangle\": [w, h]}`, `{\"circle\": r}` (quad-grid caps; see also `gen.pipe`), points \
                   `[x, y]` (`profile_closed: false` for open trims), or `{\"curve\": name}`. **Paths:** points (`smooth_path` for a \
                   curve through them, `path_closed` for loops) or `{\"curve\": name}`; built from a path curve, the result lands \
                   where the curve is.\n\n\
                   **Groups:** `side`, `cap_start`, `cap_end`, and `profile_<i>` per side of a rectangle or point section. Section \
                   corners and path joints sharper than 30 degrees get sharp edges.",
            examples: &[
                OpExample {
                    args: r#"{"name": "rail", "profile": {"rectangle": ["4cm", "2cm"]}, "path": {"curve": "path"}}"#,
                    result: "created rail (o:4) sweep 1.02 m x 20 mm x 1.02 m: 10 faces (20 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"name": "horn", "profile": {"circle": "4cm"}, "segments": 12, "path": [[0, 0, 0], [0, 0.3, 0], [0.2, 0.5, 0]], "smooth_path": true, "scale": [1, 0.1]}"#,
                    result: "created horn (o:4) sweep 0.2464 m x 0.5049 m x 77.27 mm: 210 faces (420 tris; budget 6,000)",
                },
            ],
            aliases: &["sweep", "curve bevel", "extrude along path", "loft along path", "follow me", "tube along curve"],
            complexity: "O(path points x profile points) plus O(p log p) per cap",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SweepParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_count("/resolution", p.resolution, 1, MAX_RESOLUTION)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, 256)?;
        }
        check_path_points(&p.path, p.path_closed)?;
        check_up(p.up)?;
        if p.scale.iter().any(|s| !(s.is_finite() && (1e-3..=1e3).contains(s))) {
            return Err(OpError::out_of_range("/scale", "scales must be between 0.001 and 1000"));
        }
        match &p.profile {
            SectionParam::Points(pts) if pts.len() < 2 => return Err(OpError::invalid("/profile", "a cross section needs at least 2 points")),
            SectionParam::Circle { circle } => check_length("/profile/circle", *circle, 1e-5, 1e3)?,
            SectionParam::Rectangle { rectangle } => {
                check_length("/profile/rectangle/0", rectangle[0], 1e-5, 1e3)?;
                check_length("/profile/rectangle/1", rectangle[1], 1e-5, 1e3)?;
            }
            SectionParam::Curve { curve } => {
                curve_target(doc, curve, "/profile/curve")?;
            }
            SectionParam::Points(_) => {}
        }
        if let PathParam::Curve { curve } = &p.path {
            curve_target(doc, curve, "/path/curve")?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SweepParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let section = resolve_section(tx.doc(), &p.profile, p.profile_closed, p.segments, p.resolution)?;
        let path = resolve_path(tx.doc(), &p.path, p.smooth_path, p.path_closed, p.resolution)?;
        let caps = match (p.caps, section.round) {
            (false, _) => p3d_geo::generate::SweepCaps::None,
            (true, true) => p3d_geo::generate::SweepCaps::QuadGrid,
            (true, false) => p3d_geo::generate::SweepCaps::Fill,
        };
        let spec = p3d_geo::generate::SweepSpec {
            profile: &section.points,
            profile_closed: section.closed,
            profile_spans: &section.spans,
            path: &path.points,
            path_closed: path.closed,
            twist: p.twist.radians(),
            scale: p.scale,
            caps,
            up: check_up(p.up)?.unwrap_or(glam::DVec3::Y),
        };
        let mesh = p3d_geo::generate::sweep(&spec).map_err(sweep_error)?;
        let key = create_gen(tx, p.name.as_deref(), "sweep", p.parent.as_deref(), p.position.as_ref(), path.source, mesh)?;
        Ok(created_summary(tx, key, "sweep"))
    }
}

// ---- gen.pipe ----------------------------------------------------------------------------

fn two_cm() -> Length {
    Length(0.02)
}

fn bend_segments() -> u32 {
    4
}

/// Parameters of `gen.pipe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PipeParams {
    /// Object name; a free name such as `pipe_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The path the pipe follows.
    pub path: PathParam,
    /// Joins the last inline path point back to the first (rings).
    #[serde(default)]
    pub path_closed: bool,
    /// A smooth Bezier through inline path points instead of straight runs with bends.
    #[serde(default)]
    pub smooth_path: bool,
    /// Pipe radius.
    #[serde(default = "two_cm")]
    pub radius: Length,
    /// Points around the pipe; adapts to its size when omitted (a multiple of 4 gives quad caps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<u32>,
    /// Radius of the bends at path corners (twice the pipe radius when omitted; 0 for mitered joints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend_radius: Option<Length>,
    /// Segments per bend.
    #[serde(default = "bend_segments")]
    pub bend_segments: u32,
    /// Closes the ends of an open pipe.
    #[serde(default = "yes")]
    pub caps: bool,
    /// Segments per curved span of Bezier curves.
    #[serde(default = "gen_resolution")]
    pub resolution: u32,
    /// Origin position in the parent's space. Built from a path curve, the result takes the curve's place
    /// (parent and transform) unless `parent` or `position` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
}

impl Default for PipeParams {
    fn default() -> Self {
        PipeParams {
            name: None,
            parent: None,
            path: PathParam::default(),
            path_closed: false,
            smooth_path: false,
            radius: two_cm(),
            segments: None,
            bend_radius: None,
            bend_segments: bend_segments(),
            caps: true,
            resolution: GEN_RESOLUTION,
            position: None,
        }
    }
}

/// `gen.pipe`.
pub struct PipeOp;

impl Op for PipeOp {
    const ID: &'static str = "gen.pipe";
    type Params = PipeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Pipe",
            category: Category::Gen,
            summary: "Builds a round pipe along a path with smooth bends at the corners and quad caps.",
            docs: "Corners of the path become bends of `bend_radius` (twice the pipe radius by default, shrunk where runs are \
                   short; 0 keeps mitered joints) with `bend_segments` segments. The cross section rides on rotation-minimizing \
                   frames, so long pipes never twist. Caps are quad grids when `segments` is a multiple of 4.\n\n\
                   **Paths:** points (`smooth_path` for a curve through them, `path_closed` for rings) or `{\"curve\": name}`; built \
                   from a path curve, the pipe lands where the curve is.\n\n\
                   **Groups:** `side`, `cap_start`, `cap_end`.\n\n\
                   **Use for** plumbing, railings, cables, hoses, wires, chair frames, bike handlebars. For square tubes use \
                   `gen.sweep` with a rectangle.",
            examples: &[OpExample {
                args: r#"{"name": "railing", "path": [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]], "radius": "2cm", "segments": 8}"#,
                result: "created railing (o:4) pipe 1.04 m x 1.02 m x 40 mm: 112 faces (224 tris; budget 6,000)",
            }],
            aliases: &["pipe", "tube along path", "cable", "wire", "hose", "railing", "curve bevel round"],
            complexity: "O(path points x segments)",
            flags: FLAGS,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &PipeParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_count("/resolution", p.resolution, 1, MAX_RESOLUTION)?;
        check_length("/radius", p.radius, 1e-5, 1e3)?;
        if let Some(s) = p.segments {
            check_count("/segments", s, 3, 256)?;
        }
        if let Some(b) = p.bend_radius {
            check_length("/bend_radius", b, 0.0, 1e4)?;
        }
        check_count("/bend_segments", p.bend_segments, 1, curves::MAX_FILLET_SEGMENTS)?;
        check_path_points(&p.path, p.path_closed)?;
        if let PathParam::Curve { curve } = &p.path {
            curve_target(doc, curve, "/path/curve")?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &PipeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let path = resolve_path(tx.doc(), &p.path, p.smooth_path, p.path_closed, p.resolution)?;
        let bend = p.bend_radius.map_or(2.0 * p.radius.0, |b| b.0);
        let points = if bend > 0.0 && path.points.len() > 2 {
            let pts: Vec<Vec3> = path.points.iter().map(|q| q.as_vec3()).collect();
            let rounded = curves::fillet(&pts, path.closed, bend as f32, p.bend_segments).map_err(|e| match e {
                p3d_geo::edit::KernelError::OutOfRange { message, .. } => OpError::out_of_range("/bend_radius", message),
                e => curve_error(e),
            })?;
            rounded.iter().map(|q| q.as_dvec3()).collect()
        } else {
            path.points.clone()
        };
        let segments = p.segments.unwrap_or_else(|| tx.doc().settings.round_segments(std::f64::consts::TAU * p.radius.0, 4).clamp(8, 32));
        let profile = p3d_geo::generate::round_profile(p.radius.0, segments);
        let caps = match (p.caps, segments.is_multiple_of(4)) {
            (false, _) => p3d_geo::generate::SweepCaps::None,
            (true, true) => p3d_geo::generate::SweepCaps::QuadGrid,
            (true, false) => p3d_geo::generate::SweepCaps::Fill,
        };
        let spec = p3d_geo::generate::SweepSpec {
            profile: &profile,
            profile_closed: true,
            profile_spans: &[],
            path: &points,
            path_closed: path.closed,
            twist: 0.0,
            scale: [1.0; 2],
            caps,
            up: glam::DVec3::Y,
        };
        let mesh = p3d_geo::generate::sweep(&spec).map_err(sweep_error)?;
        let key = create_gen(tx, p.name.as_deref(), "pipe", p.parent.as_deref(), p.position.as_ref(), path.source, mesh)?;
        Ok(created_summary(tx, key, "pipe"))
    }
}
