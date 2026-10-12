//! `shape`: what does each part look like? Best-fit primitive, principal
//! axes and extents, volume, area, how much of its box it fills,
//! thickness, edge sharpness, holes and mirror symmetry.

use glam::{DMat3, DVec3, Vec3};
use serde::Serialize;

use crate::Report;
use crate::fmt::{area, deg, mm, thousands, unit, volume};
use crate::sample::{surface_samples, weighted_median};
use crate::view::{Part, View};

/// Parts described in the text before it is cut short.
const MAX_PARTS: usize = 8;
/// Surface samples for primitive fits.
const FIT_SAMPLES: usize = 4096;
/// Inward rays for thickness.
const THICKNESS_SAMPLES: usize = 64;
/// Surface samples mirrored per symmetry plane.
const SYMMETRY_SAMPLES: usize = 256;

/// A primitive the part resembles.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fit {
    /// `box`, `cylinder`, `sphere` or `plane`.
    pub kind: &'static str,
    /// Cylinder axis (`x`, `y`, `z` or `a1` to `a3` for principal axes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// 100 when every sample lies on the primitive.
    pub percent: f32,
}

/// Mirror symmetry across a plane through the part's box center.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mirror {
    /// Plane normal: `x` (the YZ plane), `y`, `z`, or principal axis `a1` to `a3`.
    pub axis: String,
    pub symmetric: bool,
    /// Largest distance of a mirrored surface sample from the surface.
    pub max_dev: f32,
    /// The search stopped at 10% of the part's size: `max_dev` is a lower bound.
    pub capped: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShapeInfo {
    pub id: u32,
    pub name: String,
    /// Best fits, best first.
    pub fits: Vec<Fit>,
    /// Principal axes (unit, world space), longest first, and full extents.
    pub axes: [Vec3; 3],
    pub extents: [f32; 3],
    /// The axes are the world axes (within 2 degrees).
    pub world_aligned: bool,
    pub volume: f64,
    pub area: f64,
    /// Volume over the volume of the principal-axes box (closed parts).
    pub fill: Option<f32>,
    pub closed: bool,
    pub holes: usize,
    pub parts: usize,
    /// Inward thickness (closed parts): min and area-weighted median.
    pub thickness: Option<(f32, f32)>,
    /// Edges by dihedral angle: hard (30 degrees or more), soft (1 to 30), flat.
    pub hard_edges: usize,
    pub soft_edges: usize,
    pub flat_edges: usize,
    pub max_edge_angle: f32,
    pub mirror: Vec<Mirror>,
}

/// Eigen-decomposition of a symmetric 3x3 matrix by cyclic Jacobi
/// rotations: (eigenvalues, eigenvectors as columns), unsorted.
///
/// Complexity: O(1) (at most 32 sweeps).
pub fn sym_eigen(m: DMat3) -> ([f64; 3], DMat3) {
    let mut a = m.to_cols_array_2d();
    let mut v = DMat3::IDENTITY.to_cols_array_2d();
    for _ in 0..32 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        if off < 1e-30 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q].abs() < 1e-300 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let t = if theta == 0.0 { 1.0 } else { t };
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in &mut a {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (rp, rq) = (a[p], a[q]);
            a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
            a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    ([a[0][0], a[1][1], a[2][2]], DMat3::from_cols_array_2d(&v).transpose())
}

/// Principal axes of a part's surface (exact second moments of its
/// triangles), longest first, snapped to the world axes when within 2
/// degrees or when the spread is nearly equal in every direction.
///
/// Complexity: O(triangles).
fn principal_axes(part: &Part) -> ([Vec3; 3], bool) {
    let bvh = part.bvh();
    let (mut total, mut mean, mut second) = (0.0f64, DVec3::ZERO, DMat3::ZERO);
    for t in 0..bvh.len() as u32 {
        let [a, b, c] = bvh.triangle(t).map(Vec3::as_dvec3);
        let area = 0.5 * (b - a).cross(c - a).length();
        let s = a + b + c;
        total += area;
        mean += area * s / 3.0;
        let outer = |v: DVec3| DMat3::from_cols(v * v.x, v * v.y, v * v.z);
        second += (outer(a) + outer(b) + outer(c) + outer(s)) * (area / 12.0);
    }
    if total <= 0.0 {
        return ([Vec3::X, Vec3::Y, Vec3::Z], true);
    }
    let mean = mean / total;
    let cov = second * (1.0 / total) - DMat3::from_cols(mean * mean.x, mean * mean.y, mean * mean.z);
    let (vals, vecs) = sym_eigen(cov);
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| vals[j].total_cmp(&vals[i]));
    let axes: [Vec3; 3] = order.map(|i| {
        let v = vecs.col(i).as_vec3().normalize_or_zero();
        // Sign convention: largest component positive.
        let k = v.abs().max_position();
        if v[k] < 0.0 { -v } else { v }
    });
    let spread = vals[order[0]].max(1e-30);
    let near_equal = (vals[order[0]] - vals[order[2]]) / spread < 0.02;
    let aligned = axes.iter().all(|a| a.abs().max_element() > 2.0f32.to_radians().cos());
    if near_equal || aligned {
        // World axes, ordered by the part's box extents (longest first).
        let size = part.size();
        let mut w = [(size.x, Vec3::X), (size.y, Vec3::Y), (size.z, Vec3::Z)];
        w.sort_by(|a, b| b.0.total_cmp(&a.0));
        return (w.map(|(_, a)| a), true);
    }
    (axes, false)
}

fn axis_name(a: Vec3, i: usize) -> String {
    if a == Vec3::X {
        "x".into()
    } else if a == Vec3::Y {
        "y".into()
    } else if a == Vec3::Z {
        "z".into()
    } else {
        format!("a{}", i + 1)
    }
}

/// Distance from local point `q` (in the box frame) to a box surface of
/// half extents `h`.
fn box_dist(q: Vec3, h: Vec3) -> f32 {
    let d = q.abs() - h;
    if d.max_element() <= 0.0 { -d.max_element() } else { d.max(Vec3::ZERO).length() }
}

/// Fits of box, cylinder (about each axis), sphere and plane, best first.
///
/// Complexity: O(samples).
fn fits(part: &Part, axes: &[Vec3; 3], center: Vec3, half: Vec3) -> Vec<Fit> {
    let samples: Vec<(Vec3, f32)> = surface_samples(part, FIT_SAMPLES)
        .iter()
        .map(|s| {
            let p = s.point - center;
            (Vec3::new(p.dot(axes[0]), p.dot(axes[1]), p.dot(axes[2])), s.weight)
        })
        .collect();
    let weight: f32 = samples.iter().map(|s| s.1).sum::<f32>().max(1e-30);
    let scale = (half.x + half.y + half.z) / 3.0;
    let score = |dist: &dyn Fn(Vec3) -> f32| -> f32 {
        let mean = samples.iter().map(|(q, w)| dist(*q) * w).sum::<f32>() / weight;
        (100.0 * (1.0 - mean / (0.25 * scale.max(1e-9)))).clamp(0.0, 100.0)
    };
    let mut out = Vec::new();
    if half.z < 0.02 * half.x {
        out.push(Fit { kind: "plane", axis: None, percent: score(&|q| q.z.abs()) });
    }
    out.push(Fit { kind: "box", axis: None, percent: score(&|q| box_dist(q, half)) });
    for k in 0..3 {
        let (i, j) = ((k + 1) % 3, (k + 2) % 3);
        let r = (half[i] + half[j]) / 2.0;
        let h = half[k];
        let d = move |q: Vec3| {
            let rho = (q[i] * q[i] + q[j] * q[j]).sqrt();
            let (dr, dh) = (rho - r, q[k].abs() - h);
            if dr <= 0.0 && dh <= 0.0 { (-dr).min(-dh) } else { (dr.max(0.0).powi(2) + dh.max(0.0).powi(2)).sqrt() }
        };
        out.push(Fit { kind: "cylinder", axis: Some(axis_name(axes[k], k)), percent: score(&d) });
    }
    let r = samples.iter().map(|(q, w)| q.length() * w).sum::<f32>() / weight;
    out.push(Fit { kind: "sphere", axis: None, percent: score(&|q| (q.length() - r).abs()) });
    // Best first; ties keep the simpler kind (the order above).
    out.sort_by(|a, b| b.percent.total_cmp(&a.percent));
    out
}

/// Inward thickness of a closed part at surface samples: (min, typical),
/// where typical is the area-weighted median (most of the surface is that
/// far from the opposite side).
///
/// Complexity: O(samples x (log n + k)).
fn thickness(part: &Part) -> Option<(f32, f32)> {
    let bvh = part.bvh();
    let eps = part.size().length() * 1e-5;
    let mut found: Vec<(f32, f32)> = Vec::new();
    for s in surface_samples(part, THICKNESS_SAMPLES) {
        if s.normal == Vec3::ZERO {
            continue;
        }
        if let Some(hit) = bvh.raycast(s.point - s.normal * eps, -s.normal, f32::INFINITY) {
            found.push((hit.t + eps, s.weight));
        }
    }
    let min = found.iter().map(|f| f.0).fold(f32::INFINITY, f32::min);
    weighted_median(&mut found).map(|typical| (min, typical))
}

/// Mirror symmetry across the planes through the part's box center whose
/// normals are its axes (world axes when aligned, else principal axes).
///
/// Complexity: O(3 x samples x (log n + k)).
fn mirrors(part: &Part, axes: &[Vec3; 3], center: Vec3, tol: f32) -> Vec<Mirror> {
    let cap = part.size().length() * 0.1 + tol;
    let pts: Vec<Vec3> = surface_samples(part, SYMMETRY_SAMPLES).iter().map(|s| s.point).collect();
    let mut out: Vec<Mirror> = (0..3)
        .map(|k| {
            let n = axes[k];
            let mut max_dev = 0.0f32;
            for p in &pts {
                let m = *p - n * (2.0 * (*p - center).dot(n));
                let d = part.bvh().closest(m, cap).map_or(cap, |c| c.dist);
                max_dev = max_dev.max(d);
                if max_dev >= cap {
                    break;
                }
            }
            Mirror { axis: axis_name(n, k), symmetric: max_dev <= tol, max_dev, capped: max_dev >= cap }
        })
        .collect();
    out.sort_by(|a, b| a.axis.cmp(&b.axis));
    out
}

/// Edge counts by dihedral angle and the largest angle in degrees.
///
/// Complexity: O(E).
fn edge_sharpness(part: &Part) -> (usize, usize, usize, f32) {
    let m = part.mesh;
    let (mut hard, mut soft, mut flat, mut max) = (0, 0, 0, 0.0f32);
    for e in m.edges() {
        let [Some(f), Some(g)] = m.edge_faces(e) else { continue };
        let a = m.face_normal(f).angle_between(m.face_normal(g)).to_degrees();
        if !a.is_finite() {
            continue;
        }
        max = max.max(a);
        if a >= 30.0 {
            hard += 1;
        } else if a >= 1.0 {
            soft += 1;
        } else {
            flat += 1;
        }
    }
    (hard, soft, flat, max)
}

/// The shape of one part.
///
/// Complexity: O(n) for its n triangles plus bounded sampled queries.
pub fn shape_of(view: &View, i: usize) -> ShapeInfo {
    let part = &view.parts[i];
    let (axes, world_aligned) = principal_axes(part);
    // Extents along the axes, from the vertices.
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for v in part.mesh.verts() {
        let p = part.world.transform_point3(part.mesh.positions[v as usize]);
        let q = Vec3::new(p.dot(axes[0]), p.dot(axes[1]), p.dot(axes[2]));
        lo = lo.min(q);
        hi = hi.max(q);
    }
    let local_center = (lo + hi) / 2.0;
    let center = axes[0] * local_center.x + axes[1] * local_center.y + axes[2] * local_center.z;
    let half = (hi - lo) / 2.0;
    let topo = part.topology();
    let scale = part.world.matrix3.determinant().abs() as f64;
    let vol = topo.volume * scale;
    let closed = topo.boundary_edges == 0;
    let box_vol = f64::from(8.0 * half.x * half.y * half.z);
    let (hard_edges, soft_edges, flat_edges, max_edge_angle) = edge_sharpness(part);
    ShapeInfo {
        id: part.id,
        name: part.label.clone(),
        fits: fits(part, &axes, center, half),
        axes,
        extents: (half * 2.0).to_array(),
        world_aligned,
        volume: vol,
        area: surface_area(part),
        fill: (closed && box_vol > 0.0).then(|| (vol / box_vol) as f32),
        closed,
        holes: topo.boundary_loops,
        parts: topo.parts,
        thickness: if closed { thickness(part) } else { None },
        hard_edges,
        soft_edges,
        flat_edges,
        max_edge_angle,
        mirror: mirrors(part, &axes, center, (part.size().length() * 0.005).max(view.contact_tolerance())),
    }
}

/// World-space surface area from the part's triangles.
///
/// Complexity: O(triangles).
fn surface_area(part: &Part) -> f64 {
    let bvh = part.bvh();
    (0..bvh.len() as u32)
        .map(|t| {
            let [a, b, c] = bvh.triangle(t);
            0.5 * f64::from((b - a).cross(c - a).length())
        })
        .sum()
}

/// One block of text per part.
pub fn shape_text(view: &View, shapes: &[ShapeInfo]) -> String {
    let mut t = format!("shape of {}: {} part{}, mm\n", view.title(), view.parts.len(), if view.parts.len() == 1 { "" } else { "s" });
    for s in shapes.iter().take(MAX_PARTS) {
        let fit = |f: &Fit| match &f.axis {
            Some(a) => format!("{} {} {:.0}%", f.kind, a, f.percent),
            None => format!("{} {:.0}%", f.kind, f.percent),
        };
        let others: Vec<String> = s.fits.iter().skip(1).take(2).map(fit).collect();
        t.push_str(&format!("{} (o:{}): {} (then {})\n", s.name, s.id, s.fits.first().map(fit).unwrap_or_default(), others.join(", ")));
        let axes = if s.world_aligned {
            let names: Vec<String> = (0..3).map(|k| format!("{} {}", axis_name(s.axes[k], k).to_uppercase(), mm(s.extents[k]))).collect();
            format!("extents {} (world axes)", names.join(", "))
        } else {
            let names: Vec<String> = (0..3).map(|k| format!("{} {}", unit(s.axes[k]), mm(s.extents[k]))).collect();
            format!("principal axes {}", names.join("; "))
        };
        t.push_str(&format!("  {axes}\n"));
        let mut line = format!("  area {}", area(s.area));
        if s.closed {
            line.push_str(&format!(", volume {}", volume(s.volume)));
            if let Some(f) = s.fill {
                line.push_str(&format!(", fills {:.0}% of its box", 100.0 * f));
            }
            line.push_str(", closed");
        } else {
            line.push_str(&format!(", open: {} hole{}", s.holes, if s.holes == 1 { "" } else { "s" }));
        }
        if s.parts > 1 {
            line.push_str(&format!(", {} loose pieces", s.parts));
        }
        t.push_str(&line);
        t.push('\n');
        let mut line = String::from("  ");
        if let Some((min, mean)) = s.thickness {
            line.push_str(&format!("thickness min {}, typical {}; ", mm(min), mm(mean)));
        }
        line.push_str(&format!(
            "edges {} hard, {} soft, {} flat (max {} deg); mirror",
            thousands(s.hard_edges),
            thousands(s.soft_edges),
            thousands(s.flat_edges),
            deg(s.max_edge_angle)
        ));
        for m in &s.mirror {
            if m.symmetric {
                line.push_str(&format!(" {} yes", m.axis));
            } else {
                line.push_str(&format!(" {} no ({}{} mm)", m.axis, if m.capped { "over " } else { "" }, mm(m.max_dev)));
            }
        }
        t.push_str(&line);
        t.push('\n');
    }
    if shapes.len() > MAX_PARTS {
        t.push_str(&format!("+{} more parts (perceive shape with a target narrows the view)\n", shapes.len() - MAX_PARTS));
    }
    t
}

/// The `shape` report of a view: every part (the text describes the first
/// [`MAX_PARTS`]).
pub fn report(view: &View) -> Report {
    let shapes: Vec<ShapeInfo> = (0..view.parts.len()).map(|i| shape_of(view, i)).collect();
    Report::new("shape", shape_text(view, &shapes), &shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jacobi_finds_eigenvectors() {
        let r = glam::DQuat::from_euler(glam::EulerRot::XYZ, 0.3, 0.7, -0.2);
        let rot = DMat3::from_quat(r);
        let m = rot * DMat3::from_diagonal(DVec3::new(5.0, 2.0, 0.5)) * rot.transpose();
        let (vals, vecs) = sym_eigen(m);
        let mut sorted = vals;
        sorted.sort_by(|a, b| b.total_cmp(a));
        assert!((sorted[0] - 5.0).abs() < 1e-9 && (sorted[1] - 2.0).abs() < 1e-9 && (sorted[2] - 0.5).abs() < 1e-9, "{vals:?}");
        for (k, val) in vals.iter().enumerate() {
            let v = vecs.col(k);
            assert!((m * v - v * *val).length() < 1e-9);
        }
    }

    #[test]
    fn box_distance() {
        let h = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(box_dist(Vec3::ZERO, h), 1.0);
        assert_eq!(box_dist(Vec3::new(2.0, 0.0, 0.0), h), 1.0);
        assert!((box_dist(Vec3::new(2.0, 3.0, 0.0), h) - 2f32.sqrt()).abs() < 1e-6);
    }
}
