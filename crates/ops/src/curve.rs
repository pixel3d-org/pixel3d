//! `curve.*`: curve objects (polylines and Bezier splines) used as
//! profiles and paths by the generators.

use glam::Vec3;
use p3d_geo::curve::{self as kernel, MAX_FILLET_SEGMENTS, MAX_RESOLUTION, Plane};
use p3d_geo::edit::KernelError;
use p3d_math::format_length;
use p3d_scene::{Curve, Document, ObjectKey, Spline};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_count, check_length, check_new_object, extent, label};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Length, vec3};
use crate::tx::Tx;

const CREATE: OpFlags = OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS);

/// Most control points a curve op accepts per spline.
pub const MAX_CURVE_POINTS: usize = 100_000;
/// Bezier segments per span when a curve is sampled to a polyline.
pub const DEFAULT_RESOLUTION: u32 = 12;

pub(crate) fn kernel_error(e: KernelError) -> OpError {
    match e {
        KernelError::OutOfRange { param, message, hint } => OpError::out_of_range(&format!("/{param}"), message).with_hint(hint),
        KernelError::Empty(m) => OpError::new(ErrorCode::InvalidParam, m).at("/target"),
        KernelError::Topology(t) => OpError::new(ErrorCode::Topology, t.to_string()),
    }
}

/// A spline as a polyline: Bezier spans sampled with `resolution` segments
/// (straight spans stay one segment).
///
/// Complexity: O(points x resolution).
pub(crate) fn polyline(s: &Spline, resolution: u32) -> Result<Vec<Vec3>, KernelError> {
    let pts = s.positions();
    match &s.handles {
        None => Ok(pts),
        Some(h) => {
            let h: Vec<[Vec3; 2]> = h.iter().map(|[a, b]| [Vec3::from(*a), Vec3::from(*b)]).collect();
            kernel::sample_bezier(&pts, &h, s.closed, resolution)
        }
    }
}

/// A curve object's key, or an error at `path` naming what it is instead.
pub(crate) fn curve_target(doc: &Document, target: &str, path: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, path))?;
    if doc.object_curve(key).is_none() {
        let kind = doc.object(key).map_or("object", |o| o.kind().as_str());
        return Err(OpError::invalid(path, format!("{} is an {kind}, not a curve", label(doc, key)))
            .with_hint("make one with curve.polyline, curve.bezier, curve.circle or curve.rectangle"));
    }
    Ok(key)
}

/// `1 closed spline, 16 points, 1.87 m long, 0.6 x 0 x 0.6 m` (lengths and
/// size from the curve sampled at the default resolution).
///
/// Complexity: O(points x resolution).
fn stats(c: &Curve) -> String {
    let n = c.splines.len();
    let closed = c.splines.iter().filter(|s| s.closed).count();
    let bezier = c.splines.iter().any(Spline::is_bezier);
    let kind = if bezier { " bezier" } else { "" };
    let splines = match (n, closed) {
        (1, 1) => format!("1 closed{kind} spline"),
        (1, _) => format!("1 open{kind} spline"),
        (n, 0) => format!("{n} open{kind} splines"),
        (n, c) if c == n => format!("{n} closed{kind} splines"),
        (n, c) => format!("{n}{kind} splines ({c} closed)"),
    };
    let mut total = 0.0f32;
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for s in &c.splines {
        let pts = polyline(s, DEFAULT_RESOLUTION).unwrap_or_else(|_| s.positions());
        total += kernel::length(&pts, s.closed);
        for p in pts {
            bounds = Some(bounds.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
        }
    }
    let size = bounds.map_or_else(|| "empty".into(), |(lo, hi)| extent(lo, hi));
    format!("{splines}, {} points, {} long, {size}", c.point_count(), format_length(f64::from(total)))
}

/// `rim (o:4) curve: 1 closed spline, ...`.
fn describe(doc: &Document, key: ObjectKey) -> String {
    let s = doc.object_curve(key).map_or_else(|| "?".into(), stats);
    format!("{} curve: {s}", label(doc, key))
}

/// Creates a curve object named `name` (must be free) or a free variant of
/// `default_name`.
fn create(
    tx: &mut Tx,
    name: Option<&str>,
    default_name: &str,
    parent: Option<&str>,
    position: &[Length; 3],
    curve: Curve,
) -> Result<OpOutput, OpError> {
    let parent_key = check_new_object(tx.doc(), name, parent)?;
    let name = name.map_or_else(|| tx.doc().unique_name(default_name, parent_key), str::to_owned);
    let key = tx.doc_mut().create_curve_object(&name, parent_key, curve).map_err(|e| OpError::from_scene(e, "/name"))?;
    if let Some(o) = tx.doc_mut().object_mut(key) {
        o.transform.translation = vec3(position);
    }
    let mut out = OpOutput::summary(format!("created {}", describe(tx.doc(), key)));
    out.created.push(key);
    Ok(out)
}

/// Writes an edited curve back into its object, or into a new sibling
/// object named `output` with the same transform.
fn store(tx: &mut Tx, key: ObjectKey, curve: Curve, output: Option<&str>, what: &str) -> Result<OpOutput, OpError> {
    match output {
        None => {
            let ck = tx.doc().object(key).and_then(|o| o.curve_key()).ok_or_else(|| OpError::internal("curve vanished"))?;
            *tx.doc_mut().curve_mut(ck).ok_or_else(|| OpError::internal("curve vanished"))? = curve;
            let mut out =
                OpOutput::summary(format!("{}: {what}: {}", label(tx.doc(), key), tx.doc().object_curve(key).map_or_else(String::new, stats)));
            out.changed.push(key);
            Ok(out)
        }
        Some(name) => {
            let (parent, transform) = tx.doc().object(key).map(|o| (o.parent(), o.transform)).ok_or_else(|| OpError::internal("object vanished"))?;
            if let Some(p) = parent
                && tx.doc().object(p).is_none()
            {
                return Err(OpError::internal("parent vanished"));
            }
            p3d_scene::name::check_name(name).map_err(|e| OpError::invalid("/output", e.to_string()))?;
            let source = label(tx.doc(), key);
            let new = tx.doc_mut().create_curve_object(name, parent, curve).map_err(|e| OpError::from_scene(e, "/output"))?;
            if let Some(o) = tx.doc_mut().object_mut(new) {
                o.transform = transform;
            }
            let mut out = OpOutput::summary(format!("created {} ({what} from {source})", describe(tx.doc(), new)));
            out.created.push(new);
            Ok(out)
        }
    }
}

/// Checks an `output` name before anything changes.
fn check_output(doc: &Document, key: ObjectKey, output: Option<&str>) -> Result<(), OpError> {
    let Some(name) = output else { return Ok(()) };
    p3d_scene::name::check_name(name).map_err(|e| OpError::invalid("/output", e.to_string()))?;
    let parent = doc.object(key).and_then(|o| o.parent());
    if doc.unique_name(name, parent) != name {
        return Err(OpError::invalid("/output", format!("`{name}` is taken")).with_hint("pick a free name or omit output to edit in place"));
    }
    Ok(())
}

fn check_points(path: &str, n: usize, closed: bool) -> Result<(), OpError> {
    let min = if closed { 3 } else { 2 };
    if n < min {
        return Err(OpError::invalid(
            path,
            format!("{n} points are too few for an {} curve (at least {min})", if closed { "closed" } else { "open" }),
        ));
    }
    if n > MAX_CURVE_POINTS {
        return Err(OpError::out_of_range(path, format!("{n} points exceed the limit of {MAX_CURVE_POINTS}")));
    }
    Ok(())
}

fn points(p: &[[Length; 3]]) -> Vec<Vec3> {
    p.iter().map(vec3).collect()
}

fn check_resolution(r: u32) -> Result<(), OpError> {
    check_count("/resolution", r, 1, MAX_RESOLUTION)
}

fn zero3() -> [Length; 3] {
    [Length(0.0); 3]
}

fn default_resolution() -> u32 {
    DEFAULT_RESOLUTION
}

/// Each spline of a curve as a polyline.
///
/// Complexity: O(points x resolution).
fn polylines(c: &Curve, resolution: u32) -> Result<Vec<(Vec<Vec3>, bool)>, OpError> {
    c.splines.iter().map(|s| polyline(s, resolution).map(|p| (p, s.closed)).map_err(kernel_error)).collect()
}

fn polyline_curve(splines: Vec<(Vec<Vec3>, bool)>) -> Curve {
    Curve { splines: splines.into_iter().map(|(p, closed)| Spline::polyline(p, closed)).collect() }
}

/// The plane a flat curve is drawn in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaneParam {
    /// Front plane, facing +Z (profiles: X is the radius, Y the height).
    Xy,
    /// Ground plane, facing up (+Y).
    #[default]
    Xz,
    /// Side plane, facing +X.
    Yz,
}

impl From<PlaneParam> for Plane {
    fn from(p: PlaneParam) -> Plane {
        match p {
            PlaneParam::Xy => Plane::Xy,
            PlaneParam::Xz => Plane::Xz,
            PlaneParam::Yz => Plane::Yz,
        }
    }
}

// ---- curve.polyline ----------------------------------------------------------------------

/// Parameters of `curve.polyline`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolylineParams {
    /// Object name; a free name such as `polyline_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Points in object space, in order.
    pub points: Vec<[Length; 3]>,
    /// Joins the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    /// Origin position in the parent's space.
    #[serde(default = "zero3")]
    pub position: [Length; 3],
}

impl Default for PolylineParams {
    fn default() -> Self {
        PolylineParams {
            name: None,
            parent: None,
            points: vec![[Length(0.0); 3], [Length(1.0), Length(0.0), Length(0.0)]],
            closed: false,
            position: zero3(),
        }
    }
}

/// `curve.polyline`.
pub struct PolylineOp;

impl Op for PolylineOp {
    const ID: &'static str = "curve.polyline";
    type Params = PolylineParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Polyline",
            category: Category::Curve,
            summary: "Creates a curve object from straight segments through points, open or closed.",
            docs: "A curve object holds splines; generators read them as profiles (`gen.lathe`, `gen.extrude_shape`) or paths \
                   (`gen.sweep`, `gen.pipe`). A polyline keeps its corners sharp; round them with `curve.fillet` or use \
                   `curve.bezier` for smooth curves.\n\n\
                   **Use for** lathe profiles with crisp steps, floor plans, rails, cable paths, outlines.",
            examples: &[OpExample {
                args: r#"{"name": "rail", "points": [[0, 0, 0], [1, 0, 0], [1, 0, -1]]}"#,
                result: "created rail (o:4) curve: 1 open spline, 3 points, 2 m long, 1 x 0 x 1 m",
            }],
            aliases: &["polyline", "path", "poly curve", "line", "spline"],
            complexity: "O(points)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &PolylineParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_points("/points", p.points.len(), p.closed)
    }

    fn apply(tx: &mut Tx, p: &PolylineParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let curve = Curve::single(Spline::polyline(points(&p.points), p.closed));
        create(tx, p.name.as_deref(), "polyline", p.parent.as_deref(), &p.position, curve)
    }
}

// ---- curve.bezier ----------------------------------------------------------------------

/// Parameters of `curve.bezier`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BezierParams {
    /// Object name; a free name such as `bezier_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Points the curve passes through, in object space.
    pub points: Vec<[Length; 3]>,
    /// One `[in, out]` handle pair per point (absolute positions); smooth
    /// automatic handles when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<Vec<[[Length; 3]; 2]>>,
    /// Joins the last point back to the first.
    #[serde(default)]
    pub closed: bool,
    /// Origin position in the parent's space.
    #[serde(default = "zero3")]
    pub position: [Length; 3],
}

impl Default for BezierParams {
    fn default() -> Self {
        BezierParams {
            name: None,
            parent: None,
            points: vec![[Length(0.0); 3], [Length(0.5), Length(0.5), Length(0.0)], [Length(1.0), Length(0.0), Length(0.0)]],
            handles: None,
            closed: false,
            position: zero3(),
        }
    }
}

/// `curve.bezier`.
pub struct BezierOp;

impl Op for BezierOp {
    const ID: &'static str = "curve.bezier";
    type Params = BezierParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Bezier curve",
            category: Category::Curve,
            summary: "Creates a smooth cubic Bezier curve through points, with automatic or explicit handles.",
            docs: "Without `handles` the curve passes smoothly through every point (tangents bisect the neighbouring chords, \
                   handles a third of the chord long, so uneven spacing does not overshoot). Give `handles` for exact control: \
                   one `[in, out]` pair per point; a handle on the straight line to its neighbour makes that span straight.\n\n\
                   Generators sample each curved span with their `resolution` (12 by default); straight spans stay one segment.\n\n\
                   **Use for** vase and bottle profiles, handles, arches, cable and hose paths.",
            examples: &[OpExample {
                args: r#"{"name": "arch", "points": [[-0.5, 0, 0], [0, 0.5, 0], [0.5, 0, 0]]}"#,
                result: "created arch (o:4) curve: 1 open bezier spline, 3 points, 1.4643 m long, 1 x 0.5 x 0 m",
            }],
            aliases: &["bezier", "curve", "spline", "smooth curve", "nurbs curve"],
            complexity: "O(points)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &BezierParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_points("/points", p.points.len(), p.closed)?;
        if let Some(h) = &p.handles
            && h.len() != p.points.len()
        {
            return Err(OpError::invalid("/handles", format!("{} handle pairs for {} points", h.len(), p.points.len()))
                .with_hint("give one [in, out] pair per point, or omit handles for a smooth curve"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &BezierParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let pts = points(&p.points);
        let handles: Vec<[Vec3; 2]> = match &p.handles {
            Some(h) => h.iter().map(|[a, b]| [vec3(a), vec3(b)]).collect(),
            None => kernel::auto_handles(&pts, p.closed),
        };
        let spline = Spline {
            points: pts.iter().map(|v| v.to_array()).collect(),
            handles: Some(handles.iter().map(|[a, b]| [a.to_array(), b.to_array()]).collect()),
            closed: p.closed,
        };
        create(tx, p.name.as_deref(), "bezier", p.parent.as_deref(), &p.position, Curve::single(spline))
    }
}

// ---- curve.circle ----------------------------------------------------------------------

/// Parameters of `curve.circle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CircleParams {
    /// Object name; a free name such as `circle_1` is picked when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Circle radius.
    pub radius: Length,
    /// Points around the circle.
    pub segments: u32,
    /// The plane it lies in.
    pub plane: PlaneParam,
    /// Center position in the parent's space.
    pub position: [Length; 3],
}

impl Default for CircleParams {
    fn default() -> Self {
        CircleParams { name: None, parent: None, radius: Length(0.5), segments: 32, plane: PlaneParam::Xz, position: zero3() }
    }
}

/// `curve.circle`.
pub struct CircleOp;

impl Op for CircleOp {
    const ID: &'static str = "curve.circle";
    type Params = CircleParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Circle curve",
            category: Category::Curve,
            summary: "Creates a closed circular curve: a regular polygon of the given number of points in a plane.",
            docs: "The points start on the plane's first axis (+X for `xz` and `xy`, +Y for `yz`) and run counter-clockwise \
                   seen from the plane's normal. Generators use the points as they are, so `segments` is the vertex count \
                   around a swept or extruded profile.\n\n\
                   **Use for** pipe and cable cross sections, round extrusions with holes, rings.",
            examples: &[OpExample {
                args: r#"{"name": "rim", "radius": 0.3, "segments": 16}"#,
                result: "created rim (o:4) curve: 1 closed spline, 16 points, 1.8729 m long, 0.6 x 0 x 0.6 m",
            }],
            aliases: &["circle", "ring curve", "bezier circle", "nurbs circle"],
            complexity: "O(segments)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CircleParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_length("/radius", p.radius, 1e-5, 1e5)?;
        check_count("/segments", p.segments, 3, 4096)
    }

    fn apply(tx: &mut Tx, p: &CircleParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let pts = kernel::circle(p.radius.0 as f32, p.segments, p.plane.into());
        create(tx, p.name.as_deref(), "circle", p.parent.as_deref(), &p.position, Curve::single(Spline::polyline(pts, true)))
    }
}

// ---- curve.rectangle ---------------------------------------------------------------------

/// Parameters of `curve.rectangle`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct RectangleParams {
    /// Object name; a free name such as `rectangle_1` is picked when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Size along the plane's two axes (X and Z for `xz`).
    pub size: [Length; 2],
    /// Rounds the corners; sharp when 0.
    pub corner_radius: Length,
    /// Segments per rounded corner.
    pub corner_segments: u32,
    /// The plane it lies in.
    pub plane: PlaneParam,
    /// Center position in the parent's space.
    pub position: [Length; 3],
}

impl Default for RectangleParams {
    fn default() -> Self {
        RectangleParams {
            name: None,
            parent: None,
            size: [Length(1.0), Length(1.0)],
            corner_radius: Length(0.0),
            corner_segments: 4,
            plane: PlaneParam::Xz,
            position: zero3(),
        }
    }
}

/// `curve.rectangle`.
pub struct RectangleOp;

impl Op for RectangleOp {
    const ID: &'static str = "curve.rectangle";
    type Params = RectangleParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Rectangle curve",
            category: Category::Curve,
            summary: "Creates a closed rectangular curve centered on its origin, optionally with rounded corners.",
            docs: "Corners run counter-clockwise seen from the plane's normal, starting at the (-, -) corner. A positive \
                   `corner_radius` replaces each corner with a tangent arc of `corner_segments` segments.\n\n\
                   **Use for** panel and frame outlines (`gen.extrude_shape`), rounded slot and window profiles, sweep \
                   cross sections for rectangular trims.",
            examples: &[OpExample {
                args: r#"{"name": "slot", "size": [0.4, 0.1], "corner_radius": "2cm", "corner_segments": 3}"#,
                result: "created slot (o:4) curve: 1 closed spline, 16 points, 0.9642 m long, 0.4 x 0 x 0.1 m",
            }],
            aliases: &["rectangle", "rounded rectangle", "square curve", "rect"],
            complexity: "O(corner_segments)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &RectangleParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        check_length("/size/0", p.size[0], 1e-5, 1e5)?;
        check_length("/size/1", p.size[1], 1e-5, 1e5)?;
        let max = p.size[0].0.min(p.size[1].0) * 0.5;
        check_length("/corner_radius", p.corner_radius, 0.0, max)?;
        check_count("/corner_segments", p.corner_segments, 1, MAX_FILLET_SEGMENTS)
    }

    fn apply(tx: &mut Tx, p: &RectangleParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let size = [p.size[0].0 as f32, p.size[1].0 as f32];
        let pts = kernel::rectangle(size, p.corner_radius.0 as f32, p.corner_segments, p.plane.into()).map_err(kernel_error)?;
        create(tx, p.name.as_deref(), "rectangle", p.parent.as_deref(), &p.position, Curve::single(Spline::polyline(pts, true)))
    }
}

// ---- curve.resample ----------------------------------------------------------------------

/// Parameters of `curve.resample`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResampleParams {
    /// Curve object (name, path or id).
    pub target: String,
    /// Points per spline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Distance between points instead of a count (rounded so the spacing is even).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<Length>,
    /// Segments per curved Bezier span before resampling.
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    /// Name of a new curve object for the result; edits the target in place when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl Default for ResampleParams {
    fn default() -> Self {
        ResampleParams { target: String::new(), count: Some(16), spacing: None, resolution: DEFAULT_RESOLUTION, output: None }
    }
}

/// `curve.resample`.
pub struct ResampleOp;

impl Op for ResampleOp {
    const ID: &'static str = "curve.resample";
    type Params = ResampleParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Resample curve",
            category: Category::Curve,
            summary: "Replaces each spline with a polyline of points evenly spaced by arc length.",
            docs: "Give `count` (points per spline) or `spacing` (a length, rounded to divide each spline evenly). Open \
                   splines keep both ends; closed ones keep their first point. Bezier splines are sampled first. Corners \
                   between samples are cut, so resample smooth curves rather than sharp outlines.\n\n\
                   **Use for** matching point counts before `gen.loft`, even edge lengths along sweeps and pipes.",
            examples: &[OpExample {
                args: r#"{"target": "path", "spacing": "25cm"}"#,
                result: "path (o:2): resampled: 1 open spline, 9 points, 2 m long, 1 x 0 x 1 m",
            }],
            aliases: &["resample", "even out", "respace points", "equalize"],
            complexity: "O(points x resolution + result points)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ResampleParams) -> Result<(), OpError> {
        let key = curve_target(doc, &p.target, "/target")?;
        check_resolution(p.resolution)?;
        check_output(doc, key, p.output.as_deref())?;
        match (p.count, p.spacing) {
            (Some(n), None) => check_count("/count", n, 2, MAX_CURVE_POINTS as u32),
            (None, Some(s)) => check_length("/spacing", s, 1e-5, 1e5),
            _ => Err(OpError::invalid("", "give exactly one of count and spacing").with_hint("for example {\"spacing\": \"5cm\"}")),
        }
    }

    fn apply(tx: &mut Tx, p: &ResampleParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = curve_target(tx.doc(), &p.target, "/target")?;
        let src = polylines(tx.doc().object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?, p.resolution)?;
        let mut out = Vec::with_capacity(src.len());
        for (pts, closed) in src {
            let n = match (p.count, p.spacing) {
                (Some(n), _) => n as usize,
                (None, Some(s)) => {
                    let segs = (f64::from(kernel::length(&pts, closed)) / s.0).round().max(1.0) as usize;
                    let n = if closed { segs.max(3) } else { segs + 1 };
                    if n > MAX_CURVE_POINTS {
                        return Err(OpError::out_of_range("/spacing", format!("{n} points exceed the limit of {MAX_CURVE_POINTS}")));
                    }
                    n
                }
                (None, None) => return Err(OpError::invalid("", "give exactly one of count and spacing")),
            };
            out.push((kernel::resample(&pts, closed, n).map_err(kernel_error)?, closed));
        }
        store(tx, key, polyline_curve(out), p.output.as_deref(), "resampled")
    }
}

// ---- curve.offset ----------------------------------------------------------------------

/// Parameters of `curve.offset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OffsetParams {
    /// Curve object (name, path or id).
    pub target: String,
    /// Outward for closed splines, to the left (seen from the normal) for
    /// open ones; negative goes the other way.
    pub distance: Length,
    /// Offsets within planes perpendicular to this direction; the curve's
    /// own plane when omitted (+Y for straight lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    /// Convex corners whose miter would exceed this many distances are beveled.
    #[serde(default = "default_miter_limit")]
    pub miter_limit: f64,
    /// Segments per curved Bezier span before offsetting.
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    /// Name of a new curve object for the result; edits the target in place when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

fn default_miter_limit() -> f64 {
    4.0
}

impl Default for OffsetParams {
    fn default() -> Self {
        OffsetParams {
            target: String::new(),
            distance: Length(0.01),
            normal: None,
            miter_limit: default_miter_limit(),
            resolution: DEFAULT_RESOLUTION,
            output: None,
        }
    }
}

/// `curve.offset`.
pub struct OffsetOp;

impl Op for OffsetOp {
    const ID: &'static str = "curve.offset";
    type Params = OffsetParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Offset curve",
            category: Category::Curve,
            summary: "Moves each spline sideways by a distance within its plane, with mitered or beveled corners.",
            docs: "Closed splines grow outward for a positive `distance` (shrink for a negative one) whatever their winding. \
                   Open splines move to their left seen from the normal's tip. The plane is the curve's own unless \
                   `normal` is given; a non-planar curve needs it. Concave corners deeper than the offset fold over \
                   themselves; keep inner offsets smaller than the narrowest part. The result is a polyline.\n\n\
                   **Use for** wall thickness from a floor plan, inner and outer outlines of frames and gaskets, parallel rails.",
            examples: &[OpExample {
                args: r#"{"target": "outline", "distance": "5cm", "output": "outline_outer"}"#,
                result: "created outline_outer (o:4) curve: 1 closed spline, 4 points, 3.2 m long, 0.9 x 0 x 0.7 m (offset by 50 mm from outline (o:3))",
            }],
            aliases: &["offset", "parallel curve", "inset curve", "outline", "offset path"],
            complexity: "O(points x resolution)",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &OffsetParams) -> Result<(), OpError> {
        let key = curve_target(doc, &p.target, "/target")?;
        check_resolution(p.resolution)?;
        check_output(doc, key, p.output.as_deref())?;
        check_length("/distance", Length(p.distance.0.abs()), 0.0, 1e5)?;
        if !(p.miter_limit.is_finite() && (1.0..=100.0).contains(&p.miter_limit)) {
            return Err(OpError::out_of_range("/miter_limit", format!("{} is outside 1 to 100", p.miter_limit)));
        }
        if let Some(n) = p.normal
            && !(n.iter().all(|x| x.is_finite()) && n.iter().any(|&x| x != 0.0))
        {
            return Err(OpError::invalid("/normal", "normal must be a nonzero vector").with_hint("for example [0, 1, 0]"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &OffsetParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = curve_target(tx.doc(), &p.target, "/target")?;
        let src = polylines(tx.doc().object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?, p.resolution)?;
        let normal = p.normal.map(|n| Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32));
        let out = src
            .into_iter()
            .map(|(pts, closed)| kernel::offset(&pts, closed, p.distance.0 as f32, normal, p.miter_limit as f32).map(|o| (o, closed)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(kernel_error)?;
        store(tx, key, polyline_curve(out), p.output.as_deref(), &format!("offset by {}", format_length(p.distance.0)))
    }
}

// ---- curve.fillet ----------------------------------------------------------------------

/// Parameters of `curve.fillet`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FilletParams {
    /// Curve object (name, path or id).
    pub target: String,
    /// Arc radius at each corner (shrunk where the segments are too short).
    pub radius: Length,
    /// Segments per arc; 1 cuts a chamfer.
    #[serde(default = "four")]
    pub segments: u32,
    /// Segments per curved Bezier span before filleting.
    #[serde(default = "default_resolution")]
    pub resolution: u32,
    /// Name of a new curve object for the result; edits the target in place when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

fn four() -> u32 {
    4
}

impl Default for FilletParams {
    fn default() -> Self {
        FilletParams { target: String::new(), radius: Length(0.01), segments: 4, resolution: DEFAULT_RESOLUTION, output: None }
    }
}

/// `curve.fillet`.
pub struct FilletOp;

impl Op for FilletOp {
    const ID: &'static str = "curve.fillet";
    type Params = FilletParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Fillet curve",
            category: Category::Curve,
            summary: "Rounds every corner of each spline with a tangent arc, or chamfers it with one segment.",
            docs: "Each corner becomes an arc of `radius` tangent to both segments; the radius shrinks where a segment is \
                   shorter than both fillets need (half a segment per corner, the whole end segment of an open spline). \
                   Straight joints and reversals stay as they are. The result is a polyline.\n\n\
                   **Use for** rounding profile steps before `gen.lathe`, soft outlines for `gen.extrude_shape`, cable paths \
                   with bend radii before `gen.pipe`.",
            examples: &[OpExample {
                args: r#"{"target": "path", "radius": "20cm", "segments": 4}"#,
                result: "path (o:2): filleted: 1 open spline, 7 points, 1.9121 m long, 1 x 0 x 1 m",
            }],
            aliases: &["fillet", "round corners", "chamfer", "corner radius"],
            complexity: "O(points x (resolution + segments))",
            flags: CREATE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FilletParams) -> Result<(), OpError> {
        let key = curve_target(doc, &p.target, "/target")?;
        check_resolution(p.resolution)?;
        check_output(doc, key, p.output.as_deref())?;
        check_length("/radius", p.radius, 1e-6, 1e5)?;
        check_count("/segments", p.segments, 1, MAX_FILLET_SEGMENTS)
    }

    fn apply(tx: &mut Tx, p: &FilletParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = curve_target(tx.doc(), &p.target, "/target")?;
        let src = polylines(tx.doc().object_curve(key).ok_or_else(|| OpError::internal("curve vanished"))?, p.resolution)?;
        let out = src
            .into_iter()
            .map(|(pts, closed)| kernel::fillet(&pts, closed, p.radius.0 as f32, p.segments).map(|o| (o, closed)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(kernel_error)?;
        store(tx, key, polyline_curve(out), p.output.as_deref(), "filleted")
    }
}
