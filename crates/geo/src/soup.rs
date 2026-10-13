//! Repairing polygon soup into a 2-manifold mesh (ADR 0007). Files from
//! other tools split vertices at UV seams, repeat faces, mix winding, share
//! one edge between three or more faces or touch at single vertices; the
//! half-edge kernel accepts none of that. [`repair`] welds, drops what
//! cannot be a face, unifies winding and splits what stays non-manifold,
//! and says what it did. It never moves a vertex beyond the weld distance.

use glam::Vec3;
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use p3d_mesh::{BuildError, Mesh};

use crate::edit::simple_polygons;

/// Most rounds of detaching faces that still break the surface.
const MAX_ROUNDS: usize = 16;

/// Options of [`repair`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SoupSpec {
    /// Vertices closer than this weld into one; 0 welds identical positions only.
    pub weld: f32,
}

/// What [`repair`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoupReport {
    /// Vertices merged into another one.
    pub welded: usize,
    /// Faces dropped for having fewer than three distinct corners or a
    /// non-finite position.
    pub degenerate: usize,
    /// Faces dropped for repeating another face's corners.
    pub duplicates: usize,
    /// Faces turned to agree with their neighbours (the majority of each
    /// part keeps its winding).
    pub flipped: usize,
    /// Edges shared by three or more faces, or by faces that cannot agree
    /// on a winding (a Moebius strip), split apart.
    pub split_edges: usize,
    /// Vertex copies made where the faces around a vertex fall apart into
    /// several fans.
    pub split_verts: usize,
    /// Faces cut loose from all neighbours as a last resort.
    pub detached: usize,
}

impl SoupReport {
    /// Whether the soup was already a clean manifold mesh (welding aside).
    pub fn is_clean(&self) -> bool {
        self.degenerate == 0 && self.duplicates == 0 && self.flipped == 0 && self.split_edges == 0 && self.split_verts == 0 && self.detached == 0
    }
}

/// A repaired mesh and where its pieces came from.
#[derive(Debug, Clone, Default)]
pub struct Repaired {
    pub mesh: Mesh,
    /// Input corner of every mesh corner, in polygon order (live faces in
    /// index order, corners from `face_he`; see
    /// [`Mesh::corner_values_from_polygon_order`]).
    pub corner_src: Vec<u32>,
    /// Input face of every mesh face.
    pub face_src: Vec<u32>,
    pub report: SoupReport,
}

/// Why soup cannot be repaired.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SoupError {
    #[error("the face sizes add up to {expected} corners but {got} corners were given")]
    CornerCount { expected: usize, got: usize },
    #[error("face {face} uses vertex {vert}, but there are only {count} vertices")]
    VertexOutOfRange { face: u32, vert: u32, count: u32 },
    #[error("the mesh is too large for u32 indices")]
    TooLarge,
    #[error("no face is left")]
    Empty,
    /// A defect the repair does not handle; the message names it.
    #[error("cannot repair the mesh: {0}")]
    Unrepairable(String),
}

/// Union-find over corners; the smaller root wins, so results do not
/// depend on the order of unions.
struct Dsu(Vec<u32>);

impl Dsu {
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let up = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = up;
            x = up;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.0[hi as usize] = lo;
        }
    }
}

/// Faces as runs of corners.
#[derive(Default)]
struct Faces {
    start: Vec<u32>,
    len: Vec<u32>,
    /// Welded vertex per corner.
    vert: Vec<u32>,
    /// Input corner per corner.
    src: Vec<u32>,
    /// Input face per face.
    face_src: Vec<u32>,
}

impl Faces {
    fn count(&self) -> usize {
        self.start.len()
    }

    fn range(&self, f: usize) -> std::ops::Range<usize> {
        self.start[f] as usize..(self.start[f] + self.len[f]) as usize
    }

    /// The face of every corner.
    ///
    /// Complexity: O(C).
    fn face_of(&self) -> Vec<u32> {
        (0..self.count()).flat_map(|f| std::iter::repeat_n(f as u32, self.len[f] as usize)).collect()
    }

    /// The corner after `c` in its face `f`.
    fn next(&self, f: usize, c: usize) -> usize {
        let end = (self.start[f] + self.len[f]) as usize;
        if c + 1 == end { self.start[f] as usize } else { c + 1 }
    }

    fn push(&mut self, corners: &[(u32, u32)], face: u32) {
        self.start.push(self.vert.len() as u32);
        self.len.push(corners.len() as u32);
        self.vert.extend(corners.iter().map(|c| c.0));
        self.src.extend(corners.iter().map(|c| c.1));
        self.face_src.push(face);
    }
}

/// Welds positions: identical ones (with `weld` = 0) or those within
/// `weld` of a kept one (a uniform grid, earlier vertices kept). Returns
/// the kept index of every vertex.
///
/// Complexity: O(V) expected.
fn weld_map(positions: &[Vec3], weld: f32) -> Vec<u32> {
    let key = |p: Vec3| (p + Vec3::ZERO).to_array().map(f32::to_bits);
    if weld <= 0.0 {
        let mut first: FxHashMap<[u32; 3], u32> = FxHashMap::default();
        return positions.iter().enumerate().map(|(i, &p)| *first.entry(key(p)).or_insert(i as u32)).collect();
    }
    let cell = |p: Vec3| (p / weld).floor().as_ivec3();
    let mut grid: FxHashMap<glam::IVec3, SmallVec<[u32; 2]>> = FxHashMap::default();
    let mut out = Vec::with_capacity(positions.len());
    for (i, &p) in positions.iter().enumerate() {
        let c = cell(p);
        let mut found = None;
        'search: for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    for &k in grid.get(&(c + glam::IVec3::new(dx, dy, dz))).into_iter().flatten() {
                        if positions[k as usize].distance(p) <= weld {
                            found = Some(k);
                            break 'search;
                        }
                    }
                }
            }
        }
        match found {
            Some(k) => out.push(k),
            None => {
                grid.entry(c).or_default().push(i as u32);
                out.push(i as u32);
            }
        }
    }
    out
}

/// The undirected edges of the faces, each with the corners that start it
/// (a corner starts the edge to the next corner of its face).
///
/// Complexity: O(C) expected.
fn edge_uses(f: &Faces, skip: &FxHashSet<u32>) -> FxHashMap<(u32, u32), SmallVec<[u32; 2]>> {
    let mut map: FxHashMap<(u32, u32), SmallVec<[u32; 2]>> = FxHashMap::default();
    for i in 0..f.count() {
        if skip.contains(&(i as u32)) {
            continue;
        }
        let r = f.range(i);
        for c in r.clone() {
            let n = if c + 1 == r.end { r.start } else { c + 1 };
            let (a, b) = (f.vert[c], f.vert[n]);
            map.entry((a.min(b), a.max(b))).or_default().push(c as u32);
        }
    }
    map
}

/// Repairs polygon soup (positions, face sizes and a flat corner list) into
/// a 2-manifold mesh. See the module docs.
///
/// Complexity: O(C) expected for C corners, times the rounds of detaching
/// (one or two in practice, at most [`MAX_ROUNDS`]).
pub fn repair(positions: &[Vec3], face_sizes: &[u32], corners: &[u32], spec: &SoupSpec) -> Result<Repaired, SoupError> {
    let expected: usize = face_sizes.iter().map(|&s| s as usize).sum();
    if expected != corners.len() {
        return Err(SoupError::CornerCount { expected, got: corners.len() });
    }
    if positions.len() >= u32::MAX as usize / 2 || corners.len() >= u32::MAX as usize / 2 {
        return Err(SoupError::TooLarge);
    }
    let mut report = SoupReport::default();
    let welded = weld_map(positions, spec.weld);

    // Faces over welded vertices: no repeated corners, at least three.
    let mut faces = Faces::default();
    let mut seen_faces: FxHashSet<SmallVec<[u32; 8]>> = FxHashSet::default();
    let mut at = 0usize;
    let mut buf: Vec<(u32, u32)> = Vec::new();
    for (fi, &size) in face_sizes.iter().enumerate() {
        let run = at..at + size as usize;
        at += size as usize;
        buf.clear();
        let mut finite = true;
        for c in run {
            let v = corners[c];
            let Some(p) = positions.get(v as usize) else {
                return Err(SoupError::VertexOutOfRange { face: fi as u32, vert: v, count: positions.len() as u32 });
            };
            finite &= p.is_finite();
            let w = welded[v as usize];
            if buf.last().is_none_or(|l| l.0 != w) {
                buf.push((w, c as u32));
            }
        }
        while buf.len() > 1 && buf.first().map(|x| x.0) == buf.last().map(|x| x.0) {
            buf.pop();
        }
        if !finite || buf.len() < 3 {
            report.degenerate += 1;
            continue;
        }
        let pieces = simple_polygons(&buf);
        if pieces.is_empty() {
            report.degenerate += 1;
        }
        for piece in pieces {
            let mut key: SmallVec<[u32; 8]> = piece.iter().map(|c| c.0).collect();
            key.sort_unstable();
            if !seen_faces.insert(key) {
                report.duplicates += 1;
                continue;
            }
            faces.push(&piece, fi as u32);
        }
    }
    drop(seen_faces);
    if faces.count() == 0 {
        return Err(SoupError::Empty);
    }

    // Winding: walk each part across edges with exactly two faces and turn
    // faces to agree; the majority of each part keeps its winding. An edge
    // where the walk disagrees with itself is split.
    let uses = edge_uses(&faces, &FxHashSet::default());
    let face_of = faces.face_of();
    let forward = |c: u32| faces.vert[c as usize] < faces.vert[faces.next(face_of[c as usize] as usize, c as usize)];
    let mut flip: Vec<Option<bool>> = vec![None; faces.count()];
    let mut conflict: FxHashSet<(u32, u32)> = FxHashSet::default();
    let mut stack = Vec::new();
    for seed in 0..faces.count() {
        if flip[seed].is_some() {
            continue;
        }
        flip[seed] = Some(false);
        let mut part = vec![seed];
        stack.push(seed);
        while let Some(f) = stack.pop() {
            let ff = flip[f].unwrap_or(false);
            let r = faces.range(f);
            for c in r.clone() {
                let n = if c + 1 == r.end { r.start } else { c + 1 };
                let (a, b) = (faces.vert[c], faces.vert[n]);
                let key = (a.min(b), a.max(b));
                let Some(list) = uses.get(&key) else { continue };
                if list.len() != 2 {
                    continue;
                }
                let other = if list[0] as usize == c { list[1] } else { list[0] };
                let g = face_of[other as usize] as usize;
                let want = ff ^ (forward(c as u32) == forward(other));
                match flip[g] {
                    None => {
                        flip[g] = Some(want);
                        part.push(g);
                        stack.push(g);
                    }
                    Some(have) if have != want => {
                        conflict.insert(key);
                    }
                    Some(_) => {}
                }
            }
        }
        let turned = part.iter().filter(|&&f| flip[f] == Some(true)).count();
        if 2 * turned > part.len() {
            for &f in &part {
                flip[f] = flip[f].map(|x| !x);
            }
        }
    }
    for (f, fl) in flip.iter().enumerate() {
        if *fl == Some(true) {
            let r = faces.range(f);
            faces.vert[r.clone()].reverse();
            faces.src[r].reverse();
            report.flipped += 1;
        }
    }
    report.split_edges = conflict.len() + uses.values().filter(|l| l.len() > 2).count();
    drop(uses);

    // Fans: corners around a vertex join across shared edges; every fan
    // after the first becomes a vertex copy. Faces that still break the
    // surface are detached and the fans worked out again.
    let mut detached: FxHashSet<u32> = FxHashSet::default();
    for _ in 0..MAX_ROUNDS {
        let (verts, copies) = assign_fans(&faces, &conflict, &detached);
        let bad = broken_faces(&faces, &verts);
        if bad.is_empty() {
            report.split_verts = copies;
            report.detached = detached.len();
            report.welded = count_welded(&welded, corners);
            return build(positions, &welded, &faces, &verts, report);
        }
        detached.extend(bad);
    }
    Err(SoupError::Unrepairable("faces keep sharing edges after splitting".into()))
}

/// Inputs vertices that merged into another one.
///
/// Complexity: O(C + V).
fn count_welded(welded: &[u32], corners: &[u32]) -> usize {
    let mut used = vec![false; welded.len()];
    for &c in corners {
        used[c as usize] = true;
    }
    (0..welded.len()).filter(|&v| used[v] && welded[v] != v as u32).count()
}

/// Output vertex per corner: corners joined across shared edges (exactly
/// two faces, opposite directions, not split, not detached) share one.
/// Returns the vertex ids (welded ids, then copies numbered from the input
/// size up) and the number of copies.
///
/// Complexity: O(C) expected.
fn assign_fans(faces: &Faces, conflict: &FxHashSet<(u32, u32)>, detached: &FxHashSet<u32>) -> (Vec<u32>, usize) {
    let uses = edge_uses(faces, detached);
    let mut dsu = Dsu((0..faces.vert.len() as u32).collect());
    let face_of = faces.face_of();
    let next_of = |c: usize| faces.next(face_of[c] as usize, c);
    for (key, list) in &uses {
        if list.len() != 2 || conflict.contains(key) {
            continue;
        }
        let (c, d) = (list[0] as usize, list[1] as usize);
        let (cn, dn) = (next_of(c), next_of(d));
        // Opposite directions: c runs a -> b, d runs b -> a.
        if faces.vert[c] != faces.vert[dn] || faces.vert[cn] != faces.vert[d] {
            continue;
        }
        dsu.union(c as u32, dn as u32);
        dsu.union(cn as u32, d as u32);
    }
    let base = faces.vert.iter().copied().max().map_or(0, |m| m + 1);
    let mut id_of_root: FxHashMap<u32, u32> = FxHashMap::default();
    let mut claimed: FxHashSet<u32> = FxHashSet::default();
    let mut next = base;
    let mut copies = 0;
    let mut out = Vec::with_capacity(faces.vert.len());
    for c in 0..faces.vert.len() {
        let root = dsu.find(c as u32);
        let id = *id_of_root.entry(root).or_insert_with(|| {
            let v = faces.vert[c];
            if claimed.insert(v) {
                v
            } else {
                copies += 1;
                next += 1;
                next - 1
            }
        });
        out.push(id);
    }
    (out, copies)
}

/// Faces on an edge used twice in one direction or by three or more faces.
///
/// Complexity: O(C) expected.
fn broken_faces(faces: &Faces, verts: &[u32]) -> Vec<u32> {
    let mut directed: FxHashMap<(u32, u32), SmallVec<[u32; 1]>> = FxHashMap::default();
    for f in 0..faces.count() {
        let r = faces.range(f);
        for c in r.clone() {
            let n = if c + 1 == r.end { r.start } else { c + 1 };
            directed.entry((verts[c], verts[n])).or_default().push(f as u32);
        }
    }
    let mut bad: Vec<u32> = Vec::new();
    for (&(a, b), list) in &directed {
        let back = directed.get(&(b, a)).map_or(0, |l| l.len());
        if list.len() > 1 || list.len() + back > 2 {
            bad.extend(list.iter().copied());
        }
    }
    bad.sort_unstable();
    bad.dedup();
    bad
}

/// The mesh over the assigned vertices (unused ones dropped).
///
/// Complexity: O(V + C).
fn build(positions: &[Vec3], welded: &[u32], faces: &Faces, verts: &[u32], report: SoupReport) -> Result<Repaired, SoupError> {
    // A copy's position is its welded vertex's.
    let mut origin_of: FxHashMap<u32, u32> = FxHashMap::default();
    for (c, &v) in verts.iter().enumerate() {
        origin_of.entry(v).or_insert(welded[faces.vert[c] as usize]);
    }
    let mut map: FxHashMap<u32, u32> = FxHashMap::default();
    let mut out_pos = Vec::new();
    let mut out_corners = Vec::with_capacity(verts.len());
    for &v in verts {
        let id = *map.entry(v).or_insert_with(|| {
            out_pos.push(positions[origin_of[&v] as usize]);
            out_pos.len() as u32 - 1
        });
        out_corners.push(id);
    }
    let mesh = Mesh::from_polygons(out_pos, &faces.len, &out_corners).map_err(|e: BuildError| SoupError::Unrepairable(e.to_string()))?;
    Ok(Repaired { mesh, corner_src: faces.src.clone(), face_src: faces.face_src.clone(), report })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    /// A unit cube as soup with every face's corners separate (like glTF
    /// with flat normals): 24 positions, 6 quads.
    fn split_cube() -> (Vec<Vec3>, Vec<u32>, Vec<u32>) {
        let quads = [
            [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]],
            [[1, 0, 0], [0, 0, 0], [0, 1, 0], [1, 1, 0]],
            [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]],
            [[1, 0, 1], [1, 0, 0], [1, 1, 0], [1, 1, 1]],
            [[0, 1, 1], [1, 1, 1], [1, 1, 0], [0, 1, 0]],
            [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]],
        ];
        let pos: Vec<Vec3> = quads.iter().flatten().map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)).collect();
        (pos, vec![4; 6], (0..24).collect())
    }

    fn closed(m: &Mesh) -> bool {
        m.edges().all(|e| !m.is_boundary_edge(e))
    }

    #[test]
    fn split_vertices_weld_into_a_closed_cube() {
        let (p, s, c) = split_cube();
        let r = repair(&p, &s, &c, &SoupSpec::default()).unwrap();
        assert_eq!((r.mesh.vert_count(), r.mesh.face_count()), (8, 6));
        assert!(closed(&r.mesh));
        assert_eq!(r.report.welded, 16);
        assert!(r.report.is_clean());
        assert_eq!(r.corner_src.len(), 24);
        assert_eq!(r.face_src, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn mixed_winding_is_unified_by_majority() {
        let (p, s, mut c) = split_cube();
        c[4..8].reverse();
        c[8..12].reverse();
        let r = repair(&p, &s, &c, &SoupSpec::default()).unwrap();
        assert!(closed(&r.mesh));
        assert_eq!(r.report.flipped, 2);
        // The flipped faces' corners point back at the input corners, reversed.
        assert_eq!(&r.corner_src[4..8], &[7, 6, 5, 4]);
    }

    #[test]
    fn degenerate_duplicate_and_book_faces() {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, Vec3::NEG_Y, Vec3::new(1.0, 1.0, 0.0)];
        // Three triangles on edge 0-1 (a book), a duplicate, a degenerate one.
        let s = vec![3, 3, 3, 3, 3];
        let c = vec![0, 1, 2, 1, 0, 3, 0, 1, 4, 2, 0, 1, 5, 5, 0];
        let r = repair(&p, &s, &c, &SoupSpec::default()).unwrap();
        assert_eq!(r.report.duplicates, 1);
        assert_eq!(r.report.degenerate, 1);
        assert_eq!(r.report.split_edges, 1);
        assert_eq!(r.mesh.face_count(), 3);
        assert_eq!(r.mesh.validate(), Ok(()));
    }

    #[test]
    fn bowtie_vertices_split() {
        // Two triangles touching at vertex 0 only.
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::NEG_X, Vec3::NEG_Y];
        let r = repair(&p, &[3, 3], &[0, 1, 2, 0, 3, 4], &SoupSpec::default()).unwrap();
        assert_eq!(r.report.split_verts, 1);
        assert_eq!(r.mesh.vert_count(), 6);
    }

    #[test]
    fn near_vertices_weld_with_a_distance() {
        let (mut p, s, c) = split_cube();
        p[5].x += 1e-5;
        let exact = repair(&p, &s, &c, &SoupSpec::default()).unwrap();
        assert!(!closed(&exact.mesh));
        let near = repair(&p, &s, &c, &SoupSpec { weld: 1e-4 }).unwrap();
        assert!(closed(&near.mesh));
        assert_eq!(near.mesh.vert_count(), 8);
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(matches!(repair(&[Vec3::ZERO], &[3], &[0, 0], &SoupSpec::default()), Err(SoupError::CornerCount { .. })));
        assert!(matches!(repair(&[Vec3::ZERO], &[3], &[0, 1, 2], &SoupSpec::default()), Err(SoupError::VertexOutOfRange { .. })));
        assert_eq!(repair(&[Vec3::ZERO, Vec3::X], &[3], &[0, 1, 1], &SoupSpec::default()).unwrap_err(), SoupError::Empty);
    }
}
