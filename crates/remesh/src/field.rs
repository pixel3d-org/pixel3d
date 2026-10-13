//! The vertex graph of a dense surface sampling, its multi-resolution
//! hierarchy, and the two fields of field-aligned remeshing (Instant
//! Meshes, Jakob et al. 2015): a 4-RoSy orientation field (one tangent
//! direction per vertex, equal up to quarter turns) and a position field (a
//! lattice point per vertex, equal up to whole lattice steps), both smoothed
//! coarse to fine with extrinsic Gauss-Seidel sweeps in index order, so the
//! result is deterministic.

use glam::Vec3;
use rayon::prelude::*;

/// One resolution of the vertex graph.
#[derive(Debug, Clone, Default)]
pub struct Level {
    pub x: Vec<Vec3>,
    pub n: Vec<Vec3>,
    pub area: Vec<f32>,
    /// Neighbours of vertex i: `adj[adj_start[i]..adj_start[i + 1]]`.
    pub adj_start: Vec<u32>,
    pub adj: Vec<u32>,
    /// The coarser vertex each vertex merged into (empty on the coarsest level).
    pub parent: Vec<u32>,
    /// Vertices grouped so that no two neighbours share a group: each group
    /// updates in parallel, groups in order.
    pub colors: Vec<Vec<u32>>,
}

impl Level {
    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    #[inline]
    pub fn neighbors(&self, i: usize) -> &[u32] {
        &self.adj[self.adj_start[i] as usize..self.adj_start[i + 1] as usize]
    }

    /// Greedy graph coloring in vertex order: each vertex takes the smallest
    /// color none of its neighbours has yet.
    ///
    /// Complexity: O(V + E).
    fn color(&self) -> Vec<Vec<u32>> {
        let mut color = vec![u32::MAX; self.len()];
        let mut classes: Vec<Vec<u32>> = Vec::new();
        let mut taken: Vec<u32> = Vec::new();
        for i in 0..self.len() {
            taken.clear();
            taken.extend(self.neighbors(i).iter().map(|&j| color[j as usize]).filter(|&c| c != u32::MAX));
            taken.sort_unstable();
            taken.dedup();
            let c = taken.iter().enumerate().find(|(k, c)| **c != *k as u32).map_or(taken.len(), |(k, _)| k);
            color[i] = c as u32;
            if c == classes.len() {
                classes.push(Vec::new());
            }
            classes[c].push(i as u32);
        }
        classes
    }

    /// Builds the adjacency from undirected edges (each listed once).
    ///
    /// Complexity: O(V + E log E).
    pub fn with_edges(x: Vec<Vec3>, n: Vec<Vec3>, area: Vec<f32>, mut edges: Vec<(u32, u32)>) -> Level {
        let nv = x.len();
        edges.retain(|&(a, b)| a != b);
        let mut both: Vec<(u32, u32)> = edges.iter().flat_map(|&(a, b)| [(a, b), (b, a)]).collect();
        both.sort_unstable();
        both.dedup();
        let mut adj_start = vec![0u32; nv + 1];
        for &(a, _) in &both {
            adj_start[a as usize + 1] += 1;
        }
        for i in 0..nv {
            adj_start[i + 1] += adj_start[i];
        }
        let adj = both.into_iter().map(|(_, b)| b).collect();
        let mut l = Level { x, n, area, adj_start, adj, parent: Vec::new(), colors: Vec::new() };
        l.colors = l.color();
        l
    }

    /// The next coarser level: neighbours whose normals agree and whose
    /// areas are alike merge in pairs, best pairs first.
    ///
    /// Complexity: O(E log E).
    pub fn coarsen(&mut self) -> Level {
        let nv = self.len();
        let mut cands: Vec<(f32, u32, u32)> = Vec::new();
        for i in 0..nv {
            for &j in self.neighbors(i) {
                if (i as u32) < j {
                    let (a, b) = (self.area[i], self.area[j as usize]);
                    let ratio = a.min(b) / a.max(b).max(1e-20);
                    cands.push((self.n[i].dot(self.n[j as usize]) * ratio, i as u32, j));
                }
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let mut parent = vec![u32::MAX; nv];
        let mut x = Vec::new();
        let mut n = Vec::new();
        let mut area = Vec::new();
        for (_, i, j) in cands {
            let (i, j) = (i as usize, j as usize);
            if parent[i] != u32::MAX || parent[j] != u32::MAX {
                continue;
            }
            let (ai, aj) = (self.area[i], self.area[j]);
            let a = (ai + aj).max(1e-20);
            parent[i] = x.len() as u32;
            parent[j] = x.len() as u32;
            x.push((self.x[i] * ai + self.x[j] * aj) / a);
            n.push((self.n[i] * ai + self.n[j] * aj).normalize_or(self.n[i]));
            area.push(ai + aj);
        }
        for (i, p) in parent.iter_mut().enumerate().take(nv) {
            if *p == u32::MAX {
                *p = x.len() as u32;
                x.push(self.x[i]);
                n.push(self.n[i]);
                area.push(self.area[i]);
            }
        }
        let mut edges = Vec::with_capacity(self.adj.len() / 2);
        for i in 0..nv {
            for &j in self.neighbors(i) {
                let (a, b) = (parent[i], parent[j as usize]);
                if a < b {
                    edges.push((a, b));
                }
            }
        }
        self.parent = parent;
        Level::with_edges(x, n, area, edges)
    }
}

/// Levels from the finest (index 0) to the coarsest, until a level has at
/// most `min_vertices` vertices or stops shrinking.
///
/// Complexity: O(E log E) for the first level, less for each next one (a
/// geometric series).
pub fn hierarchy(finest: Level, min_vertices: usize) -> Vec<Level> {
    let mut levels = vec![finest];
    loop {
        let last = levels.len() - 1;
        let len = levels[last].len();
        if len <= min_vertices || levels.len() >= 32 {
            break;
        }
        let coarse = levels[last].coarsen();
        if coarse.len() as f32 > len as f32 * 0.9 {
            levels[last].parent.clear();
            break;
        }
        levels.push(coarse);
    }
    levels
}

/// The pair among the quarter turns of `q0` (about `n0`) and `q1` (about
/// `n1`) that agree best, as (rotation of q0, rotation of q1).
#[inline]
pub fn compat_orientation(q0: Vec3, n0: Vec3, q1: Vec3, n1: Vec3) -> (Vec3, Vec3) {
    let a = [q0, n0.cross(q0)];
    let b = [q1, n1.cross(q1)];
    let mut best = (0usize, 0usize, f32::NEG_INFINITY);
    for (i, ai) in a.iter().enumerate() {
        for (j, bj) in b.iter().enumerate() {
            let s = ai.dot(*bj).abs();
            if s > best.2 {
                best = (i, j, s);
            }
        }
    }
    let s = a[best.0].dot(b[best.1]).signum();
    (a[best.0], b[best.1] * if s == 0.0 { 1.0 } else { s })
}

/// A point on both tangent planes, about halfway between `p0` and `p1`.
#[inline]
fn middle_point(p0: Vec3, n0: Vec3, p1: Vec3, n1: Vec3) -> Vec3 {
    let (n0p0, n0p1, n1p0, n1p1, n0n1) = (n0.dot(p0), n0.dot(p1), n1.dot(p0), n1.dot(p1), n0.dot(n1));
    let denom = 1.0 / (1.0 - n0n1 * n0n1 + 1e-4);
    let l0 = 2.0 * (n0p1 - n0p0 - n0n1 * (n1p0 - n1p1)) * denom;
    let l1 = 2.0 * (n1p0 - n1p1 - n0n1 * (n0p1 - n0p0)) * denom;
    0.5 * (p0 + p1) - 0.25 * (n0 * l0 + n1 * l1)
}

/// The lattice points of vertex 0 (lattice through `o0` with axes `q0`,
/// `n0 x q0`) and vertex 1 that lie closest to each other near the middle
/// of the two vertices, with their integer lattice offsets from `o0` and
/// `o1`. `q1` must already agree with `q0` (see [`compat_orientation`]).
#[inline]
pub fn compat_position(
    (p0, n0, q0, o0): (Vec3, Vec3, Vec3, Vec3),
    (p1, n1, q1, o1): (Vec3, Vec3, Vec3, Vec3),
    scale: f32,
) -> (Vec3, Vec3, [i32; 2], [i32; 2]) {
    let inv = 1.0 / scale;
    let (t0, t1) = (n0.cross(q0), n1.cross(q1));
    let m = middle_point(p0, n0, p1, n1);
    let floor0 = [(q0.dot(m - o0) * inv).floor() as i32, (t0.dot(m - o0) * inv).floor() as i32];
    let floor1 = [(q1.dot(m - o1) * inv).floor() as i32, (t1.dot(m - o1) * inv).floor() as i32];
    let at0 = |k: [i32; 2]| o0 + (q0 * k[0] as f32 + t0 * k[1] as f32) * scale;
    let at1 = |k: [i32; 2]| o1 + (q1 * k[0] as f32 + t1 * k[1] as f32) * scale;
    let mut best = (f32::INFINITY, [0i32; 2], [0i32; 2]);
    for c0 in 0..4 {
        let k0 = [floor0[0] + (c0 & 1), floor0[1] + (c0 >> 1)];
        let a = at0(k0);
        for c1 in 0..4 {
            let k1 = [floor1[0] + (c1 & 1), floor1[1] + (c1 >> 1)];
            let d = (a - at1(k1)).length_squared();
            if d < best.0 {
                best = (d, k0, k1);
            }
        }
    }
    (at0(best.1), at1(best.2), best.1, best.2)
}

/// The lattice point (axes `q`, `n x q`, spacing `scale`, through `o`)
/// nearest `p`.
#[inline]
pub fn round_position(o: Vec3, q: Vec3, n: Vec3, p: Vec3, scale: f32) -> Vec3 {
    let t = n.cross(q);
    let d = p - o;
    o + q * (q.dot(d) / scale).round() * scale + t * (t.dot(d) / scale).round() * scale
}

/// SplitMix64 for seeded starting values.
fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A tangent direction at `n` turned by a seeded angle.
fn random_tangent(n: Vec3, state: &mut u64) -> Vec3 {
    let a = n.any_orthonormal_vector();
    let b = n.cross(a);
    let ang = (split_mix(state) >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
    a * ang.cos() + b * ang.sin()
}

/// The orientation field on every level: random at the coarsest level, then
/// `iterations` sweeps per level, each level started from its parents.
///
/// Complexity: O(E x iterations) per level, so O(E x iterations) overall.
pub fn orientation_field(levels: &[Level], iterations: u32, seed: u64) -> Vec<Vec<Vec3>> {
    let mut state = seed ^ 0x5EE_D0FF_1E1D;
    let mut out: Vec<Vec<Vec3>> = levels.iter().map(|l| vec![Vec3::ZERO; l.len()]).collect();
    let top = levels.len() - 1;
    for (i, q) in out[top].iter_mut().enumerate() {
        *q = random_tangent(levels[top].n[i], &mut state);
    }
    for li in (0..levels.len()).rev() {
        if li < top {
            let (fine, coarse) = out.split_at_mut(li + 1);
            for (i, q) in fine[li].iter_mut().enumerate() {
                let n = levels[li].n[i];
                let pq = coarse[0][levels[li].parent[i] as usize];
                let v = pq - n * n.dot(pq);
                *q = v.try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            }
        }
        let l = &levels[li];
        let q = &mut out[li];
        for _ in 0..iterations {
            for class in &l.colors {
                let fresh: Vec<Vec3> = class
                    .par_iter()
                    .map(|&i| {
                        let i = i as usize;
                        let ni = l.n[i];
                        let mut sum = q[i];
                        let mut weight = 0.0f32;
                        for &j in l.neighbors(i) {
                            let (a, b) = compat_orientation(sum, ni, q[j as usize], l.n[j as usize]);
                            sum = a * weight + b;
                            weight += 1.0;
                            sum -= ni * ni.dot(sum);
                            sum = sum.try_normalize().unwrap_or(a);
                        }
                        sum
                    })
                    .collect();
                for (&i, v) in class.iter().zip(fresh) {
                    if !l.neighbors(i as usize).is_empty() {
                        q[i as usize] = v;
                    }
                }
            }
        }
    }
    out
}

/// The position field on every level: lattice points of spacing `scale`,
/// starting at the vertices on the coarsest level, then `iterations` sweeps
/// per level. Each vertex keeps the lattice point nearest itself.
///
/// Complexity: O(E x iterations) per level.
pub fn position_field(levels: &[Level], q: &[Vec<Vec3>], scale: f32, iterations: u32) -> Vec<Vec<Vec3>> {
    let mut out: Vec<Vec<Vec3>> = levels.iter().map(|l| l.x.clone()).collect();
    let top = levels.len() - 1;
    for li in (0..levels.len()).rev() {
        let l = &levels[li];
        if li < top {
            let (fine, coarse) = out.split_at_mut(li + 1);
            for (i, o) in fine[li].iter_mut().enumerate() {
                let (x, n) = (l.x[i], l.n[i]);
                let po = coarse[0][l.parent[i] as usize];
                let p = po - n * n.dot(po - x);
                *o = round_position(p, q[li][i], n, x, scale);
            }
        }
        let o = &mut out[li];
        for _ in 0..iterations {
            for class in &l.colors {
                // A square lattice is the same after a quarter turn, so the
                // neighbour's own direction serves.
                let fresh: Vec<Vec3> = class
                    .par_iter()
                    .map(|&i| {
                        let i = i as usize;
                        let (xi, ni, qi) = (l.x[i], l.n[i], q[li][i]);
                        let mut sum = o[i];
                        let mut weight = 0.0f32;
                        for &j in l.neighbors(i) {
                            let j = j as usize;
                            let (a, b, _, _) = compat_position((xi, ni, qi, sum), (l.x[j], l.n[j], q[li][j], o[j]), scale);
                            sum = (a * weight + b) / (weight + 1.0);
                            weight += 1.0;
                            sum -= ni * ni.dot(sum - xi);
                        }
                        if weight > 0.0 { round_position(sum, qi, ni, xi, scale) } else { o[i] }
                    })
                    .collect();
                for (&i, v) in class.iter().zip(fresh) {
                    o[i as usize] = v;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn compatible_orientations_pick_the_closest_quarter_turn() {
        let n = Vec3::Z;
        let (a, b) = compat_orientation(Vec3::X, n, Vec3::Y, n);
        assert!(a.dot(b) > 0.99, "{a} {b}");
        let (a, b) = compat_orientation(Vec3::X, n, -Vec3::X, n);
        assert!(a.dot(b) > 0.99);
    }

    #[test]
    fn positions_round_to_the_lattice() {
        let p = round_position(Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(0.26, 0.74, 0.0), 0.5);
        assert!((p - Vec3::new(0.5, 0.5, 0.0)).length() < 1e-6);
        // Two vertices of one flat lattice agree on their offset.
        let (a, b, k0, k1) =
            compat_position((Vec3::ZERO, Vec3::Z, Vec3::X, Vec3::ZERO), (Vec3::new(1.0, 0.0, 0.0), Vec3::Z, Vec3::X, Vec3::new(1.0, 0.0, 0.0)), 1.0);
        assert!((a - b).length() < 1e-6);
        assert_eq!([k0[0] - k1[0], k0[1] - k1[1]], [1, 0]);
    }
}
