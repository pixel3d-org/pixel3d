//! `eye`: an eyeball looking along +Z with a cornea bulge, and face groups
//! for the sclera, iris and pupil (materials and UVs follow them).

use std::collections::BTreeMap;

use glam::{Affine3A, Quat, Vec3};
use p3d_geo::prim::{Origin, UvSphereSpec, uv_sphere};
use p3d_mesh::ElementKind;

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const PARAMS: &[ParamInfo] = &[
    ParamInfo { name: "radius", doc: "Eyeball radius.", kind: ParamKind::Length, default: 0.012, min: 0.001, max: 0.5 },
    ParamInfo { name: "iris", doc: "Iris width as a share of the eyeball's width.", kind: ParamKind::Ratio, default: 0.5, min: 0.1, max: 0.95 },
    ParamInfo { name: "pupil", doc: "Pupil width as a share of the iris.", kind: ParamKind::Ratio, default: 0.4, min: 0.05, max: 0.95 },
    ParamInfo { name: "cornea", doc: "Cornea bulge as a share of the radius.", kind: ParamKind::Ratio, default: 0.08, min: 0.0, max: 0.4 },
    ParamInfo { name: "segments", doc: "Segments around.", kind: ParamKind::Count, default: 24.0, min: 8.0, max: 128.0 },
];

pub struct Eye;

impl Feature for Eye {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "eye",
            label: "Eye",
            summary: "An eyeball looking along +Z with a cornea bulge and face groups sclera, iris and pupil.",
            params: PARAMS,
            landmarks: &[("center", "the eyeball's center (the origin)"), ("pupil", "the front of the cornea"), ("top", "the top of the eyeball")],
            budget: 1100,
            aliases: &["eyeball", "iris", "pupil", "cornea"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let r = p.get("radius");
        let seg = (p.count("segments") as u32).max(8);
        let mut m = uv_sphere(&UvSphereSpec { radius: r, segments: seg, rings: seg / 2 + 2 }, Origin::Center).map_err(|e| FeatureError {
            path: "/params".into(),
            message: e.to_string(),
            suggestions: Vec::new(),
        })?;
        // Poles along Z: the front pole is the pupil.
        m.transform(Affine3A::from_quat(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)));
        let rf = r as f32;
        let iris = (p.f("iris")).asin();
        let pupil = (p.f("iris") * p.f("pupil")).asin();
        let bulge = p.f("cornea") * rf;
        let verts: Vec<u32> = m.verts().collect();
        for v in verts {
            let q = m.positions[v as usize];
            let theta = (q.z / rf).clamp(-1.0, 1.0).acos();
            if theta < iris {
                let s = (theta / iris * std::f32::consts::FRAC_PI_2).cos();
                m.positions[v as usize] = q + q.normalize_or(Vec3::Z) * bulge * s * s;
            }
        }
        let mut groups: [Vec<u32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for f in m.faces() {
            let c = m.face_centroid(f).normalize_or(Vec3::Z);
            let theta = c.z.clamp(-1.0, 1.0).acos();
            groups[if theta < pupil {
                2
            } else if theta < iris {
                1
            } else {
                0
            }]
            .push(f);
        }
        for (name, list) in ["sclera", "iris", "pupil"].into_iter().zip(groups) {
            if !list.is_empty() {
                let _ = m.set_group(name, ElementKind::Face, list);
            }
        }
        let mut landmarks = BTreeMap::new();
        landmarks.insert("center".to_owned(), Vec3::ZERO);
        landmarks.insert("pupil".to_owned(), Vec3::new(0.0, 0.0, rf + bulge));
        landmarks.insert("top".to_owned(), Vec3::new(0.0, rf, 0.0));
        crate::bake_landmarks(&mut m, &landmarks);
        Ok(Built { shape: Shape::Mesh(m), landmarks })
    }
}
