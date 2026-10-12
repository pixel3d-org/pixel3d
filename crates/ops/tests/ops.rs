//! Behaviour tests for the Phase 1 ops, run through the registry exactly as
//! callers do (JSON in, output out).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use glam::Vec3;
use p3d_mesh::{EdgeFlags, ElementKind};
use p3d_ops::{Applied, ErrorCode, OpCtx, OpError, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

fn run(tx: &mut Tx, op: &str, args: Value) -> Result<Applied, OpError> {
    let r = Registry::builtin();
    let op = r.get(op)?;
    let applied = op.run(tx, &args, &OpCtx::default())?;
    tx.doc().check().map_err(OpError::internal)?;
    for (_, o) in tx.doc().objects() {
        if let Some(m) = o.mesh_key().and_then(|k| tx.doc().mesh(k)) {
            m.validate().map_err(|e| OpError::internal(e.to_string()))?;
        }
    }
    Ok(applied)
}

fn ok(tx: &mut Tx, op: &str, args: Value) -> String {
    run(tx, op, args).unwrap_or_else(|e| panic!("{op}: {e}")).output.summary
}

fn err(tx: &mut Tx, op: &str, args: Value) -> OpError {
    let before = tx.clone();
    let e = run(tx, op, args).expect_err(op);
    // A failing op leaves the working copy unchanged in these cases (validation errors).
    assert_eq!(before.doc().object_count(), tx.doc().object_count());
    e
}

/// The fixture every op example runs against: crate (o:1, 0.6 m box) with
/// child lid (o:2, box) which has child hinge (o:3, empty), and post (o:3?).
fn fixture() -> Tx {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.box", json!({ "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6 }));
    ok(&mut tx, "prim.box", json!({ "name": "lid", "parent": "crate", "width": 0.6, "height": 0.05, "depth": 0.6, "position": [0, 0.6, 0] }));
    ok(&mut tx, "prim.cylinder", json!({ "name": "post", "radius": 0.05, "height": 1.0, "segments": 8, "position": [2, 0, 0] }));
    ok(&mut tx, "scene.create_empty", json!({ "name": "hinge", "parent": "crate/lid" }));
    tx
}

#[test]
fn every_example_produces_its_documented_result() {
    let r = Registry::builtin();
    let mut failures = Vec::new();
    for op in r.iter() {
        for ex in op.meta().examples {
            let args: Value = serde_json::from_str(ex.args).unwrap();
            let mut tx = if op.id().starts_with("prim.") { Tx::new(Document::new()) } else { fixture() };
            match op.run(&mut tx, &args, &OpCtx::default()) {
                Ok(a) if a.output.summary == ex.result => {}
                Ok(a) => failures.push(format!("{}: expected `{}`\n      got `{}`", op.id(), ex.result, a.output.summary)),
                Err(e) => failures.push(format!("{}: {e}", op.id())),
            }
        }
    }
    assert!(failures.is_empty(), "examples out of date:\n  {}", failures.join("\n  "));
}

#[test]
fn defaults_are_valid_parameters() {
    let r = Registry::builtin();
    for op in r.iter() {
        let defaults = op.default_params();
        // Ops without required fields must accept their own defaults.
        let schema = op.params_schema();
        let required = schema.get("required").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        if !required {
            let mut tx = Tx::new(Document::new());
            op.run(&mut tx, &defaults, &OpCtx::default()).unwrap_or_else(|e| panic!("{} defaults: {e}", op.id()));
        }
    }
}

#[test]
fn prim_box_with_bevel_and_groups() {
    let mut tx = Tx::new(Document::new());
    let a = run(
        &mut tx,
        "prim.box",
        json!({ "name": "crate", "width": "60cm", "height": 0.4, "depth": 0.5, "bevel": { "width": "5mm", "segments": 2 } }),
    )
    .unwrap();
    let key = a.output.created[0];
    let m = tx.doc().object_mesh(key).unwrap();
    assert!(m.faces().all(|f| m.face_len(f) == 4));
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi - lo).abs_diff_eq(Vec3::new(0.6, 0.4, 0.5), 1e-5));
    assert!(lo.y.abs() < 1e-6, "origin at the bottom by default");
    let names: Vec<&str> = a.output.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["right", "left", "top", "bottom", "front", "back", "bevel"]);
    assert_eq!(a.params["width"], json!(0.6), "canonical params are in meters");
    assert_eq!(a.params["bevel"]["width"], json!(0.005));
}

#[test]
fn prim_errors_are_precise() {
    let mut tx = Tx::new(Document::new());
    let e = err(&mut tx, "prim.box", json!({ "width": 1, "bevel": { "width": 0.6 } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/bevel/width")));
    assert!(e.hint.as_deref().is_some_and(|h| h.contains("500 mm") || h.contains("0.5 m")), "{e}");
    let e = err(&mut tx, "prim.box", json!({ "widht": 1 }));
    assert_eq!((e.code, e.suggestions.clone()), (ErrorCode::InvalidParam, vec!["width".to_owned()]));
    let e = err(&mut tx, "prim.cylinder", json!({ "segments": 10 }));
    assert_eq!(e.suggestions, vec!["8", "12"]);
    let e = err(&mut tx, "prim.box", json!({ "parent": "nothing" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::NotFound, Some("/parent")));
    ok(&mut tx, "prim.box", json!({ "name": "a" }));
    let e = err(&mut tx, "prim.box", json!({ "name": "a" }));
    assert_eq!(e.suggestions, vec!["a_1"]);
}

#[test]
fn adaptive_segments_follow_the_asset_class() {
    let mut tx = Tx::new(Document::new());
    let a = run(&mut tx, "prim.cylinder", json!({ "radius": 0.05 })).unwrap();
    let m = tx.doc().object_mesh(a.output.created[0]).unwrap();
    assert_eq!(m.group("side").unwrap().count(), 8, "0.31 m around / 0.04 m edges");
    tx.doc_mut().settings.asset_class = p3d_scene::AssetClass::WeaponFp;
    let a = run(&mut tx, "prim.cylinder", json!({ "radius": 0.05 })).unwrap();
    let m = tx.doc().object_mesh(a.output.created[0]).unwrap();
    assert_eq!(m.group("side").unwrap().count(), 64, "weapon_fp uses 5 mm edges, clamped to 64");
}

#[test]
fn duplicate_copy_and_instance() {
    let mut tx = fixture();
    let copy = run(&mut tx, "scene.duplicate", json!({ "target": "post", "offset": [1, 0, 0] })).unwrap().output.created[0];
    let inst = run(&mut tx, "scene.duplicate", json!({ "target": "post", "mode": "instance" })).unwrap().output.created[0];
    let post = tx.doc().resolve("post").unwrap();
    let mk = tx.doc().object(post).unwrap().mesh_key();
    assert_eq!(tx.doc().object(inst).unwrap().mesh_key(), mk);
    assert_ne!(tx.doc().object(copy).unwrap().mesh_key(), mk);
    assert_eq!(tx.doc().object(copy).unwrap().transform.translation, Vec3::new(3.0, 0.0, 0.0));
    assert_eq!(tx.doc().path(copy).unwrap(), "post_1");
    assert_eq!(tx.doc().path(inst).unwrap(), "post_2");
}

#[test]
fn transforms_and_hierarchy() {
    let mut tx = fixture();
    ok(&mut tx, "scene.set_transform", json!({ "target": "crate", "position": [1, 0, 0], "rotation": [0, 90, 0], "scale": 2 }));
    let lid = tx.doc().resolve("lid").unwrap();
    let world = tx.doc().world_affine(lid).unwrap();
    ok(&mut tx, "scene.unparent", json!({ "target": "lid" }));
    assert!(tx.doc().world_affine(lid).unwrap().abs_diff_eq(world, 1e-5));
    ok(&mut tx, "scene.parent", json!({ "target": "lid", "parent": "post", "keep_world": true }));
    assert!(tx.doc().world_affine(lid).unwrap().abs_diff_eq(world, 1e-5));
    let e = err(&mut tx, "scene.parent", json!({ "target": "post", "parent": "post/lid/hinge" }));
    assert_eq!(e.code, ErrorCode::InvalidParam);
    ok(&mut tx, "scene.set_transform", json!({ "target": "post", "position": [0, 1, 0], "relative": true }));
    let post = tx.doc().resolve("post").unwrap();
    assert_eq!(tx.doc().object(post).unwrap().transform.translation, Vec3::new(2.0, 1.0, 0.0));
    assert_eq!(err(&mut tx, "scene.set_transform", json!({ "target": "post", "scale": [1, 0, 1] })).code, ErrorCode::OutOfRange);
}

#[test]
fn apply_transform_bakes_and_children_stay() {
    let mut tx = fixture();
    ok(&mut tx, "scene.set_transform", json!({ "target": "crate", "position": [1, 2, 3], "rotation": [0, 30, 0], "scale": [2, 1, 1] }));
    let crate_ = tx.doc().resolve("crate").unwrap();
    let lid = tx.doc().resolve("lid").unwrap();
    let lid_world = tx.doc().world_affine(lid).unwrap();
    let world_pts: Vec<Vec3> = {
        let w = tx.doc().world_affine(crate_).unwrap();
        tx.doc().object_mesh(crate_).unwrap().positions.iter().map(|&p| w.transform_point3(p)).collect()
    };
    ok(&mut tx, "scene.apply_transform", json!({ "target": "crate" }));
    assert!(tx.doc().object(crate_).unwrap().transform.is_identity(1e-6));
    let after: Vec<Vec3> = tx.doc().object_mesh(crate_).unwrap().positions.clone();
    for (a, b) in after.iter().zip(&world_pts) {
        assert!(a.abs_diff_eq(*b, 1e-5));
    }
    assert!(tx.doc().world_affine(lid).unwrap().abs_diff_eq(lid_world, 1e-5));
    // Mirroring keeps faces outward.
    ok(&mut tx, "scene.set_transform", json!({ "target": "post", "scale": [-1, 1, 1] }));
    ok(&mut tx, "scene.apply_transform", json!({ "target": "post" }));
    let post = tx.doc().resolve("post").unwrap();
    let m = tx.doc().object_mesh(post).unwrap();
    let c = {
        let (lo, hi) = m.bounds().unwrap();
        (lo + hi) / 2.0
    };
    assert!(m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0));
}

#[test]
fn set_origin_keeps_world_geometry() {
    let mut tx = fixture();
    let crate_ = tx.doc().resolve("crate").unwrap();
    let lid = tx.doc().resolve("lid").unwrap();
    let before_lid = tx.doc().world_affine(lid).unwrap();
    let world = |tx: &Tx| {
        let w = tx.doc().world_affine(crate_).unwrap();
        tx.doc().object_mesh(crate_).unwrap().positions.iter().map(|&p| w.transform_point3(p)).collect::<Vec<_>>()
    };
    let before = world(&tx);
    let s = ok(&mut tx, "scene.set_origin", json!({ "target": "crate", "to": "bbox", "bbox": [0, 0, 0] }));
    assert_eq!(s, "origin of crate (o:1) moved by -0.3 0 -0.3 m");
    for (a, b) in world(&tx).iter().zip(&before) {
        assert!(a.abs_diff_eq(*b, 1e-5));
    }
    assert!(tx.doc().world_affine(lid).unwrap().abs_diff_eq(before_lid, 1e-5));
}

#[test]
fn join_and_separate_round_trip() {
    let mut tx = fixture();
    let crate_ = tx.doc().resolve("crate").unwrap();
    let a = run(&mut tx, "scene.join", json!({ "targets": ["crate", "post"] })).unwrap();
    assert_eq!(a.output.deleted, vec![3]);
    assert!(tx.doc().resolve("post").is_err());
    let m = tx.doc().object_mesh(crate_).unwrap();
    assert_eq!(m.face_count(), 6 + 32, "box plus 8-segment cylinder with quad-grid caps");
    // The post sat 2 m to the right in the world; it still does in crate's space.
    assert!(m.bounds().unwrap().1.x > 2.0);
    let a = run(&mut tx, "scene.separate", json!({ "target": "crate", "by": "loose_parts" })).unwrap();
    assert_eq!(a.output.created.len(), 1);
    assert_eq!(tx.doc().object_mesh(crate_).unwrap().face_count(), 6);
    let part = a.output.created[0];
    assert_eq!(tx.doc().path(part).unwrap(), "crate_part_1");
    let e = err(&mut tx, "scene.join", json!({ "targets": ["crate"] }));
    assert_eq!(e.path.as_deref(), Some("/targets"));
    let e = err(&mut tx, "scene.join", json!({ "targets": ["crate", "crate/lid/hinge"] }));
    assert_eq!(e.path.as_deref(), Some("/targets/1"), "empties cannot be joined");
}

#[test]
fn separate_by_groups_names_parts_after_groups() {
    let mut tx = fixture();
    let s = ok(&mut tx, "scene.separate", json!({ "target": "crate", "by": "groups", "groups": ["top", "front"] }));
    assert!(s.contains("top (o:5)") && s.contains("front (o:6)"), "{s}");
    let top = tx.doc().resolve("top").unwrap();
    assert_eq!(tx.doc().object_mesh(top).unwrap().face_count(), 1);
    let e = err(&mut tx, "scene.separate", json!({ "target": "crate", "by": "groups", "groups": ["tpo"] }));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn delete_tags_metadata_rename() {
    let mut tx = fixture();
    assert_eq!(ok(&mut tx, "scene.tag", json!({ "target": "crate", "add": ["low", "low", "prop"] })), "crate (o:1) tags: low, prop");
    assert_eq!(ok(&mut tx, "scene.tag", json!({ "target": "crate", "remove": ["low"] })), "crate (o:1) tags: prop");
    ok(&mut tx, "scene.set_metadata", json!({ "target": "crate", "set": { "a": 1, "b": [1, 2] }, "remove": ["zzz"] }));
    let c = tx.doc().resolve("crate").unwrap();
    assert_eq!(tx.doc().object(c).unwrap().metadata["b"], json!([1, 2]));
    ok(&mut tx, "scene.rename", json!({ "target": "lid", "name": "top_lid" }));
    assert!(tx.doc().resolve("crate/top_lid/hinge").is_ok());
    let s = ok(&mut tx, "scene.delete", json!({ "target": "crate", "recursive": false }));
    assert_eq!(s, "deleted crate (o:1)");
    assert!(tx.doc().resolve("top_lid/hinge").is_ok(), "children moved to the top level");
    let e = err(&mut tx, "scene.delete", json!({ "target": "o:1" }));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn select_ops() {
    let mut tx = fixture();
    let a = run(&mut tx, "select.save", json!({ "target": "crate", "select": { "edges": { "sharp": true } }, "name": "hard" })).unwrap();
    assert_eq!((a.output.groups[0].kind, a.output.groups[0].count), (ElementKind::Edge, 12));
    let e = err(&mut tx, "select.save", json!({ "target": "crate", "select": { "faces": { "group": "tpo" } }, "name": "x" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::NotFound, Some("/select/faces/group")));
    let e = err(&mut tx, "select.save", json!({ "target": "crate", "select": { "faces": { "material": 9 } }, "name": "x" }));
    assert_eq!(e.code, ErrorCode::EmptySelection);
    ok(&mut tx, "select.grow", json!({ "target": "crate", "group": "top", "steps": 1, "save_as": "top_ring" }));
    let c = tx.doc().resolve("crate").unwrap();
    assert_eq!(tx.doc().object_mesh(c).unwrap().group("top_ring").unwrap().count(), 5);
    assert_eq!(tx.doc().object_mesh(c).unwrap().group("top").unwrap().count(), 1, "save_as keeps the original");
    ok(&mut tx, "select.shrink", json!({ "target": "crate", "group": "top_ring" }));
    assert_eq!(tx.doc().object_mesh(c).unwrap().group("top_ring").unwrap().count(), 1, "only top has all neighbours inside");
    ok(&mut tx, "select.group_rename", json!({ "target": "crate", "from": "hard", "to": "hard_edges" }));
    let e = err(&mut tx, "select.group_rename", json!({ "target": "crate", "from": "top", "to": "bottom" }));
    assert_eq!(e.path.as_deref(), Some("/to"));
    ok(&mut tx, "select.group_delete", json!({ "target": "crate", "group": "hard_edges" }));
    let e = err(&mut tx, "select.group_delete", json!({ "target": "crate", "group": "hard_edgse" }));
    assert_eq!(e.suggestions, Vec::<String>::new());
    let m = tx.doc().object_mesh(c).unwrap();
    assert!(m.edges().any(|e| m.edge_flags(e).contains(EdgeFlags::SHARP)), "geometry and flags stay");
}
