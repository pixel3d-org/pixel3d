//! Humanoid archetypes: `human`, `elf`, `dwarf`, `orc` and `halfling`.
//! One generator and a table of race defaults (height, head count, build,
//! shoulder width, leg length, face). The anatomy kit is SDF masses around
//! the joints (ribcage, pecs, lats, traps, belly, pelvis, glutes; deltoid,
//! biceps, triceps and forearm on the arms; quads, hamstrings and calves on
//! the legs) blended into one body, with feature parts on top: `head_base`
//! with a `nose`, `ear`s and `eye`s, `hand`s gripping or relaxed, eyebrows
//! and optionally a beard and hair as strands.
//!
//! Space: feet on y = 0 around the origin, facing +Z, the character's left
//! toward +X. Lengths scale with the head height `h = height / heads`.

use std::f32::consts::PI;

use glam::Vec3;
use p3d_features::{ParamInfo, ParamKind, Params};
use serde_json::{Value, json};

use super::{Archetype, ArchetypeInfo, Generated, Rig, SELF, num, vec};

const POSES: &[&str] = &["relaxed", "a_pose", "t_pose"];
const HOLDS: &[&str] = &["none", "right", "left", "both"];
const BEARDS: &[&str] = &["none", "short", "full", "braided"];
const HAIRS: &[&str] = &["none", "short", "long"];

/// The parameter list of one race, with its defaults.
#[allow(clippy::too_many_arguments)]
const fn params(
    height: f64,
    heads: f64,
    build: f64,
    belly: f64,
    shoulders: f64,
    legs: f64,
    hands: f64,
    feet: f64,
    stance: f64,
    beard: f64,
    hair: f64,
) -> [ParamInfo; 15] {
    [
        ParamInfo { name: "height", doc: "From the soles to the crown.", kind: ParamKind::Length, default: height, min: 0.2, max: 6.0 },
        ParamInfo {
            name: "heads",
            doc: "Height in head heights (chin to crown): 4.3 for a dwarf, 7.5 for a human, 8 for an elf.",
            kind: ParamKind::Ratio,
            default: heads,
            min: 3.0,
            max: 9.5,
        },
        ParamInfo { name: "build", doc: "Muscle and mass (0 lean, 1 massive).", kind: ParamKind::Ratio, default: build, min: 0.0, max: 1.0 },
        ParamInfo { name: "belly", doc: "Belly forward (0 flat, 1 barrel).", kind: ParamKind::Ratio, default: belly, min: 0.0, max: 1.0 },
        ParamInfo {
            name: "shoulders",
            doc: "Width across the shoulders in head heights.",
            kind: ParamKind::Ratio,
            default: shoulders,
            min: 1.3,
            max: 3.2,
        },
        ParamInfo {
            name: "legs",
            doc: "Crotch height as a share of the height (short dwarf legs 0.36).",
            kind: ParamKind::Ratio,
            default: legs,
            min: 0.25,
            max: 0.58,
        },
        ParamInfo { name: "arms", doc: "Arm length relative to the race's.", kind: ParamKind::Ratio, default: 1.0, min: 0.75, max: 1.35 },
        ParamInfo { name: "hands", doc: "Hand size relative to a human's.", kind: ParamKind::Ratio, default: hands, min: 0.6, max: 1.7 },
        ParamInfo { name: "feet", doc: "Foot (or boot) size relative to a human's.", kind: ParamKind::Ratio, default: feet, min: 0.6, max: 1.9 },
        ParamInfo {
            name: "stance",
            doc: "How far apart the feet stand (0 together, 1 wide).",
            kind: ParamKind::Ratio,
            default: stance,
            min: 0.0,
            max: 1.0,
        },
        ParamInfo {
            name: "pose",
            doc: "Arms: relaxed (down, just off the body), a_pose (45 degrees, for rigging) or t_pose.",
            kind: ParamKind::Choice(POSES),
            default: 0.0,
            min: 0.0,
            max: 2.0,
        },
        ParamInfo {
            name: "hold",
            doc: "Hands that grip (forearm forward, fist closed around joint grip_l or grip_r): none, right, left or both.",
            kind: ParamKind::Choice(HOLDS),
            default: 0.0,
            min: 0.0,
            max: 3.0,
        },
        ParamInfo {
            name: "beard",
            doc: "Beard strands: none, short, full or braided (braids hang from the chin).",
            kind: ParamKind::Choice(BEARDS),
            default: beard,
            min: 0.0,
            max: 3.0,
        },
        ParamInfo { name: "hair", doc: "Hair strands: none, short or long.", kind: ParamKind::Choice(HAIRS), default: hair, min: 0.0, max: 2.0 },
        ParamInfo {
            name: "detail",
            doc: "Voxel size of the body (the head and hands are finer); a 170th of the height when 0.",
            kind: ParamKind::Length,
            default: 0.0,
            min: 0.0,
            max: 0.1,
        },
    ]
}

/// What sets one race apart beyond its parameter defaults.
struct Race {
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    aliases: &'static [&'static str],
    params: &'static [ParamInfo],
    /// Chin to the top of the sternum, in head heights.
    neck: f32,
    /// Head forward of the shoulders, in head heights.
    hunch: f32,
    /// `head_base` parameters (its height comes from the head count).
    head: &'static [(&'static str, f64)],
    /// `nose` parameters, lengths in head heights.
    nose: &'static [(&'static str, f64)],
    /// Ear variant, length in head heights, pointiness and flare (degrees).
    ear: (&'static str, f32, f64, f64),
    /// Hand width, thickness, finger length and finger width.
    hand: (f64, f64, f64, f64),
    boots: bool,
    tusks: bool,
    /// Eyebrow length in head heights.
    brows: f32,
    culture: &'static str,
}

const HUMAN_PARAMS: [ParamInfo; 15] = params(1.8, 7.5, 0.5, 0.2, 2.0, 0.47, 1.0, 1.0, 0.3, 0.0, 1.0);
const ELF_PARAMS: [ParamInfo; 15] = params(1.9, 8.0, 0.3, 0.0, 1.75, 0.5, 0.95, 0.95, 0.25, 0.0, 2.0);
const DWARF_PARAMS: [ParamInfo; 15] = params(1.35, 4.3, 0.85, 0.35, 2.0, 0.36, 1.25, 1.3, 0.55, 3.0, 2.0);
const ORC_PARAMS: [ParamInfo; 15] = params(2.0, 7.0, 0.9, 0.35, 2.4, 0.45, 1.2, 1.15, 0.5, 0.0, 0.0);
const HALFLING_PARAMS: [ParamInfo; 15] = params(1.1, 5.0, 0.4, 0.55, 1.6, 0.4, 1.0, 1.45, 0.3, 0.0, 1.0);

const RACES: [Race; 5] = [
    Race {
        id: "human",
        label: "Human",
        summary: "A human body blockout, 7.5 heads tall: anatomy masses on a skeleton, head, hands, ears and eyes.",
        aliases: &["man", "woman", "person", "character", "humanoid", "body"],
        params: &HUMAN_PARAMS,
        neck: 0.38,
        hunch: 0.0,
        head: &[],
        nose: &[],
        ear: ("human", 0.27, 0.0, 15.0),
        hand: (0.45, 0.15, 1.0, 1.0),
        boots: true,
        tusks: false,
        brows: 0.06,
        culture: "human_medieval",
    },
    Race {
        id: "elf",
        label: "Elf",
        summary: "A slender elf, 8 heads tall with long legs, high cheekbones, a narrow jaw and long pointed ears.",
        aliases: &["elven", "high elf", "wood elf"],
        params: &ELF_PARAMS,
        neck: 0.42,
        hunch: 0.0,
        head: &[("width", 0.62), ("jaw", 0.7), ("cheekbones", 0.85), ("brow", 0.2), ("chin", 0.45)],
        nose: &[("length", 0.19), ("width", 0.12), ("bridge", 0.7), ("tip", 0.8)],
        ear: ("elf", 0.42, 0.9, 30.0),
        hand: (0.42, 0.13, 1.2, 0.85),
        boots: true,
        tusks: false,
        brows: 0.06,
        culture: "elven",
    },
    Race {
        id: "dwarf",
        label: "Dwarf",
        summary: "A dwarf about 4.3 heads tall: barrel chest, heavy shoulders, short thick legs, big hands, heavy boots, a big nose under a heavy brow and a braided beard.",
        aliases: &["dwarven", "dwarf warrior"],
        params: &DWARF_PARAMS,
        neck: 0.16,
        hunch: 0.05,
        head: &[
            ("width", 0.74),
            ("depth", 0.9),
            ("jaw", 1.15),
            ("chin", 0.55),
            ("brow", 0.95),
            ("cheekbones", 0.55),
            ("sockets", 0.75),
            ("neck", 0.45),
        ],
        nose: &[("length", 0.2), ("width", 0.16), ("bridge", 0.8), ("tip", 1.8), ("flare", 1.3), ("upturn", -5.0)],
        ear: ("human", 0.25, 0.0, 18.0),
        hand: (0.52, 0.19, 0.85, 1.3),
        boots: true,
        tusks: false,
        brows: 0.1,
        culture: "dwarven",
    },
    Race {
        id: "orc",
        label: "Orc",
        summary: "A massive orc, 7 heads tall: huge shoulders and traps, a forward head with a heavy brow and jaw, pointed ears and tusks.",
        aliases: &["orcish", "half orc", "brute", "ogre"],
        params: &ORC_PARAMS,
        neck: 0.2,
        hunch: 0.12,
        head: &[("width", 0.74), ("jaw", 1.25), ("chin", 0.7), ("brow", 1.0), ("cheekbones", 0.4), ("sockets", 0.8)],
        nose: &[("length", 0.15), ("width", 0.17), ("bridge", 0.2), ("tip", 1.4), ("flare", 1.8), ("upturn", 20.0)],
        ear: ("elf", 0.25, 0.7, 40.0),
        hand: (0.5, 0.18, 0.95, 1.3),
        boots: true,
        tusks: true,
        brows: 0.05,
        culture: "orcish",
    },
    Race {
        id: "halfling",
        label: "Halfling",
        summary: "A halfling, 5 heads tall: round face, small nose, slightly pointed ears, a round belly and big bare feet.",
        aliases: &["hobbit", "hobbit like", "gnome"],
        params: &HALFLING_PARAMS,
        neck: 0.3,
        hunch: 0.0,
        head: &[("width", 0.72), ("jaw", 0.75), ("chin", 0.35), ("cheekbones", 0.6), ("brow", 0.3)],
        nose: &[("length", 0.15), ("width", 0.13), ("bridge", 0.3), ("tip", 1.3), ("upturn", 15.0)],
        ear: ("elf", 0.27, 0.35, 25.0),
        hand: (0.47, 0.15, 0.95, 1.05),
        boots: false,
        tusks: false,
        brows: 0.05,
        culture: "human_medieval",
    },
];

const JOINTS: &[(&str, &str)] = &[
    ("root", "on the ground between the feet"),
    ("pelvis", "the pelvis center"),
    ("spine", "the waist"),
    ("chest", "the ribcage center"),
    ("neck", "the base of the neck"),
    ("head", "under the chin, where the head part sits"),
    ("head_top", "the crown"),
    ("back", "between the shoulder blades on the back (wings, capes, quivers)"),
    ("shoulder_l", "the left shoulder joint (_r for the right side)"),
    ("elbow_l", "the left elbow"),
    ("wrist_l", "the left wrist; its frame is the hand's (+Y along the forearm)"),
    ("grip_l", "inside the left fist: +Y along the held handle, +X where the fingers point"),
    ("hip_l", "the left hip joint"),
    ("knee_l", "the left knee"),
    ("ankle_l", "the left ankle"),
    ("toe_l", "the left toes"),
];

const PARTS: &[(&str, &str)] = &[
    ("torso", "SDF ribcage, pecs, lats, traps, belly, pelvis and glutes; arms and legs blend into it"),
    ("arm_l, arm_r", "SDF deltoid, upper arm, biceps, triceps and forearm"),
    ("leg_l, leg_r", "SDF thigh, quads, hamstrings, knee, calf and foot or boot"),
    ("head", "the head_base feature (face parameters per race)"),
    ("nose", "the nose feature, blended into the head"),
    ("ear_l, ear_r", "ear features"),
    ("eye_l, eye_r", "eye features in the sockets"),
    ("hand_l, hand_r", "hand features at the wrists"),
    ("brow_l, brow_r", "eyebrow strands"),
    ("beard", "beard strands (with beard)"),
    ("hair", "hair strands (with hair)"),
    ("tusk_l, tusk_r", "horn features as tusks (orcs)"),
];

struct Humanoid(&'static Race);

/// Every humanoid archetype.
pub(super) fn all() -> Vec<Box<dyn Archetype>> {
    RACES.iter().map(|r| Box::new(Humanoid(r)) as Box<dyn Archetype>).collect()
}

/// Sizes every joint and part derives from.
struct Body {
    height: f32,
    h: f32,
    build: f32,
    belly: f32,
    /// Thickness unit: half the head height plus half a 7.5-head height,
    /// so short races keep adult limb widths.
    t: f32,
    /// Ribcage half width and half depth.
    cw: f32,
    cd: f32,
    upper: f32,
    fore: f32,
    hand: f32,
    foot: f32,
}

impl Body {
    fn of(p: &Params) -> Body {
        let height = p.f("height");
        let h = height / p.f("heads");
        let build = p.f("build");
        let cw = 0.36 * p.f("shoulders") * h * (0.9 + 0.1 * build);
        let arms = p.f("arms");
        Body {
            height,
            h,
            build,
            belly: p.f("belly"),
            t: 0.5 * h + 0.5 * height / 7.5,
            cw,
            cd: 0.48 * h * (0.85 + 0.3 * build),
            upper: 0.186 * height * arms,
            fore: 0.146 * height * arms,
            hand: 0.108 * height * p.f("hands"),
            foot: 0.152 * height * p.f("feet"),
        }
    }
}

/// How far the hand sits along the forearm from the wrist joint (it sinks
/// into the forearm a little).
fn hand_offset(b: &Body) -> f32 {
    -0.03 * b.hand
}

/// Which hands grip.
fn holds(p: &Params, side: &str) -> bool {
    matches!((p.choice("hold"), side), ("both", _) | ("right", "r") | ("left", "l"))
}

/// A direction bent from `d` toward the forward direction (+Z made
/// perpendicular to `d`) by `angle`.
fn bend_forward(d: Vec3, angle: f32) -> Vec3 {
    let fwd = (Vec3::Z - d * d.z).try_normalize().unwrap_or(Vec3::Z);
    (d * angle.cos() + fwd * angle.sin()).normalize_or(d)
}

/// The thumb direction of a hand on a forearm along `f`: forward and a
/// little up, across the forearm (palm toward the body when the arm hangs,
/// thumb up when the forearm points forward).
fn thumb(f: Vec3) -> Vec3 {
    let want = Vec3::new(0.0, 0.6, 1.0);
    (want - f * f.dot(want)).try_normalize().unwrap_or_else(|| f.any_orthonormal_vector())
}

impl Humanoid {
    /// `hand` feature parameters for one side.
    fn hand_params(&self, p: &Params, b: &Body, side: &str) -> Value {
        let (w, t, fl, fw) = self.0.hand;
        let grip = holds(p, side);
        json!({
            "length": num(b.hand),
            "width": w,
            "thickness": t,
            "finger_length": fl,
            "finger_width": fw,
            "curl": if grip { 78.0 } else { 34.0 },
            "thumb": if grip { 40.0 } else { 22.0 },
            "spread": if grip { 2.0 } else { 6.0 },
            "knuckles": 0.3 + 0.4 * f64::from(b.build),
            "side": if side == "l" { "left" } else { "right" },
        })
    }
}

impl Archetype for Humanoid {
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
        let race = self.0;
        let b = Body::of(p);
        let (hh, h) = (b.height, b.h);
        let y_chin = hh - h;
        let y_neck = y_chin - race.neck * h;
        let y_sh = y_neck - 0.08 * h;
        // The deltoids' outer edges meet the shoulder width.
        let x_sh = p.f("shoulders") * h * 0.5 - 0.05 * b.t - 0.19 * b.t * (0.9 + 0.35 * b.build) - 0.004 * h;
        let y_crotch = p.f("legs") * hh;
        let y_hip = y_crotch + 0.045 * hh + 0.05 * h;
        let x_hip = h * (0.34 + 0.08 * b.build + 0.06 * b.belly);
        let y_pelvis = y_hip + 0.1 * h;
        let y_chest = y_sh - 0.55 * h;
        let z_head = race.hunch * h;
        let mut rig = Rig::default();
        rig.add("root", Vec3::ZERO, None);
        rig.add("pelvis", Vec3::new(0.0, y_pelvis, 0.0), Some("root"));
        rig.add("spine", Vec3::new(0.0, (y_pelvis + y_chest) * 0.5, 0.02 * h), Some("pelvis"));
        rig.add("chest", Vec3::new(0.0, y_chest, 0.0), Some("spine"));
        rig.add("neck", Vec3::new(0.0, y_neck, -0.04 * h + z_head * 0.4), Some("chest"));
        rig.add("head", Vec3::new(0.0, y_chin, z_head), Some("neck"));
        rig.add("head_top", Vec3::new(0.0, hh, z_head), Some("head"));
        rig.add("back", Vec3::new(0.0, y_chest + 0.25 * h, -b.cd * 0.95), Some("chest"));
        // Arms: abduction keeps the elbow clear of the ribs and belly.
        let clear = b.cw * 0.95 + 0.08 * b.t * b.belly + 0.19 * b.t * (0.85 + 0.3 * b.build);
        let relaxed = ((clear - x_sh) / b.upper).clamp(0.0, 0.9).asin().max(6f32.to_radians());
        for (side, s) in [("l", 1.0f32), ("r", -1.0)] {
            let sh = Vec3::new(s * x_sh, y_sh, -0.03 * h);
            let (abduct, flex, elbow) = if holds(p, side) {
                (relaxed, 14f32.to_radians(), 88f32.to_radians())
            } else {
                match p.choice("pose") {
                    "a_pose" => (45f32.to_radians(), 4f32.to_radians(), 12f32.to_radians()),
                    "t_pose" => (88f32.to_radians(), 0.0, 4f32.to_radians()),
                    _ => (relaxed, 6f32.to_radians(), 16f32.to_radians()),
                }
            };
            let up = Vec3::new(s * abduct.sin(), -abduct.cos() * flex.cos(), abduct.cos() * flex.sin()).normalize_or(Vec3::NEG_Y);
            let el = sh + up * b.upper;
            let f = bend_forward(up, elbow);
            let wr = el + f * b.fore;
            rig.add(&format!("shoulder_{side}"), sh, Some("chest"));
            rig.add(&format!("elbow_{side}"), el, Some(&format!("shoulder_{side}")));
            rig.add(&format!("wrist_{side}"), wr, Some(&format!("elbow_{side}")));
            let (c, _) = p3d_features::hand_grip(&self.hand_params(p, &b, side)).unwrap_or((Vec3::new(0.0, b.hand * 0.4, -b.hand * 0.2), 0.02));
            let x = thumb(f) * s;
            let rot = glam::Quat::from_mat3(&glam::Mat3::from_cols(x, f, x.cross(f)));
            rig.add(&format!("grip_{side}"), wr + rot * (c + Vec3::Y * hand_offset(&b)), Some(&format!("wrist_{side}")));
        }
        // Legs: the stance spreads the feet, the knees track over them.
        let stance = p.f("stance");
        let y_knee = 0.59 * y_crotch;
        let y_ankle = 0.045 * hh + 0.02 * h * p.f("feet");
        for (side, s) in [("l", 1.0f32), ("r", -1.0)] {
            let hip = Vec3::new(s * x_hip, y_hip, 0.0);
            let ax = s * (x_hip * 0.95 + stance * 0.3 * h);
            let ankle = Vec3::new(ax, y_ankle, -0.02 * h);
            let knee = Vec3::new((hip.x + ax) * 0.5 - s * 0.02 * h, y_knee, 0.04 * h);
            let out = 10f32.to_radians() * s;
            let toe = ankle + Vec3::new(out.sin() * b.foot * 0.7, 0.025 * hh - y_ankle, out.cos() * b.foot * 0.7);
            rig.add(&format!("hip_{side}"), hip, Some("pelvis"));
            rig.add(&format!("knee_{side}"), knee, Some(&format!("hip_{side}")));
            rig.add(&format!("ankle_{side}"), ankle, Some(&format!("knee_{side}")));
            rig.add(&format!("toe_{side}"), toe, Some(&format!("ankle_{side}")));
        }
        rig
    }

    fn build(&self, p: &Params, rig: &mut Rig) -> Generated {
        let race = self.0;
        let b = Body::of(p);
        let h = b.h;
        // Frames from the (possibly edited) joints.
        for (j, to) in [("pelvis", "spine"), ("spine", "chest"), ("chest", "neck"), ("neck", "head"), ("head", "head_top")] {
            rig.aim(j, to, Vec3::X);
        }
        for side in ["l", "r"] {
            let s = if side == "l" { 1.0f32 } else { -1.0 };
            let n = |j: &str| format!("{j}_{side}");
            rig.aim(&n("shoulder"), &n("elbow"), Vec3::X * s);
            rig.aim(&n("elbow"), &n("wrist"), Vec3::X * s);
            let f = (rig.at(&n("wrist")) - rig.at(&n("elbow"))).normalize_or(Vec3::NEG_Y);
            let t = thumb(f);
            rig.frame(&n("wrist"), f, t * s);
            rig.frame(&n("grip"), t, f);
            rig.aim(&n("hip"), &n("knee"), Vec3::X * s);
            rig.aim(&n("knee"), &n("ankle"), Vec3::X * s);
            rig.aim(&n("ankle"), &n("toe"), Vec3::X * s);
            let toe_dir = rig.at(&n("toe")) - rig.at(&n("ankle"));
            rig.frame(&n("toe"), Vec3::new(toe_dir.x, 0.0, toe_dir.z), Vec3::X * s);
        }
        let root = rig.at("root");
        // Body shapes are written in the instance's space around the root.
        let at = |q: Vec3| vec(q - root);
        let anchor_root = json!({ "on": SELF, "landmark": "root", "orient": "normal" });
        let detail = if p.f("detail") > 0.0 { p.f("detail") } else { b.height / 170.0 };
        let mut parts: Vec<(String, Value)> = Vec::new();

        // ---- torso -----------------------------------------------------------------
        // Heights scale with the head (h), thicknesses with t, which mixes the
        // head with the height so short races are not thin.
        let (cw, cd, bu, e, t) = (b.cw, b.cd, b.build, b.belly, b.t);
        let m = 0.85 + 0.35 * bu;
        let (pel, chest, neck, head) = (rig.at("pelvis"), rig.at("chest"), rig.at("neck"), rig.at("head"));
        let spine = rig.at("spine");
        let hip_x = rig.at("hip_l").x.abs();
        let hip_w = hip_x + 0.22 * t * (0.9 + 0.3 * bu);
        let sh = [rig.at("shoulder_l"), rig.at("shoulder_r")];
        let mut torso = vec![
            ellipsoid("ribcage", at(chest + Vec3::new(0.0, 0.05 * h, 0.0)), Vec3::new(cw, 0.6 * h, cd), None),
            ellipsoid("upper_chest", at(Vec3::new(0.0, sh[0].y - 0.14 * h, -0.01 * h)), Vec3::new(cw * 0.96, 0.28 * h, cd * 0.88), Some(0.12 * t)),
            ellipsoid(
                "belly",
                at(spine + Vec3::new(0.0, -0.04 * h, 0.02 * h + 0.06 * h * e)),
                Vec3::new(cw * (0.74 + 0.08 * e), 0.5 * h, cd * (0.88 + 0.2 * e)),
                Some(0.18 * t),
            ),
            ellipsoid("pelvis", at(pel + Vec3::new(0.0, 0.0, -0.02 * h)), Vec3::new(hip_w, 0.4 * h, cd * 0.82), Some(0.15 * t)),
            capsule(
                "neck",
                at(neck + Vec3::new(0.0, -0.1 * h, 0.0)),
                at(head + Vec3::new(0.0, 0.2 * h, -0.07 * h)),
                0.24 * t * (1.0 + 0.3 * bu),
                None,
                Some(0.1 * t),
            ),
        ];
        for (s, n) in [(1.0f32, "l"), (-1.0, "r")] {
            let sh_n = sh[usize::from(s < 0.0)];
            torso.push(ellipsoid(
                &format!("pec_{n}"),
                at(chest + Vec3::new(s * 0.44 * cw, 0.2 * h, cd * 0.7)),
                Vec3::new(0.4 * cw, 0.21 * h, 0.16 * t * m),
                Some(0.14 * t),
            ));
            torso.push(ellipsoid(
                &format!("oblique_{n}"),
                at(Vec3::new(s * cw * 0.66, pel.y + 0.34 * h, 0.0)),
                Vec3::new(0.14 * t * m, 0.26 * h, cd * 0.6),
                Some(0.12 * t),
            ));
            torso.push(ellipsoid(
                &format!("lat_{n}"),
                at(chest + Vec3::new(s * 0.82 * cw, -0.12 * h, -0.1 * t)),
                Vec3::new(0.2 * t * m, 0.48 * h, 0.34 * t),
                Some(0.12 * t),
            ));
            torso.push(ellipsoid(
                &format!("scapula_{n}"),
                at(chest + Vec3::new(s * 0.45 * cw, 0.22 * h, -cd * 0.7)),
                Vec3::new(0.26 * t, 0.3 * h, 0.12 * t),
                Some(0.12 * t),
            ));
            torso.push(capsule(
                &format!("trap_{n}"),
                at(neck + Vec3::new(0.0, 0.3 * h, -0.1 * t)),
                at(Vec3::new(sh_n.x * 0.72, sh_n.y + 0.06 * h, sh_n.z - 0.04 * t)),
                0.14 * t * (1.0 + 0.7 * bu),
                Some(0.08 * t * (1.0 + 0.5 * bu)),
                Some(0.12 * t),
            ));
            torso.push(capsule(
                &format!("clavicle_{n}"),
                at(Vec3::new(s * 0.15 * t, neck.y - 0.02 * h, cd * 0.55)),
                at(Vec3::new(sh_n.x * 0.9, sh_n.y + 0.05 * h, 0.03 * t)),
                0.06 * t,
                None,
                Some(0.06 * t),
            ));
            torso.push(ellipsoid(
                &format!("glute_{n}"),
                at(pel + Vec3::new(s * 0.48 * hip_w, -0.14 * h, -cd * 0.5)),
                Vec3::new(0.28 * t, 0.3 * h, 0.2 * t) * (0.9 + 0.2 * bu),
                Some(0.12 * t),
            ));
        }
        parts.push(("torso".into(), json!({ "sdf": torso, "detail": num(detail), "anchor": anchor_root.clone(), "tags": ["anatomy"] })));

        // ---- arms and hands ---------------------------------------------------------------
        for (s, side) in [(1.0f32, "l"), (-1.0, "r")] {
            let n = |j: &str| format!("{j}_{side}");
            let (shj, el, wr) = (rig.at(&n("shoulder")), rig.at(&n("elbow")), rig.at(&n("wrist")));
            let du = (el - shj).normalize_or(Vec3::NEG_Y);
            let df = (wr - el).normalize_or(Vec3::NEG_Y);
            let front = (Vec3::Z - du * du.z).try_normalize().unwrap_or(Vec3::Z);
            let out = (Vec3::X * s - df * df.x * s).try_normalize().unwrap_or(Vec3::X * s);
            let k = 0.06 * t;
            let mm = 1.0 + 0.4 * bu;
            let arm = vec![
                ellipsoid("deltoid", at(shj + Vec3::new(s * 0.05 * t, 0.0, 0.0)), Vec3::new(0.19 * t, 0.22 * t, 0.22 * t) * (0.9 + 0.35 * bu), None),
                capsule(
                    "deltoid_front",
                    at(shj + Vec3::new(s * 0.04 * t, 0.04 * t, 0.05 * t)),
                    at(shj + du * b.upper * 0.45 + front * 0.02 * t),
                    0.14 * t * (0.9 + 0.3 * bu),
                    Some(0.08 * t * (0.9 + 0.3 * bu)),
                    Some(0.08 * t),
                ),
                capsule("upper_arm", at(shj), at(el), 0.14 * t * (0.9 + 0.2 * bu), Some(0.12 * t * (0.9 + 0.2 * bu)), Some(k)),
                capsule(
                    "biceps",
                    at(shj + (el - shj) * 0.3 + front * 0.06 * t),
                    at(shj + (el - shj) * 0.72 + front * 0.06 * t),
                    0.13 * t * mm,
                    Some(0.11 * t * mm),
                    Some(0.05 * t),
                ),
                capsule(
                    "triceps",
                    at(shj + (el - shj) * 0.18 - front * 0.07 * t),
                    at(shj + (el - shj) * 0.68 - front * 0.05 * t),
                    0.14 * t * mm,
                    Some(0.1 * t * mm),
                    Some(0.05 * t),
                ),
                sphere("elbow", at(el - front * 0.03 * t), 0.115 * t * (0.9 + 0.2 * bu), Some(0.05 * t)),
                capsule("forearm", at(el), at(wr), 0.12 * t * (0.9 + 0.3 * bu), Some(0.09 * t * (0.9 + 0.3 * bu)), Some(k)),
                capsule(
                    "forearm_mass",
                    at(el + df * (b.fore * 0.12) + out * 0.04 * t + front * 0.02 * t),
                    at(el + df * (b.fore * 0.6) + out * 0.015 * t),
                    0.15 * t * mm,
                    Some(0.1 * t * mm),
                    Some(0.06 * t),
                ),
                capsule(
                    "flexors",
                    at(el + df * (b.fore * 0.15) - out * 0.04 * t),
                    at(el + df * (b.fore * 0.55) - out * 0.02 * t),
                    0.12 * t * mm,
                    Some(0.09 * t * mm),
                    Some(0.06 * t),
                ),
            ];
            parts.push((
                n("arm"),
                json!({ "sdf": arm, "anchor": anchor_root.clone(), "join": "blend", "into": "torso", "blend": num(0.08 * t), "tags": ["anatomy"] }),
            ));
            let hand_detail = b.hand / 60.0;
            parts.push((
                n("hand"),
                json!({
                    "feature": "hand",
                    "params": self.hand_params(p, &b, side),
                    "anchor": { "on": SELF, "landmark": n("wrist"), "orient": "normal", "offset": [0, num(hand_offset(&b)), 0] },
                    "detail": num(hand_detail),
                    "tags": ["anatomy"],
                }),
            ));
        }

        // ---- legs and feet ----------------------------------------------------------------
        for side in ["l", "r"] {
            let n = |j: &str| format!("{j}_{side}");
            let (hip, knee, ankle, toe) = (rig.at(&n("hip")), rig.at(&n("knee")), rig.at(&n("ankle")), rig.at(&n("toe")));
            let dt = (knee - hip).normalize_or(Vec3::NEG_Y);
            let fwd = (Vec3::Z - dt * dt.z).try_normalize().unwrap_or(Vec3::Z);
            let k = 0.06 * t;
            let mm = 1.0 + 0.4 * bu;
            let mut leg = vec![
                capsule("thigh", at(hip), at(knee), 0.26 * t * (0.85 + 0.3 * bu), Some(0.15 * t * (0.9 + 0.2 * bu)), None),
                capsule(
                    "quads",
                    at(hip + (knee - hip) * 0.15 + fwd * 0.1 * t),
                    at(hip + (knee - hip) * 0.82 + fwd * 0.06 * t),
                    0.19 * t * mm,
                    Some(0.14 * t * mm),
                    Some(k),
                ),
                capsule(
                    "vastus",
                    at(hip + (knee - hip) * 0.55 + fwd * 0.04 * t + Vec3::X * 0.0),
                    at(hip + (knee - hip) * 0.88 + fwd * 0.03 * t),
                    0.14 * t * mm,
                    Some(0.12 * t * mm),
                    Some(k),
                ),
                capsule(
                    "hamstrings",
                    at(hip + (knee - hip) * 0.12 - fwd * 0.09 * t),
                    at(hip + (knee - hip) * 0.78 - fwd * 0.06 * t),
                    0.18 * t * mm,
                    Some(0.12 * t * mm),
                    Some(k),
                ),
                sphere("knee", at(knee + fwd * 0.03 * t), 0.12 * t * (0.9 + 0.2 * bu), Some(0.08 * t)),
                capsule("shin", at(knee), at(ankle), 0.12 * t * (0.9 + 0.2 * bu), Some(0.085 * t * (0.9 + 0.2 * bu)), Some(k)),
                capsule(
                    "calf",
                    at(knee + (ankle - knee) * 0.1 - fwd * 0.08 * t),
                    at(knee + (ankle - knee) * 0.5 - fwd * 0.05 * t),
                    0.16 * t * mm,
                    Some(0.1 * t * mm),
                    Some(k),
                ),
            ];
            // Foot or boot, turned out along the toe joint.
            let dir = Vec3::new(toe.x - ankle.x, 0.0, toe.z - ankle.z).normalize_or(Vec3::Z);
            let yaw = num(dir.x.atan2(dir.z) * 180.0 / PI);
            let len = b.foot;
            let heel_z = -0.22 * len;
            let mid = |along: f32, y: f32| Vec3::new(ankle.x, y, ankle.z) + dir * (heel_z + along * len);
            if race.boots {
                let sole = 0.09 * t;
                let wide = len * 0.42;
                let vamp = ankle.y * 0.95 + 0.05 * t;
                let mut s1 = boxed("sole", at(mid(0.5, sole * 0.5)), Vec3::new(wide, sole, len * 0.97), sole * 0.45, Some(0.02 * t));
                s1["rotate"] = json!([0, yaw, 0]);
                let mut s2 = boxed(
                    "vamp",
                    at(mid(0.48, sole + vamp * 0.5 - 0.01 * t)),
                    Vec3::new(wide * 0.96, vamp, len * 0.93),
                    (vamp.min(wide) * 0.45).min(0.2 * t),
                    Some(0.025 * t),
                );
                s2["rotate"] = json!([0, yaw, 0]);
                leg.push(s1);
                leg.push(s2);
                leg.push(ellipsoid("toe_cap", at(mid(0.78, sole + vamp * 0.32)), Vec3::new(wide * 0.48, vamp * 0.4, len * 0.22), Some(0.03 * t)));
                leg.push(capsule(
                    "boot_shaft",
                    at(ankle - Vec3::Y * 0.12 * t),
                    at(ankle + (knee - ankle) * 0.5),
                    0.19 * t * (1.0 + 0.2 * bu),
                    Some(0.2 * t * (1.0 + 0.2 * bu)),
                    Some(0.03 * t),
                ));
                leg.push(capsule(
                    "boot_cuff",
                    at(ankle + (knee - ankle) * 0.5),
                    at(ankle + (knee - ankle) * 0.55),
                    0.22 * t * (1.0 + 0.2 * bu),
                    None,
                    Some(0.012 * t),
                ));
            } else {
                let wide = len * 0.38;
                let tall = ankle.y * 0.9;
                let mut f = boxed(
                    "foot",
                    at(mid(0.45, tall * 0.45)),
                    Vec3::new(wide, tall * 0.9, len * 0.75),
                    (tall.min(wide) * 0.45).min(0.2 * t),
                    Some(0.05 * t),
                );
                f["rotate"] = json!([0, yaw, 0]);
                leg.push(f);
                leg.push(ellipsoid("toes", at(mid(0.8, tall * 0.3)), Vec3::new(wide * 0.5, tall * 0.32, len * 0.2), Some(0.04 * t)));
                leg.push(ellipsoid("heel", at(mid(0.12, tall * 0.4)), Vec3::new(wide * 0.4, tall * 0.42, len * 0.16), Some(0.04 * t)));
            }
            parts.push((
                n("leg"),
                json!({ "sdf": leg, "anchor": anchor_root.clone(), "join": "blend", "into": "torso", "blend": num(0.08 * t), "tags": ["anatomy"] }),
            ));
        }

        // ---- head and face ----------------------------------------------------------------
        let mut head_params = serde_json::Map::new();
        head_params.insert("height".into(), num(h));
        for (k, v) in race.head {
            head_params.insert((*k).into(), json!(v));
        }
        parts.push((
            "head".into(),
            json!({
                "feature": "head_base",
                "params": head_params,
                "anchor": { "on": SELF, "landmark": "head", "orient": "normal" },
                "detail": num(h / 85.0),
                "tags": ["anatomy"],
            }),
        ));
        let mut nose = serde_json::Map::new();
        for (k, v) in race.nose {
            let lengths = matches!(*k, "length" | "width");
            nose.insert((*k).into(), if lengths { num(*v as f32 * h) } else { json!(v) });
        }
        if !nose.contains_key("length") {
            nose.insert("length".into(), num(0.19 * h));
            nose.insert("width".into(), num(0.145 * h));
        }
        parts.push(("nose".into(), json!({ "feature": "nose", "params": nose, "anchor": { "on": "head", "landmark": "nose_root" }, "join": "blend", "blend": num(0.015 * h) })));
        let (variant, ear_len, point, flare) = race.ear;
        for (side, word) in [("l", "left"), ("r", "right")] {
            parts.push((
                format!("ear_{side}"),
                json!({
                    "feature": "ear",
                    "params": { "variant": variant, "length": num(ear_len * h), "pointiness": point, "flare": flare, "thickness": num(0.012 * h), "side": word },
                    "anchor": { "on": "head", "landmark": format!("ear_root_{side}") },
                }),
            ));
            parts.push((
                format!("eye_{side}"),
                json!({ "feature": "eye", "params": { "radius": num(0.046 * h) }, "anchor": { "on": "head", "landmark": format!("eye_{side}"), "offset": [0, 0, num(-0.03 * h)] } }),
            ));
            parts.push((
                format!("brow_{side}"),
                json!({
                    "strands": "eyebrows",
                    "params": {
                        "on": "head",
                        "region": { "around": format!("brow_{side}"), "radius": [num(0.11 * h), num(0.045 * h), num(0.1 * h)] },
                        "length": num(race.brows * h),
                        "clumps": 14,
                        "width": num(0.035 * h),
                    },
                }),
            ));
        }
        match p.choice("beard") {
            "none" => {}
            style => {
                let len = if style == "short" { 0.3 } else { 1.05 };
                let mut beard = json!({
                    "on": "head",
                    "region": { "groups": ["jaw", "chin", "muzzle"] },
                    "length": num(len * h),
                    "clumps": 30,
                    "clump_layers": 2,
                    "width": num(0.26 * h),
                    "root_width": 0.5,
                    "bulge": 0.4,
                    "tip_width": 0.1,
                    "flatness": 0.6,
                    "gravity": 0.9,
                    "length_variation": 0.3,
                    "curl": 0.35,
                    "curl_turns": 0.8,
                    "noise": 0.25,
                    "volume": num(0.05 * h),
                    "sides": 8,
                    "collide": ["torso"],
                });
                if style == "braided" {
                    beard["braids"] = json!(2);
                    beard["braid_length"] = num(1.6 * h);
                    beard["braid_width"] = num(0.12 * h);
                    beard["beads"] = json!(2);
                    beard["tie"] = json!("bead");
                }
                parts.push(("beard".into(), json!({ "strands": "beard", "params": beard })));
            }
        }
        match p.choice("hair") {
            "none" => {}
            style => {
                let len = if style == "short" { 0.25 } else { 0.9 };
                parts.push((
                    "hair".into(),
                    json!({
                        "strands": "hair",
                        "params": {
                            "on": "head",
                            "region": ["hairline_front", "temple_l", "nape", "temple_r"],
                            "output": "clumps",
                            "style": "swept_back",
                            "gravity": if style == "short" { 0.6 } else { 0.85 },
                            "length": num(len * h),
                            "clumps": 60,
                            "clump_layers": 2,
                            "width": num(0.14 * h),
                            "root_width": 0.6,
                            "bulge": 0.3,
                            "tip_width": 0.15,
                            "flatness": 0.35,
                            "volume": num(0.05 * h),
                            "sides": 8,
                            "collide": ["torso"],
                        },
                    }),
                ));
            }
        }
        if race.tusks {
            for (s, side) in [(1.0f32, "l"), (-1.0, "r")] {
                parts.push((
                    format!("tusk_{side}"),
                    json!({
                        "feature": "horn",
                        "params": { "length": num(0.16 * h), "radius": num(0.028 * h), "tip": 0.25, "curl": -50, "ridges": 0, "sides": 10 },
                        "anchor": { "on": "head", "landmark": "mouth", "offset": [num(s * 0.1 * h), num(-0.06 * h), num(-0.03 * h)] },
                        "transform": { "rotate": [-15, 0, num(-s * 12.0)] },
                    }),
                ));
            }
        }
        let curves = vec![
            ("spine".to_owned(), ["pelvis", "spine", "chest", "neck", "head", "head_top"].map(str::to_owned).to_vec()),
            ("arm_l".to_owned(), ["shoulder_l", "elbow_l", "wrist_l", "grip_l"].map(str::to_owned).to_vec()),
            ("arm_r".to_owned(), ["shoulder_r", "elbow_r", "wrist_r", "grip_r"].map(str::to_owned).to_vec()),
            ("leg_l".to_owned(), ["hip_l", "knee_l", "ankle_l", "toe_l"].map(str::to_owned).to_vec()),
            ("leg_r".to_owned(), ["hip_r", "knee_r", "ankle_r", "toe_r"].map(str::to_owned).to_vec()),
        ];
        Generated { curves, parts, dna: json!({ "archetype": race.id, "culture": race.culture }) }
    }
}

fn smooth(mut v: Value, k: Option<f32>) -> Value {
    if let Some(k) = k {
        v["smooth_union"] = num(k);
    }
    v
}

fn ellipsoid(name: &str, at: Value, radii: Vec3, k: Option<f32>) -> Value {
    smooth(json!({ "ellipsoid": { "at": at, "radii": vec(radii) }, "name": name }), k)
}

fn sphere(name: &str, at: Value, r: f32, k: Option<f32>) -> Value {
    smooth(json!({ "sphere": { "at": at, "radius": num(r) }, "name": name }), k)
}

fn capsule(name: &str, a: Value, b: Value, r: f32, rb: Option<f32>, k: Option<f32>) -> Value {
    let mut c = json!({ "a": a, "b": b, "radius": num(r) });
    if let Some(rb) = rb {
        c["radius_b"] = num(rb);
    }
    smooth(json!({ "capsule": c, "name": name }), k)
}

fn boxed(name: &str, at: Value, size: Vec3, corner: f32, k: Option<f32>) -> Value {
    smooth(json!({ "box": { "at": at, "size": vec(size), "corner": num(corner) }, "name": name }), k)
}
