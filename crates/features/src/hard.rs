//! Hard-surface features: `bolt`, `rivet`, `vent_grille`, `panel_inset`
//! and `molding`, built from the lathe, extrude, box and sweep generators so
//! their edge flow is the generators' (rings, rims, rows of slats).

use std::collections::BTreeMap;

use glam::{Affine3A, DVec2, DVec3, Quat, Vec3};
use p3d_geo::edit::Axis;
use p3d_geo::generate::{LatheSpec, ShapeSpec, SweepCaps, SweepSpec, extrude_shape, lathe, sweep};
use p3d_geo::prim::{BoxSpec, CapKind, Origin, PrimError, cuboid};
use p3d_mesh::{ElementKind, Mesh};

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

fn prim(e: PrimError) -> FeatureError {
    FeatureError { path: "/params".into(), message: e.to_string(), suggestions: Vec::new() }
}

fn revolve(profile: &[DVec2], segments: u32) -> Result<Mesh, FeatureError> {
    lathe(&LatheSpec { profile, spans: &[], closed: false, segments, angle: std::f64::consts::TAU, cap: CapKind::Ngon, axis: Axis::Y }).map_err(prim)
}

/// Appends `b` to `a` with its face groups renamed under `prefix`.
fn join(a: &mut Mesh, mut b: Mesh, group: &str) {
    let all: Vec<u32> = b.faces().collect();
    let _ = b.set_group(group, ElementKind::Face, all);
    a.append(&b);
}

fn done(mut mesh: Mesh, landmarks: BTreeMap<String, Vec3>) -> Result<Built, FeatureError> {
    crate::bake_landmarks(&mut mesh, &landmarks);
    Ok(Built { shape: Shape::Mesh(mesh), landmarks })
}

// ---- bolt ------------------------------------------------------------------------

const BOLT: &[ParamInfo] = &[
    ParamInfo { name: "size", doc: "Across the flats of the hex head.", kind: ParamKind::Length, default: 0.012, min: 0.001, max: 1.0 },
    ParamInfo { name: "head", doc: "Head height as a share of the size.", kind: ParamKind::Ratio, default: 0.6, min: 0.1, max: 2.0 },
    ParamInfo { name: "washer", doc: "A washer under the head (0 none, 1 yes).", kind: ParamKind::Count, default: 1.0, min: 0.0, max: 1.0 },
    ParamInfo { name: "shaft", doc: "Visible shaft below (0 for a head only).", kind: ParamKind::Length, default: 0.0, min: 0.0, max: 1.0 },
    ParamInfo {
        name: "chamfer",
        doc: "Chamfer of the head's top edge as a share of the size.",
        kind: ParamKind::Ratio,
        default: 0.08,
        min: 0.0,
        max: 0.3,
    },
];

pub struct Bolt;

impl Feature for Bolt {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "bolt",
            label: "Bolt",
            summary: "A hex bolt head with a chamfer, an optional washer and shaft; stands on its base at the origin, up +Y.",
            params: BOLT,
            landmarks: &[("base", "the bottom center (the origin)"), ("top", "the top of the head")],
            budget: 120,
            aliases: &["hex bolt", "nut", "screw head", "fastener"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let s = p.get("size");
        let r = s / 3f64.sqrt();
        let hex: Vec<DVec2> = (0..6).map(|i| DVec2::from_angle(i as f64 * std::f64::consts::FRAC_PI_3) * r).collect();
        let washer = p.get("washer") > 0.5;
        let wh = if washer { s * 0.12 } else { 0.0 };
        let h = s * p.get("head");
        let mut head = extrude_shape(&ShapeSpec {
            contours: &[hex],
            depth: h,
            bevel: s * p.get("chamfer"),
            bevel_segments: 1,
            bevel_front: true,
            bevel_back: false,
            axis: Axis::Y,
        })
        .map_err(prim)?;
        head.transform(Affine3A::from_translation(Vec3::new(0.0, wh as f32, 0.0)));
        let mut mesh = Mesh::default();
        join(&mut mesh, head, "head");
        if washer {
            let w = revolve(&[DVec2::new(0.0, 0.0), DVec2::new(s * 0.95, 0.0), DVec2::new(s * 0.95, wh), DVec2::new(0.0, wh)], 16)?;
            join(&mut mesh, w, "washer");
        }
        if p.get("shaft") > 0.0 {
            let l = p.get("shaft");
            let shaft = revolve(&[DVec2::new(0.0, -l), DVec2::new(s * 0.4, -l), DVec2::new(s * 0.4, 0.0), DVec2::new(0.0, 0.0)], 12)?;
            join(&mut mesh, shaft, "shaft");
        }
        let mut lm = BTreeMap::new();
        lm.insert("base".to_owned(), Vec3::ZERO);
        lm.insert("top".to_owned(), Vec3::new(0.0, (wh + h) as f32, 0.0));
        done(mesh, lm)
    }
}

// ---- rivet -----------------------------------------------------------------------

const RIVET: &[ParamInfo] = &[
    ParamInfo { name: "diameter", doc: "Head diameter.", kind: ParamKind::Length, default: 0.01, min: 0.0005, max: 0.5 },
    ParamInfo { name: "height", doc: "Head height as a share of the diameter.", kind: ParamKind::Ratio, default: 0.35, min: 0.05, max: 1.0 },
    ParamInfo { name: "segments", doc: "Segments around.", kind: ParamKind::Count, default: 12.0, min: 6.0, max: 64.0 },
];

pub struct Rivet;

impl Feature for Rivet {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "rivet",
            label: "Rivet",
            summary: "A domed rivet head standing on the origin, up +Y.",
            params: RIVET,
            landmarks: &[("base", "the bottom center (the origin)"), ("top", "the top of the dome")],
            budget: 100,
            aliases: &["rivet", "stud", "dome head", "nail head"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (r, h) = (p.get("diameter") * 0.5, p.get("diameter") * p.get("height"));
        let profile = [
            DVec2::new(0.0, 0.0),
            DVec2::new(r, 0.0),
            DVec2::new(r, h * 0.3),
            DVec2::new(r * 0.85, h * 0.72),
            DVec2::new(r * 0.5, h * 0.95),
            DVec2::new(0.0, h),
        ];
        let mesh = revolve(&profile, p.count("segments") as u32)?;
        let mut lm = BTreeMap::new();
        lm.insert("base".to_owned(), Vec3::ZERO);
        lm.insert("top".to_owned(), Vec3::new(0.0, h as f32, 0.0));
        done(mesh, lm)
    }
}

// ---- vent_grille -----------------------------------------------------------------

const VENT: &[ParamInfo] = &[
    ParamInfo { name: "width", doc: "Outer width (X).", kind: ParamKind::Length, default: 0.2, min: 0.01, max: 10.0 },
    ParamInfo { name: "height", doc: "Outer height (Y).", kind: ParamKind::Length, default: 0.12, min: 0.01, max: 10.0 },
    ParamInfo { name: "depth", doc: "Frame depth (Z).", kind: ParamKind::Length, default: 0.015, min: 0.001, max: 1.0 },
    ParamInfo { name: "frame", doc: "Frame width as a share of the shorter side.", kind: ParamKind::Ratio, default: 0.1, min: 0.02, max: 0.4 },
    ParamInfo { name: "slats", doc: "Number of slats.", kind: ParamKind::Count, default: 6.0, min: 1.0, max: 200.0 },
    ParamInfo { name: "angle", doc: "Slat angle (degrees).", kind: ParamKind::Angle, default: 35.0, min: -80.0, max: 80.0 },
    ParamInfo { name: "bevel", doc: "Frame edge bevel.", kind: ParamKind::Length, default: 0.002, min: 0.0, max: 0.1 },
];

pub struct VentGrille;

impl Feature for VentGrille {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "vent_grille",
            label: "Vent grille",
            summary: "A framed vent with angled slats, centered on the origin in the XY plane facing +Z.",
            params: VENT,
            landmarks: &[("center", "the middle (the origin)"), ("top", "the frame's top edge"), ("bottom", "the frame's bottom edge")],
            budget: 400,
            aliases: &["vent", "grille", "louver", "air intake", "slats"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (w, h, d) = (p.get("width"), p.get("height"), p.get("depth"));
        let f = w.min(h) * p.get("frame");
        let rect = |a: f64, b: f64| {
            vec![DVec2::new(-a / 2.0, -b / 2.0), DVec2::new(a / 2.0, -b / 2.0), DVec2::new(a / 2.0, b / 2.0), DVec2::new(-a / 2.0, b / 2.0)]
        };
        let mut frame = extrude_shape(&ShapeSpec {
            contours: &[rect(w, h), rect(w - 2.0 * f, h - 2.0 * f)],
            depth: d,
            bevel: p.get("bevel").min(f * 0.4).min(d * 0.4),
            bevel_segments: 1,
            bevel_front: true,
            bevel_back: false,
            axis: Axis::Z,
        })
        .map_err(prim)?;
        frame.transform(Affine3A::from_translation(Vec3::new(0.0, 0.0, -d as f32)));
        let mut mesh = Mesh::default();
        join(&mut mesh, frame, "frame");
        let n = p.count("slats").max(1);
        let inner_h = h - 2.0 * f;
        let pitch = inner_h / n as f64;
        let slat = BoxSpec { size: DVec3::new(w - 2.0 * f, (pitch * 0.18).max(1e-4), d * 0.9), ..BoxSpec::default() };
        for i in 0..n {
            let mut s = cuboid(&slat, Origin::Center).map_err(prim)?;
            let y = -inner_h / 2.0 + pitch * (i as f64 + 0.5);
            s.transform(Affine3A::from_rotation_translation(Quat::from_rotation_x(-p.f("angle")), Vec3::new(0.0, y as f32, (-d * 0.5) as f32)));
            join(&mut mesh, s, "slats");
        }
        let mut lm = BTreeMap::new();
        lm.insert("center".to_owned(), Vec3::ZERO);
        lm.insert("top".to_owned(), Vec3::new(0.0, (h / 2.0) as f32, 0.0));
        lm.insert("bottom".to_owned(), Vec3::new(0.0, (-h / 2.0) as f32, 0.0));
        done(mesh, lm)
    }
}

// ---- panel_inset -----------------------------------------------------------------

const PANEL: &[ParamInfo] = &[
    ParamInfo { name: "width", doc: "Panel width (X).", kind: ParamKind::Length, default: 0.3, min: 0.01, max: 20.0 },
    ParamInfo { name: "height", doc: "Panel height (Y).", kind: ParamKind::Length, default: 0.2, min: 0.01, max: 20.0 },
    ParamInfo { name: "thickness", doc: "Plate thickness (Z).", kind: ParamKind::Length, default: 0.01, min: 0.0005, max: 2.0 },
    ParamInfo {
        name: "inset",
        doc: "Width of the raised border as a share of the shorter side.",
        kind: ParamKind::Ratio,
        default: 0.1,
        min: 0.01,
        max: 0.45,
    },
    ParamInfo {
        name: "depth",
        doc: "How deep the inner panel sits, as a share of the thickness.",
        kind: ParamKind::Ratio,
        default: 0.3,
        min: 0.02,
        max: 0.95,
    },
    ParamInfo {
        name: "corner",
        doc: "Rounding of the outline's corners as a share of the shorter side.",
        kind: ParamKind::Ratio,
        default: 0.05,
        min: 0.0,
        max: 0.45,
    },
];

pub struct PanelInset;

impl Feature for PanelInset {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "panel_inset",
            label: "Panel inset",
            summary: "A rounded plate with a raised border and a recessed inner panel, centered in the XY plane facing +Z.",
            params: PANEL,
            landmarks: &[("center", "the middle of the front face"), ("back", "the middle of the back face")],
            budget: 200,
            aliases: &["panel", "inset panel", "recessed panel", "hatch", "plate"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (w, h, t) = (p.get("width"), p.get("height"), p.get("thickness"));
        let short = w.min(h);
        // Shares of the plate, so every combination in range is valid.
        let (b, c, d) = (short * p.get("inset"), short * p.get("corner"), t * p.get("depth"));
        // Outer and inner outlines with the same points, so the border is a
        // ring of quads; inner corners keep a little roundness.
        let rounded = |a: f64, bb: f64, r: f64| -> Vec<DVec2> {
            if r < 1e-9 {
                return vec![
                    DVec2::new(a / 2.0, bb / 2.0),
                    DVec2::new(-a / 2.0, bb / 2.0),
                    DVec2::new(-a / 2.0, -bb / 2.0),
                    DVec2::new(a / 2.0, -bb / 2.0),
                ];
            }
            let mut pts = Vec::new();
            for (k, (cx, cy)) in
                [(a / 2.0 - r, bb / 2.0 - r), (-a / 2.0 + r, bb / 2.0 - r), (-a / 2.0 + r, -bb / 2.0 + r), (a / 2.0 - r, -bb / 2.0 + r)]
                    .into_iter()
                    .enumerate()
            {
                for j in 0..=4 {
                    let ang = (k as f64 + j as f64 / 4.0) * std::f64::consts::FRAC_PI_2;
                    pts.push(DVec2::new(cx, cy) + DVec2::from_angle(ang) * r);
                }
            }
            pts
        };
        let c = c.min(w.min(h) * 0.45);
        let outer = rounded(w, h, c);
        let inner = rounded(w - 2.0 * b, h - 2.0 * b, if c > 0.0 { (c - b).max(c * 0.2) } else { 0.0 });
        let n = outer.len() as u32;
        let at = |q: &DVec2, z: f64| Vec3::new(q.x as f32, q.y as f32, z as f32);
        let mut pos: Vec<Vec3> = outer.iter().map(|q| at(q, 0.0)).collect();
        pos.extend(outer.iter().map(|q| at(q, t)));
        pos.extend(inner.iter().map(|q| at(q, t)));
        pos.extend(inner.iter().map(|q| at(q, t - d)));
        let mut sizes: Vec<u32> = Vec::new();
        let mut corners: Vec<u32> = Vec::new();
        let mut groups: BTreeMap<&'static str, Vec<u32>> = BTreeMap::new();
        let mut face = |name: &'static str, vs: Vec<u32>| {
            groups.entry(name).or_default().push(sizes.len() as u32);
            sizes.push(vs.len() as u32);
            corners.extend(vs);
        };
        face("back", (0..n).rev().collect());
        for i in 0..n {
            let j = (i + 1) % n;
            face("sides", vec![i, j, n + j, n + i]);
            face("border", vec![n + i, n + j, 2 * n + j, 2 * n + i]);
            face("recess", vec![2 * n + i, 2 * n + j, 3 * n + j, 3 * n + i]);
        }
        face("inner", (3 * n..4 * n).collect());
        let mut mesh = Mesh::from_polygons(pos, &sizes, &corners).map_err(|e| prim(PrimError::from(e)))?;
        for (name, list) in groups {
            let _ = mesh.set_group(name, ElementKind::Face, list);
        }
        let mut lm = BTreeMap::new();
        lm.insert("center".to_owned(), Vec3::new(0.0, 0.0, (t - d) as f32));
        lm.insert("back".to_owned(), Vec3::ZERO);
        done(mesh, lm)
    }
}

// ---- molding ---------------------------------------------------------------------

const PROFILES: &[&str] = &["ogee", "cove", "bead", "step", "ovolo"];

const MOLDING: &[ParamInfo] = &[
    ParamInfo {
        name: "profile",
        doc: "Cross section: ogee, cove, bead, step or ovolo.",
        kind: ParamKind::Choice(PROFILES),
        default: 0.0,
        min: 0.0,
        max: 4.0,
    },
    ParamInfo { name: "length", doc: "Run length along +X.", kind: ParamKind::Length, default: 1.0, min: 0.01, max: 100.0 },
    ParamInfo { name: "size", doc: "Height and depth of the cross section.", kind: ParamKind::Length, default: 0.04, min: 0.002, max: 5.0 },
    ParamInfo { name: "resolution", doc: "Points per curved part of the profile.", kind: ParamKind::Count, default: 6.0, min: 2.0, max: 32.0 },
];

/// A closed molding cross section in a unit square (x depth from the back
/// wall, y height from the floor): the floor, the face curve from the
/// front of the floor up to the top of the back wall, and the wall.
fn profile(kind: &str, res: usize) -> Vec<DVec2> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let arc = |c: DVec2, r: f64, a0: f64, a1: f64| -> Vec<DVec2> {
        (0..=res).map(|i| c + DVec2::from_angle(a0 + (a1 - a0) * i as f64 / res as f64) * r).collect()
    };
    let face: Vec<DVec2> = match kind {
        // A concave quarter circle.
        "cove" => arc(DVec2::new(1.0, 1.0), 1.0, -FRAC_PI_2, -PI),
        // A half round at the front.
        "bead" => {
            let mut f = arc(DVec2::new(0.5, 0.5), 0.5, -FRAC_PI_2, FRAC_PI_2);
            f.push(DVec2::new(0.0, 1.0));
            f
        }
        "step" => vec![DVec2::new(0.6, 0.0), DVec2::new(0.6, 0.4), DVec2::new(0.3, 0.4), DVec2::new(0.3, 1.0), DVec2::new(0.0, 1.0)],
        // A convex quarter circle.
        "ovolo" => arc(DVec2::ZERO, 1.0, 0.0, FRAC_PI_2),
        // ogee: concave below, convex above.
        _ => {
            let mut f = arc(DVec2::new(1.0, 0.5), 0.5, -FRAC_PI_2, -PI);
            f.extend(arc(DVec2::new(0.0, 0.5), 0.5, 0.0, FRAC_PI_2).into_iter().skip(1));
            f
        }
    };
    let mut out = vec![DVec2::ZERO];
    out.extend(face);
    out.dedup_by(|x, y| x.distance(*y) < 1e-9);
    if out.last().is_some_and(|p| p.distance(DVec2::ZERO) < 1e-9) {
        out.pop();
    }
    out
}

pub struct Molding;

impl Feature for Molding {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "molding",
            label: "Molding",
            summary: "A molding (ogee, cove, bead, step, ovolo) running along +X with its back against z = 0 and its bottom at y = 0.",
            params: MOLDING,
            landmarks: &[("start", "the start of the run (the origin)"), ("end", "the end of the run")],
            budget: 200,
            aliases: &["trim", "cornice", "baseboard", "crown molding", "profile"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let s = p.get("size");
        let sec: Vec<DVec2> = profile(p.choice("profile"), p.count("resolution").max(2)).into_iter().map(|q| DVec2::new(q.x * s, q.y * s)).collect();
        // The sweep's cross section is in (x, y) across the path: x is depth (z), y is height.
        let path = [DVec3::ZERO, DVec3::new(p.get("length"), 0.0, 0.0)];
        let mesh = sweep(&SweepSpec {
            profile: &sec,
            profile_closed: true,
            profile_spans: &[],
            path: &path,
            path_closed: false,
            twist: 0.0,
            scale: [1.0, 1.0],
            caps: SweepCaps::Fill,
            up: DVec3::Y,
        })
        .map_err(prim)?;
        let mut lm = BTreeMap::new();
        lm.insert("start".to_owned(), Vec3::ZERO);
        lm.insert("end".to_owned(), Vec3::new(p.f("length"), 0.0, 0.0));
        done(mesh, lm)
    }
}
