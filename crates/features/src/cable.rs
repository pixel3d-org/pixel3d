//! `cable`: a hanging cable or twisted rope between two ends. The start is
//! the origin and the end lies `span` along +X and `drop` up (negative:
//! down); the cable sags below the straight line by `sag` of the span
//! (a parabola, close to a catenary for real sags). More than one ply
//! twists the plies around the centerline as a rope.

use std::collections::BTreeMap;
use std::f32::consts::TAU;

use glam::Vec3;
use p3d_mesh::{ElementKind, Mesh};

use crate::tube::{TubeSpec, frames, tube};
use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo { name: "span", doc: "Horizontal distance between the ends (along +X).", kind: ParamKind::Length, default: 1.0, min: 0.01, max: 200.0 },
    ParamInfo {
        name: "drop",
        doc: "Height of the end above the start (negative: below).",
        kind: ParamKind::Length,
        default: 0.0,
        min: -100.0,
        max: 100.0,
    },
    ParamInfo {
        name: "sag",
        doc: "How far the middle hangs below the straight line, as a share of the span (0 taut).",
        kind: ParamKind::Ratio,
        default: 0.08,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo {
        name: "radius",
        doc: "Cable radius (the whole rope's for several plies).",
        kind: ParamKind::Length,
        default: 0.01,
        min: 0.0002,
        max: 2.0,
    },
    ParamInfo {
        name: "plies",
        doc: "Strands twisted together: 1 for a cable, 3 for a rope.",
        kind: ParamKind::Count,
        default: 1.0,
        min: 1.0,
        max: 7.0,
    },
    ParamInfo { name: "twist", doc: "Turns of the plies per meter of length.", kind: ParamKind::Ratio, default: 6.0, min: 0.0, max: 200.0 },
    ParamInfo { name: "sides", doc: "Points around each ply.", kind: ParamKind::Count, default: 8.0, min: 3.0, max: 32.0 },
    ParamInfo { name: "segments", doc: "Rings along the length.", kind: ParamKind::Count, default: 48.0, min: 4.0, max: 2048.0 },
];

pub struct Cable;

impl Feature for Cable {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "cable",
            label: "Cable",
            summary: "A cable or twisted rope hanging between two ends with a sag; the start at the origin, the end along +X.",
            params: PARAMS,
            landmarks: &[("start", "the first end (the origin)"), ("end", "the other end"), ("mid", "the lowest point")],
            budget: 1000,
            aliases: &["rope", "wire", "cord", "hose", "power line", "catenary", "chain"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (span, drop, sag) = (p.f("span"), p.f("drop"), p.f("sag"));
        let n = p.count("segments").max(4);
        let path: Vec<Vec3> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                Vec3::new(span * t, drop * t - 4.0 * sag * span * t * (1.0 - t), 0.0)
            })
            .collect();
        let radius = p.f("radius");
        let sides = p.count("sides").max(3);
        let plies = p.count("plies").clamp(1, 7);
        let mut mesh = if plies == 1 {
            tube(&TubeSpec { path: &path, radius: &vec![radius; path.len()], sides, pointed: false })?
        } else {
            // Plies packed in a circle of the rope's radius, twisted along it.
            let s = (std::f32::consts::PI / plies as f32).sin();
            let r = radius * s / (1.0 + s);
            let d = radius - r;
            let fr = frames(&path);
            let mut len = 0.0f32;
            let along: Vec<f32> = path
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    if i > 0 {
                        len += q.distance(path[i - 1]);
                    }
                    len
                })
                .collect();
            let mut m = Mesh::default();
            for k in 0..plies {
                let ply: Vec<Vec3> = path
                    .iter()
                    .zip(&fr)
                    .zip(&along)
                    .map(|((q, (_, a, b)), s)| {
                        let th = TAU * (p.f("twist") * s + k as f32 / plies as f32);
                        *q + (*a * th.cos() + *b * th.sin()) * d
                    })
                    .collect();
                let one = tube(&TubeSpec { path: &ply, radius: &vec![r; ply.len()], sides, pointed: false })?;
                m.append(&one);
            }
            // The appended tubes keep their groups; one `side` group covers them all.
            let all: Vec<u32> = m.faces().collect();
            let _ = m.set_group("rope", ElementKind::Face, all);
            m
        };
        let low = path.iter().copied().min_by(|a, b| a.y.total_cmp(&b.y)).unwrap_or(Vec3::ZERO);
        let mut landmarks = BTreeMap::new();
        landmarks.insert("start".to_owned(), Vec3::ZERO);
        landmarks.insert("end".to_owned(), path[n]);
        landmarks.insert("mid".to_owned(), low);
        crate::bake_landmarks(&mut mesh, &landmarks);
        Ok(Built { shape: Shape::Mesh(mesh), landmarks })
    }
}
