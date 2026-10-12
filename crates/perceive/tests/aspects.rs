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
