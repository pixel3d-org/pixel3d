//! Deterministic surface samples spread by area.

use glam::Vec3;

use crate::view::Part;

/// A point on a part's surface (world space) standing for `weight` square
/// meters of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub point: Vec3,
    pub normal: Vec3,
    pub weight: f32,
    pub tri: u32,
}

/// R2 low-discrepancy steps (Roberts 2018): evenly spread, deterministic.
const R2: (f64, f64) = (0.754_877_666_246_692_7, 0.569_840_290_998_053_2);

/// About `n` samples covering the surface evenly by area. With at least `n`
/// triangles, every k-th triangle's centroid (weighted by its area);
/// otherwise each triangle gets points in proportion to its area, placed on
/// a low-discrepancy pattern, so a 12-triangle box is sampled all over its
/// faces and not just at 12 centroids.
///
/// Complexity: O(triangles + n).
pub fn surface_samples(part: &Part, n: usize) -> Vec<Sample> {
    let bvh = part.bvh();
    let tris = bvh.len();
    let mut out = Vec::with_capacity(n + tris.min(n));
    if tris == 0 || n == 0 {
        return out;
    }
    let area = |t: u32| {
        let [a, b, c] = bvh.triangle(t);
        0.5 * (b - a).cross(c - a).length()
    };
    if tris >= n {
        let stride = tris.div_ceil(n);
        for t in (0..tris as u32).step_by(stride) {
            let [a, b, c] = bvh.triangle(t);
            out.push(Sample { point: (a + b + c) / 3.0, normal: bvh.normal(t), weight: area(t) * stride as f32, tri: t });
        }
        return out;
    }
    let total: f32 = (0..tris as u32).map(area).sum::<f32>().max(1e-30);
    let mut seq = 0u64;
    for t in 0..tris as u32 {
        let at = area(t);
        let k = ((n as f32 * at / total).round() as usize).max(1);
        let [a, b, c] = bvh.triangle(t);
        let normal = bvh.normal(t);
        for _ in 0..k {
            seq += 1;
            let (mut u, mut v) = ((0.5 + R2.0 * seq as f64).fract(), (0.5 + R2.1 * seq as f64).fract());
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let point = a + (b - a) * u as f32 + (c - a) * v as f32;
            out.push(Sample { point, normal, weight: at / k as f32, tri: t });
        }
    }
    out
}

/// Weighted median of `(value, weight)` pairs (sorts them).
///
/// Complexity: O(m log m).
pub fn weighted_median(values: &mut [(f32, f32)]) -> Option<f32> {
    values.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f32 = values.iter().map(|v| v.1).sum();
    let mut acc = 0.0;
    for (x, w) in values.iter() {
        acc += w;
        if acc >= total / 2.0 {
            return Some(*x);
        }
    }
    values.last().map(|v| v.0)
}
