//! Tubes built ring by ring along a path: designed edge flow (rings and
//! lengthwise lines) for horns, tails, tentacles and cables. The base is
//! closed by one cap face, the tip by a pole.

use glam::Vec3;
use p3d_mesh::{ElementKind, Mesh};

use crate::FeatureError;

/// A tube: one ring per path point with its radius.
pub struct TubeSpec<'a> {
    pub path: &'a [Vec3],
    pub radius: &'a [f32],
    pub sides: usize,
    /// Close the last ring into a point (a tip) instead of a flat cap.
    pub pointed: bool,
}

/// Rotation-minimizing frames along a path (double reflection).
///
/// Complexity: O(points).
pub fn frames(path: &[Vec3]) -> Vec<(Vec3, Vec3, Vec3)> {
    let n = path.len();
    let tangent = |i: usize| {
        let a = path[i.saturating_sub(1)];
        let b = path[(i + 1).min(n - 1)];
        (b - a).normalize_or(Vec3::Y)
    };
    let t0 = tangent(0);
    let mut r = t0.any_orthonormal_vector();
    let mut out = Vec::with_capacity(n);
    out.push((t0, r, t0.cross(r)));
    for i in 1..n {
        let (ti, ri) = (tangent(i - 1), r);
        let v1 = path[i] - path[i - 1];
        let c1 = v1.dot(v1);
        if c1 < 1e-20 {
            out.push((tangent(i), r, tangent(i).cross(r)));
            continue;
        }
        let rl = ri - v1 * (2.0 / c1 * v1.dot(ri));
        let tl = ti - v1 * (2.0 / c1 * v1.dot(ti));
        let t = tangent(i);
        let v2 = t - tl;
        let c2 = v2.dot(v2);
        r = if c2 < 1e-20 { rl } else { rl - v2 * (2.0 / c2 * v2.dot(rl)) };
        r = (r - t * t.dot(r)).normalize_or(t.any_orthonormal_vector());
        out.push((t, r, t.cross(r)));
    }
    out
}

/// Builds the tube with groups `side`, `base` (and `tip` when pointed).
///
/// Complexity: O(points x sides).
pub fn tube(spec: &TubeSpec) -> Result<Mesh, FeatureError> {
    let n = spec.path.len();
    let s = spec.sides.max(3);
    if n < 2 || spec.radius.len() != n {
        return Err(FeatureError { path: "/params".into(), message: "a tube needs at least two rings".into(), suggestions: Vec::new() });
    }
    let fr = frames(spec.path);
    let rings = if spec.pointed { n - 1 } else { n };
    let mut pos = Vec::with_capacity(rings * s + 1);
    for (i, &(_, r, b)) in fr.iter().enumerate().take(rings) {
        for k in 0..s {
            let a = k as f32 / s as f32 * std::f32::consts::TAU;
            pos.push(spec.path[i] + (r * a.cos() + b * a.sin()) * spec.radius[i]);
        }
    }
    let tip = pos.len() as u32;
    if spec.pointed {
        pos.push(spec.path[n - 1]);
    }
    let mut sizes = Vec::new();
    let mut corners = Vec::new();
    let mut side = Vec::new();
    let mut tipf = Vec::new();
    for i in 0..rings - 1 {
        for k in 0..s {
            let (a, b) = ((i * s + k) as u32, (i * s + (k + 1) % s) as u32);
            let (c, d) = (b + s as u32, a + s as u32);
            side.push(sizes.len() as u32);
            sizes.push(4);
            // Rings run counter-clockwise seen from the tip: a, b, c, d faces out.
            corners.extend([a, b, c, d]);
        }
    }
    let last = ((rings - 1) * s) as u32;
    if spec.pointed {
        for k in 0..s as u32 {
            tipf.push(sizes.len() as u32);
            sizes.push(3);
            corners.extend([last + k, last + (k + 1) % s as u32, tip]);
        }
    } else {
        tipf.push(sizes.len() as u32);
        sizes.push(s as u32);
        corners.extend((0..s as u32).map(|k| last + k));
    }
    let base = sizes.len() as u32;
    sizes.push(s as u32);
    corners.extend((0..s as u32).rev());
    let mut m = Mesh::from_polygons(pos, &sizes, &corners).map_err(|e| FeatureError {
        path: "/params".into(),
        message: e.to_string(),
        suggestions: Vec::new(),
    })?;
    // Fresh face lists on a fresh mesh cannot fail.
    let _ = m.set_group("side", ElementKind::Face, side);
    let _ = m.set_group(if spec.pointed { "tip" } else { "end" }, ElementKind::Face, tipf);
    let _ = m.set_group("base", ElementKind::Face, vec![base]);
    Ok(m)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn tubes_are_closed_and_face_out() {
        let path: Vec<Vec3> = (0..10).map(|i| Vec3::new(0.0, i as f32 * 0.1, (i as f32 * 0.3).sin() * 0.1)).collect();
        let radius: Vec<f32> = (0..10).map(|i| 0.05 * (1.0 - i as f32 / 10.0)).collect();
        for pointed in [false, true] {
            let m = tube(&TubeSpec { path: &path, radius: &radius, sides: 8, pointed }).unwrap();
            assert_eq!(m.validate(), Ok(()));
            assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
            let mut vol = 0.0f32;
            for f in m.faces() {
                for t in m.face_triangles(f) {
                    let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize]);
                    vol += a.dot(b.cross(c)) / 6.0;
                }
            }
            assert!(vol > 0.0, "outward faces: {vol}");
        }
    }
}
