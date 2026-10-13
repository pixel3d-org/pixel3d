//! Scatter: seeded Poisson-disk points on a surface, for placing instances.

use glam::{IVec3, Vec3};
use p3d_mesh::Mesh;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::edit::KernelError;

/// Most points one scatter places.
pub const MAX_SCATTER: u32 = 10_000;
/// Dart throws per requested point before giving up on a crowded surface.
const ATTEMPTS_PER_POINT: u32 = 30;

/// A small deterministic generator (SplitMix64).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Scatter parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatterSpec {
    /// Points wanted; fewer come back when the spacing does not allow them.
    pub count: u32,
    /// No two points closer than this (0 for plain random points).
    pub min_distance: f32,
    pub seed: u64,
}

/// A scattered point on a face, with two random values in [0, 1) for the
/// caller (for example a turn and a scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScatterPoint {
    pub position: Vec3,
    pub normal: Vec3,
    pub face: u32,
    pub random: [f32; 2],
}

/// Area of the given faces.
///
/// Complexity: O(corners of the faces).
pub fn surface_area(mesh: &Mesh, faces: &[u32]) -> f32 {
    faces.iter().filter(|&&f| mesh.is_face_alive(f)).map(|&f| mesh.face_area(f)).sum()
}

/// Seeded points spread evenly over faces (by area), never closer than
/// `min_distance` (dart throwing on a hash grid, up to 30 throws per wanted
/// point). The same mesh, faces and spec always give the same points.
///
/// Complexity: O(c + a log t) for c corners, t triangles and a throws.
pub fn scatter(mesh: &Mesh, faces: &[u32], spec: &ScatterSpec) -> Result<Vec<ScatterPoint>, KernelError> {
    if spec.count == 0 || spec.count > MAX_SCATTER {
        return Err(KernelError::OutOfRange {
            param: "count",
            message: format!("{} is outside 1 to {MAX_SCATTER}", spec.count),
            hint: String::new(),
        });
    }
    if !(spec.min_distance.is_finite() && spec.min_distance >= 0.0) {
        return Err(KernelError::OutOfRange { param: "min_distance", message: "must be 0 or more".into(), hint: String::new() });
    }
    // Triangles with their faces and a running area total.
    let mut tris: Vec<([Vec3; 3], u32)> = Vec::new();
    let mut cdf: Vec<f32> = Vec::new();
    let mut total = 0.0f32;
    for &f in faces {
        if !mesh.is_face_alive(f) {
            continue;
        }
        for t in mesh.face_triangles(f) {
            let p = t.map(|h| mesh.positions[mesh.origin(h) as usize]);
            let a = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
            if a > 0.0 {
                total += a;
                tris.push((p, f));
                cdf.push(total);
            }
        }
    }
    if tris.is_empty() {
        return Err(KernelError::Empty("the faces have no area to scatter on".into()));
    }
    let d = spec.min_distance;
    let cell = |p: Vec3| (p / d).floor().as_ivec3();
    let mut grid: FxHashMap<IVec3, SmallVec<[u32; 4]>> = FxHashMap::default();
    let mut rng = Rng::new(spec.seed);
    let mut out: Vec<ScatterPoint> = Vec::with_capacity(spec.count as usize);
    let attempts = spec.count.saturating_mul(ATTEMPTS_PER_POINT);
    for _ in 0..attempts {
        if out.len() == spec.count as usize {
            break;
        }
        let r = rng.unit() * total;
        let i = cdf.partition_point(|&c| c <= r).min(tris.len() - 1);
        let ([a, b, c], f) = tris[i];
        let (s, t) = (rng.unit().sqrt(), rng.unit());
        let p = a * (1.0 - s) + b * (s * (1.0 - t)) + c * (s * t);
        let random = [rng.unit(), rng.unit()];
        if d > 0.0 {
            let c0 = cell(p);
            let crowded = (-1..=1).any(|x| {
                (-1..=1).any(|y| {
                    (-1..=1).any(|z| {
                        grid.get(&(c0 + IVec3::new(x, y, z))).is_some_and(|ids| ids.iter().any(|&j| out[j as usize].position.distance(p) < d))
                    })
                })
            });
            if crowded {
                continue;
            }
            grid.entry(c0).or_default().push(out.len() as u32);
        }
        out.push(ScatterPoint { position: p, normal: mesh.face_normal(f), face: f, random });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{PlaneSpec, plane};

    #[test]
    fn spaced_seeded_and_on_the_surface() {
        let m = plane(&PlaneSpec { size_x: 4.0, size_z: 4.0, segments_x: 8, segments_z: 8 }).unwrap();
        let faces: Vec<u32> = m.faces().collect();
        let spec = ScatterSpec { count: 100, min_distance: 0.25, seed: 7 };
        let a = scatter(&m, &faces, &spec).unwrap();
        assert_eq!(a, scatter(&m, &faces, &spec).unwrap(), "same seed, same points");
        assert_ne!(a, scatter(&m, &faces, &ScatterSpec { seed: 8, ..spec }).unwrap());
        assert_eq!(a.len(), 100);
        for (i, p) in a.iter().enumerate() {
            assert!(p.position.y.abs() < 1e-6 && p.position.x.abs() <= 2.0 && p.position.z.abs() <= 2.0);
            assert!(p.normal.abs_diff_eq(Vec3::Y, 1e-6));
            for q in &a[i + 1..] {
                assert!(p.position.distance(q.position) >= 0.25);
            }
        }
        // Too crowded: fewer points come back.
        let few = scatter(&m, &faces, &ScatterSpec { count: 500, min_distance: 0.5, seed: 1 }).unwrap();
        assert!(few.len() < 100, "{}", few.len());
        assert!(scatter(&m, &[], &spec).is_err());
        assert!(scatter(&m, &faces, &ScatterSpec { count: 0, ..spec }).is_err());
    }
}
