//! Pole cleanup of a quad-dominant mesh before one Catmull-Clark step.
//!
//! After subdivision every face of n corners becomes a vertex of valence n
//! and every vertex keeps its valence, so the poles of the result are the
//! irregular faces and vertices of the coarse mesh. Local edits lower the
//! irregularity energy `sum (n - 4)^2` over faces and vertices (the square
//! makes one valence 7 pole cost more than three valence 5 ones):
//!
//! - merge the two faces of an edge (two triangles into a quad),
//! - rotate an edge inside the polygon of its two faces (a triangle and a
//!   pentagon into two quads),
//! - split a hexagon or larger face with a chord,
//! - dissolve a vertex of valence 2,
//! - split a vertex of valence 6 or more into two joined by a new edge.
//!
//! Each round scores every candidate, applies the best ones that touch
//! disjoint faces and vertices, and rebuilds the mesh; rounds stop when
//! nothing improves. Boundary edges and vertices are never touched.

use glam::Vec3;
use p3d_mesh::Mesh;

/// The largest face an edit may create.
const MAX_SIDES: usize = 6;

/// The smallest corner sine of a face an edit creates.
const CONVEX: f32 = 0.05;

/// The smallest corner sine when splitting a face of seven or more corners.
const FOLDED: f32 = -0.95;

/// Irregularity of a face with `n` corners or a vertex with `n` edges.
#[inline]
fn cost(n: usize) -> i64 {
    let d = n as i64 - 4;
    d * d
}

/// One local edit as new polygons replacing old faces.
#[derive(Debug, Clone)]
struct Edit {
    gain: i64,
    /// Tie-break for equal gains: smaller is better (then by `key`).
    shape: f32,
    key: u32,
    faces: Vec<u32>,
    verts: Vec<u32>,
    /// Polygons replacing `faces`; `u32::MAX - k` names the k-th new vertex.
    polys: Vec<Vec<u32>>,
    new_verts: Vec<Vec3>,
}

/// Lowers the irregularity of a closed quad-dominant mesh in up to
/// `rounds` rounds; returns the number of edits applied.
///
/// Complexity: O(rounds x (C log C)) in corners: every edge and vertex is
/// scored once per round over faces of at most eight corners, then sorted.
pub fn reduce_poles(mesh: &mut Mesh, rounds: u32) -> usize {
    // Faces above six corners (folded walks) are poked into quads first; no
    // edit makes them again.
    let big: Vec<Edit> = mesh.faces().filter(|&f| mesh.face_len(f) > MAX_SIDES).map(|f| poke(mesh, f)).collect();
    if let Some(next) = rebuild(mesh, &big) {
        *mesh = next;
    }
    let mut applied = 0;
    for _ in 0..rounds {
        let mut taken = select(mesh, candidates(mesh));
        // An edit that breaks the rebuild (a fan torn in two) is rare; the
        // better half of the round is tried until one rebuild works.
        let next = loop {
            if taken.is_empty() {
                break None;
            }
            match rebuild(mesh, &taken) {
                Some(m) => break Some(m),
                None => taken.truncate(taken.len() / 2),
            }
        };
        let Some(next) = next else { break };
        *mesh = next;
        applied += taken.len();
    }
    applied
}

/// Polygon of the faces of `h` and its twin with the edge removed, starting
/// at `dest(h)` and ending at `origin(h)` on the face of `h`.
///
/// Complexity: O(face sizes).
fn merged(mesh: &Mesh, h: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(8);
    let mut x = mesh.next(h);
    loop {
        out.push(mesh.origin(x));
        if x == h {
            break;
        }
        x = mesh.next(x);
    }
    let t = mesh.twin(h);
    let mut x = mesh.next(t);
    // Skips the twin's first corner (dest(h) again) and stops before its
    // last (origin(h) again).
    x = mesh.next(x);
    while x != t {
        out.push(mesh.origin(x));
        x = mesh.next(x);
    }
    // `out` holds dest(h), the face of h back to origin(h), then the twin's
    // corners strictly between origin(h) and dest(h).
    out
}

/// Whether a polygon is convex around its Newell normal (every corner's
/// sine above `min_sine`; a negative bound allows reflex corners), and its
/// worst corner (1 - the smallest sine) for tie-breaks.
///
/// Complexity: O(n).
fn convex(pos: &[Vec3], poly: &[u32], up: Vec3, min_sine: f32) -> Option<f32> {
    let n = poly.len();
    let p = |i: usize| pos[poly[i % n] as usize];
    let mut normal = Vec3::ZERO;
    for i in 0..n {
        let (a, b) = (p(i), p(i + 1));
        normal += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    let normal = normal.try_normalize()?;
    if normal.dot(up) <= 0.0 {
        return None;
    }
    let mut worst = 0.0f32;
    for i in 0..n {
        let (a, b, c) = (p(i + n - 1), p(i), p(i + 1));
        let (u, v) = ((b - a).try_normalize()?, (c - b).try_normalize()?);
        let s = u.cross(v).dot(normal);
        if s <= min_sine {
            return None;
        }
        worst = worst.max(1.0 - s);
    }
    Some(worst)
}

/// Every improving edit on the current mesh, best first.
///
/// Complexity: O(C log C).
fn candidates(mesh: &Mesh) -> Vec<Edit> {
    let val: Vec<usize> = (0..mesh.vert_slots() as u32).map(|v| if mesh.is_vert_alive(v) { mesh.valence(v) } else { 0 }).collect();
    let open: Vec<bool> = (0..mesh.vert_slots() as u32).map(|v| mesh.is_vert_alive(v) && mesh.is_boundary_vert(v)).collect();
    let pos = &mesh.positions;
    let mut out = Vec::new();
    for e in mesh.edges() {
        if mesh.is_boundary_edge(e) {
            continue;
        }
        let h = 2 * e;
        let (Some(f), Some(g)) = (mesh.face(h), mesh.face(h ^ 1)) else { continue };
        let (a, b) = (mesh.origin(h), mesh.dest(h));
        if f == g || open[a as usize] || open[b as usize] {
            continue;
        }
        let (nf, ng) = (mesh.face_len(f), mesh.face_len(g));
        let poly = merged(mesh, h);
        let m = poly.len();
        // Faces sharing more than this edge would merge into a repeated corner.
        let mut sorted = poly.clone();
        sorted.sort_unstable();
        if m != nf + ng - 2 || sorted.windows(2).any(|w| w[0] == w[1]) {
            continue;
        }
        let up = mesh.face_normal_area2(f) + mesh.face_normal_area2(g);
        let before = cost(nf) + cost(ng) + cost(val[a as usize]) + cost(val[b as usize]);
        // Merge.
        if m <= MAX_SIDES
            && val[a as usize] > 3
            && val[b as usize] > 3
            && let Some(shape) = convex(pos, &poly, up, CONVEX)
        {
            let gain = cost(m) + cost(val[a as usize] - 1) + cost(val[b as usize] - 1) - before;
            if gain < 0 {
                out.push(Edit { gain, shape, key: e, faces: vec![f, g], verts: vec![a, b], polys: vec![poly.clone()], new_verts: Vec::new() });
            }
        }
        // Rotation: another chord of the merged polygon. `poly[0]` is b and
        // `poly[nf - 1]` is a.
        let mut best: Option<Edit> = None;
        for i in 0..m {
            for j in i + 2..m {
                if i == 0 && j == m - 1 {
                    continue;
                }
                let (c, d) = (poly[i], poly[j]);
                if (c == b && d == a) || (c == a && d == b) || mesh.find_halfedge(c, d).is_some() {
                    continue;
                }
                let (s1, s2) = (j - i + 1, m - (j - i) + 1);
                if s1 > MAX_SIDES || s2 > MAX_SIDES {
                    continue;
                }
                let mut verts = vec![a, b, c, d];
                verts.sort_unstable();
                verts.dedup();
                let delta = |v: u32| -> i64 { i64::from(v == c) + i64::from(v == d) - i64::from(v == a) - i64::from(v == b) };
                let after_v: i64 = verts.iter().map(|&v| cost((val[v as usize] as i64 + delta(v)) as usize)).sum();
                let before_v: i64 = verts.iter().map(|&v| cost(val[v as usize])).sum();
                if verts.iter().any(|&v| open[v as usize] || val[v as usize] as i64 + delta(v) < 3) {
                    continue;
                }
                let gain = cost(s1) + cost(s2) - cost(nf) - cost(ng) + after_v - before_v;
                if gain >= 0 || best.as_ref().is_some_and(|b| b.gain <= gain) {
                    continue;
                }
                let p1: Vec<u32> = poly[i..=j].to_vec();
                let p2: Vec<u32> = poly[j..].iter().chain(&poly[..=i]).copied().collect();
                let (Some(q1), Some(q2)) = (convex(pos, &p1, up, CONVEX), convex(pos, &p2, up, CONVEX)) else { continue };
                best = Some(Edit { gain, shape: q1.max(q2), key: e, faces: vec![f, g], verts, polys: vec![p1, p2], new_verts: Vec::new() });
            }
        }
        out.extend(best);
    }
    for f in mesh.faces() {
        let poly: Vec<u32> = mesh.face_verts(f).collect();
        let n = poly.len();
        if n < 6 || poly.iter().any(|&v| open[v as usize]) {
            continue;
        }
        // A chord across the middle splits a hexagon into two quads.
        let up = mesh.face_normal_area2(f);
        let mut best: Option<Edit> = None;
        for i in 0..n {
            for j in i + 2..n {
                if i == 0 && j == n - 1 {
                    continue;
                }
                let (c, d) = (poly[i], poly[j]);
                if mesh.find_halfedge(c, d).is_some() {
                    continue;
                }
                let (s1, s2) = (j - i + 1, n - (j - i) + 1);
                let gain = cost(s1) + cost(s2) - cost(n) + cost(val[c as usize] + 1) - cost(val[c as usize]) + cost(val[d as usize] + 1)
                    - cost(val[d as usize]);
                // Faces above six corners only ever get fewer (no edit makes
                // one), so splitting them at no gain still ends.
                let worth = gain < 0 || (n > MAX_SIDES && gain == 0);
                if !worth || best.as_ref().is_some_and(|b| b.gain <= gain) {
                    continue;
                }
                let p1: Vec<u32> = poly[i..=j].to_vec();
                let p2: Vec<u32> = poly[j..].iter().chain(&poly[..=i]).copied().collect();
                // A face of seven or more corners is worse than any split that
                // does not fold.
                let bound = if n >= 7 { FOLDED } else { CONVEX };
                let (Some(q1), Some(q2)) = (convex(pos, &p1, up, bound), convex(pos, &p2, up, bound)) else { continue };
                best = Some(Edit { gain, shape: q1.max(q2), key: f, faces: vec![f], verts: vec![c, d], polys: vec![p1, p2], new_verts: Vec::new() });
            }
        }
        out.extend(best);
    }
    for v in mesh.verts() {
        if open[v as usize] {
            continue;
        }
        let d = val[v as usize];
        if d == 2 {
            out.extend(dissolve(mesh, v, &val));
        } else if d >= 6 {
            out.extend(split(mesh, v, &val));
        }
    }
    out.sort_by(|x, y| x.gain.cmp(&y.gain).then(x.shape.total_cmp(&y.shape)).then(x.faces.cmp(&y.faces)).then(x.key.cmp(&y.key)));
    out
}

/// A new center vertex joined to every second corner of face `f`: quads,
/// and one triangle when the face has an odd number of corners. The
/// corners joined are the ones whose valence grows best.
///
/// Complexity: O(n^2) for a face of n corners (n is at most eight).
fn poke(mesh: &Mesh, f: u32) -> Edit {
    let poly: Vec<u32> = mesh.face_verts(f).collect();
    let n = poly.len();
    let m = n.div_ceil(2);
    let raise = |v: u32| cost(mesh.valence(v) + 1) - cost(mesh.valence(v));
    let start = (0..n).min_by_key(|&s| ((0..m).map(|i| raise(poly[(s + 2 * i) % n])).sum::<i64>(), s)).unwrap_or(0);
    let c = u32::MAX;
    let polys = (0..m)
        .map(|i| {
            let a = (start + 2 * i) % n;
            let b = if i + 1 == m { start } else { (start + 2 * (i + 1)) % n };
            let steps = (b + n - a) % n;
            let mut p: Vec<u32> = (0..=steps).map(|k| poly[(a + k) % n]).collect();
            p.push(c);
            p
        })
        .collect();
    let center = poly.iter().map(|&v| mesh.positions[v as usize]).sum::<Vec3>() / n as f32;
    Edit { gain: 0, shape: 0.0, key: f, faces: vec![f], verts: poly, polys, new_verts: vec![center] }
}

/// Removes a vertex of valence 2 from both its faces.
///
/// Complexity: O(face sizes).
fn dissolve(mesh: &Mesh, v: u32, val: &[usize]) -> Option<Edit> {
    let h = mesh.vert_he(v);
    let (f, g) = (mesh.face(h)?, mesh.face(h ^ 1)?);
    let (nf, ng) = (mesh.face_len(f), mesh.face_len(g));
    let [u, w] = {
        let mut it = mesh.vert_neighbors(v);
        [it.next()?, it.next()?]
    };
    if f == g {
        return None;
    }
    let without = |x: u32| mesh.face_verts(x).filter(|&y| y != v).collect::<Vec<u32>>();
    let lose = |x: u32| cost(val[x as usize] - 1) - cost(val[x as usize]);
    let mut options: Vec<Edit> = Vec::with_capacity(2);
    if nf >= 4 && ng >= 4 && mesh.find_halfedge(u, w).is_none() {
        // Both faces lose the corner; u and w are joined directly.
        let gain = cost(nf - 1) + cost(ng - 1) - cost(nf) - cost(ng) - cost(val[v as usize]);
        options.push(Edit {
            gain,
            shape: 0.0,
            key: v,
            faces: vec![f, g],
            verts: vec![v, u, w],
            polys: vec![without(f), without(g)],
            new_verts: Vec::new(),
        });
    }
    if (nf == 3) != (ng == 3) && val[u as usize] >= 3 && val[w as usize] >= 3 {
        // One face is the triangle (u, v, w): it goes, and the other face
        // takes its edge u-w, so u and w lose an edge each.
        let (tri, other, n_other) = if nf == 3 { (f, g, ng) } else { (g, f, nf) };
        let across = mesh.face_halfedges(tri).find(|&x| mesh.origin(x) != v && mesh.dest(x) != v).and_then(|x| mesh.face(mesh.twin(x)));
        if across.is_some_and(|a| a != other) {
            let gain = cost(n_other - 1) - cost(n_other) - cost(3) - cost(val[v as usize]) + lose(u) + lose(w);
            options.push(Edit {
                gain,
                shape: 0.0,
                key: v,
                faces: vec![tri, other],
                verts: vec![v, u, w],
                polys: vec![without(other)],
                new_verts: Vec::new(),
            });
        }
    }
    if nf + ng >= 7 && val[u as usize] >= 3 && val[w as usize] >= 3 {
        // Both faces merge, dropping the path u-v-w: f runs a1 .. a0 around
        // v, and g holds a1 -> v -> a0 the other way round.
        let rot = |x: u32| {
            let mut l: Vec<u32> = mesh.face_verts(x).collect();
            let k = l.iter().position(|&y| y == v).unwrap_or(0);
            l.rotate_left(k);
            l
        };
        let (fl, gl) = (rot(f), rot(g));
        let poly: Vec<u32> = fl[1..].iter().chain(&gl[2..gl.len() - 1]).copied().collect();
        let mut sorted = poly.clone();
        sorted.sort_unstable();
        let up = mesh.face_normal_area2(f) + mesh.face_normal_area2(g);
        if poly.len() <= MAX_SIDES && !sorted.windows(2).any(|w| w[0] == w[1]) && convex(&mesh.positions, &poly, up, FOLDED).is_some() {
            let gain = cost(poly.len()) - cost(nf) - cost(ng) - cost(val[v as usize]) + lose(u) + lose(w);
            options.push(Edit { gain, shape: 0.0, key: v, faces: vec![f, g], verts: vec![v, u, w], polys: vec![poly], new_verts: Vec::new() });
        }
    }
    // Or a chord from v across one of its faces gives it a third edge.
    for (x, nx) in [(f, nf), (g, ng)] {
        let poly: Vec<u32> = mesh.face_verts(x).collect();
        let Some(k) = poly.iter().position(|&y| y == v) else { continue };
        let up = mesh.face_normal_area2(x);
        for step in 2..nx.saturating_sub(1) {
            let c = poly[(k + step) % nx];
            if mesh.find_halfedge(v, c).is_some() {
                continue;
            }
            let p1: Vec<u32> = (0..=step).map(|i| poly[(k + i) % nx]).collect();
            let p2: Vec<u32> = (step..=nx).map(|i| poly[(k + i) % nx]).collect();
            let gain = cost(p1.len()) + cost(p2.len()) - cost(nx) + cost(3) - cost(2) + cost(val[c as usize] + 1) - cost(val[c as usize]);
            // Not a removal: only a strict gain counts.
            if gain < 0 && convex(&mesh.positions, &p1, up, FOLDED).is_some() && convex(&mesh.positions, &p2, up, FOLDED).is_some() {
                options.push(Edit { gain, shape: 0.0, key: v, faces: vec![x], verts: vec![v, c], polys: vec![p1, p2], new_verts: Vec::new() });
            }
        }
    }
    // Removing a vertex at no gain still ends: the vertex count falls, and
    // a fold left behind is taken up the next round.
    options.into_iter().filter(|e| e.gain <= 0).min_by_key(|e| e.gain)
}

/// Splits a vertex of valence 6 or more into two joined by a new edge: the
/// best contiguous split of its edges, with the two faces where the arcs
/// meet gaining a corner.
///
/// Complexity: O(d^2) for valence d, plus the faces around the vertex.
fn split(mesh: &Mesh, v: u32, val: &[usize]) -> Option<Edit> {
    let hs: Vec<u32> = mesh.vert_outgoing(v).collect();
    let d = hs.len();
    let faces: Vec<u32> = hs.iter().map(|&h| mesh.face(h)).collect::<Option<Vec<u32>>>()?;
    let mut sorted = faces.clone();
    sorted.sort_unstable();
    if sorted.windows(2).any(|w| w[0] == w[1]) {
        return None;
    }
    let size: Vec<usize> = faces.iter().map(|&f| mesh.face_len(f)).collect();
    // Arc one is hs[i..i + k] (cyclic); the face of hs[i - 1] and the face
    // of hs[i + k - 1] sit between the arcs.
    let mut best: Option<(i64, usize, usize)> = None;
    for k in 2..=d - 2 {
        for i in 0..d {
            let (fa, fb) = ((i + d - 1) % d, (i + k - 1) % d);
            if size[fa] + 1 > MAX_SIDES || size[fb] + 1 > MAX_SIDES {
                continue;
            }
            let gain =
                cost(k + 1) + cost(d - k + 1) - cost(val[v as usize]) + cost(size[fa] + 1) - cost(size[fa]) + cost(size[fb] + 1) - cost(size[fb]);
            if gain < 0 && best.is_none_or(|b| gain < b.0) {
                best = Some((gain, i, k));
            }
        }
    }
    let (gain, i, k) = best?;
    let in_one = |j: usize| (j + d - i) % d < k;
    let at = mesh.positions[v as usize];
    let mean = |one: bool| {
        let (s, n) =
            (0..d).filter(|&j| in_one(j) == one).fold((Vec3::ZERO, 0.0f32), |(s, n), j| (s + mesh.positions[mesh.dest(hs[j]) as usize], n + 1.0));
        at + (s / n - at) * 0.25
    };
    let (v1, v2) = (u32::MAX, u32::MAX - 1);
    let (fa, fb) = ((i + d - 1) % d, (i + k - 1) % d);
    let mut polys = Vec::with_capacity(d);
    for (j, &f) in faces.iter().enumerate() {
        let mut poly = Vec::with_capacity(size[j] + 1);
        for x in mesh.face_verts(f) {
            if x != v {
                poly.push(x);
            } else if j == fa {
                // Entered from arc one (hs[i]), left along arc two (hs[i - 1]).
                poly.extend([v1, v2]);
            } else if j == fb {
                poly.extend([v2, v1]);
            } else {
                // The face of hs[j] lies between hs[j] and hs[j + 1].
                poly.push(if in_one(j) { v1 } else { v2 });
            }
        }
        polys.push(poly);
    }
    let mut verts: Vec<u32> = hs.iter().map(|&h| mesh.dest(h)).collect();
    verts.push(v);
    Some(Edit { gain, shape: 0.0, key: v, faces, verts, polys, new_verts: vec![mean(true), mean(false)] })
}

/// The edits, in order, that touch disjoint faces and vertices.
///
/// Complexity: O(the corners of the edits' faces).
fn select(mesh: &Mesh, edits: Vec<Edit>) -> Vec<Edit> {
    let mut face_used = vec![false; mesh.face_slots()];
    let mut vert_used = vec![false; mesh.vert_slots()];
    let mut taken: Vec<Edit> = Vec::new();
    for e in edits {
        // The vertices of every replaced face are touched too, so two edits
        // never edit one face or create one edge twice.
        let mut touch: Vec<u32> = e.verts.clone();
        for &f in &e.faces {
            touch.extend(mesh.face_verts(f));
        }
        if e.faces.iter().any(|&f| face_used[f as usize]) || touch.iter().any(|&v| vert_used[v as usize]) {
            continue;
        }
        for &f in &e.faces {
            face_used[f as usize] = true;
        }
        for v in touch {
            vert_used[v as usize] = true;
        }
        taken.push(e);
    }
    taken
}

/// The mesh with `taken` (disjoint edits) applied; `None` when the result
/// is not a manifold mesh.
///
/// Complexity: O(V + C).
fn rebuild(mesh: &Mesh, taken: &[Edit]) -> Option<Mesh> {
    let mut replaced: Vec<Option<usize>> = vec![None; mesh.face_slots()];
    for (k, e) in taken.iter().enumerate() {
        for &f in &e.faces {
            replaced[f as usize] = Some(k);
        }
    }
    let mut pos = mesh.positions.clone();
    let mut polys: Vec<Vec<u32>> = Vec::with_capacity(mesh.face_count() + taken.len());
    let mut emitted = vec![false; taken.len()];
    for f in mesh.faces() {
        match replaced[f as usize] {
            None => polys.push(mesh.face_verts(f).collect()),
            Some(k) if !emitted[k] => {
                emitted[k] = true;
                let base = pos.len() as u32;
                pos.extend(&taken[k].new_verts);
                for p in &taken[k].polys {
                    polys.push(p.iter().map(|&x| if x >= u32::MAX - 1 { base + (u32::MAX - x) } else { x }).collect());
                }
            }
            Some(_) => {}
        }
    }
    // Compact the vertices: dissolved ones and dead slots go.
    let mut map = vec![u32::MAX; pos.len()];
    let mut out_pos = Vec::with_capacity(pos.len());
    let mut sizes = Vec::with_capacity(polys.len());
    let mut corners = Vec::with_capacity(polys.iter().map(Vec::len).sum());
    for p in &polys {
        sizes.push(p.len() as u32);
        for &x in p {
            if map[x as usize] == u32::MAX {
                map[x as usize] = out_pos.len() as u32;
                out_pos.push(pos[x as usize]);
            }
            corners.push(map[x as usize]);
        }
    }
    Mesh::from_polygons(out_pos, &sizes, &corners).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// A closed box of `n x n` quads per side.
    fn cube(n: usize) -> Mesh {
        let spec = p3d_geo::prim::BoxSpec { size: glam::DVec3::ONE, segments: [n as u32; 3], ..Default::default() };
        p3d_geo::prim::cuboid(&spec, p3d_geo::prim::Origin::Center).unwrap()
    }

    fn energy(m: &Mesh) -> i64 {
        m.faces().map(|f| cost(m.face_len(f))).sum::<i64>() + m.verts().map(|v| cost(m.valence(v))).sum::<i64>()
    }

    #[test]
    fn two_triangles_of_a_split_quad_merge_back() {
        let mut m = cube(3);
        // Split one face of the cube with a diagonal.
        let f = m.faces().next().unwrap();
        p3d_geo::edit::triangulate(&mut m, &[f]).unwrap();
        let before = energy(&m);
        let n = reduce_poles(&mut m, 4);
        assert!(n >= 1);
        assert!(energy(&m) < before);
        assert_eq!(m.validate(), Ok(()));
        assert!(m.faces().all(|f| m.face_len(f) == 4));
    }

    #[test]
    fn big_faces_are_poked_into_quads() {
        for n in [7u32, 8] {
            // A prism of n sides: two n-gon caps and n quads.
            let mut pos = Vec::new();
            for z in [0.0f32, 1.0] {
                for i in 0..n {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    pos.push(Vec3::new(a.cos(), a.sin(), z));
                }
            }
            let mut sizes = vec![n, n];
            let mut corners: Vec<u32> = (0..n).rev().collect();
            corners.extend(n..2 * n);
            for i in 0..n {
                let j = (i + 1) % n;
                sizes.push(4);
                corners.extend([i, j, n + j, n + i]);
            }
            let mut m = Mesh::from_polygons(pos, &sizes, &corners).unwrap();
            reduce_poles(&mut m, 0);
            assert_eq!(m.validate(), Ok(()));
            assert!(m.faces().all(|f| m.face_len(f) <= 4));
            let tris = m.faces().filter(|&f| m.face_len(f) == 3).count();
            assert_eq!(tris, if n % 2 == 1 { 2 } else { 0 });
        }
    }

    #[test]
    fn regular_meshes_are_left_alone() {
        let mut m = cube(4);
        let faces = m.face_count();
        assert_eq!(reduce_poles(&mut m, 4), 0);
        assert_eq!(m.face_count(), faces);
    }
}
