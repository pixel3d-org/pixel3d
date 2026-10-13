//! Flowers: rings of petal shells around a domed center on a curved stem,
//! with leaves along the stem. Petals are curved sheets (`gen.shell`):
//! one-sided cards, or closed shells with a thickness.

use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

use glam::{DVec2, Vec2, Vec3, Vec4};
use p3d_geo::generate::{Rng, ShellSpec, shell};
use p3d_mesh::Mesh;

use crate::VegError;
use crate::build::{Builder, Corner};

/// Petal outline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PetalShape {
    /// Broad and rounded (rose, poppy).
    Round,
    /// Widest low, pointed tip (lily, tulip).
    Pointed,
    /// Rounded with a notch at the tip (primrose).
    Heart,
    /// Long and narrow (daisy, sunflower).
    Strap,
}

impl PetalShape {
    pub const ALL: [PetalShape; 4] = [PetalShape::Round, PetalShape::Pointed, PetalShape::Heart, PetalShape::Strap];

    pub fn as_str(self) -> &'static str {
        match self {
            PetalShape::Round => "round",
            PetalShape::Pointed => "pointed",
            PetalShape::Heart => "heart",
            PetalShape::Strap => "strap",
        }
    }

    pub fn parse(s: &str) -> Option<PetalShape> {
        PetalShape::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Half width share at length share `y` (0 base, 1 tip).
    fn half_width(self, y: f32) -> f32 {
        let y = y.clamp(0.0, 1.0);
        match self {
            PetalShape::Round => (PI * y.powf(0.7)).sin().max(0.0).powf(0.7) * (0.35 + 0.65 * (y * 3.0).min(1.0)),
            PetalShape::Pointed => (PI * y.powf(0.75)).sin().max(0.0).powf(0.9),
            PetalShape::Heart => (PI * y.powf(0.65)).sin().max(0.0).powf(0.6),
            PetalShape::Strap => ((y * 5.0).min(1.0) * ((1.0 - y) * 7.0).min(1.0).sqrt()).max(0.0),
        }
    }
}

/// Everything that shapes a flower. Lengths in meters, angles in radians.
#[derive(Debug, Clone, PartialEq)]
pub struct FlowerSpec {
    pub petals: u32,
    /// Petal rings, outer first.
    pub layers: u32,
    pub petal_length: f32,
    pub petal_width: f32,
    pub shape: PetalShape,
    /// Cupping across the petal (0 flat, 1 deep).
    pub cup: f32,
    /// How open the bloom is (0 a closed bud, 1 flat open).
    pub open: f32,
    /// Petal thickness; 0 makes one-sided cards.
    pub thickness: f32,
    pub center_radius: f32,
    /// Dome height of the center.
    pub center_height: f32,
    pub stem_height: f32,
    pub stem_radius: f32,
    /// Curve of the stem (0 straight, 1 arching).
    pub stem_bend: f32,
    /// Tilt of the head away from the stem's top direction.
    pub head_tilt: f32,
    pub leaves: u32,
    pub leaf_length: f32,
    pub seed: u64,
}

impl Default for FlowerSpec {
    fn default() -> Self {
        FlowerSpec {
            petals: 8,
            layers: 1,
            petal_length: 0.035,
            petal_width: 0.018,
            shape: PetalShape::Round,
            cup: 0.3,
            open: 0.7,
            thickness: 0.0,
            center_radius: 0.008,
            center_height: 0.005,
            stem_height: 0.35,
            stem_radius: 0.0025,
            stem_bend: 0.25,
            head_tilt: 0.35,
            leaves: 2,
            leaf_length: 0.07,
            seed: 1,
        }
    }
}

/// A built flower: the mesh (material 0 stem and leaves, 1 petals, 2 the
/// center), groups `stem`, `leaves`, `petals`, `center`, and landmarks.
#[derive(Debug, Clone)]
pub struct Flower {
    pub mesh: Mesh,
    pub landmarks: BTreeMap<String, Vec3>,
}

fn check(name: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), VegError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(()) } else { Err(VegError::new(name, format!("{v} is outside {lo} to {hi}"))) }
}

/// Builds a flower.
///
/// Complexity: O(petals x petal faces + leaves x leaf faces + stem rings).
pub fn flower(spec: &FlowerSpec) -> Result<Flower, VegError> {
    if !(3..=60).contains(&spec.petals) || !(1..=4).contains(&spec.layers) || spec.leaves > 12 {
        return Err(VegError::new("petals", "use 3 to 60 petals, 1 to 4 layers and at most 12 leaves"));
    }
    check("petal_length", spec.petal_length, 1e-3, 2.0)?;
    check("petal_width", spec.petal_width, 5e-4, 2.0)?;
    check("cup", spec.cup, 0.0, 2.0)?;
    check("open", spec.open, 0.0, 1.0)?;
    check("thickness", spec.thickness, 0.0, spec.petal_width * 0.5)?;
    check("center_radius", spec.center_radius, 5e-4, 1.0)?;
    check("center_height", spec.center_height, 0.0, 1.0)?;
    check("stem_height", spec.stem_height, 0.0, 5.0)?;
    check("stem_radius", spec.stem_radius, 2e-4, 0.2)?;
    check("stem_bend", spec.stem_bend, 0.0, 1.0)?;
    check("head_tilt", spec.head_tilt, 0.0, PI / 2.0)?;
    check("leaf_length", spec.leaf_length, 1e-3, 2.0)?;
    let mut rng = Rng::new(spec.seed ^ 0xF10E_0006);
    let mut b = Builder::default();
    let g_stem = b.group("stem");
    let g_leaves = b.group("leaves");
    let g_petals = b.group("petals");
    let g_center = b.group("center");
    let g_cards = b.group(crate::groups::FOLIAGE);
    // Stem: a gentle arch toward +X.
    let h = spec.stem_height;
    let bend = spec.stem_bend * h * 0.3;
    let path: Vec<Vec3> = (0..=10)
        .map(|i| {
            let s = i as f32 / 10.0;
            Vec3::new(bend * s * s, h * s, 0.0)
        })
        .collect();
    let top = *path.last().unwrap_or(&Vec3::ZERO);
    let stem_dir = if h > 0.0 { (path[10] - path[9]).normalize_or(Vec3::Y) } else { Vec3::Y };
    let axis = rotate_toward_x(stem_dir, spec.head_tilt);
    if h > 0.0 {
        let radii: Vec<f32> = (0..=10).map(|i| spec.stem_radius * (1.0 - 0.25 * i as f32 / 10.0)).collect();
        sweep(&mut b, &path, &radii, 6, g_stem, 0);
    }
    // Leaves along the stem, turning by the golden angle.
    if spec.leaves > 0 && h > 0.0 {
        let outline = outline(PetalShape::Pointed, spec.leaf_length, spec.leaf_length * 0.32);
        let leaf = shell(&ShellSpec {
            contours: vec![outline],
            spacing: f64::from(spec.leaf_length) / 6.0,
            thickness: 0.0,
            cup: 0.35,
            bend: -0.12,
            twist: 0.0,
        })
        .map_err(|e| VegError::new("leaf_length", e.to_string()))?;
        for k in 0..spec.leaves {
            let s = 0.18 + 0.45 * (k as f32 + 0.5) / spec.leaves as f32;
            let i = ((s * 10.0) as usize).min(9);
            let at = path[i].lerp(path[i + 1], s * 10.0 - i as f32);
            let az = k as f32 * 2.39996 + rng.unit() * 0.4;
            let radial = Vec3::new(az.cos(), 0.0, az.sin());
            let dir = (radial * 0.82 + Vec3::Y * 0.57).normalize_or(radial);
            place(
                &mut b,
                &leaf,
                at + radial * spec.stem_radius * 0.5,
                dir,
                radial,
                spec.leaf_length,
                Vec3::new(0.85, 1.0, 0.85),
                &[g_leaves, g_cards],
                0,
                0.35,
            );
        }
    }
    // Center: a squashed dome.
    let center_base = top + axis * spec.center_height * 0.2;
    dome(&mut b, center_base, axis, spec.center_radius, spec.center_height.max(spec.center_radius * 0.15), g_center);
    // Petals: rings, outer first, each ring turned by half a petal.
    let outline_p = outline(spec.shape, spec.petal_length, spec.petal_width);
    let petal = shell(&ShellSpec {
        contours: vec![outline_p],
        spacing: f64::from((spec.petal_length / 6.0).max(spec.petal_width / 3.0)),
        thickness: f64::from(spec.thickness),
        cup: f64::from(spec.cup),
        bend: -0.1 * f64::from(spec.open),
        twist: 0.0,
    })
    .map_err(|e| VegError::new("petal_length", e.to_string()))?;
    let side = axis.any_orthonormal_vector();
    let side2 = axis.cross(side);
    let (solid, card) = ([g_petals], [g_petals, g_cards]);
    let petal_groups: &[usize] = if spec.thickness > 0.0 { &solid } else { &card };
    for layer in 0..spec.layers {
        let n = spec.petals;
        let shrink = 1.0 - 0.18 * layer as f32;
        let open = spec.open * 0.75f32.powi(layer as i32);
        let tilt = (12.0 + 83.0 * open).to_radians();
        for k in 0..n {
            let az = TAU * (k as f32 + 0.5 * layer as f32 + (rng.unit() - 0.5) * 0.15) / n as f32;
            let radial = side * az.cos() + side2 * az.sin();
            let dir = (axis * tilt.cos() + radial * tilt.sin()).normalize_or(radial);
            let at = center_base + radial * spec.center_radius * 0.75 * shrink - axis * spec.center_height * 0.1;
            let shade = 1.0 + 0.1 * (rng.unit() - 0.5) - 0.08 * layer as f32;
            place(&mut b, &petal, at, dir, radial, spec.petal_length * shrink, Vec3::splat(shade), petal_groups, 1, 0.1);
        }
    }
    let mut mesh = b.finish()?;
    let mut landmarks = BTreeMap::new();
    landmarks.insert("base".to_owned(), Vec3::ZERO);
    landmarks.insert("stem_top".to_owned(), top);
    landmarks.insert("head".to_owned(), center_base + axis * spec.center_height);
    for (name, p) in &landmarks {
        let v =
            mesh.verts().min_by(|&a, &c| mesh.positions[a as usize].distance_squared(*p).total_cmp(&mesh.positions[c as usize].distance_squared(*p)));
        if let Some(v) = v {
            // A fresh one-vertex group on a live mesh cannot fail.
            let _ = mesh.set_group(&format!("lm.{name}"), p3d_mesh::ElementKind::Vert, vec![v]);
        }
    }
    Ok(Flower { mesh, landmarks })
}

/// `v` turned by `angle` toward +X (about the axis perpendicular to both).
fn rotate_toward_x(v: Vec3, angle: f32) -> Vec3 {
    let axis = v.cross(Vec3::X).normalize_or(Vec3::Z);
    (glam::Quat::from_axis_angle(axis, -angle) * v).normalize_or(v)
}

/// A petal or leaf outline: x across, y along, base at the origin.
///
/// Complexity: O(1) (24 points).
fn outline(shape: PetalShape, length: f32, width: f32) -> Vec<DVec2> {
    const N: usize = 12;
    let mut right = Vec::with_capacity(N);
    for i in 0..=N {
        let y = i as f32 / N as f32;
        let mut w = shape.half_width(y) * width * 0.5;
        if i == 0 {
            w = width * 0.06;
        }
        right.push(DVec2::new(f64::from(w), f64::from(y * length)));
    }
    let mut pts: Vec<DVec2> = right.iter().copied().filter(|p| p.x > 1e-9).collect();
    // Heart petals dip at the tip.
    let tip = if shape == PetalShape::Heart { DVec2::new(0.0, f64::from(length * 0.86)) } else { DVec2::new(0.0, f64::from(length)) };
    pts.push(tip);
    pts.extend(right.iter().rev().filter(|p| p.x > 1e-9).map(|p| DVec2::new(-p.x, p.y)));
    pts.dedup_by(|a, b| a.distance(*b) < 1e-9);
    pts
}

/// Copies a sheet (x across, y along, front +Z) into place: its y along
/// `dir`, its front toward the head axis and up.
///
/// Complexity: O(sheet vertices + corners).
#[allow(clippy::too_many_arguments)]
fn place(b: &mut Builder, sheet: &Mesh, at: Vec3, dir: Vec3, radial: Vec3, length: f32, tint: Vec3, groups: &[usize], material: u16, stiff: f32) {
    let right = radial.cross(dir).normalize_or(dir.any_orthonormal_vector());
    let front = right.cross(dir).normalize_or(Vec3::Y);
    let (lo, hi) = sheet.bounds().unwrap_or((Vec3::ZERO, Vec3::ONE));
    let scale = length / (hi.y - lo.y).max(1e-6);
    let width = (hi.x - lo.x).max(1e-6);
    let mut map = vec![u32::MAX; sheet.vert_slots()];
    for v in sheet.verts() {
        let p = sheet.positions[v as usize];
        map[v as usize] = b.vert(at + (right * p.x + dir * (p.y - lo.y) + front * p.z) * scale);
    }
    for f in sheet.faces() {
        let corners: Vec<Corner> = sheet
            .face_verts(f)
            .map(|v| {
                let p = sheet.positions[v as usize];
                let along = (p.y - lo.y) / (hi.y - lo.y).max(1e-6);
                let shade = tint * (0.78 + 0.22 * along);
                Corner {
                    v: map[v as usize],
                    uv: Vec2::new((p.x - lo.x) / width, along),
                    wind: Vec2::new(0.0, stiff * (1.0 - along)),
                    color: Vec4::new(shade.x, shade.y, shade.z, 1.0),
                    normal: Vec3::ZERO,
                }
            })
            .collect();
        b.face(&corners, groups, material);
    }
}

/// A closed tube along `path` with a flat cap at each end.
///
/// Complexity: O(points x sides).
fn sweep(b: &mut Builder, path: &[Vec3], radii: &[f32], sides: usize, group: usize, material: u16) {
    let fr = crate::wood::frames(path);
    let first = b.pos.len() as u32;
    for (p, (&r, &(_, u, v))) in path.iter().zip(radii.iter().zip(&fr)) {
        for k in 0..sides {
            let a = k as f32 / sides as f32 * TAU;
            b.vert(*p + (u * a.cos() + v * a.sin()) * r);
        }
    }
    let n = path.len();
    let c = |v: u32, u: f32, t: f32| Corner { v, uv: Vec2::new(u, t), wind: Vec2::new(0.0, 0.6 * (1.0 - t)), color: Vec4::ONE, normal: Vec3::ZERO };
    for i in 0..n - 1 {
        let (t0, t1) = (i as f32 / (n - 1) as f32, (i + 1) as f32 / (n - 1) as f32);
        for k in 0..sides {
            let k1 = (k + 1) % sides;
            let (a, bb) = (first + (i * sides + k) as u32, first + (i * sides + k1) as u32);
            let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
            b.face(&[c(a, u0, t0), c(bb, u1, t0), c(bb + sides as u32, u1, t1), c(a + sides as u32, u0, t1)], &[group], material);
        }
    }
    let bottom: Vec<Corner> = (0..sides).rev().map(|k| c(first + k as u32, 0.0, 0.0)).collect();
    b.face(&bottom, &[group], material);
    let last = first + ((n - 1) * sides) as u32;
    let top: Vec<Corner> = (0..sides).map(|k| c(last + k as u32, 0.0, 1.0)).collect();
    b.face(&top, &[group], material);
}

/// A closed dome: rings of a squashed half sphere over a flat base.
///
/// Complexity: O(1) (6 rings of 12).
fn dome(b: &mut Builder, base: Vec3, axis: Vec3, radius: f32, height: f32, group: usize) {
    const RINGS: usize = 5;
    const SIDES: usize = 12;
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let first = b.pos.len() as u32;
    for i in 0..RINGS {
        let a = i as f32 / RINGS as f32 * PI * 0.5;
        let (r, y) = (radius * a.cos(), height * a.sin());
        for k in 0..SIDES {
            // Counter-clockwise seen along the axis, so faces point out.
            let t = k as f32 / SIDES as f32 * TAU;
            b.vert(base + axis * y + (u * t.cos() + v * t.sin()) * r);
        }
    }
    let apex = b.vert(base + axis * height);
    let c = |v: u32| Corner { v, uv: Vec2::ZERO, wind: Vec2::new(0.0, 0.1), color: Vec4::ONE, normal: Vec3::ZERO };
    for i in 0..RINGS - 1 {
        for k in 0..SIDES {
            let k1 = (k + 1) % SIDES;
            let (a, bb) = (first + (i * SIDES + k) as u32, first + (i * SIDES + k1) as u32);
            b.face(&[c(a), c(bb), c(bb + SIDES as u32), c(a + SIDES as u32)], &[group], 2);
        }
    }
    let last = first + ((RINGS - 1) * SIDES) as u32;
    for k in 0..SIDES {
        b.face(&[c(last + k as u32), c(last + ((k + 1) % SIDES) as u32), c(apex)], &[group], 2);
    }
    let bottom: Vec<Corner> = (0..SIDES).rev().map(|k| c(first + k as u32)).collect();
    b.face(&bottom, &[group], 2);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn flowers_have_their_parts_and_landmarks() {
        for shape in PetalShape::ALL {
            let f = flower(&FlowerSpec { shape, layers: 2, ..FlowerSpec::default() }).unwrap();
            f.mesh.validate().unwrap();
            for g in ["stem", "leaves", "petals", "center", "lm.head", "lm.base"] {
                assert!(f.mesh.group(g).is_some(), "{shape:?} {g}");
            }
            let (_, hi) = f.mesh.bounds().unwrap();
            assert!(hi.y > 0.3, "{hi:?}");
        }
    }
}
