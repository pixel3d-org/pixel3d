//! `horn`: a tapering horn built ring by ring along a curling, spiralling
//! path, with ridges as growth rings. It starts at the origin heading +Y
//! and curls back (-Z); the lengthwise lines and rings are its edge flow.

use std::collections::BTreeMap;

use glam::Vec3;

use crate::tube::{TubeSpec, tube};
use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo { name: "length", doc: "Along the curve, root to tip.", kind: ParamKind::Length, default: 0.25, min: 0.005, max: 5.0 },
    ParamInfo { name: "radius", doc: "Radius at the root.", kind: ParamKind::Length, default: 0.03, min: 0.001, max: 1.0 },
    ParamInfo { name: "tip", doc: "Radius near the tip as a share of the root's.", kind: ParamKind::Ratio, default: 0.12, min: 0.0, max: 1.0 },
    ParamInfo {
        name: "curl",
        doc: "Total turn backward along the length (degrees).",
        kind: ParamKind::Angle,
        default: 120.0,
        min: -720.0,
        max: 720.0,
    },
    ParamInfo {
        name: "spiral",
        doc: "Sideways drift toward +X as a share of the length (ram horns).",
        kind: ParamKind::Ratio,
        default: 0.0,
        min: -1.0,
        max: 1.0,
    },
    ParamInfo { name: "ridges", doc: "Growth rings along the length.", kind: ParamKind::Count, default: 8.0, min: 0.0, max: 64.0 },
    ParamInfo { name: "ridge_depth", doc: "Depth of the rings as a share of the radius.", kind: ParamKind::Ratio, default: 0.08, min: 0.0, max: 0.5 },
    ParamInfo { name: "sides", doc: "Points around each ring.", kind: ParamKind::Count, default: 12.0, min: 3.0, max: 64.0 },
    ParamInfo { name: "rings", doc: "Rings along the length.", kind: ParamKind::Count, default: 32.0, min: 4.0, max: 512.0 },
];

pub struct Horn;

impl Feature for Horn {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "horn",
            label: "Horn",
            summary: "A tapering horn that curls back and can spiral sideways, with growth rings; ring-and-line edge flow.",
            params: PARAMS,
            landmarks: &[("root", "the base center (the origin)"), ("tip", "the point"), ("mid", "halfway along")],
            budget: 800,
            aliases: &["ram horn", "antler", "spike", "tusk", "tentacle"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let len = p.f("length");
        let n = p.count("rings").max(4);
        let curl = p.f("curl");
        let spiral = p.f("spiral");
        let path: Vec<Vec3> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let s = t * len;
                let (y, z) = if curl.abs() < 1e-6 {
                    (s, 0.0)
                } else {
                    let r = len / curl;
                    (r * (curl * t).sin(), -r * (1.0 - (curl * t).cos()))
                };
                Vec3::new(spiral * len * t * t, y, z)
            })
            .collect();
        let (r0, tip, ridges, depth) = (p.f("radius"), p.f("tip"), p.f("ridges"), p.f("ridge_depth"));
        let radius: Vec<f32> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let ring = 1.0 + depth * 0.5 * (1.0 - (std::f32::consts::TAU * ridges * t).cos());
                r0 * (1.0 - (1.0 - tip) * t) * if ridges > 0.0 { ring } else { 1.0 }
            })
            .collect();
        let mut m = tube(&TubeSpec { path: &path, radius: &radius, sides: p.count("sides").max(3), pointed: true })?;
        let mut landmarks = BTreeMap::new();
        landmarks.insert("root".to_owned(), Vec3::ZERO);
        landmarks.insert("tip".to_owned(), path[n]);
        landmarks.insert("mid".to_owned(), path[n / 2]);
        crate::bake_landmarks(&mut m, &landmarks);
        Ok(Built { shape: Shape::Mesh(m), landmarks })
    }
}
