//! Per-face retopology: poke (fan to a center), triangulate (beauty) and
//! tris to quads (greedy pairing).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::{Vec2, Vec3};
use p3d_mesh::{CornerSrc, EdgeFlags, MAX_EAR_CLIP_CORNERS, Mesh};
use rustc_hash::FxHashMap;

use super::{KernelError, face_bits, live_faces, patch};

/// Replaces each face with a fan of triangles around a new center vertex,
/// moved by `offset` along the face normal. Returns the new triangles.
///
/// Complexity: O(c log c) for the c corners of the faces.
pub fn poke(mesh: &mut Mesh, faces: &[u32], offset: f32) -> Result<Vec<u32>, KernelError> {
    let faces = live_faces(mesh, faces)?;
    if !offset.is_finite() {
        return Err(KernelError::OutOfRange { param: "offset", message: "offset must be finite".into(), hint: "use 0 to keep faces flat".into() });
    }
    let mut p = patch(mesh);
    for &f in &faces {
        let hs: Vec<u32> = mesh.face_halfedges(f).collect();
        let c = p.new_vert(mesh.face_centroid(f) + mesh.face_normal(f) * offset, None);
        // The center's attributes: halfway between two opposite corners.
        let mid = CornerSrc::lerp(hs[0], hs[hs.len() / 2], 0.5);
        p.remove_face(f);
        for i in 0..hs.len() {
            let (a, b) = (hs[i], hs[(i + 1) % hs.len()]);
            p.add_face(&[mesh.origin(a), mesh.origin(b), c], &[CornerSrc::copy(a), CornerSrc::copy(b), mid], mesh.face_material(f), Some(f));
        }
    }
    Ok(mesh.apply_patch(&p)?.faces)
}

/// An orthonormal pair spanning the plane with normal `n`.
fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let n = n.normalize_or(Vec3::Z);
    let u = n.any_orthonormal_vector();
    (u, n.cross(u))
}

/// z of the 2D cross product (b - a) x (c - a).
fn orient(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

/// Smallest corner angle of a triangle (radians).
fn min_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let ang = |o: Vec3, x: Vec3, y: Vec3| (x - o).angle_between(y - o);
    ang(a, b, c).min(ang(b, c, a)).min(ang(c, a, b))
}

/// Triangulates one face into local corner indices, then flips inner
/// diagonals while that raises the smaller of the two triangles' minimum
/// angles (Lawson flips, done in the face's plane), giving the "beauty"
/// triangulation. Faces above [`MAX_EAR_CLIP_CORNERS`] corners keep the
/// kernel's triangulation.
///
/// Complexity: O(k^2) for k <= MAX_EAR_CLIP_CORNERS corners.
fn beauty_triangles(mesh: &Mesh, f: u32) -> Vec<[usize; 3]> {
    let hs: Vec<u32> = mesh.face_halfedges(f).collect();
    let k = hs.len();
    let local: FxHashMap<u32, usize> = hs.iter().enumerate().map(|(i, &h)| (h, i)).collect();
    let mut tris: Vec<[usize; 3]> = mesh.face_triangles(f).iter().map(|t| t.map(|h| local[&h])).collect();
    if k <= 3 || k > MAX_EAR_CLIP_CORNERS {
        return tris;
    }
    let p3: Vec<Vec3> = hs.iter().map(|&h| mesh.positions[mesh.origin(h) as usize]).collect();
    let (u, v) = plane_basis(mesh.face_normal(f));
    let p2: Vec<Vec2> = p3.iter().map(|p| Vec2::new(p.dot(u), p.dot(v))).collect();
    // Directed edge -> (triangle, index of the opposite corner).
    let mut owner: FxHashMap<(usize, usize), (usize, usize)> = FxHashMap::default();
    let set = |owner: &mut FxHashMap<(usize, usize), (usize, usize)>, t: usize, tri: [usize; 3]| {
        for e in 0..3 {
            owner.insert((tri[e], tri[(e + 1) % 3]), (t, (e + 2) % 3));
        }
    };
    for (t, tri) in tris.iter().enumerate() {
        set(&mut owner, t, *tri);
    }
    // Lawson flips from a stack of edges to check; a flip pushes the four
    // edges around it. Flips are capped at k^2 for degenerate input.
    let mut stack: Vec<(usize, usize)> = owner.keys().copied().filter(|&(a, b)| a < b).collect();
    stack.sort_unstable();
    let mut flips = 0usize;
    while let Some((a, b)) = stack.pop() {
        let (Some(&(t1, o1)), Some(&(t2, o2))) = (owner.get(&(a, b)), owner.get(&(b, a))) else { continue };
        let (c, d) = (tris[t1][o1], tris[t2][o2]);
        // The new diagonal c-d must cross a-b inside the quad.
        let crosses =
            orient(p2[c], p2[d], p2[a]) * orient(p2[c], p2[d], p2[b]) < 0.0 && orient(p2[a], p2[b], p2[c]) * orient(p2[a], p2[b], p2[d]) < 0.0;
        if !crosses {
            continue;
        }
        let before = min_angle(p3[a], p3[b], p3[c]).min(min_angle(p3[b], p3[a], p3[d]));
        let after = min_angle(p3[c], p3[d], p3[b]).min(min_angle(p3[d], p3[c], p3[a]));
        if after > before + 1e-6 && flips < k * k {
            // (a, b, c) and (b, a, d) become (c, d, b) and (d, c, a), same winding.
            owner.remove(&(a, b));
            owner.remove(&(b, a));
            tris[t1] = [c, d, b];
            tris[t2] = [d, c, a];
            set(&mut owner, t1, tris[t1]);
            set(&mut owner, t2, tris[t2]);
            stack.extend([(a, c), (c, b), (b, d), (d, a)].map(|(x, y)| (x.min(y), x.max(y))));
            flips += 1;
        }
    }
    tris
}

/// Splits faces with more than three corners into triangles, choosing
/// diagonals that keep angles large (beauty). Triangles keep the groups and
/// material of their face. Returns every resulting triangle (faces that were
/// triangles already included) and how many faces were split.
///
/// Complexity: O(c log c) for the c corners, plus O(k^2) per face of k
/// corners (k <= 256; larger faces keep a plain fan).
pub fn triangulate(mesh: &mut Mesh, faces: &[u32]) -> Result<(Vec<u32>, usize), KernelError> {
    let faces = live_faces(mesh, faces)?;
    let mut p = patch(mesh);
    let mut kept = Vec::new();
    let mut split = 0usize;
    for &f in &faces {
        if mesh.face_len(f) == 3 {
            kept.push(f);
            continue;
        }
        let hs: Vec<u32> = mesh.face_halfedges(f).collect();
        p.remove_face(f);
        for t in beauty_triangles(mesh, f) {
            let vs = t.map(|i| mesh.origin(hs[i]));
            p.add_face(&vs, &t.map(|i| CornerSrc::copy(hs[i])), mesh.face_material(f), Some(f));
        }
        split += 1;
    }
    let mut out = mesh.apply_patch(&p)?.faces;
    out.extend(kept);
    Ok((out, split))
}

/// Limits for joining triangles into quads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadSpec {
    /// Largest angle between the two triangles' normals (degrees).
    pub max_face_angle_deg: f32,
    /// Largest difference of a quad corner from 90 degrees.
    pub max_shape_angle_deg: f32,
    /// Also join across sharp edges, seams and material borders.
    pub ignore_boundaries: bool,
}

impl Default for QuadSpec {
    fn default() -> Self {
        QuadSpec { max_face_angle_deg: 40.0, max_shape_angle_deg: 40.0, ignore_boundaries: false }
    }
}

/// A candidate join: lower error first, then lower edge index.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    error: f32,
    edge: u32,
}

impl Eq for Candidate {}

impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        // BinaryHeap is a max-heap: reverse for smallest error first.
        o.error.total_cmp(&self.error).then(o.edge.cmp(&self.edge))
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The quad two triangles across half-edge `h` would form, as corner
/// half-edges in winding order: for triangles (a, b, c) holding `h` = b -> c
/// and (c, b, d), the quad is a, b, d, c.
fn quad_corners(mesh: &Mesh, h: u32) -> [u32; 4] {
    let t = h ^ 1;
    // In h's triangle: h (b -> c), next(h) (c -> a), prev(h) (a -> b).
    // In t's triangle: t (c -> b), next(t) (b -> d), prev(t) (d -> c).
    [mesh.prev(h), mesh.next(t), mesh.prev(t), mesh.next(h)]
}

/// Joins pairs of adjacent triangles into quads, best-shaped pairs first
/// (greedy matching from a priority queue). A pair qualifies when both are
/// selected triangles, their normals differ by at most the face angle, every
/// quad corner is within the shape angle of 90 degrees and the quad is
/// convex; by default sharp edges, seams and material borders are kept.
/// Returns the new quads.
///
/// Complexity: O(n log n) for n selected triangles.
pub fn tris_to_quads(mesh: &mut Mesh, faces: &[u32], spec: &QuadSpec) -> Result<Vec<u32>, KernelError> {
    let faces = live_faces(mesh, faces)?;
    let tri_faces: Vec<u32> = faces.into_iter().filter(|&f| mesh.face_len(f) == 3).collect();
    if tri_faces.is_empty() {
        return Ok(Vec::new());
    }
    let sel = face_bits(mesh, &tri_faces);
    let face_cos = spec.max_face_angle_deg.clamp(0.0, 180.0).to_radians().cos();
    let shape = spec.max_shape_angle_deg.clamp(0.0, 90.0).to_radians();
    let mut heap = BinaryHeap::new();
    for &f in &tri_faces {
        for h in mesh.face_halfedges(f) {
            let Some(g) = mesh.face(h ^ 1) else { continue };
            // Each edge once, from its lower-numbered face.
            if g < f || !sel.contains(g as usize) {
                continue;
            }
            let e = h >> 1;
            if !spec.ignore_boundaries
                && (mesh.edge_flags(e).contains(EdgeFlags::SHARP)
                    || mesh.edge_flags(e).contains(EdgeFlags::SEAM)
                    || mesh.face_material(f) != mesh.face_material(g))
            {
                continue;
            }
            let (nf, ng) = (mesh.face_normal(f), mesh.face_normal(g));
            let dot = nf.dot(ng);
            if dot < face_cos {
                continue;
            }
            let q = quad_corners(mesh, h).map(|c| mesh.positions[mesh.origin(c) as usize]);
            let n = (nf + ng).normalize_or_zero();
            let mut worst = 0.0f32;
            let mut convex = true;
            for i in 0..4 {
                let (a, o, b) = (q[(i + 3) % 4], q[i], q[(i + 1) % 4]);
                convex &= (b - o).cross(a - o).dot(n) > 0.0;
                worst = worst.max(((b - o).angle_between(a - o) - std::f32::consts::FRAC_PI_2).abs());
            }
            if convex && worst <= shape {
                heap.push(Candidate { error: worst + dot.clamp(-1.0, 1.0).acos(), edge: e });
            }
        }
    }
    let mut used = FxHashMap::default();
    let mut p = patch(mesh);
    while let Some(c) = heap.pop() {
        let h = 2 * c.edge;
        let (Some(f), Some(g)) = (mesh.face(h), mesh.face(h ^ 1)) else { continue };
        if used.contains_key(&f) || used.contains_key(&g) {
            continue;
        }
        used.insert(f, ());
        used.insert(g, ());
        let q = quad_corners(mesh, h);
        p.remove_face(f);
        p.remove_face(g);
        p.add_face(&q.map(|c| mesh.origin(c)), &q.map(CornerSrc::copy), mesh.face_material(f), Some(f));
    }
    if p.is_empty() {
        return Ok(Vec::new());
    }
    Ok(mesh.apply_patch(&p)?.faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{CapKind, CylinderSpec, Origin, PlaneSpec, cylinder, plane};
    use p3d_mesh::ElementKind;

    #[test]
    fn poke_makes_fans() {
        let mut m = plane(&PlaneSpec { segments_x: 2, segments_z: 1, ..PlaneSpec::default() }).unwrap();
        m.set_group("left", ElementKind::Face, [0]).unwrap();
        let t = poke(&mut m, &[0], 0.1).unwrap();
        m.validate().unwrap();
        assert_eq!((t.len(), m.face_count(), m.vert_count()), (4, 5, 7));
        assert_eq!(m.group("left").unwrap().count(), 4);
        let top = m.verts().map(|v| m.positions[v as usize].y).fold(f32::MIN, f32::max);
        assert!((top - 0.1).abs() < 1e-6);
    }

    #[test]
    fn triangulate_ngons_with_good_angles() {
        let mut m = cylinder(&CylinderSpec { segments: 12, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Center).unwrap();
        let faces: Vec<u32> = m.faces().collect();
        let tris_before = m.triangle_count();
        let (tris, split) = triangulate(&mut m, &faces).unwrap();
        m.validate().unwrap();
        assert_eq!(split, 14);
        assert_eq!(tris.len(), tris_before);
        assert!(m.faces().all(|f| m.face_len(f) == 3));
        assert_eq!(m.group("cap_top").unwrap().count(), 10, "triangles keep their face's groups");
    }

    #[test]
    fn beauty_beats_a_fan_on_long_polygons() {
        // An octagon on a 4:1 ellipse: a fan from one corner makes slivers.
        let p: Vec<Vec3> = (0..8).map(|i| (i as f32) * std::f32::consts::TAU / 8.0).map(|a| Vec3::new(4.0 * a.cos(), a.sin(), 0.0)).collect();
        let m = Mesh::from_polygons(p, &[8], &(0..8).collect::<Vec<u32>>()).unwrap();
        let angle_of = |tris: &[[Vec3; 3]]| tris.iter().map(|t| min_angle(t[0], t[1], t[2])).fold(f32::MAX, f32::min);
        let pos = |h: u32| m.positions[m.origin(h) as usize];
        let fan: Vec<[Vec3; 3]> = m.face_triangles(0).iter().map(|t| t.map(pos)).collect();
        let hs: Vec<u32> = m.face_halfedges(0).collect();
        let nice: Vec<[Vec3; 3]> = beauty_triangles(&m, 0).iter().map(|t| t.map(|i| pos(hs[i]))).collect();
        assert_eq!(nice.len(), 6);
        assert!(angle_of(&nice) > angle_of(&fan) * 1.5, "{} vs {}", angle_of(&nice).to_degrees(), angle_of(&fan).to_degrees());
    }

    #[test]
    fn tris_to_quads_round_trips_a_grid() {
        let mut m = plane(&PlaneSpec { segments_x: 4, segments_z: 4, ..PlaneSpec::default() }).unwrap();
        let all: Vec<u32> = m.faces().collect();
        triangulate(&mut m, &all).unwrap();
        assert_eq!(m.face_count(), 32);
        let all: Vec<u32> = m.faces().collect();
        let q = tris_to_quads(&mut m, &all, &QuadSpec::default()).unwrap();
        m.validate().unwrap();
        assert_eq!((q.len(), m.face_count()), (16, 16));
        assert!(m.faces().all(|f| m.face_len(f) == 4));
    }

    #[test]
    fn tris_to_quads_respects_sharp_edges_and_shape() {
        let mut m = plane(&PlaneSpec { segments_x: 1, segments_z: 1, ..PlaneSpec::default() }).unwrap();
        triangulate(&mut m, &[0]).unwrap();
        let diag = m.edges().find(|&e| !m.is_boundary_edge(e)).unwrap();
        m.set_edge_flag(diag, EdgeFlags::SHARP, true);
        let all: Vec<u32> = m.faces().collect();
        assert!(tris_to_quads(&mut m, &all, &QuadSpec::default()).unwrap().is_empty());
        let q = tris_to_quads(&mut m, &all, &QuadSpec { ignore_boundaries: true, ..QuadSpec::default() }).unwrap();
        assert_eq!(q.len(), 1);
        m.validate().unwrap();
    }
}
