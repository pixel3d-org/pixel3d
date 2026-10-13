//! The typed Model Spec (project spec 17.3, ADR 0010) and its parsing
//! from the JSON the author wrote, with errors at JSON pointer paths.

use std::collections::BTreeMap;

use p3d_ops::params::{Angle, Length};
use p3d_ops::sdf::SdfShapeParam;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SpecError;

/// A Model Spec: named parts and how they relate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    /// Name of the spec (informational: the root object's name is used).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<String>,
    /// The brief this spec answers, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// Asset category (for example `humanoid_head`): its landmarks and
    /// default expectations; the brief's when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Style knobs (preset, bevel, exaggeration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    /// Visual DNA genes (ADR 0011).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dna: Option<p3d_perceive::dna::DnaSpec>,
    /// Seed of every random choice in the spec (noise, scatter).
    #[serde(default)]
    pub seed: u64,
    pub parts: BTreeMap<String, Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<crate::constraints::Constraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect: Vec<Value>,
}

/// One named part: a shape, a placement and a join.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// SDF shapes in order (the `sdf.mesh` shape list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdf: Option<Vec<SdfShapeParam>>,
    /// Voxel size of an `sdf` part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Length>,
    /// One primitive by kind: `{"box": {"width": 0.4, ...}}` runs `prim.box`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primitive: Option<BTreeMap<String, Value>>,
    /// Generator parameters (`gen.lathe`, `gen.sweep`, `gen.loft`,
    /// `gen.extrude_shape`, `gen.pipe`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lathe: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loft: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extrude: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe: Option<Value>,
    /// A curved sheet from one outline (`gen.shell`): ears, leaves, wings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<Value>,
    /// A solid from front, side and top outlines (`sdf.silhouettes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silhouettes: Option<Value>,
    /// A feature template by id (`ear`, `horn`, `nose`, `bolt`, ...) built
    /// from `params`; SDF features (`nose`) blend like `sdf` parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// Strands grown on another part: `hair`, `fur`, `eyebrows`, `beard`
    /// or `grass`, shaped by `params` (`on`, `region`, `style`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strands: Option<String>,
    /// The parameters of a `feature` (`{"variant": "elf"}`) or of
    /// `strands` (`{"on": "head", "region": [...]}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// A mirrored copy of another part across a root plane (`plane`, x by
    /// default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<Axis>,
    /// Another part's mesh, shared, at this part's own placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Where the part sits on its parent's surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    /// Position, rotation and scale in the root's space (with an anchor:
    /// rotation and scale only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<PartTransform>,
    /// How the part joins its anchor parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<Join>,
    /// Fillet size of a `blend` join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Visual importance (drives budget allocation); 1 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<f64>,
    /// Edit layers: ops run on the part's mesh after its shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<EditCall>,
    /// Handle deformation after the edits: sculpting by numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deform: Option<DeformSpec>,
    /// Landmarks: a category name (`"humanoid_head"`, or `"category"` for
    /// the spec's), or named surface coordinates of this part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmarks: Option<Landmarks>,
}

/// The part's landmark set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Landmarks {
    Category(String),
    Named(BTreeMap<String, Anchor>),
}

/// An axis or plane name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    #[default]
    X,
    Y,
    Z,
}

impl Axis {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How a part joins its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// Its own mesh object; it may pass through the parent (eyes, buttons,
    /// armor plates).
    #[default]
    Separate,
    /// Merged into the parent's surface with a smooth fillet (SDF parts).
    Blend,
    /// Stitched into the parent (hole cut, loops bridged); not yet built.
    Weld,
    /// Texture-only detail baked onto the parent later; compiled hidden.
    Decal,
}

/// A surface coordinate on a part, used by anchors and named landmarks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// The part it is on (required for anchors; a named landmark is on its
    /// own part when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// A landmark of that part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmark: Option<String>,
    /// (u, v) on the part's bounding ellipsoid: u around +Y from +Z toward
    /// +X, v from the bottom (0) to the top (1); projected onto the surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<[f64; 2]>,
    /// The first hit of a ray from outside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ray: Option<RaySpec>,
    /// The surface point nearest a point (root space).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest: Option<[Length; 3]>,
    /// Added to the resolved point, along the child's axes (the parent's,
    /// or the surface frame with `orient: "normal"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[Length; 3]>,
    /// The child's axes: the parent's (default) or +Y along the surface
    /// normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orient: Option<Orient>,
}

/// Orientation of an anchored part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orient {
    #[default]
    Parent,
    Normal,
}

/// A ray from outside a part toward it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaySpec {
    /// The side it comes from: `+x`, `-x`, `+y`, `-y`, `+z` or `-z`.
    pub from: String,
    /// Height (y) of a horizontal ray; the part's middle when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Length>,
    /// A point the ray passes through (its coordinate along the ray is
    /// ignored); overrides `height`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through: Option<[Length; 3]>,
}

/// Placement in the root's space.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartTransform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[Length; 3]>,
    /// Euler angles in degrees about X, then Y, then Z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<[Angle; 3]>,
    /// Uniform or per-axis scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scale {
    Uniform(f64),
    Axes([f64; 3]),
}

impl Scale {
    pub fn vec(self) -> [f64; 3] {
        match self {
            Scale::Uniform(s) => [s; 3],
            Scale::Axes(a) => a,
        }
    }
}

/// Handles on a part (part space): points moved by offsets, pins held,
/// the surface following within `radius`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeformSpec {
    pub handles: Vec<DeformHandle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LocalPoint>,
    /// Reach along the surface; 40% of the part's diagonal when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<Length>,
}

/// One handle: a point of the part and its offset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeformHandle {
    #[serde(rename = "move")]
    pub at: LocalPoint,
    pub by: [Length; 3],
}

/// A point of a part: part-space coordinates, or a name (a landmark of the
/// part, one of its SDF shapes, or a face group of its mesh).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LocalPoint {
    At([Length; 3]),
    Name(String),
}

/// An op run on the part's mesh (an edit layer). The compiler sets
/// `target`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditCall {
    pub op: String,
    #[serde(default)]
    pub args: Value,
    /// Names the groups the op creates (`<label>.<role>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The shape of a part, after checking that exactly one kind is given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind<'a> {
    Sdf(&'a [SdfShapeParam]),
    /// The spec key, the op id and its arguments.
    Op(&'static str, &'static str, &'a Value),
    Primitive(&'a str, &'a Value),
    Mirror(&'a str),
    Instance(&'a str),
    /// A feature template id and its parameters.
    Feature(&'a str, &'a Value),
    /// A strands kind and its parameters.
    Strands(&'a str, &'a Value),
}

const NO_PARAMS: &Value = &Value::Null;

const PRIMITIVES: [&str; 11] = ["box", "cylinder", "cone", "quad_sphere", "uv_sphere", "ico_sphere", "torus", "tube", "capsule", "wedge", "plane"];

/// Shape kind keys and the op each runs.
pub const GENERATORS: [(&str, &str); 7] = [
    ("lathe", "gen.lathe"),
    ("sweep", "gen.sweep"),
    ("loft", "gen.loft"),
    ("extrude", "gen.extrude_shape"),
    ("pipe", "gen.pipe"),
    ("shell", "gen.shell"),
    ("silhouettes", "sdf.silhouettes"),
];

impl Part {
    /// The part's one shape kind, or an error naming what is wrong.
    ///
    /// Complexity: O(1).
    pub fn kind(&self, path: &str) -> Result<Kind<'_>, SpecError> {
        let gens = [&self.lathe, &self.sweep, &self.loft, &self.extrude, &self.pipe, &self.shell, &self.silhouettes];
        let mut kinds: Vec<(&str, Kind<'_>)> = Vec::new();
        if let Some(s) = &self.sdf {
            kinds.push(("sdf", Kind::Sdf(s)));
        }
        if let Some(p) = &self.primitive {
            let mut it = p.iter();
            match (it.next(), it.next()) {
                (Some((k, v)), None) if PRIMITIVES.contains(&k.as_str()) => kinds.push(("primitive", Kind::Primitive(k, v))),
                (Some((k, _)), None) => {
                    return Err(SpecError::new(
                        &format!("{path}/primitive/{k}"),
                        format!("`{k}` is not a primitive; use one of {}", PRIMITIVES.join(", ")),
                    ));
                }
                _ => return Err(SpecError::new(&format!("{path}/primitive"), "give exactly one primitive, for example {\"box\": {\"width\": 0.4}}")),
            }
        }
        for ((key, op), v) in GENERATORS.iter().zip(gens) {
            if let Some(v) = v {
                kinds.push((key, Kind::Op(key, op, v)));
            }
        }
        let params = self.params.as_ref().unwrap_or(NO_PARAMS);
        if let Some(f) = &self.feature {
            kinds.push(("feature", Kind::Feature(f, params)));
        }
        if let Some(k) = &self.strands {
            kinds.push(("strands", Kind::Strands(k, params)));
        }
        if self.params.is_some() && self.feature.is_none() && self.strands.is_none() {
            return Err(SpecError::new(&format!("{path}/params"), "params belong to a feature or strands part: give \"feature\" or \"strands\" too"));
        }
        if let Some(m) = &self.mirror {
            kinds.push(("mirror", Kind::Mirror(m)));
        }
        if let Some(i) = &self.instance {
            kinds.push(("instance", Kind::Instance(i)));
        }
        match kinds.as_slice() {
            [(_, k)] => Ok(*k),
            [] => Err(SpecError::new(
                path,
                "the part has no shape: give one of sdf, primitive, lathe, sweep, loft, extrude, pipe, shell, silhouettes, feature, strands, mirror or instance",
            )),
            [(a, _), (b, _), ..] => Err(SpecError::new(path, format!("the part has two shapes (`{a}` and `{b}`); give one"))),
        }
    }

    /// Whether the part's own shape is an SDF (`sdf` shapes or an SDF
    /// feature such as `nose`); mirrors answer for their source elsewhere.
    ///
    /// Complexity: O(1).
    pub fn is_sdf(&self) -> bool {
        self.sdf.is_some() || self.feature.as_deref().and_then(|f| p3d_features::get(f).ok()).is_some_and(|f| f.info().sdf)
    }

    pub fn join(&self) -> Join {
        self.join.unwrap_or_default()
    }
}

/// Parses a spec, with errors at the JSON pointer of the offending value.
///
/// Complexity: O(size of the spec).
pub fn parse(source: &Value) -> Result<ModelSpec, SpecError> {
    let spec: ModelSpec = serde_path_to_error::deserialize(source).map_err(|e| {
        let path = pointer(&e.path().to_string());
        SpecError::new(&path, e.inner().to_string())
    })?;
    if spec.parts.is_empty() {
        return Err(SpecError::new("/parts", "give at least one part"));
    }
    if let Some(d) = &spec.dna {
        p3d_perceive::dna::targets(d).map_err(|(path, message)| SpecError::new(&format!("/dna{path}"), message))?;
    }
    for name in spec.parts.keys() {
        p3d_scene::name::check_name(name)
            .map_err(|e| SpecError::new(&format!("/parts/{name}"), format!("`{name}` is not a valid part name: {e}")))?;
    }
    Ok(spec)
}

/// `parts.head.sdf[2].radius` (serde_path_to_error) as `/parts/head/sdf/2/radius`.
pub(crate) fn pointer(dotted: &str) -> String {
    if dotted == "." || dotted.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for seg in dotted.split('.') {
        // `sdf[2]` and `[2]` both carry an index.
        let mut rest = seg;
        if let Some(i) = rest.find('[') {
            if i > 0 {
                out.push('/');
                out.push_str(&rest[..i]);
            }
            rest = &rest[i..];
            while let Some(end) = rest.find(']') {
                out.push('/');
                out.push_str(&rest[1..end]);
                rest = &rest[end + 1..];
            }
        } else if !rest.is_empty() && rest != "?" {
            out.push('/');
            out.push_str(rest);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn errors_point_at_the_value() {
        let e = parse(&json!({ "parts": { "head": { "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": "big" } }] } } })).unwrap_err();
        assert!(e.path.starts_with("/parts/head/sdf/0"), "{e:?}");
        let e = parse(&json!({ "parts": { "head": { "sdff": [] } } })).unwrap_err();
        assert_eq!(e.path, "/parts/head/sdff", "{e:?}");
        assert!(e.message.contains("sdff"), "{e:?}");
        let e = parse(&json!({ "parts": {} })).unwrap_err();
        assert_eq!(e.path, "/parts");
    }

    #[test]
    fn a_part_has_exactly_one_shape() {
        let s = parse(&json!({ "parts": {
            "a": { "primitive": { "box": { "width": 0.2 } } },
            "b": { "mirror": "a" },
            "c": {},
            "d": { "mirror": "a", "instance": "a" },
            "e": { "primitive": { "blob": {} } }
        } }))
        .unwrap();
        assert!(matches!(s.parts["a"].kind("/parts/a"), Ok(Kind::Primitive("box", _))));
        assert!(matches!(s.parts["b"].kind("/parts/b"), Ok(Kind::Mirror("a"))));
        assert!(s.parts["c"].kind("/parts/c").unwrap_err().message.contains("no shape"));
        assert!(s.parts["d"].kind("/parts/d").unwrap_err().message.contains("two shapes"));
        assert_eq!(s.parts["e"].kind("/parts/e").unwrap_err().path, "/parts/e/primitive/blob");
    }

    #[test]
    fn dotted_paths_become_pointers() {
        assert_eq!(pointer("parts.head.sdf[2].radius"), "/parts/head/sdf/2/radius");
        assert_eq!(pointer("parts.head"), "/parts/head");
        assert_eq!(pointer("."), "");
    }
}
