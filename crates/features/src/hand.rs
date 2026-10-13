//! `hand`: a hand as SDF shapes (palm, wrist, thenar pad, knuckles and a
//! capsule per finger bone) that blends onto an arm. A left hand: the
//! wrist at the origin, fingers along +Y, the palm facing -Z and the thumb
//! toward +X; mirror it for the right. Finger bones are face groups
//! (`index_1` to `index_3`, `thumb_2`, ...).

use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use p3d_sdf::{Combine, Item, Modifiers, Prim, Shape as SdfShape};

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "length",
        doc: "From the wrist to the middle fingertip, straight.",
        kind: ParamKind::Length,
        default: 0.19,
        min: 0.01,
        max: 5.0,
    },
    ParamInfo { name: "width", doc: "Palm width as a share of the length.", kind: ParamKind::Ratio, default: 0.45, min: 0.25, max: 0.8 },
    ParamInfo { name: "thickness", doc: "Palm thickness as a share of the length.", kind: ParamKind::Ratio, default: 0.15, min: 0.08, max: 0.3 },
    ParamInfo {
        name: "fingers",
        doc: "Fingers besides the thumb: 4 (index, middle, ring, pinky) or 3 (index, middle, pinky).",
        kind: ParamKind::Count,
        default: 4.0,
        min: 3.0,
        max: 4.0,
    },
    ParamInfo {
        name: "finger_length",
        doc: "Finger length relative to the palm (0.7 stubby, 1.4 long and elegant).",
        kind: ParamKind::Ratio,
        default: 1.0,
        min: 0.6,
        max: 1.6,
    },
    ParamInfo { name: "finger_width", doc: "Finger thickness (0.7 slender, 1.4 thick).", kind: ParamKind::Ratio, default: 1.0, min: 0.6, max: 1.4 },
    ParamInfo { name: "taper", doc: "How much the fingers thin toward the tips.", kind: ParamKind::Ratio, default: 0.2, min: 0.0, max: 0.6 },
    ParamInfo { name: "spread", doc: "Fan angle of the outer fingers (degrees).", kind: ParamKind::Angle, default: 6.0, min: 0.0, max: 30.0 },
    ParamInfo {
        name: "curl",
        doc: "Bend at each finger joint toward the palm (degrees; 0 flat, 80 a fist).",
        kind: ParamKind::Angle,
        default: 12.0,
        min: -10.0,
        max: 85.0,
    },
    ParamInfo { name: "thumb", doc: "Thumb opening from the fingers (degrees).", kind: ParamKind::Angle, default: 35.0, min: 5.0, max: 80.0 },
    ParamInfo { name: "knuckles", doc: "Knuckle bumps on the back (0 none).", kind: ParamKind::Ratio, default: 0.3, min: 0.0, max: 1.0 },
];

fn item(prim: Prim, center: Vec3, combine: Combine, name: &str) -> Item {
    Item { shape: SdfShape::Prim(prim), center, rotation: Quat::IDENTITY, combine, modifiers: Modifiers::default(), name: Some(name.to_owned()) }
}

/// Bends a direction toward the palm (-Z) by `angle`.
fn bend(d: Vec3, angle: f32) -> Vec3 {
    let axis = Vec3::Z.cross(d).normalize_or(Vec3::NEG_X);
    (Quat::from_axis_angle(axis, angle) * d).normalize_or(d)
}

pub struct Hand;

impl Feature for Hand {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "hand",
            label: "Hand",
            summary: "A left hand (fingers +Y, palm -Z, thumb +X) as SDF capsules per finger bone with palm, thenar pad and knuckles; curl, spread and thumb pose it.",
            params: PARAMS,
            landmarks: &[
                ("wrist", "the wrist's underside (the origin side)"),
                ("palm", "the palm's center"),
                ("back", "the back of the hand"),
                ("thumb_tip", "the thumb's tip"),
                ("index_tip", "the index fingertip"),
                ("middle_tip", "the middle fingertip"),
                ("pinky_tip", "the little fingertip (ring_tip too with 4 fingers)"),
            ],
            budget: 6000,
            aliases: &["hand", "fingers", "fist", "palm", "claw", "paw"],
            sdf: true,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let l = p.f("length");
        let fl = p.f("finger_length");
        let palm = l * 0.57 / (0.57 + 0.43 * fl);
        let fingers_len = l - palm;
        let pw = p.f("width") * l;
        let th = p.f("thickness") * l;
        let n = p.count("fingers").clamp(3, 4);
        let (names, rel, arch, girth): (&[&str], &[f32], &[f32], &[f32]) = if n == 4 {
            (&["index", "middle", "ring", "pinky"], &[0.92, 1.0, 0.95, 0.76], &[0.02, 0.0, 0.025, 0.08], &[1.0, 1.03, 0.97, 0.85])
        } else {
            (&["index", "middle", "pinky"], &[0.95, 1.0, 0.85], &[0.02, 0.0, 0.07], &[1.0, 1.03, 0.9])
        };
        let r0 = p.f("finger_width") * pw / (n as f32 * 2.0) * 0.82;
        let (taper, spread, curl) = (p.f("taper"), p.f("spread"), p.f("curl"));
        let k = r0 * 0.5;
        let mut items = vec![
            item(
                Prim::Box { half: Vec3::new(pw * 0.5, palm * 0.5, th * 0.5), round: th * 0.45 },
                Vec3::new(0.0, palm * 0.5, 0.0),
                Combine::Union,
                "palm",
            ),
            item(Prim::Ellipsoid { radii: Vec3::new(pw * 0.36, 0.06 * l, th * 0.6) }, Vec3::ZERO, Combine::SmoothUnion(th * 0.5), "wrist"),
            item(
                Prim::Ellipsoid { radii: Vec3::new(pw * 0.2, palm * 0.25, th * 0.35) },
                Vec3::new(pw * 0.25, palm * 0.3, -th * 0.25),
                Combine::SmoothUnion(th * 0.4),
                "thenar",
            ),
        ];
        let mut landmarks = BTreeMap::new();
        let half = pw * 0.5 - r0 * 1.1;
        for (i, name) in names.iter().enumerate() {
            let x = half * (1.0 - 2.0 * i as f32 / (n - 1) as f32);
            let r = r0 * girth[i];
            let mut a = Vec3::new(x, palm * (1.0 - arch[i]) - r * 1.5, 0.0);
            let mut dir = Quat::from_rotation_z(-spread * x / (pw * 0.5)) * Vec3::Y;
            if p.f("knuckles") > 0.0 {
                let c = a + dir * r * 1.5 + Vec3::Z * th * 0.25;
                items.push(item(Prim::Sphere { radius: r * 0.9 * p.f("knuckles") }, c, Combine::SmoothUnion(k), &format!("knuckle_{name}")));
            }
            let len = fingers_len * rel[i] + r * 1.5;
            for (j, (share, bend_by)) in [(0.45f32, 1.0f32), (0.3, 1.1), (0.25, 0.8)].into_iter().enumerate() {
                dir = bend(dir, curl * bend_by);
                let b = a + dir * len * share;
                let (ra, rb) = (r * (1.0 - taper * j as f32 / 3.0), r * (1.0 - taper * (j + 1) as f32 / 3.0));
                let combine = if j == 0 { Combine::SmoothUnion(k) } else { Combine::Union };
                items.push(item(Prim::Capsule { a, b, ra, rb }, Vec3::ZERO, combine, &format!("{name}_{}", j + 1)));
                a = b;
            }
            landmarks.insert(format!("{name}_tip"), a + dir * r);
        }
        // The thumb: a metacarpal inside the thenar pad, then two bones.
        let open = p.f("thumb");
        let mut dir = Vec3::new(open.sin(), open.cos(), -0.4).normalize_or(Vec3::Y);
        let mut a = Vec3::new(pw * 0.3, palm * 0.15, -th * 0.15);
        let rt = r0 * 1.25;
        for (j, share) in [0.24f32, 0.17 * fl, 0.14 * fl].into_iter().enumerate() {
            if j > 0 {
                dir = bend(dir, curl * 0.6);
            }
            let b = a + dir * l * share;
            let (ra, rb) = (rt * (1.0 - taper * j as f32 / 3.0), rt * (1.0 - taper * (j + 1) as f32 / 3.0));
            let combine = if j == 0 { Combine::SmoothUnion(th * 0.4) } else { Combine::Union };
            items.push(item(Prim::Capsule { a, b, ra, rb }, Vec3::ZERO, combine, &format!("thumb_{}", j + 1)));
            a = b;
        }
        landmarks.insert("thumb_tip".to_owned(), a + dir * rt);
        landmarks.insert("wrist".to_owned(), Vec3::new(0.0, -0.06 * l, 0.0));
        landmarks.insert("palm".to_owned(), Vec3::new(0.0, palm * 0.5, -th));
        landmarks.insert("back".to_owned(), Vec3::new(0.0, palm * 0.5, th));
        crate::onto_surface(&items, &mut landmarks);
        Ok(Built { shape: Shape::Sdf(items), landmarks })
    }
}
