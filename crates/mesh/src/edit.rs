//! Local topology edits that keep the mesh 2-manifold, and compaction.

use crate::attrs::EdgeFlags;
use crate::groups::ElementKind;
use crate::mesh::{INVALID, Mesh};

/// Why an edit was refused. Nothing is changed when an edit fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("face {0} does not exist")]
    NoSuchFace(u32),
    #[error("half-edge {0} is not on the boundary")]
    NotBoundary(u32),
    #[error("a boundary loop of {edges} edges cannot become a face")]
    LoopTooShort { edges: usize },
    #[error("new face {face} is malformed: {reason}")]
    BadFace { face: usize, reason: String },
    #[error("vertex {0} does not exist or the new vertex position is not finite")]
    BadVertex(u32),
    #[error("edge {a}-{b} is already used in this direction (the result would be non-manifold or inconsistently wound)")]
    EdgeInUse { a: u32, b: u32 },
    #[error("the mesh changed after the patch was made")]
    StalePatch,
}

/// What [`Mesh::delete_faces`] removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteReport {
    pub faces: usize,
    /// Edges left without any face.
    pub edges: usize,
    /// Vertices left without any edge.
    pub verts: usize,
    /// Vertices added to keep the mesh manifold where the remaining faces
    /// around one vertex fell apart into several fans.
    pub split_verts: usize,
}

/// Old-to-new index maps returned by [`Mesh::compact`]; dead elements map
/// to [`INVALID`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remap {
    pub verts: Vec<u32>,
    pub edges: Vec<u32>,
    pub faces: Vec<u32>,
}

impl Remap {
    /// New index of an old half-edge.
    #[inline]
    pub fn halfedge(&self, h: u32) -> u32 {
        match self.edges.get((h >> 1) as usize) {
            Some(&e) if e != INVALID => 2 * e + (h & 1),
            _ => INVALID,
        }
    }
}

impl Mesh {
    /// Deletes faces. Edges left without faces and vertices left without
    /// edges are deleted too. Where the remaining faces around a vertex form
    /// several separate fans, the vertex is split (one copy per extra fan,
    /// same position and vertex groups) so the mesh stays manifold.
    ///
    /// Deleted slots stay until [`Mesh::compact`].
    ///
    /// Complexity: O(c log c) for the c corners of the deleted faces, plus the
    /// valences of their vertices, plus groups x affected elements. Independent
    /// of the mesh size.
    pub fn delete_faces(&mut self, faces: &[u32]) -> Result<DeleteReport, EditError> {
        if let Some(&f) = faces.iter().find(|&&f| !self.is_face_alive(f)) {
            return Err(EditError::NoSuchFace(f));
        }
        // Sort and dedupe instead of a mesh-sized bitset, so deleting a few faces
        // costs O(k log k), not O(mesh size).
        let mut list = faces.to_vec();
        list.sort_unstable();
        list.dedup();
        if list.is_empty() {
            return Ok(DeleteReport::default());
        }

        // Affected vertices and their outgoing half-edges in rotation order,
        // read before anything changes.
        let mut verts = Vec::new();
        let mut hes = Vec::new();
        for &f in &list {
            for h in self.face_halfedges(f) {
                hes.push(h);
                verts.push(self.origin(h));
            }
        }
        verts.sort_unstable();
        verts.dedup();
        let mut ring = Vec::new();
        let mut ring_start = Vec::with_capacity(verts.len() + 1);
        for &v in &verts {
            ring_start.push(ring.len());
            ring.extend(self.vert_outgoing(v));
        }
        ring_start.push(ring.len());

        let mut report = DeleteReport { faces: list.len(), ..DeleteReport::default() };

        // Faces.
        for &h in &hes {
            self.he_face[h as usize] = INVALID;
        }
        for &f in &list {
            self.face_he[f as usize] = INVALID;
            self.free_faces.push(f);
            self.ungroup(ElementKind::Face, f);
        }

        // Edges without faces.
        for &h in &hes {
            let e = h >> 1;
            let (h0, h1) = (2 * e as usize, 2 * e as usize + 1);
            if self.he_vert[h0] != INVALID && self.he_face[h0] == INVALID && self.he_face[h1] == INVALID {
                for x in [h0, h1] {
                    self.he_vert[x] = INVALID;
                    self.he_next[x] = INVALID;
                    self.he_prev[x] = INVALID;
                }
                self.edge_flags[e as usize] = EdgeFlags::NONE;
                if let Some(c) = &mut self.edge_crease {
                    c[e as usize] = 0.0;
                }
                self.free_edges.push(e);
                self.ungroup(ElementKind::Edge, e);
                report.edges += 1;
            }
        }

        // Vertices: split into fans, relink boundary loops.
        let mut present: Vec<bool> = Vec::new();
        for (i, &v) in verts.iter().enumerate() {
            let fan = &ring[ring_start[i]..ring_start[i + 1]];
            let k = fan.len();
            // Sector j is the face between fan[j] and fan[j + 1]: face(fan[j]).
            present.clear();
            present.extend(fan.iter().map(|&h| self.he_face[h as usize] != INVALID));
            let Some(first) = (0..k).find(|&j| present[j] && !present[(j + k - 1) % k]) else {
                if !present.iter().any(|&p| p) {
                    self.vert_he[v as usize] = INVALID;
                    self.free_verts.push(v);
                    self.ungroup(ElementKind::Vert, v);
                    report.verts += 1;
                }
                continue;
            };
            let mut s = first;
            let mut run = 0usize;
            loop {
                // Run of present sectors s..=e; its outgoing half-edges are fan[s..=e + 1].
                let mut e = s;
                while present[(e + 1) % k] {
                    e += 1;
                }
                let target = if run == 0 { v } else { self.split_vertex(v) };
                if target != v {
                    for j in s..=e + 1 {
                        self.he_vert[fan[j % k] as usize] = target;
                    }
                    report.split_verts += 1;
                }
                let in_b = fan[s % k] ^ 1;
                let out_b = fan[(e + 1) % k];
                self.he_next[in_b as usize] = out_b;
                self.he_prev[out_b as usize] = in_b;
                self.vert_he[target as usize] = out_b;
                run += 1;
                // Next run start: the next present sector after the gap.
                let mut next = e + 2;
                while next < first + k && !present[next % k] {
                    next += 1;
                }
                if next >= first + k {
                    break;
                }
                s = next;
            }
        }
        self.revision += 1;
        Ok(report)
    }

    /// Adds a vertex at the position of `v`, in the same vertex groups, with
    /// no edges yet.
    ///
    /// Complexity: O(groups).
    fn split_vertex(&mut self, v: u32) -> u32 {
        let n = self.vert_slots() as u32;
        self.positions.push(self.positions[v as usize]);
        self.vert_he.push(INVALID);
        self.grow_groups();
        for g in self.groups.values_mut() {
            if g.kind() == ElementKind::Vert && g.contains(v) {
                g.insert(n);
            }
        }
        n
    }

    /// Removes dead slots, renumbering live elements densely in their old
    /// order. Returns the old-to-new maps.
    ///
    /// Complexity: O(V + E + F + C) plus O(slots) per group.
    pub fn compact(&mut self) -> Remap {
        fn dense(alive: impl Iterator<Item = bool>) -> Vec<u32> {
            let mut next = 0u32;
            alive
                .map(|a| {
                    if a {
                        next += 1;
                        next - 1
                    } else {
                        INVALID
                    }
                })
                .collect()
        }
        let remap = Remap {
            verts: dense(self.vert_he.iter().map(|&h| h != INVALID)),
            edges: dense((0..self.edge_slots()).map(|e| self.he_vert[2 * e] != INVALID)),
            faces: dense(self.face_he.iter().map(|&h| h != INVALID)),
        };
        let map_v = |v: u32| if v == INVALID { INVALID } else { remap.verts[v as usize] };
        let map_f = |f: u32| if f == INVALID { INVALID } else { remap.faces[f as usize] };
        let map_h = |h: u32| if h == INVALID { INVALID } else { remap.halfedge(h) };

        let live_h: Vec<usize> = (0..self.he_slots()).filter(|&h| self.he_vert[h] != INVALID).collect();
        let live_e: Vec<usize> = (0..self.edge_slots()).filter(|&e| remap.edges[e] != INVALID).collect();
        let live_v: Vec<usize> = (0..self.vert_slots()).filter(|&v| remap.verts[v] != INVALID).collect();
        let live_f: Vec<usize> = (0..self.face_slots()).filter(|&f| remap.faces[f] != INVALID).collect();

        self.he_next = live_h.iter().map(|&h| map_h(self.he_next[h])).collect();
        self.he_prev = live_h.iter().map(|&h| map_h(self.he_prev[h])).collect();
        self.he_face = live_h.iter().map(|&h| map_f(self.he_face[h])).collect();
        self.he_vert = live_h.iter().map(|&h| map_v(self.he_vert[h])).collect();
        self.vert_he = live_v.iter().map(|&v| map_h(self.vert_he[v])).collect();
        self.face_he = live_f.iter().map(|&f| map_h(self.face_he[f])).collect();
        self.positions = live_v.iter().map(|&v| self.positions[v]).collect();
        for layer in &mut self.corner_uv {
            *layer = live_h.iter().map(|&h| layer[h]).collect();
        }
        if let Some(n) = &mut self.corner_normal {
            *n = live_h.iter().map(|&h| n[h]).collect();
        }
        if let Some(c) = &mut self.corner_color {
            *c = live_h.iter().map(|&h| c[h]).collect();
        }
        self.face_material = live_f.iter().map(|&f| self.face_material[f]).collect();
        self.edge_flags = live_e.iter().map(|&e| self.edge_flags[e]).collect();
        if let Some(c) = &mut self.edge_crease {
            *c = live_e.iter().map(|&e| c[e]).collect();
        }
        let (nv, ne, nf) = (live_v.len(), live_e.len(), live_f.len());
        for g in self.groups.values_mut() {
            let (map, n) = match g.kind() {
                ElementKind::Vert => (&remap.verts, nv),
                ElementKind::Edge => (&remap.edges, ne),
                ElementKind::Face => (&remap.faces, nf),
            };
            g.remap(map, n);
        }
        self.free_verts.clear();
        self.free_edges.clear();
        self.free_faces.clear();
        self.revision += 1;
        remap
    }

    /// Grows every group's bitset to the current slot counts.
    ///
    /// Complexity: O(groups).
    pub(crate) fn grow_groups(&mut self) {
        let (nv, ne, nf) = (self.vert_slots(), self.edge_slots(), self.face_slots());
        for g in self.groups.values_mut() {
            g.grow(match g.kind() {
                ElementKind::Vert => nv,
                ElementKind::Edge => ne,
                ElementKind::Face => nf,
            });
        }
    }

    /// Clears one element from every group of its kind.
    ///
    /// Complexity: O(groups).
    pub(crate) fn ungroup(&mut self, kind: ElementKind, i: u32) {
        for g in self.groups.values_mut() {
            if g.kind() == kind {
                g.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

    /// n x n grid of unit quads in the XY plane, vertices row by row.
    fn grid(n: u32) -> Mesh {
        let w = n + 1;
        let p = (0..w * w).map(|i| Vec3::new((i % w) as f32, (i / w) as f32, 0.0)).collect();
        let mut corners = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * w + x;
                corners.extend([a, a + 1, a + w + 1, a + w]);
            }
        }
        Mesh::from_polygons(p, &vec![4; (n * n) as usize], &corners).unwrap()
    }

    fn cube() -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    #[test]
    fn open_a_cube() {
        let mut m = cube();
        let r = m.delete_faces(&[0]).unwrap();
        assert_eq!(r, DeleteReport { faces: 1, ..DeleteReport::default() });
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.edge_count(), m.face_count()), (8, 12, 5));
        assert_eq!(m.edges().filter(|&e| m.is_boundary_edge(e)).count(), 4);
        // Opposite face too: a tube.
        m.delete_faces(&[1]).unwrap();
        m.validate().unwrap();
        assert_eq!(m.edges().filter(|&e| m.is_boundary_edge(e)).count(), 8);
    }

    #[test]
    fn delete_everything() {
        let mut m = cube();
        let all: Vec<u32> = m.faces().collect();
        let r = m.delete_faces(&all).unwrap();
        assert_eq!((r.faces, r.edges, r.verts), (6, 12, 8));
        m.validate().unwrap();
        m.compact();
        assert_eq!((m.vert_slots(), m.edge_slots(), m.face_slots()), (0, 0, 0));
        m.validate().unwrap();
    }

    #[test]
    fn diagonal_deletion_splits_the_center_vertex() {
        let mut m = grid(2);
        m.set_group("center", ElementKind::Vert, [4]).unwrap();
        // Quads 0 (bottom left) and 3 (top right) leave two quads touching at vertex 4.
        let r = m.delete_faces(&[0, 3]).unwrap();
        assert_eq!(r.split_verts, 1);
        m.validate().unwrap();
        assert_eq!(m.vert_count(), 7 + 1);
        assert_eq!(m.group("center").unwrap().count(), 2, "the copy joins the vertex groups");
        let copy = m.vert_slots() as u32 - 1;
        assert_eq!(m.positions[copy as usize], m.positions[4]);
    }

    #[test]
    fn errors_change_nothing() {
        let mut m = cube();
        let before = m.clone();
        assert_eq!(m.delete_faces(&[0, 42]), Err(EditError::NoSuchFace(42)));
        assert_eq!(m, before);
        m.delete_faces(&[0]).unwrap();
        assert_eq!(m.delete_faces(&[0]), Err(EditError::NoSuchFace(0)), "already deleted");
    }

    #[test]
    fn compact_preserves_live_data() {
        let mut m = grid(3);
        m.add_uv_layer_from_polygon_order(&(0..36).map(|i| crate::Vec2::splat(i as f32)).collect::<Vec<_>>()).unwrap();
        m.face_material[8] = 3;
        m.set_group("right", ElementKind::Face, [2, 5, 8]).unwrap();
        m.set_group("rim", ElementKind::Edge, m.edges().filter(|&e| m.is_boundary_edge(e)).collect::<Vec<_>>()).unwrap();
        m.delete_faces(&[0, 4]).unwrap();
        let before_tris = m.triangle_count();
        let corner_uvs: Vec<(u32, crate::Vec2)> =
            m.faces().flat_map(|f| m.face_halfedges(f).map(|h| (m.origin(h), m.corner_uv(0, h).unwrap())).collect::<Vec<_>>()).collect();
        let pos: Vec<Vec3> = m.verts().map(|v| m.positions[v as usize]).collect();

        let remap = m.compact();
        m.validate().unwrap();
        assert!(!m.has_holes_in_storage());
        assert_eq!(m.face_count(), 7);
        assert_eq!(m.triangle_count(), before_tris);
        assert_eq!(m.positions, pos);
        let after: Vec<(u32, crate::Vec2)> =
            m.faces().flat_map(|f| m.face_halfedges(f).map(|h| (m.origin(h), m.corner_uv(0, h).unwrap())).collect::<Vec<_>>()).collect();
        let mapped: Vec<(u32, crate::Vec2)> = corner_uvs.iter().map(|&(v, uv)| (remap.verts[v as usize], uv)).collect();
        assert_eq!(after, mapped);
        assert_eq!(m.face_material[remap.faces[8] as usize], 3);
        assert_eq!(m.group("right").unwrap().iter().collect::<Vec<_>>(), vec![remap.faces[2], remap.faces[5], remap.faces[8]]);
        assert_eq!(remap.faces[0], INVALID);
    }

    #[test]
    fn random_deletions_stay_valid() {
        // Deterministic pseudo-random order (LCG), validating after each step.
        let mut m = grid(8);
        let mut state = 12345u64;
        while m.face_count() > 0 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let live: Vec<u32> = m.faces().collect();
            let f = live[(state >> 33) as usize % live.len()];
            m.delete_faces(&[f]).unwrap();
            m.validate().unwrap();
            if state.is_multiple_of(5) {
                m.compact();
                m.validate().unwrap();
            }
        }
        assert_eq!((m.vert_count(), m.edge_count()), (0, 0));
    }
}
