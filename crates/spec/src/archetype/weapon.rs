//! Weapon archetypes: `sword`, `dagger`, `axe`, `hammer` and `spear`. The
//! skeleton is the weapon's centerline along +Y with the edge or face
//! toward +X and the flats facing +-Z: `pommel`, `grip`, `guard`,
//! `blade_base` and `tip` for blades; `butt`, `grip`, `grip_upper`,
//! `head`, `top` and `edge` (or `face`, or `tip`) for hafted weapons.
//! The root joint is `grip`, so `at: "dwarf.grip_r"` puts the weapon in a
//! fist. Styles `dwarven` (broad, thick, angular, faceted), `human` and
//! `elven` (slim, leaf shaped, curved) change shapes and proportions.

use glam::{Vec2, Vec3};
use p3d_features::{ParamInfo, ParamKind, Params};
use serde_json::{Value, json};

use super::{Archetype, ArchetypeInfo, Generated, Rig, SELF, num, vec};

const STYLES: &[&str] = &["dwarven", "human", "elven"];

/// What a weapon is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Sword,
    Dagger,
    Axe,
    Hammer,
    Spear,
}

const fn length(default: f64, max: f64) -> ParamInfo {
    ParamInfo { name: "length", doc: "Overall length, end to end.", kind: ParamKind::Length, default, min: 0.05, max }
}

const fn style(default: f64) -> ParamInfo {
    ParamInfo {
        name: "style",
        doc: "dwarven (broad, thick, angular), human or elven (slim, leaf shaped, curved).",
        kind: ParamKind::Choice(STYLES),
        default,
        min: 0.0,
        max: 2.0,
    }
}

const WIDTH: ParamInfo =
    ParamInfo { name: "width", doc: "Blade or head width relative to the style's.", kind: ParamKind::Ratio, default: 1.0, min: 0.5, max: 2.0 };
const CURVE: ParamInfo = ParamInfo {
    name: "curve",
    doc: "Sweep of the blade (or haft) toward its edge, degrees at the tip (a gesture; 0 straight).",
    kind: ParamKind::Angle,
    default: 0.0,
    min: -40.0,
    max: 40.0,
};
const DETAIL: ParamInfo = ParamInfo {
    name: "detail",
    doc: "Voxel size of outline-built heads; 0.4% of the length when 0.",
    kind: ParamKind::Length,
    default: 0.0,
    min: 0.0,
    max: 0.05,
};

const SWORD: &[ParamInfo] = &[
    length(0.95, 3.0),
    ParamInfo { name: "blade", doc: "Blade length as a share of the whole.", kind: ParamKind::Ratio, default: 0.74, min: 0.4, max: 0.9 },
    ParamInfo { name: "grip", doc: "Grip length as a share of the whole.", kind: ParamKind::Ratio, default: 0.17, min: 0.06, max: 0.4 },
    WIDTH,
    CURVE,
    style(1.0),
];
const DAGGER: &[ParamInfo] = &[
    length(0.38, 1.0),
    ParamInfo { name: "blade", doc: "Blade length as a share of the whole.", kind: ParamKind::Ratio, default: 0.62, min: 0.4, max: 0.85 },
    ParamInfo { name: "grip", doc: "Grip length as a share of the whole.", kind: ParamKind::Ratio, default: 0.27, min: 0.1, max: 0.45 },
    WIDTH,
    CURVE,
    style(1.0),
];
const AXE: &[ParamInfo] = &[
    length(0.9, 3.0),
    ParamInfo {
        name: "head",
        doc: "Height of the bit at its edge as a share of the length.",
        kind: ParamKind::Ratio,
        default: 0.3,
        min: 0.1,
        max: 0.6,
    },
    ParamInfo {
        name: "grip",
        doc: "Where the main hand holds, as a share of the length from the butt.",
        kind: ParamKind::Ratio,
        default: 0.35,
        min: 0.05,
        max: 0.9,
    },
    ParamInfo {
        name: "bits",
        doc: "Blades: 1 (with a poll behind) or 2 (a double-bit axe).",
        kind: ParamKind::Count,
        default: 1.0,
        min: 1.0,
        max: 2.0,
    },
    WIDTH,
    CURVE,
    style(0.0),
    DETAIL,
];
const HAMMER: &[ParamInfo] = &[
    length(0.8, 3.0),
    ParamInfo {
        name: "head",
        doc: "Length of the head (across the haft) as a share of the length.",
        kind: ParamKind::Ratio,
        default: 0.26,
        min: 0.1,
        max: 0.6,
    },
    ParamInfo {
        name: "grip",
        doc: "Where the main hand holds, as a share of the length from the butt.",
        kind: ParamKind::Ratio,
        default: 0.3,
        min: 0.05,
        max: 0.9,
    },
    WIDTH,
    style(0.0),
];
const SPEAR: &[ParamInfo] = &[
    length(2.1, 5.0),
    ParamInfo { name: "head", doc: "Spearhead length as a share of the length.", kind: ParamKind::Ratio, default: 0.15, min: 0.05, max: 0.4 },
    ParamInfo {
        name: "grip",
        doc: "Where the main hand holds, as a share of the length from the butt.",
        kind: ParamKind::Ratio,
        default: 0.45,
        min: 0.05,
        max: 0.9,
    },
    WIDTH,
    CURVE,
    style(1.0),
];

const BLADE_JOINTS: &[(&str, &str)] = &[
    ("pommel", "the bottom of the pommel"),
    ("grip", "the middle of the grip (the root: `at` puts it in a fist)"),
    ("guard", "the guard's center"),
    ("blade_base", "where the blade leaves the guard"),
    ("tip", "the point"),
];
const HAFT_JOINTS: &[(&str, &str)] = &[
    ("butt", "the bottom end"),
    ("grip", "where the main hand holds (the root: `at` puts it in a fist)"),
    ("grip_upper", "where a second hand holds"),
    ("head", "the head's center on the haft"),
    ("top", "the top end"),
    ("edge", "the middle of the cutting edge (axe), the striking face (hammer) or the point (spear)"),
];

struct Weapon {
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    aliases: &'static [&'static str],
    family: Family,
    params: &'static [ParamInfo],
}

const WEAPONS: [Weapon; 5] = [
    Weapon {
        id: "sword",
        label: "Sword",
        summary: "A sword on its centerline: a lofted blade with a central ridge (straight, leaf shaped or curved), a guard, a wrapped grip and a pommel.",
        aliases: &["longsword", "blade", "sabre", "saber", "broadsword"],
        family: Family::Sword,
        params: SWORD,
    },
    Weapon {
        id: "dagger",
        label: "Dagger",
        summary: "A dagger or knife: a short ridged blade, a small guard, a wrapped grip and a pommel.",
        aliases: &["knife", "dirk", "short blade"],
        family: Family::Dagger,
        params: DAGGER,
    },
    Weapon {
        id: "axe",
        label: "Axe",
        summary: "A hafted axe: an octagonal haft, a bit (or two) built from a crescent outline that thins to an edge, a socket block, a top spike, collar bands and a pommel.",
        aliases: &["battle axe", "war axe", "hatchet", "double axe"],
        family: Family::Axe,
        params: AXE,
    },
    Weapon {
        id: "hammer",
        label: "Hammer",
        summary: "A war hammer: a haft, a bevelled block head with flared faces, a top spike and a pommel.",
        aliases: &["war hammer", "maul", "mace"],
        family: Family::Hammer,
        params: HAMMER,
    },
    Weapon {
        id: "spear",
        label: "Spear",
        summary: "A spear: a long shaft, a leaf-shaped ridged spearhead on a socket, and a butt spike.",
        aliases: &["lance", "pike", "polearm", "javelin"],
        family: Family::Spear,
        params: SPEAR,
    },
];

const BLADE_PARTS: &[(&str, &str)] =
    &[("blade", "lofted blade with a ridge"), ("guard", "crossguard"), ("grip", "wrapped grip (lathe)"), ("pommel", "pommel (lathe)")];
const AXE_PARTS: &[(&str, &str)] = &[
    ("haft", "octagonal haft (lathe)"),
    ("blade", "the bit or bits (silhouettes)"),
    ("socket", "the block around the haft"),
    ("spike", "top spike"),
    ("band_upper, band_lower", "collar bands"),
    ("pommel", "butt cap"),
];
const HAMMER_PARTS: &[(&str, &str)] = &[
    ("haft", "haft (lathe)"),
    ("hammer_head", "bevelled block"),
    ("face_l, face_r", "striking faces"),
    ("spike", "top spike"),
    ("pommel", "butt cap"),
];
const SPEAR_PARTS: &[(&str, &str)] =
    &[("shaft", "shaft (lathe)"), ("spearhead", "lofted leaf blade"), ("socket", "the cone holding it"), ("butt", "butt spike")];

/// Every weapon archetype.
pub(super) fn all() -> Vec<Box<dyn Archetype>> {
    WEAPONS.iter().map(|w| Box::new(Arch(w)) as Box<dyn Archetype>).collect()
}

struct Arch(&'static Weapon);

/// Style factors: blade width, thickness, and whether shapes are faceted.
fn style_of(p: &Params) -> (&'static str, f32, f32) {
    match p.choice("style") {
        "dwarven" => ("dwarven", 1.25, 1.3),
        "elven" => ("elven", 0.82, 0.85),
        _ => ("human", 1.0, 1.0),
    }
}

/// A lathe part's shape: profile points `[radius, height]`.
fn lathe(profile: &[(f32, f32)], segments: u32) -> Value {
    json!({ "lathe": { "profile": profile.iter().map(|(r, y)| json!([num(*r), num(*y)])).collect::<Vec<_>>(), "segments": segments } })
}

/// A part anchored at a joint of the instance, in the joint's frame.
fn on_joint(mut shape: Value, joint: &str, offset: Vec3) -> Value {
    shape["anchor"] = json!({ "on": SELF, "landmark": joint, "orient": "normal", "offset": vec(offset) });
    shape
}

/// Six-point lens sections of a blade along +Y: half widths and half
/// thicknesses, offsets toward +X for a curve.
fn blade_loft(rows: &[(f32, f32, f32, f32)]) -> Value {
    let sections: Vec<Value> = rows
        .iter()
        .map(|&(y, w, th, dx)| {
            let w = w.max(0.0004);
            let th = th.max(0.0002);
            json!([
                [num(dx - w), num(y), 0],
                [num(dx - 0.42 * w), num(y), num(-th)],
                [num(dx + 0.42 * w), num(y), num(-th)],
                [num(dx + w), num(y), 0],
                [num(dx + 0.42 * w), num(y), num(th)],
                [num(dx - 0.42 * w), num(y), num(th)]
            ])
        })
        .collect();
    json!({ "loft": { "sections": sections, "smooth": true, "steps": 3 } })
}

impl Arch {
    fn blade_weapon(&self, p: &Params, rig: &Rig) -> Vec<(String, Value)> {
        let l = p.f("length");
        let (style, wf, tf) = style_of(p);
        let base = rig.at("blade_base");
        let blade_len = rig.at("tip").y - base.y;
        let w = 0.05 * l * p.f("width") * wf * if self.0.family == Family::Dagger { 1.3 } else { 1.0 };
        let thick = 0.0042 * l * tf * if self.0.family == Family::Dagger { 1.5 } else { 1.0 };
        let sweep = p.f("curve").tan() * blade_len * 0.5;
        let n = 12;
        let rows: Vec<(f32, f32, f32, f32)> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let half = match style {
                    // Broad, parallel edges, then a short clipped point.
                    "dwarven" => {
                        if t < 0.86 {
                            0.5 * w * (1.0 - 0.06 * t)
                        } else {
                            0.5 * w * 0.95 * (1.0 - (t - 0.86) / 0.14 * 0.85)
                        }
                    }
                    // A leaf: widest past the middle, a long point.
                    "elven" => 0.5 * w * (0.75 + 0.35 * (std::f32::consts::PI * t * 0.95).sin()) * ((1.0 - t) / 0.25).min(1.0).sqrt(),
                    _ => 0.5 * w * (1.0 - 0.3 * t) * ((1.0 - t) / 0.18).min(1.0).sqrt(),
                };
                let th = thick * (1.0 - 0.55 * t) * if i == n { 0.2 } else { 1.0 };
                (t * blade_len, half, th, sweep * t * t)
            })
            .collect();
        let grip_len = rig.at("guard").y - rig.at("pommel").y;
        let guard_y = rig.at("guard").y;
        let gh = rig.at("blade_base").y - guard_y;
        let rg = 0.0145 * l * if style == "dwarven" { 1.2 } else { 1.0 } * if self.0.family == Family::Dagger { 1.6 } else { 1.0 };
        let mut parts = vec![("blade".to_owned(), on_joint(blade_loft(&rows), "blade_base", Vec3::ZERO))];
        let guard = match style {
            "dwarven" => {
                json!({ "primitive": { "box": { "width": num(w * 3.2), "height": num(gh * 2.0), "depth": num(w * 1.1), "origin": "center", "bevel": { "width": num(gh * 0.35), "segments": 1 } } } })
            }
            "elven" => {
                // A swept guard: wings curving up toward the blade.
                let s = w * 2.4;
                let outline: Vec<Value> = [
                    (-s, 0.35),
                    (-s * 0.75, 0.0),
                    (-s * 0.35, -0.3),
                    (0.0, -0.45),
                    (s * 0.35, -0.3),
                    (s * 0.75, 0.0),
                    (s, 0.35),
                    (s * 0.85, 0.6),
                    (s * 0.5, 0.35),
                    (0.0, 0.3),
                    (-s * 0.5, 0.35),
                    (-s * 0.85, 0.6),
                ]
                .iter()
                .map(|(x, y)| json!([num(*x), num(y * gh * 2.5)]))
                .collect();
                json!({ "extrude": { "outline": outline, "depth": num(w * 0.6), "bevel": { "width": num(gh * 0.25), "segments": 2, "sides": "both" } } })
            }
            _ => {
                json!({ "primitive": { "box": { "width": num(w * 4.6), "height": num(gh * 1.6), "depth": num(w * 0.8), "origin": "center", "bevel": { "width": num(gh * 0.3), "segments": 1 } } } })
            }
        };
        let guard_offset = if style == "elven" { Vec3::new(0.0, 0.0, -w * 0.3) } else { Vec3::ZERO };
        parts.push(("guard".into(), on_joint(guard, "guard", guard_offset)));
        // A wrapped grip: rings of the wrap along a slightly barrel profile.
        let grip_h = grip_len * 0.82;
        let rings = 7;
        let mut prof = vec![(rg * 0.92, 0.0)];
        for i in 0..rings {
            let y0 = grip_h * (i as f32 + 0.15) / rings as f32;
            let y1 = grip_h * (i as f32 + 0.85) / rings as f32;
            let bulge = 1.0 + 0.06 * (std::f32::consts::PI * (i as f32 + 0.5) / rings as f32).sin();
            prof.push((rg * bulge, y0));
            prof.push((rg * bulge, y1));
            prof.push((rg * bulge * 0.94, (y1 + grip_h * (i as f32 + 1.15) / rings as f32) * 0.5));
        }
        prof.pop();
        prof.push((rg * 0.92, grip_h));
        let grip_from = rig.at("guard").y - grip_h;
        parts.push(("grip".into(), on_joint(lathe(&prof, 12), "grip", Vec3::new(0.0, grip_from, 0.0))));
        // The pommel under the grip.
        let pm = rig.at("guard").y - grip_h - rig.at("pommel").y;
        let r = rg * 2.1;
        let (profile, segments): (Vec<(f32, f32)>, u32) = match style {
            "dwarven" => (vec![(0.0, 0.0), (r * 0.8, 0.0), (r, pm * 0.2), (r, pm * 0.65), (rg * 1.2, pm * 0.9), (rg * 0.9, pm)], 8),
            "elven" => (vec![(0.0, 0.0), (r * 0.45, pm * 0.12), (r * 0.8, pm * 0.45), (rg * 1.0, pm * 0.85), (rg * 0.9, pm)], 12),
            _ => (vec![(0.0, 0.0), (r * 0.7, 0.0), (r, pm * 0.3), (r, pm * 0.6), (r * 0.6, pm * 0.85), (rg * 0.9, pm)], 16),
        };
        parts.push(("pommel".into(), on_joint(lathe(&profile, segments), "pommel", Vec3::ZERO)));
        parts
    }

    fn axe(&self, p: &Params, rig: &Rig) -> Vec<(String, Value)> {
        let l = p.f("length");
        let (style, wf, _) = style_of(p);
        let haft_len = rig.at("top").y - rig.at("butt").y;
        let rh = 0.0185 * l * if style == "dwarven" { 1.1 } else { 0.95 };
        let segs = if style == "dwarven" { 8 } else { 12 };
        let mut parts =
            vec![("haft".to_owned(), on_joint(lathe(&[(rh, 0.0), (rh * 1.06, haft_len * 0.45), (rh * 0.98, haft_len)], segs), "butt", Vec3::ZERO))];
        // The bit: a crescent outline from the eye to the edge, a lens
        // from the top so it thins to the edge.
        let reach = 0.21 * l * p.f("width") * wf;
        let tall = p.f("head") * l;
        let eye = 0.05 * l;
        let mut right: Vec<Vec2> = Vec::new();
        let quad = |a: Vec2, c: Vec2, b: Vec2, t: f32| a * (1.0 - t) * (1.0 - t) + c * 2.0 * t * (1.0 - t) + b * t * t;
        let edge_top = Vec2::new(reach, tall * 0.5);
        let edge_bot = Vec2::new(reach * if style == "dwarven" { 0.9 } else { 1.0 }, -tall * if style == "dwarven" { 0.62 } else { 0.5 });
        // Upper line: from the eye, concave, up to the top horn.
        for i in 0..=8 {
            right.push(quad(Vec2::new(rh * 1.6, eye * 0.85), Vec2::new(reach * 0.55, eye * 0.9), edge_top, i as f32 / 8.0));
        }
        // The edge: a convex crescent.
        let bulge = Vec2::new(reach + tall * 0.18, (edge_top.y + edge_bot.y) * 0.5);
        for i in 1..=10 {
            right.push(quad(edge_top, bulge, edge_bot, i as f32 / 10.0));
        }
        // Lower line back to the eye (a bearded hook for dwarves).
        let hook = if style == "dwarven" { Vec2::new(reach * 0.45, -eye * 2.2) } else { Vec2::new(reach * 0.5, -eye * 0.95) };
        for i in 1..=8 {
            right.push(quad(edge_bot, hook, Vec2::new(rh * 1.6, -eye * 0.85), i as f32 / 8.0));
        }
        let mut outline: Vec<Vec2> = right.clone();
        if p.count("bits") >= 2 {
            outline.extend(right.iter().rev().map(|v| Vec2::new(-v.x, v.y)));
        } else {
            // A square poll behind the haft.
            let poll = rh * 3.2;
            outline.extend([
                Vec2::new(-poll * 0.6, -eye * 0.8),
                Vec2::new(-poll, -eye * 0.55),
                Vec2::new(-poll, eye * 0.55),
                Vec2::new(-poll * 0.6, eye * 0.8),
            ]);
        }
        // The top view: thick at the eye, thin at the edge (both edges for
        // two bits, a square poll for one).
        let thick = rh * 1.35;
        let edge = (reach + tall * 0.2, 0.0015 * l);
        let lens: Vec<(f32, f32)> = if p.count("bits") >= 2 {
            vec![(-edge.0, edge.1), (-reach * 0.6, thick * 0.45), (-rh * 1.5, thick), (rh * 1.5, thick), (reach * 0.6, thick * 0.45), edge]
        } else {
            vec![(-rh * 3.2, thick * 0.9), (-rh * 1.5, thick), (rh * 1.5, thick), (reach * 0.6, thick * 0.45), edge]
        };
        let top_lens: Vec<Value> =
            lens.iter().map(|&(x, z)| json!([num(x), num(z)])).chain(lens.iter().rev().map(|&(x, z)| json!([num(x), num(-z)]))).collect();
        let detail = if p.f("detail") > 0.0 { p.f("detail") } else { 0.004 * l };
        let blade = json!({ "silhouettes": {
            "front": outline.iter().map(|v| json!([num(v.x), num(v.y)])).collect::<Vec<_>>(),
            "top": top_lens,
            "round": num(0.0015 * l),
            "detail": num(detail),
        } });
        parts.push(("blade".into(), on_joint(blade, "head", Vec3::ZERO)));
        let bevel = if style == "dwarven" { 0.006 * l } else { 0.004 * l };
        parts.push((
            "socket".into(),
            on_joint(
                json!({ "primitive": { "box": { "width": num(rh * 4.0), "height": num(eye * 2.4), "depth": num(rh * 2.9), "origin": "center", "bevel": { "width": num(bevel), "segments": 1 } } } }),
                "head",
                Vec3::ZERO,
            ),
        ));
        let spike: Vec<(f32, f32)> = vec![(rh * 1.1, 0.0), (rh * 1.2, 0.012 * l), (rh * 0.7, 0.035 * l), (0.0, 0.085 * l)];
        parts.push(("spike".into(), on_joint(lathe(&spike, segs), "top", Vec3::new(0.0, -0.004 * l, 0.0))));
        let band = |h: f32| lathe(&[(rh * 0.98, 0.0), (rh * 1.28, h * 0.12), (rh * 1.28, h * 0.88), (rh * 0.98, h)], segs);
        parts.push(("band_upper".into(), on_joint(band(0.018 * l), "head", Vec3::new(0.0, -eye * 1.2 - 0.03 * l, 0.0))));
        parts.push(("band_lower".into(), on_joint(band(0.011 * l), "head", Vec3::new(0.0, -eye * 1.2 - 0.06 * l, 0.0))));
        let pm = 0.07 * l;
        let pommel: Vec<(f32, f32)> =
            vec![(0.0, -pm), (rh * 0.95, -pm * 0.94), (rh * 1.55, -pm * 0.64), (rh * 1.45, -pm * 0.2), (rh * 1.05, 0.0), (rh, pm * 0.07)];
        parts.push(("pommel".into(), on_joint(lathe(&pommel, segs), "butt", Vec3::ZERO)));
        parts
    }

    fn hammer(&self, p: &Params, rig: &Rig) -> Vec<(String, Value)> {
        let l = p.f("length");
        let (style, wf, _) = style_of(p);
        let rh = 0.02 * l;
        let haft_len = rig.at("top").y - rig.at("butt").y;
        let segs = if style == "dwarven" { 8 } else { 12 };
        let across = p.f("head") * l * wf.sqrt() * p.f("width").sqrt();
        let tall = across * 0.5;
        let deep = across * 0.44;
        let mut parts =
            vec![("haft".to_owned(), on_joint(lathe(&[(rh, 0.0), (rh * 1.05, haft_len * 0.5), (rh * 0.97, haft_len)], segs), "butt", Vec3::ZERO))];
        let bevel = tall * 0.12;
        parts.push((
            "hammer_head".into(),
            on_joint(
                json!({ "primitive": { "box": { "width": num(across * 0.8), "height": num(tall), "depth": num(deep), "origin": "center", "bevel": { "width": num(bevel), "segments": 2 } } } }),
                "head",
                Vec3::ZERO,
            ),
        ));
        for (side, s) in [("l", -1.0f32), ("r", 1.0)] {
            parts.push((
                format!("face_{side}"),
                on_joint(
                    json!({ "primitive": { "box": { "width": num(across * 0.14), "height": num(tall * 1.18), "depth": num(deep * 1.15), "origin": "center", "bevel": { "width": num(bevel * 0.8), "segments": 1 } } } }),
                    "head",
                    Vec3::new(s * across * 0.43, 0.0, 0.0),
                ),
            ));
        }
        let spike: Vec<(f32, f32)> = vec![(rh * 1.1, 0.0), (rh * 1.25, 0.01 * l), (rh * 0.6, 0.03 * l), (0.0, 0.07 * l)];
        parts.push(("spike".into(), on_joint(lathe(&spike, segs), "top", Vec3::new(0.0, -0.004 * l, 0.0))));
        let pm = 0.06 * l;
        let pommel: Vec<(f32, f32)> =
            vec![(0.0, -pm), (rh * 1.0, -pm * 0.94), (rh * 1.6, -pm * 0.6), (rh * 1.5, -pm * 0.2), (rh * 1.05, 0.0), (rh, pm * 0.07)];
        parts.push(("pommel".into(), on_joint(lathe(&pommel, segs), "butt", Vec3::ZERO)));
        parts
    }

    fn spear(&self, p: &Params, rig: &Rig) -> Vec<(String, Value)> {
        let l = p.f("length");
        let (style, wf, tf) = style_of(p);
        let rh = 0.0075 * l * if style == "dwarven" { 1.25 } else { 1.0 };
        let shaft_len = rig.at("head").y - rig.at("butt").y;
        let mut parts = vec![(
            "shaft".to_owned(),
            on_joint(lathe(&[(rh * 0.95, 0.0), (rh * 1.04, shaft_len * 0.4), (rh * 0.95, shaft_len)], 12), "butt", Vec3::ZERO),
        )];
        let head_len = rig.at("tip").y - rig.at("head").y;
        let w = 0.032 * l * p.f("width") * wf;
        let th = 0.004 * l * tf;
        let sweep = p.f("curve").tan() * head_len * 0.5;
        let n = 10;
        let rows: Vec<(f32, f32, f32, f32)> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let half =
                    0.5 * w * (0.35 + 0.9 * (std::f32::consts::PI * (0.15 + 0.85 * t) * 0.92).sin().max(0.0)) * ((1.0 - t) / 0.3).min(1.0).sqrt();
                (t * head_len, half.max(rh * 0.5 * (1.0 - t)), th * (1.0 - 0.5 * t) * if i == n { 0.2 } else { 1.0 }, sweep * t * t)
            })
            .collect();
        parts.push(("spearhead".into(), on_joint(blade_loft(&rows), "head", Vec3::ZERO)));
        let sock = 0.06 * l;
        parts.push((
            "socket".into(),
            on_joint(
                lathe(&[(rh * 1.25, 0.0), (rh * 1.3, sock * 0.15), (rh * 0.85, sock), (rh * 0.5, sock * 1.05)], 12),
                "head",
                Vec3::new(0.0, -sock * 0.95, 0.0),
            ),
        ));
        let butt: Vec<(f32, f32)> = vec![(0.0, -0.04 * l), (rh * 0.7, -0.025 * l), (rh * 1.15, 0.0), (rh, 0.01 * l)];
        parts.push(("butt".into(), on_joint(lathe(&butt, 12), "butt", Vec3::ZERO)));
        parts
    }
}

impl Archetype for Arch {
    fn info(&self) -> ArchetypeInfo {
        let w = self.0;
        let (joints, parts) = match w.family {
            Family::Sword | Family::Dagger => (BLADE_JOINTS, BLADE_PARTS),
            Family::Axe => (HAFT_JOINTS, AXE_PARTS),
            Family::Hammer => (HAFT_JOINTS, HAMMER_PARTS),
            Family::Spear => (HAFT_JOINTS, SPEAR_PARTS),
        };
        ArchetypeInfo { id: w.id, label: w.label, summary: w.summary, params: w.params, joints, parts, root: "grip", aliases: w.aliases }
    }

    fn joints(&self, p: &Params) -> Rig {
        let l = p.f("length");
        let mut rig = Rig::default();
        match self.0.family {
            Family::Sword | Family::Dagger => {
                let blade = p.f("blade") * l;
                let grip = p.f("grip") * l;
                let pommel = (l - blade - grip) * 0.6;
                let guard = (l - blade - grip) * 0.4;
                let sweep = p.f("curve").tan() * blade * 0.5;
                rig.add("pommel", Vec3::new(0.0, -grip * 0.5 - pommel, 0.0), None);
                rig.add("grip", Vec3::ZERO, Some("pommel"));
                rig.add("guard", Vec3::new(0.0, grip * 0.5 + guard * 0.5, 0.0), Some("grip"));
                rig.add("blade_base", Vec3::new(0.0, grip * 0.5 + guard, 0.0), Some("guard"));
                rig.add("tip", Vec3::new(sweep, grip * 0.5 + guard + blade, 0.0), Some("blade_base"));
            }
            family => {
                let g = p.f("grip");
                // The hammer head sits under the spike at the top.
                let hammer_tall = p.f("head") * l * style_of(p).1.sqrt() * p.f("width").sqrt() * 0.5;
                let head = match family {
                    Family::Spear => l * (1.0 - p.f("head")),
                    Family::Hammer => l * 0.93 - hammer_tall * 0.5,
                    _ => l * 0.86,
                };
                rig.add("butt", Vec3::new(0.0, -g * l, 0.0), None);
                rig.add("grip", Vec3::ZERO, Some("butt"));
                rig.add("grip_upper", Vec3::new(0.0, ((1.0 - g) * l * 0.45).min(head - g * l - 0.1 * l).max(0.0), 0.0), Some("grip"));
                rig.add("head", Vec3::new(0.0, head - g * l, 0.0), Some("grip_upper"));
                let top = match family {
                    Family::Spear => head - g * l,
                    Family::Hammer => head - g * l + hammer_tall * 0.5,
                    _ => (1.0 - g) * l - 0.085 * l,
                };
                rig.add("top", Vec3::new(0.0, top, 0.0), Some("head"));
                let edge = match family {
                    Family::Axe => Vec3::new(0.21 * l * p.f("width") * style_of(p).1, head - g * l, 0.0),
                    Family::Hammer => Vec3::new(p.f("head") * l * 0.5, head - g * l, 0.0),
                    _ => Vec3::new(p.f("curve").tan() * p.f("head") * l * 0.5, (1.0 - g) * l, 0.0),
                };
                rig.add(if family == Family::Spear { "tip" } else { "edge" }, edge, Some("head"));
            }
        }
        rig
    }

    fn build(&self, p: &Params, rig: &mut Rig) -> Generated {
        let names: Vec<String> = rig.joints.iter().map(|j| j.name.clone()).collect();
        for n in &names {
            rig.frame(n, Vec3::Y, Vec3::X);
        }
        let (parts, curves) = match self.0.family {
            Family::Sword | Family::Dagger => (self.blade_weapon(p, rig), vec![("centerline".to_owned(), names.clone())]),
            Family::Axe => (self.axe(p, rig), vec![("haft".to_owned(), names[..5].to_vec())]),
            Family::Hammer => (self.hammer(p, rig), vec![("haft".to_owned(), names[..5].to_vec())]),
            Family::Spear => (self.spear(p, rig), vec![("haft".to_owned(), names[..4].to_vec())]),
        };
        let culture = match p.choice("style") {
            "dwarven" => "dwarven",
            "elven" => "elven",
            _ => "human_medieval",
        };
        Generated { curves, parts, dna: json!({ "archetype": self.0.id, "culture": culture }) }
    }
}
