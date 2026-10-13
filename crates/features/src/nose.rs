//! `nose`: an SDF nose (bridge, tip, wings and nostrils) that blends into
//! an SDF head. The root (between the eyes) is the origin; the nose runs
//! down (-Y) and forward (+Z).

use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use p3d_sdf::{Combine, Item, Modifiers, Prim, Shape as SdfShape};

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo { name: "length", doc: "From the root to the base, down the face.", kind: ParamKind::Length, default: 0.045, min: 0.003, max: 1.0 },
    ParamInfo { name: "width", doc: "Across the wings.", kind: ParamKind::Length, default: 0.034, min: 0.002, max: 1.0 },
    ParamInfo {
        name: "bridge",
        doc: "How far the bridge stands out (0 flat to 1 prominent).",
        kind: ParamKind::Ratio,
        default: 0.5,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo { name: "tip", doc: "Size of the tip (0.5 small to 2 bulbous).", kind: ParamKind::Ratio, default: 1.0, min: 0.3, max: 2.5 },
    ParamInfo { name: "flare", doc: "Width of the wings (alae).", kind: ParamKind::Ratio, default: 1.0, min: 0.3, max: 2.0 },
    ParamInfo { name: "upturn", doc: "Tilt of the tip upward (degrees).", kind: ParamKind::Angle, default: 5.0, min: -30.0, max: 45.0 },
    ParamInfo { name: "nostrils", doc: "Depth of the nostrils (0 none).", kind: ParamKind::Ratio, default: 0.6, min: 0.0, max: 1.0 },
];

fn item(prim: Prim, center: Vec3, combine: Combine, name: &str) -> Item {
    Item { shape: SdfShape::Prim(prim), center, rotation: Quat::IDENTITY, combine, modifiers: Modifiers::default(), name: Some(name.to_owned()) }
}

pub struct Nose;

impl Feature for Nose {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "nose",
            label: "Nose",
            summary: "A nose (bridge, tip, wings, nostrils) as SDF shapes that blend into an SDF head; root at the origin, pointing down and forward.",
            params: PARAMS,
            landmarks: &[
                ("root", "between the eyes (the origin)"),
                ("tip", "the tip"),
                ("base", "under the tip, above the lip"),
                ("ala_l", "the left wing"),
                ("ala_r", "the right wing"),
            ],
            budget: 1500,
            aliases: &["nose", "snout", "nostrils", "bridge"],
            sdf: true,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (len, wid) = (p.f("length"), p.f("width"));
        let proj = len * (0.35 + 0.35 * p.f("bridge"));
        let lift = p.f("upturn").sin() * 0.3 * len;
        let tip = Vec3::new(0.0, -len * 0.85 + lift, proj);
        let tip_r = wid * 0.24 * p.f("tip");
        let k = wid * 0.18;
        let mut items = vec![
            item(
                Prim::Capsule { a: Vec3::new(0.0, 0.0, wid * 0.05), b: tip * 0.88, ra: wid * 0.14, rb: wid * 0.2 },
                Vec3::ZERO,
                Combine::Union,
                "bridge",
            ),
            item(Prim::Sphere { radius: tip_r }, tip, Combine::SmoothUnion(k), "tip"),
        ];
        let ala = |side: f32| Vec3::new(side * wid * 0.36, tip.y - len * 0.06, tip.z - proj * 0.45);
        for (side, name) in [(1.0f32, "ala_l"), (-1.0, "ala_r")] {
            items.push(item(
                Prim::Ellipsoid { radii: Vec3::new(wid * 0.2 * p.f("flare"), len * 0.14, proj * 0.3) },
                ala(side),
                Combine::SmoothUnion(k * 0.8),
                name,
            ));
        }
        if p.f("nostrils") > 0.0 {
            let depth = p.f("nostrils");
            for (side, name) in [(1.0f32, "nostril_l"), (-1.0, "nostril_r")] {
                let c = Vec3::new(side * wid * 0.16, tip.y - len * 0.13, tip.z - proj * 0.28);
                items.push(item(
                    Prim::Ellipsoid { radii: Vec3::new(wid * 0.08, len * 0.05 * depth, proj * 0.16) },
                    c,
                    Combine::SmoothSubtract(wid * 0.04),
                    name,
                ));
            }
        }
        let mut landmarks = BTreeMap::new();
        landmarks.insert("root".to_owned(), Vec3::ZERO);
        landmarks.insert("tip".to_owned(), tip + Vec3::Z * tip_r);
        landmarks.insert("base".to_owned(), Vec3::new(0.0, tip.y - len * 0.15, tip.z - proj * 0.45));
        landmarks.insert("ala_l".to_owned(), ala(1.0) + Vec3::X * wid * 0.2);
        landmarks.insert("ala_r".to_owned(), ala(-1.0) - Vec3::X * wid * 0.2);
        Ok(Built { shape: Shape::Sdf(items), landmarks })
    }
}
