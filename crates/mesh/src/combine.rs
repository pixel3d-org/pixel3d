//! Whole-mesh transforms, orientation flips, appending and splitting.

use fixedbitset::FixedBitSet;
use glam::{Affine3A, Vec2, Vec3, Vec4};

use crate::groups::{ElementKind, Group};
use crate::mesh::{INVALID, Mesh};

impl Mesh {
    /// Transforms positions (and custom normals by the inverse transpose).
    /// A mirroring transform (negative determinant) also flips every face so
    /// normals keep pointing outward.
    ///
    /// Complexity: O(V + H).
    pub fn transform(&mut self, m: Affine3A) {
        for v in 0..self.vert_slots() {
            if self.vert_he[v] != INVALID {
                self.positions[v] = m.transform_point3(self.positions[v]);
            }
        }
        let normal_m = m.matrix3.inverse().transpose();
        if let Some(n) = &mut self.corner_normal {
            for x in n.iter_mut() {
                *x = (normal_m * *x).normalize_or_zero();
            }
        }
        if m.matrix3.determinant() < 0.0 {
            self.flip_all();
        }
        self.revision += 1;
    }

    /// Reverses the winding of every face (and boundary loop). Corner data
    /// moves with its corner, so UVs, normals and colors stay on the same
    /// face and vertex.
    ///
    /// Complexity: O(V + H + F).
    pub fn flip_all(&mut self) {
        let nh = self.he_slots();
        // Face loop h0 -> h1 -> h2 becomes twin(h2) -> twin(h1) -> twin(h0):
        // the half-edge h ^ 1 takes over h's face, and next(h ^ 1) = prev(h) ^ 1.
        let mut next = vec![INVALID; nh];
        let mut prev = vec![INVALID; nh];
        let mut face = vec![INVALID; nh];
        for h in 0..nh as u32 {
            if self.he_vert[h as usize] == INVALID {
                continue;
            }
            let t = (h ^ 1) as usize;
            face[t] = self.he_face[h as usize];
            next[t] = self.he_prev[h as usize] ^ 1;
            prev[t] = self.he_next[h as usize] ^ 1;
        }
        // The corner of face f at vertex a was on h (a -> b, in f); in the
        // flipped face it is prev(h) ^ 1 (a -> c), which starts at a too.
        let moved = |col: &mut Vec<Vec2>| {
            let old = col.clone();
            for h in 0..nh {
                if self.he_vert[h] != INVALID && self.he_face[h] != INVALID {
                    col[(self.he_prev[h] ^ 1) as usize] = old[h];
                }
            }
        };
        for layer in &mut self.corner_uv {
            moved(layer);
        }
        let move3 = |col: &mut Vec<Vec3>| {
            let old = col.clone();
            for h in 0..nh {
                if self.he_vert[h] != INVALID && self.he_face[h] != INVALID {
                    col[(self.he_prev[h] ^ 1) as usize] = old[h];
                }
            }
        };
        if let Some(n) = &mut self.corner_normal {
            move3(n);
        }
        let move4 = |col: &mut Vec<Vec4>| {
            let old = col.clone();
            for h in 0..nh {
                if self.he_vert[h] != INVALID && self.he_face[h] != INVALID {
                    col[(self.he_prev[h] ^ 1) as usize] = old[h];
                }
            }
        };
        if let Some(c) = &mut self.corner_color {
            move4(c);
        }
        for f in 0..self.face_slots() {
            let h = self.face_he[f];
            if h != INVALID {
                // Keep the same first vertex: its corner moved to prev(h) ^ 1.
                self.face_he[f] = self.he_prev[h as usize] ^ 1;
            }
        }
        self.he_next = next;
        self.he_prev = prev;
        self.he_face = face;
        for h in 0..nh as u32 {
            if self.he_vert[h as usize] != INVALID && self.he_face[h as usize] == INVALID {
                self.vert_he[self.he_vert[h as usize] as usize] = h;
            }
        }
        self.revision += 1;
    }

    /// Appends another mesh as a separate part. UV layers are padded with
    /// zeros, custom normals are filled with computed ones, colors with white.
    /// Groups with the same name and kind are merged; a same-named group of
    /// another kind is renamed with a `_1`, `_2` suffix. Returns the offsets
    /// of the appended vertices, edges and faces.
    ///
    /// Complexity: O(size of `other`) plus amortized O(groups x slots / 64) to
    /// grow group bitsets; group merging is O(members of `other`).
    pub fn append(&mut self, other: &Mesh) -> [u32; 3] {
        // Shading of this mesh, needed only if the other one brings custom normals.
        let own_normals = (self.corner_normal.is_none() && other.corner_normal.is_some()).then(|| self.compute_corner_normals());
        let (vo, eo, fo) = (self.vert_slots() as u32, self.edge_slots() as u32, self.face_slots() as u32);
        let ho = 2 * eo;
        let off = |x: u32, o: u32| if x == INVALID { INVALID } else { x + o };
        self.he_next.extend(other.he_next.iter().map(|&h| off(h, ho)));
        self.he_prev.extend(other.he_prev.iter().map(|&h| off(h, ho)));
        self.he_vert.extend(other.he_vert.iter().map(|&v| off(v, vo)));
        self.he_face.extend(other.he_face.iter().map(|&f| off(f, fo)));
        self.vert_he.extend(other.vert_he.iter().map(|&h| off(h, ho)));
        self.face_he.extend(other.face_he.iter().map(|&h| off(h, ho)));
        self.positions.extend_from_slice(&other.positions);
        self.free_verts.extend(other.free_verts.iter().map(|&v| v + vo));
        self.free_edges.extend(other.free_edges.iter().map(|&e| e + eo));
        self.free_faces.extend(other.free_faces.iter().map(|&f| f + fo));

        let nh_other = other.he_slots();
        let layers = self.corner_uv.len().max(other.corner_uv.len());
        while self.corner_uv.len() < layers {
            self.corner_uv.push(vec![Vec2::ZERO; ho as usize]);
        }
        for (l, layer) in self.corner_uv.iter_mut().enumerate() {
            match other.corner_uv.get(l) {
                Some(src) => layer.extend_from_slice(src),
                None => layer.extend(std::iter::repeat_n(Vec2::ZERO, nh_other)),
            }
        }
        if self.corner_normal.is_some() || other.corner_normal.is_some() {
            let mut mine = self.corner_normal.take().or(own_normals).unwrap_or_else(|| vec![Vec3::ZERO; ho as usize]);
            mine.extend(other.compute_corner_normals());
            self.corner_normal = Some(mine);
        }
        if self.corner_color.is_some() || other.corner_color.is_some() {
            let mut mine = self.corner_color.take().unwrap_or_else(|| vec![Vec4::ONE; ho as usize]);
            match &other.corner_color {
                Some(c) => mine.extend_from_slice(c),
                None => mine.extend(std::iter::repeat_n(Vec4::ONE, nh_other)),
            }
            self.corner_color = Some(mine);
        }
        self.face_material.extend_from_slice(&other.face_material);
        self.edge_flags.extend_from_slice(&other.edge_flags);
        if self.edge_crease.is_some() || other.edge_crease.is_some() {
            let mut mine = self.edge_crease.take().unwrap_or_else(|| vec![0.0; eo as usize]);
            match &other.edge_crease {
                Some(c) => mine.extend_from_slice(c),
                None => mine.extend(std::iter::repeat_n(0.0, other.edge_slots())),
            }
            self.edge_crease = Some(mine);
        }

        self.grow_groups();
        for (name, g) in other.groups() {
            let offset = match g.kind() {
                ElementKind::Vert => vo,
                ElementKind::Edge => eo,
                ElementKind::Face => fo,
            };
            let mut target = name.to_owned();
            let mut i = 1;
            while self.group(&target).is_some_and(|x| x.kind() != g.kind()) {
                target = format!("{name}_{i}");
                i += 1;
            }
            // Merge in place: O(members of the other group), not O(receiver size),
            // so joining many parts one by one stays linear.
            let slots = self.slots(g.kind());
            let merged = self.groups.entry(target).or_insert_with(|| Group::from_bits(g.kind(), FixedBitSet::with_capacity(slots)));
            merged.grow(slots);
            for x in g.iter() {
                merged.insert(x + offset);
            }
        }
        self.revision += 1;
        [vo, eo, fo]
    }

    /// Splits the mesh into parts by a label per face slot (`INVALID` drops
    /// the face). Part `i` holds the faces labeled `i`, in face order.
    /// Wherever faces around a vertex fall into different parts, or into
    /// separate fans of one part, the vertex is duplicated, so every part is
    /// manifold. Corner data, materials, edge flags, creases and groups are
    /// carried over.
    ///
    /// Complexity: O(V + H + F + parts x groups) plus O(groups x members).
    pub fn split_by_face_labels(&self, labels: &[u32]) -> Vec<Mesh> {
        let parts = self.faces().filter_map(|f| labels.get(f as usize).copied().filter(|&l| l != INVALID)).max().map_or(0, |m| m as usize + 1);
        let label = |h: u32| -> u32 { self.face(h).and_then(|f| labels.get(f as usize).copied()).unwrap_or(INVALID) };
        // Local vertex of every corner, and each part's vertices.
        let mut corner_vert = vec![INVALID; self.he_slots()];
        let mut part_pos: Vec<Vec<Vec3>> = vec![Vec::new(); parts];
        let mut part_src: Vec<Vec<u32>> = vec![Vec::new(); parts];
        let mut fan: Vec<u32> = Vec::new();
        for v in self.verts() {
            fan.clear();
            fan.extend(self.vert_outgoing(v));
            let k = fan.len();
            let lab: Vec<u32> = fan.iter().map(|&h| label(h)).collect();
            // Sectors j and j + 1 share the edge of fan[j + 1]; a run continues
            // while the label stays the same and is not INVALID.
            let start = (0..k).find(|&j| lab[j] != lab[(j + k - 1) % k]).unwrap_or(0);
            let mut j = start;
            while j < start + k {
                let l = lab[j % k];
                let run_start = j;
                while j < start + k && lab[j % k] == l {
                    j += 1;
                }
                if l == INVALID {
                    continue;
                }
                let p = l as usize;
                let id = part_pos[p].len() as u32;
                part_pos[p].push(self.positions[v as usize]);
                part_src[p].push(v);
                for i in run_start..j {
                    corner_vert[fan[i % k] as usize] = id;
                }
            }
        }

        let mut part_faces: Vec<Vec<u32>> = vec![Vec::new(); parts];
        for f in self.faces() {
            if let Some(&l) = labels.get(f as usize)
                && l != INVALID
            {
                part_faces[l as usize].push(f);
            }
        }

        let mut out = Vec::with_capacity(parts);
        for p in 0..parts as u32 {
            let faces = std::mem::take(&mut part_faces[p as usize]);
            let corners_old: Vec<u32> = faces.iter().flat_map(|&f| self.face_halfedges(f)).collect();
            let sizes: Vec<u32> = faces.iter().map(|&f| self.face_len(f) as u32).collect();
            let corners: Vec<u32> = corners_old.iter().map(|&h| corner_vert[h as usize]).collect();
            let positions = std::mem::take(&mut part_pos[p as usize]);
            // Built from runs of a manifold mesh, so the part is manifold.
            let Ok(mut m) = Mesh::from_polygons(positions, &sizes, &corners) else {
                out.push(Mesh::new());
                continue;
            };
            // New corner h_new (in polygon order) corresponds to corners_old[i].
            let new_corners: Vec<u32> = m.faces().flat_map(|f| m.face_halfedges(f).collect::<Vec<_>>()).collect();
            for layer in &self.corner_uv {
                let mut col = vec![Vec2::ZERO; m.he_slots()];
                for (n, o) in new_corners.iter().zip(&corners_old) {
                    col[*n as usize] = layer[*o as usize];
                }
                m.corner_uv.push(col);
            }
            if let Some(src) = &self.corner_normal {
                let mut col = vec![Vec3::ZERO; m.he_slots()];
                for (n, o) in new_corners.iter().zip(&corners_old) {
                    col[*n as usize] = src[*o as usize];
                }
                m.corner_normal = Some(col);
            }
            if let Some(src) = &self.corner_color {
                let mut col = vec![Vec4::ONE; m.he_slots()];
                for (n, o) in new_corners.iter().zip(&corners_old) {
                    col[*n as usize] = src[*o as usize];
                }
                m.corner_color = Some(col);
            }
            for (i, &f) in faces.iter().enumerate() {
                m.face_material[i] = self.face_material[f as usize];
            }
            let mut edge_src = vec![INVALID; m.edge_slots()];
            for (n, o) in new_corners.iter().zip(&corners_old) {
                edge_src[(n >> 1) as usize] = o >> 1;
                m.edge_flags[(n >> 1) as usize] = self.edge_flags[(o >> 1) as usize];
            }
            if let Some(src) = &self.edge_crease {
                m.edge_crease = Some(edge_src.iter().map(|&e| if e == INVALID { 0.0 } else { src[e as usize] }).collect());
            }
            // Groups: faces by order, vertices by source vertex, edges by
            // source edge; each walks only this part's elements.
            let vert_src = &part_src[p as usize];
            for (name, g) in self.groups() {
                let members: Vec<u32> = match g.kind() {
                    ElementKind::Face => (0..faces.len() as u32).filter(|&i| g.contains(faces[i as usize])).collect(),
                    ElementKind::Vert => (0..vert_src.len() as u32).filter(|&i| g.contains(vert_src[i as usize])).collect(),
                    ElementKind::Edge => {
                        (0..edge_src.len() as u32).filter(|&e| edge_src[e as usize] != INVALID && g.contains(edge_src[e as usize])).collect()
                    }
                };
                if !members.is_empty() {
                    let _ = m.set_group(name, g.kind(), members);
                }
            }
            out.push(m);
        }
        out
    }

    /// Connected parts: faces joined across edges get the same label,
    /// numbered in order of their lowest face index. Dead faces get `INVALID`.
    ///
    /// Complexity: O(F + H).
    pub fn face_components(&self) -> (Vec<u32>, u32) {
        let mut label = vec![INVALID; self.face_slots()];
        let mut next = 0u32;
        let mut stack = Vec::new();
        for f in self.faces() {
            if label[f as usize] != INVALID {
                continue;
            }
            label[f as usize] = next;
            stack.push(f);
            while let Some(g) = stack.pop() {
                for h in self.face_halfedges(g) {
                    if let Some(n) = self.face(h ^ 1)
                        && label[n as usize] == INVALID
                    {
                        label[n as usize] = next;
                        stack.push(n);
                    }
                }
            }
            next += 1;
        }
        (label, next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TriOptions;

    fn cube(offset: Vec3) -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) + offset).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    fn outward(m: &Mesh) -> bool {
        let c = m.bounds().map(|(a, b)| (a + b) / 2.0).unwrap_or_default();
        m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0)
    }

    #[test]
    fn mirror_transform_keeps_normals_outward() {
        let mut m = cube(Vec3::ZERO);
        let uvs: Vec<Vec2> = (0..24).map(|i| Vec2::new(i as f32, 0.0)).collect();
        m.add_uv_layer_from_polygon_order(&uvs).unwrap();
        let corner_uv_by_face_vert = |m: &Mesh| -> Vec<(u32, u32, Vec2)> {
            let mut v: Vec<_> =
                m.faces().flat_map(|f| m.face_halfedges(f).map(move |h| (f, m.origin(h), m.corner_uv(0, h).unwrap())).collect::<Vec<_>>()).collect();
            v.sort_by_key(|x| (x.0, x.1));
            v
        };
        let before = corner_uv_by_face_vert(&m);
        m.transform(Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
        m.validate().unwrap();
        assert!(outward(&m));
        assert_eq!(corner_uv_by_face_vert(&m), before, "corner data stays on its face and vertex");
        let first: Vec<u32> = m.faces().map(|f| m.origin(m.face_he(f))).collect();
        assert_eq!(first, vec![0, 4, 0, 2, 0, 1], "faces keep their first vertex");
    }

    #[test]
    fn flip_twice_is_identity_and_open_meshes_flip() {
        let mut m = cube(Vec3::ZERO);
        m.delete_faces(&[3]).unwrap();
        let orig = m.clone();
        m.flip_all();
        m.validate().unwrap();
        assert!(m.faces().all(|f| m.face_normal(f).dot(orig.face_normal(f)) < -0.99));
        m.flip_all();
        m.validate().unwrap();
        assert_eq!(m.to_tri_buffers(&TriOptions::default()).indices, orig.to_tri_buffers(&TriOptions::default()).indices);
    }

    #[test]
    fn append_and_split_round_trip() {
        let mut a = cube(Vec3::ZERO);
        a.set_group("top", ElementKind::Face, [3]).unwrap();
        a.face_material[0] = 1;
        let mut b = cube(Vec3::new(3.0, 0.0, 0.0));
        b.set_group("top", ElementKind::Face, [3]).unwrap();
        b.set_group("corner", ElementKind::Vert, [7]).unwrap();
        b.add_uv_layer().unwrap();
        let [vo, _, fo] = a.append(&b);
        a.validate().unwrap();
        assert_eq!((vo, fo), (8, 6));
        assert_eq!((a.vert_count(), a.face_count()), (16, 12));
        assert_eq!(a.group("top").unwrap().iter().collect::<Vec<_>>(), vec![3, 9]);
        assert_eq!(a.uv_layer_count(), 1);

        let (labels, n) = a.face_components();
        assert_eq!(n, 2);
        let parts = a.split_by_face_labels(&labels);
        assert_eq!(parts.len(), 2);
        for (i, p) in parts.iter().enumerate() {
            p.validate().unwrap();
            assert_eq!((p.vert_count(), p.face_count()), (8, 6));
            assert!(outward(p));
            assert_eq!(p.group("top").unwrap().count(), 1, "part {i}");
        }
        assert_eq!(parts[0].face_material[0], 1);
        assert_eq!(parts[1].group("corner").unwrap().count(), 1);
        assert!(parts[0].group("corner").is_none());
    }

    #[test]
    fn split_duplicates_border_and_bowtie_vertices() {
        // 2 x 2 grid; label the diagonal quads 0 and 1 alternately.
        let p = (0..9).map(|i| Vec3::new((i % 3) as f32, (i / 3) as f32, 0.0)).collect();
        let corners = [0, 1, 4, 3, 1, 2, 5, 4, 3, 4, 7, 6, 4, 5, 8, 7];
        let m = Mesh::from_polygons(p, &[4; 4], &corners).unwrap();
        let parts = m.split_by_face_labels(&[0, 1, 1, 0]);
        assert_eq!(parts.len(), 2);
        for part in &parts {
            part.validate().unwrap();
            assert_eq!(part.face_count(), 2);
            // The two quads of a part touch only at the center: it is split.
            assert_eq!(part.vert_count(), 8);
        }
        // Dropping faces with INVALID.
        let only = m.split_by_face_labels(&[INVALID, 0, INVALID, INVALID]);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].face_count(), 1);
    }
}
