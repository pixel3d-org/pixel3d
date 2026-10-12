//! Perception reports on the test scenes: text snapshots, JSON snapshots
//! and token budgets.

use p3d_perceive::{Report, TOKEN_BUDGET, View, relations, structure};
use p3d_testdata::scenes;

fn check(name: &str, r: &Report) {
    assert!(r.tokens() <= TOKEN_BUDGET, "{name}: {} tokens\n{}", r.tokens(), r.text);
    assert!(!r.text.contains('\u{2014}') && !r.text.contains('\u{2013}'), "{name}: dashes");
    insta::assert_snapshot!(format!("{name}_text"), r.text);
    insta::assert_json_snapshot!(format!("{name}_json"), r.data);
}

#[test]
fn structure_reports() {
    let doc = scenes::crate_scene();
    check("structure_crate", &structure::report(&View::new(&doc, None).unwrap()));
    check("structure_crate_lid", &structure::report(&View::new(&doc, Some("lid")).unwrap()));
    let doc = scenes::head_scene();
    check("structure_head", &structure::report(&View::new(&doc, None).unwrap()));
}

#[test]
fn relations_reports() {
    let doc = scenes::crate_scene();
    check("relations_crate", &relations::report(&View::new(&doc, None).unwrap()));
    let doc = scenes::head_scene();
    check("relations_head", &relations::report(&View::new(&doc, None).unwrap()));
}

#[test]
fn unknown_targets_get_suggestions() {
    let doc = scenes::crate_scene();
    let e = View::new(&doc, Some("crat")).err().unwrap();
    assert_eq!((e.code, e.path.as_deref()), ("NOT_FOUND", Some("/target")));
    assert_eq!(e.suggestions.first().map(String::as_str), Some("crate"));
}

#[test]
fn shape_reports() {
    let doc = scenes::crate_scene();
    check("shape_crate", &p3d_perceive::shape::report(&View::new(&doc, None).unwrap()));
    let doc = scenes::head_scene();
    check("shape_head", &p3d_perceive::shape::report(&View::new(&doc, None).unwrap()));
}

#[test]
fn shape_families() {
    let doc = scenes::shapes_scene();
    let view = View::new(&doc, None).unwrap();
    let r = p3d_perceive::shape::report(&view);
    check("shape_families", &r);
    let best = |i: usize| (r.data[i]["fits"][0]["kind"].as_str().unwrap().to_owned(), r.data[i]["fits"][0]["axis"].as_str().map(str::to_owned));
    assert_eq!(best(0), ("cylinder".into(), Some("y".into())));
    assert_eq!(best(1).0, "box");
    assert_eq!(r.data[1]["world_aligned"], false, "the plank is turned 30 degrees");
    assert_eq!(best(2).0, "plane");
    assert_eq!(r.data[3]["holes"], 1);
    assert_eq!(r.data[3]["closed"], false);
}

#[test]
fn silhouette_reports() {
    use p3d_perceive::silhouette::{SilhouetteOptions, ViewDir, report};
    let doc = scenes::crate_scene();
    let view = View::new(&doc, None).unwrap();
    for dir in [ViewDir::Front, ViewDir::Right, ViewDir::Top] {
        let r = report(&view, &SilhouetteOptions { view: dir, ..SilhouetteOptions::default() }).unwrap();
        check(&format!("silhouette_crate_{}", dir.name()), &r);
    }
    let doc = scenes::head_scene();
    let r = report(&View::new(&doc, None).unwrap(), &SilhouetteOptions::default()).unwrap();
    check("silhouette_head_front", &r);
    let e = report(&view, &SilhouetteOptions { points: 2, ..SilhouetteOptions::default() }).unwrap_err();
    assert_eq!((e.code, e.path.as_deref()), ("OUT_OF_RANGE", Some("/points")));
}

#[test]
fn section_reports() {
    use p3d_perceive::section::{SectionOptions, SliceAxis, report};
    let doc = scenes::crate_scene();
    let view = View::new(&doc, None).unwrap();
    check("section_crate_y420", &report(&view, &SectionOptions { at: Some(0.42), ..SectionOptions::default() }).unwrap());
    check("section_crate_x", &report(&view, &SectionOptions { axis: SliceAxis::X, ..SectionOptions::default() }).unwrap());
    let doc = scenes::shapes_scene();
    let view = View::new(&doc, None).unwrap();
    let r = report(&view, &SectionOptions { at: Some(0.1), ..SectionOptions::default() }).unwrap();
    check("section_shapes_y100", &r);
    assert_eq!(r.data["loops"][1]["part"], "post");
    assert_eq!(r.data["loops"][1]["kind"], "outer");
    // A vertical slice through the open bin is an open U.
    let r = report(&View::new(&doc, Some("bin")).unwrap(), &SectionOptions { axis: SliceAxis::X, ..SectionOptions::default() }).unwrap();
    check("section_bin_x", &r);
    assert_eq!(r.data["loops"][0]["kind"], "open");
    let doc = scenes::head_scene();
    check("section_head_z", &report(&View::new(&doc, None).unwrap(), &SectionOptions { axis: SliceAxis::Z, ..SectionOptions::default() }).unwrap());
}

#[test]
fn section_holes_and_walls() {
    use p3d_perceive::section::{SectionOptions, report};
    let mut doc = p3d_scene::Document::new();
    scenes::add(&mut doc, "tank", None, scenes::hollow_box_mesh([0.5, 0.5, 0.5], 0.02), [0.0; 3]);
    let r = report(&View::new(&doc, None).unwrap(), &SectionOptions::default()).unwrap();
    check("section_tank", &r);
    assert_eq!(r.data["loops"][0]["kind"], "outer");
    assert_eq!(r.data["loops"][1]["kind"], "hole");
    let wall = r.data["loops"][1]["wall"].as_f64().unwrap();
    assert!((wall - 0.02).abs() < 1e-4, "{wall}");
}

#[test]
fn probe_reports() {
    use p3d_perceive::probe::{RayFrom, SurfacePoint, report};
    let doc = scenes::crate_scene();
    let view = View::new(&doc, None).unwrap();
    let at = |on: &str| SurfacePoint { on: on.into(), ..SurfacePoint::default() };
    let r = report(&view, &SurfacePoint { landmark: Some("top".into()), ..at("crate") }).unwrap();
    check("probe_crate_top", &r);
    assert_eq!(r.data["nearest_part"][0], "lid");
    let r = report(&view, &SurfacePoint { landmark: Some("left".into()), ..at("crate") }).unwrap();
    check("probe_crate_left", &r);
    let r = report(&view, &SurfacePoint { ray: Some(RayFrom { from: "+z".into(), at: Some([0.29, 0.3]) }), ..at("crate") }).unwrap();
    check("probe_crate_bevel", &r);
    let r = report(&view, &SurfacePoint { group: Some("front".into()), ..at("handle_r") }).unwrap();
    check("probe_handle_group", &r);
    let doc = scenes::head_scene();
    let view = View::new(&doc, None).unwrap();
    let r = report(&view, &SurfacePoint { nearest: Some([0.0, 0.3, 0.2]), ..at("head") }).unwrap();
    check("probe_head_nearest", &r);

    let e = report(&view, &SurfacePoint { landmark: Some("tpo".into()), ..at("head") }).unwrap_err();
    assert_eq!((e.code, e.suggestions.first().map(String::as_str)), ("NOT_FOUND", Some("top")));
    let e = report(&view, &SurfacePoint { landmark: Some("top".into()), group: Some("x".into()), ..at("head") }).unwrap_err();
    assert_eq!(e.code, "INVALID_PARAM");
    let e = report(&view, &SurfacePoint { ray: Some(RayFrom { from: "up".into(), at: None }), ..at("head") }).unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/at/ray/from"));
}

#[test]
fn diff_reports() {
    let before = scenes::crate_scene();
    let mut after = before.clone();
    after.revision = 1;
    let lid = after.resolve("lid").unwrap();
    after.object_mut(lid).unwrap().transform.translation.y += 0.012;
    let handle = after.resolve("handle_r").unwrap();
    after.delete(handle, true).unwrap();
    scenes::add(&mut after, "badge", None, scenes::box_mesh([0.1, 0.1, 0.01], 0.0), [0.0, 0.2, 0.3]);
    let c = after.resolve("crate").unwrap();
    let mk = after.object(c).unwrap().mesh_key().unwrap();
    let m = after.mesh_mut(mk).unwrap();
    let top: Vec<u32> = m.group("top").unwrap().iter().collect();
    m.delete_faces(&top).unwrap();
    let r = p3d_perceive::diff::report(&before, &after, None).unwrap();
    check("diff_crate", &r);
    assert_eq!(r.data["removed"][0]["name"], "handle_r");
    assert_eq!(r.data["added"][0]["name"], "badge");
    // An unchanged scene has no changes.
    let r = p3d_perceive::diff::report(&before, &before.clone(), None).unwrap();
    assert!(r.text.contains("no changes"), "{}", r.text);
}
