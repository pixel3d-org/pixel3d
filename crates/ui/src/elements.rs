//! Vertex, edge and face selection (the element modes, keys 2 to 4): the
//! selected elements of the mesh being edited (the active mesh object),
//! picking them under the pointer with a CPU ray cast, and the overlay that
//! shows them. Element ids are valid only for the mesh they were picked on
//! (project spec 4.3): an edit that replaces the mesh clears them.
//!
//! The ids picked in the current mode are the selection; the display flushes
//! them as Blender does (vertices select the edges and faces they span,
//! faces select their edges and vertices).

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::{Pos2, Rect, pos2};
use glam::{Mat4, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use p3d_render::{EditOverlay, ElementSets, OrbitCamera};
use p3d_scene::ObjectKey;

use crate::SelectMode;

/// How far from the pointer (points) a vertex or an edge still counts as
/// hit when the ray misses the surface (silhouettes).
const MISS_RADIUS: f32 = 10.0;

/// The selected elements of the mesh being edited.
#[derive(Default)]
pub struct EditState {
    /// The object being edited and the mesh the ids belong to.
    pub object: Option<ObjectKey>,
    mesh: Option<Arc<Mesh>>,
    /// The picked ids, in the kind of the mode they were picked in.
    picked: BTreeSet<u32>,
    picked_mode: Option<SelectMode>,
    /// Bumped on every change (part of the overlay key).
    pub version: u64,
    bvh: Option<(Arc<Mesh>, Arc<TriBvh>)>,
    shown: Option<(u64, Arc<EditOverlay>, Flushed)>,
}

/// The selection flushed to all three element kinds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flushed {
    pub verts: BTreeSet<u32>,
    pub edges: BTreeSet<u32>,
    pub faces: BTreeSet<u32>,
}

/// `picked` (ids of `mode`'s kind) flushed to vertices, edges and faces.
///
/// Complexity: O((V + E + F corners) log s), s the selection size.
pub fn flush(mesh: &Mesh, mode: SelectMode, picked: &BTreeSet<u32>) -> Flushed {
    let mut f = Flushed::default();
    match mode {
        SelectMode::Object => {}
        SelectMode::Vert => {
            f.verts = picked.iter().copied().filter(|&v| mesh.is_alive(p3d_mesh::ElementKind::Vert, v)).collect();
            f.edges = mesh.edges().filter(|&e| mesh.edge_verts(e).iter().all(|v| f.verts.contains(v))).collect();
            f.faces = mesh.faces().filter(|&x| mesh.face_verts(x).all(|v| f.verts.contains(&v))).collect();
        }
        SelectMode::Edge => {
            f.edges = picked.iter().copied().filter(|&e| mesh.is_alive(p3d_mesh::ElementKind::Edge, e)).collect();
            f.verts = f.edges.iter().flat_map(|&e| mesh.edge_verts(e)).collect();
            f.faces = mesh.faces().filter(|&x| mesh.face_halfedges(x).all(|h| f.edges.contains(&(h >> 1)))).collect();
        }
        SelectMode::Face => {
            f.faces = picked.iter().copied().filter(|&x| mesh.is_alive(p3d_mesh::ElementKind::Face, x)).collect();
            f.edges = f.faces.iter().flat_map(|&x| mesh.face_halfedges(x).map(|h| h >> 1)).collect();
            f.verts = f.faces.iter().flat_map(|&x| mesh.face_verts(x)).collect();
        }
    }
    f
}

impl EditState {
    /// The picked ids and their mode.
    pub fn picked(&self) -> (&BTreeSet<u32>, Option<SelectMode>) {
        (&self.picked, self.picked_mode)
    }

    /// Clears the selection (keeps the object).
    pub fn clear(&mut self) {
        if !self.picked.is_empty() {
            self.picked.clear();
            self.version += 1;
        }
    }

    /// Follows the edited object and its mesh: another object, a hidden
    /// one or a replaced mesh clears the ids. Converts the ids when the mode
    /// changed (through the flushed sets, as Blender does).
    ///
    /// Complexity: O(1) when nothing changed; a conversion as [`flush`].
    pub fn sync(&mut self, object: Option<ObjectKey>, mesh: Option<&Arc<Mesh>>, mode: SelectMode) {
        let same_mesh = match (&self.mesh, mesh) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if self.object != object || !same_mesh {
            self.object = object;
            self.mesh = mesh.cloned();
            self.picked.clear();
            self.picked_mode = None;
            self.version += 1;
        }
        if mode == SelectMode::Object || self.picked_mode == Some(mode) {
            return;
        }
        if let (Some(from), Some(m)) = (self.picked_mode, &self.mesh)
            && !self.picked.is_empty()
        {
            let f = flush(m, from, &self.picked);
            self.picked = match mode {
                SelectMode::Vert => f.verts,
                SelectMode::Edge => f.edges,
                _ => f.faces,
            };
            self.version += 1;
        }
        self.picked_mode = Some(mode);
    }

    /// Applies a click result: `hit` replaces the selection, or toggles in
    /// it with `add`; a miss without `add` clears it.
    pub fn click(&mut self, hit: Option<u32>, add: bool) {
        match (hit, add) {
            (Some(id), true) => {
                if !self.picked.remove(&id) {
                    self.picked.insert(id);
                }
            }
            (Some(id), false) => {
                self.picked.clear();
                self.picked.insert(id);
            }
            (None, false) => self.picked.clear(),
            (None, true) => return,
        }
        self.version += 1;
    }

    /// The ray-cast structure of the edited mesh, built once per mesh.
    ///
    /// Complexity: O(1) when cached, O(F log F) to build.
    pub fn bvh(&mut self) -> Option<Arc<TriBvh>> {
        let mesh = self.mesh.as_ref()?;
        if let Some((m, b)) = &self.bvh
            && Arc::ptr_eq(m, mesh)
        {
            return Some(Arc::clone(b));
        }
        let b = Arc::new(TriBvh::from_mesh(mesh, glam::Affine3A::IDENTITY));
        self.bvh = Some((Arc::clone(mesh), Arc::clone(&b)));
        Some(b)
    }

    /// The overlay for part `part` in `mode` and its key, rebuilt only
    /// when the selection, the mesh or the mode changed.
    ///
    /// Complexity: O(1) when cached; a rebuild is O(V + E + F) as
    /// [`flush`] and [`p3d_render::edit_overlay`].
    pub fn overlay(&mut self, part: u32, mode: SelectMode) -> Option<(Arc<EditOverlay>, u64, &Flushed)> {
        let mesh = self.mesh.clone()?;
        let key = self.version.wrapping_mul(31).wrapping_add(u64::from(part)).wrapping_mul(8).wrapping_add(mode as u64);
        if self.shown.as_ref().is_none_or(|(k, _, _)| *k != key) {
            let f = if self.picked_mode == Some(mode) { flush(&mesh, mode, &self.picked) } else { Flushed::default() };
            let sets = ElementSets { verts: &f.verts, edges: &f.edges, faces: &f.faces };
            let o = Arc::new(p3d_render::edit_overlay(&mesh, part, sets, mode == SelectMode::Vert));
            self.shown = Some((key, o, f));
        }
        self.shown.as_ref().map(|(k, o, f)| (Arc::clone(o), *k, f))
    }

    /// The mesh being edited.
    pub fn mesh(&self) -> Option<&Arc<Mesh>> {
        self.mesh.as_ref()
    }

    /// The selection as last shown, flushed to all kinds.
    pub fn flushed(&self) -> Option<&Flushed> {
        self.shown.as_ref().map(|(_, _, f)| f)
    }

    /// Selects every element of `mode`'s kind.
    ///
    /// Complexity: O(n log n) in the elements of that kind.
    pub fn select_all(&mut self, mode: SelectMode) {
        let Some(m) = &self.mesh else { return };
        self.picked = match mode {
            SelectMode::Object => return,
            SelectMode::Vert => m.verts().collect(),
            SelectMode::Edge => m.edges().collect(),
            SelectMode::Face => m.faces().collect(),
        };
        self.picked_mode = Some(mode);
        self.version += 1;
    }
}

/// How a box selection combines with the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxOp {
    /// Replace (plain drag).
    Set,
    /// Extend (Shift).
    Add,
    /// Subtract (Ctrl or Cmd).
    Sub,
}

impl EditState {
    /// Applies a box selection of `ids` in `mode`.
    ///
    /// Complexity: O(ids log n).
    pub fn apply_box(&mut self, ids: Vec<u32>, mode: SelectMode, op: BoxOp) {
        if self.picked_mode != Some(mode) {
            self.picked.clear();
            self.picked_mode = Some(mode);
        }
        match op {
            BoxOp::Set => self.picked = ids.into_iter().collect(),
            BoxOp::Add => self.picked.extend(ids),
            BoxOp::Sub => {
                for id in ids {
                    self.picked.remove(&id);
                }
            }
        }
        self.version += 1;
    }
}

/// Screen position (points) of a world point, `None` behind the camera.
fn project(vp: Mat4, rect: Rect, world: Vec3) -> Option<Pos2> {
    let c = vp * world.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// The nearest candidate by distance, ties to the lower id.
fn nearest(candidates: impl Iterator<Item = (f32, u32)>) -> Option<(f32, u32)> {
    candidates.min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
}

/// The elements of `mode` inside `area` (points): vertices by position,
/// edges by midpoint, faces by center. Unless `through` (x-ray), elements
/// behind the surface are left out: a ray toward each candidate must not
/// hit the mesh in front of it.
///
/// Complexity: O(V + E + F corners) projections, plus O(log F) per
/// candidate inside the box when not `through`.
#[allow(clippy::too_many_arguments)] // the picking context, like `pick`
pub fn box_pick(mesh: &Mesh, bvh: &TriBvh, model: Mat4, camera: &OrbitCamera, rect: Rect, area: Rect, mode: SelectMode, through: bool) -> Vec<u32> {
    let vp = camera.view_proj(rect.width() / rect.height().max(1.0));
    let inv = model.inverse();
    let visible = |local: Vec3, at: Pos2| {
        if through {
            return true;
        }
        let (o, d) = camera.ray(at.x - rect.left(), at.y - rect.top(), rect.width(), rect.height());
        let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d).normalize_or_zero());
        let t = (local - lo).dot(ld);
        bvh.raycast(lo, ld, f32::INFINITY).is_none_or(|h| h.t >= t - (1e-4 * t.abs() + 1e-4))
    };
    let inside = |local: Vec3| project(vp, rect, model.transform_point3(local)).filter(|s| area.contains(*s)).is_some_and(|s| visible(local, s));
    let pos = |v: u32| mesh.positions.get(v as usize).copied().unwrap_or(Vec3::ZERO);
    match mode {
        SelectMode::Object => Vec::new(),
        SelectMode::Vert => mesh.verts().filter(|&v| inside(pos(v))).collect(),
        SelectMode::Edge => mesh
            .edges()
            .filter(|&e| {
                let [a, b] = mesh.edge_verts(e);
                inside((pos(a) + pos(b)) * 0.5)
            })
            .collect(),
        SelectMode::Face => {
            let normals = mesh.face_normals();
            mesh.faces()
                .filter(|&f| {
                    let (sum, n) = mesh.face_verts(f).fold((Vec3::ZERO, 0.0), |(s, n), v| (s + pos(v), n + 1.0));
                    // Just off the surface, so the face itself does not hide its center.
                    let lift = normals.get(f as usize).copied().unwrap_or(Vec3::ZERO) * 1e-4;
                    n > 0.0 && inside(sum / n + lift)
                })
                .collect()
        }
    }
}

/// Whether the screen segments `a`-`b` and `c`-`d` cross.
fn segments_cross(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
    let side = |p: Pos2, q: Pos2, r: Pos2| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let (d1, d2) = (side(a, b, c), side(a, b, d));
    let (d3, d4) = (side(c, d, a), side(c, d, b));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Faces whose outline on screen the segment `a`-`b` (points) crosses;
/// unless `through`, only faces turned toward the camera.
///
/// Complexity: O(face corners).
#[allow(clippy::too_many_arguments)] // the picking context, like `pick`
pub fn faces_crossed(mesh: &Mesh, model: Mat4, camera: &OrbitCamera, rect: Rect, a: Pos2, b: Pos2, through: bool) -> Vec<u32> {
    let vp = camera.view_proj(rect.width() / rect.height().max(1.0));
    let inv = model.inverse();
    let eye = inv.transform_point3(camera.eye());
    let toward = inv.transform_vector3(camera.toward());
    let normals = mesh.face_normals();
    let mut corners: Vec<Pos2> = Vec::new();
    mesh.faces()
        .filter(|&f| {
            let pos = |v: u32| mesh.positions.get(v as usize).copied().unwrap_or(Vec3::ZERO);
            if !through {
                let n = normals.get(f as usize).copied().unwrap_or(Vec3::ZERO);
                let to_eye = if camera.ortho { toward } else { eye - mesh.face_verts(f).next().map_or(Vec3::ZERO, pos) };
                if n.dot(to_eye) <= 0.0 {
                    return false;
                }
            }
            corners.clear();
            for v in mesh.face_verts(f) {
                match project(vp, rect, model.transform_point3(pos(v))) {
                    Some(p) => corners.push(p),
                    None => return false,
                }
            }
            let n = corners.len();
            n >= 3 && (0..n).any(|i| segments_cross(a, b, corners[i], corners[(i + 1) % n]))
        })
        .collect()
}

/// The element of `mode` under `at` (points) on `mesh` drawn with `model`.
/// The ray picks the face under the pointer; vertex and edge modes take
/// that face's nearest vertex or edge on screen. When the ray misses, the
/// nearest vertex or edge within a few points counts (silhouettes).
///
/// Complexity: O(log F + face size) on a hit, O(V) or O(E) on a miss.
pub fn pick(mesh: &Mesh, bvh: &TriBvh, model: Mat4, camera: &OrbitCamera, rect: Rect, at: Pos2, mode: SelectMode) -> Option<u32> {
    let (o, d) = camera.ray(at.x - rect.left(), at.y - rect.top(), rect.width(), rect.height());
    let inv = model.inverse();
    let (lo, ld) = (inv.transform_point3(o), inv.transform_vector3(d).normalize_or_zero());
    let hit = bvh.raycast(lo, ld, f32::INFINITY).map(|h| h.face);
    let vp = camera.view_proj(rect.width() / rect.height().max(1.0));
    let screen = |v: u32| mesh.positions.get(v as usize).and_then(|p| project(vp, rect, model.transform_point3(*p)));
    match mode {
        SelectMode::Object => None,
        SelectMode::Face => hit,
        SelectMode::Vert => {
            let d = |v: u32| screen(v).map(|s| (s.distance(at), v));
            match hit {
                Some(f) => nearest(mesh.face_verts(f).filter_map(d)).map(|x| x.1),
                None => nearest(mesh.verts().filter_map(d)).filter(|x| x.0 <= MISS_RADIUS).map(|x| x.1),
            }
        }
        SelectMode::Edge => {
            let d = |e: u32| {
                let [a, b] = mesh.edge_verts(e);
                Some((segment_distance(at, screen(a)?, screen(b)?), e))
            };
            match hit {
                Some(f) => nearest(mesh.face_halfedges(f).filter_map(|h| d(h >> 1))).map(|x| x.1),
                None => nearest(mesh.edges().filter_map(d)).filter(|x| x.0 <= MISS_RADIUS).map(|x| x.1),
            }
        }
    }
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
    fn flushing_follows_blender() {
        let m = cube();
        // One face selects its 4 edges and 4 vertices.
        let face = m.faces().next().unwrap();
        let f = flush(&m, SelectMode::Face, &[face].into());
        assert_eq!((f.verts.len(), f.edges.len(), f.faces.len()), (4, 4, 1));
        // Its 4 vertices select the face back, and its edges.
        let v = flush(&m, SelectMode::Vert, &f.verts);
        assert_eq!(v, f);
        // Two vertices of an edge select that edge and no face.
        let e = m.edges().next().unwrap();
        let ends: BTreeSet<u32> = m.edge_verts(e).into_iter().collect();
        let v = flush(&m, SelectMode::Vert, &ends);
        assert_eq!((v.edges.len(), v.faces.len()), (1, 0));
    }

    #[test]
    fn clicks_and_modes() {
        let m = cube();
        let mut s = EditState::default();
        s.sync(Some(ObjectKey::default()), Some(&m), SelectMode::Face);
        let face = m.faces().next().unwrap();
        s.click(Some(face), false);
        assert_eq!(s.picked().0.len(), 1);
        // Switching to vertex mode converts the selection.
        s.sync(Some(ObjectKey::default()), Some(&m), SelectMode::Vert);
        assert_eq!(s.picked().0.len(), 4);
        // Shift toggles; a plain miss clears.
        let v = *s.picked().0.iter().next().unwrap();
        s.click(Some(v), true);
        assert_eq!(s.picked().0.len(), 3);
        s.click(None, true);
        assert_eq!(s.picked().0.len(), 3);
        s.click(None, false);
        assert!(s.picked().0.is_empty());
        // Another mesh clears the ids.
        s.click(Some(v), false);
        s.sync(Some(ObjectKey::default()), Some(&cube()), SelectMode::Vert);
        assert!(s.picked().0.is_empty());
    }

    #[test]
    fn box_selection_skips_hidden_elements_unless_through() {
        let m = cube();
        let bvh = TriBvh::from_mesh(&m, glam::Affine3A::IDENTITY);
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        let (lo, hi) = m.bounds().unwrap();
        cam.frame(lo, hi, 1.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        // The whole view: the 4 front vertices when hidden ones are skipped,
        // all 8 in x-ray.
        assert_eq!(box_pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, rect, SelectMode::Vert, false).len(), 4);
        assert_eq!(box_pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, rect, SelectMode::Vert, true).len(), 8);
        assert_eq!(box_pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, rect, SelectMode::Face, false).len(), 1, "only the front face");
        // An empty corner selects nothing.
        let corner = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(5.0, 5.0));
        assert!(box_pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, corner, SelectMode::Vert, true).is_empty());
        let mut s = EditState::default();
        s.apply_box(vec![1, 2], SelectMode::Vert, BoxOp::Set);
        s.apply_box(vec![3], SelectMode::Vert, BoxOp::Add);
        s.apply_box(vec![1], SelectMode::Vert, BoxOp::Sub);
        assert_eq!(s.picked().0.iter().copied().collect::<Vec<_>>(), vec![2, 3]);
    }

    #[test]
    fn picking_hits_faces_vertices_and_edges() {
        let m = cube();
        let bvh = TriBvh::from_mesh(&m, glam::Affine3A::IDENTITY);
        let mut cam = OrbitCamera::default();
        cam.set_view(p3d_render::ViewKind::Front);
        let (lo, hi) = m.bounds().unwrap();
        cam.frame(lo, hi, 1.0);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        // The middle of the front view is the front face (normal toward +Z).
        let f = pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, rect.center(), SelectMode::Face).unwrap();
        let normal = m.face_normals()[f as usize];
        assert!(normal.z > 0.9, "{normal:?}");
        // Near a corner of the front face: that corner's vertex.
        let corner = project(cam.view_proj(1.0), rect, Vec3::new(hi.x, hi.y, hi.z)).unwrap();
        let v = pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, corner + egui::vec2(-6.0, 6.0), SelectMode::Vert).unwrap();
        assert!((m.positions[v as usize] - Vec3::new(hi.x, hi.y, hi.z)).length() < 1e-4);
        // Near the middle of the top edge of the front face: that edge.
        let top = project(cam.view_proj(1.0), rect, Vec3::new(0.5 * (lo.x + hi.x), hi.y, hi.z)).unwrap();
        let e = pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, top + egui::vec2(0.0, 4.0), SelectMode::Edge).unwrap();
        let [a, b] = m.edge_verts(e);
        assert!(m.positions[a as usize].y > hi.y - 1e-4 && m.positions[b as usize].y > hi.y - 1e-4);
        // Far from the mesh: nothing.
        assert_eq!(pick(&m, &bvh, Mat4::IDENTITY, &cam, rect, pos2(2.0, 2.0), SelectMode::Vert), None);
    }
}
