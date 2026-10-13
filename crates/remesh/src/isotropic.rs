//! Isotropic triangle remeshing (Botsch and Kobbelt 2004): edges longer
//! than 4/3 of the target are split, shorter than 4/5 collapsed, edges
//! flipped toward valence 6, vertices relaxed in their tangent planes and
//! projected back onto the input. Boundaries and feature edges (dihedral
//! above `feature_deg`) stay where they are. Every phase processes edges in
//! a fixed order and locks what it touched, so results are deterministic.

use glam::{Affine3A, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use rustc_hash::FxHashMap;

use crate::quad::RemeshError;

/// Settings of an isotropic remesh.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsotropicSpec {
    /// Target edge length.
    pub edge: f32,
    /// Rounds of split, collapse, flip, relax and project.
    pub iterations: u32,
    /// Edges bent more than this (degrees) are features and kept.
    pub feature_deg: f32,
}

impl Default for IsotropicSpec {
    fn default() -> Self {
        IsotropicSpec { edge: 0.05, iterations: 6, feature_deg: 40.0 }
    }
}

/// Most triangles a remesh may make.
pub const MAX_TRIANGLES: usize = 4_000_000;

/// A triangle soup with live flags and per-vertex feature marks.
struct Tris {
    pos: Vec<Vec3>,
    feature: Vec<bool>,
    tris: Vec<[u32; 3]>,
    alive: Vec<bool>,
}

type EdgeMap = FxHashMap<(u32, u32), [u32; 2]>;

const NONE: u32 = u32::MAX;

fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

impl Tris {
    /// Edge -> up to two live triangles.
    ///
    /// Complexity: O(T).
    fn edges(&self) -> EdgeMap {
        let mut m: EdgeMap = FxHashMap::default();
        for (t, tri) in self.tris.iter().enumerate().filter(|(t, _)| self.alive[*t]) {
            for k in 0..3 {
                let e = m.entry(key(tri[k], tri[(k + 1) % 3])).or_insert([NONE, NONE]);
                if e[0] == NONE {
                    e[0] = t as u32;
                } else {
                    e[1] = t as u32;
                }
            }
        }
        m
    }

    /// Vertex -> live triangles.
    ///
    /// Complexity: O(V + T).
    fn ring(&self) -> Vec<Vec<u32>> {
        let mut r = vec![Vec::new(); self.pos.len()];
        for (t, tri) in self.tris.iter().enumerate().filter(|(t, _)| self.alive[*t]) {
            for &v in tri {
                r[v as usize].push(t as u32);
            }
        }
        r
    }

    fn normal(&self, t: [u32; 3]) -> Vec3 {
        let [a, b, c] = t.map(|v| self.pos[v as usize]);
        (b - a).cross(c - a)
    }

    fn len(&self, a: u32, b: u32) -> f32 {
        self.pos[a as usize].distance(self.pos[b as usize])
    }
}

/// The vertex of `t` that is not `a` or `b`.
fn opposite(t: [u32; 3], a: u32, b: u32) -> u32 {
    t.into_iter().find(|&v| v != a && v != b).unwrap_or(NONE)
}

/// Splits edges longer than `hi`, longest first, each triangle at most once
/// per pass; passes repeat while anything splits.
///
/// Complexity: O(T log T) per pass.
fn split(m: &mut Tris, hi: f32) -> Result<(), RemeshError> {
    for _ in 0..16 {
        let edges = m.edges();
        let mut long: Vec<((u32, u32), f32)> = edges.keys().map(|&(a, b)| ((a, b), m.len(a, b))).filter(|&(_, l)| l > hi).collect();
        if long.is_empty() {
            return Ok(());
        }
        long.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        let mut used = vec![false; m.tris.len()];
        for ((a, b), _) in long {
            let ts = edges[&(a, b)];
            if ts.iter().any(|&t| t != NONE && used[t as usize]) {
                continue;
            }
            let mid = m.pos.len() as u32;
            m.pos.push((m.pos[a as usize] + m.pos[b as usize]) * 0.5);
            m.feature.push(m.feature[a as usize] && m.feature[b as usize]);
            for &t in ts.iter().filter(|&&t| t != NONE) {
                used[t as usize] = true;
                let tri = m.tris[t as usize];
                // Keep the winding: replace b by mid, and add (mid, b, c).
                let k = (0..3).find(|&k| (tri[k] == a && tri[(k + 1) % 3] == b) || (tri[k] == b && tri[(k + 1) % 3] == a)).unwrap_or(0);
                let (p, q, r) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
                m.tris[t as usize] = [p, mid, r];
                m.tris.push([mid, q, r]);
                m.alive.push(true);
                used.push(true);
            }
            if m.tris.len() > MAX_TRIANGLES {
                return Err(RemeshError::TooLarge(m.tris.len()));
            }
        }
    }
    Ok(())
}

/// Collapses edges shorter than `lo` into their midpoint (or onto a
/// boundary or feature end) when the result keeps the mesh manifold, makes
/// no edge longer than `hi` and flips no triangle.
///
/// Complexity: O(T log T + collapses x ring sizes).
fn collapse(m: &mut Tris, lo: f32, hi: f32, boundary: &[bool]) {
    let edges = m.edges();
    let mut short: Vec<((u32, u32), f32)> = edges.keys().map(|&(a, b)| ((a, b), m.len(a, b))).filter(|&(_, l)| l < lo).collect();
    short.sort_by(|x, y| x.1.total_cmp(&y.1).then(x.0.cmp(&y.0)));
    let mut ring = m.ring();
    let mut locked = vec![false; m.pos.len()];
    for ((a, b), _) in short {
        if locked[a as usize] || locked[b as usize] {
            continue;
        }
        let ts = edges[&(a, b)];
        if ts[1] == NONE {
            continue; // Boundary edges stay.
        }
        let fixed = |v: u32| boundary[v as usize] || m.feature[v as usize];
        let (fa, fb) = (fixed(a), fixed(b));
        if fa && fb {
            continue;
        }
        let (keep, gone) = if fb { (b, a) } else { (a, b) };
        let target = if fa || fb { m.pos[keep as usize] } else { (m.pos[a as usize] + m.pos[b as usize]) * 0.5 };
        // Link condition: a and b share exactly the two opposite vertices.
        let nbrs = |v: u32| {
            let mut n: Vec<u32> = ring[v as usize].iter().flat_map(|&t| m.tris[t as usize]).filter(|&w| w != v).collect();
            n.sort_unstable();
            n.dedup();
            n
        };
        let (na, nb) = (nbrs(a), nbrs(b));
        let common = na.iter().filter(|v| nb.binary_search(v).is_ok()).count();
        if common != 2 {
            continue;
        }
        // No long edges and no flipped triangles afterwards.
        let after: Vec<u32> = ring[a as usize].iter().chain(&ring[b as usize]).copied().filter(|&t| !ts.contains(&t)).collect();
        let mut ok = true;
        for &t in &after {
            let old = m.tris[t as usize];
            let new = old.map(|v| if v == a || v == b { NONE } else { v });
            let n0 = m.normal(old);
            let p = new.map(|v| if v == NONE { target } else { m.pos[v as usize] });
            let n1 = (p[1] - p[0]).cross(p[2] - p[0]);
            if n1.length_squared() < 1e-20 || n0.dot(n1) <= 0.2 * n0.length() * n1.length() {
                ok = false;
                break;
            }
            if p.iter().any(|q| q.distance(target) > hi) {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        m.pos[keep as usize] = target;
        for &t in &ts {
            m.alive[t as usize] = false;
        }
        for &t in &ring[gone as usize] {
            if m.alive[t as usize] {
                m.tris[t as usize] = m.tris[t as usize].map(|v| if v == gone { keep } else { v });
            }
        }
        // The kept vertex's ring is the union; lock the neighbourhood.
        let moved: Vec<u32> = ring[gone as usize].iter().copied().filter(|&t| m.alive[t as usize]).collect();
        ring[keep as usize].retain(|&t| m.alive[t as usize]);
        ring[keep as usize].extend(moved);
        for v in na.iter().chain(&nb).chain([&a, &b]) {
            locked[*v as usize] = true;
        }
    }
}

/// Flips interior edges when that brings the four vertices closer to
/// valence 6 (4 on boundaries) without folding a triangle.
///
/// Complexity: O(T log T).
fn flip(m: &mut Tris, boundary: &[bool]) {
    let edges = m.edges();
    let mut val = vec![0i32; m.pos.len()];
    for &(a, b) in edges.keys() {
        val[a as usize] += 1;
        val[b as usize] += 1;
    }
    let target = |v: u32| if boundary[v as usize] { 4 } else { 6 };
    let mut keys: Vec<(u32, u32)> = edges.keys().copied().collect();
    keys.sort_unstable();
    let mut locked = vec![false; m.tris.len()];
    for (a, b) in keys {
        let [t0, t1] = edges[&(a, b)];
        if t1 == NONE || locked[t0 as usize] || locked[t1 as usize] {
            continue;
        }
        if m.feature[a as usize] && m.feature[b as usize] {
            continue;
        }
        let (c, d) = (opposite(m.tris[t0 as usize], a, b), opposite(m.tris[t1 as usize], a, b));
        if c == NONE || d == NONE || c == d || edges.contains_key(&key(c, d)) {
            continue;
        }
        let dev = |v: u32, dv: i32| (val[v as usize] + dv - target(v)).abs();
        let before = dev(a, 0) + dev(b, 0) + dev(c, 0) + dev(d, 0);
        let after = dev(a, -1) + dev(b, -1) + dev(c, 1) + dev(d, 1);
        if after >= before {
            continue;
        }
        // The triangle holding a -> b keeps its winding: (a, b, c) becomes
        // (c, d, b)... found from t0's order.
        let tri = m.tris[t0 as usize];
        let k = (0..3).find(|&k| tri[k] == a).unwrap_or(0);
        let (p, q) = if tri[(k + 1) % 3] == b { (a, b) } else { (b, a) };
        // t0 = (p, q, c), t1 = (q, p, d).
        let (n0, n1) = (m.normal(tri), m.normal(m.tris[t1 as usize]));
        let (x, y) = ([c, p, d], [d, q, c]);
        let (nx, ny) = (m.normal(x), m.normal(y));
        let up = n0 + n1;
        if nx.dot(up) <= 0.0 || ny.dot(up) <= 0.0 || nx.length_squared() < 1e-20 || ny.length_squared() < 1e-20 {
            continue;
        }
        m.tris[t0 as usize] = x;
        m.tris[t1 as usize] = y;
        locked[t0 as usize] = true;
        locked[t1 as usize] = true;
        val[a as usize] -= 1;
        val[b as usize] -= 1;
        val[c as usize] += 1;
        val[d as usize] += 1;
    }
}

/// Moves free vertices toward the centroid of their neighbours within
/// their tangent planes, then onto the input surface.
///
/// Complexity: O(T + V log n).
fn relax(m: &mut Tris, boundary: &[bool], bvh: &TriBvh, reach: f32) {
    let n = m.pos.len();
    let mut sum = vec![Vec3::ZERO; n];
    let mut cnt = vec![0u32; n];
    let mut nrm = vec![Vec3::ZERO; n];
    for (t, tri) in m.tris.iter().enumerate() {
        if !m.alive[t] {
            continue;
        }
        let fnrm = m.normal(*tri);
        for k in 0..3 {
            let (v, w) = (tri[k] as usize, tri[(k + 1) % 3] as usize);
            sum[v] += m.pos[w];
            cnt[v] += 1;
            nrm[v] += fnrm;
        }
    }
    let next: Vec<Vec3> = (0..n)
        .map(|v| {
            let p = m.pos[v];
            if cnt[v] == 0 || boundary[v] || m.feature[v] {
                return p;
            }
            let c = sum[v] / cnt[v] as f32;
            let nv = nrm[v].normalize_or(Vec3::Y);
            let d = c - p;
            let q = p + d - nv * nv.dot(d);
            match bvh.closest(q, reach) {
                Some(h) if bvh.normal(h.tri).dot(nv) > 0.0 => h.point,
                _ => q,
            }
        })
        .collect();
    m.pos = next;
}

/// Remeshes a mesh into even triangles about `spec.edge` long.
///
/// Complexity: O(iterations x T log T) for T output triangles, plus
/// O(V log n) per relaxation for the projection.
pub fn isotropic(mesh: &Mesh, spec: &IsotropicSpec) -> Result<Mesh, RemeshError> {
    if !(spec.edge.is_finite() && spec.edge > 0.0) {
        return Err(RemeshError::BadEdge);
    }
    let mut map = vec![u32::MAX; mesh.vert_slots()];
    let mut pos = Vec::with_capacity(mesh.vert_count());
    for v in mesh.verts() {
        map[v as usize] = pos.len() as u32;
        pos.push(mesh.positions[v as usize]);
    }
    let tris: Vec<[u32; 3]> = mesh.faces().flat_map(|f| mesh.face_triangles(f)).map(|t| t.map(|h| map[mesh.origin(h) as usize])).collect();
    if tris.is_empty() {
        return Err(RemeshError::Empty);
    }
    // Features from the input: vertices on edges bent beyond the angle.
    let cos = spec.feature_deg.to_radians().cos();
    let mut feature = vec![false; pos.len()];
    for e in mesh.edges() {
        if let [Some(f), Some(g)] = mesh.edge_faces(e)
            && mesh.face_normal(f).dot(mesh.face_normal(g)) < cos
        {
            for v in mesh.edge_verts(e) {
                feature[map[v as usize] as usize] = true;
            }
        }
    }
    let bvh = TriBvh::from_mesh(mesh, Affine3A::IDENTITY);
    let reach = mesh.bounds().map_or(1.0, |(lo, hi)| (hi - lo).length());
    let n = tris.len();
    let mut m = Tris { pos, feature, tris, alive: vec![true; n] };
    let (lo, hi) = (spec.edge * 0.8, spec.edge * 4.0 / 3.0);
    for _ in 0..spec.iterations.max(1) {
        split(&mut m, hi)?;
        // Boundary vertices: on an edge with one triangle.
        let boundary = |m: &Tris| {
            let mut b = vec![false; m.pos.len()];
            for (&(x, y), ts) in &m.edges() {
                if ts[1] == NONE {
                    b[x as usize] = true;
                    b[y as usize] = true;
                }
            }
            b
        };
        let bd = boundary(&m);
        collapse(&mut m, lo, hi, &bd);
        let bd = boundary(&m);
        flip(&mut m, &bd);
        relax(&mut m, &bd, &bvh, reach);
    }
    // Compact and build.
    let mut keep = vec![u32::MAX; m.pos.len()];
    let mut out_pos = Vec::new();
    let mut corners = Vec::new();
    for (t, tri) in m.tris.iter().enumerate() {
        if !m.alive[t] {
            continue;
        }
        for &v in tri {
            if keep[v as usize] == u32::MAX {
                keep[v as usize] = out_pos.len() as u32;
                out_pos.push(m.pos[v as usize]);
            }
            corners.push(keep[v as usize]);
        }
    }
    let sizes = vec![3u32; corners.len() / 3];
    Mesh::from_polygons(out_pos, &sizes, &corners).map_err(|e| RemeshError::Build(e.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use p3d_geo::prim::{BoxSpec, Origin, TorusSpec, cuboid, torus};

    use super::*;

    fn edge_stats(m: &Mesh) -> (f32, f32) {
        let ls: Vec<f32> = m
            .edges()
            .map(|e| {
                let [a, b] = m.edge_verts(e);
                m.positions[a as usize].distance(m.positions[b as usize])
            })
            .collect();
        let mean = ls.iter().sum::<f32>() / ls.len() as f32;
        let var = ls.iter().map(|l| (l - mean) * (l - mean)).sum::<f32>() / ls.len() as f32;
        (mean, var.sqrt() / mean)
    }

    #[test]
    fn a_torus_gets_even_edges_and_stays_closed() {
        let t = torus(&TorusSpec { major_radius: 0.4, minor_radius: 0.15, segments: 24, sides: 12 }, Origin::Center).unwrap();
        let m = isotropic(&t, &IsotropicSpec { edge: 0.03, ..IsotropicSpec::default() }).unwrap();
        assert_eq!(m.validate(), Ok(()));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed");
        let (mean, cv) = edge_stats(&m);
        assert!((mean - 0.03).abs() < 0.006, "{mean}");
        assert!(cv < 0.25, "edges vary {cv}");
        // Valence near 6.
        let six = m.verts().filter(|&v| (5..=7).contains(&m.valence(v))).count();
        assert!(six * 10 > m.vert_count() * 9, "{six} of {}", m.vert_count());
    }

    #[test]
    fn box_corners_and_edges_stay_sharp() {
        let b = cuboid(&BoxSpec { size: glam::DVec3::splat(0.4), ..BoxSpec::default() }, Origin::Center).unwrap();
        let m = isotropic(&b, &IsotropicSpec { edge: 0.05, ..IsotropicSpec::default() }).unwrap();
        assert_eq!(m.validate(), Ok(()));
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - lo - Vec3::splat(0.4)).abs().max_element() < 1e-4, "{lo} {hi}");
        assert!(m.verts().all(|v| m.positions[v as usize].abs().max_element() > 0.2 - 1e-4), "on the box's faces");
        assert!(m.face_count() > 100);
    }

    #[test]
    fn deterministic() {
        let t = torus(&TorusSpec { major_radius: 0.4, minor_radius: 0.15, segments: 16, sides: 8 }, Origin::Center).unwrap();
        let a = isotropic(&t, &IsotropicSpec { edge: 0.05, ..IsotropicSpec::default() }).unwrap();
        let b = isotropic(&t, &IsotropicSpec { edge: 0.05, ..IsotropicSpec::default() }).unwrap();
        assert_eq!(a.positions, b.positions);
    }
}
