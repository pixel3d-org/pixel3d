//! Topology metrics: element counts, face kinds, poles, corner angles,
//! boundary loops, connected parts, orientation and degenerate elements.

use glam::{DVec3, Vec3};
use p3d_mesh::{FixedBitSet, INVALID, Mesh};
use serde::Serialize;

/// Edges shorter than this (1 micrometer) count as degenerate.
pub const SHORT_EDGE: f32 = 1e-6;

/// Faces whose area is below this fraction of their longest edge squared
/// count as degenerate: collapsed faces and slivers with collinear corners.
pub const SLIVER_RATIO: f32 = 1e-6;

/// Whole-mesh topology metrics, the same numbers `analyze` reports and the
/// benchmark uses.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Topology {
    pub verts: usize,
    pub edges: usize,
    pub faces: usize,
    /// Triangles after triangulating every face.
    pub tris: usize,
    pub tri_faces: usize,
    pub quad_faces: usize,
    /// Faces with more than four corners.
    pub ngon_faces: usize,
    /// Share of quads among all faces, in percent.
    pub quad_percent: f32,
    pub boundary_edges: usize,
    /// Holes and open borders: closed chains of boundary edges.
    pub boundary_loops: usize,
    /// Connected parts (faces joined across edges).
    pub parts: usize,
    /// Parts without boundary edges.
    pub closed_parts: usize,
    /// Closed parts whose normals point inward (negative volume).
    pub inverted_parts: usize,
    /// V - E + F.
    pub euler: i64,
    /// Interior vertices with valence 3.
    pub poles_3: usize,
    /// Interior vertices with valence 5.
    pub poles_5: usize,
    /// Interior vertices with valence 6 or more.
    pub poles_6_plus: usize,
    pub max_valence: usize,
    /// Smallest face corner angle in degrees (0 for an empty mesh).
    pub min_angle_deg: f32,
    /// Largest face corner angle in degrees (0 for an empty mesh).
    pub max_angle_deg: f32,
    /// Collapsed faces and slivers (see [`SLIVER_RATIO`]).
    pub degenerate_faces: usize,
    /// Edges shorter than [`SHORT_EDGE`].
    pub short_edges: usize,
    /// Surface area in square meters.
    pub area: f64,
    /// Signed volume of the closed parts in cubic meters.
    pub volume: f64,
}

impl Topology {
    /// Every part is closed (no boundary edges).
    pub fn is_closed(&self) -> bool {
        self.parts > 0 && self.boundary_edges == 0
    }

    /// Every part is closed and points outward (inner walls of hollow parts
    /// face inward by design).
    pub fn is_watertight(&self) -> bool {
        self.parts > 0 && self.boundary_edges == 0 && self.inverted_parts == 0
    }
}

/// One connected part of a mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Part {
    /// Lowest face index of the part (parts are numbered in this order).
    pub first_face: u32,
    pub faces: u32,
    pub boundary_edges: u32,
    pub closed: bool,
    /// Surface area in square meters.
    pub area: f64,
    /// Signed volume in cubic meters; meaningful when closed (negative when
    /// the normals point inward).
    pub volume: f64,
    pub min: Vec3,
    pub max: Vec3,
    /// A closed part facing inward inside the box of an outward closed part:
    /// the inner wall of a hollow object, not a mistake.
    pub cavity: bool,
}

/// Connected parts with each face's part label (`INVALID` for dead slots).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parts {
    pub labels: Vec<u32>,
    pub parts: Vec<Part>,
}

/// A closed chain of boundary edges: a hole or an open border.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BoundaryLoop {
    /// One boundary half-edge of the loop (its lowest index).
    pub start: u32,
    pub edges: u32,
    /// Length in meters.
    pub length: f32,
    /// Average of the loop's vertex positions.
    pub centroid: Vec3,
    /// Part (see [`parts`]) the loop borders.
    pub part: u32,
}

/// Is face `f` collapsed or a sliver? Also returns its area.
///
/// Complexity: O(face size).
pub fn face_degenerate(mesh: &Mesh, f: u32) -> (bool, f32) {
    let area = mesh.face_area(f);
    let longest = mesh.face_halfedges(f).map(|h| mesh.edge_length(h >> 1)).fold(0.0f32, f32::max);
    (longest < SHORT_EDGE || area <= SLIVER_RATIO * longest * longest, area)
}

/// Connected parts with area, signed volume, bounds and openness.
///
/// Complexity: O(F + C) for F face slots and C corners.
pub fn parts(mesh: &Mesh) -> Parts {
    let (labels, n) = mesh.face_components();
    // Volumes are taken about the mesh center: translation invariant for
    // closed parts and better conditioned than the origin.
    let center = mesh.bounds().map_or(DVec3::ZERO, |(lo, hi)| ((lo + hi) / 2.0).as_dvec3());
    let mut parts: Vec<Part> = (0..n)
        .map(|_| Part {
            first_face: INVALID,
            faces: 0,
            boundary_edges: 0,
            closed: true,
            area: 0.0,
            volume: 0.0,
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
            cavity: false,
        })
        .collect();
    for f in mesh.faces() {
        let p = &mut parts[labels[f as usize] as usize];
        if p.first_face == INVALID {
            p.first_face = f;
        }
        p.faces += 1;
        p.area += f64::from(mesh.face_area(f));
        let mut hs = mesh.face_halfedges(f);
        let Some(h0) = hs.next() else { continue };
        let a = mesh.positions[mesh.origin(h0) as usize].as_dvec3() - center;
        p.min = p.min.min(mesh.positions[mesh.origin(h0) as usize]);
        p.max = p.max.max(mesh.positions[mesh.origin(h0) as usize]);
        if mesh.is_boundary_he(h0 ^ 1) {
            p.boundary_edges += 1;
        }
        // Fan triangles (a, b, c) of the face; exact for planar and non-planar faces.
        for h in hs {
            let v = mesh.positions[mesh.origin(h) as usize];
            p.min = p.min.min(v);
            p.max = p.max.max(v);
            if mesh.is_boundary_he(h ^ 1) {
                p.boundary_edges += 1;
            }
            let b = v.as_dvec3() - center;
            let c = mesh.positions[mesh.dest(h) as usize].as_dvec3() - center;
            if mesh.dest(h) != mesh.origin(h0) {
                p.volume += a.dot(b.cross(c)) / 6.0;
            }
        }
    }
    for p in &mut parts {
        p.closed = p.boundary_edges == 0;
    }
    mark_cavities(&mut parts);
    Parts { labels, parts }
}

/// Marks inward-facing closed parts that sit inside the box of an
/// outward-facing closed part as cavities.
///
/// Complexity: O(k log k + overlapping box pairs) for k parts.
fn mark_cavities(parts: &mut [Part]) {
    if !parts.iter().any(|p| p.closed && p.volume < 0.0) {
        return;
    }
    let boxes: Vec<(Vec3, Vec3)> = parts.iter().map(|p| (p.min, p.max)).collect();
    let (pairs, _) = p3d_geo::bvh::box_pairs(&boxes, 0.0, 1 << 20);
    for (a, b) in pairs {
        for (inner, outer) in [(a, b), (b, a)] {
            let (i, o) = (&parts[inner], &parts[outer]);
            if i.closed && o.closed && i.volume < 0.0 && o.volume > 0.0 && i.min.cmpge(o.min).all() && i.max.cmple(o.max).all() {
                parts[inner].cavity = true;
            }
        }
    }
}

/// Every boundary loop, ordered by its lowest half-edge index.
///
/// Complexity: O(H) for H half-edge slots.
pub fn boundary_loops(mesh: &Mesh, parts: &Parts) -> Vec<BoundaryLoop> {
    let mut seen = FixedBitSet::with_capacity(mesh.he_slots());
    let mut out = Vec::new();
    for h in 0..mesh.he_slots() as u32 {
        if !mesh.is_he_alive(h) || !mesh.is_boundary_he(h) || seen.contains(h as usize) {
            continue;
        }
        let mut lp = BoundaryLoop { start: h, edges: 0, length: 0.0, centroid: Vec3::ZERO, part: INVALID };
        let mut sum = DVec3::ZERO;
        let mut cur = h;
        // A boundary chain is a simple cycle (one boundary fan per vertex); the
        // guard only protects against corrupt input.
        for _ in 0..mesh.he_slots() {
            seen.insert(cur as usize);
            lp.edges += 1;
            lp.length += mesh.edge_length(cur >> 1);
            sum += mesh.positions[mesh.origin(cur) as usize].as_dvec3();
            if lp.part == INVALID
                && let Some(f) = mesh.face(cur ^ 1)
            {
                lp.part = parts.labels.get(f as usize).copied().unwrap_or(INVALID);
            }
            cur = mesh.next(cur);
            if cur == h {
                break;
            }
        }
        lp.centroid = (sum / f64::from(lp.edges.max(1))).as_vec3();
        out.push(lp);
    }
    out
}

/// Number of boundary loops (holes and open borders).
///
/// Complexity: O(H) for H half-edge slots.
pub fn boundary_loop_count(mesh: &Mesh) -> usize {
    let mut seen = FixedBitSet::with_capacity(mesh.he_slots());
    let mut n = 0;
    for h in 0..mesh.he_slots() as u32 {
        if !mesh.is_he_alive(h) || !mesh.is_boundary_he(h) || seen.contains(h as usize) {
            continue;
        }
        n += 1;
        let mut cur = h;
        // Boundary loops are simple cycles; stop at any revisit on corrupt input.
        while !seen.contains(cur as usize) {
            seen.insert(cur as usize);
            cur = mesh.next(cur);
        }
    }
    n
}

/// Topology metrics of a whole mesh.
///
/// Complexity: O(V + E + F + C) in element slots.
pub fn topology(mesh: &Mesh) -> Topology {
    let ps = parts(mesh);
    let loops = boundary_loops(mesh, &ps);
    let mut t = Topology {
        verts: mesh.vert_count(),
        edges: mesh.edge_count(),
        faces: mesh.face_count(),
        parts: ps.parts.len(),
        boundary_loops: loops.len(),
        min_angle_deg: if mesh.face_count() > 0 { 180.0 } else { 0.0 },
        ..Topology::default()
    };
    for f in mesh.faces() {
        let n = mesh.face_len(f);
        t.tris += n.saturating_sub(2);
        match n {
            3 => t.tri_faces += 1,
            4 => t.quad_faces += 1,
            _ => t.ngon_faces += 1,
        }
        if face_degenerate(mesh, f).0 {
            t.degenerate_faces += 1;
        }
        for h in mesh.face_halfedges(f) {
            let a = mesh.corner_angle(h).to_degrees();
            if a.is_finite() {
                t.min_angle_deg = t.min_angle_deg.min(a);
                t.max_angle_deg = t.max_angle_deg.max(a);
            }
        }
    }
    t.quad_percent = if t.faces == 0 { 0.0 } else { 100.0 * t.quad_faces as f32 / t.faces as f32 };
    for e in mesh.edges() {
        if mesh.is_boundary_edge(e) {
            t.boundary_edges += 1;
        }
        if mesh.edge_length(e) < SHORT_EDGE {
            t.short_edges += 1;
        }
    }
    for v in mesh.verts() {
        let val = mesh.valence(v);
        t.max_valence = t.max_valence.max(val);
        if !mesh.is_boundary_vert(v) {
            match val {
                3 => t.poles_3 += 1,
                5 => t.poles_5 += 1,
                6.. => t.poles_6_plus += 1,
                _ => {}
            }
        }
    }
    for p in &ps.parts {
        t.area += p.area;
        if p.closed {
            t.volume += p.volume;
            if p.volume < 0.0 && !p.cavity {
                t.inverted_parts += 1;
            }
            t.closed_parts += 1;
        }
    }
    t.euler = t.verts as i64 - t.edges as i64 + t.faces as i64;
    t
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use p3d_mesh::Mesh;

    use super::*;

    fn cube(offset: Vec3, size: f32) -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) * size + offset).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    #[test]
    fn closed_cube() {
        let t = topology(&cube(Vec3::ZERO, 2.0));
        assert_eq!((t.verts, t.edges, t.faces, t.tris, t.quad_faces), (8, 12, 6, 12, 6));
        assert_eq!((t.parts, t.closed_parts, t.inverted_parts, t.boundary_loops, t.euler), (1, 1, 0, 0, 2));
        assert_eq!((t.poles_3, t.poles_5, t.poles_6_plus, t.max_valence), (8, 0, 0, 3));
        assert!((t.volume - 8.0).abs() < 1e-9 && (t.area - 24.0).abs() < 1e-9);
        assert!((t.min_angle_deg - 90.0).abs() < 1e-3 && (t.max_angle_deg - 90.0).abs() < 1e-3);
        assert_eq!(t.quad_percent, 100.0);
        assert!(t.is_watertight());
    }

    #[test]
    fn holes_inversion_and_parts() {
        let mut m = cube(Vec3::ZERO, 1.0);
        m.delete_faces(&[3]).unwrap();
        let mut inverted = cube(Vec3::new(3.0, 0.0, 0.0), 1.0);
        inverted.flip_all();
        m.append(&inverted);
        let t = topology(&m);
        assert_eq!((t.parts, t.closed_parts, t.inverted_parts), (2, 1, 1));
        assert_eq!((t.boundary_edges, t.boundary_loops), (4, 1));
        assert!((t.volume + 1.0).abs() < 1e-9, "only the closed part counts: {}", t.volume);
        assert!(!t.is_watertight());

        let ps = parts(&m);
        let loops = boundary_loops(&m, &ps);
        assert_eq!(loops.len(), 1);
        assert_eq!((loops[0].edges, loops[0].part), (4, 0));
        assert!((loops[0].length - 4.0).abs() < 1e-6);
        assert!(loops[0].centroid.distance(Vec3::new(0.5, 1.0, 0.5)) < 1e-6, "{:?}", loops[0].centroid);
        assert!(!ps.parts[0].closed && ps.parts[1].closed && ps.parts[1].volume < 0.0);
    }

    #[test]
    fn hollow_parts_are_not_inverted() {
        let mut m = cube(Vec3::ZERO, 1.0);
        let mut inner = cube(Vec3::splat(0.1), 0.8);
        inner.flip_all();
        m.append(&inner);
        let t = topology(&m);
        assert_eq!((t.parts, t.closed_parts, t.inverted_parts), (2, 2, 0));
        assert!(t.is_watertight() && t.is_closed());
        assert!((t.volume - (1.0 - 0.512)).abs() < 1e-6, "net solid volume: {}", t.volume);
        let ps = parts(&m);
        assert!(!ps.parts[0].cavity && ps.parts[1].cavity);
        // An inward box outside any other part is a mistake, not a cavity.
        let mut lone = cube(Vec3::new(5.0, 0.0, 0.0), 1.0);
        lone.flip_all();
        m.append(&lone);
        assert_eq!(topology(&m).inverted_parts, 1);
    }

    #[test]
    fn degenerate_faces_and_short_edges() {
        // A quad whose top two corners coincide in space, and a sliver triangle
        // continuing one of its edges.
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(0.5, 1.0, 0.0), Vec3::new(0.5, 1.0, 0.0), Vec3::new(1.5, -1.0, 1e-9)];
        let m = Mesh::from_polygons(p, &[4, 3], &[0, 1, 2, 3, 2, 1, 4]).unwrap();
        let t = topology(&m);
        assert_eq!(t.short_edges, 1);
        assert_eq!(t.degenerate_faces, 1, "the sliver; the quad still has area");
        assert!(face_degenerate(&m, 1).0);
    }

    #[test]
    fn empty_mesh() {
        let t = topology(&Mesh::new());
        assert_eq!(t, Topology::default());
        assert!(!t.is_watertight());
    }
}
