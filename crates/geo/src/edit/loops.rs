//! Edge loops: loop cuts across quad rings, and bridges between two holes.

use p3d_mesh::{CornerSrc, EdgeFlags, Mesh, Patch, PatchResult};
use rustc_hash::{FxHashMap, FxHashSet};

use super::{KernelError, patch};

/// The ring of quads through edge `e`: the half-edge by which the ring
/// enters each quad (it leaves through the opposite side,
/// `next(next(h))`), and whether the ring closes. The ring stops at the
/// boundary and at faces that are not quads.
///
/// Complexity: O(ring length).
fn edge_ring(mesh: &Mesh, e: u32) -> (Vec<u32>, bool) {
    let limit = mesh.face_slots() + 1;
    let walk = |start: u32| -> (Vec<u32>, bool) {
        let mut steps = Vec::new();
        let mut h = start;
        while let Some(f) = mesh.face(h) {
            if mesh.face_len(f) != 4 || steps.len() > limit {
                break;
            }
            steps.push(h);
            h = mesh.next(mesh.next(h)) ^ 1;
            if h == start {
                return (steps, true);
            }
        }
        (steps, false)
    };
    let (forward, closed) = walk(2 * e);
    if closed {
        return (forward, true);
    }
    // The other way, re-entered from the far side so all steps point the same way.
    let (backward, _) = walk(2 * e + 1);
    let mut ring: Vec<u32> = backward.iter().rev().map(|&h| mesh.next(mesh.next(h))).collect();
    ring.extend(forward);
    (ring, false)
}

/// Mesh id of a vertex named in a patch, after the patch was applied.
fn map_vert(p: &Patch, r: &PatchResult, v: u32) -> u32 {
    if p.is_new_vert(v) { r.verts[p.new_vert_index(v)] } else { v }
}

/// What [`loop_cut`] made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopCutResult {
    /// The new edges across the ring quads.
    pub edges: Vec<u32>,
    /// The quads the cut split the rings into.
    pub faces: Vec<u32>,
    /// Rings cut.
    pub rings: usize,
}

/// Largest number of parallel cuts per ring.
pub const MAX_CUTS: u32 = 64;

/// Cuts edge loops across the quad rings through the given edges: every ring
/// edge is split at `cuts` evenly spaced points (a single cut is shifted by
/// `offset` in [-1, 1] toward either side) and every quad of the ring
/// becomes `cuts + 1` quads. A ring ends at the boundary or at a face that
/// is not a quad; that face gains the cut points as extra corners. Several
/// selected edges of one ring cut it once; rings that cross are refused.
///
/// Complexity: O(ring length x cuts + c log c) for the c corners touched.
pub fn loop_cut(mesh: &mut Mesh, edges: &[u32], cuts: u32, offset: f32) -> Result<LoopCutResult, KernelError> {
    if !(1..=MAX_CUTS).contains(&cuts) {
        return Err(KernelError::OutOfRange {
            param: "cuts",
            message: format!("{cuts} cuts is outside 1 to {MAX_CUTS}"),
            hint: "1 to 4 cuts are typical".into(),
        });
    }
    if !(offset.is_finite() && (-1.0..=1.0).contains(&offset)) {
        return Err(KernelError::OutOfRange {
            param: "offset",
            message: format!("offset {offset} is outside -1 to 1"),
            hint: "0 cuts through the middle".into(),
        });
    }
    let mut edges = edges.to_vec();
    edges.sort_unstable();
    edges.dedup();
    if let Some(&e) = edges.iter().find(|&&e| !mesh.is_edge_alive(e)) {
        return Err(KernelError::Empty(format!("edge {e} does not exist")));
    }
    let mut ring_edges: FxHashSet<u32> = FxHashSet::default();
    let mut ring_faces: FxHashSet<u32> = FxHashSet::default();
    let mut rings: Vec<(Vec<u32>, bool)> = Vec::new();
    for &e in &edges {
        if ring_edges.contains(&e) {
            continue;
        }
        let (ring, closed) = edge_ring(mesh, e);
        if ring.is_empty() {
            continue;
        }
        for &h in &ring {
            if !ring_faces.insert(mesh.face(h).unwrap_or(u32::MAX)) {
                return Err(KernelError::OutOfRange {
                    param: "select",
                    message: "the selected edge rings cross each other (or one crosses itself)".into(),
                    hint: "cut one ring per call".into(),
                });
            }
            ring_edges.insert(h >> 1);
        }
        if let Some(&last) = ring.last() {
            ring_edges.insert(mesh.next(mesh.next(last)) >> 1);
        }
        rings.push((ring, closed));
    }
    if rings.is_empty() {
        return Err(KernelError::Empty("no ring of quads runs through the selected edges".into()));
    }
    let fractions: Vec<f32> = if cuts == 1 { vec![0.5 + 0.49 * offset] } else { (1..=cuts).map(|i| i as f32 / (cuts + 1) as f32).collect() };

    let mut p = patch(mesh);
    // Cut points along each split half-edge, ordered from its origin, with
    // their fractions from the origin.
    let mut points: FxHashMap<u32, Vec<(u32, f32)>> = FxHashMap::default();
    let mut made: Vec<usize> = Vec::new();
    let mut cut_pairs: Vec<(u32, u32)> = Vec::new();
    let mut ends: Vec<u32> = Vec::new();
    for (ring, closed) in &rings {
        let mut sides: Vec<u32> = ring.clone();
        if !closed && let Some(&last) = ring.last() {
            sides.push(mesh.next(mesh.next(last)) ^ 1);
        }
        for &h in &sides {
            let (a, b) = (mesh.positions[mesh.origin(h) as usize], mesh.positions[mesh.dest(h) as usize]);
            let pts: Vec<(u32, f32)> = fractions.iter().map(|&t| (p.new_vert(a.lerp(b, t), None), t)).collect();
            points.insert(h ^ 1, pts.iter().rev().map(|&(v, t)| (v, 1.0 - t)).collect());
            points.insert(h, pts);
        }
        for &h0 in ring {
            let f = mesh.face(h0).unwrap_or(u32::MAX);
            p.remove_face(f);
            // Quad a -> b -> c -> d entered by h0 = a -> b; the opposite side h2 = c -> d.
            let (h1, h2) = (mesh.next(h0), mesh.next(mesh.next(h0)));
            let h3 = mesh.next(h2);
            let (on_ab, on_cd) = (&points[&h0], &points[&h2]);
            let k = on_ab.len();
            // Cut i joins on_ab[i] to the point the same fraction from d.
            let cut = |i: usize| (on_ab[i].0, on_cd[k - 1 - i].0, CornerSrc::lerp(h0, h1, on_ab[i].1), CornerSrc::lerp(h3, h2, on_ab[i].1));
            for i in 0..=k {
                let (va, vd, sa, sd) = if i == 0 { (mesh.origin(h0), mesh.origin(h3), CornerSrc::copy(h0), CornerSrc::copy(h3)) } else { cut(i - 1) };
                let (vb, vc, sb, sc) = if i == k { (mesh.origin(h1), mesh.origin(h2), CornerSrc::copy(h1), CornerSrc::copy(h2)) } else { cut(i) };
                made.push(p.add_face(&[va, vb, vc, vd], &[sa, sb, sc, sd], mesh.face_material(f), Some(f)));
                if i < k {
                    cut_pairs.push((vb, vc));
                }
            }
        }
        if !closed {
            // The faces across the ring's first entry side and last exit side.
            for h in [ring[0] ^ 1, ring.last().map_or(0, |&l| mesh.next(mesh.next(l)) ^ 1)] {
                if let Some(g) = mesh.face(h) {
                    ends.push(g);
                }
            }
        }
    }
    ends.sort_unstable();
    ends.dedup();
    for g in ends {
        let (mut vs, mut src) = (Vec::new(), Vec::new());
        for h in mesh.face_halfedges(g) {
            vs.push(mesh.origin(h));
            src.push(CornerSrc::copy(h));
            for &(v, t) in points.get(&h).map(Vec::as_slice).unwrap_or(&[]) {
                vs.push(v);
                src.push(CornerSrc::lerp(h, mesh.next(h), t));
            }
        }
        p.remove_face(g);
        p.add_face(&vs, &src, mesh.face_material(g), Some(g));
    }
    // Split edges keep their flags on every piece.
    let flagged: Vec<(Vec<u32>, EdgeFlags)> = points
        .iter()
        .filter(|&(&h, _)| h & 1 == 0 && mesh.edge_flags(h >> 1) != EdgeFlags::NONE)
        .map(|(&h, pts)| {
            let mut chain = vec![mesh.origin(h)];
            chain.extend(pts.iter().map(|x| x.0));
            chain.push(mesh.dest(h));
            (chain, mesh.edge_flags(h >> 1))
        })
        .collect();
    let r = mesh.apply_patch(&p)?;
    for (chain, fl) in flagged {
        for w in chain.windows(2) {
            if let Some(h) = mesh.find_halfedge(map_vert(&p, &r, w[0]), map_vert(&p, &r, w[1])) {
                for flag in [EdgeFlags::SHARP, EdgeFlags::SEAM] {
                    if fl.contains(flag) {
                        mesh.set_edge_flag(h >> 1, flag, true);
                    }
                }
            }
        }
    }
    let mut out = LoopCutResult { rings: rings.len(), faces: made.into_iter().map(|i| r.faces[i]).collect(), edges: Vec::new() };
    for (a, b) in cut_pairs {
        if let Some(h) = mesh.find_halfedge(map_vert(&p, &r, a), map_vert(&p, &r, b)) {
            out.edges.push(h >> 1);
        }
    }
    out.edges.sort_unstable();
    out.edges.dedup();
    Ok(out)
}

/// What [`bridge`] made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeResult {
    pub faces: Vec<u32>,
}

/// Largest hole size for the alignment search of [`bridge`] (it compares
/// every pairing offset, O(a x b)).
pub const MAX_BRIDGE_LOOP: usize = 4096;

/// Connects two holes (boundary loops, given by one boundary half-edge of
/// each) with a band of quads. The loops are aligned at the offset with the
/// shortest total connection; a band face runs along each hole in the
/// direction of its boundary, so the band's facing follows the surface.
/// Holes with different vertex counts get triangles where one side has
/// spare vertices. `segments` > 1 adds straight interpolated rings.
///
/// Complexity: O(a x b) for holes of a and b vertices (alignment), each at
/// most [`MAX_BRIDGE_LOOP`], plus the patch.
pub fn bridge(mesh: &mut Mesh, loop_a: u32, loop_b: u32, segments: u32) -> Result<BridgeResult, KernelError> {
    let la = mesh.boundary_loop(loop_a)?;
    let lb = mesh.boundary_loop(loop_b)?;
    if la.contains(&loop_b) {
        return Err(KernelError::OutOfRange {
            param: "select",
            message: "both picks are on the same hole".into(),
            hint: "select edges of two different holes".into(),
        });
    }
    if !(1..=MAX_CUTS).contains(&segments) {
        return Err(KernelError::OutOfRange {
            param: "segments",
            message: format!("{segments} segments is outside 1 to {MAX_CUTS}"),
            hint: "1 makes one band of quads".into(),
        });
    }
    if la.len() > MAX_BRIDGE_LOOP || lb.len() > MAX_BRIDGE_LOOP {
        return Err(KernelError::OutOfRange {
            param: "select",
            message: format!("holes larger than {MAX_BRIDGE_LOOP} edges cannot be bridged"),
            hint: "bridge smaller holes".into(),
        });
    }
    let pa: Vec<u32> = la.iter().map(|&h| mesh.origin(h)).collect();
    // A band face uses a forward along its boundary and b against it, so b
    // is walked backward.
    let mut pb: Vec<u32> = lb.iter().map(|&h| mesh.origin(h)).collect();
    pb.reverse();
    let pos = |v: u32| mesh.positions[v as usize];
    let (na, nb) = (pa.len(), pb.len());
    let n = na.max(nb);
    let sample = |list: &[u32], len: usize, i: usize| list[(i * len / n) % len];
    let mut best = (f32::INFINITY, 0usize);
    for s in 0..nb {
        let cost: f32 = (0..n).map(|i| pos(sample(&pa, na, i)).distance(pos(pb[(s + i * nb / n) % nb]))).sum();
        if cost < best.0 {
            best = (cost, s);
        }
    }
    let pb: Vec<u32> = (0..nb).map(|i| pb[(best.1 + i) % nb]).collect();
    let mut p = patch(mesh);
    let mut rows: Vec<Vec<u32>> = vec![(0..n).map(|i| sample(&pa, na, i)).collect()];
    for s in 1..segments {
        let t = s as f32 / segments as f32;
        rows.push((0..n).map(|i| p.new_vert(pos(sample(&pa, na, i)).lerp(pos(sample(&pb, nb, i)), t), None)).collect());
    }
    rows.push((0..n).map(|i| sample(&pb, nb, i)).collect());
    let mut made = Vec::new();
    for s in 0..segments as usize {
        for i in 0..n {
            let j = (i + 1) % n;
            let quad = [rows[s][i], rows[s][j], rows[s + 1][j], rows[s + 1][i]];
            // Spare vertices on one side repeat: those quads are triangles.
            let poly: Vec<u32> = (0..4).filter(|&k| quad[k] != quad[(k + 3) % 4]).map(|k| quad[k]).collect();
            if poly.len() >= 3 {
                made.push(p.add_face(&poly, &[], 0, None));
            }
        }
    }
    let r = mesh.apply_patch(&p)?;
    Ok(BridgeResult { faces: made.into_iter().map(|i| r.faces[i]).collect() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, CapKind, CylinderSpec, Origin, PlaneSpec, cuboid, cylinder, plane};
    use glam::{DVec3, Vec3};

    #[test]
    fn loop_cut_a_cylinder_side() {
        let mut m = cylinder(&CylinderSpec { segments: 8, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Bottom).unwrap();
        // A vertical side edge: its ring runs around the cylinder and closes.
        let e = m.edges().find(|&e| {
            let [a, b] = m.edge_verts(e);
            (m.positions[a as usize].y - m.positions[b as usize].y).abs() > 0.5
        });
        let r = loop_cut(&mut m, &[e.unwrap()], 1, 0.0).unwrap();
        m.validate().unwrap();
        assert_eq!((r.rings, r.faces.len(), r.edges.len()), (1, 16, 8));
        assert_eq!(m.face_count(), 16 + 2);
        assert!(r.edges.iter().all(|&e| m.edge_verts(e).iter().all(|&v| (m.positions[v as usize].y - 0.5).abs() < 1e-6)));
        assert_eq!(m.group("side").unwrap().count(), 16);
        // An edge along the rim: its ring crosses the caps, which are n-gons,
        // so it stops there and they gain the cut points.
        let rim = m.edges().find(|&e| m.edge_verts(e).iter().all(|&v| m.positions[v as usize].y.abs() < 1e-6)).unwrap();
        let r = loop_cut(&mut m, &[rim], 2, 0.0).unwrap();
        m.validate().unwrap();
        assert_eq!(r.faces.len(), 2 * 3);
        assert!(m.faces().any(|f| m.face_len(f) == 10), "the bottom n-gon gained two corners");
    }

    #[test]
    fn loop_cut_keeps_sharp_edges_and_offsets() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let e = m.edges().next().unwrap();
        let r = loop_cut(&mut m, &[e], 1, 0.5).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces.len(), r.edges.len()), (8, 4));
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 16, "12 box edges, 4 of them split in two");
        assert!(loop_cut(&mut m, &[e], 0, 0.0).is_err());
    }

    #[test]
    fn crossing_rings_are_refused() {
        let mut m = plane(&PlaneSpec { segments_x: 2, segments_z: 2, ..PlaneSpec::default() }).unwrap();
        let before = m.clone();
        let hor = m.edges().find(|&e| m.edge_verts(e) == [0, 1] || m.edge_verts(e) == [1, 0]).unwrap();
        let ver = m.edges().find(|&e| m.edge_verts(e) == [0, 3] || m.edge_verts(e) == [3, 0]).unwrap();
        assert!(loop_cut(&mut m, &[hor, ver], 1, 0.0).is_err());
        assert_eq!(m, before);
    }

    #[test]
    fn bridge_two_tubes() {
        let open = |y: f64| {
            let mut t =
                cylinder(&CylinderSpec { radius: 0.5, height: 1.0, segments: 8, height_segments: 1, cap: CapKind::None }, Origin::Center).unwrap();
            t.transform(glam::Affine3A::from_translation(Vec3::new(0.0, y as f32, 0.0)));
            t
        };
        let mut m = open(0.0);
        m.append(&open(2.0));
        let boundary = |m: &Mesh, y: f32| {
            (0..m.he_slots() as u32)
                .find(|&h| m.is_he_alive(h) && m.is_boundary_he(h) && (m.positions[m.origin(h) as usize].y - y).abs() < 1e-5)
                .unwrap()
        };
        let (a, b) = (boundary(&m, 0.5), boundary(&m, 1.5));
        let r = bridge(&mut m, a, b, 2).unwrap();
        m.validate().unwrap();
        assert_eq!(r.faces.len(), 16);
        assert_eq!(m.face_components().1, 1, "one tube now");
        assert_eq!(m.edges().filter(|&e| m.is_boundary_edge(e)).count(), 16, "only the two far ends stay open");
        // Facing agrees with the tubes: outward.
        assert!(r.faces.iter().all(|&f| {
            let c = m.face_centroid(f);
            m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0
        }));
    }

    #[test]
    fn bridge_unequal_holes() {
        let mut a =
            cylinder(&CylinderSpec { radius: 0.5, height: 1.0, segments: 8, height_segments: 1, cap: CapKind::None }, Origin::Center).unwrap();
        let mut b =
            cylinder(&CylinderSpec { radius: 0.3, height: 1.0, segments: 6, height_segments: 1, cap: CapKind::None }, Origin::Center).unwrap();
        b.transform(glam::Affine3A::from_translation(Vec3::new(0.0, 2.0, 0.0)));
        a.append(&b);
        let boundary = |m: &Mesh, y: f32| {
            (0..m.he_slots() as u32)
                .find(|&h| m.is_he_alive(h) && m.is_boundary_he(h) && (m.positions[m.origin(h) as usize].y - y).abs() < 1e-5)
                .unwrap()
        };
        let (ha, hb) = (boundary(&a, 0.5), boundary(&a, 1.5));
        let r = bridge(&mut a, ha, hb, 1).unwrap();
        a.validate().unwrap();
        assert_eq!(r.faces.len(), 8);
        assert_eq!(r.faces.iter().filter(|&&f| a.face_len(f) == 3).count(), 2);
        assert!(bridge(&mut a, ha, ha, 1).is_err());
    }
}
