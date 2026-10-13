//! Every fix diagnostics suggest for the broken-model corpus resolves its
//! finding, as predicted, and applies as one undoable transaction.

use p3d_engine::Session;
use p3d_engine::predict::{apply_fix, diagnose_predicted, explain};
use p3d_ops::Registry;
use p3d_testdata::scenes;

#[test]
fn top_fixes_resolve_their_findings() {
    let registry = Registry::builtin();
    let mut checked = 0;
    for (name, doc) in scenes::broken_corpus() {
        let findings = diagnose_predicted(&registry, &doc, None, 0).unwrap();
        for f in findings.iter().filter(|f| !f.fixes.is_empty()) {
            // Info findings are predicted on request.
            let f = if f.fixes[0].predicted.as_deref().is_some_and(|p| p.starts_with("not predicted")) {
                explain(&registry, &doc, None, 0, &f.id).unwrap()
            } else {
                f.clone()
            };
            let p = f.fixes[0].predicted.as_deref().unwrap();
            assert!(p.starts_with("resolved"), "{name} {} {}: {p}", f.id, f.code);
            checked += 1;
        }
    }
    assert!(checked >= 7, "{checked} fixes checked");
}

#[test]
fn applying_a_fix_is_undoable() {
    let (_, doc) = scenes::broken_corpus().into_iter().find(|(n, _)| *n == "floating_part").unwrap();
    let mut s = Session::new();
    s.load(doc);
    let (f, r) = apply_fix(&mut s, None, "F1", 0).unwrap();
    assert_eq!(f.code, "FLOATING_PART");
    assert_eq!(r.revision, 1);
    assert_eq!(s.history()[0].label, "fix F1 FLOATING_PART lid");
    let after = diagnose_predicted(s.registry(), s.doc(), None, 0).unwrap();
    assert!(after.is_empty(), "{after:#?}");
    s.undo().unwrap();
    assert_eq!(diagnose_predicted(s.registry(), s.doc(), None, 0).unwrap().len(), 1);

    let e = apply_fix(&mut s, None, "F9", 0).unwrap_err();
    assert_eq!(e.code, p3d_ops::ErrorCode::NotFound);
    let e = apply_fix(&mut s, None, "F1", 3).unwrap_err();
    assert_eq!(e.code, p3d_ops::ErrorCode::OutOfRange);
    let x = explain(s.registry(), s.doc(), None, 0, "f1").unwrap();
    assert!(x.fixes[0].predicted.as_deref().is_some_and(|p| p.starts_with("resolved")));
}
