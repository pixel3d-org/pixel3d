//! Guide curves: seeded Poisson-disk roots on the region, a direction from
//! the style blended toward gravity with curl and noise, and a push out of
//! the parent after every step so strands never penetrate it.

use std::f32::consts::TAU;

use glam::Vec3;
use p3d_geo::bvh::TriBvh;
use p3d_geo::generate::{Rng, ScatterSpec, scatter};
use p3d_mesh::Mesh;

use crate::{Output, StrandSpec, StrandsError, Style};

/// One grown guide: a clump's curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Guide {
    /// Points from the root (lifted off the surface) to the tip.
    pub points: Vec<Vec3>,
    /// The parent's outward normal nearest each point.
    pub outward: Vec<Vec3>,
    /// Width scale (outer layers and flyaways are thinner).
    pub width: f32,
    /// 0 for the innermost layer.
    pub layer: u32,
    /// Atlas column (0 to 3).
    pub slot: u32,
    pub flyaway: bool,
}

/// A generator of its own per guide, so guides do not depend on order.
fn rng_for(seed: u64, layer: u32, i: usize) -> Rng {
    Rng::new(seed ^ u64::from(layer).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
}

fn unit_vec(rng: &mut Rng) -> Vec3 {
    let z = rng.unit() * 2.0 - 1.0;
    let a = rng.unit() * TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * a.cos(), r * a.sin(), z)
}

/// Moves `p` out of the parent to at least `off` above its closest surface
/// point; returns the point and that surface's normal.
///
/// Complexity: one closest-point query, O(log F) near the surface.
fn push_out(bvh: &TriBvh, p: Vec3, off: f32, fallback: Vec3) -> (Vec3, Vec3) {
    match bvh.closest(p, f32::INFINITY) {
        Some(c) => {
            let n = bvh.normal(c.tri);
            let d = (p - c.point).dot(n);
            (if d < off { p + n * (off - d) } else { p }, n)
        }
        None => (p, fallback),
    }
}

/// Pulls `p` back down onto the surface (at `off`) where the surface does
/// not face down: draping hair rests on the scalp and sides of a head and
/// only hangs free past them.
///
/// Complexity: one closest-point query.
/// Also says whether the point rests on the surface.
fn hug(bvh: &TriBvh, p: Vec3, off: f32, n0: Vec3) -> (Vec3, Vec3, bool) {
    match bvh.closest(p, f32::INFINITY) {
        Some(c) => {
            let n = bvh.normal(c.tri);
            let d = (p - c.point).dot(n);
            if n.y > -0.3 { (p - n * (d - off).max(0.0), n, true) } else { (p, n, d <= off * 1.01 + 1e-5) }
        }
        None => (p, n0, false),
    }
}

/// Whether a style lies along the surface (and so hugs it).
fn drapes(style: Style) -> bool {
    matches!(style, Style::SweptBack | Style::Parted(_) | Style::Straight | Style::Combed { .. } | Style::Ponytail(_))
}

/// The style's direction at a root with normal `n`.
fn style_dir(style: Style, p: Vec3, n: Vec3, center: Vec3) -> Vec3 {
    let along = |v: Vec3| (v - n * v.dot(n)).try_normalize();
    let r = p - center;
    let fallback = along(Vec3::new(r.x, 0.0, r.z)).or_else(|| along(Vec3::NEG_Y)).unwrap_or_else(|| n.any_orthonormal_vector());
    let rise = |t: Option<Vec3>, k: f32| (t.unwrap_or(fallback) + n * k).normalize_or(n);
    match style {
        Style::SweptBack => rise(along(Vec3::new(0.0, -0.3, -1.0)), 0.25),
        Style::Parted(x0) => {
            let side = if p.x >= x0 { 1.0 } else { -1.0 };
            rise(along(Vec3::new(side, -0.5, 0.0)), 0.25)
        }
        Style::Spiky => n,
        Style::Straight => rise(along(Vec3::NEG_Y), 0.1),
        Style::Combed { flow, outward } => {
            let f = if outward && p.x < 0.0 { Vec3::new(-flow.x, flow.y, flow.z) } else { flow };
            rise(along(f), 0.1)
        }
        Style::Upright => (n + Vec3::Y).normalize_or(Vec3::Y),
        Style::WindBlown(w) => (n * 0.3 + w.normalize_or(Vec3::X)).normalize_or(n),
        Style::Ponytail(a) => rise(along(a - p), 0.1),
    }
}

/// How one guide grows.
struct Growth {
    lift: f32,
    noise: f32,
    length: f32,
}

/// Integrates one guide from a root.
///
/// Complexity: O(segments x log F).
fn integrate(bvh: &TriBvh, root: Vec3, n: Vec3, center: Vec3, spec: &StrandSpec, rng: &mut Rng, g: &Growth) -> (Vec<Vec3>, Vec<Vec3>) {
    let len = spec.length * g.length * (1.0 - spec.length_variation * rng.unit());
    let m = spec.segments as usize;
    let seg = len / m as f32;
    let base = style_dir(spec.style, root, n, center);
    let jitter = unit_vec(rng) * g.noise * 0.6;
    let phase = rng.unit() * TAU;
    let mut p = root + n * g.lift;
    let mut dir = (base + jitter).normalize_or(base);
    let mut out_n = n;
    let mut free = false;
    let mut points = Vec::with_capacity(m + 1);
    let mut outward = Vec::with_capacity(m + 1);
    points.push(p);
    outward.push(n);
    for i in 1..=m {
        let t = i as f32 / m as f32;
        let steer = match spec.style {
            Style::Ponytail(a) if a.distance(p) > seg * 2.0 => (a - p).normalize_or(base),
            Style::Ponytail(_) => Vec3::NEG_Y,
            _ => base,
        };
        let mut want = (steer + jitter + unit_vec(rng) * g.noise * 0.25).normalize_or(steer);
        if spec.curl > 0.0 {
            let u = dir.cross(out_n).normalize_or(dir.any_orthonormal_vector());
            let v = dir.cross(u);
            let th = phase + TAU * spec.curl_turns * t;
            want = (want + (u * th.cos() + v * th.sin()) * spec.curl * 1.5).normalize_or(want);
        }
        // Draping strands that leave the surface fall.
        let pull = if free { (spec.gravity * t).max(0.5 + 0.5 * spec.gravity) } else { spec.gravity * t };
        want = (want * (1.0 - pull) + Vec3::NEG_Y * pull).normalize_or(want);
        dir = (dir + want).normalize_or(want);
        let off = g.lift + spec.volume * t;
        let (q, nq) = push_out(bvh, p + dir * seg, off, out_n);
        // Keep the step length after the push.
        dir = (q - p).normalize_or(dir);
        let (q, nq) = push_out(bvh, p + dir * seg, off, nq);
        let (q, nq) = if drapes(spec.style) {
            let (q, nq, rests) = hug(bvh, q, off, nq);
            free = !rests;
            (q, nq)
        } else {
            (q, nq)
        };
        p = q;
        out_n = nq;
        points.push(p);
        outward.push(out_n);
    }
    (points, outward)
}

/// Grows every guide: one per root and layer, then flyaways (cards only).
///
/// Complexity: O(roots x layers x segments x log F) after the scatter.
pub(crate) fn grow(parent: &Mesh, faces: &[u32], bvh: &TriBvh, spec: &StrandSpec, spacing: f32) -> Result<Vec<Guide>, StrandsError> {
    let roots = scatter(parent, faces, &ScatterSpec { count: spec.clumps, min_distance: spacing, seed: spec.seed })
        .map_err(|e| StrandsError::new("clumps", e.to_string()))?;
    let (lo, hi) = parent.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let center = (lo + hi) * 0.5;
    let cards = spec.output == Output::Cards;
    let layers = if cards { spec.layers } else { 1 };
    let gap = spec.volume.max(spec.lift).max(spacing * 0.05) * 0.5;
    let mut out = Vec::with_capacity(roots.len() * (layers as usize + 1));
    for layer in 0..layers {
        for (i, r) in roots.iter().enumerate() {
            let mut rng = rng_for(spec.seed, layer, i);
            let (mut root, mut n) = (r.position, r.normal);
            if layer > 0 {
                // Outer layers start between the inner roots to cover gaps.
                let t = unit_vec(&mut rng);
                let t = (t - n * t.dot(n)) * spacing * 0.5;
                if let Some(c) = bvh.closest(root + t, f32::INFINITY) {
                    root = c.point;
                    n = bvh.normal(c.tri);
                }
            }
            let g = Growth { lift: spec.lift + gap * layer as f32, noise: spec.noise, length: 1.0 };
            let (points, outward) = integrate(bvh, root, n, center, spec, &mut rng, &g);
            let slot = (rng.next_u64() % 4) as u32;
            out.push(Guide { points, outward, width: 1.0 - 0.12 * layer as f32, layer, slot, flyaway: false });
        }
    }
    let count = if cards { ((roots.len() as f32 * spec.flyaways).round() as usize).min(roots.len()) } else { 0 };
    if let Some(step) = roots.len().checked_div(count) {
        let step = step.max(1);
        for j in 0..count {
            let i = (j * step).min(roots.len() - 1);
            let r = roots[i];
            let mut rng = rng_for(spec.seed, u32::MAX, i);
            let g = Growth { lift: spec.lift + gap * layers as f32, noise: (spec.noise * 2.5).min(1.0), length: 1.1 };
            let (points, outward) = integrate(bvh, r.position, r.normal, center, spec, &mut rng, &g);
            let slot = (rng.next_u64() % 4) as u32;
            out.push(Guide { points, outward, width: 0.35, layer: layers, slot, flyaway: true });
        }
    }
    Ok(out)
}

/// Guide points inside the parent.
///
/// Complexity: O(points x log F).
pub(crate) fn penetrating(bvh: &TriBvh, guides: &[Guide]) -> usize {
    guides
        .iter()
        .flat_map(|g| g.points.iter())
        .filter(|&&p| bvh.closest(p, f32::INFINITY).is_some_and(|c| (p - c.point).dot(bvh.normal(c.tri)) < -1e-5))
        .count()
}
