//! The wood: each branch chain swept as a tapered tube on
//! rotation-minimizing frames. A branch starts inside its parent, heading
//! along the parent, and bends out on a curve, so it emerges from the
//! parent's surface at a shallow angle with a collar instead of being stuck
//! on. The trunk's base flares into buttress roots that are part of its own
//! surface: lobes of the cross-section, growing toward the ground.

use std::f32::consts::TAU;

use glam::{Vec2, Vec3, Vec4};
use p3d_geo::generate::Rng;

use crate::build::{Builder, Corner};
use crate::groups;
use crate::hull::Hull;
use crate::skeleton::Skeleton;
use crate::{Foliage, Species, Style, TreeSpec, VegError};

/// What the wood pass built.
#[derive(Debug, Clone, Default)]
pub(crate) struct WoodOut {
    /// Chains built as tubes.
    pub chains: usize,
    pub triangles: usize,
}

/// One ring of a tube.
#[derive(Debug, Clone, Copy)]
struct Ring {
    center: Vec3,
    radius: f32,
    /// Buttress lobe amplitude (share of the radius) and general swell.
    lobes: f32,
    swell: f32,
    /// Extra frame turn (twist), radians.
    turn: f32,
}

/// Rotation-minimizing frames (tangent, u, v) along a polyline, by double
/// reflection.
///
/// Complexity: O(points).
pub(crate) fn frames(path: &[Vec3]) -> Vec<(Vec3, Vec3, Vec3)> {
    let n = path.len();
    let tangent = |i: usize| (path[(i + 1).min(n - 1)] - path[i.saturating_sub(1)]).normalize_or(Vec3::Y);
    let t0 = tangent(0);
    let mut r = if t0.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    r = (r - t0 * t0.dot(r)).normalize_or(t0.any_orthonormal_vector());
    let mut out = Vec::with_capacity(n);
    out.push((t0, r, t0.cross(r)));
    for i in 1..n {
        let ti = tangent(i - 1);
        let v1 = path[i] - path[i - 1];
        let c1 = v1.dot(v1);
        let t = tangent(i);
        if c1 > 1e-20 {
            let rl = r - v1 * (2.0 / c1 * v1.dot(r));
            let tl = ti - v1 * (2.0 / c1 * v1.dot(ti));
            let v2 = t - tl;
            let c2 = v2.dot(v2);
            r = if c2 < 1e-20 { rl } else { rl - v2 * (2.0 / c2 * v2.dot(rl)) };
        }
        r = (r - t * t.dot(r)).normalize_or(t.any_orthonormal_vector());
        out.push((t, r, t.cross(r)));
    }
    out
}

/// Sides around a tube of each level.
fn sides(spec: &TreeSpec, level: u8) -> usize {
    let low = spec.style == Style::LowPoly;
    match level {
        0 => {
            let k = if spec.roots > 0 { (spec.roots as usize * if low { 2 } else { 5 }).max(if low { 6 } else { 16 }) } else { 12 };
            if low { k.min(12) } else { k.clamp(12, 40) }
        }
        1 => {
            if low {
                6
            } else {
                9
            }
        }
        2 => {
            if low {
                4
            } else {
                5
            }
        }
        _ => 4,
    }
}

/// Share of the triangle budget the wood may use.
fn wood_share(spec: &TreeSpec) -> f32 {
    match spec.foliage {
        Foliage::Cards => {
            if spec.species == Species::Palm {
                0.45
            } else {
                0.3
            }
        }
        Foliage::Clumps => 0.4,
        Foliage::Shell => 0.6,
        Foliage::None => 1.0,
    }
}

/// Triangles of a tube with `rings` rings, a tip and a base cap.
fn tube_tris(rings: usize, sides: usize) -> usize {
    2 * sides * rings.saturating_sub(1) + sides + sides.saturating_sub(2)
}

/// Builds the wood into `b`.
///
/// Complexity: O(N + W) for N skeleton nodes and W wood faces, plus
/// O(C log C) to rank C chains for the budget.
pub(crate) fn build(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, b: &mut Builder) -> Result<WoodOut, VegError> {
    let n_chains = sk.chains.len();
    let mut meshed = vec![false; n_chains];
    // Rank the chains by base radius; take them while the budget lasts and
    // their parent is built.
    let mut order: Vec<usize> = (0..n_chains).filter(|&c| c == 0 || (sk.chains[c].level as u32 <= spec.levels && sk.chains[c].len >= 2)).collect();
    let base_radius = |c: usize| {
        let nodes = sk.chain(c);
        nodes.get(1).or(nodes.first()).map_or(0.0, |&n| sk.radius[n as usize])
    };
    order.sort_by(|&a, &c| base_radius(c).total_cmp(&base_radius(a)).then(a.cmp(&c)));
    let budget = (spec.budget as f32 * wood_share(spec)) as usize;
    let mut used = 0usize;
    let min_r = min_radius(spec);
    let mut chain_parent = vec![usize::MAX; n_chains];
    for (c, cp) in chain_parent.iter_mut().enumerate().skip(1) {
        if let Some(&a) = sk.chain(c).first() {
            *cp = sk.chain_of[a as usize] as usize;
        }
    }
    for &c in &order {
        let lvl = sk.chains[c].level.min(3);
        let s = sides(spec, lvl);
        let rings = rings_of(spec, sk, c).len();
        let cost = tube_tris(rings, s);
        let parent_ok = c == 0 || meshed[chain_parent[c]];
        if c != 0 && (!parent_ok || base_radius(c) < min_r || used + cost > budget) {
            continue;
        }
        meshed[c] = true;
        used += cost;
    }
    let g_trunk = b.group(groups::TRUNK);
    let g_roots = b.group(groups::ROOTS);
    let g_levels = [b.group("level_1"), b.group("level_2"), b.group("level_3")];
    let mut out = WoodOut::default();
    let mut rng = Rng::new(spec.seed ^ 0xBA4C_0002);
    let root_az: Vec<f32> = (0..spec.roots).map(|i| (i as f32 + (rng.unit() - 0.5) * 0.45) / spec.roots.max(1) as f32 * TAU).collect();
    for (c, _) in meshed.iter().enumerate().filter(|(_, m)| **m) {
        let lvl = sk.chains[c].level.min(3);
        let rings = rings_of(spec, sk, c);
        let group = if c == 0 { g_trunk } else { g_levels[(lvl as usize).clamp(1, 3) - 1] };
        let tip = tip_of(sk, c, &rings);
        tube(spec, sk, hull, b, c, &rings, tip, sides(spec, if c == 0 { 0 } else { lvl.max(1) }), group, g_roots, &root_az);
        out.chains += 1;
    }
    out.triangles = b.triangles();
    Ok(out)
}

/// The rings of a chain: a curved emergence from the parent for branches,
/// the root flare and a rounded top for the trunk.
///
/// Complexity: O(chain nodes).
fn rings_of(spec: &TreeSpec, sk: &Skeleton, c: usize) -> Vec<Ring> {
    let nodes = sk.chain(c);
    let mut out: Vec<Ring> = Vec::with_capacity(nodes.len() + 8);
    let lvl = sk.chains[c].level;
    let len_total: f32 = nodes.windows(2).map(|w| sk.pos[w[0] as usize].distance(sk.pos[w[1] as usize])).sum::<f32>().max(1e-6);
    if c == 0 {
        // Trunk: a ring below the ground, dense rings through the flare,
        // then one per node.
        let r0 = sk.radius[nodes[0] as usize];
        let flare_h = r0 * 3.6;
        // Ridges reach root_spread times the radius at the ground.
        let spread = (spec.root_spread / 1.06 - 1.0).max(0.0);
        let at = |y: f32| -> Vec3 {
            // The trunk polyline at height y.
            for w in nodes.windows(2) {
                let (a, b) = (sk.pos[w[0] as usize], sk.pos[w[1] as usize]);
                if y <= b.y {
                    let s = ((y - a.y) / (b.y - a.y).max(1e-6)).clamp(0.0, 1.0);
                    return a.lerp(b, s);
                }
            }
            sk.pos[nodes[nodes.len() - 1] as usize]
        };
        let ring = |y: f32, center: Vec3| {
            let k = 1.0 - (y.max(0.0) / flare_h).min(1.0);
            Ring { center, radius: sk.radius[nodes[0] as usize], lobes: spread * k.powf(1.6), swell: 0.06 * k * k, turn: 0.0 }
        };
        // Below the ground the roots keep spreading as they dive.
        let mut under = ring(0.0, Vec3::new(0.0, -r0 * 0.3, 0.0));
        under.lobes *= 1.25;
        out.push(under);
        if spread > 0.0 && spec.roots > 0 {
            for f in [0.0, 0.08, 0.18, 0.32, 0.5, 0.72, 1.0] {
                let y = flare_h * f;
                out.push(ring(y, at(y)));
            }
        }
        let last_flare = out.last().map_or(0.0, |r| r.center.y);
        let top = sk.pos[nodes[nodes.len() - 1] as usize].y.max(1e-6);
        let above: Vec<u32> =
            nodes.iter().copied().filter(|&n| sk.pos[n as usize].y > last_flare + flare_h * 0.3 || n == nodes[nodes.len() - 1]).collect();
        for k in keep(sk, &above, r0 * 3.0, spec.step() * 2.0) {
            let n = above[k];
            let p = sk.pos[n as usize];
            let mut r = ring(p.y, p);
            r.radius = sk.radius[n as usize];
            r.turn = spec.twist * (p.y / top).min(1.0);
            out.push(r);
        }
        for r in &mut out {
            r.radius = r.radius.max(1e-4);
            if r.turn == 0.0 {
                r.turn = spec.twist * (r.center.y.max(0.0) / top).min(1.0);
            }
        }
        return out;
    }
    // A branch: from inside the parent, along it, curving out to its own
    // second node; thin branches keep every other node.
    let a = nodes[0];
    let n1 = nodes[1];
    let parent_dir = sk.dir(a);
    let pr = sk.radius[a as usize];
    // Scaffold limbs at a decurrent fork split the trunk itself: they start
    // deep in its top as wide as most of it.
    let split = a == sk.fork && lvl == 1 && matches!(spec.species, Species::Broadleaf | Species::Stylized);
    let target = sk.pos[n1 as usize];
    let mut start = sk.pos[a as usize] - parent_dir * pr * if split { 1.1 } else { 0.35 };
    if split {
        // Off the axis toward the limb, so the limbs together fill the trunk.
        let out_dir = target - sk.pos[a as usize];
        let side = (out_dir - parent_dir * out_dir.dot(parent_dir)).normalize_or_zero();
        start += side * pr * 0.3;
    }
    let reach = start.distance(target);
    let ctrl = start + parent_dir * reach * 0.5;
    let r1 = sk.radius[n1 as usize];
    let emerge: &[(f32, f32)] = if lvl <= 1 { &[(0.0, 1.3), (0.34, 1.18), (0.67, 1.07)] } else { &[(0.0, 1.18), (0.5, 1.08)] };
    for &(u, collar) in emerge {
        let p = start * (1.0 - u) * (1.0 - u) + ctrl * 2.0 * u * (1.0 - u) + target * u * u;
        let radius = if split { (pr * 0.8 * (1.0 - u) + r1 * collar * u).max(r1 * collar).min(pr * 0.95) } else { (r1 * collar).min(pr * 0.95) };
        out.push(Ring { center: p, radius, lobes: 0.0, swell: 0.0, turn: 0.0 });
    }
    let own = &nodes[1..];
    let seg = if lvl <= 1 { spec.step() * 1.5 } else { spec.step() * 2.5 };
    let kept = keep(sk, own, r1 * if lvl <= 1 { 4.0 } else { 5.0 }, seg);
    let mut along = vec![0.0f32; own.len()];
    for i in 1..own.len() {
        along[i] = along[i - 1] + sk.pos[own[i - 1] as usize].distance(sk.pos[own[i] as usize]);
    }
    let min_r = min_radius(spec);
    for k in kept.into_iter().skip(1) {
        let n = own[k];
        // Branches end where they get thinner than wood is built.
        if sk.radius[n as usize] < min_r && out.len() >= 3 {
            break;
        }
        let twist = if lvl == 1 { spec.twist * along[k] / len_total } else { 0.0 };
        out.push(Ring { center: sk.pos[n as usize], radius: sk.radius[n as usize].max(1e-4), lobes: 0.0, swell: 0.0, turn: twist });
    }
    out
}

/// Indices of the nodes worth a ring: the first and last, and wherever the
/// branch has run `seg` (at least `min_seg`) or turned by more than 12
/// degrees since the last kept node.
///
/// Complexity: O(nodes).
fn keep(sk: &Skeleton, nodes: &[u32], seg: f32, min_seg: f32) -> Vec<usize> {
    let mut out = Vec::with_capacity(nodes.len());
    if nodes.is_empty() {
        return out;
    }
    out.push(0);
    let seg = seg.max(min_seg);
    let cos_turn = 12f32.to_radians().cos();
    let mut last_dir = sk.dir(nodes[0]);
    let mut run = 0.0f32;
    for i in 1..nodes.len() {
        let (a, b) = (sk.pos[nodes[i - 1] as usize], sk.pos[nodes[i] as usize]);
        run += a.distance(b);
        let dir = sk.dir(nodes[i]);
        if i == nodes.len() - 1 || run >= seg || dir.dot(last_dir) < cos_turn {
            out.push(i);
            run = 0.0;
            last_dir = dir;
        }
    }
    out
}

/// Where a tube ends: a point a little past its last ring.
fn tip_of(sk: &Skeleton, c: usize, rings: &[Ring]) -> Vec3 {
    let n = rings.len();
    let last = rings[n - 1];
    let dir = if n >= 2 { (last.center - rings[n - 2].center).normalize_or(Vec3::Y) } else { Vec3::Y };
    let k = if c == 0 { 0.9 } else { 1.6 };
    let _ = sk;
    last.center + dir * last.radius * k
}

/// Wind stiffness of a chain level (1 rigid).
fn stiffness(level: u8) -> f32 {
    match level {
        0 => 1.0,
        1 => 0.72,
        2 => 0.45,
        _ => 0.28,
    }
}

/// Emits one tube: quads between rings, a fan to the tip; the trunk gets a
/// cap under the ground. Groups: `group`, the trunk's flare in `roots`.
///
/// Complexity: O(rings x sides).
#[allow(clippy::too_many_arguments)]
fn tube(
    spec: &TreeSpec,
    sk: &Skeleton,
    hull: &Hull,
    b: &mut Builder,
    c: usize,
    rings: &[Ring],
    tip: Vec3,
    sides: usize,
    group: usize,
    roots: usize,
    root_az: &[f32],
) {
    let chain = &sk.chains[c];
    let level = chain.level.min(3);
    let path: Vec<Vec3> = rings.iter().map(|r| r.center).collect();
    let fr = frames(&path);
    let base_r = rings.iter().map(|r| r.radius).fold(0.0f32, f32::max).max(1e-4);
    let circumference = TAU * base_r;
    let lobe_power = 16.0;
    let mut along = 0.0f32;
    let mut vs: Vec<f32> = Vec::with_capacity(rings.len());
    let first = b.pos.len() as u32;
    for (i, (r, &(_, u, v))) in rings.iter().zip(&fr).enumerate() {
        if i > 0 {
            along += r.center.distance(rings[i - 1].center);
        }
        vs.push(along / circumference);
        for k in 0..sides {
            let a = k as f32 / sides as f32 * TAU + r.turn;
            let mut rad = r.radius * (1.0 + r.swell);
            if r.lobes > 0.0 {
                let lobe = root_az.iter().map(|&az| ((a - r.turn - az).cos().max(0.0)).powf(lobe_power)).fold(0.0f32, f32::max);
                rad *= 1.0 + r.lobes * lobe;
            }
            let mut p = r.center + (u * a.cos() + v * a.sin()) * rad;
            if c == 0 && i == 0 {
                p.y = r.center.y;
            }
            b.vert(p);
        }
    }
    let total = along.max(1e-6);
    let apex = b.vert(tip);
    let phase = chain.phase;
    let wind_level = f32::from(level) / 3.0;
    let shade = |p: Vec3| {
        let inside = (1.0 - hull.share(p)).clamp(0.0, 1.0);
        1.0 - spec.gradient * 0.35 * inside
    };
    let corner = |vtx: u32, uv: Vec2, along_share: f32, p: Vec3| {
        let stiff = stiffness(level) * (1.0 - 0.45 * along_share);
        let s = shade(p);
        Corner { v: vtx, uv, wind: Vec2::new(phase, stiff), color: Vec4::new(s, s, s, wind_level), normal: Vec3::ZERO }
    };
    let flare_top = rings.first().map_or(0.0, |r| r.radius) * 3.2 * 0.45;
    for i in 0..rings.len() - 1 {
        let (v0, v1) = (vs[i], vs[i + 1]);
        let (s0, s1) = (vs[i] * circumference / total, vs[i + 1] * circumference / total);
        let in_roots = c == 0 && spec.roots > 0 && rings[i].center.y < flare_top;
        let g = if in_roots { roots } else { group };
        for k in 0..sides {
            let k1 = (k + 1) % sides;
            let (a, bb) = (first + (i * sides + k) as u32, first + (i * sides + k1) as u32);
            let (cc, d) = (bb + sides as u32, a + sides as u32);
            let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
            let pa = b.pos[a as usize];
            b.face(
                &[
                    corner(a, Vec2::new(u0, v0), s0, pa),
                    corner(bb, Vec2::new(u1, v0), s0, pa),
                    corner(cc, Vec2::new(u1, v1), s1, pa),
                    corner(d, Vec2::new(u0, v1), s1, pa),
                ],
                &[g],
                groups::BARK_MATERIAL,
            );
        }
    }
    let last = first + ((rings.len() - 1) * sides) as u32;
    let v_last = vs[rings.len() - 1];
    let v_tip = v_last + rings[rings.len() - 1].radius / circumference;
    for k in 0..sides {
        let k1 = (k + 1) % sides;
        let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
        let p = b.pos[(last + k as u32) as usize];
        b.face(
            &[
                corner(last + k as u32, Vec2::new(u0, v_last), 1.0, p),
                corner(last + k1 as u32, Vec2::new(u1, v_last), 1.0, p),
                corner(apex, Vec2::new((u0 + u1) * 0.5, v_tip), 1.0, p),
            ],
            &[group],
            groups::BARK_MATERIAL,
        );
    }
    // Close the base: under the ground for the trunk, inside the parent for
    // a branch.
    let cap: Vec<Corner> =
        (0..sides).rev().map(|k| corner(first + k as u32, Vec2::new(k as f32 / sides as f32, 0.0), 0.0, rings[0].center)).collect();
    b.face(&cap, &[if c == 0 && spec.roots > 0 { roots } else { group }], groups::BARK_MATERIAL);
    if c != 0 {
        b.vert_group(groups::TWIG_TIPS, apex);
    }
}

/// Thinnest branch radius built as wood (thinner twigs are left to the
/// cards; low poly keeps only chunky limbs).
fn min_radius(spec: &TreeSpec) -> f32 {
    spec.trunk_radius * if spec.style == Style::LowPoly { 0.12 } else { 0.012 }
}
