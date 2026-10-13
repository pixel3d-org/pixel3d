//! The branch skeleton: a tree of nodes grown from the trunk. The trunk and
//! the scaffold limbs (or whorls, for conifers) are laid out from the
//! species; finer branches grow toward attractors scattered in the crown
//! hull by space colonization (Runions et al. 2007). Radii follow the pipe
//! model, and the nodes are split into branch chains with levels.

use glam::{IVec3, Vec2, Vec3};
use p3d_geo::deform::noise3;
use p3d_geo::generate::Rng;
use rustc_hash::FxHashMap;

use crate::hull::Hull;
use crate::{MAX_NODES, Species, TreeSpec, VegError};

/// Parent of the root node.
pub(crate) const NONE: u32 = u32::MAX;

/// A branch: its nodes from the attachment point on the parent chain (the
/// trunk's first node is the root) to the tip.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chain {
    /// Range in [`Skeleton::chain_nodes`].
    pub start: u32,
    pub len: u32,
    /// 0 trunk (and leader), 1 scaffold limbs, and so on.
    pub level: u8,
    /// Wind phase (0 to 1).
    pub phase: f32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Skeleton {
    pub pos: Vec<Vec3>,
    pub parent: Vec<u32>,
    pub radius: Vec<f32>,
    /// Tips carried (pipe model weight).
    pub weight: Vec<f32>,
    /// The chain each node belongs to (the root's is the trunk).
    pub chain_of: Vec<u32>,
    pub chains: Vec<Chain>,
    pub chain_nodes: Vec<u32>,
    /// First fork (broadleaf), crown base on the leader (conifer, birch) or
    /// the top of the trunk (palm).
    pub fork: u32,
    /// Last node of the trunk chain.
    pub top: u32,
    /// Crown axis (x, z).
    pub crown_axis: Vec2,
    /// Scaffold limbs, whorl branches or fronds.
    pub limbs: usize,
}

impl Skeleton {
    pub fn chain(&self, c: usize) -> &[u32] {
        let ch = &self.chains[c];
        &self.chain_nodes[ch.start as usize..(ch.start + ch.len) as usize]
    }

    /// Unit direction of the branch at node `n` (from its parent).
    pub fn dir(&self, n: u32) -> Vec3 {
        match self.parent[n as usize] {
            NONE => Vec3::Y,
            p => (self.pos[n as usize] - self.pos[p as usize]).normalize_or(Vec3::Y),
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A unit vector `angle` away from `axis`, turned `azimuth` around it.
fn cone_dir(axis: Vec3, angle: f32, azimuth: f32) -> Vec3 {
    let u = if axis.y.abs() > 0.99 { axis.cross(Vec3::X).normalize_or(Vec3::Z) } else { axis.cross(Vec3::Y).normalize_or(Vec3::Z) };
    let w = axis.cross(u);
    (axis * angle.cos() + (u * azimuth.cos() + w * azimuth.sin()) * angle.sin()).normalize_or(axis)
}

/// Nodes under construction.
struct Grower {
    pos: Vec<Vec3>,
    parent: Vec<u32>,
    /// Can grow new branches by colonization.
    growable: Vec<bool>,
    /// Linked children lists.
    first_child: Vec<u32>,
    next_sibling: Vec<u32>,
    children: Vec<u8>,
}

impl Grower {
    fn add(&mut self, p: Vec3, parent: u32, growable: bool) -> Result<u32, VegError> {
        if self.pos.len() >= MAX_NODES {
            return Err(VegError::new(
                "attractors",
                format!("the skeleton would exceed {MAX_NODES} nodes; use fewer attractors or a larger branch_spacing"),
            ));
        }
        let id = self.pos.len() as u32;
        self.pos.push(p);
        self.parent.push(parent);
        self.growable.push(growable);
        self.first_child.push(NONE);
        self.next_sibling.push(NONE);
        self.children.push(0);
        if parent != NONE {
            let pi = parent as usize;
            self.next_sibling[id as usize] = self.first_child[pi];
            self.first_child[pi] = id;
            self.children[pi] = self.children[pi].saturating_add(1);
        }
        Ok(id)
    }

    /// Whether `parent` already has a child within `eps` of `p`.
    ///
    /// Complexity: O(children of parent), at most 4.
    fn has_child_near(&self, parent: u32, p: Vec3, eps: f32) -> bool {
        let mut c = self.first_child[parent as usize];
        while c != NONE {
            if self.pos[c as usize].distance_squared(p) < eps * eps {
                return true;
            }
            c = self.next_sibling[c as usize];
        }
        false
    }

    /// Appends a polyline of `n` steps from `from` in direction `dir`,
    /// bending by `bend(i, dir)` after each step. Returns the node ids.
    ///
    /// Complexity: O(n).
    fn march(
        &mut self,
        from: u32,
        n: usize,
        step: f32,
        mut dir: Vec3,
        growable: bool,
        mut bend: impl FnMut(usize, Vec3, Vec3) -> Vec3,
    ) -> Result<Vec<u32>, VegError> {
        let mut out = Vec::with_capacity(n);
        let mut at = from;
        for i in 0..n {
            let p = self.pos[at as usize] + dir * step;
            at = self.add(p, at, growable)?;
            out.push(at);
            dir = bend(i, dir, p).normalize_or(dir);
        }
        Ok(out)
    }
}

/// Grows the skeleton of a tree.
///
/// Complexity: O(A x I x k) for colonization with A attractors, I rounds
/// and k nodes added near each attractor per round (hash grid), plus O(N)
/// for radii and chains over N nodes.
pub(crate) fn grow(spec: &TreeSpec) -> Result<Skeleton, VegError> {
    let step = spec.step();
    let h = spec.height;
    let mut rng = Rng::new(spec.seed ^ 0x7EE5_0001);
    let mut g =
        Grower { pos: Vec::new(), parent: Vec::new(), growable: Vec::new(), first_child: Vec::new(), next_sibling: Vec::new(), children: Vec::new() };
    let lean_dir = Vec3::new(spec.lean_toward.cos() * spec.lean.sin(), spec.lean.cos(), spec.lean_toward.sin() * spec.lean.sin());
    // The trunk: to the fork (decurrent) or the top (leaders, palms).
    let trunk_top = match spec.species {
        Species::Broadleaf | Species::Stylized => spec.fork * h,
        Species::Palm => spec.fork * h,
        Species::Conifer => h * 0.985,
        Species::Birch => h * 0.9,
    };
    let trunk_steps = ((trunk_top / step).ceil() as usize).clamp(3, 4096);
    let trunk_step = trunk_top / lean_dir.y.max(0.5) / trunk_steps as f32;
    let seed = spec.seed;
    let wobble = spec.gnarl * 0.05 * trunk_top;
    let root = g.add(Vec3::ZERO, NONE, false)?;
    let mut trunk = vec![root];
    {
        let mut at = root;
        let mut dir = lean_dir;
        let mut clean = Vec3::ZERO;
        for i in 0..trunk_steps {
            let s = (i + 1) as f32 / trunk_steps as f32;
            clean += dir * trunk_step;
            if spec.species == Species::Palm {
                // Palms lean at the base and turn back toward the light.
                dir = lean_dir.lerp(Vec3::Y, s.powf(1.5) * 0.7).normalize_or(Vec3::Y);
            }
            // A low-frequency sideways wobble, growing from the base.
            let q = Vec3::new(s * 2.3, 0.0, 0.0);
            let off = Vec3::new(noise3(q, seed), 0.0, noise3(q + Vec3::splat(17.0), seed)) * wobble * s;
            at = g.add(clean + off, at, false)?;
            trunk.push(at);
        }
    }
    let fork_idx = match spec.species {
        Species::Conifer | Species::Birch => {
            let base = spec.hull(Vec2::ZERO).base;
            trunk.iter().copied().find(|&n| g.pos[n as usize].y >= base).unwrap_or(root)
        }
        _ => *trunk.last().unwrap_or(&root),
    };
    let top = *trunk.last().unwrap_or(&root);
    let axis_node = match spec.species {
        Species::Conifer | Species::Birch => {
            let mid = (spec.hull(Vec2::ZERO).base + h) / 2.0;
            trunk.iter().copied().min_by(|&a, &b| (g.pos[a as usize].y - mid).abs().total_cmp(&(g.pos[b as usize].y - mid).abs())).unwrap_or(top)
        }
        _ => fork_idx,
    };
    let ap = g.pos[axis_node as usize];
    let crown_axis = Vec2::new(ap.x, ap.z);
    let hull = spec.hull(crown_axis);
    let mut limbs = 0usize;
    let mut seeds: Vec<u32> = Vec::new();
    match spec.species {
        Species::Broadleaf | Species::Stylized => {
            limbs = spec.limbs.max(1) as usize;
            let az0 = rng.unit() * std::f32::consts::TAU;
            for i in 0..limbs {
                // Varied angles: the first limbs more upright, the last ones wider.
                let spread_k = if limbs > 1 { i as f32 / (limbs - 1) as f32 } else { 0.5 };
                let angle = spec.branch_angle * (0.72 + 0.5 * spread_k) * (0.92 + 0.16 * rng.unit());
                let az = az0 + std::f32::consts::TAU * (i as f32 + (rng.unit() - 0.5) * 0.5) / limbs as f32;
                let dir = cone_dir(lean_dir, angle.min(1.45), az);
                // Each limb climbs until it is a little more than halfway to the hull.
                let reach = hull.radius * 0.55 / angle.sin().max(0.35);
                let n = ((reach / step).ceil() as usize).clamp(2, 512);
                let gnarl = spec.gnarl;
                let s0 = seed.wrapping_add(i as u64 * 7919);
                let nodes = g.march(fork_idx, n, step, dir, true, |k, d, p| {
                    let wob = Vec3::new(noise3(p * 0.6, s0), noise3(p * 0.6 + Vec3::splat(5.0), s0), noise3(p * 0.6 + Vec3::splat(11.0), s0));
                    // Limbs turn up toward the light and wander with the gnarl.
                    d + Vec3::Y * 0.035 * (1.0 + k as f32 * 0.02) + wob * gnarl * 0.18
                })?;
                seeds.extend(nodes);
            }
        }
        Species::Birch => {
            seeds.extend(trunk.iter().copied().filter(|&n| g.pos[n as usize].y >= hull.base));
        }
        Species::Conifer => {
            limbs = whorls(spec, &mut g, &trunk, &hull, &mut rng)?;
        }
        Species::Palm => {
            limbs = spec.limbs as usize;
        }
    }
    let laid_out = g.pos.len() as u32;
    if matches!(spec.species, Species::Broadleaf | Species::Stylized | Species::Birch) {
        for &s in &seeds {
            g.growable[s as usize] = true;
        }
        colonize(spec, &mut g, &hull, &seeds, &mut rng)?;
        smooth(&mut g, laid_out);
    }
    finish(spec, g, fork_idx, top, crown_axis, limbs, &mut rng)
}

/// Conifer whorls: rings of drooping branches along the leader from the
/// crown base to the top, as long as the hull is wide there, with flat
/// side shoots alternating left and right. Returns the branch count.
///
/// Complexity: O(whorls x limbs x branch nodes).
fn whorls(spec: &TreeSpec, g: &mut Grower, trunk: &[u32], hull: &Hull, rng: &mut Rng) -> Result<usize, VegError> {
    let step = spec.step().min(spec.height / 12.0);
    let spacing = if spec.branch_spacing > 0.0 { spec.branch_spacing } else { spec.height / 28.0 };
    let shoot_step = (spacing * 0.6).max(0.02);
    let mut y = hull.base + spacing * 0.3;
    let mut count = 0usize;
    let mut az = rng.unit() * std::f32::consts::TAU;
    let droop = 0.06 * (0.5 + spec.droop);
    // One branch with its side shoots, from the leader node at height y.
    let branch = |g: &mut Grower, y: f32, a: f32, scale: f32, rng: &mut Rng| -> Result<bool, VegError> {
        let Some(&at) = trunk.iter().find(|&&n| g.pos[n as usize].y >= y) else { return Ok(false) };
        let t = hull.t(y).clamp(0.0, 1.0);
        let reach = hull.radius_at(y).max(spacing);
        // Lower branches droop past horizontal, upper ones reach up.
        let angle = spec.branch_angle + (0.28 - 0.62 * t) * (1.0 + 0.2 * (rng.unit() - 0.5));
        let dir = cone_dir(Vec3::Y, angle.clamp(0.2, 2.2), a);
        let len = reach * (0.86 + 0.14 * rng.unit()) * scale;
        let n = ((len / step).ceil() as usize).clamp(2, 256);
        let nodes = g.march(at, n, len / n as f32, dir, false, |_, d, _| d - Vec3::Y * droop)?;
        // Side shoots in the branch's plane, shorter toward the tip.
        for (j, &node) in nodes.iter().enumerate().take(n.saturating_sub(1)) {
            let d = (g.pos[node as usize] - g.pos[g.parent[node as usize] as usize]).normalize_or(dir);
            let side = d.cross(Vec3::Y).normalize_or(Vec3::X) * if j % 2 == 0 { 1.0 } else { -1.0 };
            let remain = len * (1.0 - (j + 1) as f32 / n as f32);
            let sl = (remain * 0.42 + shoot_step).min(len * 0.4);
            let sn = ((sl / shoot_step).ceil() as usize).clamp(1, 6);
            let sd = (d * 0.62 + side * 0.78).normalize_or(d);
            g.march(node, sn, sl / sn as f32, sd, false, |_, d, _| d - Vec3::Y * droop)?;
        }
        Ok(true)
    };
    while y < hull.top - spacing * 0.5 {
        let per = spec.limbs.max(1);
        az += 2.39996;
        for k in 0..per {
            let a = az + std::f32::consts::TAU * (k as f32 + (rng.unit() - 0.5) * 0.4) / per as f32;
            if branch(g, y, a, 1.0, rng)? {
                count += 1;
            }
        }
        let gap = spacing * (0.85 + 0.3 * rng.unit());
        // Smaller branches between the whorls fill the tiers.
        for k in 0..2 {
            let a = az + 1.3 + k as f32 * std::f32::consts::PI + rng.unit();
            if branch(g, y + gap * (0.35 + 0.3 * k as f32), a, 0.62, rng)? {
                count += 1;
            }
        }
        y += gap;
    }
    Ok(count)
}

/// Space colonization: attractors scattered in the hull (denser in the
/// outer shell and sparse in the open lower middle) pull the nearest
/// growable node within the influence radius; each pulled node grows one
/// step toward their mean direction; attractors closer than the kill
/// distance to a node die. Each attractor keeps its nearest node and only
/// checks the nodes added near it in a round, through a hash grid.
///
/// Complexity: O(A x I x k) for A attractors, I rounds (at most 240) and k
/// nodes added per grid neighbourhood per round.
fn colonize(spec: &TreeSpec, g: &mut Grower, hull: &Hull, seeds: &[u32], rng: &mut Rng) -> Result<(), VegError> {
    let step = spec.step();
    let wanted =
        if spec.attractors > 0 { spec.attractors as usize } else { ((hull.volume() / (step * step * step)) * 0.3).clamp(300.0, 8000.0) as usize };
    let (inner, open_low) = match spec.species {
        Species::Birch => (0.2, 0.5),
        Species::Stylized => (0.3, 0.4),
        _ => (0.3, 0.15),
    };
    let mut attractors: Vec<Vec3> = Vec::with_capacity(wanted);
    let mut tries = 0usize;
    while attractors.len() < wanted && tries < wanted * 40 {
        tries += 1;
        let p = hull.sample(rng);
        let s = hull.share(p);
        let t = hull.t(p.y);
        // Dense outer shell, a sparse core, and an open middle underneath
        // (except at the drooping rim).
        let shell = 0.06 + 0.94 * smoothstep(inner, 0.95, s);
        let low = open_low + (1.0 - open_low) * smoothstep(0.0, 0.32, t).max(smoothstep(0.75, 0.95, s));
        if rng.unit() < shell * low {
            attractors.push(p);
        }
    }
    let di = step * 9.0;
    let dk = step * 1.5;
    let tropism = match spec.species {
        Species::Birch => Vec3::Y * 0.12,
        Species::Stylized => Vec3::Y * 0.25,
        _ => Vec3::Y * 0.45,
    };
    let jitter = if spec.style == crate::Style::LowPoly { 0.1 + spec.gnarl * 0.1 } else { 0.08 + spec.gnarl * 0.25 };
    let max_turn = (12.0 + 18.0 * spec.gnarl).to_radians();
    let max_branch = spec.branch_angle.clamp(0.35, 1.4);
    let min_branch = (max_branch * 0.45).min(0.4);
    let n_attr = attractors.len();
    let mut alive = vec![true; n_attr];
    let mut nearest = vec![NONE; n_attr];
    let mut best = vec![f32::INFINITY; n_attr];
    let cell = |p: Vec3, size: f32| (p / size).floor().as_ivec3();
    let mut fresh: Vec<u32> = seeds.to_vec();
    let mut grid: FxHashMap<IVec3, Vec<u32>> = FxHashMap::default();
    let mut pull = vec![Vec3::ZERO; g.pos.len()];
    let mut pulled: Vec<u32> = Vec::new();
    for _round in 0..240 {
        if fresh.is_empty() {
            break;
        }
        // The nodes added last round, on a grid of influence-sized cells.
        grid.clear();
        for &n in &fresh {
            grid.entry(cell(g.pos[n as usize], di)).or_default().push(n);
        }
        let mut any = false;
        for (i, a) in attractors.iter().enumerate() {
            if !alive[i] {
                continue;
            }
            let k = cell(*a, di);
            let mut killed = false;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = grid.get(&(k + IVec3::new(dx, dy, dz))) else { continue };
                        for &n in list {
                            let d2 = g.pos[n as usize].distance_squared(*a);
                            if d2 < dk * dk {
                                killed = true;
                            }
                            if d2 < best[i] {
                                best[i] = d2;
                                nearest[i] = n;
                            }
                        }
                    }
                }
            }
            if killed {
                alive[i] = false;
                continue;
            }
            any = true;
        }
        if !any {
            break;
        }
        // Pull every node toward the attractors it is nearest to.
        pull.resize(g.pos.len(), Vec3::ZERO);
        pulled.clear();
        for (i, a) in attractors.iter().enumerate() {
            let n = nearest[i];
            if !alive[i] || n == NONE || best[i] > di * di {
                continue;
            }
            let ni = n as usize;
            if pull[ni] == Vec3::ZERO {
                pulled.push(n);
            }
            pull[ni] += (*a - g.pos[ni]).normalize_or_zero() + Vec3::splat(1e-7);
        }
        pulled.sort_unstable();
        fresh.clear();
        for &n in &pulled {
            let ni = n as usize;
            let mean = pull[ni].normalize_or_zero();
            pull[ni] = Vec3::ZERO;
            if !g.growable[ni] || g.children[ni] >= 4 {
                continue;
            }
            let prev = match g.parent[ni] {
                NONE => Vec3::Y,
                p => (g.pos[ni] - g.pos[p as usize]).normalize_or(Vec3::Y),
            };
            let wiggle = Vec3::new(rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5) * jitter;
            let want = (mean + prev * 0.25 + tropism + wiggle).normalize_or(mean);
            // A branch keeps going with gentle turns; a new one leaves its
            // parent at a branching angle.
            let dir =
                if g.children[ni] == 0 { turn_toward(prev, want, 0.0, max_turn, rng) } else { turn_toward(prev, want, min_branch, max_branch, rng) };
            let p = g.pos[ni] + dir * step;
            if g.has_child_near(n, p, step * 0.3) {
                continue;
            }
            let id = g.add(p, n, true)?;
            fresh.push(id);
        }
    }
    Ok(())
}

/// `from` turned toward `to` by an angle clamped to `[min, max]` (a
/// random side when they point the same way).
fn turn_toward(from: Vec3, to: Vec3, min: f32, max: f32, rng: &mut Rng) -> Vec3 {
    let a = from.angle_between(to);
    let mut axis = from.cross(to);
    if axis.length_squared() < 1e-10 {
        axis = from.any_orthonormal_vector();
        let spin = glam::Quat::from_axis_angle(from, rng.unit() * std::f32::consts::TAU);
        axis = spin * axis;
    }
    let turn = a.clamp(min, max.max(min));
    (glam::Quat::from_axis_angle(axis.normalize_or(Vec3::X), turn) * from).normalize_or(to)
}

/// Smooths the colonized nodes along their branches (laid-out nodes stay).
///
/// Complexity: O(N) per pass, two passes.
fn smooth(g: &mut Grower, laid_out: u32) {
    for _ in 0..2 {
        let old = g.pos.clone();
        for n in laid_out as usize..g.pos.len() {
            let p = g.parent[n];
            let c = g.first_child[n];
            if p == NONE || c == NONE {
                continue;
            }
            // The first child added is the last in the list: follow to the end.
            let mut first = c;
            while g.next_sibling[first as usize] != NONE {
                first = g.next_sibling[first as usize];
            }
            g.pos[n] = old[n] * 0.5 + (old[p as usize] + old[first as usize]) * 0.25;
        }
    }
}

/// Radii by the pipe model and the split into chains.
///
/// Complexity: O(N log c) for N nodes with at most c children each.
#[allow(clippy::too_many_arguments)]
fn finish(spec: &TreeSpec, g: Grower, fork: u32, top: u32, crown_axis: Vec2, limbs: usize, rng: &mut Rng) -> Result<Skeleton, VegError> {
    let n = g.pos.len();
    // Children in CSR form, by node index.
    let mut start = vec![0u32; n + 1];
    for &p in &g.parent {
        if p != NONE {
            start[p as usize + 1] += 1;
        }
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut fill = start.clone();
    let mut kids = vec![0u32; start[n] as usize];
    for (c, &p) in g.parent.iter().enumerate() {
        if p != NONE {
            kids[fill[p as usize] as usize] = c as u32;
            fill[p as usize] += 1;
        }
    }
    // Pipe model: children always come after their parents.
    let mut weight = vec![0.0f32; n];
    for i in (0..n).rev() {
        let (a, b) = (start[i] as usize, start[i + 1] as usize);
        weight[i] = if a == b { 1.0 } else { kids[a..b].iter().map(|&c| weight[c as usize]).sum::<f32>() + 0.05 };
    }
    let e = std::f32::consts::LN_2 / (1.0 / spec.taper).ln().max(1e-3);
    let fork_w = weight[fork as usize].max(1.0);
    let decurrent = matches!(spec.species, Species::Broadleaf | Species::Stylized);
    let r_fork = spec.trunk_radius * if decurrent { 0.82 } else { 1.0 };
    let r_tip = r_fork / fork_w.powf(1.0 / e);
    let mut radius: Vec<f32> = weight.iter().map(|w| r_tip * w.powf(1.0 / e)).collect();
    // Chains: the heaviest child continues its parent's chain, the others
    // start new ones a level deeper; every branch at a decurrent fork starts
    // a scaffold limb.
    let mut chain_of = vec![0u32; n];
    let mut chains: Vec<Chain> = vec![Chain { start: 0, len: 0, level: 0, phase: rng.unit() }];
    let mut attach: Vec<u32> = vec![NONE];
    let mut order: Vec<u32> = Vec::new();
    for i in 0..n {
        let (a, b) = (start[i] as usize, start[i + 1] as usize);
        if a == b {
            continue;
        }
        order.clear();
        order.extend_from_slice(&kids[a..b]);
        order.sort_by(|&x, &y| weight[y as usize].total_cmp(&weight[x as usize]).then(x.cmp(&y)));
        let c = chain_of[i] as usize;
        let level = chains[c].level;
        let split_all = decurrent && i as u32 == fork;
        for (k, &child) in order.iter().enumerate() {
            if k == 0 && !split_all {
                chain_of[child as usize] = c as u32;
            } else {
                chain_of[child as usize] = chains.len() as u32;
                let phase = (chains[c].phase + 0.15 + rng.unit() * 0.7).fract();
                chains.push(Chain { start: 0, len: 0, level: level.saturating_add(1), phase });
                attach.push(i as u32);
            }
        }
    }
    // Explicit trunk radii (with a gentle taper to the fork), and children
    // never fatter than their parent where they attach.
    let base_r = spec.trunk_radius;
    for i in 0..n {
        let p = g.parent[i];
        if chain_of[i] == 0 && (decurrent || spec.species == Species::Palm) {
            let s = (g.pos[i].y / g.pos[top as usize].y.max(1e-6)).clamp(0.0, 1.0);
            radius[i] = base_r * (1.06 - 0.24 * s);
        } else if chain_of[i] == 0 {
            // Leaders taper toward the top whatever they carry.
            let s = (g.pos[i].y / spec.height).clamp(0.0, 1.0);
            radius[i] = radius[i].min(base_r * 1.06).min(base_r * (1.0 - s).max(0.03).powf(0.9));
        }
        if p != NONE {
            let cap = radius[p as usize] * if chain_of[i] == chain_of[p as usize] { 1.0 } else { 0.78 };
            radius[i] = radius[i].min(cap);
        }
    }
    // Scaffold limbs start thick where they leave the fork.
    if decurrent {
        let mut along = vec![0.0f32; chains.len()];
        let mut total = vec![0.0f32; chains.len()];
        for (i, &ci) in chain_of.iter().enumerate() {
            let c = ci as usize;
            if chains[c].level == 1 && g.parent[i] != NONE {
                total[c] += g.pos[i].distance(g.pos[g.parent[i] as usize]);
            }
        }
        for (i, &ci) in chain_of.iter().enumerate() {
            let c = ci as usize;
            if chains[c].level == 1 && g.parent[i] != NONE {
                along[c] += g.pos[i].distance(g.pos[g.parent[i] as usize]);
                let s = (along[c] / total[c].max(1e-6)).min(1.0);
                radius[i] = (radius[i] * (1.0 + 0.45 * (1.0 - s) * (1.0 - s))).min(r_fork * 0.8);
            }
        }
    }
    // Node lists: the attachment node first, then the chain's own nodes in order.
    let mut counts = vec![0u32; chains.len()];
    for &c in &chain_of {
        counts[c as usize] += 1;
    }
    let mut at = 0u32;
    for (c, ch) in chains.iter_mut().enumerate() {
        ch.start = at;
        ch.len = counts[c] + u32::from(attach[c] != NONE);
        at += ch.len;
    }
    let mut chain_nodes = vec![0u32; at as usize];
    let mut cursor: Vec<u32> = chains.iter().map(|c| c.start).collect();
    for (c, &a) in attach.iter().enumerate() {
        if a != NONE {
            chain_nodes[cursor[c] as usize] = a;
            cursor[c] += 1;
        }
    }
    for (i, &c) in chain_of.iter().enumerate() {
        chain_nodes[cursor[c as usize] as usize] = i as u32;
        cursor[c as usize] += 1;
    }
    let last_trunk = chain_nodes.get((chains[0].start + chains[0].len).saturating_sub(1) as usize).copied().unwrap_or(top);
    Ok(Skeleton {
        pos: g.pos,
        parent: g.parent,
        radius,
        weight,
        chain_of,
        chains,
        chain_nodes,
        fork,
        top: if decurrent { fork } else { last_trunk },
        crown_axis,
        limbs,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn broadleaf_skeleton_forks_into_its_limbs_and_fills_the_crown() {
        let spec = TreeSpec::preset(Species::Broadleaf);
        let sk = grow(&spec).unwrap();
        let fork = sk.pos[sk.fork as usize];
        assert!((fork.y / spec.height - spec.fork).abs() < 0.03, "fork at {}", fork.y);
        let limbs = sk.chains.iter().filter(|c| c.level == 1 && sk.chain_nodes[c.start as usize] == sk.fork).count();
        assert_eq!(limbs, spec.limbs as usize);
        assert!(sk.pos.len() > 300, "{} nodes", sk.pos.len());
        let hull = spec.hull(sk.crown_axis);
        let outside = sk.pos.iter().filter(|p| hull.share(**p) > 1.2 && p.y > fork.y).count();
        assert!(outside < sk.pos.len() / 50, "{outside} nodes outside the hull");
        // Every chain is connected: each node follows its predecessor's parent link.
        for c in 0..sk.chains.len() {
            let nodes = sk.chain(c);
            for w in nodes.windows(2) {
                assert_eq!(sk.parent[w[1] as usize], w[0]);
            }
        }
        // Radii shrink toward the tips.
        for (i, &p) in sk.parent.iter().enumerate() {
            if p != NONE {
                assert!(sk.radius[i] <= sk.radius[p as usize] + 1e-6);
            }
        }
    }

    #[test]
    fn growth_is_deterministic_and_seeded() {
        let spec = TreeSpec::preset(Species::Stylized);
        let a = grow(&spec).unwrap();
        let b = grow(&spec).unwrap();
        assert_eq!(a.pos, b.pos);
        let c = grow(&TreeSpec { seed: 9, ..spec }).unwrap();
        assert_ne!(a.pos, c.pos);
    }
}
