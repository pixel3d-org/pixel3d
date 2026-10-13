//! Snapping targets for transforms (the Snap To settings of the header):
//! vertices, edges, edge centers, perpendicular feet, faces (projected
//! under the pointer or nearest to the moving point) and volume centers.
//!
//! Everything is found through each mesh's BVH, never by looping over all
//! elements: a ray under the pointer (and a ring of rays around it, for
//! silhouettes) finds faces; vertex and edge candidates are the elements of
//! those faces and their neighbors.

use std::sync::Arc;

use egui::{Pos2, Rect, pos2};
use glam::{Mat4, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use p3d_render::OrbitCamera;

use crate::transform::SnapTarget;

/// How near (points) an element must be on screen to snap.
pub const RADIUS: f32 = 14.0;

/// A mesh the snapping may use, with its world matrix and, for the edited
/// mesh, the vertices that move (never targets).
pub struct Candidate<'a> {
    pub mesh: &'a Arc<Mesh>,
    pub bvh: &'a TriBvh,
    pub model: Mat4,
    pub moving: Option<&'a std::collections::BTreeSet<u32>>,
}

/// A found target: the world point and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub point: Vec3,
    pub target: SnapTarget,
}

fn project(vp: Mat4, rect: Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    a + ab * t
}

/// Faces under `at` and around it (a ring of rays), with the nearest hit.
///
/// Complexity: O(rays x log F).
fn faces_near(c: &Candidate<'_>, cam: &OrbitCamera, rect: Rect, at: Pos2) -> (Vec<u32>, Option<(f32, Vec3, u32)>) {
    let inv = c.model.inverse();
    let mut faces = Vec::new();
    let mut nearest: Option<(f32, Vec3, u32)> = None;
    let ring = std::iter::once(egui::Vec2::ZERO).chain((0..8).map(|k| egui::Vec2::angled(k as f32 * std::f32::consts::FRAC_PI_4) * (RADIUS * 0.7)));
    for (i, off) in ring.enumerate() {
        let p = at + off;
        let (o, d) = cam.ray(p.x - rect.left(), p.y - rect.top(), rect.width(), rect.height());
        let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d).normalize_or_zero());
        // Skip the moving part of the edited mesh: look past it.
        let mut from = lo;
        for _ in 0..4 {
            let Some(h) = c.bvh.raycast(from, ld, f32::INFINITY) else { break };
            let moving = c.moving.is_some_and(|m| c.mesh.face_verts(h.face).any(|v| m.contains(&v)));
            if moving {
                from += ld * (h.t + 1e-4);
                continue;
            }
            faces.push(h.face);
            if i == 0 {
                let world = c.model.transform_point3(from + ld * h.t);
                nearest = Some(((world - o).length(), world, h.face));
            }
            break;
        }
    }
    faces.sort_unstable();
    faces.dedup();
    (faces, nearest)
}

/// Faces sharing a vertex with `faces`.
///
/// Complexity: O(corners of the faces x vertex valence).
fn grow(mesh: &Mesh, faces: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = faces.iter().flat_map(|&f| mesh.face_verts(f).flat_map(|v| mesh.vert_faces(v))).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The best snap target near the pointer `at` for the enabled `targets`,
/// starting from the moving point `from` (world; for perpendicular feet
/// and nearest faces). Vertices win over edge centers, edges and faces
/// at the same distance on screen.
///
/// Complexity: O(meshes x rays x log F + candidate elements).
pub fn find(cands: &[Candidate<'_>], cam: &OrbitCamera, rect: Rect, at: Pos2, from: Vec3, targets: &[SnapTarget], face_nearest: bool) -> Option<Hit> {
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let on = |t: SnapTarget| targets.contains(&t);
    let screen = |p: Vec3| project(vp, rect, p).map_or(f32::INFINITY, |s| s.distance(at));
    // Rank: (priority, screen distance); lower wins.
    let mut best: Option<((u8, f32), Hit)> = None;
    let mut offer = |prio: u8, point: Vec3, target: SnapTarget| {
        let d = screen(point);
        if d <= RADIUS && best.as_ref().is_none_or(|(k, _)| (prio, d) < (k.0, k.1)) {
            best = Some(((prio, d), Hit { point, target }));
        }
    };
    let mut surface: Option<(f32, Vec3)> = None;
    for c in cands {
        let (faces, nearest) = faces_near(c, cam, rect, at);
        if let Some((dist, p, _)) = nearest {
            if surface.is_none_or(|s| dist < s.0) {
                surface = Some((dist, p));
            }
            if on(SnapTarget::Volume) {
                // Halfway between where the ray enters and leaves the mesh.
                let inv = c.model.inverse();
                let (_, d) = cam.ray(at.x - rect.left(), at.y - rect.top(), rect.width(), rect.height());
                let ld = inv.transform_vector3(d).normalize_or_zero();
                let entry = inv.transform_point3(p);
                if let Some(exit) = c.bvh.raycast(entry + ld * 1e-4, ld, f32::INFINITY) {
                    let mid = c.model.transform_point3(entry + ld * (exit.t * 0.5));
                    offer(4, mid, SnapTarget::Volume);
                }
            }
        }
        if faces.is_empty() {
            continue;
        }
        let near = grow(c.mesh, &faces);
        let pos = |v: u32| c.model.transform_point3(c.mesh.positions.get(v as usize).copied().unwrap_or(Vec3::ZERO));
        let still = |v: u32| c.moving.is_none_or(|m| !m.contains(&v));
        if on(SnapTarget::Vertex) {
            for &f in &near {
                for v in c.mesh.face_verts(f).filter(|&v| still(v)) {
                    offer(0, pos(v), SnapTarget::Vertex);
                }
            }
        }
        let wants_edges = on(SnapTarget::Edge) || on(SnapTarget::EdgeCenter) || on(SnapTarget::EdgePerpendicular);
        if wants_edges {
            for &f in &near {
                for h in c.mesh.face_halfedges(f) {
                    let [a, b] = c.mesh.edge_verts(h >> 1);
                    if !still(a) || !still(b) {
                        continue;
                    }
                    let (pa, pb) = (pos(a), pos(b));
                    if on(SnapTarget::EdgeCenter) {
                        offer(1, (pa + pb) * 0.5, SnapTarget::EdgeCenter);
                    }
                    if on(SnapTarget::EdgePerpendicular) {
                        offer(2, closest_on_segment(from, pa, pb), SnapTarget::EdgePerpendicular);
                    }
                    if on(SnapTarget::Edge) {
                        // The point of the edge nearest to the pointer ray.
                        let (o, d) = cam.ray(at.x - rect.left(), at.y - rect.top(), rect.width(), rect.height());
                        let ab = pb - pa;
                        let (b2, w) = (ab.dot(d), pa - o);
                        let denom = ab.length_squared() - b2 * b2;
                        let t = if denom.abs() > 1e-9 { ((b2 * d.dot(w) - ab.dot(w)) / denom).clamp(0.0, 1.0) } else { 0.0 };
                        offer(3, pa + ab * t, SnapTarget::Edge);
                    }
                }
            }
        }
    }
    if on(SnapTarget::Face)
        && let Some((_, p)) = surface
    {
        offer(5, p, SnapTarget::Face);
    }
    if face_nearest {
        // The surface point nearest to the moving point, whatever the pointer.
        let mut near: Option<(f32, Vec3)> = None;
        for c in cands {
            let inv = c.model.inverse();
            if let Some(h) = c.bvh.closest(inv.transform_point3(from), f32::INFINITY) {
                let w = c.model.transform_point3(h.point);
                let d = (w - from).length();
                if near.is_none_or(|n| d < n.0) {
                    near = Some((d, w));
                }
            }
        }
        if let Some((_, p)) = near
            && best.is_none()
        {
            return Some(Hit { point: p, target: SnapTarget::Face });
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cube() -> Arc<Mesh> {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "c" })).unwrap();
        let k = app.doc().resolve("c").unwrap();
        Arc::new(app.doc().object_mesh(k).unwrap().clone())
    }

    #[test]
    fn snaps_to_vertices_edges_and_faces_near_the_pointer() {
        let m = cube();
        let bvh = TriBvh::from_mesh(&m, glam::Affine3A::IDENTITY);
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        let (lo, hi) = m.bounds().unwrap();
        cam.frame(lo, hi, 1.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let vp = cam.view_proj(1.0);
        let c = [Candidate { mesh: &m, bvh: &bvh, model: Mat4::IDENTITY, moving: None }];
        let corner = Vec3::new(hi.x, hi.y, hi.z);
        let near_corner = project(vp, rect, corner).unwrap() + egui::vec2(-5.0, 5.0);
        let all = [SnapTarget::Vertex, SnapTarget::Edge, SnapTarget::Face];
        let h = find(&c, &cam, rect, near_corner, Vec3::ZERO, &all, false).unwrap();
        assert_eq!(h.target, SnapTarget::Vertex);
        assert!((h.point - corner).length() < 1e-4);
        // In the middle of the face only the face is near.
        let h = find(&c, &cam, rect, rect.center(), Vec3::ZERO, &all, false).unwrap();
        assert_eq!(h.target, SnapTarget::Face);
        assert!((h.point.z - hi.z).abs() < 1e-4, "on the front face");
        // Edge centers: the middle of the top front edge.
        let top = project(vp, rect, Vec3::new(0.0, hi.y, hi.z)).unwrap();
        let h = find(&c, &cam, rect, top + egui::vec2(3.0, 3.0), Vec3::ZERO, &[SnapTarget::EdgeCenter], false).unwrap();
        assert!((h.point - Vec3::new(0.5 * (lo.x + hi.x), hi.y, hi.z)).length() < 1e-4);
        // Volume: the middle of the box along the ray.
        let h = find(&c, &cam, rect, rect.center(), Vec3::ZERO, &[SnapTarget::Volume], false).unwrap();
        assert!((h.point.z - 0.5 * (lo.z + hi.z)).abs() < 1e-3);
        // Nothing near an empty corner of the view.
        assert_eq!(find(&c, &cam, rect, pos2(3.0, 3.0), Vec3::ZERO, &all, false), None);
        // Moving vertices are never targets.
        let moving: std::collections::BTreeSet<u32> = m.verts().collect();
        let c = [Candidate { mesh: &m, bvh: &bvh, model: Mat4::IDENTITY, moving: Some(&moving) }];
        assert_eq!(find(&c, &cam, rect, near_corner, Vec3::ZERO, &all, false), None);
    }
}
