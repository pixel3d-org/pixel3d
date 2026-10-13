//! Edge bevel: replaces edges with strips of faces (a chamfer, or a rounded
//! profile), with caps where strips end or meet.
//!
//! Around each vertex of a beveled edge, every face corner moves to a new
//! position: where the offset lines of two beveled edges meet inside the
//! face, or slid along the face's other edge until it reaches the offset
//! line. Strips run between the moved corners of their two faces, with
//! profile points in between; where strips meet or end, a cap closes the
//! hole around the old vertex. Everything is one patch.

use glam::Vec3;
use p3d_mesh::{CornerSrc, Mesh, Patch};
use rustc_hash::{FxHashMap, FxHashSet};

use super::merge::simple_polygons;
use super::{KernelError, patch};

/// Bevel parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BevelSpec {
    /// Distance from each beveled edge to the new edges, measured inside the
    /// faces.
    pub width: f32,
    /// Faces across the bevel: 1 is a flat chamfer.
    pub segments: u32,
    /// Profile shape for 2 or more segments: 0.5 is round, toward 0 flat,
    /// toward 1 pointed.
    pub profile: f32,
    /// Shrink the width where it would overlap neighbouring geometry instead
    /// of failing.
    pub clamp: bool,
}

impl Default for BevelSpec {
    fn default() -> Self {
        BevelSpec { width: 0.005, segments: 1, profile: 0.5, clamp: true }
    }
}

/// What a bevel did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BevelResult {
    /// Faces of the strips and caps.
    pub faces: Vec<u32>,
    pub edges: usize,
    /// The width used (smaller than asked when clamped).
    pub width: f32,
    /// Selected boundary edges that cannot be beveled.
    pub skipped: usize,
}

/// Largest number of bevel segments.
pub const MAX_BEVEL_SEGMENTS: u32 = 64;

/// Faces that differ by less than this (degrees) count as one flat region.
const FLAT_DEG: f32 = 0.5;

/// Flat runs around a vertex: two beveled edges (ring indices `a` and `b`,
/// following the rotation) with only plain edges between them and every
/// sector between them in one plane, such as a quad split by a diagonal or
/// a face split by a boolean. Their corners all move to the one point
/// where the two offset lines meet, as if the plain edges were not there.
/// Each run also returns the turn from `a` to `b` (under 180 degrees).
///
/// Complexity: O(valence).
fn flat_runs(mesh: &Mesh, v: u32, ring: &[u32], is_bev: &impl Fn(u32) -> bool) -> Vec<(usize, usize, f32)> {
    let n = ring.len();
    let pos = |x: u32| mesh.positions[x as usize];
    let cos = FLAT_DEG.to_radians().cos();
    let mut out = Vec::new();
    for a in (0..n).filter(|&a| is_bev(ring[a])) {
        let Some(f0) = mesh.face(ring[a]) else { continue };
        let n0 = mesh.face_normal(f0);
        let mut turn = 0.0;
        let mut k = a;
        let end = loop {
            let (hp, hq) = (ring[k], ring[(k + 1) % n]);
            match mesh.face(hp) {
                Some(f) if mesh.face_normal(f).dot(n0) >= cos => {}
                _ => break None,
            }
            turn += dir(pos(v), pos(mesh.dest(hp))).angle_between(dir(pos(v), pos(mesh.dest(hq))));
            k = (k + 1) % n;
            if k == a {
                break None;
            }
            if is_bev(ring[k]) {
                break Some(k);
            }
        };
        let Some(b) = end else { continue };
        // One sector is the ordinary case; a turn of 180 degrees or more has no inside corner.
        if (b + n - a) % n >= 2 && turn < std::f32::consts::PI - 1e-2 {
            out.push((a, b, turn));
        }
    }
    out
}

/// Unit direction from `a` to `b` (zero when they coincide).
fn dir(a: Vec3, b: Vec3) -> Vec3 {
    (b - a).normalize_or_zero()
}

/// Bevels edges. Boundary edges are skipped. With `clamp`, a width that
/// would make moved corners pass each other is reduced to fit (the result
/// reports the width used); without it such a width fails.
///
/// Complexity: O(c log c + valences) for the c corners of the faces around
/// the beveled edges.
pub fn bevel(mesh: &mut Mesh, edges: &[u32], spec: &BevelSpec) -> Result<BevelResult, KernelError> {
    if !(spec.width.is_finite() && spec.width > 0.0) {
        return Err(KernelError::OutOfRange {
            param: "width",
            message: "bevel width must be more than 0".into(),
            hint: "3 to 10 mm reads well on 0.5 to 1 m props".into(),
        });
    }
    if !(1..=MAX_BEVEL_SEGMENTS).contains(&spec.segments) {
        return Err(KernelError::OutOfRange {
            param: "segments",
            message: format!("{} segments is outside 1 to {MAX_BEVEL_SEGMENTS}", spec.segments),
            hint: "1 for small props, 2 to 3 for hero assets".into(),
        });
    }
    if !(spec.profile.is_finite() && (0.0..=1.0).contains(&spec.profile)) {
        return Err(KernelError::OutOfRange {
            param: "profile",
            message: format!("profile {} is outside 0 to 1", spec.profile),
            hint: "0.5 is round".into(),
        });
    }
    let mut out = BevelResult::default();
    let mut bev: FxHashSet<u32> = FxHashSet::default();
    for &e in edges {
        if !mesh.is_edge_alive(e) {
            continue;
        }
        match mesh.edge_faces(e) {
            [Some(a), Some(b)] if a != b => {
                bev.insert(e);
            }
            _ => out.skipped += 1,
        }
    }
    if bev.is_empty() {
        return Err(KernelError::Empty("no interior edges selected (boundary edges cannot be beveled)".into()));
    }
    let mut bev_edges: Vec<u32> = bev.iter().copied().collect();
    bev_edges.sort_unstable();
    let mut verts: Vec<u32> = bev_edges.iter().flat_map(|&e| mesh.edge_verts(e)).collect();
    verts.sort_unstable();
    verts.dedup();
    let pos = |v: u32| mesh.positions[v as usize];
    let is_bev = |h: u32| bev.contains(&(h >> 1));

    // ---- width limits ------------------------------------------------------
    // Every moved corner slides along some edge by `width x factor`; the
    // slides from both ends of an edge must not meet.
    let mut along: FxHashMap<(u32, u32), f32> = FxHashMap::default(); // (edge, from vertex) -> factor
    for &v in &verts {
        let ring: Vec<u32> = mesh.vert_outgoing(v).collect();
        let n = ring.len();
        let mut in_run = vec![false; n];
        for (a, b, turn) in flat_runs(mesh, v, &ring, &is_bev) {
            let f = 1.0 / (turn / 2.0).tan();
            for h in [ring[a], ring[b]] {
                let k = along.entry((h >> 1, v)).or_insert(0.0);
                *k = k.max(f);
            }
            let mut k = a;
            while k != b {
                in_run[k] = true;
                k = (k + 1) % n;
            }
        }
        for i in 0..n {
            if mesh.face(ring[i]).is_none() || in_run[i] {
                continue;
            }
            let (hp, hq) = (ring[i], ring[(i + 1) % n]);
            let (up, uq) = (dir(pos(v), pos(mesh.dest(hp))), dir(pos(v), pos(mesh.dest(hq))));
            let theta = up.angle_between(uq).clamp(1e-3, std::f32::consts::PI - 1e-3);
            match (is_bev(hp), is_bev(hq)) {
                (true, true) => {
                    // The corner moves along both edges by cot(theta / 2).
                    let f = 1.0 / (theta / 2.0).tan();
                    for h in [hp, hq] {
                        let k = along.entry((h >> 1, v)).or_insert(0.0);
                        *k = k.max(f);
                    }
                }
                (true, false) | (false, true) => {
                    let (b, s) = if is_bev(hp) { (hp, hq) } else { (hq, hp) };
                    // Slides 1 / sin(theta) along the plain edge, cot(theta) along the beveled one.
                    let k = along.entry((s >> 1, v)).or_insert(0.0);
                    *k = k.max(1.0 / theta.sin());
                    let k = along.entry((b >> 1, v)).or_insert(0.0);
                    *k = k.max(1.0 / theta.tan()).max(0.0);
                }
                (false, false) => {}
            }
        }
    }
    let mut w_max = f32::INFINITY;
    let mut per_edge: FxHashMap<u32, f32> = FxHashMap::default();
    for (&(e, _), &k) in &along {
        *per_edge.entry(e).or_insert(0.0) += k;
    }
    for (&e, &k) in &per_edge {
        if k > 0.0 {
            w_max = w_max.min(mesh.edge_length(e) / k);
        }
    }
    let mut width = spec.width;
    if width >= w_max * 0.999 {
        if spec.clamp {
            width = w_max * 0.98;
        } else {
            return Err(KernelError::OutOfRange {
                param: "width",
                message: format!("a {} m bevel overlaps neighbouring edges", spec.width),
                hint: format!("use a width below {} m, or set clamp_overlap", w_max),
            });
        }
    }
    out.width = width;

    let mut p = patch(mesh);
    // ---- moved corners -----------------------------------------------------
    // New vertex of each face corner (keyed by the corner's half-edge) at a
    // bevel vertex; slide points keyed by (edge, end vertex).
    let mut corner_new: FxHashMap<u32, u32> = FxHashMap::default();
    let mut slide_req: FxHashMap<(u32, u32), Vec<Vec3>> = FxHashMap::default();
    let mut slide_of_corner: Vec<(u32, (u32, u32))> = Vec::new();
    // Plain edges inside flat runs and the run corner their ends move to.
    let mut transparent: FxHashSet<u32> = FxHashSet::default();
    let mut run_slide: Vec<((u32, u32), u32)> = Vec::new();
    for &v in &verts {
        let ring: Vec<u32> = mesh.vert_outgoing(v).collect();
        let n = ring.len();
        let mut in_run = vec![false; n];
        for (a, b, turn) in flat_runs(mesh, v, &ring, &is_bev) {
            let (ua, ub) = (dir(pos(v), pos(mesh.dest(ring[a]))), dir(pos(v), pos(mesh.dest(ring[b]))));
            let c = p.new_vert(pos(v) + (ua + ub).normalize_or_zero() * (width / (turn / 2.0).sin()), Some(v));
            let mut k = a;
            while k != b {
                in_run[k] = true;
                corner_new.insert(ring[k], c);
                if k != a {
                    transparent.insert(ring[k]);
                    run_slide.push(((ring[k] >> 1, v), c));
                }
                k = (k + 1) % n;
            }
        }
        for i in 0..n {
            let (hp, hq) = (ring[i], ring[(i + 1) % n]);
            if mesh.face(hp).is_none() || in_run[i] {
                continue;
            }
            let (up, uq) = (dir(pos(v), pos(mesh.dest(hp))), dir(pos(v), pos(mesh.dest(hq))));
            let theta = up.angle_between(uq).clamp(1e-3, std::f32::consts::PI - 1e-3);
            match (is_bev(hp), is_bev(hq)) {
                (true, true) => {
                    let bis = (up + uq).normalize_or_zero();
                    let c = p.new_vert(pos(v) + bis * (width / (theta / 2.0).sin()), Some(v));
                    corner_new.insert(hp, c);
                }
                (true, false) => {
                    slide_req.entry((hq >> 1, v)).or_default().push(pos(v) + uq * (width / theta.sin()));
                    slide_of_corner.push((hp, (hq >> 1, v)));
                }
                (false, true) => {
                    slide_req.entry((hp >> 1, v)).or_default().push(pos(v) + up * (width / theta.sin()));
                    slide_of_corner.push((hp, (hp >> 1, v)));
                }
                (false, false) => {}
            }
        }
    }
    let mut keys: Vec<(u32, u32)> = slide_req.keys().copied().collect();
    keys.sort_unstable();
    let mut slide: FxHashMap<(u32, u32), u32> = FxHashMap::default();
    for k in keys {
        let pts = &slide_req[&k];
        let mean = pts.iter().sum::<Vec3>() / pts.len() as f32;
        slide.insert(k, p.new_vert(mean, Some(k.1)));
    }
    for (h, k) in slide_of_corner {
        corner_new.insert(h, slide[&k]);
    }
    slide.extend(run_slide);
    let moved = |h: u32| corner_new.get(&h).copied().unwrap_or_else(|| mesh.origin(h));

    // ---- profile chains, shared by strips that continue through a vertex ----
    let s = spec.segments as usize;
    let mut chains: FxHashMap<(u32, u32, u32), Vec<u32>> = FxHashMap::default();
    // The profile of beveled edge `e` at its end `v` bends toward the point of
    // the edge level with the chain's ends (the old vertex in the middle of
    // an edge loop, a point down the edge where strips end at a corner).
    let mut chain = |p: &mut Patch, v: u32, e: u32, a: u32, b: u32| -> Vec<u32> {
        let (lo, hi) = (a.min(b), a.max(b));
        let mids = chains
            .entry((v, lo, hi))
            .or_insert_with(|| {
                let at = |x: u32| p.new_vert_position(x).unwrap_or_else(|| mesh.positions[x as usize]);
                let (l, r) = (at(lo), at(hi));
                let m = (l + r) * 0.5;
                let [e0, e1] = mesh.edge_verts(e);
                let d = dir(pos(e0), pos(e1));
                let c = pos(v) + d * d.dot(m - pos(v));
                let k = m + (c - m) * (2.0 * spec.profile);
                (1..s)
                    .map(|j| {
                        let t = j as f32 / s as f32;
                        let q = l * (1.0 - t) * (1.0 - t) + k * (2.0 * t * (1.0 - t)) + r * (t * t);
                        // On flat ground a round profile passes through the old
                        // vertex: reuse it instead of stacking a copy on it.
                        if q.distance(pos(v)) <= 1e-6 * width { v } else { p.new_vert(q, Some(v)) }
                    })
                    .collect()
            })
            .clone();
        let mut out = vec![lo];
        out.extend(mids);
        out.push(hi);
        if a != lo {
            out.reverse();
        }
        out
    };

    // ---- faces around the bevel vertices -------------------------------------
    let mut faces: Vec<u32> = verts.iter().flat_map(|&v| mesh.vert_faces(v).collect::<Vec<_>>()).collect();
    faces.sort_unstable();
    faces.dedup();
    for &f in &faces {
        let mut poly: Vec<(u32, CornerSrc)> = Vec::new();
        for h in mesh.face_halfedges(f) {
            let (u, u2) = (mesh.origin(h), mesh.dest(h));
            let c = moved(h);
            poly.push((c, CornerSrc::copy(h)));
            if !is_bev(h) {
                let e = h >> 1;
                // Slide points on this edge that are not this face's own corners.
                if let Some(&x) = slide.get(&(e, u))
                    && x != c
                {
                    poly.push((x, CornerSrc::lerp(h, mesh.next(h), 0.1)));
                }
                if let Some(&x) = slide.get(&(e, u2))
                    && x != moved(mesh.next(h))
                {
                    poly.push((x, CornerSrc::lerp(h, mesh.next(h), 0.9)));
                }
            }
        }
        p.remove_face(f);
        let mut ring: Vec<(u32, CornerSrc)> = Vec::with_capacity(poly.len());
        for c in poly {
            if ring.last().is_none_or(|l: &(u32, CornerSrc)| l.0 != c.0) {
                ring.push(c);
            }
        }
        while ring.len() > 1 && ring[0].0 == ring[ring.len() - 1].0 {
            ring.pop();
        }
        if ring.len() >= 3 {
            let vs: Vec<u32> = ring.iter().map(|c| c.0).collect();
            let src: Vec<CornerSrc> = ring.iter().map(|c| c.1).collect();
            p.add_face(&vs, &src, mesh.face_material(f), Some(f));
        }
    }

    // ---- strips ----------------------------------------------------------------
    let mut made: Vec<usize> = Vec::new();
    for &e in &bev_edges {
        let h = 2 * e;
        let (v, w) = (mesh.origin(h), mesh.dest(h));
        let (Some(f1), Some(_)) = (mesh.face(h), mesh.face(h ^ 1)) else { continue };
        // Side 1 runs along face f1 (holding h = v -> w), side 2 along the other face.
        let (c1v, c2v) = (moved(h), moved(mesh.next(h ^ 1)));
        let (c1w, c2w) = (moved(mesh.next(h)), moved(h ^ 1));
        let cv = chain(&mut p, v, e, c1v, c2v);
        let cw = chain(&mut p, w, e, c1w, c2w);
        for k in 0..s {
            let quad = [cw[k], cv[k], cv[k + 1], cw[k + 1]];
            let poly: Vec<u32> = (0..4).filter(|&i| quad[i] != quad[(i + 3) % 4]).map(|i| quad[i]).collect();
            if poly.len() >= 3 {
                let t0 = k as f32 / s as f32;
                let t1 = (k + 1) as f32 / s as f32;
                let (a, b) = (h, mesh.next(h ^ 1));
                let src = [
                    CornerSrc::lerp(mesh.next(h), h ^ 1, t0),
                    CornerSrc::lerp(a, b, t0),
                    CornerSrc::lerp(a, b, t1),
                    CornerSrc::lerp(mesh.next(h), h ^ 1, t1),
                ];
                made.push(p.add_face(&poly, &src, mesh.face_material(f1), None));
            }
        }
        out.edges += 1;
    }

    // ---- caps around each bevel vertex ---------------------------------------------
    for &v in &verts {
        let ring: Vec<u32> = mesh.vert_outgoing(v).collect();
        let n = ring.len();
        // Walk around v: beveled edges give their profile chain, plain edges
        // their slide point (or v), sectors their moved corner (v if none).
        let mut walk: Vec<u32> = Vec::new();
        let sector = |i: usize| -> u32 { if mesh.face(ring[i]).is_some() { moved(ring[i]) } else { v } };
        // At the open border the hole around v is closed only when a face
        // still uses v (a sector between two plain edges).
        let open = ring.iter().any(|&h| mesh.face(h).is_none());
        let v_used = ring.iter().enumerate().any(|(i, &h)| mesh.face(h).is_some() && !is_bev(h) && !is_bev(ring[(i + 1) % n]));
        if open && !v_used {
            continue;
        }
        let like = mesh.vert_faces(v).next().map_or(0, |f| mesh.face_material(f));
        // Every edge beveled and an even profile: a quad grid per corner sector.
        // Plain edges inside flat runs do not count: their faces share one corner.
        let bevels: Vec<usize> = (0..n).filter(|&i| is_bev(ring[i])).collect();
        if !open && s >= 2 && s.is_multiple_of(2) && bevels.len() >= 3 && ring.iter().all(|&h| is_bev(h) || transparent.contains(&h)) {
            let chains: Vec<Vec<u32>> = bevels.iter().map(|&i| chain(&mut p, v, ring[i] >> 1, sector((i + n - 1) % n), sector(i))).collect();
            made.extend(corner_grid(&mut p, &chains, pos(v), spec.profile, like, Some(v)));
            continue;
        }
        let mut beveled = 0usize;
        for (i, &h) in ring.iter().enumerate() {
            if is_bev(h) {
                beveled += 1;
                let before = sector((i + n - 1) % n);
                let after = sector(i);
                walk.extend(chain(&mut p, v, h >> 1, before, after));
            } else {
                walk.push(slide.get(&(h >> 1, v)).copied().unwrap_or(v));
            }
            walk.push(sector(i));
        }
        // The rotation turns counter-clockwise seen from outside, which is
        // the cap's winding (strips run each chain the other way).
        let corners: Vec<(u32, ())> = walk.into_iter().map(|x| (x, ())).collect();
        for piece in simple_polygons(&corners) {
            let vs: Vec<u32> = piece.iter().map(|c| c.0).collect();
            if s >= 2 && beveled >= 3 && vs.len() > 4 {
                // A rounded corner: fan around a center pulled toward the old vertex.
                let at = |x: u32| p.new_vert_position(x).unwrap_or_else(|| mesh.positions[x as usize]);
                let mean = vs.iter().map(|&x| at(x)).sum::<Vec3>() / vs.len() as f32;
                let center = p.new_vert(mean + (pos(v) - mean) * spec.profile * 0.6, Some(v));
                for i in 0..vs.len() {
                    made.push(p.add_face(&[vs[i], vs[(i + 1) % vs.len()], center], &[], like, None));
                }
            } else {
                made.push(p.add_face(&vs, &[], like, None));
            }
        }
    }
    let r = mesh.apply_patch(&p)?;
    out.faces = made.into_iter().map(|i| r.faces[i]).collect();
    Ok(out)
}

/// Fills the corner where `chains.len()` strips meet with quads: chain i runs
/// (in the cap's winding) from moved corner i - 1 to moved corner i with an
/// even number s of segments. A center point sits between the chains'
/// middles, pulled toward the old vertex `v` by the profile; spokes join
/// each chain middle to it, and each moved corner gets an (s/2 x s/2) grid
/// bounded by half of each neighbouring chain and two spokes (a Coons
/// patch). Returns the patch indices of the quads.
///
/// Complexity: O(k x s^2) for k chains of s segments.
fn corner_grid(p: &mut Patch, chains: &[Vec<u32>], v: Vec3, profile: f32, material: u16, like_vert: Option<u32>) -> Vec<usize> {
    let k = chains.len();
    let s = chains[0].len() - 1;
    let h = s / 2;
    let at = |p: &Patch, x: u32| p.new_vert_position(x).unwrap_or(v);
    let mids: Vec<u32> = chains.iter().map(|c| c[h]).collect();
    let mean = mids.iter().map(|&x| at(p, x)).sum::<Vec3>() / k as f32;
    // 0.31 puts the center of a round (0.5) square corner on its sphere:
    // the chain middles average w / 2 from the corner, the sphere point
    // w (1 - 1 / sqrt 3).
    let center_pos = mean + (v - mean) * profile * 0.31;
    let center = p.new_vert(center_pos, like_vert);
    // Spoke j: from the middle of chain j (t = 0) to the center (t = h).
    let spokes: Vec<Vec<u32>> = mids
        .iter()
        .map(|&m| {
            let a = at(p, m);
            let mut sp = vec![m];
            for t in 1..h {
                sp.push(p.new_vert(a.lerp(center_pos, t as f32 / h as f32), like_vert));
            }
            sp.push(center);
            sp
        })
        .collect();
    let mut made = Vec::new();
    for i in 0..k {
        let j = (i + 1) % k;
        // Boundary curves of the patch around moved corner i (= chain i's end).
        let bottom: Vec<u32> = chains[i][h..=s].to_vec(); // mid_i -> corner
        let right: Vec<u32> = chains[j][0..=h].to_vec(); // corner -> mid_j
        let left: Vec<u32> = spokes[i].clone(); // mid_i -> center
        let top: Vec<u32> = spokes[j].iter().rev().copied().collect(); // center -> mid_j
        let mut grid = vec![vec![0u32; h + 1]; h + 1];
        for a in 0..=h {
            grid[a][0] = bottom[a];
            grid[a][h] = top[a];
        }
        grid[0].copy_from_slice(&left);
        grid[h].copy_from_slice(&right);
        for a in 1..h {
            for b in 1..h {
                let (u, w) = (a as f32 / h as f32, b as f32 / h as f32);
                let (pb, pt, pl, pr) = (at(p, bottom[a]), at(p, top[a]), at(p, left[b]), at(p, right[b]));
                let (c00, c10, c01, c11) = (at(p, grid[0][0]), at(p, grid[h][0]), at(p, grid[0][h]), at(p, grid[h][h]));
                let coons = pb * (1.0 - w) + pt * w + pl * (1.0 - u) + pr * u
                    - (c00 * (1.0 - u) * (1.0 - w) + c10 * u * (1.0 - w) + c01 * (1.0 - u) * w + c11 * u * w);
                grid[a][b] = p.new_vert(coons, like_vert);
            }
        }
        for a in 0..h {
            for b in 0..h {
                made.push(p.add_face(&[grid[a][b], grid[a + 1][b], grid[a + 1][b + 1], grid[a][b + 1]], &[], material, None));
            }
        }
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    fn euler(m: &Mesh) -> i64 {
        m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
    }

    fn unit_box() -> Mesh {
        cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap()
    }

    #[test]
    fn chamfer_every_box_edge() {
        let mut m = unit_box();
        let all: Vec<u32> = m.edges().collect();
        let r = bevel(&mut m, &all, &BevelSpec { width: 0.1, segments: 1, ..BevelSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.face_count(), euler(&m)), (24, 26, 2), "the chamfered box");
        assert_eq!((r.edges, r.faces.len()), (12, 20));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // Flat faces shrank by the width on each side.
        let top = m.group("top").unwrap().iter().next().unwrap();
        assert!((m.face_area(top) - 0.64).abs() < 1e-5);
    }

    #[test]
    fn round_bevel_on_all_box_edges() {
        let mut m = unit_box();
        let all: Vec<u32> = m.edges().collect();
        bevel(&mut m, &all, &BevelSpec { width: 0.1, segments: 3, ..BevelSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!(euler(&m), 2);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - lo).abs_diff_eq(Vec3::ONE, 1e-5), "same size: {}", hi - lo);
    }

    #[test]
    fn even_round_bevels_are_all_quads() {
        for s in [2, 4, 6] {
            let mut m = unit_box();
            let all: Vec<u32> = m.edges().collect();
            let r = bevel(&mut m, &all, &BevelSpec { width: 0.1, segments: s, ..BevelSpec::default() }).unwrap();
            m.validate().unwrap();
            assert!(m.faces().all(|f| m.face_len(f) == 4), "{s} segments: quads only");
            // 12 strips of s quads, 8 corners of 3 (s/2)^2 grids.
            let h = (s / 2) as usize;
            assert_eq!(r.faces.len(), 12 * s as usize + 8 * 3 * h * h);
            assert!(m.verts().all(|v| m.valence(v) <= 4), "no poles above valence 4");
        }
    }

    #[test]
    fn round_corners_follow_the_rounded_box() {
        // A round bevel of width w on a cube is close to the cube shrunk by w
        // and grown by a ball of radius w: no dents where three strips meet
        // (the profiles at a corner bend toward the edge, not the old corner).
        for s in [2, 4] {
            let mut m = unit_box();
            let all: Vec<u32> = m.edges().collect();
            let w = 0.1;
            bevel(&mut m, &all, &BevelSpec { width: w, segments: s, ..BevelSpec::default() }).unwrap();
            let (lo, hi) = m.bounds().unwrap();
            let inner_lo = lo + Vec3::splat(w);
            let inner_hi = hi - Vec3::splat(w);
            for v in m.verts() {
                let p = m.positions[v as usize];
                let d = p.distance(p.clamp(inner_lo, inner_hi));
                assert!((d - w).abs() < 0.1 * w, "{s} segments: {p:?} is {d} from the inner box");
            }
        }
    }

    #[test]
    fn faces_split_by_plain_edges_bevel_like_whole_ones() {
        // Diagonals in the faces (as triangulation or a boolean leaves) must
        // not change the result: every corner still gets a quad grid, and
        // the shape is the one the whole box gives.
        let volume = |m: &Mesh| -> f32 {
            m.faces()
                .flat_map(|f| m.face_triangles(f))
                .map(|t| {
                    let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize]);
                    a.dot(b.cross(c)) / 6.0
                })
                .sum()
        };
        let mut whole = unit_box();
        let all: Vec<u32> = whole.edges().collect();
        bevel(&mut whole, &all, &BevelSpec { width: 0.1, segments: 2, ..BevelSpec::default() }).unwrap();
        let mut split = unit_box();
        let faces: Vec<u32> = split.faces().collect();
        crate::edit::triangulate(&mut split, &faces[..3]).unwrap();
        let sharp: Vec<u32> = split
            .edges()
            .filter(|&e| match split.edge_faces(e) {
                [Some(a), Some(b)] => split.face_normal(a).dot(split.face_normal(b)) < 0.5,
                _ => false,
            })
            .collect();
        assert_eq!(sharp.len(), 12);
        let r = bevel(&mut split, &sharp, &BevelSpec { width: 0.1, segments: 2, ..BevelSpec::default() }).unwrap();
        split.validate().unwrap();
        assert!(r.faces.iter().all(|&f| split.face_len(f) == 4), "bevel faces are quads");
        assert_eq!(r.faces.len(), 12 * 2 + 8 * 3);
        assert!((volume(&split) - volume(&whole)).abs() < 1e-5, "{} vs {}", volume(&split), volume(&whole));
    }

    #[test]
    fn one_box_edge_ends_with_caps() {
        let mut m = unit_box();
        let e = m.edges().next().unwrap();
        let r = bevel(&mut m, &[e], &BevelSpec { width: 0.1, ..BevelSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!(euler(&m), 2);
        assert_eq!(r.faces.len(), 3, "a strip and a triangle cap at each end");
        assert_eq!(r.faces.iter().filter(|&&f| m.face_len(f) == 3).count(), 2);
    }

    #[test]
    fn edge_loop_through_a_grid() {
        // Bevel the middle row of edges of a 4 x 4 grid: strips pass straight
        // through inner vertices, ending at the border.
        let mut m = plane(&PlaneSpec { segments_x: 4, segments_z: 4, ..PlaneSpec::default() }).unwrap();
        let row: Vec<u32> = m
            .edges()
            .filter(|&e| {
                let [a, b] = m.edge_verts(e);
                m.positions[a as usize].z.abs() < 1e-6 && m.positions[b as usize].z.abs() < 1e-6
            })
            .collect();
        assert_eq!(row.len(), 4);
        let r = bevel(&mut m, &row, &BevelSpec { width: 0.05, segments: 2, ..BevelSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!(r.skipped, 0);
        assert_eq!(euler(&m), 1, "still a disc");
        assert_eq!(r.faces.iter().filter(|&&f| m.face_len(f) == 4).count(), 8, "two quads across, four strips");
    }

    #[test]
    fn width_is_clamped_or_refused() {
        let mut m = unit_box();
        let all: Vec<u32> = m.edges().collect();
        let before = m.clone();
        let e = bevel(&mut m, &all, &BevelSpec { width: 0.8, clamp: false, ..BevelSpec::default() }).unwrap_err();
        assert!(matches!(e, KernelError::OutOfRange { param: "width", .. }));
        assert_eq!(m, before);
        let r = bevel(&mut m, &all, &BevelSpec { width: 0.8, clamp: true, ..BevelSpec::default() }).unwrap();
        m.validate().unwrap();
        assert!(r.width < 0.5, "{}", r.width);
    }
}
