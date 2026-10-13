//! `wing`: a feathered or membrane wing (ADR 0011 wings). A left wing: the
//! shoulder at the origin, the span along +X, the leading edge toward +Z
//! and the top toward +Y; `side: right` builds the mirror image.
//!
//! - Feathered: an arm (humerus, forearm, hand) as a tapered lens tube,
//!   primaries fanned along the hand, secondaries along the forearm,
//!   tertials at the elbow and rows of coverts over their roots, each
//!   feather a curved, cambered slab (or a card) with a raised shaft.
//! - Membrane: the arm and three to five finger bones, and a membrane
//!   shell between them whose trailing edge sags between the finger tips
//!   back to the body.
//!
//! [`bones`] gives the joint positions the archetype skeleton uses.

use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use p3d_mesh::{ElementKind, Mesh};

use crate::tube::{TubeSpec, tube};
use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

const TYPES: &[&str] = &["feathered", "membrane"];
const FEATHERS: &[&str] = &["solid", "cards"];
const SIDES: &[&str] = &["left", "right"];

const PARAMS: &[ParamInfo] = &[
    ParamInfo {
        name: "type",
        doc: "feathered (bird, angel, griffin) or membrane (bat, dragon, demon).",
        kind: ParamKind::Choice(TYPES),
        default: 0.0,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo { name: "span", doc: "Shoulder to wing tip, spread.", kind: ParamKind::Length, default: 1.2, min: 0.05, max: 20.0 },
    ParamInfo { name: "fold", doc: "0 spread to 1 folded against the back.", kind: ParamKind::Ratio, default: 0.0, min: 0.0, max: 1.0 },
    ParamInfo { name: "raise", doc: "Lift of the wing above level (degrees).", kind: ParamKind::Angle, default: 15.0, min: -45.0, max: 80.0 },
    ParamInfo {
        name: "tilt",
        doc: "Turn about the span so the trailing edge hangs down (degrees): 0 in flight, about 70 for wings spread on display behind a body.",
        kind: ParamKind::Angle,
        default: 0.0,
        min: -90.0,
        max: 90.0,
    },
    ParamInfo { name: "sweep", doc: "How far the hand and tip sweep back (degrees).", kind: ParamKind::Angle, default: 18.0, min: -20.0, max: 70.0 },
    ParamInfo {
        name: "chord",
        doc: "Wing depth (the longest feathers or the membrane) as a share of the span.",
        kind: ParamKind::Ratio,
        default: 0.42,
        min: 0.15,
        max: 0.9,
    },
    ParamInfo { name: "primaries", doc: "Flight feathers along the hand.", kind: ParamKind::Count, default: 10.0, min: 3.0, max: 16.0 },
    ParamInfo { name: "secondaries", doc: "Flight feathers along the forearm.", kind: ParamKind::Count, default: 12.0, min: 3.0, max: 30.0 },
    ParamInfo {
        name: "coverts",
        doc: "Rows of covert feathers over the flight feather roots.",
        kind: ParamKind::Count,
        default: 2.0,
        min: 0.0,
        max: 3.0,
    },
    ParamInfo {
        name: "feathers",
        doc: "solid (closed thin slabs) or cards (open strips for alpha textures).",
        kind: ParamKind::Choice(FEATHERS),
        default: 0.0,
        min: 0.0,
        max: 1.0,
    },
    ParamInfo { name: "fingers", doc: "Membrane: finger bones fanning from the wrist.", kind: ParamKind::Count, default: 4.0, min: 3.0, max: 5.0 },
    ParamInfo {
        name: "scallop",
        doc: "Membrane: how far the trailing edge sags between finger tips (share of their distance).",
        kind: ParamKind::Ratio,
        default: 0.32,
        min: 0.0,
        max: 0.5,
    },
    ParamInfo {
        name: "bone",
        doc: "Arm thickness at the shoulder as a share of the span.",
        kind: ParamKind::Ratio,
        default: 0.035,
        min: 0.01,
        max: 0.12,
    },
    ParamInfo { name: "side", doc: "left (spans +X) or right (the mirror image).", kind: ParamKind::Choice(SIDES), default: 0.0, min: 0.0, max: 1.0 },
];

/// The arm's joints of a left wing in feature space: shoulder, elbow,
/// wrist, tip and (membrane) the finger tips.
#[derive(Debug, Clone, PartialEq)]
pub struct Bones {
    pub shoulder: Vec3,
    pub elbow: Vec3,
    pub wrist: Vec3,
    pub tip: Vec3,
    pub fingers: Vec<Vec3>,
    /// The wing's up (normal of its plane).
    pub up: Vec3,
    /// Toward the trailing edge, in the wing's plane.
    pub back: Vec3,
}

/// A heading in the wing plane: toward +X, turned toward +Z by `deg`.
fn heading(deg: f32) -> Vec3 {
    let a = deg.to_radians();
    Vec3::new(a.cos(), 0.0, a.sin())
}

/// The bones of a wing from its parameters (left side).
///
/// Complexity: O(fingers).
pub fn bones(p: &Params) -> Bones {
    let span = p.f("span");
    let fold = p.f("fold");
    let sweep = p.f("sweep").to_degrees();
    let lerp = |a: f32, b: f32| a + (b - a) * fold;
    let membrane = p.choice("type") == "membrane";
    let (hu, fo, ha) = if membrane { (0.26, 0.36, 0.38) } else { (0.27, 0.33, 0.4) };
    let shoulder = Vec3::ZERO;
    let elbow = shoulder + heading(lerp(-18.0, -60.0)) * span * hu;
    let wrist = elbow + heading(lerp(8.0, 100.0)) * span * fo;
    let hand = heading(lerp(-sweep, -95.0));
    let tip = wrist + hand * span * ha;
    let mut fingers = Vec::new();
    if membrane {
        let n = p.count("fingers").clamp(3, 5);
        for i in 0..n {
            // From along the leading edge to back toward the body.
            let t = i as f32 / (n - 1) as f32;
            let turn = lerp(-sweep * 0.5, -95.0) - t * lerp(115.0, 15.0);
            let len = span * ha * (1.25 - 0.45 * t);
            fingers.push(wrist + heading(turn) * len);
        }
    }
    // Tilt the trailing edge down about the span, then lift the wing about
    // the shoulder's forward axis.
    let lift = Quat::from_rotation_z(p.f("raise") * (1.0 - 0.7 * fold)) * Quat::from_rotation_x(-p.f("tilt"));
    let up = lift * Vec3::Y;
    Bones {
        back: lift * Vec3::NEG_Z,
        shoulder,
        elbow: lift * elbow,
        wrist: lift * wrist,
        tip: lift * if membrane { fingers.first().copied().unwrap_or(tip) } else { tip },
        fingers: fingers.iter().map(|f| lift * *f).collect(),
        up,
    }
}

/// Closed slabs and cards under construction.
#[derive(Default)]
struct Sheet {
    pos: Vec<Vec3>,
    sizes: Vec<u32>,
    corners: Vec<u32>,
    groups: BTreeMap<&'static str, Vec<u32>>,
}

impl Sheet {
    fn face(&mut self, group: &'static str, c: &[u32]) {
        self.groups.entry(group).or_default().push(self.sizes.len() as u32);
        self.sizes.push(c.len() as u32);
        self.corners.extend_from_slice(c);
    }

    /// One feather from `root` along `dir` with the wing's `up`: rows of
    /// (left edge, shaft, right edge), cambered down toward the tip. Solid
    /// feathers are closed slabs; cards are their top surface.
    ///
    /// Complexity: O(rows).
    #[allow(clippy::too_many_arguments)]
    fn feather(&mut self, group: &'static str, root: Vec3, dir: Vec3, up: Vec3, len: f32, width: f32, outer: f32, solid: bool, thick: f32) {
        const ROWS: usize = 7;
        let d = dir.normalize_or(Vec3::NEG_Z);
        let across = d.cross(up).normalize_or(d.any_orthonormal_vector());
        let n = across.cross(d).normalize_or(up);
        let base = self.pos.len() as u32;
        for i in 0..ROWS {
            let s = i as f32 / (ROWS - 1) as f32;
            // Narrow quill, full vane, a rounded tip.
            let w = width * (0.35 + s / 0.25).min(1.0) * (1.0 - ((s - 0.78).max(0.0) / 0.22).powi(2)).max(0.0).sqrt().max(0.25);
            let c = root + d * (len * s) - n * (0.12 * len * s * s);
            let lift = if solid { thick * 0.5 } else { 0.0 };
            let shaft = n * (thick * 0.6);
            self.pos.extend([c - across * (w * (1.0 - outer)) + n * lift, c + shaft + n * lift, c + across * (w * outer) + n * lift]);
            if solid {
                self.pos.extend([c - across * (w * (1.0 - outer)) - n * lift, c - n * lift, c + across * (w * outer) - n * lift]);
            }
        }
        let stride = if solid { 6u32 } else { 3 };
        let at = |row: usize, k: u32| base + row as u32 * stride + k;
        for i in 0..ROWS - 1 {
            // Top: left half, right half (counter-clockwise seen from n).
            self.face(group, &[at(i, 0), at(i, 1), at(i + 1, 1), at(i + 1, 0)]);
            self.face(group, &[at(i, 1), at(i, 2), at(i + 1, 2), at(i + 1, 1)]);
            if solid {
                self.face(group, &[at(i, 3), at(i + 1, 3), at(i + 1, 4), at(i, 4)]);
                self.face(group, &[at(i, 4), at(i + 1, 4), at(i + 1, 5), at(i, 5)]);
                self.face(group, &[at(i, 0), at(i + 1, 0), at(i + 1, 3), at(i, 3)]);
                self.face(group, &[at(i, 2), at(i, 5), at(i + 1, 5), at(i + 1, 2)]);
            }
        }
        if solid {
            let (r, t) = (0, ROWS - 1);
            self.face(group, &[at(r, 0), at(r, 3), at(r, 4), at(r, 1)]);
            self.face(group, &[at(r, 1), at(r, 4), at(r, 5), at(r, 2)]);
            self.face(group, &[at(t, 0), at(t, 1), at(t, 4), at(t, 3)]);
            self.face(group, &[at(t, 1), at(t, 2), at(t, 5), at(t, 4)]);
        }
    }

    /// A closed thin slab over a grid of points (rows of equal length):
    /// the grid offset half the thickness each way along `up`, the top
    /// facing `up`, joined by walls around the rim.
    ///
    /// Complexity: O(grid points).
    fn slab(&mut self, group: &'static str, grid: &[Vec<Vec3>], up: Vec3, thick: f32) {
        let (rows, cols) = (grid.len(), grid.first().map_or(0, Vec::len));
        if rows < 2 || cols < 2 {
            return;
        }
        // Which winding faces up: the first quad's normal decides.
        let n0 = (grid[0][1] - grid[0][0]).cross(grid[1][1] - grid[0][1]);
        let flip = n0.dot(up) < 0.0;
        let top = self.pos.len() as u32;
        for row in grid {
            self.pos.extend(row.iter().map(|p| *p + up * (thick * 0.5)));
        }
        let bottom = self.pos.len() as u32;
        for row in grid {
            self.pos.extend(row.iter().map(|p| *p - up * (thick * 0.5)));
        }
        let at = |base: u32, r: usize, c: usize| base + (r * cols + c) as u32;
        let mut quad = |q: [u32; 4], down: bool| {
            if down != flip {
                self.face(group, &[q[3], q[2], q[1], q[0]]);
            } else {
                self.face(group, &q);
            }
        };
        for r in 0..rows - 1 {
            for c in 0..cols - 1 {
                quad([at(top, r, c), at(top, r, c + 1), at(top, r + 1, c + 1), at(top, r + 1, c)], false);
                quad([at(bottom, r, c), at(bottom, r, c + 1), at(bottom, r + 1, c + 1), at(bottom, r + 1, c)], true);
            }
        }
        // The rim, walked around the grid's border in the top's winding.
        let mut rim: Vec<(usize, usize)> = (0..cols - 1).map(|c| (0, c)).collect();
        rim.extend((0..rows - 1).map(|r| (r, cols - 1)));
        rim.extend((1..cols).rev().map(|c| (rows - 1, c)));
        rim.extend((1..rows).rev().map(|r| (r, 0)));
        for i in 0..rim.len() {
            let (a, b) = (rim[i], rim[(i + 1) % rim.len()]);
            quad([at(top, b.0, b.1), at(top, a.0, a.1), at(bottom, a.0, a.1), at(bottom, b.0, b.1)], false);
        }
    }

    fn finish(self) -> Result<Mesh, FeatureError> {
        let err = |e: String| FeatureError { path: "/params".into(), message: e, suggestions: Vec::new() };
        let mut m = Mesh::from_polygons(self.pos, &self.sizes, &self.corners).map_err(|e| err(e.to_string()))?;
        for (g, faces) in self.groups {
            m.set_group(g, ElementKind::Face, faces).map_err(|e| err(e.to_string()))?;
        }
        Ok(m)
    }
}

/// Points along a polyline at even shares of its length.
fn along(path: &[Vec3], t: f32) -> Vec3 {
    let total: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut want = t.clamp(0.0, 1.0) * total;
    for w in path.windows(2) {
        let l = w[0].distance(w[1]);
        if want <= l || l <= 0.0 {
            return w[0].lerp(w[1], if l > 0.0 { want / l } else { 0.0 });
        }
        want -= l;
    }
    path.last().copied().unwrap_or(Vec3::ZERO)
}

/// A tapered lens tube along a bone chain.
fn arm(chain: &[Vec3], r0: f32, r1: f32, up: Vec3) -> Result<Mesh, FeatureError> {
    let n = 24;
    let path: Vec<Vec3> = (0..=n).map(|i| along(chain, i as f32 / n as f32)).collect();
    let radius: Vec<f32> = (0..=n).map(|i| r0 + (r1 - r0) * (i as f32 / n as f32).powf(0.8)).collect();
    let mut m = tube(&TubeSpec { path: &path, radius: &radius, sides: 10, pointed: true })?;
    // Flatten across the wing plane: the arm is a lens, thin top to bottom.
    let verts: Vec<u32> = m.verts().collect();
    for v in verts {
        let p = m.positions[v as usize];
        let i = path.iter().enumerate().min_by(|a, b| a.1.distance_squared(p).total_cmp(&b.1.distance_squared(p))).map_or(0, |(i, _)| i);
        let off = p - path[i];
        m.positions[v as usize] = path[i] + off - up * (off.dot(up) * 0.45);
    }
    let all: Vec<u32> = m.faces().collect();
    let _ = m.set_group("bones", ElementKind::Face, all);
    Ok(m)
}

pub struct Wing;

impl Feature for Wing {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "wing",
            label: "Wing",
            summary: "A feathered wing (primaries, secondaries, tertials and covert rows as cambered feathers on a lens arm) or a membrane wing (finger bones and a scalloped membrane); shoulder at the origin, spanning +X.",
            params: PARAMS,
            landmarks: &[
                ("root", "the shoulder (the origin)"),
                ("elbow", "the elbow"),
                ("wrist", "the wrist"),
                ("tip", "the wing tip"),
                ("trailing", "the trailing edge behind the wrist"),
            ],
            budget: 6000,
            aliases: &["wings", "feathers", "bat wing", "dragon wing", "angel wing", "bird wing"],
            sdf: false,
        }
    }

    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let b = bones(p);
        let span = p.f("span");
        let chord = p.f("chord") * span;
        let up = b.up;
        let r0 = p.f("bone") * span;
        let mut mesh = arm(&[b.shoulder, b.elbow, b.wrist, b.tip], r0, r0 * 0.25, up)?;
        // The trailing direction: back in the wing plane, away from the leading edge.
        let back = |bone: Vec3| {
            let d = bone.normalize_or(Vec3::X);
            let t = up.cross(d).normalize_or(b.back);
            if t.dot(b.back) < 0.0 { -t } else { t }
        };
        let mut trailing = b.wrist + back(b.wrist - b.elbow) * chord * 0.7;
        if p.choice("type") == "membrane" {
            // Finger bones, then the membrane between them as a shell.
            for f in &b.fingers {
                let path: Vec<Vec3> = (0..=8).map(|i| b.wrist.lerp(*f, i as f32 / 8.0)).collect();
                let radius: Vec<f32> = (0..=8).map(|i| r0 * 0.55 * (1.0 - 0.7 * i as f32 / 8.0)).collect();
                let mut finger = tube(&TubeSpec { path: &path, radius: &radius, sides: 8, pointed: true })?;
                let all: Vec<u32> = finger.faces().collect();
                let _ = finger.set_group("bones", ElementKind::Face, all);
                mesh.append(&finger);
            }
            // The membrane: ruled panels between neighbouring fingers (from just
            // off the wrist), and one from the arm back to the body; trailing
            // edges sag toward the wrist between the tips.
            let sc = p.f("scallop");
            let thick = (r0 * 0.12).max(0.0004);
            let body = b.shoulder + back(b.elbow - b.shoulder) * chord * 1.15 - (b.elbow - b.shoulder).normalize_or(Vec3::X) * chord * 0.05;
            let sag = |a: Vec3, c: Vec3, t: f32| (b.wrist - a.lerp(c, t)) * (sc * 4.0 * t * (1.0 - t) * 0.5);
            let mut sheet = Sheet::default();
            for w in b.fingers.windows(2) {
                let (fa, fb) = (w[0] - b.wrist, w[1] - b.wrist);
                let grid: Vec<Vec<Vec3>> = (0..=8)
                    .map(|j| {
                        let r = 0.04 + 0.96 * j as f32 / 8.0;
                        (0..=6)
                            .map(|k| {
                                let t = k as f32 / 6.0;
                                b.wrist + fa.lerp(fb, t) * r + sag(w[0], w[1], t) * (r * r)
                            })
                            .collect()
                    })
                    .collect();
                sheet.slab("membrane", &grid, up, thick);
            }
            if let Some(&last) = b.fingers.last() {
                // From the wrist back along the arm to the shoulder, and from the
                // last finger tip to the body.
                let lead = [b.wrist, b.elbow, b.shoulder];
                let grid: Vec<Vec<Vec3>> = (0..=12)
                    .map(|i| {
                        let u = i as f32 / 12.0;
                        let l = along(&lead, u);
                        let tr = last.lerp(body, u) + sag(last, body, u);
                        (0..=6).map(|k| l.lerp(tr, k as f32 / 6.0)).collect()
                    })
                    .collect();
                sheet.slab("membrane", &grid, up, thick);
            }
            mesh.append(&sheet.finish()?);
            let mut tips = b.fingers.clone();
            tips.push(body);
            trailing = tips.get(tips.len() / 2).copied().unwrap_or(trailing);
        } else {
            let solid = p.choice("feathers") == "solid";
            let thick = (span * 0.004).max(0.0005);
            let mut s = Sheet::default();
            // Primaries: along the hand, fanning from trailing to along the hand.
            let np = p.count("primaries").max(3);
            let hand = b.tip - b.wrist;
            for i in 0..np {
                let t = (i as f32 + 0.5) / np as f32;
                let root = b.wrist + hand * (t * 0.85);
                let dir = back(hand).lerp(hand.normalize_or(Vec3::X), 0.15 + 0.75 * t * t);
                let len = chord * (0.85 + 0.35 * t);
                let lift = up * (thick * 1.2 * (np - i) as f32);
                s.feather("primaries", root + lift, dir, up, len, len * 0.2, 0.32, solid, thick);
            }
            // Secondaries: along the forearm, pointing back.
            let ns = p.count("secondaries").max(3);
            for i in 0..ns {
                let t = (i as f32 + 0.5) / ns as f32;
                let root = b.elbow.lerp(b.wrist, t);
                let dir = back(b.wrist - b.elbow) + (b.wrist - b.elbow).normalize_or(Vec3::X) * 0.15 * t;
                let len = chord * (0.72 + 0.08 * t);
                s.feather("secondaries", root + up * (thick * 1.1 * i as f32), dir, up, len, len * 0.27, 0.4, solid, thick);
            }
            // Tertials near the body, shorter and broad.
            for i in 0..4 {
                let t = (i as f32 + 0.5) / 4.0;
                let root = b.shoulder.lerp(b.elbow, 0.45 + 0.5 * t);
                let len = chord * (0.45 + 0.15 * t);
                s.feather("secondaries", root + up * (thick * 1.5), back(b.elbow - b.shoulder), up, len, len * 0.3, 0.5, solid, thick);
            }
            // Coverts: rows over the flight feather roots, shorter each row.
            let rows = p.count("coverts").min(3);
            let chain = [b.shoulder, b.elbow, b.wrist, b.tip];
            for r in 0..rows {
                let n = (np + ns) * 3 / 4 + 4;
                let share = [0.38f32, 0.24, 0.14][r];
                for i in 0..n {
                    let t = (i as f32 + 0.5) / n as f32 * 0.92;
                    let at = along(&chain, t);
                    let next = along(&chain, (t + 0.02).min(1.0));
                    let dir = back(next - at) + (next - at).normalize_or(Vec3::X) * 0.25 * t;
                    let len = chord * share * (1.0 - 0.35 * t);
                    let root = at + up * (thick * (6.0 + 3.0 * r as f32) + r0 * 0.3) - back(next - at) * (chord * 0.04 * r as f32);
                    s.feather("coverts", root, dir, up, len, len * 0.42, 0.45, solid, thick);
                }
            }
            mesh.append(&s.finish()?);
        }
        let mut landmarks = BTreeMap::new();
        landmarks.insert("root".to_owned(), b.shoulder);
        landmarks.insert("elbow".to_owned(), b.elbow);
        landmarks.insert("wrist".to_owned(), b.wrist);
        landmarks.insert("tip".to_owned(), b.tip);
        landmarks.insert("trailing".to_owned(), trailing);
        crate::bake_landmarks(&mut mesh, &landmarks);
        if p.choice("side") == "right" {
            mesh.transform(glam::Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
            for v in landmarks.values_mut() {
                v.x = -v.x;
            }
        }
        Ok(Built { shape: Shape::Mesh(mesh), landmarks })
    }
}
