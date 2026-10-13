//! Behaviour tests for the Phase 1 ops, run through the registry exactly as
//! callers do (JSON in, output out).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use glam::Vec3;
use p3d_mesh::{EdgeFlags, ElementKind};
use p3d_ops::{Applied, ErrorCode, OpCtx, OpError, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

mod fixtures;
use fixtures::*;

#[test]
fn every_example_produces_its_documented_result() {
    let r = Registry::builtin();
    let mut failures = Vec::new();
    let (_dir, files) = io_folder();
    for op in r.iter() {
        for ex in op.meta().examples {
            let args: Value = serde_json::from_str(ex.args).unwrap();
            let mut tx = for_category(op.meta().category);
            match op.run(&mut tx, &args, &OpCtx { files: Some(&files), ..OpCtx::default() }) {
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
fn prim_cone_points_and_frustums() {
    let mut tx = Tx::new(Document::new());
    let a = run(&mut tx, "prim.cone", json!({ "name": "spike", "radius_bottom": "2cm", "height": "10cm", "segments": 8, "cap": "ngon" })).unwrap();
    let names: Vec<&str> = a.output.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["side", "cap_bottom"], "a point has no cap");
    let m = tx.doc().object_mesh(a.output.created[0]).unwrap();
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi - lo).abs_diff_eq(Vec3::new(0.04, 0.1, 0.04), 1e-5));
    let e = err(&mut tx, "prim.cone", json!({ "radius_bottom": 0, "radius_top": 0 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/radius_top")));
    let e = err(&mut tx, "prim.cone", json!({ "segments": 6 }));
    assert_eq!(e.suggestions, vec!["4", "8"]);
    let e = err(&mut tx, "prim.cone", json!({ "radius_top": "-1cm" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/radius_top")));
}

#[test]
fn ring_and_round_primitives_check_their_sizes() {
    let mut tx = Tx::new(Document::new());
    let e = err(&mut tx, "prim.torus", json!({ "major_radius": 0.1, "minor_radius": 0.2 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/minor_radius")));
    let e = err(&mut tx, "prim.tube", json!({ "radius": 0.1, "inner_radius": 0.1 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/inner_radius")));
    let e = err(&mut tx, "prim.capsule", json!({ "radius": 0.2, "height": 0.4 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/height")));
    let e = err(&mut tx, "prim.capsule", json!({ "segments": 12 }));
    assert_eq!(e.suggestions, vec!["8", "16"]);
    let groups = |a: &Applied| a.output.groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
    let a = run(&mut tx, "prim.tube", json!({ "name": "pipe" })).unwrap();
    assert_eq!(groups(&a), vec!["outer", "inner", "cap_top", "cap_bottom"]);
    let a = run(&mut tx, "prim.wedge", json!({ "name": "ramp" })).unwrap();
    assert_eq!(groups(&a), vec!["bottom", "back", "slope", "left", "right"]);
    let a = run(&mut tx, "prim.capsule", json!({ "name": "pill", "radius": 0.1, "height": 0.5 })).unwrap();
    let m = tx.doc().object_mesh(a.output.created[0]).unwrap();
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi - lo).abs_diff_eq(Vec3::new(0.2, 0.5, 0.2), 1e-5), "{}", hi - lo);
    assert!(m.faces().all(|f| m.face_len(f) == 4));
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

#[test]
fn join_into_a_nested_target_is_refused() {
    let mut tx = fixture();
    let e = err(&mut tx, "scene.join", json!({ "targets": ["crate", "crate/lid"], "into": "crate/lid" }));
    assert_eq!(e.path.as_deref(), Some("/into"));
    // The other way round works: lid's geometry joins crate, hinge moves to crate.
    let s = ok(&mut tx, "scene.join", json!({ "targets": ["crate", "crate/lid"] }));
    assert_eq!(s, "joined 2 objects into crate (o:1): 12 faces");
    assert!(tx.doc().resolve("crate/hinge").is_ok());
}

fn topo(tx: &Tx, target: &str) -> p3d_analyze::Topology {
    let key = tx.doc().resolve(target).unwrap();
    p3d_analyze::topology(tx.doc().object_mesh(key).unwrap())
}

#[test]
fn fill_closes_holes_with_labelled_groups() {
    let mut tx = edit_fixture();
    assert_eq!(topo(&tx, "bin").boundary_loops, 1);
    let a = run(&mut tx, "edit.fill", json!({ "target": "bin" })).unwrap();
    let t = topo(&tx, "bin");
    assert!(t.is_watertight() && t.volume > 0.0, "{t:?}");
    assert_eq!(a.output.groups[0].name, "fill_1.faces");
    let e = err(&mut tx, "edit.fill", json!({ "target": "bin" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::EmptySelection, Some("/select")));

    // A second fill on the same mesh does not overwrite fill_1's group.
    ok(&mut tx, "edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "fill_1.faces" } } }));
    ok(&mut tx, "edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "bottom" } } }));
    assert_eq!(topo(&tx, "bin").boundary_loops, 2);
    // Only the hole touching the selection is filled.
    let a =
        run(&mut tx, "edit.fill", json!({ "target": "bin", "select": { "edges": { "in_box": { "min": [-1, -0.01, -1], "max": [1, 0.01, 1] } } } }))
            .unwrap();
    assert_eq!(a.output.groups[0].name, "fill_2.faces");
    assert_eq!(topo(&tx, "bin").boundary_loops, 1);
    let key = tx.doc().resolve("bin").unwrap();
    let mesh = tx.doc().object_mesh(key).unwrap();
    let f = mesh.group("fill_2.faces").unwrap().iter().next().unwrap();
    assert!(mesh.face_normal(f).y < -0.99, "the bottom cap faces down");

    // An explicit call label names the group.
    let r = Registry::builtin();
    let ctx = OpCtx { label: Some("lid_cap"), ..OpCtx::default() };
    let a = r.get("edit.fill").unwrap().run(&mut tx, &json!({ "target": "bin" }), &ctx).unwrap();
    assert_eq!(a.output.groups[0].name, "lid_cap.faces");
    assert!(topo(&tx, "bin").is_watertight());
}

#[test]
fn recalc_normals_flips_inverted_parts() {
    let mut tx = edit_fixture();
    let key = tx.doc().resolve("crate").unwrap();
    tx.mesh_mut(key, "").unwrap().flip_all();
    assert_eq!(topo(&tx, "crate").inverted_parts, 1);
    let s = ok(&mut tx, "edit.recalc_normals", json!({ "target": "crate" }));
    assert_eq!(s, "crate (o:1): flipped 1 of 1 part (6 faces); normals face outward");
    assert_eq!(topo(&tx, "crate").inverted_parts, 0);
    assert!(topo(&tx, "crate").volume > 0.0);
    let s = ok(&mut tx, "edit.recalc_normals", json!({ "target": "crate", "inside": true }));
    assert!(s.contains("face inward"), "{s}");
    assert!(topo(&tx, "crate").volume < 0.0);

    // Open parts: the bin's faces point away from its center.
    let s = ok(&mut tx, "edit.recalc_normals", json!({ "target": "bin" }));
    assert!(s.contains("already face"), "{s}");
    let bin = tx.doc().resolve("bin").unwrap();
    tx.mesh_mut(bin, "").unwrap().flip_all();
    let s = ok(&mut tx, "edit.recalc_normals", json!({ "target": "bin" }));
    assert!(s.contains("flipped 1 of 1 part"), "{s}");
}

#[test]
fn delete_elements_by_faces_edges_and_vertices() {
    let mut tx = edit_fixture();
    let s = ok(
        &mut tx,
        "edit.delete_elements",
        json!({ "target": "crate", "select": { "edges": { "in_box": { "min": [-1, 0.59, -1], "max": [1, 0.61, 1] } } } }),
    );
    // The top edges touch the top and the four sides: only the bottom is left.
    assert_eq!(s, "crate (o:1): deleted 5 faces, 8 edges and 4 vertices; 1 hole opened");
    let s = ok(
        &mut tx,
        "edit.delete_elements",
        json!({ "target": "post", "select": { "verts": { "in_box": { "min": [-1, 0.99, -1], "max": [1, 1.01, 1] } } } }),
    );
    assert!(s.contains("1 hole opened"), "{s}");
    let e =
        err(&mut tx, "edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "normal": { "dir": [0, 1, 0], "within_deg": 1 } } } }));
    assert_eq!(e.code, ErrorCode::EmptySelection);
    let e = err(&mut tx, "edit.delete_elements", json!({ "target": "hinge", "select": { "faces": {} } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/target")));
    let a = run(&mut tx, "edit.delete_elements", json!({ "target": "bin", "select": { "faces": {} } })).unwrap();
    assert_eq!(a.output.warnings[0].fix.as_ref().map(|f| f.op.as_str()), Some("scene.delete"));
}

#[test]
fn extrude_and_inset_make_labelled_groups() {
    let mut tx = edit_fixture();
    let a = run(&mut tx, "edit.inset", json!({ "target": "crate", "select": { "faces": { "group": "top" } }, "thickness": "5cm" })).unwrap();
    let names: Vec<&str> = a.output.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["inset_1.inner", "inset_1.rim"]);
    let a =
        run(&mut tx, "edit.extrude", json!({ "target": "crate", "select": { "faces": { "group": "inset_1.inner" } }, "distance": "-2cm" })).unwrap();
    assert_eq!(a.output.groups.iter().map(|g| (g.name.as_str(), g.count)).collect::<Vec<_>>(), vec![("extrude_1.cap", 1), ("extrude_1.side", 4)]);
    let t = topo(&tx, "crate");
    assert!(t.is_watertight() && t.volume > 0.0, "{t:?}");
    let key = tx.doc().resolve("crate").unwrap();
    let m = tx.doc().object_mesh(key).unwrap();
    assert!((m.face_centroid(m.group("extrude_1.cap").unwrap().iter().next().unwrap()).y - 0.58).abs() < 1e-5);
    assert_eq!(m.group("top").unwrap().count(), 5, "the recess floor and the rim stay in top");

    let e = err(&mut tx, "edit.extrude", json!({ "target": "crate", "select": { "edges": { "sharp": true } } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/select")));
    let e = err(&mut tx, "edit.extrude", json!({ "target": "crate", "select": { "faces": {} }, "distance": 0 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/distance")));
    let e = err(&mut tx, "edit.inset", json!({ "target": "crate", "select": { "faces": { "group": "front" } }, "thickness": "40cm" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/thickness")));
    assert!(e.hint.as_deref().is_some_and(|h| h.contains("shortest edge")), "{e}");
    let e = err(&mut tx, "edit.extrude", json!({ "target": "crate", "select": { "faces": { "group": "nope" } } }));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn split_flip_and_subdivide() {
    let mut tx = edit_fixture();
    // Split the lid of the crate off and make it its own object.
    ok(&mut tx, "edit.split", json!({ "target": "crate", "select": { "faces": { "group": "top" } } }));
    let s = ok(&mut tx, "scene.separate", json!({ "target": "crate", "by": "groups", "groups": ["split_1.faces"] }));
    assert!(s.contains("split_1.faces"), "{s}");
    assert_eq!(topo(&tx, "crate").boundary_loops, 1);
    // Flipping part of a part flips all of it, with a warning.
    let a = run(&mut tx, "edit.flip", json!({ "target": "bin", "select": { "faces": { "group": "bottom" } } })).unwrap();
    assert_eq!(a.output.summary, "bin (o:5): flipped 1 part (5 faces)");
    assert!(a.output.warnings[0].message.contains("4 faces outside the selection"), "{:?}", a.output.warnings);
    let a = run(&mut tx, "edit.subdivide", json!({ "target": "post", "levels": 2 })).unwrap();
    assert!(a.output.summary.contains("into 512 (2 levels)"), "{}", a.output.summary);
    assert!(topo(&tx, "post").is_watertight());
    let e = err(&mut tx, "edit.subdivide", json!({ "target": "post", "levels": 7 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/levels")));
}

#[test]
fn loop_cut_and_bridge_refuse_bad_selections() {
    let mut tx = edit_fixture();
    let e = err(&mut tx, "edit.loop_cut", json!({ "target": "crate", "select": { "faces": { "group": "top" } } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/select")));
    let e = err(&mut tx, "edit.loop_cut", json!({ "target": "crate", "select": { "edges": {} }, "offset": 2 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/offset")));
    let e = err(&mut tx, "edit.bridge", json!({ "target": "bin" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::EmptySelection, Some("/select")));
    assert!(e.message.contains("1 hole"), "{e}");
    // Bridging the duct's inner holes leaves one long tube open at both ends.
    ok(
        &mut tx,
        "edit.bridge",
        json!({ "target": "duct", "select": { "edges": { "in_box": { "min": [-1, 0.39, -1], "max": [1, 0.61, 1] } } }, "segments": 2 }),
    );
    assert_eq!(topo(&tx, "duct").boundary_loops, 2);
    assert_eq!(topo(&tx, "duct").parts, 1);
}

#[test]
fn merge_weld_and_dissolve() {
    let mut tx = edit_fixture();
    let s = ok(&mut tx, "edit.merge_by_distance", json!({ "target": "panels" }));
    assert!(s.contains("welded 2 vertices"), "{s}");
    assert_eq!(topo(&tx, "panels").boundary_loops, 1, "one sheet now");
    let s = ok(&mut tx, "edit.merge_by_distance", json!({ "target": "panels" }));
    assert!(s.contains("no vertices within"), "{s}");
    // Dissolving the shared seam edge makes one 6-corner polygon.
    let s = ok(&mut tx, "edit.dissolve", json!({ "target": "panels", "select": { "edges": { "boundary": false } } }));
    assert!(s.contains("dissolved into 1 polygon"), "{s}");
    let e = err(&mut tx, "edit.dissolve", json!({ "target": "panels" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/select")));
    let e = err(&mut tx, "edit.merge", json!({ "target": "crate", "select": { "verts": { "in_box": { "min": [9, 9, 9], "max": [9, 9, 9] } } } }));
    assert_eq!(e.code, ErrorCode::EmptySelection);
    let e = err(&mut tx, "edit.merge_by_distance", json!({ "target": "crate", "distance": 0 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/distance")));
    // Collapsing the crate's top face to a point makes a pyramid roof.
    ok(&mut tx, "edit.merge", json!({ "target": "crate", "select": { "faces": { "group": "top" } }, "mode": "collapse" }));
    let t = topo(&tx, "crate");
    assert!(t.is_watertight() && t.faces == 5, "{t:?}");
}

#[test]
fn cut_mirror_thicken_and_smooth() {
    let mut tx = edit_fixture();
    let e = err(&mut tx, "edit.plane_cut", json!({ "target": "crate", "normal": [0, 0, 0] }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/normal")));
    let s = ok(&mut tx, "edit.plane_cut", json!({ "target": "crate", "point": [0, 5, 0] }));
    assert!(s.contains("misses the mesh"), "{s}");
    // Shift the crate's mesh right, then symmetrize: it is centered again.
    ok(&mut tx, "edit.plane_cut", json!({ "target": "crate", "point": ["10cm", 0, 0], "normal": [1, 0, 0], "keep": "below", "fill": true }));
    ok(&mut tx, "edit.symmetrize", json!({ "target": "crate" }));
    let key = tx.doc().resolve("crate").unwrap();
    let (lo, hi) = tx.doc().object_mesh(key).unwrap().bounds().unwrap();
    assert!((lo.x + 0.1).abs() < 1e-5 && (hi.x - 0.1).abs() < 1e-5, "{lo} {hi}");
    assert!(topo(&tx, "crate").is_watertight());
    let e = err(&mut tx, "edit.thicken", json!({ "target": "crate", "select": { "faces": { "group": "top" } } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/select")));
    let s = ok(&mut tx, "edit.smooth", json!({ "target": "panels" }));
    assert!(s.contains("smoothed 0 vertices"), "{s}");
    let a = run(&mut tx, "edit.smooth", json!({ "target": "panels" })).unwrap();
    assert_eq!(a.output.warnings.len(), 1, "all border vertices are pinned");
}

#[test]
fn bevel_edges_with_clamping() {
    let mut tx = edit_fixture();
    let a = run(&mut tx, "edit.bevel", json!({ "target": "crate", "select": { "edges": { "sharp": true } }, "width": "50cm" })).unwrap();
    assert!(a.output.warnings.iter().any(|w| w.message.contains("clamped")), "{:?}", a.output.warnings);
    assert!(topo(&tx, "crate").is_watertight());
    let e =
        err(&mut tx, "edit.bevel", json!({ "target": "lid", "select": { "edges": {} }, "width": "50cm", "advanced": { "clamp_overlap": false } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/width")));
    let e = err(&mut tx, "edit.bevel", json!({ "target": "lid", "select": { "verts": {} } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/select")));
    let e = err(&mut tx, "edit.bevel", json!({ "target": "bin", "select": { "edges": { "boundary": true } } }));
    assert_eq!(e.code, ErrorCode::EmptySelection);
}

#[test]
fn shading_flags_normals_and_creases() {
    let mut tx = edit_fixture();
    let key = tx.doc().resolve("crate").unwrap();
    let sharp = |tx: &Tx| {
        let m = tx.doc().object_mesh(key).unwrap();
        m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count()
    };
    ok(&mut tx, "shade.set_smooth", json!({ "target": "crate" }));
    assert_eq!(sharp(&tx), 0);
    ok(&mut tx, "shade.mark_sharp", json!({ "target": "crate", "select": { "faces": { "group": "top" } } }));
    assert_eq!(sharp(&tx), 4);
    ok(&mut tx, "shade.weighted_normals", json!({ "target": "crate", "weighting": "area_angle" }));
    assert!(tx.doc().object_mesh(key).unwrap().corner_normal.is_some());
    ok(&mut tx, "shade.set_flat", json!({ "target": "crate" }));
    assert!(tx.doc().object_mesh(key).unwrap().corner_normal.is_none(), "flat shading drops custom normals");
    assert_eq!(sharp(&tx), 12);
    ok(
        &mut tx,
        "shade.crease",
        json!({ "target": "crate", "select": { "verts": { "in_box": { "min": [-1, 0.5, -1], "max": [1, 1, 1] } } }, "weight": 0.5 }),
    );
    let m = tx.doc().object_mesh(key).unwrap();
    assert_eq!(m.edges().filter(|&e| m.edge_crease(e) == 0.5).count(), 4);
    let e = err(&mut tx, "shade.crease", json!({ "target": "crate", "select": { "edges": {} }, "weight": 2 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/weight")));
}

#[test]
fn catmull_clark_subdivides_and_guards_the_budget() {
    let mut tx = edit_fixture();
    let a = run(&mut tx, "subd.catmull_clark", json!({ "target": "crate", "levels": 2 })).unwrap();
    assert!(a.output.warnings.is_empty());
    assert!(topo(&tx, "crate").is_watertight());
    let a = run(&mut tx, "subd.catmull_clark", json!({ "target": "crate", "levels": 3 })).unwrap();
    assert!(a.output.warnings[0].message.contains("exceed"), "{:?}", a.output.warnings);
    let e = err(&mut tx, "subd.catmull_clark", json!({ "target": "crate", "levels": 6 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::BudgetExceeded, Some("/levels")));
}

#[test]
fn deformers_move_vertices() {
    let mut tx = edit_fixture();
    let key = tx.doc().resolve("post").unwrap();
    let before = tx.doc().object_mesh(key).unwrap().positions.clone();
    ok(&mut tx, "deform.twist", json!({ "target": "post", "angle": 90 }));
    ok(&mut tx, "deform.taper", json!({ "target": "post", "factor": 0.5 }));
    ok(&mut tx, "deform.bend", json!({ "target": "post", "angle": 30, "toward": "z" }));
    ok(&mut tx, "deform.displace", json!({ "target": "post", "strength": "5mm", "seed": 3 }));
    assert_ne!(tx.doc().object_mesh(key).unwrap().positions, before);
    assert!(topo(&tx, "post").is_watertight());
    let e = err(&mut tx, "deform.bend", json!({ "target": "post", "axis": "x", "toward": "x" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/toward")));
    let e = err(&mut tx, "deform.displace", json!({ "target": "post", "octaves": 9 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/octaves")));
}

#[test]
fn element_transforms_by_ids_with_proportional_editing() {
    let mut tx = edit_fixture();
    let key = tx.doc().resolve("tiles").unwrap();
    let center = {
        let m = tx.doc().object_mesh(key).unwrap();
        m.verts().find(|&v| m.positions[v as usize].length() < 1e-6).unwrap()
    };
    let a = run(
        &mut tx,
        "edit.transform",
        json!({ "target": "tiles", "select": { "verts": { "ids": [center] } }, "translate": [0, 0.5, 0], "proportional": { "radius": 0.8, "falloff": "sphere" } }),
    )
    .unwrap();
    assert!(a.output.summary.contains("+8 by proportional editing"), "{}", a.output.summary);
    let m = tx.doc().object_mesh(key).unwrap();
    assert_eq!(m.positions[center as usize].y, 0.5);
    let e = err(&mut tx, "edit.transform", json!({ "target": "tiles", "select": { "verts": { "ids": [9999] } } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/select/verts/ids/0")));
    let e = err(&mut tx, "edit.transform", json!({ "target": "tiles", "select": { "verts": {} }, "axes": [[1, 0, 0], [2, 0, 0], [0, 0, 1]] }));
    assert_eq!(e.path.as_deref(), Some("/axes"));
    // A knife cut through the crate's front face only: the sides gain the cut points.
    ok(&mut tx, "edit.plane_cut", json!({ "target": "crate", "point": [0, "30cm", 0], "select": { "faces": { "group": "front" } } }));
    assert!(topo(&tx, "crate").is_watertight());
    ok(&mut tx, "edit.shrink_fatten", json!({ "target": "crate", "select": { "faces": { "group": "top" } }, "distance": "1cm" }));
}

#[test]
fn curves_are_created_reshaped_and_copied() {
    let mut tx = curve_fixture();
    let s = ok(&mut tx, "curve.bezier", json!({ "name": "hose", "points": [[0, 0, 0], [1, 1, 0], [2, 0, 0], [3, 1, 0]] }));
    assert!(s.contains("1 open bezier spline, 4 points"), "{s}");
    let curve = |tx: &Tx, name: &str| tx.doc().object_curve(tx.doc().resolve(name).unwrap()).unwrap().clone();
    assert!(curve(&tx, "hose").splines[0].is_bezier());
    // Resampling samples the Bezier first and leaves a polyline.
    ok(&mut tx, "curve.resample", json!({ "target": "hose", "count": 20 }));
    let hose = curve(&tx, "hose");
    assert_eq!((hose.splines[0].points.len(), hose.splines[0].is_bezier()), (20, false));
    // A copy keeps the source and lands next to it with the same transform.
    ok(&mut tx, "scene.set_transform", json!({ "target": "outline", "position": [0, 1, 0] }));
    let s = ok(&mut tx, "curve.fillet", json!({ "target": "outline", "radius": "5cm", "output": "soft" }));
    assert!(s.starts_with("created soft (o:"), "{s}");
    assert!(s.ends_with("(filleted from outline (o:3))"), "{s}");
    assert_eq!(curve(&tx, "outline").splines[0].points.len(), 4);
    assert_eq!(curve(&tx, "soft").splines[0].points.len(), 20);
    let soft = tx.doc().resolve("soft").unwrap();
    assert_eq!(tx.doc().object(soft).unwrap().transform.translation, Vec3::Y);
    // Offsets in place keep the closed spline closed and grow it outward.
    let s = ok(&mut tx, "curve.offset", json!({ "target": "outline", "distance": "-10cm" }));
    assert_eq!(s, "outline (o:3): offset by -0.1 m: 1 closed spline, 4 points, 2 m long, 0.6 x 0 x 0.4 m");

    let e = err(&mut tx, "curve.offset", json!({ "target": "crate", "distance": 0.1 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::InvalidParam, Some("/target")));
    assert!(e.message.contains("not a curve"), "{}", e.message);
    let e = err(&mut tx, "curve.polyline", json!({ "points": [[0, 0, 0], [1, 0, 0]], "closed": true }));
    assert_eq!(e.path.as_deref(), Some("/points"));
    let e = err(&mut tx, "curve.bezier", json!({ "points": [[0, 0, 0], [1, 0, 0]], "handles": [[[0, 0, 0], [0, 0, 0]]] }));
    assert_eq!(e.path.as_deref(), Some("/handles"));
    let e = err(&mut tx, "curve.resample", json!({ "target": "path", "count": 5, "spacing": 0.1 }));
    assert_eq!(e.code, ErrorCode::InvalidParam);
    let e = err(&mut tx, "curve.fillet", json!({ "target": "path", "radius": 0.1, "output": "outline" }));
    assert_eq!(e.path.as_deref(), Some("/output"));
    let e = err(&mut tx, "curve.rectangle", json!({ "size": [1, 0.2], "corner_radius": 0.2 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/corner_radius")));
}

#[test]
fn lathe_presets_points_and_curves() {
    let mut tx = Tx::new(Document::new());
    for preset in ["barrel", "vase", "bottle", "bowl", "column"] {
        ok(&mut tx, "gen.lathe", json!({ "name": preset, "profile": preset }));
        let t = topo(&tx, preset);
        assert!(t.is_watertight(), "{preset} is not closed");
    }
    let m = tx.doc().object_mesh(tx.doc().resolve("column").unwrap()).unwrap();
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi.y - lo.y - 3.0).abs() < 1e-5 && (hi.x - 0.25).abs() < 1e-5, "column default size");
    // The barrel workflow: loop cuts on the side, a recessed lid on the top cap.
    ok(&mut tx, "gen.lathe", json!({ "name": "keg", "profile": "barrel", "height": "60cm", "segments": 16 }));
    ok(
        &mut tx,
        "edit.loop_cut",
        json!({ "target": "keg", "select": { "edges": { "in_box": { "min": [0.1, 0.01, 0.1], "max": [0.3, 0.04, 0.3] } } }, "cuts": 2 }),
    );
    ok(&mut tx, "edit.inset", json!({ "target": "keg", "select": { "faces": { "group": "cap_top" } }, "thickness": "2cm", "depth": "-1cm" }));
    assert!(topo(&tx, "keg").is_watertight());
    // A curve object as the profile, revolved around Z.
    ok(&mut tx, "curve.bezier", json!({ "name": "vase_line", "points": [[0.1, 0, 0], [0.15, 0.2, 0], [0.06, 0.35, 0]] }));
    let s = ok(&mut tx, "gen.lathe", json!({ "name": "urn", "profile": { "curve": "vase_line" }, "axis": "z", "segments": 12 }));
    assert!(s.starts_with("created urn"), "{s}");
    let urn = tx.doc().object_mesh(tx.doc().resolve("urn").unwrap()).unwrap();
    let (lo, hi) = urn.bounds().unwrap();
    assert!((hi.z - 0.35).abs() < 1e-5 && lo.z.abs() < 1e-5, "height along Z");

    let e = err(&mut tx, "gen.lathe", json!({ "profile": [[0.1, 0], [0.1, 1]], "height": 2 }));
    assert_eq!(e.path.as_deref(), Some("/height"));
    let e = err(&mut tx, "gen.lathe", json!({ "profile": "teapot" }));
    assert!(e.message.contains("presets are barrel"), "{}", e.message);
    let e = err(&mut tx, "gen.lathe", json!({ "profile": [[0.1, 0], [0.0, 0.5], [0.1, 1]] }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/profile")));
    let e = err(&mut tx, "gen.lathe", json!({ "profile": [[0.1, 0], [0.1, 1]], "segments": 10 }));
    assert_eq!(e.path.as_deref(), Some("/segments"));
    ok(&mut tx, "curve.circle", json!({ "name": "flat_ring", "radius": 0.2 }));
    let e = err(&mut tx, "gen.lathe", json!({ "profile": { "curve": "flat_ring" } }));
    assert!(e.message.contains("XY plane"), "{}", e.message);
}

#[test]
fn shapes_extrude_from_points_and_joined_curves() {
    let mut tx = curve_fixture();
    // A washer plate from an outline curve and a hole curve joined into one.
    ok(&mut tx, "curve.rectangle", json!({ "name": "plate", "size": [0.4, 0.3], "corner_radius": "3cm", "plane": "xy" }));
    ok(&mut tx, "curve.circle", json!({ "name": "bore", "radius": "5cm", "segments": 16, "plane": "xy", "position": [0.05, 0, 0] }));
    let s = ok(&mut tx, "scene.join", json!({ "targets": ["plate", "bore"] }));
    assert!(s.ends_with("2 splines, 36 points"), "{s}");
    ok(&mut tx, "gen.extrude_shape", json!({ "name": "washer", "curve": "plate", "depth": "1cm", "bevel": { "width": "2mm" } }));
    let t = topo(&tx, "washer");
    assert!(t.is_watertight());
    let m = tx.doc().object_mesh(tx.doc().resolve("washer").unwrap()).unwrap();
    let euler = m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64;
    assert_eq!(euler, 0, "one through hole");
    // The joined bore keeps its world place: the hole is off center.
    let (lo, hi) = m.bounds().unwrap();
    assert!((hi.z - lo.z - 0.01).abs() < 1e-5 && (hi.x - lo.x - 0.4).abs() < 1e-5);
    // A floor plan in the ground plane rises along +Y from its curve's height.
    ok(&mut tx, "scene.set_transform", json!({ "target": "outline", "position": [0, 0, 0] }));
    ok(&mut tx, "curve.polyline", json!({ "name": "plan", "points": [[0, 0.5, 0], [2, 0.5, 0], [2, 0.5, -1], [0, 0.5, -1]], "closed": true }));
    ok(&mut tx, "gen.extrude_shape", json!({ "name": "floor", "curve": "plan", "depth": "20cm" }));
    let floor = tx.doc().object_mesh(tx.doc().resolve("floor").unwrap()).unwrap();
    let (lo, hi) = floor.bounds().unwrap();
    assert!((lo.y - 0.5).abs() < 1e-5 && (hi.y - 0.7).abs() < 1e-5, "{lo} {hi}");

    let e = err(&mut tx, "gen.extrude_shape", json!({ "curve": "path" }));
    assert!(e.message.contains("open"), "{}", e.message);
    let e = err(&mut tx, "gen.extrude_shape", json!({ "outline": [[0, 0], [1, 0], [1, 1]], "curve": "outline" }));
    assert_eq!(e.code, ErrorCode::InvalidParam);
    let e = err(&mut tx, "gen.extrude_shape", json!({ "outline": [[0, 0], [1, 1], [1, 0], [0, 1]] }));
    assert_eq!(e.path.as_deref(), Some("/outline"));
    let e = err(&mut tx, "gen.extrude_shape", json!({ "outline": [[0, 0], [0.1, 0], [0.1, 1], [0, 1]], "depth": 1, "bevel": { "width": "6cm" } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/bevel/width")));
    let e = err(&mut tx, "scene.join", json!({ "targets": ["outline", "crate"] }));
    assert!(e.message.contains("curves with curves"), "{}", e.message);
}

#[test]
fn sweeps_and_pipes_follow_paths() {
    let mut tx = curve_fixture();
    // A pipe along a moved curve lands where the curve is.
    ok(&mut tx, "scene.set_transform", json!({ "target": "path", "position": [0, 2, 0] }));
    ok(&mut tx, "gen.pipe", json!({ "name": "hose", "path": { "curve": "path" }, "radius": "3cm", "segments": 12 }));
    assert!(topo(&tx, "hose").is_watertight());
    let hose = tx.doc().resolve("hose").unwrap();
    assert_eq!(tx.doc().object(hose).unwrap().transform.translation, Vec3::new(0.0, 2.0, 0.0));
    // An explicit position wins over the curve's place.
    ok(&mut tx, "gen.pipe", json!({ "name": "hose2", "path": { "curve": "path" }, "position": [5, 0, 0] }));
    let hose2 = tx.doc().resolve("hose2").unwrap();
    assert_eq!(tx.doc().object(hose2).unwrap().transform.translation, Vec3::new(5.0, 0.0, 0.0));
    // A ring: closed path, no caps, torus topology.
    ok(
        &mut tx,
        "gen.pipe",
        json!({ "name": "ring", "path": [[1, 0, 0], [0, 0, -1], [-1, 0, 0], [0, 0, 1]], "path_closed": true, "smooth_path": true }),
    );
    let t = topo(&tx, "ring");
    assert!(t.is_watertight());
    let ring = tx.doc().object_mesh(tx.doc().resolve("ring").unwrap()).unwrap();
    assert_eq!(ring.vert_count() as i64 - ring.edge_count() as i64 + ring.face_count() as i64, 0);
    // A sweep of a curve cross section (a circle drawn on the ground) along a smooth path.
    ok(&mut tx, "curve.circle", json!({ "name": "section", "radius": "5cm", "segments": 10 }));
    ok(
        &mut tx,
        "gen.sweep",
        json!({ "name": "tusk", "profile": { "curve": "section" }, "path": [[0, 0, 0], [0, 0.5, 0.2], [0, 0.8, 0.6]], "smooth_path": true, "scale": [1, 0.2], "twist": 90 }),
    );
    assert!(topo(&tx, "tusk").is_watertight());
    // An open trim profile along a closed rectangle: a picture frame molding with two profile groups.
    let s = ok(
        &mut tx,
        "gen.sweep",
        json!({ "name": "molding", "profile": [[0, 0], [0.04, 0], [0.04, 0.02]], "profile_closed": false, "path": { "curve": "outline" } }),
    );
    assert!(s.contains("molding"), "{s}");
    let m = tx.doc().object_mesh(tx.doc().resolve("molding").unwrap()).unwrap();
    assert_eq!((m.group("profile_0").unwrap().count(), m.group("profile_1").unwrap().count()), (4, 4));

    let e = err(&mut tx, "gen.sweep", json!({ "profile": { "circle": 0.1 }, "path": { "curve": "outline" }, "twist": 45 }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/twist")));
    let e = err(&mut tx, "gen.sweep", json!({ "profile": { "hexagon": 1 }, "path": [[0, 0, 0], [0, 1, 0]] }));
    assert!(e.message.contains("cross section"), "{}", e.message);
    let e = err(&mut tx, "gen.sweep", json!({ "profile": { "circle": 0.1 }, "path": [[0, 0, 0], [0, 1, 0]], "up": [0, 0, 0] }));
    assert_eq!(e.path.as_deref(), Some("/up"));
    let e = err(&mut tx, "gen.pipe", json!({ "path": [[0, 0, 0], [1, 0, 0], [0, 0, 0]] }));
    assert_eq!(e.code, ErrorCode::OutOfRange);
    let e = err(&mut tx, "gen.pipe", json!({ "path": { "curve": "crate" } }));
    assert!(e.message.contains("not a curve"), "{}", e.message);
}

#[test]
fn lofts_between_placed_curves() {
    let mut tx = curve_fixture();
    // A rounded square base blending into a smaller circle 40 cm above it.
    ok(&mut tx, "curve.rectangle", json!({ "name": "base", "size": [0.4, 0.4], "corner_radius": "2cm", "position": [1, 0, 0] }));
    ok(&mut tx, "curve.circle", json!({ "name": "top", "radius": "10cm", "segments": 12, "position": [1, 0.4, 0] }));
    ok(&mut tx, "gen.loft", json!({ "name": "vase", "sections": [{ "curve": "base" }, { "curve": "top" }], "points": 24, "steps": 3 }));
    assert!(topo(&tx, "vase").is_watertight());
    let key = tx.doc().resolve("vase").unwrap();
    assert_eq!(tx.doc().object(key).unwrap().transform.translation, Vec3::new(1.0, 0.0, 0.0), "the base curve's place");
    let (lo, hi) = tx.doc().object_mesh(key).unwrap().bounds().unwrap();
    assert!(lo.y.abs() < 1e-5 && (hi.y - 0.4).abs() < 1e-5, "the top curve keeps its world height: {lo} {hi}");
    let e = err(&mut tx, "gen.loft", json!({ "sections": [{ "curve": "base" }, { "curve": "path" }] }));
    assert!(e.message.contains("all be closed"), "{}", e.message);
    let e = err(&mut tx, "gen.loft", json!({ "sections": [{ "curve": "base" }] }));
    assert_eq!(e.path.as_deref(), Some("/sections"));
}

#[test]
fn arrays_and_mirrors_repeat_meshes() {
    let mut tx = curve_fixture();
    // Fence posts along the path curve (moved away from the posts' origin).
    ok(&mut tx, "scene.set_transform", json!({ "target": "path", "position": [0, 0, 2] }));
    ok(&mut tx, "prim.box", json!({ "name": "post", "width": 0.1, "height": 1, "depth": 0.1 }));
    let s = ok(&mut tx, "gen.array", json!({ "target": "post", "mode": "curve", "curve": "path", "count": 5 }));
    assert!(s.contains("5 copies along path (o:2)"), "{s}");
    let m = tx.doc().object_mesh(tx.doc().resolve("post").unwrap()).unwrap();
    let (lo, hi) = m.bounds().unwrap();
    assert!(lo.z > 0.9 && hi.z < 2.1 && hi.x > 1.0, "posts follow the curve in world space: {lo} {hi}");
    // Open tiles side by side weld into one sheet.
    ok(&mut tx, "prim.plane", json!({ "name": "tile", "width": 0.5, "depth": 0.5 }));
    let s = ok(&mut tx, "gen.array", json!({ "target": "tile", "count": 4, "relative_offset": [1, 0, 0] }));
    assert!(s.contains("welded 6 vertices"), "{s}");
    assert_eq!(topo(&tx, "tile").boundary_loops, 1);
    // Half a spindle (both profile ends on the axis) mirrored back into a whole one, with and without bisecting.
    ok(&mut tx, "gen.lathe", json!({ "name": "bowl", "profile": [[0, 0], [0.3, 0.1], [0, 0.3]], "smooth": true, "angle": 180, "segments": 12 }));
    let s = ok(&mut tx, "gen.mirror", json!({ "target": "bowl", "axis": "x" }));
    assert!(!s.contains("welded 0"), "{s}");
    assert!(topo(&tx, "bowl").is_watertight());
    ok(&mut tx, "prim.quad_sphere", json!({ "name": "ball" }));
    ok(&mut tx, "gen.mirror", json!({ "target": "ball", "axis": "z", "bisect": true, "keep_positive": false }));
    assert!(topo(&tx, "ball").is_watertight());

    let e = err(&mut tx, "gen.array", json!({ "target": "post", "mode": "curve" }));
    assert_eq!(e.path.as_deref(), Some("/curve"));
    let e = err(&mut tx, "gen.array", json!({ "target": "post", "offset": [1, 0, 0], "relative_offset": [1, 0, 0] }));
    assert_eq!(e.path.as_deref(), Some("/relative_offset"));
    let e = err(&mut tx, "gen.array", json!({ "target": "path" }));
    assert_eq!(e.path.as_deref(), Some("/target"));
    let e = err(&mut tx, "gen.array", json!({ "target": "post", "count": 20000 }));
    assert_eq!(e.code, ErrorCode::OutOfRange);
}

#[test]
fn scatter_places_seeded_instances_on_surfaces() {
    let layout = |seed: u64| {
        let mut tx = curve_fixture();
        ok(&mut tx, "prim.plane", json!({ "name": "ground", "width": 4, "depth": 4, "advanced": { "segments": [4, 4] } }));
        ok(&mut tx, "prim.ico_sphere", json!({ "name": "rock", "radius": 0.1, "subdivisions": 1 }));
        let s = ok(
            &mut tx,
            "gen.scatter",
            json!({ "name": "rocks", "target": "ground", "source": "rock", "count": 30, "seed": seed, "scale": [0.5, 1.5] }),
        );
        assert!(s.starts_with("scattered 30 instances of rock"), "{s}");
        let doc = tx.doc();
        let holder = doc.resolve("ground/rocks").unwrap();
        let kids = doc.object(holder).unwrap().children().to_vec();
        assert_eq!(kids.len(), 30);
        let rock_mesh = doc.object(doc.resolve("rock").unwrap()).unwrap().mesh_key();
        kids.iter()
            .map(|&k| {
                let o = doc.object(k).unwrap();
                assert_eq!(o.mesh_key(), rock_mesh, "instances share the source mesh");
                assert!(o.transform.translation.y.abs() < 1e-6, "on the ground");
                o.transform
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(layout(4), layout(4), "same seed, same layout");
    assert_ne!(layout(4), layout(5));

    let mut tx = curve_fixture();
    let e = err(&mut tx, "gen.scatter", json!({ "target": "crate", "source": "path" }));
    assert_eq!(e.path.as_deref(), Some("/source"));
    let e = err(&mut tx, "gen.scatter", json!({ "target": "crate", "source": "crate", "scale": [2, 1] }));
    assert_eq!(e.path.as_deref(), Some("/scale"));
    let e = err(&mut tx, "gen.scatter", json!({ "target": "crate", "source": "crate", "select": { "faces": { "group": "nope" } } }));
    assert!(e.path.as_deref().is_some_and(|p| p.starts_with("/select")), "{e}");
}

#[test]
fn cleanup_reports_then_fixes_once() {
    let mut tx = edit_fixture();
    let before = tx.doc().object_mesh(tx.doc().resolve("panels").unwrap()).unwrap().clone();
    let s = ok(&mut tx, "cleanup.report", json!({ "target": "panels" }));
    assert!(s.contains("would weld 2 vertices"), "{s}");
    assert_eq!(tx.doc().object_mesh(tx.doc().resolve("panels").unwrap()).unwrap(), &before, "the report changes nothing");
    let s = ok(&mut tx, "cleanup.auto", json!({ "target": "panels" }));
    assert!(s.contains("welded 2 vertices"), "{s}");
    assert_eq!(topo(&tx, "panels").parts, 1);
    let s = ok(&mut tx, "cleanup.auto", json!({ "target": "panels" }));
    assert!(s.contains("already clean"), "{s}");
    // The duct is two open tubes stacked with a gap: their 8-edge borders stay open.
    let s = ok(&mut tx, "cleanup.auto", json!({ "target": "duct" }));
    assert!(s.contains("already clean"), "{s}");
    // Nothing is filled when holes are switched off.
    let s = ok(&mut tx, "cleanup.auto", json!({ "target": "bin", "max_hole_edges": 0 }));
    assert!(s.contains("already clean"), "{s}");
    let e = err(&mut tx, "cleanup.auto", json!({ "target": "bin", "weld": "1m" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::OutOfRange, Some("/weld")));
}

#[test]
fn expectations_are_added_checked_and_removed() {
    let mut tx = fixture();
    let s = ok(&mut tx, "expect.add", json!({ "expect": { "budget": {} } }));
    // crate 12 + lid 12 + post 64 (16 side and 48 quad-grid cap triangles).
    assert_eq!(s, "added E1 budget(scene): passes now (88 tris, want at most 6,000)");
    let s = ok(&mut tx, "expect.add", json!({ "expect": { "dims": { "target": "lid", "width": { "min": "0.7m" } } } }));
    assert!(s.starts_with("added E2 dims(lid): fails now"), "{s}");
    let s = ok(&mut tx, "expect.add", json!({ "name": "mirror", "expect": { "symmetric": { "a": "crate" } } }));
    assert!(s.contains("passes now"), "{s}");
    assert_eq!(tx.doc().expectations().len(), 3);

    let e = err(&mut tx, "expect.add", json!({ "expect": { "attached": { "part": "lidd", "to": "crate" } } }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::NotFound, Some("/expect/attached/part")));
    assert_eq!(e.suggestions.first().map(String::as_str), Some("lid"));
    let e = err(&mut tx, "expect.add", json!({ "name": "mirror", "expect": { "watertight": { "target": "crate" } } }));
    assert_eq!((e.code, e.suggestions.clone()), (ErrorCode::InvalidParam, vec!["E3".to_owned()]));
    let e = err(&mut tx, "expect.add", json!({ "expect": { "dims": { "target": "lid" } } }));
    assert_eq!(e.path.as_deref(), Some("/expect/dims"));
    let e = err(&mut tx, "expect.add", json!({ "expect": { "not_intersecting": { "parts": ["lid"] } } }));
    assert_eq!(e.code, ErrorCode::OutOfRange);
    let e = err(&mut tx, "expect.add", json!({ "expect": { "watertigth": { "target": "crate" } } }));
    assert_eq!(e.suggestions.first().map(String::as_str), Some("watertight"));

    assert_eq!(ok(&mut tx, "expect.remove", json!({ "name": "E2" })), "removed E2 dims(lid); 2 left");
    let e = err(&mut tx, "expect.remove", json!({ "name": "E9" }));
    assert_eq!((e.code, e.path.as_deref()), (ErrorCode::NotFound, Some("/name")));
}

#[test]
fn edit_boolean_cuts_adds_and_intersects() {
    let mut tx = edit_fixture();
    // The drill is deleted by default and its hole lined with 8 faces.
    let s = ok(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": "drill" }));
    assert!(s.contains("deleted drill"), "{s}");
    assert!(tx.resolve("drill", "/t").is_err());
    let m = tx.doc().object_mesh(tx.resolve("crate", "/target").unwrap()).unwrap();
    assert_eq!(m.group("boolean_1.cut").unwrap().count(), 8);
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
    // Groups of the target survive: the top face is still "top", with a hole.
    assert!(m.group("top").unwrap().count() >= 2);

    // keep_tool, several tools and a union.
    ok(&mut tx, "prim.box", json!({ "name": "knob", "width": 0.2, "height": 0.2, "depth": 0.2, "position": [0, 0.5, 0] }));
    ok(&mut tx, "prim.box", json!({ "name": "knob2", "width": 0.2, "height": 0.2, "depth": 0.2, "position": [0.25, 0.5, 0] }));
    let s = ok(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": ["knob", "knob2"], "operation": "union", "keep_tool": true }));
    assert!(s.contains("union with knob (o:"), "{s}");
    assert!(!s.contains("deleted"), "{s}");
    assert!(tx.resolve("knob2", "/t").is_ok());

    // Intersection of disjoint objects is refused, nothing changes.
    let e = err(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": "post", "operation": "intersect" }));
    assert_eq!(e.code, ErrorCode::Topology);
    assert!(e.message.contains("do not overlap"), "{}", e.message);
    // A difference that misses changes nothing and keeps the tool.
    let s = ok(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": "post" }));
    assert!(s.contains("misses it; nothing changed"), "{s}");
    assert!(tx.resolve("post", "/t").is_ok());
}

#[test]
fn edit_boolean_errors_are_precise() {
    let mut tx = edit_fixture();
    let e = err(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": "crate" }));
    assert_eq!(e.path.as_deref(), Some("/tool"));
    let e = err(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": ["drill", "drill"] }));
    assert_eq!(e.path.as_deref(), Some("/tool/1"));
    let e = err(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": [] }));
    assert_eq!(e.path.as_deref(), Some("/tool"));
    // The open bin has no inside.
    let e = err(&mut tx, "edit.boolean", json!({ "target": "bin", "tool": "drill" }));
    assert_eq!(e.code, ErrorCode::NonManifoldInput);
    assert_eq!(e.path.as_deref(), Some("/target"));
    assert!(e.message.contains("4 open edges"), "{}", e.message);
    let e = err(&mut tx, "edit.boolean", json!({ "target": "crate", "tool": "hinge" }));
    assert_eq!(e.path.as_deref(), Some("/tool"));
}

fn import(tx: &mut Tx, files: Option<&p3d_ops::FileAccess>, args: Value) -> Result<Applied, OpError> {
    let r = Registry::builtin();
    let applied = r.get("io.import")?.run(tx, &args, &OpCtx { files, ..OpCtx::default() })?;
    tx.doc().check().map_err(OpError::internal)?;
    Ok(applied)
}

#[test]
fn io_import_reads_inside_the_allowed_folder() {
    let (dir, files) = io_folder();
    let mut tx = fixture();
    let a = import(&mut tx, Some(&files), json!({ "path": "props/crate.obj", "parent": "post" })).unwrap();
    // A second import of the same file gets a free name.
    let b = import(&mut tx, Some(&files), json!({ "path": dir.join("props/crate.obj").display().to_string(), "name": "spare" })).unwrap();
    assert!(a.output.summary.contains("into post/crate (o:5)"), "{}", a.output.summary);
    assert!(b.output.summary.contains("into spare (o:6)"), "{}", b.output.summary);
    let k = tx.resolve("post/crate", "/t").unwrap();
    let m = tx.doc().object_mesh(k).unwrap();
    assert_eq!((m.vert_count(), m.face_count()), (8, 6));
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
    let o = tx.doc().object(k).unwrap();
    assert_eq!(o.metadata["materials"], json!(["wood", "metal"]));
    assert_eq!(o.metadata["source"], json!("crate.obj"));
    // An open mesh comes in with a cleanup.auto suggestion.
    let c = import(&mut tx, Some(&files), json!({ "path": "props/open.obj" })).unwrap();
    let w = &c.output.warnings[0];
    assert!(w.message.contains("cleanup.auto would fill 1 hole"), "{}", w.message);
    assert_eq!(w.fix.as_ref().unwrap().op, "cleanup.auto");
}

#[test]
fn io_import_refuses_paths_outside_and_missing_files() {
    let (_dir, files) = io_folder();
    let mut tx = fixture();
    let before = tx.doc().object_count();
    let e = import(&mut tx, Some(&files), json!({ "path": "../outside.obj" })).unwrap_err();
    assert_eq!(e.code, ErrorCode::PathNotAllowed);
    assert_eq!(e.path.as_deref(), Some("/path"));
    let e = import(&mut tx, None, json!({ "path": "props/crate.obj" })).unwrap_err();
    assert_eq!(e.code, ErrorCode::PathNotAllowed);
    let e = import(&mut tx, Some(&files), json!({ "path": "props/missing.obj" })).unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = import(&mut tx, Some(&files), json!({ "path": "props/crate.blend" })).unwrap_err();
    assert_eq!(e.code, ErrorCode::Unsupported);
    let e = import(&mut tx, Some(&files), json!({ "path": "props/crate.obj", "name": "a/b" })).unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/name"));
    assert_eq!(tx.doc().object_count(), before);
}

#[test]
fn sdf_mesh_blends_named_shapes_and_mirrors() {
    let mut tx = Tx::new(Document::new());
    let s = ok(
        &mut tx,
        "sdf.mesh",
        json!({ "name": "head", "detail": "6mm", "shapes": [
            { "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" },
            { "ellipsoid": { "at": [0, 0.06, 0.035], "radii": [0.055, 0.045, 0.055] }, "name": "jaw", "smooth_union": "2.5cm" },
            { "capsule": { "a": [0.07, 0.13, -0.01], "b": [0.11, 0.2, -0.04], "radius": "1.2cm", "radius_b": "2mm" }, "name": "ear", "mirror": "x", "smooth_union": "1cm" },
            { "sphere": { "at": [0.03, 0.14, 0.085], "radius": "1.4cm" }, "name": "eye_socket", "mirror": "x", "smooth_subtract": "6mm" }
        ] }),
    );
    assert!(s.starts_with("created head (o:1) sdf"), "{s}");
    let m = tx.doc().object_mesh(tx.resolve("head", "/t").unwrap()).unwrap();
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
    for g in ["cranium", "jaw", "ear", "eye_socket"] {
        assert!(m.group(g).is_some_and(|g| g.count() > 0), "{g}");
    }
    // Both ears: faces of the ear group on either side of x = 0.
    let ear = m.group("ear").unwrap();
    let side = |f: u32| m.face_verts(f).map(|v| m.positions[v as usize].x).sum::<f32>();
    assert!(ear.iter().any(|f| side(f) > 0.0) && ear.iter().any(|f| side(f) < 0.0));
    assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only");
}

#[test]
fn sdf_mesh_rebuilds_in_place_and_hides_shapes() {
    let mut tx = Tx::new(Document::new());
    let sphere = |x: f64, r: f64| json!({ "sphere": { "at": [x, 0, 0], "radius": r } });
    ok(&mut tx, "sdf.mesh", json!({ "name": "blob", "detail": "1cm", "shapes": [sphere(0.0, 0.1), sphere(0.12, 0.06)] }));
    let key = tx.resolve("blob", "/t").unwrap();
    let objects = tx.doc().object_count();
    // Hide the first (unnamed) shape: the second keeps its group name shape_1.
    let mut hidden = sphere(0.0, 0.1);
    hidden["hidden"] = json!(true);
    let s = ok(&mut tx, "sdf.mesh", json!({ "target": "blob", "detail": "1cm", "shapes": [hidden.clone(), sphere(0.12, 0.06)] }));
    assert!(s.starts_with("rebuilt blob (o:1) sdf"), "{s}");
    assert_eq!((tx.doc().object_count(), tx.resolve("blob", "/t").unwrap()), (objects, key), "the same object, rebuilt");
    let m = tx.doc().object_mesh(key).unwrap();
    assert!(m.group("shape_0").is_none() && m.group("shape_1").is_some_and(|g| g.count() > 0));
    let (lo, hi) = m.bounds().unwrap();
    assert!(lo.x > 0.0 && hi.x < 0.19, "only the second sphere is left: {lo} {hi}");
    // Errors name the shape as written, hidden ones counted.
    let mut bad = sphere(0.0, 0.1);
    bad["sphere"]["radius"] = json!(-0.1);
    let e = err(&mut tx, "sdf.mesh", json!({ "target": "blob", "shapes": [hidden.clone(), bad] }));
    assert_eq!(e.path.as_deref(), Some("/shapes/1"));
    let e = err(&mut tx, "sdf.mesh", json!({ "target": "blob", "shapes": [hidden] }));
    assert!(e.message.contains("hidden"), "{}", e.message);
    let e = err(&mut tx, "sdf.mesh", json!({ "target": "blob", "name": "other", "shapes": [sphere(0.0, 0.1)] }));
    assert_eq!(e.path.as_deref(), Some("/target"));
}

#[test]
fn sdf_mesh_errors_are_precise() {
    let mut tx = Tx::new(Document::new());
    let e = err(&mut tx, "sdf.mesh", json!({ "shapes": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 }, "smooth_unoin": 0.02 }] }));
    assert!(e.message.contains("smooth_unoin"), "{}", e.message);
    let e = err(
        &mut tx,
        "sdf.mesh",
        json!({ "shapes": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 }, "box": { "at": [0, 0, 0], "size": [1, 1, 1] } }] }),
    );
    assert!(e.message.contains("one primitive"), "{}", e.message);
    let e = err(&mut tx, "sdf.mesh", json!({ "shapes": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 }, "subtract": true, "intersect": true }] }));
    assert_eq!(e.path.as_deref(), Some("/shapes/0"));
    let e = err(&mut tx, "sdf.mesh", json!({ "shapes": [{ "sphere": { "at": [0, 0, 0], "radius": -0.1 } }] }));
    assert_eq!(e.path.as_deref(), Some("/shapes/0"));
    let e = err(
        &mut tx,
        "sdf.mesh",
        json!({ "shapes": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 } }, { "sphere": { "at": [0, 0, 0], "radius": 0.2 }, "subtract": true }] }),
    );
    assert_eq!(e.code, ErrorCode::Topology);
    let e = err(&mut tx, "sdf.mesh", json!({ "shapes": [] }));
    assert_eq!(e.path.as_deref(), Some("/shapes"));
}

#[test]
fn remesh_quad_turns_a_torus_into_even_quads() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.torus", json!({ "name": "ring", "major_radius": 0.3, "minor_radius": 0.1 }));
    ok(
        &mut tx,
        "select.save",
        json!({ "target": "ring", "select": { "faces": { "normal": { "dir": [0, 1, 0], "within_deg": 30 } } }, "name": "top" }),
    );
    let s = ok(&mut tx, "remesh.quad", json!({ "target": "ring", "edge": "3cm" }));
    assert!(s.starts_with("ring (o:1): remeshed into"), "{s}");
    let m = tx.doc().object_mesh(tx.resolve("ring", "/t").unwrap()).unwrap();
    assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only");
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed");
    // The face group follows the surface it covered.
    let top = m.group("top").expect("the face group carries over");
    let up = top.iter().filter(|&f| m.face_normal(f).y > 0.5).count();
    assert!(up * 10 > top.count() * 9, "{up} of {}", top.count());
}

#[test]
fn remesh_defaults_are_valid_and_runs_report_progress() {
    let r = Registry::builtin();
    let quad = r.get("remesh.quad").unwrap().default_params();
    assert_eq!((quad["iterations"].clone(), quad["relax"].clone()), (json!(12), json!(4)), "the documented defaults");
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.torus", json!({ "name": "ring", "major_radius": 0.3, "minor_radius": 0.1 }));
    for (op, args) in [("remesh.quad", json!({ "target": "ring", "edge": "4cm" })), ("sdf.from_mesh", json!({ "target": "ring", "detail": "3cm" }))] {
        let progress = std::sync::atomic::AtomicU32::new(0);
        r.get(op).unwrap().run(&mut tx, &args, &OpCtx { progress: Some(&progress), ..OpCtx::default() }).unwrap();
        assert!(progress.load(std::sync::atomic::Ordering::Relaxed) >= 900_000, "{op} reports how far it got");
    }
}

#[test]
fn remesh_quad_refuses_open_meshes() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.plane", json!({ "name": "floor" }));
    let e = err(&mut tx, "remesh.quad", json!({ "target": "floor" }));
    assert_eq!(e.code, ErrorCode::NonManifoldInput);
    assert!(e.message.contains("open edges"), "{}", e.message);
    let e = err(&mut tx, "remesh.quad", json!({ "target": "floor", "iterations": 0 }));
    assert_eq!(e.path.as_deref(), Some("/iterations"));
}

#[test]
fn sdf_from_mesh_fuses_overlapping_parts() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "prim.box", json!({ "name": "block", "width": 0.4, "height": 0.4, "depth": 0.4 }));
    ok(&mut tx, "prim.box", json!({ "name": "lid", "width": 0.4, "height": 0.4, "depth": 0.4, "position": [0.2, 0.1, 0] }));
    ok(&mut tx, "scene.join", json!({ "targets": ["block", "lid"] }));
    let s = ok(&mut tx, "sdf.from_mesh", json!({ "target": "block", "detail": "2cm" }));
    assert!(s.contains("voxel remeshed at 20 mm"), "{s}");
    let m = tx.doc().object_mesh(tx.resolve("block", "/t").unwrap()).unwrap();
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed");
    assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only");
}
