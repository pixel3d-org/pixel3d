//! Wing archetypes: `feathered_wings` and `membrane_wings`, a pair of
//! `wing` features on a skeleton of wing bones (shoulder, elbow, wrist,
//! tip and, for membranes, the finger tips) on both sides of a root joint
//! between the shoulder blades. `at: "dwarf.back"` puts them on a body.
//! The wing shapes come from the parameters; the joints say where their
//! bones are, for anchors and rigging.

use glam::Vec3;
use p3d_features::{ParamInfo, ParamKind, Params};
use serde_json::{Map, Value, json};

use super::{Archetype, ArchetypeInfo, Generated, Rig, SELF, num};

const FEATHERS: &[&str] = &["solid", "cards"];

/// The parameters of a wing pair; `membrane` sets the defaults that differ.
const fn params(span: f64, chord: f64, sweep: f64) -> [ParamInfo; 13] {
    [
        ParamInfo { name: "span", doc: "Shoulder to wing tip, spread (one wing).", kind: ParamKind::Length, default: span, min: 0.05, max: 20.0 },
        ParamInfo { name: "apart", doc: "Distance between the two wing roots.", kind: ParamKind::Length, default: 0.2, min: 0.0, max: 5.0 },
        ParamInfo { name: "fold", doc: "0 spread to 1 folded against the back.", kind: ParamKind::Ratio, default: 0.0, min: 0.0, max: 1.0 },
        ParamInfo { name: "raise", doc: "Lift of the wings above level (degrees).", kind: ParamKind::Angle, default: 22.0, min: -45.0, max: 80.0 },
        ParamInfo {
            name: "tilt",
            doc: "Turn about the span so the trailing edges hang down (degrees): 0 in flight, 70 spread on display.",
            kind: ParamKind::Angle,
            default: 70.0,
            min: -90.0,
            max: 90.0,
        },
        ParamInfo { name: "sweep", doc: "How far the tips sweep back (degrees).", kind: ParamKind::Angle, default: sweep, min: -20.0, max: 70.0 },
        ParamInfo { name: "chord", doc: "Wing depth as a share of the span.", kind: ParamKind::Ratio, default: chord, min: 0.15, max: 0.9 },
        ParamInfo {
            name: "primaries",
            doc: "Feathered: flight feathers along the hand.",
            kind: ParamKind::Count,
            default: 10.0,
            min: 3.0,
            max: 16.0,
        },
        ParamInfo {
            name: "secondaries",
            doc: "Feathered: flight feathers along the forearm.",
            kind: ParamKind::Count,
            default: 12.0,
            min: 3.0,
            max: 30.0,
        },
        ParamInfo { name: "coverts", doc: "Feathered: rows of covert feathers.", kind: ParamKind::Count, default: 2.0, min: 0.0, max: 3.0 },
        ParamInfo { name: "feathers", doc: "Feathered: solid slabs or cards.", kind: ParamKind::Choice(FEATHERS), default: 0.0, min: 0.0, max: 1.0 },
        ParamInfo { name: "fingers", doc: "Membrane: finger bones per wing.", kind: ParamKind::Count, default: 4.0, min: 3.0, max: 5.0 },
        ParamInfo {
            name: "scallop",
            doc: "Membrane: sag of the trailing edge between finger tips.",
            kind: ParamKind::Ratio,
            default: 0.32,
            min: 0.0,
            max: 0.5,
        },
    ]
}

const FEATHERED: [ParamInfo; 13] = params(1.4, 0.42, 18.0);
const MEMBRANE: [ParamInfo; 13] = params(1.6, 0.5, 12.0);

const JOINTS: &[(&str, &str)] = &[
    ("root", "between the wing roots (the root: `at` puts it on a body's back)"),
    ("shoulder_l", "the left wing's root (_r for the right)"),
    ("elbow_l", "the left elbow"),
    ("wrist_l", "the left wrist"),
    ("tip_l", "the left wing tip"),
    ("finger_1_l", "membrane: the left finger tips, leading first"),
];

const PARTS: &[(&str, &str)] = &[("wing_l, wing_r", "wing features, the right one mirrored")];

struct Wings {
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    aliases: &'static [&'static str],
    params: &'static [ParamInfo],
    membrane: bool,
}

const KINDS: [Wings; 2] = [
    Wings {
        id: "feathered_wings",
        label: "Feathered wings",
        summary: "A pair of feathered wings: lens arms with primaries, secondaries, tertials and covert rows as cambered feather slabs or cards.",
        aliases: &["wings", "angel wings", "bird wings", "griffin wings"],
        params: &FEATHERED,
        membrane: false,
    },
    Wings {
        id: "membrane_wings",
        label: "Membrane wings",
        summary: "A pair of membrane wings: arm and finger bones with a cupped membrane whose trailing edge sags between the finger tips.",
        aliases: &["bat wings", "dragon wings", "demon wings"],
        params: &MEMBRANE,
        membrane: true,
    },
];

/// Every wing archetype.
pub(super) fn all() -> Vec<Box<dyn Archetype>> {
    KINDS.iter().map(|k| Box::new(Arch(k)) as Box<dyn Archetype>).collect()
}

struct Arch(&'static Wings);

impl Arch {
    /// The `wing` feature parameters for one side.
    fn wing_params(&self, p: &Params, side: &str) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), json!(if self.0.membrane { "membrane" } else { "feathered" }));
        m.insert("span".into(), num(p.f("span")));
        m.insert("fold".into(), num(p.f("fold")));
        m.insert("raise".into(), num(p.f("raise").to_degrees()));
        m.insert("sweep".into(), num(p.f("sweep").to_degrees()));
        m.insert("tilt".into(), num(p.f("tilt").to_degrees()));
        m.insert("chord".into(), num(p.f("chord")));
        if self.0.membrane {
            m.insert("fingers".into(), json!(p.count("fingers")));
            m.insert("scallop".into(), num(p.f("scallop")));
        } else {
            m.insert("primaries".into(), json!(p.count("primaries")));
            m.insert("secondaries".into(), json!(p.count("secondaries")));
            m.insert("coverts".into(), json!(p.count("coverts")));
            m.insert("feathers".into(), json!(p.choice("feathers")));
        }
        m.insert("side".into(), json!(side));
        Value::Object(m)
    }
}

impl Archetype for Arch {
    fn info(&self) -> ArchetypeInfo {
        ArchetypeInfo {
            id: self.0.id,
            label: self.0.label,
            summary: self.0.summary,
            params: self.0.params,
            joints: JOINTS,
            parts: PARTS,
            root: "root",
            aliases: self.0.aliases,
        }
    }

    fn joints(&self, p: &Params) -> Rig {
        let mut rig = Rig::default();
        rig.add("root", Vec3::ZERO, None);
        let half = p.f("apart") * 0.5;
        // The same bones the wing feature builds (a left wing), mirrored for the right.
        let bones = p3d_features::wing_bones(&self.wing_params(p, "left")).ok();
        for (side, s) in [("l", 1.0f32), ("r", -1.0)] {
            let m = Vec3::new(s, 1.0, 1.0);
            let at = |v: Vec3| Vec3::new(half * s, 0.0, 0.0) + v * m;
            let n = |j: &str| format!("{j}_{side}");
            rig.add(&n("shoulder"), at(Vec3::ZERO), Some("root"));
            if let Some(b) = &bones {
                rig.add(&n("elbow"), at(b.elbow), Some(&n("shoulder")));
                rig.add(&n("wrist"), at(b.wrist), Some(&n("elbow")));
                rig.add(&n("tip"), at(b.tip), Some(&n("wrist")));
                for (i, f) in b.fingers.iter().enumerate() {
                    rig.add(&format!("finger_{}_{side}", i + 1), at(*f), Some(&n("wrist")));
                }
            }
        }
        rig
    }

    fn build(&self, p: &Params, rig: &mut Rig) -> Generated {
        let names: Vec<String> = rig.joints.iter().map(|j| j.name.clone()).collect();
        for n in &names {
            let s = if n.ends_with("_r") { -1.0 } else { 1.0 };
            rig.frame(n, Vec3::Y, Vec3::X * s);
        }
        // The shoulders keep the root's frame, so each wing is built in it.
        rig.frame("shoulder_l", Vec3::Y, Vec3::X);
        rig.frame("shoulder_r", Vec3::Y, Vec3::X);
        let mut parts = Vec::new();
        for (side, word) in [("l", "left"), ("r", "right")] {
            parts.push((
                format!("wing_{side}"),
                json!({
                    "feature": "wing",
                    "params": self.wing_params(p, word),
                    "anchor": { "on": SELF, "landmark": format!("shoulder_{side}"), "orient": "normal" },
                }),
            ));
        }
        let chain = |side: &str| -> Vec<String> {
            ["shoulder", "elbow", "wrist", "tip"].iter().map(|j| format!("{j}_{side}")).filter(|j| names.contains(j)).collect()
        };
        let curves = vec![("wing_l".to_owned(), chain("l")), ("wing_r".to_owned(), chain("r"))];
        Generated { curves, parts, dna: json!({ "archetype": self.0.id }) }
    }
}
