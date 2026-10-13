//! Where strands grow: the whole surface, face groups, or the patch a
//! closed loop of surface points cuts off (ADR 0013).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::{Affine3A, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::{ElementKind, Mesh};

use crate::{Region, StrandsError};

/// Most points in a region loop.
pub const MAX_LOOP_POINTS: usize = 64;

/// A heap entry ordered by smallest distance first.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry(f32, u32);

impl Eq for Entry {}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The faces of a region, ascending.
///
/// Complexity: O(F) for all faces or groups; for a loop of k points,
/// O(k E log V) for the shortest edge paths plus O(F) for the flood fill.
pub fn region_faces(mesh: &Mesh, region: &Region) -> Result<Vec<u32>, StrandsError> {
    match region {
        Region::All => Ok(mesh.faces().collect()),
        Region::Groups(names) => groups(mesh, names),
        Region::Loop { points, inside } => cut_off(mesh, points, *inside),
    }
}

fn groups(mesh: &Mesh, names: &[String]) -> Result<Vec<u32>, StrandsError> {
    if names.is_empty() {
        return Err(StrandsError::new("region", "name at least one face group"));
    }
    let mut out = Vec::new();
    for n in names {
        match mesh.group(n).filter(|g| g.kind() == ElementKind::Face) {
            Some(g) => out.extend(g.iter()),
            None => {
                let known: Vec<&str> = mesh.groups().filter(|(_, g)| g.kind() == ElementKind::Face).map(|(n, _)| n).collect();
                let mut e = StrandsError::new("region", format!("the surface has no face group `{n}`"));
                e.suggestions = p3d_scene::name::suggest(n, known.iter().copied(), 3);
                return Err(e);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// The vertex of the face nearest to `p` that is closest to it.
fn snap(mesh: &Mesh, bvh: &TriBvh, p: Vec3) -> Option<u32> {
    let c = bvh.closest(p, f32::INFINITY)?;
    mesh.face_verts(c.face)
        .min_by(|&a, &b| mesh.positions[a as usize].distance_squared(p).total_cmp(&mesh.positions[b as usize].distance_squared(p)).then(a.cmp(&b)))
}

/// Marks the edges of a shortest edge path from `a` to `b`.
///
/// Complexity: O(E log V), stopped when `b` is reached.
fn mark_path(mesh: &Mesh, a: u32, b: u32, dist: &mut [f32], via: &mut [u32], cut: &mut [bool]) -> bool {
    dist.fill(f32::INFINITY);
    via.fill(u32::MAX);
    let mut heap = BinaryHeap::new();
    dist[a as usize] = 0.0;
    heap.push(Entry(0.0, a));
    while let Some(Entry(d, v)) = heap.pop() {
        if v == b {
            break;
        }
        if d > dist[v as usize] {
            continue;
        }
        for h in mesh.vert_outgoing(v) {
            let w = mesh.dest(h);
            let nd = d + mesh.positions[v as usize].distance(mesh.positions[w as usize]);
            if nd < dist[w as usize] {
                dist[w as usize] = nd;
                via[w as usize] = h;
                heap.push(Entry(nd, w));
            }
        }
    }
    if !dist[b as usize].is_finite() {
        return false;
    }
    let mut v = b;
    while v != a {
        let h = via[v as usize];
        cut[mesh.edge_of(h) as usize] = true;
        v = mesh.origin(h);
    }
    true
}

fn cut_off(mesh: &Mesh, points: &[Vec3], inside: Option<Vec3>) -> Result<Vec<u32>, StrandsError> {
    if points.len() < 3 || points.len() > MAX_LOOP_POINTS {
        return Err(StrandsError::new("region", format!("a region loop takes 3 to {MAX_LOOP_POINTS} points, not {}", points.len())));
    }
    if points.iter().chain(inside.iter()).any(|p| !p.is_finite()) {
        return Err(StrandsError::new("region", "the loop points must be finite"));
    }
    let bvh = TriBvh::from_mesh(mesh, Affine3A::IDENTITY);
    let mut verts: Vec<u32> = Vec::with_capacity(points.len());
    for &p in points {
        let v = snap(mesh, &bvh, p).ok_or_else(|| StrandsError::new("region", "the surface has no faces"))?;
        if verts.last() != Some(&v) {
            verts.push(v);
        }
    }
    while verts.len() > 1 && verts.first() == verts.last() {
        verts.pop();
    }
    if verts.len() < 3 {
        return Err(StrandsError::new("region", "the loop points land on fewer than 3 distinct vertices; spread them out"));
    }
    let mut dist = vec![f32::INFINITY; mesh.vert_slots()];
    let mut via = vec![u32::MAX; mesh.vert_slots()];
    let mut cut = vec![false; mesh.edge_slots()];
    for i in 0..verts.len() {
        let (a, b) = (verts[i], verts[(i + 1) % verts.len()]);
        if !mark_path(mesh, a, b, &mut dist, &mut via, &mut cut) {
            return Err(StrandsError::new("region", "the loop points lie on separate pieces of the surface"));
        }
    }
    // Flood fill faces across uncut edges.
    let mut comp = vec![u32::MAX; mesh.face_slots()];
    let mut areas: Vec<f32> = Vec::new();
    let mut stack = Vec::new();
    for f in mesh.faces() {
        if comp[f as usize] != u32::MAX {
            continue;
        }
        let id = areas.len() as u32;
        let mut area = 0.0;
        comp[f as usize] = id;
        stack.push(f);
        while let Some(g) = stack.pop() {
            area += mesh.face_area(g);
            for h in mesh.face_halfedges(g) {
                if cut[mesh.edge_of(h) as usize] {
                    continue;
                }
                if let Some(n) = mesh.face(mesh.twin(h))
                    && comp[n as usize] == u32::MAX
                {
                    comp[n as usize] = id;
                    stack.push(n);
                }
            }
        }
        areas.push(area);
    }
    if areas.len() < 2 {
        return Err(StrandsError::new("region", "the loop does not close off a patch; put its points around the area (in order)"));
    }
    let pick = match inside {
        Some(p) => {
            let c = bvh.closest(p, f32::INFINITY).ok_or_else(|| StrandsError::new("region", "the surface has no faces"))?;
            comp[c.face as usize]
        }
        None => (0..areas.len() as u32).min_by(|&a, &b| areas[a as usize].total_cmp(&areas[b as usize]).then(a.cmp(&b))).unwrap_or(0),
    };
    Ok(mesh.faces().filter(|&f| comp[f as usize] == pick).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use p3d_geo::prim::{Origin, QuadSphereSpec, quad_sphere};

    use super::*;

    #[test]
    fn a_loop_cuts_off_the_smaller_cap_or_the_inside_one() {
        let m = quad_sphere(&QuadSphereSpec { radius: 0.1, segments: 12 }, Origin::Center).unwrap();
        // A ring at 45 degrees north: the polar cap is about 15% of the sphere.
        let ring: Vec<Vec3> = (0..8)
            .map(|i| {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                Vec3::new(a.cos() * 0.0707, 0.0707, a.sin() * 0.0707)
            })
            .collect();
        let cap = region_faces(&m, &Region::Loop { points: ring.clone(), inside: None }).unwrap();
        let total: f32 = m.faces().map(|f| m.face_area(f)).sum();
        let area: f32 = cap.iter().map(|&f| m.face_area(f)).sum();
        assert!(area / total > 0.1 && area / total < 0.2, "{}", area / total);
        assert!(cap.iter().all(|&f| m.face_centroid(f).y > 0.05));
        let rest = region_faces(&m, &Region::Loop { points: ring, inside: Some(Vec3::new(0.0, -0.1, 0.0)) }).unwrap();
        assert_eq!(rest.len() + cap.len(), m.face_count());
    }

    #[test]
    fn bad_regions_are_errors() {
        let m = quad_sphere(&QuadSphereSpec { radius: 0.1, segments: 4 }, Origin::Center).unwrap();
        let e = region_faces(&m, &Region::Loop { points: vec![Vec3::Y * 0.1; 4], inside: None }).unwrap_err();
        assert!(e.message.contains("distinct"), "{e}");
        let e = region_faces(&m, &Region::Groups(vec!["scalp".into()])).unwrap_err();
        assert!(e.message.contains("scalp"), "{e}");
    }
}
