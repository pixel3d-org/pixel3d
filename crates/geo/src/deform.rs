//! Deformers: bend, twist, taper and noise displacement of vertex positions.
//!
//! Bend, twist and taper progress along an axis through the object origin:
//! `t` runs from 0 at the lowest affected vertex along the axis to 1 at the
//! highest.

use glam::{Quat, Vec3};
use p3d_mesh::Mesh;

use crate::edit::KernelError;

/// An object-space axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DeformAxis {
    X,
    #[default]
    Y,
    Z,
}

impl DeformAxis {
    pub fn index(self) -> usize {
        match self {
            DeformAxis::X => 0,
            DeformAxis::Y => 1,
            DeformAxis::Z => 2,
        }
    }

    pub fn unit(self) -> Vec3 {
        let mut v = Vec3::ZERO;
        v[self.index()] = 1.0;
        v
    }
}

/// Live, deduplicated vertices and their extent along an axis.
///
/// Complexity: O(k log k).
fn extent(mesh: &Mesh, verts: &[u32], axis: DeformAxis) -> Result<(Vec<u32>, f32, f32), KernelError> {
    let mut vs: Vec<u32> = verts.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
    vs.sort_unstable();
    vs.dedup();
    if vs.is_empty() {
        return Err(KernelError::Empty("no vertices selected".into()));
    }
    let i = axis.index();
    let (lo, hi) = vs.iter().map(|&v| mesh.positions[v as usize][i]).fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)));
    Ok((vs, lo, hi))
}

fn finite(param: &'static str, x: f32) -> Result<(), KernelError> {
    if x.is_finite() { Ok(()) } else { Err(KernelError::OutOfRange { param, message: format!("{param} must be finite"), hint: String::new() }) }
}

/// Twists vertices around the axis line through the origin: the rotation
/// grows from 0 at the low end to `angle` (radians) at the high end.
/// Returns the number of vertices moved.
///
/// Complexity: O(k log k) for k vertices.
pub fn twist(mesh: &mut Mesh, verts: &[u32], axis: DeformAxis, angle: f32) -> Result<usize, KernelError> {
    finite("angle", angle)?;
    let (vs, lo, hi) = extent(mesh, verts, axis)?;
    let len = (hi - lo).max(1e-12);
    let i = axis.index();
    for &v in &vs {
        let p = mesh.positions[v as usize];
        let t = (p[i] - lo) / len;
        mesh.positions[v as usize] = Quat::from_axis_angle(axis.unit(), angle * t) * p;
    }
    mesh.revision += 1;
    Ok(vs.len())
}

/// Scales vertices toward the axis line through the origin: the scale grows
/// from 1 at the low end to `factor` at the high end.
///
/// Complexity: O(k log k) for k vertices.
pub fn taper(mesh: &mut Mesh, verts: &[u32], axis: DeformAxis, factor: f32) -> Result<usize, KernelError> {
    if !(factor.is_finite() && factor >= 0.0) {
        return Err(KernelError::OutOfRange {
            param: "factor",
            message: format!("taper factor {factor} must be 0 or more"),
            hint: "0.5 halves the far end".into(),
        });
    }
    let (vs, lo, hi) = extent(mesh, verts, axis)?;
    let len = (hi - lo).max(1e-12);
    let i = axis.index();
    for &v in &vs {
        let p = mesh.positions[v as usize];
        let t = (p[i] - lo) / len;
        let s = 1.0 + (factor - 1.0) * t;
        let mut q = p * s;
        q[i] = p[i];
        mesh.positions[v as usize] = q;
    }
    mesh.revision += 1;
    Ok(vs.len())
}

/// Bends vertices: the stretch along `axis` (from its low end) curls onto a
/// circular arc toward `toward`, turning by `angle` (radians) over the
/// whole length; coordinates along the third axis stay.
///
/// Complexity: O(k log k) for k vertices.
pub fn bend(mesh: &mut Mesh, verts: &[u32], axis: DeformAxis, toward: DeformAxis, angle: f32) -> Result<usize, KernelError> {
    finite("angle", angle)?;
    if axis == toward {
        return Err(KernelError::OutOfRange {
            param: "toward",
            message: "the bend direction must differ from the axis".into(),
            hint: "for a Y axis bend toward x or z".into(),
        });
    }
    let (vs, lo, hi) = extent(mesh, verts, axis)?;
    if angle.abs() < 1e-6 {
        return Ok(0);
    }
    let len = (hi - lo).max(1e-12);
    let r = len / angle;
    let (i, j) = (axis.index(), toward.index());
    for &v in &vs {
        let p = mesh.positions[v as usize];
        let (a, d) = (p[i], p[j]);
        let phi = (a - lo) / r;
        let rr = r - d;
        let mut q = p;
        q[i] = lo + rr * phi.sin();
        q[j] = r - rr * phi.cos();
        mesh.positions[v as usize] = q;
    }
    mesh.revision += 1;
    Ok(vs.len())
}

/// A hash of lattice coordinates and a seed (well mixed, deterministic).
#[inline]
fn hash(seed: u64, x: i32, y: i32, z: i32) -> u32 {
    let mut h = seed
        ^ (x as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u32 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    h as u32
}

/// Gradient for a lattice point: one of the 12 cube edge directions.
#[inline]
fn grad(h: u32, d: Vec3) -> f32 {
    match h % 12 {
        0 => d.x + d.y,
        1 => -d.x + d.y,
        2 => d.x - d.y,
        3 => -d.x - d.y,
        4 => d.x + d.z,
        5 => -d.x + d.z,
        6 => d.x - d.z,
        7 => -d.x - d.z,
        8 => d.y + d.z,
        9 => -d.y + d.z,
        10 => d.y - d.z,
        _ => -d.y - d.z,
    }
}

/// Seeded 3D gradient noise (Perlin's improved noise), about -1 to 1.
///
/// Complexity: O(1).
pub fn noise3(p: Vec3, seed: u64) -> f32 {
    let f = p.floor();
    let (x, y, z) = (f.x as i32, f.y as i32, f.z as i32);
    let d = p - f;
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(d.x), fade(d.y), fade(d.z));
    let corner = |i: i32, j: i32, k: i32| grad(hash(seed, x + i, y + j, z + k), d - Vec3::new(i as f32, j as f32, k as f32));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractal noise: `octaves` layers of [`noise3`], each twice as fine and half
/// as strong, normalized to about -1 to 1.
///
/// Complexity: O(octaves).
pub fn fbm(p: Vec3, seed: u64, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut freq) = (0.0, 1.0, 0.0, 1.0);
    for o in 0..octaves.max(1) {
        sum += noise3(p * freq, seed.wrapping_add(u64::from(o) * 0x9E37)) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Noise displacement parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplaceSpec {
    /// Largest offset along the normals, in meters.
    pub strength: f32,
    /// Size of the noise features, in meters.
    pub scale: f32,
    pub seed: u64,
    /// Noise layers: 1 is smooth, more adds finer detail.
    pub octaves: u32,
}

/// Largest number of noise octaves.
pub const MAX_OCTAVES: u32 = 8;

/// Moves vertices along their normals (area-weighted) by seeded fractal
/// noise. Same inputs and seed give the same result.
///
/// Complexity: O(F + C) for the normals plus O(k x octaves) for k vertices.
pub fn displace(mesh: &mut Mesh, verts: &[u32], spec: &DisplaceSpec) -> Result<usize, KernelError> {
    finite("strength", spec.strength)?;
    if !(spec.scale.is_finite() && spec.scale > 0.0) {
        return Err(KernelError::OutOfRange {
            param: "scale",
            message: "noise scale must be more than 0".into(),
            hint: "about the size of the bumps, e.g. 0.1".into(),
        });
    }
    if !(1..=MAX_OCTAVES).contains(&spec.octaves) {
        return Err(KernelError::OutOfRange {
            param: "octaves",
            message: format!("{} octaves is outside 1 to {MAX_OCTAVES}", spec.octaves),
            hint: "1 to 4".into(),
        });
    }
    let (vs, _, _) = extent(mesh, verts, DeformAxis::Y)?;
    let face_n: Vec<Vec3> =
        (0..mesh.face_slots() as u32).map(|f| if mesh.is_face_alive(f) { mesh.face_normal_area2(f) } else { Vec3::ZERO }).collect();
    let offsets: Vec<Vec3> = vs
        .iter()
        .map(|&v| {
            let n = mesh.vert_faces(v).map(|f| face_n[f as usize]).sum::<Vec3>().normalize_or_zero();
            n * (spec.strength * fbm(mesh.positions[v as usize] / spec.scale, spec.seed, spec.octaves))
        })
        .collect();
    for (&v, off) in vs.iter().zip(offsets) {
        mesh.positions[v as usize] += off;
    }
    mesh.revision += 1;
    Ok(vs.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{CapKind, CylinderSpec, IcoSphereSpec, Origin, PlaneSpec, cylinder, ico_sphere, plane};

    fn post() -> Mesh {
        cylinder(&CylinderSpec { radius: 0.1, height: 2.0, segments: 8, height_segments: 10, cap: CapKind::Ngon }, Origin::Bottom).unwrap()
    }

    #[test]
    fn twist_turns_the_top() {
        let mut m = post();
        let all: Vec<u32> = m.verts().collect();
        let front_top = m.verts().find(|&v| m.positions[v as usize].abs_diff_eq(Vec3::new(0.0, 2.0, 0.1), 1e-6)).unwrap();
        twist(&mut m, &all, DeformAxis::Y, std::f32::consts::FRAC_PI_2).unwrap();
        m.validate().unwrap();
        assert!(m.positions[front_top as usize].abs_diff_eq(Vec3::new(0.1, 2.0, 0.0), 1e-5), "{}", m.positions[front_top as usize]);
    }

    #[test]
    fn taper_shrinks_the_far_end() {
        let mut m = post();
        let all: Vec<u32> = m.verts().collect();
        taper(&mut m, &all, DeformAxis::Y, 0.0).unwrap();
        let top_r = m
            .verts()
            .filter(|&v| m.positions[v as usize].y > 1.99)
            .map(|v| Vec3::new(m.positions[v as usize].x, 0.0, m.positions[v as usize].z).length())
            .fold(0.0f32, f32::max);
        assert!(top_r < 1e-6, "a point at the top");
        assert!(taper(&mut m, &all, DeformAxis::Y, -1.0).is_err());
    }

    #[test]
    fn bend_curls_a_post_into_a_quarter_circle() {
        let mut m = post();
        let all: Vec<u32> = m.verts().collect();
        bend(&mut m, &all, DeformAxis::Y, DeformAxis::X, std::f32::consts::FRAC_PI_2).unwrap();
        m.validate().unwrap();
        // The axis line (x = z = 0) ends at (R, R) with R = 2 / (pi / 2); the
        // top cap then faces +X at x = R and the outer side reaches y = R + 0.1.
        let r = 2.0 / std::f32::consts::FRAC_PI_2;
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.x - r).abs() < 1e-4 && (hi.y - (r + 0.1)).abs() < 1e-4, "{lo} {hi}");
        assert!((lo.x + 0.1).abs() < 1e-4 && lo.y.abs() < 1e-4);
        assert!(bend(&mut m, &all, DeformAxis::Y, DeformAxis::Y, 1.0).is_err());
    }

    #[test]
    fn displacement_is_seeded() {
        let make = |seed| {
            let mut m = ico_sphere(&IcoSphereSpec { radius: 1.0, subdivisions: 3 }, Origin::Center).unwrap();
            let all: Vec<u32> = m.verts().collect();
            displace(&mut m, &all, &DisplaceSpec { strength: 0.1, scale: 0.5, seed, octaves: 3 }).unwrap();
            m
        };
        assert_eq!(make(7).positions, make(7).positions);
        assert_ne!(make(7).positions, make(8).positions);
        let m = make(7);
        m.validate().unwrap();
        assert!(m.verts().all(|v| (m.positions[v as usize].length() - 1.0).abs() <= 0.1 + 1e-4));
        assert!(m.verts().any(|v| (m.positions[v as usize].length() - 1.0).abs() > 0.01), "something moved");
        let mut flat = plane(&PlaneSpec::default()).unwrap();
        assert!(displace(&mut flat, &[0], &DisplaceSpec { strength: 1.0, scale: 0.0, seed: 1, octaves: 1 }).is_err());
    }

    #[test]
    fn noise_is_bounded_and_smooth() {
        let mut max = 0.0f32;
        for i in 0..1000 {
            let p = Vec3::new(i as f32 * 0.137, (i as f32 * 0.071).sin() * 3.0, i as f32 * 0.013);
            let n = noise3(p, 42);
            max = max.max(n.abs());
            let near = noise3(p + Vec3::splat(1e-3), 42);
            assert!((n - near).abs() < 0.01);
        }
        assert!(max <= 1.0 && max > 0.3, "{max}");
    }
}
