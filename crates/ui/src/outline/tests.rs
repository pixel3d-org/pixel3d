use std::time::Duration;

use super::*;
use crate::Services;
use crate::viewport::ViewportEvent;

/// A new app with a starter outline object, selected.
fn with(starter: &str) -> PixelApp {
    let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
    a.run("outline.new", json!({ "starter": starter })).unwrap();
    a
}

/// Lets background runs finish and takes them in.
fn settle(a: &mut PixelApp) {
    let ctx = egui::Context::default();
    for _ in 0..4000 {
        a.spec_tick(&ctx);
        a.outline_tick(&ctx);
        if !a.outline.busy() && !a.spec.busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the outline runs did not finish");
}

fn height(a: &PixelApp, name: &str) -> f32 {
    let doc = a.doc();
    doc.object_mesh(doc.resolve(name).unwrap()).and_then(|m| m.bounds()).map_or(0.0, |(lo, hi)| hi.y - lo.y)
}

#[test]
fn a_shell_opens_the_outline_tab() {
    let a = with("leaf");
    let k = a.doc().resolve("leaf").unwrap();
    assert_eq!((a.outline.object, a.outline.kind, a.ui.inspector_tab), (Some(k), Some(Kind::Shell), inspector::Tab::Outline));
    assert_eq!(a.outline.view, Some(ViewId::Outline));
    assert_eq!(a.outline.layers.len(), 1);
    assert_eq!(a.outline.layers[0].outline.contours[0].len(), 2, "two points with handles");
    assert_eq!(a.outline.fields.get("cup"), Some(&json!(0.3)));
    // Another object closes it.
    let mut a = a;
    a.run("prim.box", json!({ "name": "crate" })).unwrap();
    a.outline_follow();
    assert!(!a.outline.is_open());
}

#[test]
fn moving_a_point_rebuilds_in_place_as_one_step() {
    let mut a = with("leaf");
    let k = a.doc().resolve("leaf").unwrap();
    let (steps, objects, before) = (a.session.history().len(), a.doc().object_count(), height(&a, "leaf"));
    // The tip (0, 0.1) up to 0.15.
    a.run("outline.move_point", json!({ "index": 1, "to": [0, 0.15] })).unwrap();
    assert!(a.outline.busy());
    settle(&mut a);
    assert_eq!((a.session.history().len(), a.doc().object_count()), (steps + 1, objects), "one step, the same object");
    assert_eq!(a.doc().resolve("leaf").unwrap(), k);
    assert!(height(&a, "leaf") > before + 0.03, "{before} -> {}", height(&a, "leaf"));
    // The history holds the new outline: the editor reads it back.
    a.outline_follow();
    assert!(a.outline.layers[0].outline.to_svg().contains("0 0.15"), "{}", a.outline.layers[0].outline.to_svg());
    // Undo puts the old outline back in the editor.
    a.run("edit.undo", Value::Null).unwrap();
    a.outline_follow();
    assert!((height(&a, "leaf") - before).abs() < 1e-5);
    assert!(!a.outline.layers[0].outline.to_svg().contains("0.15"));
}

#[test]
fn points_add_delete_and_turn_smooth() {
    let mut a = with("ear");
    a.run("outline.insert_point", json!({ "segment": 0, "t": 0.5 })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert_eq!(a.outline.layers[0].outline.contours[0].len(), 3);
    assert_eq!(a.outline.selected.map(|s| s.node), Some(1), "the new point is selected");
    a.run("outline.toggle_smooth", json!({ "index": 1 })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert!(!a.outline.layers[0].outline.contours[0][1].smooth, "a split curve point is smooth; toggling made it a corner");
    a.run("outline.toggle_smooth", json!({ "index": 1 })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert!(a.outline.layers[0].outline.contours[0][1].smooth);
    // A contour keeps three points.
    assert!(a.run("outline.delete_point", json!({ "index": 1 })).is_err());
    a.run("outline.set_points", json!({ "points": [[0, 0], [0.04, 0.02], [0.03, 0.08], [0, 0.1]] })).unwrap();
    settle(&mut a);
    a.outline_follow();
    a.run("outline.delete_point", json!({ "index": 2 })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert_eq!(a.outline.layers[0].outline.contours[0].len(), 3);
}

#[test]
fn holes_and_fields_for_shells() {
    let mut a = with("leaf");
    a.run("outline.add", json!({ "layer": "hole" })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert_eq!(a.outline.layers.iter().map(|l| l.key).collect::<Vec<_>>(), [LayerKey::Outline, LayerKey::Hole(0)]);
    let k = a.doc().resolve("leaf").unwrap();
    let args = &a.doc().history(k).unwrap().nodes.last().unwrap().params;
    assert_eq!(args["holes"].as_array().map(Vec::len), Some(1));
    assert_eq!(args["target"], json!("o:1"));
    a.run("outline.set", json!({ "thickness": 0.002, "cup": 0.5 })).unwrap();
    settle(&mut a);
    let m = a.doc().object_mesh(k).unwrap();
    assert!(m.group("rim").is_some(), "a thick shell is closed with a rim");
    a.run("outline.remove", json!({ "layer": "holes/0" })).unwrap();
    settle(&mut a);
    a.outline_follow();
    assert_eq!(a.outline.layers.len(), 1);
    assert!(a.run("outline.set", json!({ "colour": 1 })).is_err());
}

#[test]
fn silhouettes_views_guides_and_blueprint() {
    let mut a = with("axe_head");
    assert_eq!(a.outline.kind, Some(Kind::Silhouettes));
    assert_eq!(a.outline.layers.iter().map(|l| l.key).collect::<Vec<_>>(), [LayerKey::Front, LayerKey::Top]);
    assert_eq!(a.outline.layers[0].outline.contours.len(), 3, "the blade and two holes");
    assert!(a.outline.layers[1].outline.points_form, "the top view stays a point list");
    // The front view's guides: x from the top view, none for y (no side).
    let (gx, gy) = a.outline.guides(ViewId::Front);
    assert!(gx.len() == 2 && (gx[1] - 0.2).abs() < 1e-9 && gy.is_empty(), "{gx:?} {gy:?}");
    a.run("outline.select_view", json!({ "view": "top" })).unwrap();
    assert_eq!(a.outline.view, Some(ViewId::Top));
    assert!(a.run("outline.select_view", json!({ "view": "outline" })).is_err());
    // A side view spans the front's height and the top's depth.
    a.run("outline.add", json!({ "layer": "side" })).unwrap();
    let side = a.outline.view_bounds(ViewId::Side).unwrap();
    assert!((side.1.y - 0.14).abs() < 1e-6 && (side.1.x - 0.011).abs() < 1e-6, "{side:?}");
    settle(&mut a);
    a.outline_follow();
    assert_eq!(a.outline.layers.len(), 3);
    // The blueprint draws on planes around the object.
    assert!(ViewId::Side.to_3d(DVec2::new(0.01, 0.02), -0.3).abs_diff_eq(Vec3::new(-0.3, 0.02, 0.01), 1e-6));
}

#[test]
fn spec_parts_patch_their_kind() {
    let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
    let leaf = json!({ "outline": [[0, 0], [0.04, 0.03], [0, 0.1], [-0.04, 0.03]], "cup": 0.2 });
    a.run("spec.set", json!({ "name": "plant", "spec": { "parts": { "leaf": { "shell": leaf } } } })).unwrap();
    let k = a.doc().resolve("plant/leaf").unwrap();
    a.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
    a.spec_follow();
    a.outline_follow();
    let root = a.doc().resolve("plant").unwrap();
    assert_eq!((a.outline.part.clone(), a.outline.object), (Some((root, "leaf".to_owned())), Some(k)));
    let steps = a.session.history().len();
    a.run("outline.move_point", json!({ "index": 2, "to": [0, 0.2] })).unwrap();
    settle(&mut a);
    assert_eq!(a.session.history().len(), steps + 1);
    let shell = &a.doc().spec(root).unwrap().source["parts"]["leaf"]["shell"];
    assert_eq!(shell["outline"][2], json!([0.0, 0.2]), "{shell}");
    assert_eq!(shell["cup"], json!(0.2));
    assert!(shell.get("holes").is_none(), "absent keys stay absent");
}

#[test]
fn the_inspect_state_names_points_on_the_canvas() {
    let a = with("leaf");
    let s = a.outline_state();
    assert_eq!((s["op"].clone(), s["view"].clone()), (json!("gen.shell"), json!("outline")));
    assert_eq!(s["layers"][0]["layer"], "outline");
    assert!(s["layers"][0]["value"].as_str().is_some_and(|v| v.starts_with("M 0 0 C")));
}
