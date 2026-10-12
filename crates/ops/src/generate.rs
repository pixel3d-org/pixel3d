//! `gen.*`: generators that build meshes from profiles and paths.

use std::borrow::Cow;

use glam::{DVec2, Vec3};
use p3d_geo::curve::{self as curves, MAX_RESOLUTION};
use p3d_geo::edit::Axis;
use p3d_geo::generate::{LatheSpec, MAX_PROFILE_POINTS, lathe};
use p3d_geo::prim::{CapKind, MAX_SEGMENTS, PrimError};
use p3d_scene::{Document, Spline};
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
    /// Origin position in the parent's space.
    #[serde(default = "zero3")]
    pub position: [Length; 3],
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
            position: zero3(),
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
                    result: "created barrel (o:1) lathe 0.62 x 0.9 x 0.62 m: 504 faces (1,008 tris; budget 6,000)",
                },
                OpExample {
                    args: r#"{"name": "knob", "profile": [[0.02, 0], [0.03, 0.01], [0.025, 0.03], [0, 0.035]], "smooth": true, "segments": 16}"#,
                    result: "created knob (o:1) lathe 61.25 x 35 x 61.25 mm: 336 faces (656 tris; budget 6,000)",
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
        let key = create_mesh_object(tx, p.name.as_deref(), "lathe", p.parent.as_deref(), &p.position, mesh)?;
        Ok(created_summary(tx, key, "lathe"))
    }
}
