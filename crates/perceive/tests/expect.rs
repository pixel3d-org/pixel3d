//! Expectation checks pass and fail where they should, with measured values.

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::unwrap_used)]

use p3d_perceive::expect::{check, check_all, report, tests_line};
use p3d_scene::{Check, Document, Expectation, ObjectRef, Range};
use p3d_testdata::scenes;

fn r(doc: &Document, name: &str) -> ObjectRef {
    let k = doc.resolve(name).unwrap();
    ObjectRef { name: name.into(), id: doc.object(k).unwrap().id() }
}

fn e(name: &str, check: Check) -> Expectation {
    Expectation { name: name.into(), check }
}

#[test]
fn every_check_passes_and_fails() {
    let c = scenes::crate_scene();
    let h = scenes::head_scene();
    let s = scenes::shapes_scene();
    let mm = |x: f64| x / 1000.0;
    let range = |a: Option<f64>, b: Option<f64>| Some(Range { min: a.map(mm), max: b.map(mm) });
    let cases: Vec<(&Document, Expectation, bool)> = vec![
        (
            &c,
            e("dims_ok", Check::Dims { target: r(&c, "lid"), width: range(Some(610.0), Some(630.0)), height: None, depth: range(None, Some(700.0)) }),
            true,
        ),
        (&c, e("dims_bad", Check::Dims { target: r(&c, "lid"), width: None, height: range(None, Some(30.0)), depth: None }), false),
        (&c, e("sym_pair", Check::Symmetric { a: r(&c, "handle_l"), b: Some(r(&c, "handle_r")), axis: 'x', at: None, tolerance: mm(1.0) }), true),
        (&c, e("sym_pair_bad", Check::Symmetric { a: r(&c, "handle_l"), b: Some(r(&c, "lid")), axis: 'x', at: None, tolerance: mm(1.0) }), false),
        (&c, e("sym_self", Check::Symmetric { a: r(&c, "crate"), b: None, axis: 'x', at: None, tolerance: mm(1.0) }), true),
        (&s, e("sym_self_bad", Check::Symmetric { a: r(&s, "bin"), b: None, axis: 'y', at: None, tolerance: mm(1.0) }), false),
        (&c, e("attached", Check::Attached { part: r(&c, "lid"), to: r(&c, "crate"), max_gap: mm(1.0), max_penetration: None }), true),
        (&h, e("attached_gap", Check::Attached { part: r(&h, "hat"), to: r(&h, "head"), max_gap: mm(1.0), max_penetration: None }), false),
        (
            &h,
            e("attached_deep", Check::Attached { part: r(&h, "ear_l"), to: r(&h, "head"), max_gap: mm(1.0), max_penetration: Some(mm(10.0)) }),
            false,
        ),
        (&h, e("attached_ear", Check::Attached { part: r(&h, "ear_l"), to: r(&h, "head"), max_gap: mm(1.0), max_penetration: Some(mm(25.0)) }), true),
        (&c, e("apart", Check::NotIntersecting { parts: vec![r(&c, "handle_l"), r(&c, "handle_r"), r(&c, "lid")] }), true),
        (&h, e("apart_bad", Check::NotIntersecting { parts: vec![r(&h, "ear_l"), r(&h, "head")] }), false),
        (&c, e("budget", Check::Budget { target: None, min_tris: None, max_tris: 1000 }), true),
        (&c, e("budget_bad", Check::Budget { target: Some(r(&c, "crate")), min_tris: Some(200), max_tris: 1000 }), false),
        (&c, e("closed", Check::Watertight { target: r(&c, "crate") }), true),
        (&s, e("closed_bad", Check::Watertight { target: r(&s, "bin") }), false),
    ];
    let mut text = String::new();
    for (doc, exp, want) in cases {
        let o = check(doc, &exp);
        assert_eq!(o.pass, want, "{} {}: {}", o.name, o.label, o.message);
        text.push_str(&format!("{} {} {}: {}\n", if o.pass { "pass" } else { "FAIL" }, o.name, o.label, o.message));
    }
    insta::assert_snapshot!("expect_cases", text);
}

#[test]
fn renamed_and_deleted_targets() {
    let mut doc = scenes::crate_scene();
    let lid = r(&doc, "lid");
    let crate_ref = r(&doc, "crate");
    doc.expectations_mut().push(e("E1", Check::Attached { part: lid.clone(), to: crate_ref, max_gap: 0.001, max_penetration: None }));
    let k = doc.resolve("lid").unwrap();
    doc.rename(k, "cover").unwrap();
    assert!(check_all(&doc)[0].pass, "found by id after the rename");
    doc.delete(k, true).unwrap();
    let o = &check_all(&doc)[0];
    assert!(!o.pass);
    assert_eq!(o.message, "lid (o:2) no longer exists");
    assert_eq!(tests_line(&check_all(&doc)), "tests 0/1 pass; failing: E1 attached(lid, crate)");
    insta::assert_snapshot!("expect_report", report(&doc).text);
}
