//! Replacing faces with new polygons: the building block of local edits
//! (extrude, inset, bevel, cuts, dissolves, triangulation).
//!
//! A [`Patch`] lists faces to remove, vertices to add and polygons to add.
//! [`Mesh::apply_patch`] applies it atomically: new polygons reuse existing
//! edges where they meet the rest of the mesh, boundary loops are relinked,
//! edges and vertices left without faces are deleted, and vertices whose
//! faces fall apart into several fans are split, so the mesh stays
//! 2-manifold. The cost depends on the patch, not on the mesh size.

use glam::{Vec2, Vec3, Vec4};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::attrs::EdgeFlags;
use crate::edit::EditError;
use crate::groups::ElementKind;
use crate::mesh::{INVALID, Mesh};

/// Where a new corner takes its corner attributes (UVs, custom normals,
/// colors) from: the blend `(1 - t) a + t b` of two existing corners
/// (half-edges of faces), read before the patch changes anything.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerSrc {
    pub a: u32,
    pub b: u32,
    pub t: f32,
}

impl CornerSrc {
    /// No source: UVs are zero, custom normals take the face normal, colors
    /// are white.
    pub const NONE: CornerSrc = CornerSrc { a: INVALID, b: INVALID, t: 0.0 };

    /// Copies the corner at half-edge `h`.
    pub fn copy(h: u32) -> CornerSrc {
        CornerSrc { a: h, b: h, t: 0.0 }
    }

    /// Blends two corners.
    pub fn lerp(a: u32, b: u32, t: f32) -> CornerSrc {
        CornerSrc { a, b, t }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct NewFace {
    start: u32,
    len: u32,
    material: u16,
    like: u32,
}

/// A set of face removals and additions, applied by [`Mesh::apply_patch`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Patch {
    base: u32,
    remove: Vec<u32>,
    new_verts: Vec<(Vec3, u32)>,
    faces: Vec<NewFace>,
    corner_vert: Vec<u32>,
    corner_src: Vec<CornerSrc>,
}

/// What [`Mesh::apply_patch`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchResult {
    /// Mesh face of each added polygon, in the order they were added.
    pub faces: Vec<u32>,
    /// Mesh vertex of each patch vertex (`INVALID` if no face used it).
    pub verts: Vec<u32>,
    pub removed_faces: usize,
    /// Edges deleted because no face used them any more.
    pub removed_edges: usize,
    /// Vertices deleted because no face used them any more.
    pub removed_verts: usize,
    /// Vertex copies made where the faces around one vertex fell apart.
    pub split_verts: usize,
}

impl Patch {
    /// An empty patch for `mesh`. Apply it before changing the mesh in any other way.
    pub fn new(mesh: &Mesh) -> Patch {
        Patch { base: mesh.vert_slots() as u32, ..Patch::default() }
    }

    /// Marks a face for removal.
    pub fn remove_face(&mut self, f: u32) {
        self.remove.push(f);
    }

    /// Adds a vertex and returns the id it will have. It joins the vertex
    /// groups of `like`.
    pub fn new_vert(&mut self, p: Vec3, like: Option<u32>) -> u32 {
        self.new_verts.push((p, like.unwrap_or(INVALID)));
        self.base + self.new_verts.len() as u32 - 1
    }

    /// Position of a vertex added with [`Patch::new_vert`].
    pub fn new_vert_position(&self, v: u32) -> Option<Vec3> {
        v.checked_sub(self.base).and_then(|i| self.new_verts.get(i as usize)).map(|x| x.0)
    }

    /// Whether `v` is a vertex added by this patch.
    pub fn is_new_vert(&self, v: u32) -> bool {
        v >= self.base
    }

    /// Index of a new vertex in [`PatchResult::verts`] (0 for an old vertex).
    pub fn new_vert_index(&self, v: u32) -> usize {
        v.saturating_sub(self.base) as usize
    }

    /// Adds a polygon over existing or new vertices in winding order (the
    /// normal follows the right-hand rule). `src` gives each corner's
    /// attribute source (missing entries mean [`CornerSrc::NONE`]). The face
    /// takes `material` and joins the face groups of `like`. Returns the
    /// polygon's index in the patch.
    pub fn add_face(&mut self, verts: &[u32], src: &[CornerSrc], material: u16, like: Option<u32>) -> usize {
        let start = self.corner_vert.len() as u32;
        self.corner_vert.extend_from_slice(verts);
        self.corner_src.extend((0..verts.len()).map(|i| src.get(i).copied().unwrap_or(CornerSrc::NONE)));
        self.faces.push(NewFace { start, len: verts.len() as u32, material, like: like.unwrap_or(INVALID) });
        self.faces.len() - 1
    }

    /// Number of polygons added so far.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Faces marked for removal.
    pub fn removed(&self) -> &[u32] {
        &self.remove
    }

    pub fn is_empty(&self) -> bool {
        self.remove.is_empty() && self.faces.is_empty()
    }

    fn corners(&self, nf: &NewFace) -> std::ops::Range<usize> {
        nf.start as usize..(nf.start + nf.len) as usize
    }

    /// Corner after corner `c` of polygon `nf`.
    fn next_corner(&self, c: usize, nf: &NewFace) -> usize {
        nf.start as usize + (c - nf.start as usize + 1) % nf.len as usize
    }
}

#[inline]
fn key(a: u32, b: u32) -> u64 {
    (u64::from(a) << 32) | u64::from(b)
}

impl Mesh {
    /// Applies a patch: removes its faces, adds its vertices and polygons,
    /// links the polygons to the rest of the mesh through shared edges,
    /// deletes edges and vertices left without faces and splits vertices
    /// whose faces fall apart into several fans. Nothing changes when it
    /// fails.
    ///
    /// Fails when a polygon is malformed (fewer than 3 corners, a repeated,
    /// dead or unknown vertex, a corner source that is not a face corner),
    /// when a polygon edge is already used in the same direction by a face
    /// that stays or by another new polygon (the result would be
    /// non-manifold or inconsistently wound), or when the mesh changed since
    /// the patch was created.
    ///
    /// Complexity: O(c log c + sum of valences of the touched vertices +
    /// groups x new elements) for the c corners of the removed and added
    /// faces; independent of the mesh size.
    pub fn apply_patch(&mut self, p: &Patch) -> Result<PatchResult, EditError> {
        // ---- checks: nothing changes until they pass ----
        if self.vert_slots() as u32 != p.base {
            return Err(EditError::StalePatch);
        }
        let nv_all = p.base + p.new_verts.len() as u32;
        let mut remove = p.remove.clone();
        remove.sort_unstable();
        remove.dedup();
        if let Some(&f) = remove.iter().find(|&&f| !self.is_face_alive(f)) {
            return Err(EditError::NoSuchFace(f));
        }
        let removing = |f: u32| remove.binary_search(&f).is_ok();
        let is_corner = |h: u32| self.is_he_alive(h) && !self.is_boundary_he(h);
        let mut seen: FxHashSet<u32> = FxHashSet::default();
        for (i, nf) in p.faces.iter().enumerate() {
            let vs = &p.corner_vert[p.corners(nf)];
            let bad = |reason: String| EditError::BadFace { face: i, reason };
            if vs.len() < 3 {
                return Err(bad(format!("{} corners; a face needs at least 3", vs.len())));
            }
            seen.clear();
            for &v in vs {
                if !(if v < p.base { self.is_vert_alive(v) } else { v < nv_all }) {
                    return Err(bad(format!("vertex {v} does not exist")));
                }
                if !seen.insert(v) {
                    return Err(bad(format!("vertex {v} is used twice")));
                }
            }
            for s in &p.corner_src[p.corners(nf)] {
                if s.a != INVALID && !(is_corner(s.a) && is_corner(s.b)) {
                    return Err(bad(format!("corner source {} or {} is not a face corner", s.a, s.b)));
                }
            }
            if nf.like != INVALID && !self.is_face_alive(nf.like) {
                return Err(EditError::NoSuchFace(nf.like));
            }
        }
        if let Some(&(_, like)) = p.new_verts.iter().find(|&&(pos, like)| !pos.is_finite() || (like != INVALID && !self.is_vert_alive(like))) {
            return Err(EditError::BadVertex(like));
        }

        // The half-edge of every new corner: an existing one (on the boundary
        // or freed by a removed face) or one of a new edge.
        let corners = p.corner_vert.len();
        let mut directed: FxHashMap<u64, u32> = FxHashMap::default();
        directed.reserve(corners);
        let mut corner_he = vec![INVALID; corners];
        // Origin of the first half-edge of each new edge.
        let mut new_edges: Vec<u32> = Vec::new();
        let mut new_edge_of: FxHashMap<u64, u32> = FxHashMap::default();
        let edge_base = self.edge_slots() as u32;
        for nf in &p.faces {
            for c in p.corners(nf) {
                let (a, b) = (p.corner_vert[c], p.corner_vert[p.next_corner(c, nf)]);
                if directed.insert(key(a, b), c as u32).is_some() {
                    return Err(EditError::EdgeInUse { a, b });
                }
                if a < p.base
                    && b < p.base
                    && let Some(h) = self.find_halfedge(a, b)
                {
                    match self.face(h) {
                        Some(f) if !removing(f) => return Err(EditError::EdgeInUse { a, b }),
                        _ => corner_he[c] = h,
                    }
                    continue;
                }
                let k = key(a.min(b), a.max(b));
                corner_he[c] = match new_edge_of.get(&k) {
                    Some(&i) => 2 * (edge_base + i) + u32::from(new_edges[i as usize] != a),
                    None => {
                        let i = new_edges.len() as u32;
                        new_edge_of.insert(k, i);
                        new_edges.push(a);
                        2 * (edge_base + i)
                    }
                };
            }
        }

        // Corner attributes of the new corners, read before anything changes.
        let blend2 = |col: &[Vec2], s: &CornerSrc| if s.a == INVALID { Vec2::ZERO } else { col[s.a as usize].lerp(col[s.b as usize], s.t) };
        let uvs: Vec<Vec<Vec2>> = self.corner_uv.iter().map(|col| p.corner_src.iter().map(|s| blend2(col, s)).collect()).collect();
        let normals: Option<Vec<Option<Vec3>>> = self.corner_normal.as_ref().map(|col| {
            p.corner_src.iter().map(|s| (s.a != INVALID).then(|| col[s.a as usize].lerp(col[s.b as usize], s.t).normalize_or_zero())).collect()
        });
        let colors: Option<Vec<Vec4>> = self.corner_color.as_ref().map(|col| {
            p.corner_src.iter().map(|s| if s.a == INVALID { Vec4::ONE } else { col[s.a as usize].lerp(col[s.b as usize], s.t) }).collect()
        });
        // Face groups of each `like` face and vertex groups of each `like` vertex.
        let groups_of = |kind: ElementKind, x: u32| -> Vec<usize> {
            if x == INVALID {
                return Vec::new();
            }
            self.groups.values().enumerate().filter(|(_, g)| g.kind() == kind && g.contains(x)).map(|(i, _)| i).collect()
        };
        let face_like: Vec<Vec<usize>> = p.faces.iter().map(|nf| groups_of(ElementKind::Face, nf.like)).collect();
        let vert_like: Vec<Vec<usize>> = p.new_verts.iter().map(|&(_, like)| groups_of(ElementKind::Vert, like)).collect();

        // Old outgoing half-edges of every touched vertex, from the still valid rotation.
        let mut touched: Vec<u32> = Vec::new();
        for &f in &remove {
            touched.extend(self.face_verts(f));
        }
        touched.extend(p.corner_vert.iter().copied().filter(|&v| v < p.base));
        touched.sort_unstable();
        touched.dedup();
        let mut out_pairs: Vec<(u32, u32)> = Vec::new();
        for &v in &touched {
            out_pairs.extend(self.vert_outgoing(v).map(|h| (v, h)));
        }
        let old_edges: Vec<u32> = {
            let mut e: Vec<u32> = out_pairs.iter().map(|&(_, h)| h >> 1).collect();
            e.sort_unstable();
            e.dedup();
            e
        };

        // ---- mutation ----
        let mut result = PatchResult { removed_faces: remove.len(), ..PatchResult::default() };
        self.reserve(p.new_verts.len(), new_edges.len(), p.faces.len());
        for (i, &(pos, _)) in p.new_verts.iter().enumerate() {
            let v = self.vert_slots() as u32;
            self.positions.push(pos);
            self.vert_he.push(INVALID);
            self.grow_groups();
            for &gi in &vert_like[i] {
                self.groups[gi].insert(v);
            }
            result.verts.push(v);
        }
        for &a in &new_edges {
            // The twin's origin is set from the polygons below.
            self.he_vert.extend_from_slice(&[a, INVALID]);
            self.he_face.extend_from_slice(&[INVALID, INVALID]);
            self.he_next.extend_from_slice(&[INVALID, INVALID]);
            self.he_prev.extend_from_slice(&[INVALID, INVALID]);
            self.edge_flags.push(EdgeFlags::NONE);
            if let Some(c) = &mut self.edge_crease {
                c.push(0.0);
            }
        }
        let nh = self.he_slots();
        for col in &mut self.corner_uv {
            col.resize(nh, Vec2::ZERO);
        }
        if let Some(col) = &mut self.corner_normal {
            col.resize(nh, Vec3::ZERO);
        }
        if let Some(col) = &mut self.corner_color {
            col.resize(nh, Vec4::ONE);
        }
        for &f in &remove {
            let mut h = self.face_he[f as usize];
            // A face loop has at most as many half-edges as there are slots.
            for _ in 0..nh {
                self.he_face[h as usize] = INVALID;
                h = self.he_next[h as usize];
                if h == self.face_he[f as usize] {
                    break;
                }
            }
            self.face_he[f as usize] = INVALID;
            self.free_faces.push(f);
            self.ungroup(ElementKind::Face, f);
        }
        for (i, nf) in p.faces.iter().enumerate() {
            let f = self.face_slots() as u32;
            let range = p.corners(nf);
            self.face_he.push(corner_he[range.start]);
            self.face_material.push(nf.material);
            self.grow_groups();
            for &gi in &face_like[i] {
                self.groups[gi].insert(f);
            }
            for c in range {
                let c1 = p.next_corner(c, nf);
                let (h, n) = (corner_he[c], corner_he[c1]);
                self.he_face[h as usize] = f;
                self.he_next[h as usize] = n;
                self.he_prev[n as usize] = h;
                self.he_vert[h as usize] = p.corner_vert[c];
                self.he_vert[(h ^ 1) as usize] = p.corner_vert[c1];
                for (l, col) in self.corner_uv.iter_mut().enumerate() {
                    col[h as usize] = uvs[l][c];
                }
                if let Some(cs) = &colors
                    && let Some(col) = &mut self.corner_color
                {
                    col[h as usize] = cs[c];
                }
            }
            result.faces.push(f);
        }
        if let Some(ns) = &normals {
            for (i, nf) in p.faces.iter().enumerate() {
                let n = self.face_normal(result.faces[i]);
                if let Some(col) = &mut self.corner_normal {
                    for c in p.corners(nf) {
                        col[corner_he[c] as usize] = ns[c].unwrap_or(n);
                    }
                }
            }
        }

        // Edges left without faces (only old ones can be: new edges have a face).
        for &e in &old_edges {
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
                result.removed_edges += 1;
            }
        }

        // Rebuild the fans of every touched or new vertex: surviving old
        // outgoing half-edges plus the new half-edges and their twins.
        for &h in &corner_he {
            out_pairs.push((self.he_vert[h as usize], h));
            out_pairs.push((self.he_vert[(h ^ 1) as usize], h ^ 1));
        }
        out_pairs.retain(|&(_, h)| self.he_vert[h as usize] != INVALID);
        out_pairs.sort_unstable();
        out_pairs.dedup();
        let mut visited: FxHashSet<u32> = FxHashSet::default();
        let mut fan: Vec<u32> = Vec::new();
        let mut handled: Vec<u32> = Vec::new();
        let mut i = 0;
        while i < out_pairs.len() {
            let v = out_pairs[i].0;
            let mut j = i;
            while j < out_pairs.len() && out_pairs[j].0 == v {
                j += 1;
            }
            handled.push(v);
            let degree = j - i;
            let mut fans = 0usize;
            for &(_, start) in &out_pairs[i..j] {
                if self.he_face[start as usize] == INVALID || visited.contains(&start) {
                    continue;
                }
                // Back up to the first corner of an open fan; a closed fan
                // brings the walk back to the start.
                let mut first = start;
                for _ in 0..degree {
                    let t = first ^ 1;
                    if self.he_face[t as usize] == INVALID {
                        break;
                    }
                    let before = self.he_next[t as usize];
                    if before == start {
                        break;
                    }
                    first = before;
                }
                fan.clear();
                fan.push(first);
                let mut out_b = INVALID;
                let mut h = first;
                while fan.len() <= degree {
                    let t = self.he_prev[h as usize] ^ 1;
                    if self.he_face[t as usize] == INVALID {
                        out_b = t;
                        break;
                    }
                    if t == first {
                        break;
                    }
                    fan.push(t);
                    h = t;
                }
                visited.extend(fan.iter().copied());
                let target = if fans == 0 {
                    v
                } else {
                    result.split_verts += 1;
                    self.copy_vertex(v)
                };
                for &x in &fan {
                    self.he_vert[x as usize] = target;
                }
                if out_b == INVALID {
                    self.vert_he[target as usize] = first;
                } else {
                    let in_b = first ^ 1;
                    self.he_vert[out_b as usize] = target;
                    self.he_next[in_b as usize] = out_b;
                    self.he_prev[out_b as usize] = in_b;
                    self.vert_he[target as usize] = out_b;
                }
                fans += 1;
            }
            if fans == 0 {
                self.kill_vertex(v, &mut result);
            }
            i = j;
        }
        // Touched vertices that lost every edge, and new vertices no face uses.
        handled.sort_unstable();
        for v in touched.iter().copied().chain(p.base..nv_all) {
            if handled.binary_search(&v).is_err() {
                self.kill_vertex(v, &mut result);
            }
        }
        for x in &mut result.verts {
            if self.vert_he[*x as usize] == INVALID {
                *x = INVALID;
            }
        }
        self.revision += 1;
        Ok(result)
    }

    /// A new vertex at the position of `v`, in the same vertex groups, with
    /// no edges yet.
    ///
    /// Complexity: O(groups).
    fn copy_vertex(&mut self, v: u32) -> u32 {
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

    /// Deletes a vertex that has no edges left. Each vertex is passed here at
    /// most once per patch.
    ///
    /// Complexity: O(groups).
    fn kill_vertex(&mut self, v: u32, result: &mut PatchResult) {
        self.vert_he[v as usize] = INVALID;
        self.free_verts.push(v);
        self.ungroup(ElementKind::Vert, v);
        result.removed_verts += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

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

    fn euler(m: &Mesh) -> i64 {
        m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
    }

    /// Re-adds every removed face exactly: the topology comes back.
    #[test]
    fn identity_patch_keeps_the_mesh() {
        let mut m = grid(3);
        m.add_uv_layer_from_polygon_order(&(0..36).map(|i| Vec2::splat(i as f32)).collect::<Vec<_>>()).unwrap();
        m.set_group("mid", ElementKind::Face, [4]).unwrap();
        let before_uv: Vec<Vec2> = m.face_halfedges(4).map(|h| m.corner_uv(0, h).unwrap()).collect();
        let mut p = Patch::new(&m);
        p.remove_face(4);
        let vs: Vec<u32> = m.face_verts(4).collect();
        let src: Vec<CornerSrc> = m.face_halfedges(4).map(CornerSrc::copy).collect();
        p.add_face(&vs, &src, 2, Some(4));
        let r = m.apply_patch(&p).unwrap();
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.edge_count(), m.face_count()), (16, 24, 9));
        assert_eq!((r.removed_edges, r.removed_verts, r.split_verts), (0, 0, 0));
        let f = r.faces[0];
        assert_eq!(m.face_material(f), 2);
        assert!(m.group("mid").unwrap().contains(f), "joins the groups of `like`");
        assert_eq!(m.face_halfedges(f).map(|h| m.corner_uv(0, h).unwrap()).collect::<Vec<_>>(), before_uv);
    }

    /// Extrudes the middle quad of a grid by hand: a cap and four side walls.
    #[test]
    fn extrude_by_patch() {
        let mut m = grid(3);
        let f = 4;
        let vs: Vec<u32> = m.face_verts(f).collect();
        let hs: Vec<u32> = m.face_halfedges(f).collect();
        let mut p = Patch::new(&m);
        p.remove_face(f);
        let top: Vec<u32> = vs.iter().map(|&v| p.new_vert(m.positions[v as usize] + Vec3::Z, Some(v))).collect();
        let cap_src: Vec<CornerSrc> = hs.iter().map(|&h| CornerSrc::copy(h)).collect();
        p.add_face(&top, &cap_src, 0, Some(f));
        for i in 0..4 {
            let j = (i + 1) % 4;
            p.add_face(&[vs[i], vs[j], top[j], top[i]], &[], 0, None);
        }
        let r = m.apply_patch(&p).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 9 - 1 + 5);
        assert_eq!(m.vert_count(), 16 + 4);
        assert_eq!(euler(&m), 1, "still a disc");
        assert!(m.face_normal(r.faces[0]).abs_diff_eq(Vec3::Z, 1e-6));
        assert!(m.edges().filter(|&e| m.is_boundary_edge(e)).count() == 12);
    }

    #[test]
    fn closing_a_cube_and_poking_a_face() {
        // Remove the top of a cube and close it with four triangles to a new apex.
        let mut m = cube();
        let f = 3;
        let vs: Vec<u32> = m.face_verts(f).collect();
        let mut p = Patch::new(&m);
        p.remove_face(f);
        let apex = p.new_vert(Vec3::new(0.5, 1.5, 0.5), None);
        for i in 0..4 {
            p.add_face(&[vs[i], vs[(i + 1) % 4], apex], &[], 0, None);
        }
        m.apply_patch(&p).unwrap();
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.face_count(), euler(&m)), (9, 9, 2));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
    }

    #[test]
    fn removing_only_deletes_like_delete_faces() {
        let mut a = grid(2);
        let mut b = a.clone();
        let mut p = Patch::new(&a);
        p.remove_face(0);
        p.remove_face(3);
        let r = a.apply_patch(&p).unwrap();
        a.validate().unwrap();
        let d = b.delete_faces(&[0, 3]).unwrap();
        b.validate().unwrap();
        assert_eq!((r.removed_edges, r.removed_verts, r.split_verts), (d.edges, d.verts, d.split_verts));
        assert_eq!((a.vert_count(), a.edge_count(), a.face_count()), (b.vert_count(), b.edge_count(), b.face_count()));
    }

    #[test]
    fn bowtie_vertices_are_split() {
        // A triangle touching a grid corner only at one vertex.
        let mut m = grid(1);
        let mut p = Patch::new(&m);
        let a = p.new_vert(Vec3::new(-1.0, 0.0, 0.0), None);
        let b = p.new_vert(Vec3::new(-1.0, -1.0, 0.0), None);
        p.add_face(&[0, a, b], &[], 0, None);
        let r = m.apply_patch(&p).unwrap();
        m.validate().unwrap();
        assert_eq!(r.split_verts, 1);
        assert_eq!(m.vert_count(), 4 + 2 + 1);
    }

    #[test]
    fn errors_change_nothing() {
        let mut m = grid(2);
        let before = m.clone();
        // Over an existing face that stays, in the same direction.
        let vs: Vec<u32> = m.face_verts(0).collect();
        let mut p = Patch::new(&m);
        p.add_face(&vs, &[], 0, None);
        assert!(matches!(m.apply_patch(&p), Err(EditError::EdgeInUse { .. })));
        let mut p = Patch::new(&m);
        p.add_face(&[0, 1], &[], 0, None);
        assert!(matches!(m.apply_patch(&p), Err(EditError::BadFace { .. })));
        let mut p = Patch::new(&m);
        p.remove_face(0);
        p.add_face(&[0, 1, 0, 3], &[], 0, None);
        assert!(matches!(m.apply_patch(&p), Err(EditError::BadFace { .. })));
        let mut p = Patch::new(&m);
        p.remove_face(42);
        assert_eq!(m.apply_patch(&p), Err(EditError::NoSuchFace(42)));
        let mut p = Patch::new(&m);
        p.remove_face(0);
        p.add_face(&vs, &[CornerSrc::copy(9999)], 0, None);
        assert!(matches!(m.apply_patch(&p), Err(EditError::BadFace { .. })));
        let stale = Patch::new(&m);
        let mut grown = m.clone();
        let mut q = Patch::new(&grown);
        q.new_vert(Vec3::ZERO, None);
        q.add_face(&[1, 0, 9], &[], 0, None);
        grown.apply_patch(&q).unwrap();
        assert_eq!(grown.apply_patch(&stale), Err(EditError::StalePatch));
        assert_eq!(m, before);
    }

    #[test]
    fn reversing_a_whole_part_flips_it() {
        let mut m = cube();
        let mut p = Patch::new(&m);
        for f in 0..6 {
            p.remove_face(f);
            let mut vs: Vec<u32> = m.face_verts(f).collect();
            vs.reverse();
            p.add_face(&vs, &[], 0, None);
        }
        let r = m.apply_patch(&p).unwrap();
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.edge_count(), m.face_count()), (8, 12, 6));
        assert_eq!((r.removed_edges, r.removed_verts), (0, 0));
        let c = Vec3::splat(0.5);
        assert!(m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f) - c) < 0.0), "inward now");
    }
}
