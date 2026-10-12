//! Closing holes and reorienting connected parts.

use crate::edit::EditError;
use crate::mesh::{INVALID, Mesh};

impl Mesh {
    /// Half-edges of the boundary loop through boundary half-edge `h`, in
    /// loop order starting at `h`.
    ///
    /// Complexity: O(loop length).
    pub fn boundary_loop(&self, h: u32) -> Result<Vec<u32>, EditError> {
        if !self.is_he_alive(h) || !self.is_boundary_he(h) {
            return Err(EditError::NotBoundary(h));
        }
        let mut out = vec![h];
        let mut cur = self.next(h);
        // Boundary loops are simple cycles; the guard only protects against corrupt input.
        while cur != h && out.len() <= self.he_slots() {
            out.push(cur);
            cur = self.next(cur);
        }
        Ok(out)
    }

    /// Closes the boundary loop through boundary half-edge `h` with one new
    /// face over the loop's vertices (an n-gon). The face takes the material
    /// of the face across `h`; each corner copies UVs and colors from the
    /// neighbouring corner at the same vertex; custom normals get the new
    /// face's normal. Returns the new face.
    ///
    /// Complexity: O(loop length + groups).
    pub fn fill_boundary_loop(&mut self, h: u32) -> Result<u32, EditError> {
        let hs = self.boundary_loop(h)?;
        if hs.len() < 3 {
            return Err(EditError::LoopTooShort { edges: hs.len() });
        }
        let f = self.face_slots() as u32;
        let material = self.face(h ^ 1).map_or(0, |g| self.face_material[g as usize]);
        self.face_he.push(h);
        self.face_material.push(material);
        self.grow_groups();
        for &x in &hs {
            self.he_face[x as usize] = f;
            // The neighbouring corner at origin(x) belongs to the face across
            // x: it is the half-edge after twin(x), which starts there.
            let src = self.next(x ^ 1) as usize;
            for layer in &mut self.corner_uv {
                layer[x as usize] = layer[src];
            }
            if let Some(c) = &mut self.corner_color {
                c[x as usize] = c[src];
            }
        }
        if self.corner_normal.is_some() {
            let n = self.face_normal(f);
            if let Some(col) = &mut self.corner_normal {
                for &x in &hs {
                    col[x as usize] = n;
                }
            }
        }
        // Each vertex had exactly one boundary fan (this loop), so none of
        // them is on the boundary any more and any outgoing half-edge will do.
        self.revision += 1;
        Ok(f)
    }

    /// Faces of the connected part containing face `f`, in ascending order.
    ///
    /// Complexity: O(part size log part size).
    pub fn part_faces(&self, f: u32) -> Result<Vec<u32>, EditError> {
        if !self.is_face_alive(f) {
            return Err(EditError::NoSuchFace(f));
        }
        let mut seen = vec![f];
        let mut stack = vec![f];
        let mut mark = rustc_hash::FxHashSet::default();
        mark.insert(f);
        while let Some(g) = stack.pop() {
            for h in self.face_halfedges(g) {
                if let Some(n) = self.face(h ^ 1)
                    && mark.insert(n)
                {
                    seen.push(n);
                    stack.push(n);
                }
            }
        }
        seen.sort_unstable();
        Ok(seen)
    }

    /// Reverses the winding of the connected part containing face `f` (and
    /// of its boundary loops), like [`Mesh::flip_all`] restricted to that
    /// part. Corner data moves with its corner. Returns the number of faces
    /// flipped.
    ///
    /// Complexity: O(c log c) for the c corners of the part.
    pub fn flip_part(&mut self, f: u32) -> Result<usize, EditError> {
        let faces = self.part_faces(f)?;
        // Every half-edge of the part and its twin (boundary half-edges included):
        // a set closed under twin, so the rewiring below stays inside it.
        let mut hs: Vec<u32> = Vec::new();
        for &g in &faces {
            for h in self.face_halfedges(g) {
                hs.push(h);
                hs.push(h ^ 1);
            }
        }
        hs.sort_unstable();
        hs.dedup();
        // New links: the half-edge h ^ 1 takes over h's face, next(h ^ 1) = prev(h) ^ 1.
        let new: Vec<(u32, u32, u32)> =
            hs.iter().map(|&h| (self.he_prev[h as usize] ^ 1, self.he_next[h as usize] ^ 1, self.he_face[h as usize])).collect();
        // Corners: the corner on h moves to prev(h) ^ 1, which starts at the same vertex.
        let corner_dest: Vec<(u32, u32)> =
            hs.iter().filter(|&&h| self.he_face[h as usize] != INVALID).map(|&h| (h, self.he_prev[h as usize] ^ 1)).collect();
        for layer in &mut self.corner_uv {
            let old: Vec<_> = corner_dest.iter().map(|&(h, _)| layer[h as usize]).collect();
            for (&(_, d), v) in corner_dest.iter().zip(old) {
                layer[d as usize] = v;
            }
        }
        if let Some(col) = &mut self.corner_normal {
            let old: Vec<_> = corner_dest.iter().map(|&(h, _)| col[h as usize]).collect();
            for (&(_, d), v) in corner_dest.iter().zip(old) {
                col[d as usize] = v;
            }
        }
        if let Some(col) = &mut self.corner_color {
            let old: Vec<_> = corner_dest.iter().map(|&(h, _)| col[h as usize]).collect();
            for (&(_, d), v) in corner_dest.iter().zip(old) {
                col[d as usize] = v;
            }
        }
        for &g in &faces {
            // Keep the same first vertex: its corner moved to prev(h) ^ 1.
            let h = self.face_he[g as usize];
            self.face_he[g as usize] = self.he_prev[h as usize] ^ 1;
        }
        for (&h, &(next, prev, face)) in hs.iter().zip(&new) {
            let t = (h ^ 1) as usize;
            self.he_next[t] = next;
            self.he_prev[t] = prev;
            self.he_face[t] = face;
        }
        for &h in &hs {
            if self.he_face[h as usize] == INVALID {
                self.vert_he[self.he_vert[h as usize] as usize] = h;
            }
        }
        self.revision += 1;
        Ok(faces.len())
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};

    use crate::{Mesh, TriOptions};

    fn cube(offset: Vec3) -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) + offset).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    fn signed_volume(m: &Mesh) -> f32 {
        m.faces()
            .map(|f| {
                let v: Vec<Vec3> = m.face_verts(f).map(|v| m.positions[v as usize]).collect();
                (1..v.len() - 1).map(|i| v[0].dot(v[i].cross(v[i + 1])) / 6.0).sum::<f32>()
            })
            .sum()
    }

    #[test]
    fn fill_closes_a_hole_outward() {
        let mut m = cube(Vec3::ZERO);
        m.add_uv_layer().unwrap();
        m.face_material[3] = 2;
        m.delete_faces(&[3]).unwrap();
        let h = (0..m.he_slots() as u32).find(|&h| m.is_he_alive(h) && m.is_boundary_he(h)).unwrap();
        assert_eq!(m.boundary_loop(h).unwrap().len(), 4);
        let f = m.fill_boundary_loop(h).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 6);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        assert!(m.face_normal(f).y > 0.99, "the filled top faces up: {}", m.face_normal(f));
        assert!((signed_volume(&m) - 1.0).abs() < 1e-6);
        assert_eq!(m.face_material[f as usize], 0, "material of the face across the first edge");
        assert!(m.fill_boundary_loop(h).is_err(), "no longer a boundary");
    }

    #[test]
    fn fill_rejects_interior_half_edges() {
        let mut m = cube(Vec3::ZERO);
        assert!(m.fill_boundary_loop(0).is_err());
        assert!(m.boundary_loop(0).is_err());
    }

    #[test]
    fn flip_part_flips_only_that_part() {
        let mut m = cube(Vec3::ZERO);
        m.append(&cube(Vec3::new(3.0, 0.0, 0.0)));
        m.delete_faces(&[1]).unwrap();
        let uvs: Vec<Vec2> = (0..m.he_slots()).map(|i| Vec2::new(i as f32, 0.0)).collect();
        m.corner_uv.push(uvs);
        let uv_at = |m: &Mesh, f: u32| -> Vec<(u32, Vec2)> {
            let mut v: Vec<_> = m.face_halfedges(f).map(|h| (m.origin(h), m.corner_uv(0, h).unwrap())).collect();
            v.sort_by_key(|x| x.0);
            v
        };
        let before: Vec<_> = m.faces().map(|f| (m.face_normal(f), uv_at(&m, f))).collect();
        assert_eq!(m.part_faces(0).unwrap(), vec![0, 2, 3, 4, 5]);
        assert_eq!(m.flip_part(0).unwrap(), 5);
        m.validate().unwrap();
        for (i, f) in m.faces().enumerate() {
            let (n, uv) = &before[i];
            let flipped = f < 6;
            assert!(m.face_normal(f).dot(*n) * if flipped { -1.0 } else { 1.0 } > 0.99, "face {f}");
            assert_eq!(&uv_at(&m, f), uv, "corner data stays on its face and vertex");
        }
        m.flip_part(0).unwrap();
        m.validate().unwrap();
        assert_eq!(m.faces().map(|f| m.face_normal(f)).collect::<Vec<_>>(), before.iter().map(|b| b.0).collect::<Vec<_>>());
        let _ = m.to_tri_buffers(&TriOptions::default());
    }
}
