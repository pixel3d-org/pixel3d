//! Detaching faces (split) and simple subdivision.

use p3d_mesh::{CornerSrc, EdgeFlags, Mesh, Patch};
use rustc_hash::FxHashMap;

use super::{KernelError, corner_edge_flags, face_bits, live_faces, patch, region_fans, restore_edge_flags};

/// What [`split`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitResult {
    /// The detached faces (they replace the selected ones, same groups).
    pub faces: Vec<u32>,
    /// Vertices duplicated along the cut.
    pub duplicated: usize,
}

/// Detaches faces from the rest of the mesh: every vertex where selected
/// and unselected faces meet is duplicated, so the selection becomes its
/// own piece (still in the same mesh, touching in place). Faces keep their
/// groups, materials, corner attributes and edge flags.
///
/// Complexity: O(c log c + valences of the selection's vertices) for the c
/// selected corners.
pub fn split(mesh: &mut Mesh, faces: &[u32]) -> Result<SplitResult, KernelError> {
    let faces = live_faces(mesh, faces)?;
    let region = face_bits(mesh, &faces);
    let fans = region_fans(mesh, &region, &faces);
    let flags = corner_edge_flags(mesh, &faces);
    let mut p = patch(mesh);
    let mut vert_of: Vec<u32> = Vec::with_capacity(fans.corners.len());
    let mut duplicated = 0usize;
    for (i, fan) in fans.corners.iter().enumerate() {
        let v = mesh.origin(fan[0]);
        // Only vertices shared with unselected faces need a copy.
        let shared = !fans.closed[i] && mesh.vert_faces(v).any(|f| !region.contains(f as usize));
        vert_of.push(if shared {
            duplicated += 1;
            p.new_vert(mesh.positions[v as usize], Some(v))
        } else {
            v
        });
    }
    if duplicated == 0 {
        return Ok(SplitResult { faces, duplicated: 0 });
    }
    for &f in &faces {
        p.remove_face(f);
        let vs: Vec<u32> = mesh.face_halfedges(f).map(|h| vert_of[fans.of_corner[&h] as usize]).collect();
        let src: Vec<CornerSrc> = mesh.face_halfedges(f).map(CornerSrc::copy).collect();
        p.add_face(&vs, &src, mesh.face_material(f), Some(f));
    }
    let r = mesh.apply_patch(&p)?;
    restore_edge_flags(mesh, &r.faces, &flags);
    Ok(SplitResult { faces: r.faces, duplicated })
}

/// What [`subdivide`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubdivideResult {
    /// The faces the selection became.
    pub faces: Vec<u32>,
    /// Unselected neighbours that gained vertices on shared edges.
    pub neighbours: usize,
}

/// Largest number of simple subdivision levels in one call (4^6 = 4096
/// quads per quad).
pub const MAX_SUBDIVIDE_LEVELS: u32 = 6;

/// Splits each selected face into quads without moving anything: a vertex at
/// the middle of every edge and of every face, then one quad per face corner
/// (corner, edge middle, face middle, edge middle), `levels` times. Quads
/// become 4 quads, triangles 3 quads, n-gons n quads. Unselected faces next
/// to the selection get the edge-middle vertices too (they gain corners, so
/// no cracks open).
///
/// Complexity: O(4^levels x c log c) for the c corners of the selection.
pub fn subdivide(mesh: &mut Mesh, faces: &[u32], levels: u32) -> Result<SubdivideResult, KernelError> {
    let mut faces = live_faces(mesh, faces)?;
    if !(1..=MAX_SUBDIVIDE_LEVELS).contains(&levels) {
        return Err(KernelError::OutOfRange {
            param: "levels",
            message: format!("{levels} levels is outside 1 to {MAX_SUBDIVIDE_LEVELS}"),
            hint: "each level multiplies the face count by about 4".into(),
        });
    }
    let mut neighbours_total = 0usize;
    for _ in 0..levels {
        let (out, n) = subdivide_once(mesh, &faces)?;
        faces = out;
        neighbours_total = neighbours_total.max(n);
    }
    Ok(SubdivideResult { faces, neighbours: neighbours_total })
}

/// One level of [`subdivide`]; returns the new faces and the number of
/// neighbours that gained vertices.
///
/// Complexity: O(c log c + valences) for the c corners of the selection.
fn subdivide_once(mesh: &mut Mesh, faces: &[u32]) -> Result<(Vec<u32>, usize), KernelError> {
    let region = face_bits(mesh, faces);
    let mut p = patch(mesh);
    // Middle vertex of every edge of the selection, keyed by edge.
    let mut mid: FxHashMap<u32, u32> = FxHashMap::default();
    let mut neighbours: Vec<u32> = Vec::new();
    for &f in faces {
        for h in mesh.face_halfedges(f) {
            let e = h >> 1;
            mid.entry(e).or_insert_with(|| {
                let [a, b] = mesh.edge_verts(e);
                p.new_vert((mesh.positions[a as usize] + mesh.positions[b as usize]) * 0.5, None)
            });
            if let Some(g) = mesh.face(h ^ 1)
                && !region.contains(g as usize)
            {
                neighbours.push(g);
            }
        }
    }
    neighbours.sort_unstable();
    neighbours.dedup();
    let mut made = Vec::new();
    for &f in faces {
        let hs: Vec<u32> = mesh.face_halfedges(f).collect();
        let k = hs.len();
        let center = p.new_vert(mesh.face_centroid(f), None);
        let center_src = CornerSrc::lerp(hs[0], hs[k / 2], 0.5);
        p.remove_face(f);
        for i in 0..k {
            let (prev, h) = (hs[(i + k - 1) % k], hs[i]);
            let vs = [mesh.origin(h), mid[&(h >> 1)], center, mid[&(prev >> 1)]];
            let src = [CornerSrc::copy(h), CornerSrc::lerp(h, mesh.next(h), 0.5), center_src, CornerSrc::lerp(prev, h, 0.5)];
            made.push(p.add_face(&vs, &src, mesh.face_material(f), Some(f)));
        }
    }
    for &g in &neighbours {
        add_with_midpoints(mesh, &mut p, g, &mid);
    }
    // Both halves of a split edge keep its flags.
    let flagged: Vec<(u32, u32, u32, EdgeFlags)> = mid
        .iter()
        .filter(|&(&e, _)| mesh.edge_flags(e) != EdgeFlags::NONE)
        .map(|(&e, &m)| {
            let [a, b] = mesh.edge_verts(e);
            (a, b, m, mesh.edge_flags(e))
        })
        .collect();
    let r = mesh.apply_patch(&p)?;
    for (a, b, m, fl) in flagged {
        for (x, y) in [(a, m), (m, b)] {
            if let Some(h) = mesh.find_halfedge(x, y) {
                for flag in [EdgeFlags::SHARP, EdgeFlags::SEAM] {
                    if fl.contains(flag) {
                        mesh.set_edge_flag(h >> 1, flag, true);
                    }
                }
            }
        }
    }
    Ok((made.into_iter().map(|i| r.faces[i]).collect(), neighbours.len()))
}

/// Re-adds face `g` with the middle vertex of every split edge inserted
/// after the corner where that edge starts.
fn add_with_midpoints(mesh: &Mesh, p: &mut Patch, g: u32, mid: &FxHashMap<u32, u32>) {
    let mut vs = Vec::new();
    let mut src = Vec::new();
    for h in mesh.face_halfedges(g) {
        vs.push(mesh.origin(h));
        src.push(CornerSrc::copy(h));
        if let Some(&m) = mid.get(&(h >> 1)) {
            vs.push(m);
            src.push(CornerSrc::lerp(h, mesh.next(h), 0.5));
        }
    }
    p.remove_face(g);
    p.add_face(&vs, &src, mesh.face_material(g), Some(g));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    #[test]
    fn split_detaches_a_box_side() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let top: Vec<u32> = m.group("top").unwrap().iter().collect();
        let r = split(&mut m, &top).unwrap();
        m.validate().unwrap();
        assert_eq!((r.duplicated, m.vert_count(), m.face_count()), (4, 12, 6));
        assert_eq!(m.face_components().1, 2);
        assert!(m.group("top").unwrap().contains(r.faces[0]));
        // Splitting the whole mesh changes nothing.
        let all: Vec<u32> = m.faces().collect();
        let before = m.clone();
        assert_eq!(split(&mut m, &all).unwrap().duplicated, 0);
        assert_eq!(m, before);
    }

    #[test]
    fn subdivide_quads_and_neighbours() {
        let mut m = plane(&PlaneSpec { size_x: 2.0, size_z: 1.0, segments_x: 2, segments_z: 1 }).unwrap();
        let r = subdivide(&mut m, &[0], 1).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces.len(), r.neighbours), (4, 1));
        assert_eq!(m.face_count(), 5);
        // The neighbour gained the middle of the shared edge: a 5-gon.
        assert_eq!(m.faces().filter(|&f| m.face_len(f) == 5).count(), 1);
        let all: Vec<u32> = m.faces().collect();
        let r = subdivide(&mut m, &all, 2).unwrap();
        m.validate().unwrap();
        assert_eq!(r.faces.len(), 4 * 16 + 5 * 4, "quads give 16 each, the 5-gon 5 then 20");
        assert!((m.faces().map(|f| m.face_area(f)).sum::<f32>() - 2.0).abs() < 1e-4, "flat and same area");
    }

    #[test]
    fn subdivide_a_closed_box_stays_closed() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let all: Vec<u32> = m.faces().collect();
        let r = subdivide(&mut m, &all, 1).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces.len(), r.neighbours), (24, 0));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        assert!(subdivide(&mut m, &[0], 9).is_err());
    }
}
