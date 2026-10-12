//! The half-edge mesh: topology arrays, element liveness, adjacency queries
//! and simple geometric measures.

use glam::{Vec2, Vec3, Vec4};
use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::attrs::EdgeFlags;
use crate::groups::Group;

/// Marker for "no element" in topology arrays.
pub const INVALID: u32 = u32::MAX;

/// A 2-manifold polygon mesh with boundary, stored as half-edges in
/// structure-of-arrays form with `u32` indices.
///
/// - Half-edges are allocated in pairs: the twin of `h` is `h ^ 1` and the
///   edge of `h` is `h >> 1`.
/// - A half-edge without a face (`face(h) == None`) lies on the boundary.
///   Every edge has at least one face; there are no wire edges.
/// - Every live vertex has at least one edge (no loose vertices). Its
///   `vert_he` is an outgoing half-edge, the boundary one when the vertex
///   lies on the boundary.
/// - Deleted elements stay in the arrays until [`Mesh::compact`]: a dead
///   vertex has `vert_he == INVALID`, a dead edge has `origin == INVALID` on
///   both half-edges, a dead face has `face_he == INVALID`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub(crate) he_next: Vec<u32>,
    pub(crate) he_prev: Vec<u32>,
    pub(crate) he_vert: Vec<u32>,
    pub(crate) he_face: Vec<u32>,
    pub(crate) vert_he: Vec<u32>,
    pub(crate) face_he: Vec<u32>,
    /// Vertex positions in object space, one per vertex slot.
    pub positions: Vec<Vec3>,
    /// UV layers (UV0, UV1 for lightmaps), one value per half-edge slot:
    /// the corner of the half-edge's face at its origin vertex.
    pub corner_uv: SmallVec<[Vec<Vec2>; 2]>,
    /// Custom split normals per half-edge slot.
    pub corner_normal: Option<Vec<Vec3>>,
    /// Vertex colors per half-edge slot.
    pub corner_color: Option<Vec<Vec4>>,
    /// Material slot per face slot.
    pub face_material: Vec<u16>,
    /// Seam and sharp flags per edge slot.
    pub edge_flags: Vec<EdgeFlags>,
    /// Subdivision crease weight in [0, 1] per edge slot.
    pub edge_crease: Option<Vec<f32>>,
    /// Named element groups in creation order.
    pub(crate) groups: IndexMap<String, Group>,
    pub(crate) free_verts: Vec<u32>,
    pub(crate) free_edges: Vec<u32>,
    pub(crate) free_faces: Vec<u32>,
    /// Incremented by every topology or attribute change made through the
    /// kernel's editing functions.
    pub revision: u64,
}

impl Mesh {
    /// An empty mesh.
    pub fn new() -> Self {
        Self::default()
    }

    // ---- element slots and liveness -------------------------------------

    /// Number of vertex slots, live or dead.
    #[inline]
    pub fn vert_slots(&self) -> usize {
        self.vert_he.len()
    }

    /// Number of edge slots, live or dead.
    #[inline]
    pub fn edge_slots(&self) -> usize {
        self.he_vert.len() / 2
    }

    /// Number of half-edge slots, live or dead (twice the edge slots).
    #[inline]
    pub fn he_slots(&self) -> usize {
        self.he_vert.len()
    }

    /// Number of face slots, live or dead.
    #[inline]
    pub fn face_slots(&self) -> usize {
        self.face_he.len()
    }

    #[inline]
    pub fn vert_count(&self) -> usize {
        self.vert_slots() - self.free_verts.len()
    }

    #[inline]
    pub fn edge_count(&self) -> usize {
        self.edge_slots() - self.free_edges.len()
    }

    #[inline]
    pub fn face_count(&self) -> usize {
        self.face_slots() - self.free_faces.len()
    }

    /// Whether the mesh has dead slots that [`Mesh::compact`] would remove.
    #[inline]
    pub fn has_holes_in_storage(&self) -> bool {
        !(self.free_verts.is_empty() && self.free_edges.is_empty() && self.free_faces.is_empty())
    }

    #[inline]
    pub fn is_vert_alive(&self, v: u32) -> bool {
        self.vert_he.get(v as usize).is_some_and(|&h| h != INVALID)
    }

    #[inline]
    pub fn is_edge_alive(&self, e: u32) -> bool {
        self.he_vert.get(2 * e as usize).is_some_and(|&v| v != INVALID)
    }

    #[inline]
    pub fn is_he_alive(&self, h: u32) -> bool {
        self.he_vert.get(h as usize).is_some_and(|&v| v != INVALID)
    }

    #[inline]
    pub fn is_face_alive(&self, f: u32) -> bool {
        self.face_he.get(f as usize).is_some_and(|&h| h != INVALID)
    }

    /// Live vertices in index order.
    ///
    /// Complexity: O(vertex slots).
    pub fn verts(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.vert_slots() as u32).filter(|&v| self.vert_he[v as usize] != INVALID)
    }

    /// Live edges in index order.
    ///
    /// Complexity: O(edge slots).
    pub fn edges(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.edge_slots() as u32).filter(|&e| self.he_vert[2 * e as usize] != INVALID)
    }

    /// Live faces in index order.
    ///
    /// Complexity: O(face slots).
    pub fn faces(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.face_slots() as u32).filter(|&f| self.face_he[f as usize] != INVALID)
    }

    // ---- half-edge navigation (all O(1)) -----------------------------------

    #[inline]
    pub fn twin(&self, h: u32) -> u32 {
        h ^ 1
    }

    #[inline]
    pub fn next(&self, h: u32) -> u32 {
        self.he_next[h as usize]
    }

    #[inline]
    pub fn prev(&self, h: u32) -> u32 {
        self.he_prev[h as usize]
    }

    /// Vertex the half-edge starts at.
    #[inline]
    pub fn origin(&self, h: u32) -> u32 {
        self.he_vert[h as usize]
    }

    /// Vertex the half-edge points to.
    #[inline]
    pub fn dest(&self, h: u32) -> u32 {
        self.he_vert[(h ^ 1) as usize]
    }

    /// Face of the half-edge, `None` on the boundary.
    #[inline]
    pub fn face(&self, h: u32) -> Option<u32> {
        let f = self.he_face[h as usize];
        (f != INVALID).then_some(f)
    }

    #[inline]
    pub fn is_boundary_he(&self, h: u32) -> bool {
        self.he_face[h as usize] == INVALID
    }

    /// Edge index of a half-edge.
    #[inline]
    pub fn edge_of(&self, h: u32) -> u32 {
        h >> 1
    }

    /// First half-edge of an edge (its twin is `edge_he(e) ^ 1`).
    #[inline]
    pub fn edge_he(&self, e: u32) -> u32 {
        2 * e
    }

    /// One outgoing half-edge of a vertex (the boundary one on the boundary).
    #[inline]
    pub fn vert_he(&self, v: u32) -> u32 {
        self.vert_he[v as usize]
    }

    /// First half-edge of a face (the corner of its first vertex).
    #[inline]
    pub fn face_he(&self, f: u32) -> u32 {
        self.face_he[f as usize]
    }

    /// Next outgoing half-edge around the origin vertex of `h`.
    #[inline]
    pub fn rotate(&self, h: u32) -> u32 {
        self.prev(h) ^ 1
    }

    /// The two vertices of an edge.
    #[inline]
    pub fn edge_verts(&self, e: u32) -> [u32; 2] {
        [self.he_vert[2 * e as usize], self.he_vert[2 * e as usize + 1]]
    }

    /// The faces on both sides of an edge (`None` on the boundary side).
    #[inline]
    pub fn edge_faces(&self, e: u32) -> [Option<u32>; 2] {
        [self.face(2 * e), self.face(2 * e + 1)]
    }

    #[inline]
    pub fn is_boundary_edge(&self, e: u32) -> bool {
        self.is_boundary_he(2 * e) || self.is_boundary_he(2 * e + 1)
    }

    #[inline]
    pub fn is_boundary_vert(&self, v: u32) -> bool {
        self.is_boundary_he(self.vert_he(v))
    }

    // ---- circulators ----------------------------------------------------------

    /// Half-edges of a face in winding order, starting at `face_he(f)`.
    ///
    /// Complexity: O(face size).
    pub fn face_halfedges(&self, f: u32) -> Loop<'_> {
        Loop { mesh: self, start: self.face_he(f), cur: Some(self.face_he(f)), guard: self.he_slots(), step: Step::Next }
    }

    /// Vertices of a face in winding order.
    ///
    /// Complexity: O(face size).
    pub fn face_verts(&self, f: u32) -> impl Iterator<Item = u32> + '_ {
        self.face_halfedges(f).map(|h| self.origin(h))
    }

    /// Number of corners of a face.
    ///
    /// Complexity: O(face size).
    pub fn face_len(&self, f: u32) -> usize {
        self.face_halfedges(f).count()
    }

    /// Outgoing half-edges of a vertex, starting at `vert_he(v)`.
    ///
    /// Complexity: O(valence).
    pub fn vert_outgoing(&self, v: u32) -> Loop<'_> {
        Loop { mesh: self, start: self.vert_he(v), cur: Some(self.vert_he(v)), guard: self.he_slots(), step: Step::Rotate }
    }

    /// Neighbouring vertices of a vertex.
    ///
    /// Complexity: O(valence).
    pub fn vert_neighbors(&self, v: u32) -> impl Iterator<Item = u32> + '_ {
        self.vert_outgoing(v).map(|h| self.dest(h))
    }

    /// Faces around a vertex.
    ///
    /// Complexity: O(valence).
    pub fn vert_faces(&self, v: u32) -> impl Iterator<Item = u32> + '_ {
        self.vert_outgoing(v).filter_map(|h| self.face(h))
    }

    /// Number of edges at a vertex.
    ///
    /// Complexity: O(valence).
    pub fn valence(&self, v: u32) -> usize {
        self.vert_outgoing(v).count()
    }

    /// The half-edge from `a` to `b`, if the edge exists.
    ///
    /// Complexity: O(valence of a).
    pub fn find_halfedge(&self, a: u32, b: u32) -> Option<u32> {
        self.vert_outgoing(a).find(|&h| self.dest(h) == b)
    }

    // ---- geometry -------------------------------------------------------------

    /// Unnormalized face normal by Newell's method; its length is twice the
    /// face area. Robust for non-planar and concave faces.
    ///
    /// Complexity: O(face size).
    pub fn face_normal_area2(&self, f: u32) -> Vec3 {
        let mut n = Vec3::ZERO;
        for h in self.face_halfedges(f) {
            let a = self.positions[self.origin(h) as usize];
            let b = self.positions[self.dest(h) as usize];
            n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        n
    }

    /// Unit face normal (zero for degenerate faces).
    ///
    /// Complexity: O(face size).
    pub fn face_normal(&self, f: u32) -> Vec3 {
        self.face_normal_area2(f).normalize_or_zero()
    }

    /// Face area.
    ///
    /// Complexity: O(face size).
    pub fn face_area(&self, f: u32) -> f32 {
        0.5 * self.face_normal_area2(f).length()
    }

    /// Average of the face's vertex positions.
    ///
    /// Complexity: O(face size).
    pub fn face_centroid(&self, f: u32) -> Vec3 {
        let (sum, n) = self.face_verts(f).fold((Vec3::ZERO, 0u32), |(s, n), v| (s + self.positions[v as usize], n + 1));
        if n == 0 { Vec3::ZERO } else { sum / n as f32 }
    }

    /// Edge length.
    pub fn edge_length(&self, e: u32) -> f32 {
        let [a, b] = self.edge_verts(e);
        self.positions[a as usize].distance(self.positions[b as usize])
    }

    /// Axis-aligned bounds of the live vertices, `None` when empty.
    ///
    /// Complexity: O(vertex slots).
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.verts().map(|v| self.positions[v as usize]).fold(None, |acc, p| match acc {
            None => Some((p, p)),
            Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
        })
    }

    /// Number of triangles after triangulating every face.
    ///
    /// Complexity: O(face slots + half-edges).
    pub fn triangle_count(&self) -> usize {
        self.faces().map(|f| self.face_len(f).saturating_sub(2)).sum()
    }
}

#[derive(Clone, Copy)]
enum Step {
    Next,
    Rotate,
}

/// Iterator over a closed cycle of half-edges (a face loop or a vertex fan).
/// Stops after one full turn, or after visiting every half-edge slot if the
/// mesh is corrupted, so it always terminates.
pub struct Loop<'a> {
    mesh: &'a Mesh,
    start: u32,
    cur: Option<u32>,
    guard: usize,
    step: Step,
}

impl Iterator for Loop<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        let h = self.cur?;
        if h == INVALID || self.guard == 0 {
            self.cur = None;
            return None;
        }
        self.guard -= 1;
        let n = match self.step {
            Step::Next => self.mesh.next(h),
            Step::Rotate => self.mesh.rotate(h),
        };
        self.cur = (n != self.start).then_some(n);
        Some(h)
    }
}
