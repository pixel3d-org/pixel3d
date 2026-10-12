//! Small scenes for perception, diagnostics and agent-surface tests, built
//! directly on the document (no ops), in meters.

use glam::{DVec3, Quat, Vec3};
use p3d_geo::prim::{BoxSpec, Origin, QuadSphereSpec, cuboid, quad_sphere};
use p3d_mesh::Mesh;
use p3d_scene::{Document, ObjectKey};

/// A box with the bottom-center origin; `bevel` 0 for sharp edges.
pub fn box_mesh(size: [f64; 3], bevel: f64) -> Mesh {
    let spec = BoxSpec { size: DVec3::from(size), segments: [1; 3], bevel_width: bevel, bevel_segments: if bevel > 0.0 { 2 } else { 0 } };
    cuboid(&spec, Origin::Bottom).unwrap_or_default()
}

/// A quad sphere of `radius` centered on its origin.
pub fn sphere_mesh(radius: f64, segments: u32) -> Mesh {
    quad_sphere(&QuadSphereSpec { radius, segments }, Origin::Center).unwrap_or_default()
}

/// Adds a mesh object at `at` (parent space). Panics on a name clash, which
/// is a bug in the fixture.
pub fn add(doc: &mut Document, name: &str, parent: Option<ObjectKey>, mesh: Mesh, at: [f32; 3]) -> ObjectKey {
    let key = doc.create_mesh_object(name, parent, mesh).unwrap_or_else(|e| unreachable!("fixture object {name}: {e}"));
    if let Some(o) = doc.object_mut(key) {
        o.transform.translation = Vec3::from(at);
    }
    key
}

/// Rotates an object about Y by `deg` degrees.
pub fn turn_y(doc: &mut Document, key: ObjectKey, deg: f32) {
    if let Some(o) = doc.object_mut(key) {
        o.transform.rotation = Quat::from_rotation_y(deg.to_radians());
    }
}

/// The crate scene:
///
/// - `crate` (o:1): 600 mm cube with an 8 mm bevel, standing on the ground;
/// - `lid` (o:2, child of crate): 620 x 40 x 620 mm, resting on the crate;
/// - `handle_l` (o:3) and `handle_r` (o:4), children of crate: 30 x 40 x
///   160 mm bars against the left and right sides, 400 mm up.
pub fn crate_scene() -> Document {
    let mut doc = Document::new();
    let c = add(&mut doc, "crate", None, box_mesh([0.6, 0.6, 0.6], 0.008), [0.0; 3]);
    add(&mut doc, "lid", Some(c), box_mesh([0.62, 0.04, 0.62], 0.0), [0.0, 0.6, 0.0]);
    add(&mut doc, "handle_l", Some(c), box_mesh([0.03, 0.04, 0.16], 0.0), [-0.315, 0.4, 0.0]);
    add(&mut doc, "handle_r", Some(c), box_mesh([0.03, 0.04, 0.16], 0.0), [0.315, 0.4, 0.0]);
    doc
}

/// A head-like scene with curved parts: `head` (o:1, 100 mm sphere at 150
/// mm up), `ear_l` and `ear_r` (o:2, o:3, 30 mm spheres sunk into its
/// sides) and `hat` (o:4, a box floating 20 mm above the head).
pub fn head_scene() -> Document {
    let mut doc = Document::new();
    add(&mut doc, "head", None, sphere_mesh(0.1, 8), [0.0, 0.15, 0.0]);
    add(&mut doc, "ear_l", None, sphere_mesh(0.03, 4), [-0.11, 0.16, 0.0]);
    add(&mut doc, "ear_r", None, sphere_mesh(0.03, 4), [0.11, 0.16, 0.0]);
    add(&mut doc, "hat", None, box_mesh([0.12, 0.05, 0.12], 0.0), [0.0, 0.27, 0.0]);
    doc
}

/// One part per shape family, spaced 1 m apart along X:
///
/// - `post` (o:1): cylinder, 100 mm radius, 500 mm tall, 16 segments;
/// - `plank` (o:2): 800 x 50 x 200 mm box turned 30 degrees about Y;
/// - `floor` (o:3): 1 x 1 m plane;
/// - `bin` (o:4): 400 x 300 x 400 mm box without its top (open).
pub fn shapes_scene() -> Document {
    use p3d_geo::prim::{CapKind, CylinderSpec, PlaneSpec, cylinder, plane};
    let mut doc = Document::new();
    let post = cylinder(&CylinderSpec { radius: 0.1, height: 0.5, segments: 16, height_segments: 1, cap: CapKind::QuadGrid }, Origin::Bottom)
        .unwrap_or_default();
    add(&mut doc, "post", None, post, [0.0, 0.0, 0.0]);
    let plank = add(&mut doc, "plank", None, box_mesh([0.8, 0.05, 0.2], 0.0), [1.5, 0.0, 0.0]);
    turn_y(&mut doc, plank, 30.0);
    let floor = plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: 4, segments_z: 4 }).unwrap_or_default();
    add(&mut doc, "floor", None, floor, [3.0, 0.0, 0.0]);
    let mut bin = box_mesh([0.4, 0.3, 0.4], 0.0);
    let top: Vec<u32> = bin.group("top").map(|g| g.iter().collect()).unwrap_or_default();
    let _ = bin.delete_faces(&top);
    add(&mut doc, "bin", None, bin, [4.5, 0.0, 0.0]);
    doc
}
