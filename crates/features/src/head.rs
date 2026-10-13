//! `head_base`: a humanoid head blockout as SDF shapes (cranium, face,
//! jaw, muzzle, chin, cheekbones, brow, eye sockets, neck) with the
//! `humanoid_head` landmarks on its surface. The chin's underside is at
//! y = 0, the face looks toward +Z and the character's left is +X; ears
//! and the nose are features of their own that blend in.

use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use p3d_sdf::{Combine, Item, Modifiers, Prim, Shape as SdfShape};

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo { name: "height", doc: "From the chin's underside to the crown.", kind: ParamKind::Length, default: 0.23, min: 0.01, max: 5.0 },
    ParamInfo { name: "width", doc: "Skull width as a share of the height.", kind: ParamKind::Ratio, default: 0.66, min: 0.4, max: 1.0 },
    ParamInfo { name: "depth", doc: "Front to back as a share of the height.", kind: ParamKind::Ratio, default: 0.86, min: 0.6, max: 1.2 },
    ParamInfo {
        name: "jaw",
        doc: "Jaw width as a share of the skull's (0.6 narrow, 1.2 heavy).",
        kind: ParamKind::Ratio,
        default: 0.8,
        min: 0.5,
        max: 1.3,
    },
    ParamInfo { name: "chin", doc: "How far the chin projects (0 receding to 1 strong).", kind: ParamKind::Ratio, default: 0.5, min: 0.0, max: 1.0 },
    ParamInfo {
        name: "brow",
        doc: "Brow ridge strength (0 smooth to 1 heavy, dwarves and orcs).",
        kind: ParamKind::Ratio,
        default: 0.4,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo {
        name: "cheekbones",
        doc: "Cheekbone prominence (high elven cheekbones near 1).",
        kind: ParamKind::Ratio,
        default: 0.5,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo { name: "sockets", doc: "Eye socket depth (0 none).", kind: ParamKind::Ratio, default: 0.5, min: 0.0, max: 1.0 },
    ParamInfo {
        name: "neck",
        doc: "Neck length below the head as a share of the height (0 none).",
        kind: ParamKind::Ratio,
        default: 0.6,
        min: 0.0,
        max: 1.0,
    },
];

const LANDMARKS: &[(&str, &str)] = &[
    ("crown", "the top"),
    ("chin", "the chin's front"),
    ("forehead", "the forehead's center"),
    ("hairline_front", "where the hair starts"),
    ("nose_root", "between the eyes (anchor a nose here)"),
    ("nose_base", "under the nose"),
    ("mouth", "the mouth's center"),
    ("eye_l", "the left eye socket"),
    ("eye_r", "the right eye socket"),
    ("brow_l", "the left brow"),
    ("brow_r", "the right brow"),
    ("cheek_l", "the left cheekbone"),
    ("cheek_r", "the right cheekbone"),
    ("jaw_l", "the left jaw angle"),
    ("jaw_r", "the right jaw angle"),
    ("ear_root_l", "where the left ear attaches"),
    ("ear_root_r", "where the right ear attaches"),
    ("temple_l", "the left temple"),
    ("temple_r", "the right temple"),
    ("nape", "the back of the neck under the skull"),
    ("back_of_head", "the back of the skull"),
];

fn item(prim: Prim, center: Vec3, combine: Combine, name: &str) -> Item {
    Item { shape: SdfShape::Prim(prim), center, rotation: Quat::IDENTITY, combine, modifiers: Modifiers::default(), name: Some(name.to_owned()) }
}

pub struct HeadBase;

impl Feature for HeadBase {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "head_base",
            label: "Head base",
            summary: "A humanoid head blockout (cranium, jaw, chin, cheekbones, brow, eye sockets, neck) as SDF shapes with the humanoid_head landmarks; blend a nose and ears onto it.",
            params: PARAMS,
            landmarks: LANDMARKS,
            budget: 8000,
            aliases: &["head", "skull", "face", "humanoid head", "character head", "bust"],
            sdf: true,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        // Proportions of an adult head in head heights (chin to crown), the
        // ear canal at z = 0; `sx` widens and `sz` deepens the skull.
        let h = p.f("height");
        let sx = p.f("width") / 0.66;
        let sz = p.f("depth") / 0.86;
        let jaw = p.f("jaw") / 0.8;
        let v = |x: f32, y: f32, z: f32| Vec3::new(x * sx, y, z) * h;
        let mut items = vec![
            item(Prim::Ellipsoid { radii: v(0.33, 0.4, 0.43 * sz) }, v(0.0, 0.6, 0.0), Combine::Union, "cranium"),
            item(Prim::Ellipsoid { radii: v(0.24, 0.2, 0.22) }, v(0.0, 0.38, 0.2), Combine::SmoothUnion(0.1 * h), "face"),
            item(Prim::Ellipsoid { radii: v(0.25 * jaw, 0.2, 0.24) }, v(0.0, 0.2, 0.15), Combine::SmoothUnion(0.09 * h), "jaw"),
            item(Prim::Ellipsoid { radii: v(0.13, 0.08, 0.1) }, v(0.0, 0.21, 0.31), Combine::SmoothUnion(0.06 * h), "muzzle"),
            item(
                Prim::Ellipsoid { radii: Vec3::new(0.07, 0.05, 0.05) * h },
                v(0.0, 0.06, 0.29 + 0.06 * p.f("chin")),
                Combine::SmoothUnion(0.06 * h),
                "chin",
            ),
        ];
        let cheek = 0.6 + 0.8 * p.f("cheekbones");
        for (s, n) in [(1.0f32, "cheekbone_l"), (-1.0, "cheekbone_r")] {
            items.push(item(
                Prim::Ellipsoid { radii: Vec3::new(0.06, 0.035, 0.08) * h * cheek },
                v(s * 0.21, 0.46, 0.22),
                Combine::SmoothUnion(0.05 * h),
                n,
            ));
        }
        let brow_r = 0.035 * h * (0.4 + 1.2 * p.f("brow"));
        for (s, n) in [(1.0f32, "brow_ridge_l"), (-1.0, "brow_ridge_r")] {
            let cap = Prim::Capsule { a: v(0.0, 0.58, 0.4 * sz), b: v(s * 0.17, 0.59, 0.34 * sz), ra: brow_r, rb: brow_r * 0.8 };
            items.push(item(cap, Vec3::ZERO, Combine::SmoothUnion(0.04 * h), n));
        }
        let neck = p.f("neck");
        if neck > 0.0 {
            // A flat cut at the bottom, like a bust, to blend onto a body.
            let (top, bottom) = (0.32, -neck * 0.45);
            let edge = 0.02 * h;
            let cyl = Prim::Cylinder { height: (top - bottom) * h - 2.0 * edge, radius: 0.2 * sx * h - edge };
            let mut neck = item(cyl, v(0.0, (top + bottom) * 0.5, -0.07), Combine::SmoothUnion(0.08 * h), "neck");
            neck.modifiers.round = edge;
            items.push(neck);
        }
        let sockets = p.f("sockets");
        if sockets > 0.0 {
            for (s, n) in [(1.0f32, "socket_l"), (-1.0, "socket_r")] {
                let r = Vec3::new(0.075, 0.055, 0.03 + 0.06 * sockets) * h;
                items.push(item(Prim::Ellipsoid { radii: r }, v(s * 0.137, 0.5, 0.39 * sz), Combine::SmoothSubtract(0.025 * h), n));
            }
        }
        let mut landmarks = BTreeMap::new();
        let mut mark = |n: &str, x: f32, y: f32, z: f32| {
            landmarks.insert(n.to_owned(), v(x, y, z));
        };
        mark("crown", 0.0, 1.0, 0.0);
        mark("chin", 0.0, 0.02, 0.36);
        mark("forehead", 0.0, 0.72, 0.44 * sz);
        mark("hairline_front", 0.0, 0.85, 0.34 * sz);
        mark("nose_root", 0.0, 0.53, 0.44 * sz);
        mark("nose_base", 0.0, 0.31, 0.44);
        mark("mouth", 0.0, 0.2, 0.42);
        mark("nape", 0.0, 0.3, -0.36 * sz);
        mark("back_of_head", 0.0, 0.6, -0.46 * sz);
        for (s, side) in [(1.0f32, "l"), (-1.0, "r")] {
            mark(&format!("eye_{side}"), s * 0.137, 0.5, 0.38 * sz);
            mark(&format!("brow_{side}"), s * 0.137, 0.585, 0.44 * sz);
            mark(&format!("cheek_{side}"), s * 0.26, 0.44, 0.24);
            mark(&format!("jaw_{side}"), s * 0.3 * jaw, 0.14, 0.02);
            mark(&format!("ear_root_{side}"), s * 0.4, 0.44, 0.0);
            mark(&format!("temple_{side}"), s * 0.3, 0.62, 0.24 * sz);
        }
        crate::onto_surface(&items, &mut landmarks);
        Ok(Built { shape: Shape::Sdf(items), landmarks })
    }
}
