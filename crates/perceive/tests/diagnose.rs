//! The broken-model corpus: each scene yields exactly its expected
//! findings, at the expected place, with the expected fix.

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic)]

use p3d_perceive::diagnose::{Severity, diagnose, report};
use p3d_perceive::{TOKEN_BUDGET, View};
use p3d_testdata::scenes;

/// (scene, [(code, severity, part, side, fix op)])
fn expected(name: &str) -> Vec<(&'static str, Severity, &'static str, &'static str, Option<&'static str>)> {
    use Severity::*;
    match name {
        "open_boundary" => vec![("OPEN_BOUNDARY", Warn, "box", "top", Some("edit.fill"))],
        "inverted_normals" => vec![("INVERTED_NORMALS", Error, "box", "center", Some("edit.recalc_normals"))],
        "degenerate_faces" => vec![("DEGENERATE_FACES", Warn, "panel", "right", None)],
        "duplicate_faces" => vec![("DUPLICATE_FACES", Warn, "box", "right", None), ("LOOSE_PIECES", Info, "box", "center", Some("scene.separate"))],
        "self_intersection" => vec![("SELF_INTERSECTION", Warn, "pair", "center", Some("scene.separate"))],
        "thin_wall" => vec![("THIN_WALL", Warn, "shell", "bottom", Some("edit.shrink_fatten"))],
        "floating_part" => vec![("FLOATING_PART", Warn, "lid", "bottom", Some("scene.set_transform"))],
        "penetration" => vec![("PENETRATION", Warn, "handle", "left", Some("scene.set_transform"))],
        "z_fighting" => vec![("Z_FIGHTING", Warn, "label", "center", Some("scene.set_transform"))],
        other => panic!("no expectation for {other}"),
    }
}

#[test]
fn corpus_yields_exactly_the_expected_findings() {
    let mut failures = Vec::new();
    for (name, doc) in scenes::broken_corpus() {
        let view = View::new(&doc, None).unwrap();
        let got: Vec<_> =
            diagnose(&view).into_iter().map(|f| (f.code, f.severity, f.part.clone(), f.side, f.fixes.first().map(|x| x.op.clone()))).collect();
        let want: Vec<_> = expected(name).into_iter().map(|(c, s, p, side, op)| (c, s, p.to_owned(), side, op.map(str::to_owned))).collect();
        if got != want {
            failures.push(format!("{name}:\n    got  {got:?}\n    want {want:?}"));
        }
        let r = report(&view);
        assert!(r.tokens() <= TOKEN_BUDGET, "{name}: {} tokens", r.tokens());
        insta::assert_snapshot!(format!("diagnose_{name}"), r.text);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn clean_scenes_have_no_warnings() {
    let doc = scenes::crate_scene();
    let f = diagnose(&View::new(&doc, None).unwrap());
    assert!(f.iter().all(|f| f.severity == Severity::Info), "{f:#?}");
}

#[test]
fn findings_carry_the_last_change_as_cause() {
    let mut doc = scenes::crate_scene();
    let key = doc.resolve("crate").unwrap();
    doc.push_history(
        key,
        p3d_scene::OpNode { op: "edit.delete_elements".into(), version: 1, params: serde_json::json!({}), label: "cut_top".into() },
    )
    .unwrap();
    // Open the crate's top so a finding exists.
    let mk = doc.object(key).unwrap().mesh_key().unwrap();
    let m = doc.mesh_mut(mk).unwrap();
    let top: Vec<u32> = m.group("top").unwrap().iter().collect();
    m.delete_faces(&top).unwrap();
    let f = diagnose(&View::new(&doc, Some("crate")).unwrap());
    assert_eq!(f[0].cause.as_deref(), Some("last changed by edit.delete_elements (cut_top)"));
}
