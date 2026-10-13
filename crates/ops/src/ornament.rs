//! `gen.ornament`: a motif repeated along a path or around a band, as
//! geometry (studs, pyramids, beads, runes, rope, scroll filigree).

use glam::Vec3;
use p3d_features::ornament::{Motif, OrnamentSpec, Outward, band, ornament};
use p3d_scene::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_length, check_new_object, create_mesh_object};
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::Length;
use crate::prim::created_summary;
use crate::tx::Tx;

/// What repeats along the path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MotifParam {
    /// Domed rivet heads.
    #[default]
    Stud,
    /// Square pyramid studs.
    Pyramid,
    /// A beaded rim.
    Bead,
    /// Angular glyphs of raised strokes.
    Rune,
    /// Twisted plies, continuous.
    Rope,
    /// A wave with curls, continuous filigree.
    Scroll,
}

/// An axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BandAxis {
    X,
    #[default]
    Y,
    Z,
}

/// A ring around an axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BandParam {
    /// Ring radius: the surface's radius there (a haft, a helmet rim).
    pub radius: Length,
    /// The axis the ring goes around.
    #[serde(default)]
    pub axis: BandAxis,
    /// The ring's center.
    #[serde(default)]
    pub at: [Length; 3],
}

fn default_size() -> Length {
    Length(0.01)
}

/// Parameters of `gen.ornament`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OrnamentParams {
    /// Object name; `ornament` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Origin position in the parent's space.
    #[serde(default)]
    pub position: [Length; 3],
    /// What repeats: `stud`, `pyramid`, `bead`, `rune`, `rope` or `scroll`.
    #[serde(default)]
    pub motif: MotifParam,
    /// The curve to follow, as points; a 5 cm band around Y when neither `path` nor `band` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<[Length; 3]>>,
    /// Join the path's last point back to the first.
    #[serde(default)]
    pub closed: bool,
    /// A ring around an axis instead of a path: `{"radius": "2cm", "axis": "y", "at": [0, 0.3, 0]}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<BandParam>,
    /// Out of the surface along a path (made perpendicular to it); +Y when omitted, and away from the middle for a closed path. Bands face away from their axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    /// Motifs along the path; fills it at `spacing` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Distance between motif centers (scroll: the wave length); 1.5 sizes when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<Length>,
    /// Motif width (rope and scroll: the band width).
    #[serde(default = "default_size")]
    pub size: Length,
    /// Height above the surface; 0.4 sizes when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relief: Option<Length>,
    /// Depth sunk into the surface; 0.15 sizes when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sink: Option<Length>,
    /// Random change of spacing, size and turn: 0 even (regular rhythm) to 1 loose.
    #[serde(default)]
    pub variation: f64,
    /// Rope plies (2 to 5); 3 when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plies: Option<u32>,
}

impl Default for OrnamentParams {
    fn default() -> Self {
        OrnamentParams {
            name: None,
            parent: None,
            position: [Length(0.0); 3],
            motif: MotifParam::Stud,
            path: None,
            closed: false,
            band: None,
            normal: None,
            count: None,
            spacing: None,
            size: default_size(),
            relief: None,
            sink: None,
            variation: 0.0,
            plies: None,
        }
    }
}

fn v3(p: &[Length; 3]) -> Vec3 {
    Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32)
}

/// The kernel spec from parameters; `seed` drives the variation and rune
/// choice.
///
/// Complexity: O(path points).
pub fn ornament_spec(p: &OrnamentParams, seed: u64) -> Result<OrnamentSpec, OpError> {
    check_length("/size", p.size, 1e-5, 10.0)?;
    for (path, v) in [("/spacing", p.spacing), ("/relief", p.relief), ("/sink", p.sink)] {
        if let Some(v) = v {
            check_length(path, v, 0.0, 10.0)?;
        }
    }
    if !(0.0..=1.0).contains(&p.variation) {
        return Err(OpError::out_of_range("/variation", "variation runs from 0 to 1"));
    }
    if p.plies.is_some_and(|n| !(2..=5).contains(&n)) {
        return Err(OpError::out_of_range("/plies", "a rope has 2 to 5 plies"));
    }
    let size = p.size.0 as f32;
    let (path, closed, outward) = match (&p.path, &p.band) {
        (Some(_), Some(_)) => return Err(OpError::invalid("/band", "give a path or a band, not both")),
        (Some(pts), None) => {
            if pts.len() < 2 {
                return Err(OpError::invalid("/path", "a path needs at least two points"));
            }
            let path: Vec<Vec3> = pts.iter().map(v3).collect();
            let outward = match p.normal {
                Some(n) => {
                    let n = Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32);
                    if !n.is_finite() || n.length() < 1e-6 {
                        return Err(OpError::invalid("/normal", "the normal must be a nonzero direction"));
                    }
                    Outward::Along(n)
                }
                // A closed path faces away from its middle, around the normal
                // of its plane (Newell's method): a belt, a rim.
                None if p.closed && path.len() >= 3 => {
                    let center = path.iter().copied().sum::<Vec3>() / path.len() as f32;
                    let mut axis = Vec3::ZERO;
                    for (i, a) in path.iter().enumerate() {
                        let b = path[(i + 1) % path.len()];
                        axis += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
                    }
                    Outward::FromAxis { point: center, axis: axis.try_normalize().unwrap_or(Vec3::Y) }
                }
                None => Outward::Along(Vec3::Y),
            };
            (path, p.closed, outward)
        }
        (None, b) => {
            let (radius, axis, at) = match b {
                Some(b) => {
                    check_length("/band/radius", b.radius, 1e-5, 100.0)?;
                    let axis = match b.axis {
                        BandAxis::X => Vec3::X,
                        BandAxis::Y => Vec3::Y,
                        BandAxis::Z => Vec3::Z,
                    };
                    (b.radius.0 as f32, axis, v3(&b.at))
                }
                None => (0.05, Vec3::Y, Vec3::ZERO),
            };
            // Enough points that the ring reads round at its size.
            let n = ((std::f32::consts::TAU * radius / (size * 0.3)).ceil() as usize).clamp(24, 2048);
            (band(radius, at, axis, n), true, Outward::FromAxis { point: at, axis })
        }
    };
    Ok(OrnamentSpec {
        path,
        closed,
        outward,
        motif: match p.motif {
            MotifParam::Stud => Motif::Stud,
            MotifParam::Pyramid => Motif::Pyramid,
            MotifParam::Bead => Motif::Bead,
            MotifParam::Rune => Motif::Rune,
            MotifParam::Rope => Motif::Rope,
            MotifParam::Scroll => Motif::Scroll,
        },
        count: p.count.unwrap_or(0),
        spacing: p.spacing.map_or(0.0, |s| s.0 as f32),
        size,
        relief: p.relief.map_or(size * 0.4, |r| r.0 as f32),
        sink: p.sink.map_or(size * 0.15, |s| s.0 as f32),
        variation: p.variation as f32,
        plies: p.plies.unwrap_or(3),
        seed,
    })
}

/// `gen.ornament`.
pub struct GenOrnament;

impl Op for GenOrnament {
    const ID: &'static str = "gen.ornament";
    type Params = OrnamentParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Ornament",
            category: Category::Gen,
            summary: "Repeats a motif (studs, pyramids, beads, runes, rope, scroll filigree) along a path or around a band, raised off the surface as geometry.",
            docs: "Ornament is described, not drawn: a `motif`, where it runs (`path` points with the `normal` out of the surface, or a \
                   `band` around an axis such as a haft or a helmet rim), and `size`, `relief` above the surface and `sink` into it. \
                   Motifs are spaced 1.5 sizes apart unless `count` or `spacing` says otherwise; `variation` loosens spacing, size \
                   and turn so the rhythm is not mechanically even (leave it at 0 for regular dwarven work). Runes are angular \
                   glyphs of raised strokes chosen by the seed; rope is twisted plies; scroll is a wave with curls. Every piece is \
                   closed. In a Model Spec the `ornament` shape kind takes the same parameters.\n\n\
                   **Use for** rune bands on hafts and belts, studs along an edge, rope trim, beaded rims and filigree.",
            examples: &[OpExample {
                args: r#"{"motif": "stud", "band": {"radius": "2cm"}, "count": 8, "size": "8mm"}"#,
                result: "created ornament (o:4) stud ornament 43.59 x 8 x 43.53 mm: 488 faces (944 tris; budget 6,000)",
            }],
            aliases: &[
                "ornament",
                "trim",
                "studs",
                "rivets along",
                "runes",
                "rune band",
                "filigree",
                "rope trim",
                "beading",
                "decoration",
                "array along curve",
            ],
            complexity: "O(path points + motifs x motif faces)",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &OrnamentParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        ornament_spec(p, 0).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &OrnamentParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = ornament_spec(p, ctx.seed)?;
        let m = ornament(&spec).map_err(|e| OpError::invalid("/size", e.message))?;
        let key = create_mesh_object(tx, Some(p.name.as_deref().unwrap_or("ornament")), "ornament", p.parent.as_deref(), &p.position, m)?;
        Ok(created_summary(tx, key, &format!("{} ornament", spec.motif.as_str())))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn closed_paths_face_away_from_their_middle() {
        let ring: Vec<[Length; 3]> = (0..12)
            .map(|i| {
                let a = i as f64 / 12.0 * std::f64::consts::TAU;
                [Length(0.3 * a.sin()), Length(0.6), Length(0.2 * a.cos())]
            })
            .collect();
        let p = OrnamentParams { path: Some(ring.clone()), closed: true, ..OrnamentParams::default() };
        match ornament_spec(&p, 0).unwrap().outward {
            Outward::FromAxis { point, axis } => assert!(point.y > 0.59 && axis.y.abs() > 0.99, "{point} {axis}"),
            other => panic!("{other:?}"),
        }
        // An open path, or a given normal, keeps a fixed direction.
        let open = OrnamentParams { path: Some(ring), ..OrnamentParams::default() };
        assert_eq!(ornament_spec(&open, 0).unwrap().outward, Outward::Along(Vec3::Y));
    }
}
