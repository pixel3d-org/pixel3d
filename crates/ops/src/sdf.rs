//! `gen.sdf`: organic shapes from a list of named primitives blended in
//! order (kernel in `p3d_sdf`). This is the agent's sculpting tool: every
//! shape is a few numbers, every join a word, and every face remembers the
//! shape it came from.

use std::borrow::Cow;

use glam::{EulerRot, Quat, Vec3};
use p3d_mesh::ElementKind;
use p3d_scene::Document;
use p3d_sdf::{Combine, Item, MeshSpec, Modifiers, Noise, Prim, Sdf, SdfError, Shape, mesh};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::common::{check_length, check_new_object, dims, label, mesh_groups, mesh_target, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::{Angle, Length};
use crate::prim::created_summary;
use crate::tx::Tx;

/// Smallest and largest voxel size.
const MIN_DETAIL: f64 = 1e-4;
const MAX_DETAIL: f64 = 0.5;

/// A sphere around `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpherePrim {
    pub at: [Length; 3],
    pub radius: Length,
}

/// An ellipsoid around `at` with semi-axes along x, y, z.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EllipsoidPrim {
    pub at: [Length; 3],
    pub radii: [Length; 3],
}

/// A box around `at`, optionally with rounded corners.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxPrim {
    pub at: [Length; 3],
    /// Full size along x, y, z.
    pub size: [Length; 3],
    /// Corner and edge rounding radius (inside the size).
    #[serde(default)]
    pub corner: Length,
}

/// A segment from `a` to `b` swept by a sphere: a capsule, or a tapered one
/// (horns, fingers, limbs) when `radius_b` differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapsulePrim {
    pub a: [Length; 3],
    pub b: [Length; 3],
    /// Radius at `a` (and at `b` unless `radius_b` is given).
    pub radius: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_b: Option<Length>,
}

/// A cone or frustum standing on its base center `at`, along +Y.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConePrim {
    pub at: [Length; 3],
    pub height: Length,
    pub radius_bottom: Length,
    #[serde(default)]
    pub radius_top: Length,
}

/// A ring around Y centered at `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TorusPrim {
    pub at: [Length; 3],
    /// Distance from the center to the middle of the ring.
    pub major: Length,
    /// Radius of the ring's cross-section.
    pub minor: Length,
}

/// A capped cylinder along Y centered at `at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CylinderPrim {
    pub at: [Length; 3],
    pub height: Length,
    pub radius: Length,
}

/// The primitive of a shape (exactly one per shape).
#[derive(Debug, Clone, PartialEq)]
pub enum SdfPrimParam {
    Sphere(SpherePrim),
    Ellipsoid(EllipsoidPrim),
    Box(BoxPrim),
    Capsule(CapsulePrim),
    Cone(ConePrim),
    Torus(TorusPrim),
    Cylinder(CylinderPrim),
    /// Shapes combined among themselves first, then joined as one.
    Group(Vec<SdfShapeParam>),
}

const PRIM_KEYS: [&str; 8] = ["sphere", "ellipsoid", "box", "capsule", "cone", "torus", "cylinder", "group"];

/// Axes to mirror across: `"x"` or a list such as `["x", "z"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MirrorParam {
    One(AxisName),
    Many(Vec<AxisName>),
}

/// A part axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AxisName {
    X,
    Y,
    Z,
}

/// Seeded bumps on the surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoiseParam {
    /// Largest bump height.
    pub amplitude: Length,
    /// Size of the bumps.
    pub scale: Length,
    /// Same seed, same bumps; the call's seed when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Layers of finer bumps (1 to 6).
    #[serde(default = "two")]
    pub octaves: u32,
}

fn two() -> u32 {
    2
}

/// Everything about a shape besides its primitive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SdfShapeOptions {
    /// Name of the shape: its faces form a group of this name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Join with a fillet of this size instead of a hard union.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_union: Option<Length>,
    /// Cut this shape out of the shapes before it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub subtract: bool,
    /// Cut with a rounded fillet of this size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_subtract: Option<Length>,
    /// Keep only what this shape and the shapes before it share.
    #[serde(default, skip_serializing_if = "is_false")]
    pub intersect: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth_intersect: Option<Length>,
    /// Euler angles in degrees about X, then Y, then Z, around the shape's own center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<[Angle; 3]>,
    /// Grow the shape by this much with rounded edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<Length>,
    /// Hollow the shape into a skin this thick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<Length>,
    /// Turn around the shape's Y axis, degrees per meter of height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twist: Option<Angle>,
    /// Bend the shape's Y axis toward +X, degrees per meter (negative bends toward -X).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bend: Option<Angle>,
    /// Mirror across the part's planes: the positive side is kept and reflected (write the left ear, get both).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<MirrorParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<NoiseParam>,
    /// Groups only: the group's center for `rotate` (the part's origin when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[Length; 3]>,
    /// Leave the shape out of the result; it stays in the list, and the other shapes keep their group names.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One shape of an SDF: one primitive key plus how it joins the shapes
/// before it and how it is changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SdfShapeParam {
    pub prim: SdfPrimParam,
    pub options: SdfShapeOptions,
}

impl<'de> Deserialize<'de> for SdfShapeParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let Value::Object(mut map) = v else {
            return Err(de::Error::custom("a shape is an object such as {\"sphere\": {\"at\": [0, 0, 0], \"radius\": 0.1}}"));
        };
        let found: Vec<&str> = PRIM_KEYS.iter().copied().filter(|k| map.contains_key(*k)).collect();
        let key = match found.as_slice() {
            [k] => *k,
            [] => {
                return Err(de::Error::custom(format!(
                    "a shape needs one of {} (keys given: {})",
                    PRIM_KEYS.join(", "),
                    map.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
            many => return Err(de::Error::custom(format!("a shape has one primitive, not {}", many.join(" and ")))),
        };
        let body = map.remove(key).unwrap_or(Value::Null);
        let field = |e: serde_json::Error| de::Error::custom(format!("{key}: {e}"));
        let prim = match key {
            "sphere" => SdfPrimParam::Sphere(serde_json::from_value(body).map_err(field)?),
            "ellipsoid" => SdfPrimParam::Ellipsoid(serde_json::from_value(body).map_err(field)?),
            "box" => SdfPrimParam::Box(serde_json::from_value(body).map_err(field)?),
            "capsule" => SdfPrimParam::Capsule(serde_json::from_value(body).map_err(field)?),
            "cone" => SdfPrimParam::Cone(serde_json::from_value(body).map_err(field)?),
            "torus" => SdfPrimParam::Torus(serde_json::from_value(body).map_err(field)?),
            "cylinder" => SdfPrimParam::Cylinder(serde_json::from_value(body).map_err(field)?),
            _ => SdfPrimParam::Group(serde_json::from_value(body).map_err(field)?),
        };
        let options: SdfShapeOptions = serde_json::from_value(Value::Object(map)).map_err(de::Error::custom)?;
        Ok(SdfShapeParam { prim, options })
    }
}

impl Serialize for SdfShapeParam {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = match serde_json::to_value(&self.options) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let (key, body) = match &self.prim {
            SdfPrimParam::Sphere(p) => ("sphere", serde_json::to_value(p)),
            SdfPrimParam::Ellipsoid(p) => ("ellipsoid", serde_json::to_value(p)),
            SdfPrimParam::Box(p) => ("box", serde_json::to_value(p)),
            SdfPrimParam::Capsule(p) => ("capsule", serde_json::to_value(p)),
            SdfPrimParam::Cone(p) => ("cone", serde_json::to_value(p)),
            SdfPrimParam::Torus(p) => ("torus", serde_json::to_value(p)),
            SdfPrimParam::Cylinder(p) => ("cylinder", serde_json::to_value(p)),
            SdfPrimParam::Group(g) => ("group", serde_json::to_value(g)),
        };
        map.insert(key.to_owned(), body.map_err(serde::ser::Error::custom)?);
        Value::Object(map).serialize(s)
    }
}

impl JsonSchema for SdfShapeParam {
    fn schema_name() -> Cow<'static, str> {
        "SdfShape".into()
    }

    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let length = g.subschema_for::<Length>();
        let angle = g.subschema_for::<Angle>();
        let vec3 = json_schema!({ "type": "array", "items": length, "minItems": 3, "maxItems": 3 });
        let angles = json_schema!({ "type": "array", "items": angle, "minItems": 3, "maxItems": 3 });
        let shape = g.subschema_for::<SdfShapeParam>();
        let props = serde_json::json!({
            "sphere": g.subschema_for::<SpherePrim>(),
            "ellipsoid": g.subschema_for::<EllipsoidPrim>(),
            "box": g.subschema_for::<BoxPrim>(),
            "capsule": g.subschema_for::<CapsulePrim>(),
            "cone": g.subschema_for::<ConePrim>(),
            "torus": g.subschema_for::<TorusPrim>(),
            "cylinder": g.subschema_for::<CylinderPrim>(),
            "group": { "type": "array", "items": shape, "minItems": 1, "description": "Shapes combined among themselves first, then joined as one." },
            "name": { "type": "string", "description": "Name of the shape: its faces form a group of this name." },
            "smooth_union": { "description": "Join with a fillet of this size instead of a hard union.", "allOf": [length] },
            "subtract": { "type": "boolean", "description": "Cut this shape out of the shapes before it." },
            "smooth_subtract": { "description": "Cut with a rounded fillet of this size.", "allOf": [length] },
            "intersect": { "type": "boolean", "description": "Keep only what this shape and the shapes before it share." },
            "smooth_intersect": { "allOf": [length] },
            "rotate": { "description": "Euler angles in degrees about X, then Y, then Z, around the shape's own center.", "allOf": [angles] },
            "round": { "description": "Grow the shape by this much with rounded edges.", "allOf": [length] },
            "shell": { "description": "Hollow the shape into a skin this thick.", "allOf": [length] },
            "twist": { "description": "Turn around the shape's Y axis, degrees per meter of height.", "allOf": [angle] },
            "bend": { "description": "Bend the shape's Y axis toward +X, degrees per meter (negative toward -X).", "allOf": [angle] },
            "mirror": g.subschema_for::<MirrorParam>(),
            "noise": g.subschema_for::<NoiseParam>(),
            "at": { "description": "Groups only: the group's center for rotate.", "allOf": [vec3] },
            "hidden": { "type": "boolean", "description": "Leave the shape out of the result; it stays in the list." }
        });
        json_schema!({
            "description": "One shape: exactly one primitive key (sphere, ellipsoid, box, capsule, cone, torus, cylinder or group) plus how it joins the shapes before it (union by default; smooth_union, subtract, smooth_subtract, intersect, smooth_intersect) and optional changes (rotate, round, shell, twist, bend, mirror, noise, name, hidden).",
            "type": "object",
            "properties": props,
            "additionalProperties": false,
            "minProperties": 1
        })
    }
}

/// Parameters of `gen.sdf`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SdfParams {
    /// Object name; a free name such as `sdf_1` is picked when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Origin position in the parent's space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
    /// The shapes in order: each joins the shapes before it.
    pub shapes: Vec<SdfShapeParam>,
    /// Voxel size, about the edge length of the result: smaller is finer. The default fits about 64 voxels across the largest side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Length>,
    /// Mesh object to rebuild in place from the shapes (it keeps its name, transform, parent and children) instead of making a new object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Where a kernel item came from: its index among the shapes as written
/// (hidden shapes left out of the tree shift the kernel's numbering), and
/// the same for a group's members.
#[derive(Debug, Default)]
struct Origin {
    index: usize,
    inner: Vec<Origin>,
}

/// The parameter path of a kernel item path such as `1.0`.
///
/// Complexity: O(depth).
fn param_path(origins: &[Origin], kernel: &str) -> String {
    let mut out = String::from("/shapes");
    let mut level = origins;
    for (depth, part) in kernel.split('.').enumerate() {
        let Ok(i) = part.parse::<usize>() else { break };
        let o = level.get(i);
        if depth > 0 {
            out.push_str("/group");
        }
        out.push_str(&format!("/{}", o.map_or(i, |o| o.index)));
        level = o.map_or(&[], |o| o.inner.as_slice());
    }
    out
}

/// Prims in a shape, hidden ones included (for stable group numbers).
///
/// Complexity: O(shapes in it).
fn leaf_count(s: &SdfShapeParam) -> usize {
    match &s.prim {
        SdfPrimParam::Group(g) => g.iter().map(leaf_count).sum(),
        _ => 1,
    }
}

fn v3(p: &[Length; 3]) -> Vec3 {
    Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32)
}

fn f(l: Length) -> f32 {
    l.0 as f32
}

/// The kernel tree of the parameters without the hidden shapes, and where
/// each item came from. Unnamed prims are named `item_<n>` by their place
/// among all prims, hidden ones included, so hiding a shape never renames
/// another shape's group.
///
/// Complexity: O(shapes).
fn tree(shapes: &[SdfShapeParam], seed: u64) -> Result<(Vec<Item>, Vec<Origin>), OpError> {
    let mut leaf = 0;
    let mut origins = Vec::new();
    let items = items(shapes, seed, "/shapes", &mut leaf, &mut origins)?;
    Ok((items, origins))
}

fn items(shapes: &[SdfShapeParam], seed: u64, path: &str, leaf: &mut usize, origins: &mut Vec<Origin>) -> Result<Vec<Item>, OpError> {
    let mut out = Vec::with_capacity(shapes.len());
    for (i, s) in shapes.iter().enumerate() {
        if s.options.hidden {
            *leaf += leaf_count(s);
            continue;
        }
        let here = format!("{path}/{i}");
        let mut origin = Origin { index: i, inner: Vec::new() };
        let o = &s.options;
        let joins = [o.smooth_union.is_some(), o.subtract, o.smooth_subtract.is_some(), o.intersect, o.smooth_intersect.is_some()];
        if joins.iter().filter(|j| **j).count() > 1 {
            return Err(OpError::invalid(
                &here,
                "a shape joins one way: union (default), smooth_union, subtract, smooth_subtract, intersect or smooth_intersect",
            ));
        }
        let combine = if let Some(k) = o.smooth_union {
            Combine::SmoothUnion(f(k))
        } else if o.subtract {
            Combine::Subtract
        } else if let Some(k) = o.smooth_subtract {
            Combine::SmoothSubtract(f(k))
        } else if o.intersect {
            Combine::Intersect
        } else if let Some(k) = o.smooth_intersect {
            Combine::SmoothIntersect(f(k))
        } else {
            Combine::Union
        };
        let mut name = o.name.clone();
        if !matches!(s.prim, SdfPrimParam::Group(_)) {
            name = name.or_else(|| Some(format!("item_{leaf}")));
            *leaf += 1;
        }
        let (shape, center) = match &s.prim {
            SdfPrimParam::Sphere(p) => (Shape::Prim(Prim::Sphere { radius: f(p.radius) }), v3(&p.at)),
            SdfPrimParam::Ellipsoid(p) => (Shape::Prim(Prim::Ellipsoid { radii: v3(&p.radii) }), v3(&p.at)),
            SdfPrimParam::Box(p) => (Shape::Prim(Prim::Box { half: v3(&p.size) * 0.5, round: f(p.corner) }), v3(&p.at)),
            SdfPrimParam::Capsule(p) => {
                let (a, b) = (v3(&p.a), v3(&p.b));
                let c = (a + b) * 0.5;
                let r = f(p.radius);
                (Shape::Prim(Prim::Capsule { a: a - c, b: b - c, ra: r, rb: p.radius_b.map_or(r, f) }), c)
            }
            SdfPrimParam::Cone(p) => (Shape::Prim(Prim::Cone { height: f(p.height), bottom: f(p.radius_bottom), top: f(p.radius_top) }), v3(&p.at)),
            SdfPrimParam::Torus(p) => (Shape::Prim(Prim::Torus { major: f(p.major), minor: f(p.minor) }), v3(&p.at)),
            SdfPrimParam::Cylinder(p) => (Shape::Prim(Prim::Cylinder { height: f(p.height), radius: f(p.radius) }), v3(&p.at)),
            SdfPrimParam::Group(g) => {
                let c = o.at.as_ref().map_or(Vec3::ZERO, v3);
                // Members are written in the part's space; the group's frame sits at its center.
                let mut inner = items(g, seed, &format!("{here}/group"), leaf, &mut origin.inner)?;
                for it in &mut inner {
                    it.center -= c;
                }
                (Shape::Group(inner), c)
            }
        };
        if o.at.is_some() && !matches!(s.prim, SdfPrimParam::Group(_)) {
            return Err(OpError::invalid(&format!("{here}/at"), "`at` outside a primitive is for groups; put it inside the primitive"));
        }
        let rotation = o.rotate.map_or(Quat::IDENTITY, |r| Quat::from_euler(EulerRot::XYZ, r[0].0 as f32, r[1].0 as f32, r[2].0 as f32));
        let mut mirror = [false; 3];
        let axes = match &o.mirror {
            None => vec![],
            Some(MirrorParam::One(a)) => vec![*a],
            Some(MirrorParam::Many(v)) => v.clone(),
        };
        for a in axes {
            mirror[a as usize] = true;
        }
        let noise = match &o.noise {
            Some(n) => {
                if !(1..=6).contains(&n.octaves) {
                    return Err(OpError::out_of_range(&format!("{here}/noise/octaves"), "noise octaves go from 1 to 6"));
                }
                Some(Noise { amplitude: f(n.amplitude), scale: f(n.scale), seed: n.seed.unwrap_or(seed.wrapping_add(i as u64)), octaves: n.octaves })
            }
            None => None,
        };
        let modifiers = Modifiers {
            round: o.round.map_or(0.0, f),
            shell: o.shell.map_or(0.0, f),
            twist: o.twist.map_or(0.0, |a| a.0 as f32),
            bend: o.bend.map_or(0.0, |a| a.0 as f32),
            mirror,
            noise,
        };
        origins.push(origin);
        out.push(Item { shape, center, rotation, combine, modifiers, name });
    }
    Ok(out)
}

fn sdf_error(e: SdfError, origins: &[Origin]) -> OpError {
    match e {
        SdfError::Empty => OpError::new(ErrorCode::Topology, "the shapes leave nothing: every part is cut away, outside an intersection or hidden")
            .at("/shapes")
            .with_hint("check subtract and intersect: a list that starts with a cut starts from nothing"),
        SdfError::Invalid { path, message } => OpError::invalid(&param_path(origins, &path), message),
        SdfError::TooManyItems => OpError::new(ErrorCode::BudgetExceeded, e.to_string()).at("/shapes"),
        SdfError::TooDeep => OpError::new(ErrorCode::InvalidParam, e.to_string()).at("/shapes"),
        SdfError::TooLarge(n) => OpError::new(ErrorCode::BudgetExceeded, format!("{} voxels is more than one mesh takes", thousands(n as usize)))
            .at("/detail")
            .with_hint("use a larger detail (voxel size)"),
    }
}

/// `gen.sdf`.
pub struct GenSdf;

impl Op for GenSdf {
    const ID: &'static str = "gen.sdf";
    type Params = SdfParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "SDF shape",
            category: Category::Gen,
            summary: "Builds an organic shape from named primitives blended in order (smooth unions, cuts) into a closed quad mesh.",
            docs: "The sculpting tool for agents: write the shapes in order, each one primitive (`sphere`, `ellipsoid`, `box` with \
                   `corner` rounding, `capsule` with an optional `radius_b` taper, `cone`, `torus`, `cylinder`, or a `group` of shapes) \
                   plus how it joins the shapes before it: union (default), `smooth_union` with a fillet size, `subtract`, \
                   `smooth_subtract`, `intersect` or `smooth_intersect`. Each shape can be turned (`rotate`, degrees), grown \
                   (`round`), hollowed (`shell`), twisted or bent (degrees per meter), given seeded bumps (`noise`), and mirrored \
                   across the part's planes (`mirror: \"x\"` keeps the +x side and reflects it: write the left ear, get both).\n\n\
                   Every face joins a group named after the shape that made it (`name`, or `shape_<n>`), so perception and \
                   diagnostics can name the shapes. `detail` is the voxel size, about the edge length of the result; walls thinner \
                   than a voxel make the mesher refine on its own.\n\n\
                   A shape with `hidden: true` is left out but stays in the list (the others keep their group names). With \
                   `target`, the shapes rebuild that mesh object in place (it keeps its name, transform, parent and children): \
                   the object's history then holds the shapes to edit and run again.\n\n\
                   **Use for** heads, bodies, creatures, rocks, organic props and stylized forms: blocking out with ellipsoids and \
                   capsules, then blending detail in. Hard-surface parts are better built with primitives, extrusions and bevels.",
            examples: &[OpExample {
                args: r#"{"name": "blob", "shapes": [{"sphere": {"at": [0, 0.1, 0], "radius": 0.1}, "name": "body"}, {"sphere": {"at": [0, 0.21, 0], "radius": 0.06}, "name": "head", "smooth_union": 0.03}], "detail": "1cm"}"#,
                result: "created blob (o:4) sdf 0.1997 x 0.2697 x 0.1997 m: 2,182 faces (4,364 tris; budget 6,000)",
            }],
            aliases: &["sdf", "metaballs", "sculpt", "blob", "implicit surface", "csg", "smooth union", "volume modeling", "clay"],
            complexity: "O(v x shapes) for the v voxels near the surface (about area / detail^2 x 8), parallel",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SdfParams) -> Result<(), OpError> {
        match &p.target {
            Some(t) => {
                if p.name.is_some() || p.parent.is_some() || p.position.is_some() {
                    return Err(OpError::invalid("/target", "name, parent and position are for a new object; leave them out to rebuild the target"));
                }
                mesh_target(doc, t)?;
            }
            None => {
                check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
            }
        }
        if p.shapes.is_empty() {
            return Err(OpError::invalid("/shapes", "give at least one shape").with_hint("{\"sphere\": {\"at\": [0, 0, 0], \"radius\": 0.1}}"));
        }
        if let Some(d) = p.detail {
            check_length("/detail", d, MIN_DETAIL, MAX_DETAIL)?;
        }
        let (items, origins) = tree(&p.shapes, 0)?;
        Sdf::new(items).map(|_| ()).map_err(|e| sdf_error(e, &origins))
    }

    fn apply(tx: &mut Tx, p: &SdfParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let (items, origins) = tree(&p.shapes, ctx.seed)?;
        let sdf = Sdf::new(items).map_err(|e| sdf_error(e, &origins))?;
        let (lo, hi) = sdf.bounds();
        let voxel = p.detail.map_or_else(|| ((hi - lo).max_element() / 64.0).clamp(MIN_DETAIL as f32, MAX_DETAIL as f32), f);
        ctx.check_cancel()?;
        let r = mesh(&sdf, &MeshSpec { voxel }).map_err(|e| sdf_error(e, &origins))?;
        ctx.check_cancel()?;
        let mut m = r.mesh;
        let leaves = sdf.leaves();
        let mut by_leaf: Vec<Vec<u32>> = vec![Vec::new(); leaves.len()];
        let faces: Vec<u32> = m.faces().collect();
        for (&face, &leaf) in faces.iter().zip(&r.face_leaf) {
            if let Some(list) = by_leaf.get_mut(leaf as usize) {
                list.push(face);
            }
        }
        for (name, list) in leaves.iter().zip(by_leaf) {
            let group = name.strip_prefix("item_").map_or_else(|| name.clone(), |n| format!("shape_{n}"));
            if !list.is_empty() {
                m.set_group(&group, ElementKind::Face, list).map_err(|e| OpError::internal(e.to_string()))?;
            }
        }
        let mut out = match &p.target {
            Some(t) => {
                // Rebuild in place: the object keeps its name, transform,
                // parent and children; only its mesh is new.
                let key = mesh_target(tx.doc(), t)?;
                *tx.mesh_mut(key, "/target")? = m;
                let doc = tx.doc();
                let (dims, faces, tris) = doc.object_mesh(key).map_or_else(|| ("?".into(), 0, 0), |m| (dims(m), m.face_count(), m.triangle_count()));
                let mut out = OpOutput::summary(format!(
                    "rebuilt {} sdf {dims}: {} faces ({} tris; budget {})",
                    label(doc, key),
                    thousands(faces),
                    thousands(tris),
                    thousands(doc.settings.tri_budget() as usize)
                ));
                out.changed.push(key);
                out.groups = mesh_groups(doc, key);
                out
            }
            None => {
                let key = crate::common::create_mesh_object(
                    tx,
                    p.name.as_deref(),
                    "sdf",
                    p.parent.as_deref(),
                    p.position.as_ref().unwrap_or(&[Length(0.0); 3]),
                    m,
                )?;
                created_summary(tx, key, "sdf")
            }
        };
        if r.voxel < voxel * 0.99 {
            out.warnings.push(Warning {
                message: format!("walls thinner than the {} mm detail: meshed at {} mm instead (more faces)", fmt_mm(voxel), fmt_mm(r.voxel)),
                fix: None,
            });
        }
        if r.specks > 0 {
            out.warnings.push(Warning { message: format!("dropped {} bit(s) smaller than a voxel", r.specks), fix: None });
        }
        if !r.repair.is_clean() || r.unresolved > 0 {
            out.warnings.push(Warning {
                message: format!("{} spots could not be meshed cleanly; a smaller detail may fix them", r.repair.split_edges + r.unresolved),
                fix: None,
            });
        }
        Ok(out)
    }
}

fn fmt_mm(m: f32) -> String {
    let mm = f64::from(m) * 1000.0;
    if mm >= 10.0 { format!("{mm:.0}") } else { format!("{mm:.2}").trim_end_matches('0').trim_end_matches('.').to_owned() }
}
