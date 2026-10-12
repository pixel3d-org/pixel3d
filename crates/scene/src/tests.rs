use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3};
use p3d_math::Trs;
use p3d_mesh::Mesh;

use crate::{Document, ObjectKind, OpNode, SceneError};

fn tri() -> Mesh {
    Mesh::from_polygons(vec![Vec3::ZERO, Vec3::X, Vec3::Y], &[3], &[0, 1, 2]).unwrap()
}

fn chest() -> Document {
    let mut d = Document::new();
    let chest = d.create_object("chest", None, ObjectKind::Empty).unwrap();
    d.create_mesh_object("body", Some(chest), tri()).unwrap();
    let lid = d.create_mesh_object("lid", Some(chest), tri()).unwrap();
    d.create_object("hinge", Some(lid), ObjectKind::Empty).unwrap();
    d.check().unwrap();
    d
}

#[test]
fn paths_ids_and_resolution() {
    let d = chest();
    let lid = d.resolve("chest/lid").unwrap();
    assert_eq!(d.path(lid).unwrap(), "chest/lid");
    assert_eq!(d.object(lid).unwrap().public_id(), "o:3");
    assert_eq!(d.resolve("o:3").unwrap(), lid);
    assert_eq!(d.resolve("lid").unwrap(), lid, "unique leaf name");
    assert_eq!(d.path(d.resolve("hinge").unwrap()).unwrap(), "chest/lid/hinge");
    let names: Vec<&str> = d.objects().map(|(_, o)| o.name()).collect();
    assert_eq!(names, vec!["chest", "body", "lid", "hinge"], "creation order");
    assert_eq!(d.roots().count(), 1);
    match d.resolve("lidd") {
        Err(SceneError::NotFound { suggestions, .. }) => assert!(suggestions.contains(&"lid".to_owned()), "{suggestions:?}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(d.resolve("o:99"), Err(SceneError::NotFound { .. })));
}

#[test]
fn leaf_names_can_repeat_across_parents() {
    let mut d = chest();
    let other = d.create_object("chest2", None, ObjectKind::Empty).unwrap();
    d.create_object("lid", Some(other), ObjectKind::Empty).unwrap();
    match d.resolve("lid") {
        Err(SceneError::Ambiguous { candidates, .. }) => assert_eq!(candidates, vec!["chest/lid", "chest2/lid"]),
        other => panic!("{other:?}"),
    }
    assert!(d.resolve("chest2/lid").is_ok());
    let err = d.create_object("lid", Some(other), ObjectKind::Empty).unwrap_err();
    assert_eq!(err, SceneError::NameTaken { path: "chest2/lid".into(), suggestion: "lid_1".into() });
    assert!(matches!(d.create_object("a/b", None, ObjectKind::Empty), Err(SceneError::InvalidName(_))));
    d.check().unwrap();
}

#[test]
fn rename_moves_subtree_paths() {
    let mut d = chest();
    let lid = d.resolve("chest/lid").unwrap();
    d.rename(lid, "top").unwrap();
    assert!(d.resolve("chest/top/hinge").is_ok());
    assert!(d.resolve("chest/lid/hinge").is_err());
    let body = d.resolve("body").unwrap();
    assert!(matches!(d.rename(body, "top"), Err(SceneError::NameTaken { .. })));
    d.check().unwrap();
}

#[test]
fn reparent_keeps_world_transform_and_rejects_cycles() {
    let mut d = chest();
    let chest = d.resolve("chest").unwrap();
    let lid = d.resolve("lid").unwrap();
    let hinge = d.resolve("hinge").unwrap();
    d.object_mut(chest).unwrap().transform =
        Trs { translation: Vec3::new(1.0, 2.0, 3.0), rotation: Quat::from_rotation_y(0.7), scale: Vec3::splat(2.0) };
    d.object_mut(lid).unwrap().transform = Trs::from_translation(Vec3::new(0.0, 0.5, 0.0));
    let before = d.world_affine(hinge).unwrap();
    d.set_parent(hinge, None, true).unwrap();
    assert!(d.world_affine(hinge).unwrap().abs_diff_eq(before, 1e-5));
    assert_eq!(d.path(hinge).unwrap(), "hinge");
    assert!(matches!(d.set_parent(chest, Some(lid), false), Err(SceneError::Cycle { .. })));
    d.set_parent(hinge, Some(lid), false).unwrap();
    assert_eq!(d.path(hinge).unwrap(), "chest/lid/hinge");
    d.check().unwrap();
}

#[test]
fn delete_recursive_and_flat() {
    let mut d = chest();
    let meshes = d.mesh_count();
    let lid = d.resolve("lid").unwrap();
    assert_eq!(d.delete(lid, true).unwrap(), vec![3, 4]);
    assert_eq!(d.mesh_count(), meshes - 1, "lid's mesh is gone");
    d.check().unwrap();

    let mut d = chest();
    let chest = d.resolve("chest").unwrap();
    d.create_object("body", None, ObjectKind::Empty).unwrap();
    d.object_mut(chest).unwrap().transform = Trs::from_translation(Vec3::X);
    d.delete(chest, false).unwrap();
    // Children moved to the top level; `body` was taken there.
    assert!(d.resolve("body_1").is_ok() && d.resolve("lid/hinge").is_ok());
    let lid = d.resolve("lid").unwrap();
    assert!(d.world_affine(lid).unwrap().abs_diff_eq(Affine3A::from_translation(Vec3::X), 1e-6));
    d.check().unwrap();
}

#[test]
fn ids_are_never_reused() {
    let mut d = chest();
    let hinge = d.resolve("hinge").unwrap();
    d.delete(hinge, true).unwrap();
    let k = d.create_object("hinge", None, ObjectKind::Empty).unwrap();
    assert_eq!(d.object(k).unwrap().id(), 5);
    assert!(matches!(d.create_object_with_id(1, "x", None, ObjectKind::Empty), Err(SceneError::IdTaken(1))));
}

#[test]
fn instances_and_copies_share_mesh_data() {
    let mut d = Document::new();
    let a = d.create_mesh_object("a", None, tri()).unwrap();
    let mk = d.object(a).unwrap().mesh_key().unwrap();
    let b = d.create_object("b", None, ObjectKind::Mesh { mesh: mk }).unwrap();
    assert_eq!(d.mesh_users(mk), 2);
    let copy = d.make_mesh_unique(b).unwrap();
    assert_ne!(copy, mk);
    assert!(Arc::ptr_eq(d.mesh_arc(copy).unwrap(), d.mesh_arc(mk).unwrap()), "copy shares data until edited");
    d.mesh_mut(copy).unwrap().positions[0] = Vec3::ONE;
    assert_eq!(d.mesh(mk).unwrap().positions[0], Vec3::ZERO);
    assert_eq!((d.mesh_users(mk), d.mesh_users(copy)), (1, 1));
    d.delete(b, true).unwrap();
    assert_eq!(d.mesh_count(), 1);
    d.check().unwrap();
}

#[test]
fn snapshots_copy_only_what_changes() {
    let d = chest();
    let mut next = d.clone();
    let lid = next.resolve("lid").unwrap();
    let body = next.resolve("body").unwrap();
    next.object_mut(lid).unwrap().visible = false;
    assert!(d.object(lid).unwrap().visible, "the snapshot is untouched");
    assert!(next.shares_object(&d, body), "untouched objects are shared");
    assert!(!next.shares_object(&d, lid));
}

#[test]
fn history_is_appended() {
    let mut d = chest();
    let lid = d.resolve("lid").unwrap();
    assert!(d.history(lid).is_none());
    for i in 0..3 {
        d.push_history(
            lid,
            OpNode { op: "edit.extrude".into(), version: 1, params: serde_json::json!({ "distance": i }), label: format!("extrude_{i}") },
        )
        .unwrap();
    }
    assert_eq!(d.history(lid).unwrap().len(), 3);
}

#[test]
fn flat_delete_renames_colliding_children_without_duplicates() {
    let mut d = Document::new();
    d.create_object("a", None, ObjectKind::Empty).unwrap();
    let box_ = d.create_object("box", None, ObjectKind::Empty).unwrap();
    // Children "a" and "a_1": "a" is taken at the top level, so it becomes
    // "a_1", which forces the original "a_1" to "a_1_1".
    d.create_object("a", Some(box_), ObjectKind::Empty).unwrap();
    d.create_object("a_1", Some(box_), ObjectKind::Empty).unwrap();
    d.delete(box_, false).unwrap();
    d.check().unwrap();
    let mut names: Vec<String> = d.objects().map(|(k, _)| d.path(k).unwrap()).collect();
    names.sort();
    assert_eq!(names, vec!["a", "a_1", "a_1_1"]);
}

#[test]
fn curve_objects_own_their_curves() {
    use crate::{Curve, Spline};
    let mut d = Document::new();
    let c = Curve::single(Spline::polyline([glam::Vec3::ZERO, glam::Vec3::X], false));
    let k = d.create_curve_object("path", None, c.clone()).unwrap();
    assert_eq!(d.object_curve(k), Some(&c));
    assert_eq!(d.object(k).unwrap().kind().as_str(), "curve");
    d.check().unwrap();
    // A curve cannot belong to two objects.
    let ck = d.object(k).unwrap().curve_key().unwrap();
    assert!(matches!(d.create_object("again", None, ObjectKind::Curve { curve: ck }), Err(SceneError::StaleCurve)));
    // Undo snapshots keep the old curve.
    let snap = d.clone();
    d.curve_mut(ck).unwrap().splines[0].closed = true;
    assert!(!snap.curve(ck).unwrap().splines[0].closed);
    // Turning it into an empty or deleting it drops the curve.
    d.set_kind(k, ObjectKind::Empty).unwrap();
    assert!(d.curve(ck).is_none());
    d.check().unwrap();
    let k2 = d.create_curve_object("path2", None, c).unwrap();
    d.delete(k2, false).unwrap();
    assert_eq!(d.curve_entries().count(), 0);
    d.check().unwrap();
}
