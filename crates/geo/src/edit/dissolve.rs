//! Dissolving vertices, edges and faces: removing elements while keeping the
//! surface closed, by merging the faces around them into larger polygons.
//!
//! Every mode works in at most two patches: regions of faces merge into one
//! polygon each (over the region's border), then vertices left with two
//! edges drop out of their faces. Dissolving many neighbouring elements
//! never grows a polygon step by step, so the cost stays linear.

use p3d_mesh::{CornerSrc, EdgeFlags, Mesh, Patch};
use rustc_hash::{FxHashMap, FxHashSet};

use super::merge::simple_polygons;
use super::{KernelError, face_bits, patch};

/// What a dissolve did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DissolveResult {
    /// Polygons made by merging faces.
    pub faces: Vec<u32>,
    pub removed_verts: usize,
    pub removed_edges: usize,
    /// Selected elements left alone (on the boundary, a merge that would
    /// need a hole, or a face that would drop below three corners).
    pub skipped: usize,
}

/// Adds the simple polygons of a corner list (vertex, source half-edge) to a
/// patch and returns their patch indices.
fn add_polygons(mesh: &Mesh, p: &mut Patch, corners: &[(u32, u32)], like: u32) -> Vec<usize> {
    simple_polygons(corners)
        .into_iter()
        .map(|piece| {
            let vs: Vec<u32> = piece.iter().map(|c| c.0).collect();
            let src: Vec<CornerSrc> = piece.iter().map(|c| CornerSrc::copy(c.1)).collect();
            p.add_face(&vs, &src, mesh.face_material(like), Some(like))
        })
        .collect()
}

/// Applies a patch and records what it did.
fn apply(mesh: &mut Mesh, p: &Patch, made: &[usize], out: &mut DissolveResult) -> Result<(), KernelError> {
    if p.is_empty() {
        return Ok(());
    }
    let r = mesh.apply_patch(p)?;
    out.faces.retain(|&f| mesh.is_face_alive(f));
    out.faces.extend(made.iter().map(|&i| r.faces[i]));
    out.removed_verts += r.removed_verts;
    out.removed_edges += r.removed_edges;
    Ok(())
}

/// A region of faces: its members (sorted) and its border loops as
/// half-edge lists in winding order.
type Region = (Vec<u32>, Vec<Vec<u32>>);

/// Connected regions of the given faces, joined across edges where `join`
/// holds, each with its border loops.
///
/// Complexity: O(c) expected for the c corners of the faces.
fn regions(mesh: &Mesh, faces: &[u32], join: impl Fn(u32) -> bool) -> Vec<Region> {
    let sel = face_bits(mesh, faces);
    let mut label: FxHashMap<u32, usize> = FxHashMap::default();
    let mut out: Vec<Region> = Vec::new();
    for &f in faces {
        if label.contains_key(&f) {
            continue;
        }
        let id = out.len();
        let mut members = vec![f];
        label.insert(f, id);
        let mut stack = vec![f];
        while let Some(x) = stack.pop() {
            for h in mesh.face_halfedges(x) {
                if let Some(y) = mesh.face(h ^ 1)
                    && sel.contains(y as usize)
                    && join(h >> 1)
                    && !label.contains_key(&y)
                {
                    label.insert(y, id);
                    members.push(y);
                    stack.push(y);
                }
            }
        }
        members.sort_unstable();
        out.push((members, Vec::new()));
    }
    let inside = |h: u32, id: usize| mesh.face(h ^ 1).is_some_and(|y| label.get(&y) == Some(&id) && join(h >> 1));
    let guard = mesh.he_slots();
    for (id, (members, loops)) in out.iter_mut().enumerate() {
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        for &f in members.iter() {
            for h in mesh.face_halfedges(f) {
                if inside(h, id) || seen.contains(&h) {
                    continue;
                }
                // From a border half-edge, the next one starts at its end:
                // turn around that vertex inside the region until leaving it.
                let mut lp = Vec::new();
                let mut x = h;
                for _ in 0..guard {
                    seen.insert(x);
                    lp.push(x);
                    let mut y = mesh.next(x);
                    for _ in 0..guard {
                        if !inside(y, id) {
                            break;
                        }
                        y = mesh.next(y ^ 1);
                    }
                    x = y;
                    if x == h {
                        break;
                    }
                }
                loops.push(lp);
            }
        }
    }
    out
}

/// Replaces every region of two or more faces whose border is one loop by
/// one polygon over that border, leaving out the vertices in `drop` (also
/// single faces that lose vertices). Regions with holes, or whose polygon
/// would have fewer than three corners, stay as they are.
///
/// Complexity: O(c log c) for the c corners of the regions.
fn merge_regions(mesh: &mut Mesh, regions: Vec<Region>, drop: &FxHashSet<u32>, out: &mut DissolveResult) -> Result<(), KernelError> {
    let mut p = patch(mesh);
    let mut made = Vec::new();
    for (members, loops) in regions {
        let poly: Vec<(u32, u32)> =
            loops.first().map(|l| l.iter().map(|&h| (mesh.origin(h), h)).filter(|c| !drop.contains(&c.0)).collect()).unwrap_or_default();
        let changed = members.len() >= 2 || loops.first().is_some_and(|l| l.len() != poly.len());
        if !changed {
            continue;
        }
        if loops.len() != 1 || poly.len() < 3 {
            out.skipped += 1;
            continue;
        }
        for &f in &members {
            p.remove_face(f);
        }
        made.extend(add_polygons(mesh, &mut p, &poly, members[0]));
    }
    apply(mesh, &p, &made, out)
}

/// Takes vertices that have exactly two edges out of their faces (their two
/// edges join into one), all at once. A face that would drop below three
/// corners keeps its vertices.
///
/// Complexity: O(c log c) for the c corners of the faces around them.
fn drop_chain_verts(mesh: &mut Mesh, cands: &[u32], out: &mut DissolveResult) -> Result<(), KernelError> {
    let mut set: FxHashSet<u32> = cands.iter().copied().filter(|&v| mesh.is_vert_alive(v) && mesh.valence(v) == 2).collect();
    let mut faces: Vec<u32> = set.iter().flat_map(|&v| mesh.vert_faces(v).collect::<Vec<_>>()).collect();
    faces.sort_unstable();
    faces.dedup();
    // One pass suffices: keeping a vertex only gives other faces more corners.
    for &f in &faces {
        let n = mesh.face_verts(f).filter(|v| set.contains(v)).count();
        if mesh.face_len(f) - n < 3 {
            for v in mesh.face_verts(f).collect::<Vec<_>>() {
                if set.remove(&v) {
                    out.skipped += 1;
                }
            }
        }
    }
    if set.is_empty() {
        return Ok(());
    }
    let mut p = patch(mesh);
    let mut made = Vec::new();
    for &f in &faces {
        if !mesh.face_verts(f).any(|v| set.contains(&v)) {
            continue;
        }
        let corners: Vec<(u32, u32)> = mesh.face_halfedges(f).map(|h| (mesh.origin(h), h)).filter(|c| !set.contains(&c.0)).collect();
        p.remove_face(f);
        made.extend(add_polygons(mesh, &mut p, &corners, f));
    }
    apply(mesh, &p, &made, out)
}

/// Dissolves vertices. A vertex with two edges drops out of its faces (its
/// edges join into one); any other vertex disappears with the faces around
/// it merged into one polygon (neighbouring dissolved vertices share one
/// polygon).
///
/// Complexity: O(c log c) for the c corners of the faces around the vertices.
pub fn dissolve_verts(mesh: &mut Mesh, verts: &[u32]) -> Result<DissolveResult, KernelError> {
    let mut verts: Vec<u32> = verts.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
    verts.sort_unstable();
    verts.dedup();
    let (chain, fans): (Vec<u32>, Vec<u32>) = verts.into_iter().partition(|&v| mesh.valence(v) == 2);
    let fan_set: FxHashSet<u32> = fans.iter().copied().collect();
    let mut faces: Vec<u32> = fans.iter().flat_map(|&v| mesh.vert_faces(v).collect::<Vec<_>>()).collect();
    faces.sort_unstable();
    faces.dedup();
    let mut out = DissolveResult::default();
    if !faces.is_empty() {
        let regs = regions(mesh, &faces, |e| mesh.edge_verts(e).iter().any(|v| fan_set.contains(v)));
        merge_regions(mesh, regs, &fan_set, &mut out)?;
    }
    drop_chain_verts(mesh, &chain, &mut out)?;
    Ok(out)
}

/// How far from a straight line a two-edge vertex may bend and still be
/// dissolved as redundant after an edge dissolve.
pub const STRAIGHT_DEG: f32 = 5.0;

/// Whether `v` has exactly two edges that continue each other within the
/// angle whose cosine is `cos`.
///
/// Complexity: O(valence).
fn is_straight(mesh: &Mesh, v: u32, cos: f32) -> bool {
    let mut ns = mesh.vert_neighbors(v);
    let (Some(a), Some(b), None) = (ns.next(), ns.next(), ns.next()) else { return false };
    let p = mesh.positions[v as usize];
    let (da, db) = ((mesh.positions[a as usize] - p).normalize_or_zero(), (mesh.positions[b as usize] - p).normalize_or_zero());
    da.dot(-db) >= cos
}

/// Dissolves edges, merging the faces on both sides of each into one
/// polygon (connected dissolved edges merge their faces together).
/// Boundary edges are skipped. With `clean_verts`, end vertices left with
/// two edges on a straight line (within [`STRAIGHT_DEG`]) go too.
///
/// Complexity: O(c log c) for the c corners of the touched faces.
pub fn dissolve_edges(mesh: &mut Mesh, edges: &[u32], clean_verts: bool) -> Result<DissolveResult, KernelError> {
    let mut out = DissolveResult::default();
    let mut set: FxHashSet<u32> = FxHashSet::default();
    for &e in edges {
        if !mesh.is_edge_alive(e) {
            continue;
        }
        match mesh.edge_faces(e) {
            [Some(a), Some(b)] if a != b => {
                set.insert(e);
            }
            _ => out.skipped += 1,
        }
    }
    let mut faces: Vec<u32> = set.iter().flat_map(|&e| mesh.edge_faces(e).into_iter().flatten()).collect();
    faces.sort_unstable();
    faces.dedup();
    let mut ends: Vec<u32> = set.iter().flat_map(|&e| mesh.edge_verts(e)).collect();
    ends.sort_unstable();
    ends.dedup();
    if !faces.is_empty() {
        let regs = regions(mesh, &faces, |e| set.contains(&e));
        merge_regions(mesh, regs, &FxHashSet::default(), &mut out)?;
    }
    if clean_verts {
        let cos = STRAIGHT_DEG.to_radians().cos();
        let chain: Vec<u32> = ends.into_iter().filter(|&v| mesh.is_vert_alive(v) && is_straight(mesh, v, cos)).collect();
        drop_chain_verts(mesh, &chain, &mut out)?;
    }
    Ok(out)
}

/// Dissolves faces: each connected region of selected faces becomes one
/// polygon over its border (inner vertices and edges disappear). Regions
/// with holes are skipped.
///
/// Complexity: O(c log c) for the c corners of the selected faces.
pub fn dissolve_faces(mesh: &mut Mesh, faces: &[u32]) -> Result<DissolveResult, KernelError> {
    let faces = super::live_faces(mesh, faces)?;
    let mut out = DissolveResult::default();
    let regs = regions(mesh, &faces, |_| true);
    merge_regions(mesh, regs, &FxHashSet::default(), &mut out)?;
    Ok(out)
}

/// Limited dissolve: merges faces across edges flatter than `angle_deg`
/// (not sharp, not seams, same material) into polygons, then drops
/// vertices of those polygons that have two edges on a straight line
/// (within the angle). Regions that would need a hole are kept.
///
/// Complexity: O(c log c) for the c corners of the selected faces.
pub fn dissolve_limited(mesh: &mut Mesh, faces: &[u32], angle_deg: f32) -> Result<DissolveResult, KernelError> {
    let faces = super::live_faces(mesh, faces)?;
    if !(angle_deg.is_finite() && (0.0..=180.0).contains(&angle_deg)) {
        return Err(KernelError::OutOfRange {
            param: "angle",
            message: format!("{angle_deg} degrees is outside 0 to 180"),
            hint: "5 degrees merges flat areas".into(),
        });
    }
    let cos = angle_deg.to_radians().cos();
    let flat = |e: u32| -> bool {
        let [Some(a), Some(b)] = mesh.edge_faces(e) else { return false };
        let fl = mesh.edge_flags(e);
        !fl.contains(EdgeFlags::SHARP)
            && !fl.contains(EdgeFlags::SEAM)
            && mesh.face_material(a) == mesh.face_material(b)
            && mesh.face_normal(a).dot(mesh.face_normal(b)) >= cos
    };
    let mut out = DissolveResult::default();
    let regs = regions(mesh, &faces, flat);
    merge_regions(mesh, regs, &FxHashSet::default(), &mut out)?;
    let mut cands: Vec<u32> = out.faces.iter().flat_map(|&f| mesh.face_verts(f).collect::<Vec<_>>()).collect();
    cands.sort_unstable();
    cands.dedup();
    let straight: Vec<u32> = cands.into_iter().filter(|&v| is_straight(mesh, v, cos)).collect();
    drop_chain_verts(mesh, &straight, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    fn grid(n: u32) -> Mesh {
        plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: n, segments_z: n }).unwrap()
    }

    #[test]
    fn dissolve_a_grid_center_vertex() {
        let mut m = grid(2);
        let r = dissolve_verts(&mut m, &[4]).unwrap();
        m.validate().unwrap();
        assert_eq!((m.face_count(), m.vert_count(), r.faces.len()), (1, 8, 1));
        assert_eq!(m.face_len(r.faces[0]), 8);
        // Corner vertex of a single quad: the quad becomes a triangle; a
        // triangle cannot lose a corner.
        let mut m = grid(1);
        let r = dissolve_verts(&mut m, &[0]).unwrap();
        m.validate().unwrap();
        assert_eq!((r.skipped, m.face_len(r.faces[0]), m.vert_count()), (0, 3, 3));
        let v = m.verts().next().unwrap();
        assert_eq!(dissolve_verts(&mut m, &[v]).unwrap().skipped, 1);
    }

    #[test]
    fn dissolving_neighbouring_vertices_at_once() {
        // All 9 inner vertices of a 4 x 4 grid: one 16-corner polygon.
        let mut m = grid(4);
        let inner: Vec<u32> = m.verts().filter(|&v| !m.is_boundary_vert(v)).collect();
        let r = dissolve_verts(&mut m, &inner).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces.len(), m.face_count(), m.vert_count()), (1, 1, 16));
    }

    #[test]
    fn dissolve_edges_and_clean_chains() {
        let mut m = grid(2);
        // The middle edge between faces 0 and 1 (vertices 1 and 4).
        let e = m.find_halfedge(1, 4).map(|h| h >> 1).unwrap();
        let r = dissolve_edges(&mut m, &[e], true).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 3);
        assert_eq!(r.removed_verts, 1, "vertex 1 on the straight border goes");
        let border = m.edges().find(|&e| m.is_boundary_edge(e)).unwrap();
        assert_eq!(dissolve_edges(&mut m, &[border], false).unwrap().skipped, 1);
    }

    #[test]
    fn dissolve_faces_of_a_box_side() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, segments: [3, 3, 3], ..BoxSpec::default() }, Origin::Center).unwrap();
        let top: Vec<u32> = m.group("top").unwrap().iter().collect();
        let r = dissolve_faces(&mut m, &top).unwrap();
        m.validate().unwrap();
        assert_eq!(r.faces.len(), 1);
        assert_eq!(m.face_len(r.faces[0]), 12, "the top's border has 12 vertices");
        assert!(m.group("top").unwrap().contains(r.faces[0]));
        assert_eq!(r.removed_verts, 4);
    }

    #[test]
    fn limited_dissolve_flattens_a_segmented_box() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, segments: [4, 4, 4], ..BoxSpec::default() }, Origin::Center).unwrap();
        let all: Vec<u32> = m.faces().collect();
        dissolve_limited(&mut m, &all, 5.0).unwrap();
        m.validate().unwrap();
        // Back to a plain box: sharp group borders stop the merge at the edges.
        assert_eq!((m.face_count(), m.vert_count()), (6, 8));
        assert!(m.faces().all(|f| m.face_len(f) == 4));
    }
}
