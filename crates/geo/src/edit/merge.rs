//! Merging vertices: into one point (merge) or wherever they nearly
//! coincide (merge by distance, welding seams).

use glam::Vec3;
use p3d_mesh::{CornerSrc, Mesh, Patch};
use rustc_hash::FxHashMap;

use super::{KernelError, patch};

/// Splits a closed polygon wherever a vertex repeats into simple polygons,
/// after removing consecutive repeats; pieces with fewer than 3 corners are
/// dropped. Each corner carries a payload (its attribute source).
///
/// Complexity: O(k) expected for k corners.
pub(crate) fn simple_polygons<T: Copy>(corners: &[(u32, T)]) -> Vec<Vec<(u32, T)>> {
    // Consecutive repeats (cyclic) collapse to the first corner.
    let mut ring: Vec<(u32, T)> = Vec::with_capacity(corners.len());
    for &c in corners {
        if ring.last().is_none_or(|l| l.0 != c.0) {
            ring.push(c);
        }
    }
    while ring.len() > 1 && ring[0].0 == ring[ring.len() - 1].0 {
        ring.pop();
    }
    // Cut out loops between two visits of a vertex, innermost first, with a stack.
    let mut out = Vec::new();
    let mut stack: Vec<(u32, T)> = Vec::with_capacity(ring.len());
    let mut at: FxHashMap<u32, usize> = FxHashMap::default();
    for c in ring {
        if let Some(&i) = at.get(&c.0) {
            let piece: Vec<(u32, T)> = stack.drain(i..).collect();
            for x in &piece {
                at.remove(&x.0);
            }
            if piece.len() >= 3 {
                out.push(piece);
            }
        }
        at.insert(c.0, stack.len());
        stack.push(c);
    }
    if stack.len() >= 3 {
        out.push(stack);
    }
    out
}

/// Replaces every face around the mapped vertices with its corners renamed
/// through `rep` (vertices not in the map stay), splitting pinched results
/// and dropping collapsed ones.
///
/// Complexity: O(c log c) for the c corners of the affected faces.
pub(crate) fn remap_faces(mesh: &Mesh, p: &mut Patch, rep: &FxHashMap<u32, u32>) -> usize {
    let mut faces: Vec<u32> = rep.keys().flat_map(|&v| mesh.vert_faces(v)).collect();
    faces.sort_unstable();
    faces.dedup();
    let mut dropped = 0usize;
    for &f in &faces {
        let corners: Vec<(u32, u32)> = mesh.face_halfedges(f).map(|h| (rep.get(&mesh.origin(h)).copied().unwrap_or(mesh.origin(h)), h)).collect();
        p.remove_face(f);
        let pieces = simple_polygons(&corners);
        if pieces.is_empty() {
            dropped += 1;
        }
        for piece in pieces {
            let vs: Vec<u32> = piece.iter().map(|c| c.0).collect();
            let src: Vec<CornerSrc> = piece.iter().map(|c| CornerSrc::copy(c.1)).collect();
            p.add_face(&vs, &src, mesh.face_material(f), Some(f));
        }
    }
    dropped
}

/// How [`merge`] groups the vertices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeMode {
    /// All selected vertices become one, at their center.
    #[default]
    Center,
    /// Each connected run of selected vertices (joined by edges between
    /// selected vertices) becomes one, at its own center.
    Collapse,
}

/// What a merge did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// Vertices removed by merging (copies split off again where faces only
    /// touched at a merged vertex are not counted).
    pub merged: usize,
    /// Faces that collapsed to nothing (fewer than 3 distinct corners).
    pub faces_removed: usize,
    /// Groups of vertices that became one.
    pub clusters: usize,
}

/// Union-find root with path halving.
fn root(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

/// Merges vertices (see [`MergeMode`]). Faces that lose corners shrink;
/// faces left with fewer than three corners disappear; faces pinched at a
/// merged vertex split there. Fails without changes when the result would
/// not be manifold.
///
/// Complexity: O(k + c log c) for k selected vertices and the c corners of
/// their faces.
pub fn merge(mesh: &mut Mesh, verts: &[u32], mode: MergeMode) -> Result<MergeResult, KernelError> {
    let mut verts = verts.to_vec();
    verts.sort_unstable();
    verts.dedup();
    if let Some(&v) = verts.iter().find(|&&v| !mesh.is_vert_alive(v)) {
        return Err(KernelError::Empty(format!("vertex {v} does not exist")));
    }
    if verts.len() < 2 {
        return Err(KernelError::Empty("merging needs at least two vertices".into()));
    }
    // Clusters: one, or one per connected run.
    let index: FxHashMap<u32, u32> = verts.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
    let mut parent: Vec<u32> = (0..verts.len() as u32).collect();
    if mode == MergeMode::Collapse {
        for (i, &v) in verts.iter().enumerate() {
            for w in mesh.vert_neighbors(v) {
                if let Some(&j) = index.get(&w) {
                    let (a, b) = (root(&mut parent, i as u32), root(&mut parent, j));
                    parent[a.max(b) as usize] = a.min(b);
                }
            }
        }
    } else {
        parent.iter_mut().for_each(|x| *x = 0);
    }
    let mut members: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for i in 0..verts.len() as u32 {
        let r = root(&mut parent, i);
        members.entry(r).or_default().push(verts[i as usize]);
    }
    let mut roots: Vec<u32> = members.keys().copied().collect();
    roots.sort_unstable();
    let mut p = patch(mesh);
    let mut rep: FxHashMap<u32, u32> = FxHashMap::default();
    let mut clusters = 0usize;
    for r in roots {
        let vs = &members[&r];
        if vs.len() < 2 {
            continue;
        }
        let center = vs.iter().map(|&v| mesh.positions[v as usize]).sum::<Vec3>() / vs.len() as f32;
        let target = p.new_vert(center, Some(vs[0]));
        for &v in vs {
            rep.insert(v, target);
        }
        clusters += 1;
    }
    if clusters == 0 {
        return Ok(MergeResult::default());
    }
    let faces_removed = remap_faces(mesh, &mut p, &rep);
    let res = mesh.apply_patch(&p)?;
    // Vertices pinched apart again (faces that only touched at the merged vertex) do not count.
    Ok(MergeResult { merged: (rep.len() - clusters).saturating_sub(res.split_verts), faces_removed, clusters })
}

/// Welds vertices closer than `distance`: in vertex id order, each vertex not
/// welded yet takes every unwelded vertex within the distance of it (chains
/// do not collapse), and moves to the group's center. With `only`, just those vertices take
/// part. Seams of mirrored halves and joined parts close this way.
///
/// Complexity: O(n log n) for the n vertices taking part (a sorted uniform
/// grid with cells of size `distance`), plus the corners of affected faces.
pub fn merge_by_distance(mesh: &mut Mesh, only: Option<&[u32]>, distance: f32) -> Result<MergeResult, KernelError> {
    if !(distance.is_finite() && distance > 0.0) {
        return Err(KernelError::OutOfRange {
            param: "distance",
            message: "merge distance must be more than 0".into(),
            hint: "0.0001 (0.1 mm) welds coincident vertices".into(),
        });
    }
    let verts: Vec<u32> = match only {
        Some(list) => {
            let mut v: Vec<u32> = list.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
            v.sort_unstable();
            v.dedup();
            v
        }
        None => mesh.verts().collect(),
    };
    if verts.len() < 2 {
        return Ok(MergeResult::default());
    }
    let cell = |p: Vec3| -> [i64; 3] { (p / distance).floor().to_array().map(|x| x as i64) };
    // Vertices sorted by cell; each cell maps to its range in the order.
    let mut order: Vec<([i64; 3], u32)> = verts.iter().map(|&v| (cell(mesh.positions[v as usize]), v)).collect();
    order.sort_unstable();
    let mut ranges: FxHashMap<[i64; 3], (usize, usize)> = FxHashMap::default();
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j < order.len() && order[j].0 == order[i].0 {
            j += 1;
        }
        ranges.insert(order[i].0, (i, j));
        i = j;
    }
    // Non-transitive welding: in vertex id order, a vertex not welded yet
    // becomes a target and takes every unwelded vertex within the distance
    // of it (so a chain of close vertices does not collapse into one).
    let d2 = distance * distance;
    let mut welded: FxHashMap<u32, ()> = FxHashMap::default();
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for &v in &verts {
        if welded.contains_key(&v) {
            continue;
        }
        let pv = mesh.positions[v as usize];
        let c = cell(pv);
        let mut group = vec![v];
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(&(s, e)) = ranges.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                    for &(_, w) in &order[s..e] {
                        if w > v && !welded.contains_key(&w) && mesh.positions[w as usize].distance_squared(pv) <= d2 {
                            group.push(w);
                        }
                    }
                }
            }
        }
        if group.len() > 1 {
            for &w in &group[1..] {
                welded.insert(w, ());
            }
            group[1..].sort_unstable();
            groups.push(group);
        }
    }
    if groups.is_empty() {
        return Ok(MergeResult::default());
    }
    let mut rep: FxHashMap<u32, u32> = FxHashMap::default();
    let mut centers: Vec<(u32, Vec3)> = Vec::with_capacity(groups.len());
    for g in &groups {
        for &v in &g[1..] {
            rep.insert(v, g[0]);
        }
        centers.push((g[0], g.iter().map(|&v| mesh.positions[v as usize]).sum::<Vec3>() / g.len() as f32));
    }
    let mut p = patch(mesh);
    let faces_removed = remap_faces(mesh, &mut p, &rep);
    let res = mesh.apply_patch(&p)?;
    // The representatives survive (or were split into copies at the same place).
    for (v, c) in centers {
        if mesh.is_vert_alive(v) {
            mesh.positions[v as usize] = c;
        }
    }
    // Vertices pinched apart again (faces that only touched at the merged vertex) do not count.
    Ok(MergeResult { merged: rep.len().saturating_sub(res.split_verts), faces_removed, clusters: groups.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    #[test]
    fn simple_polygons_split_pinches() {
        let c = |v: &[u32]| v.iter().map(|&x| (x, ())).collect::<Vec<_>>();
        let ids = |ps: Vec<Vec<(u32, ())>>| ps.into_iter().map(|p| p.into_iter().map(|c| c.0).collect::<Vec<_>>()).collect::<Vec<_>>();
        assert_eq!(ids(simple_polygons(&c(&[1, 2, 2, 3, 4, 1]))), vec![vec![1, 2, 3, 4]]);
        assert_eq!(ids(simple_polygons(&c(&[1, 2, 3, 1, 4, 5]))), vec![vec![1, 2, 3], vec![1, 4, 5]]);
        assert!(simple_polygons(&c(&[1, 2, 1])).is_empty());
    }

    #[test]
    fn merging_a_quad_edge_makes_a_triangle() {
        let mut m = plane(&PlaneSpec::default()).unwrap();
        let r = merge(&mut m, &[2, 3], MergeMode::Center).unwrap();
        m.validate().unwrap();
        assert_eq!((r.merged, r.clusters, m.vert_count(), m.face_count()), (1, 1, 3, 1));
        assert_eq!(m.face_len(m.faces().next().unwrap()), 3);
    }

    #[test]
    fn collapse_runs_separately() {
        let mut m = plane(&PlaneSpec { segments_x: 4, segments_z: 1, ..PlaneSpec::default() }).unwrap();
        // Bottom row 0..=4, top row 5..=9: collapse edges 0-1 and 3-4 separately.
        let r = merge(&mut m, &[0, 1, 3, 4], MergeMode::Collapse).unwrap();
        m.validate().unwrap();
        assert_eq!((r.clusters, r.merged), (2, 2));
        assert_eq!(m.faces().filter(|&f| m.face_len(f) == 3).count(), 2);
    }

    #[test]
    fn weld_two_boxes_sharing_a_face() {
        let a = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let mut b = a.clone();
        b.transform(glam::Affine3A::from_translation(Vec3::X));
        let mut m = a.clone();
        m.append(&b);
        // Delete the two touching faces, then weld: one long box.
        let right: Vec<u32> = m.group("right").unwrap().iter().take(1).collect();
        let left: Vec<u32> = m.group("left").unwrap().iter().skip(1).collect();
        m.delete_faces(&[right, left].concat()).unwrap();
        let r = merge_by_distance(&mut m, None, 1e-4).unwrap();
        m.validate().unwrap();
        assert_eq!((r.merged, r.clusters), (4, 4));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        assert!(merge_by_distance(&mut m, None, 0.0).is_err());
        assert_eq!(merge_by_distance(&mut m, None, 1e-4).unwrap(), MergeResult::default(), "nothing left to weld");
    }
}
