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
