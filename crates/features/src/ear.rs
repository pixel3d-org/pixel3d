//! `ear`: an ear as a cupped shell (project spec 17.5). The outline comes
//! from the variant (human, elf, cat, dog, horse, bat), sharpened by
//! `pointiness` and widened at the lobe by `lobe`; the shell is cupped and
//! bent, thickened, and turned to face outward (+X, a left ear; mirror for
//! the right), tilted back and flared away from the head. The root sits at
//! the origin, so anchoring on `ear_root_l` attaches it.

use std::collections::BTreeMap;

use glam::{DVec2, Mat3, Vec3};
use p3d_geo::generate::{ShellSpec, shell};

use crate::outline::{round_corners, shaped};
use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const VARIANTS: &[&str] = &["human", "elf", "cat", "dog", "horse", "bat"];

const PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "variant",
        doc: "Ear shape: human, elf, cat, dog, horse or bat.",
        kind: ParamKind::Choice(VARIANTS),
        default: 1.0,
        min: 0.0,
        max: 5.0,
    },
    ParamInfo { name: "length", doc: "From the root to the tip.", kind: ParamKind::Length, default: 0.08, min: 0.005, max: 2.0 },
    ParamInfo { name: "width", doc: "Width as a share of the length.", kind: ParamKind::Ratio, default: 0.5, min: 0.15, max: 1.5 },
    ParamInfo { name: "pointiness", doc: "0 round to 1 needle-sharp tip.", kind: ParamKind::Ratio, default: 0.7, min: 0.0, max: 1.0 },
    ParamInfo { name: "cup", doc: "How deep the bowl is (edges curl outward).", kind: ParamKind::Ratio, default: 0.35, min: -1.0, max: 1.5 },
    ParamInfo { name: "bend", doc: "The tip curls outward (negative: droops in).", kind: ParamKind::Ratio, default: 0.1, min: -1.0, max: 1.0 },
    ParamInfo { name: "lobe", doc: "Size of the lobe at the back of the base.", kind: ParamKind::Ratio, default: 0.3, min: 0.0, max: 1.0 },
    ParamInfo {
        name: "tilt_back",
        doc: "Lean of the ear toward the back of the head (degrees).",
        kind: ParamKind::Angle,
        default: 20.0,
        min: -60.0,
        max: 90.0,
    },
    ParamInfo { name: "flare", doc: "Angle away from the head (degrees).", kind: ParamKind::Angle, default: 25.0, min: -30.0, max: 90.0 },
    ParamInfo { name: "thickness", doc: "Total thickness.", kind: ParamKind::Length, default: 0.003, min: 0.0002, max: 0.1 },
    ParamInfo { name: "detail", doc: "Edge length; a 14th of the length when 0.", kind: ParamKind::Length, default: 0.0, min: 0.0, max: 0.5 },
];

/// Normalized outline points (x across from front -0.5 to back 0.5, y from
/// the root to the tip) and whether each is a sharp corner.
fn variant_points(v: &str) -> Vec<(f64, f64, bool)> {
    match v {
        "human" => vec![
            (-0.30, 0.0, false),
            (0.25, 0.0, false),
            (0.48, 0.15, false),
            (0.50, 0.45, false),
            (0.40, 0.80, false),
            (0.10, 1.0, false),
            (-0.20, 0.92, false),
            (-0.35, 0.70, false),
            (-0.38, 0.40, false),
            (-0.35, 0.15, false),
        ],
        "cat" => vec![(-0.5, 0.0, false), (0.5, 0.0, false), (0.30, 0.5, false), (0.05, 1.0, true), (-0.25, 0.5, false)],
        "dog" => vec![
            (-0.4, 0.0, false),
            (0.4, 0.0, false),
            (0.5, 0.4, false),
            (0.35, 0.85, false),
            (0.0, 1.0, false),
            (-0.35, 0.85, false),
            (-0.5, 0.4, false),
        ],
        "horse" => vec![
            (-0.3, 0.0, false),
            (0.3, 0.0, false),
            (0.35, 0.4, false),
            (0.12, 0.85, false),
            (0.0, 1.0, true),
            (-0.15, 0.80, false),
            (-0.3, 0.4, false),
        ],
        "bat" => vec![
            (-0.5, 0.0, false),
            (0.5, 0.0, false),
            (0.55, 0.35, false),
            (0.35, 0.75, false),
            (0.10, 1.0, true),
            (-0.20, 0.70, false),
            (-0.45, 0.30, false),
        ],
        // elf
        _ => vec![
            (-0.32, 0.0, false),
            (0.30, 0.0, false),
            (0.46, 0.12, false),
            (0.42, 0.42, false),
            (0.27, 0.72, false),
            (0.18, 1.0, true),
            (0.02, 0.74, false),
            (-0.17, 0.45, false),
            (-0.30, 0.18, false),
        ],
    }
}

pub struct Ear;

impl Feature for Ear {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "ear",
            label: "Ear",
            summary: "An ear (human, elf, cat, dog, horse or bat) as a cupped, bent shell with a lobe, tilted and flared; faces +X (a left ear).",
            params: PARAMS,
            landmarks: &[
                ("root", "where it joins the head (the origin)"),
                ("tip", "the top point"),
                ("lobe", "the bottom of the back edge"),
                ("rim_back", "the back edge at mid height"),
            ],
            budget: 950,
            aliases: &["elf ear", "pointed ear", "animal ear", "cat ear", "pinna"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let len = p.get("length");
        let wid = len * p.get("width");
        let mut pts = variant_points(p.choice("variant"));
        // Pointiness pulls the tip's neighbours toward it and makes it sharp.
        if let Some(t) = pts.iter().enumerate().max_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(i, _)| i) {
            let tip = pts[t];
            let k = 0.4 * p.get("pointiness");
            let n = pts.len();
            for j in [(t + n - 1) % n, (t + 1) % n] {
                let q = &mut pts[j];
                q.0 += (tip.0 - q.0) * k;
                q.1 += (tip.1 - q.1) * k * 0.5;
            }
            pts[t].2 = p.get("pointiness") > 0.3;
        }
        // The lobe bulges the back of the base.
        let lobe = p.get("lobe");
        for q in pts.iter_mut().filter(|q| q.0 > 0.0 && q.1 > 0.02 && q.1 < 0.3) {
            q.0 += 0.18 * lobe * (1.0 - q.1 / 0.3);
        }
        // A sharp tip keeps a small round nub (about the thickness across) so
        // it shades smoothly and keeps a wall.
        let nub = (0.5 * p.get("thickness")).max(0.02 * len);
        let outline: Vec<DVec2> = round_corners(&shaped(&pts, wid, len, 3), nub, std::f64::consts::FRAC_PI_4);
        let spacing = if p.get("detail") > 0.0 { p.get("detail") } else { len / 14.0 };
        let spec = ShellSpec { contours: vec![outline], spacing, thickness: p.get("thickness"), cup: p.get("cup"), bend: p.get("bend"), twist: 0.0 };
        let mut mesh = shell(&spec).map_err(|e| FeatureError { path: "/params".into(), message: e.to_string(), suggestions: Vec::new() })?;
        // Face +X (outward), lean back, flare away from the head.
        let turn =
            Mat3::from_rotation_z(-p.f("flare")) * Mat3::from_rotation_x(-p.f("tilt_back")) * Mat3::from_rotation_y(std::f32::consts::FRAC_PI_2);
        mesh.transform(glam::Affine3A::from_mat3(turn));
        let at = |x: f64, y: f64| turn * Vec3::new(x as f32, y as f32, 0.0);
        let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(l, h), q| (l.min(q.0), h.max(q.0)));
        let tip = pts.iter().max_by(|a, b| a.1.total_cmp(&b.1)).copied().unwrap_or((0.0, 1.0, true));
        let mut landmarks = BTreeMap::new();
        landmarks.insert("root".to_owned(), at((lo + hi) * 0.5 * wid, 0.0));
        landmarks.insert("tip".to_owned(), at(tip.0 * wid, tip.1 * len));
        landmarks.insert("lobe".to_owned(), at(hi * wid * 0.9, 0.08 * len));
        landmarks.insert("rim_back".to_owned(), at(hi * wid, 0.5 * len));
        crate::bake_landmarks(&mut mesh, &landmarks);
        Ok(Built { shape: Shape::Mesh(mesh), landmarks })
    }
}
