//! Session behaviour: transactions, undo, checkpoints, rollback, jobs.

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Instant;

use p3d_engine::{BatchOptions, OpCall, Session, UndoLimits};
use p3d_ops::{Applied, DynOp, ErrorCode, OpCtx, OpError, OpMeta, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

fn call(op: &str, args: Value) -> OpCall {
    OpCall::new(op, args)
}

fn exec(s: &mut Session, calls: &[OpCall]) {
    s.execute(calls, &BatchOptions::default()).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn a_batch_is_one_transaction() {
    let mut s = Session::new();
    let r = s
        .execute(
            &[
                call("prim.box", json!({ "name": "crate" })),
                call("select.save", json!({ "target": "crate", "select": { "faces": { "group": "top" } }, "name": "lid" })),
            ],
            &BatchOptions::default(),
        )
        .unwrap();
    assert_eq!((r.revision, r.results.len()), (1, 2));
    assert_eq!(r.results[0].output.metrics.tris, 12);
    assert_eq!(r.results[0].output.metrics.objects, 1);
    assert_eq!(r.results[1].output.metrics.tris, 0);
    assert_eq!(s.history().len(), 1);
    assert_eq!(s.history()[0].label, "box_1, save_2");
    // History graph of the object records both ops.
    let k = s.doc().resolve("crate").unwrap();
    let ops: Vec<&str> = s.doc().history(k).unwrap().nodes.iter().map(|n| n.op.as_str()).collect();
    assert_eq!(ops, vec!["prim.box", "select.save"]);
}

#[test]
fn a_failing_op_rolls_back_the_whole_batch() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    let before = Arc::clone(s.doc());
    let e = s
        .execute(&[call("prim.box", json!({ "name": "b" })), call("prim.box", json!({ "name": "c", "width": -1 }))], &BatchOptions::default())
        .unwrap_err();
    assert_eq!(e.index, 1);
    assert_eq!((e.error.code, e.error.path.as_deref()), (ErrorCode::OutOfRange, Some("/ops/1/args/width")));
    assert!(Arc::ptr_eq(&before, s.doc()), "nothing committed");
    assert!(s.doc().resolve("b").is_err());
    let e = s.execute(&[call("prim.cube", json!({}))], &BatchOptions::default()).unwrap_err();
    assert_eq!((e.error.code, e.error.path.as_deref()), (ErrorCode::UnknownOp, Some("/ops/0/op")));
}

#[test]
fn dry_run_reports_without_committing() {
    let mut s = Session::new();
    let r = s.execute(&[call("prim.box", json!({}))], &BatchOptions { dry_run: true, ..BatchOptions::default() }).unwrap();
    assert!(r.dry_run);
    assert_eq!(r.results[0].output.created.len(), 1);
    assert_eq!((s.doc().object_count(), s.revision(), s.can_undo()), (0, 0, false));
}

#[test]
fn undo_redo_and_redo_invalidation() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    exec(&mut s, &[call("prim.box", json!({ "name": "b" }))]);
    assert_eq!(s.undo().unwrap(), "box_1");
    assert_eq!(s.doc().object_count(), 1);
    assert_eq!(s.redo().unwrap(), "box_1");
    assert_eq!(s.doc().object_count(), 2);
    s.undo().unwrap();
    exec(&mut s, &[call("prim.box", json!({ "name": "c" }))]);
    assert!(!s.can_redo(), "a new transaction clears redo");
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(s.undo().unwrap_err().message, "nothing to undo");
    assert_eq!(s.doc().object_count(), 0);
}

#[test]
fn checkpoints_and_revert_are_undoable() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    s.checkpoint("before_more");
    exec(&mut s, &[call("prim.box", json!({ "name": "b" })), call("prim.box", json!({ "name": "c" }))]);
    s.revert("before_more").unwrap();
    assert_eq!(s.doc().object_count(), 1);
    s.undo().unwrap();
    assert_eq!(s.doc().object_count(), 3, "revert is a transaction");
    let e = s.revert("befor_more").unwrap_err();
    assert_eq!((e.code, e.suggestions.clone()), (ErrorCode::NotFound, vec!["before_more".to_owned()]));
    let r = s.execute(&[call("prim.box", json!({}))], &BatchOptions { checkpoint: Some("four".into()), ..BatchOptions::default() }).unwrap();
    assert_eq!(s.checkpoints().collect::<Vec<_>>(), vec!["before_more", "four"]);
    assert_eq!(r.revision, s.revision());
}

#[test]
fn undo_limits_drop_the_oldest_steps() {
    let mut s = Session::new();
    s.set_limits(UndoLimits { max_steps: 3, max_bytes: usize::MAX });
    for i in 0..5 {
        exec(&mut s, &[call("prim.box", json!({ "name": format!("b{i}") }))]);
    }
    assert_eq!(s.history().len(), 3);
    assert_eq!(s.history()[0].label, "box_1");
    assert_eq!(s.history()[0].revision, 3);
}

#[test]
fn undo_memory_counts_only_unshared_mesh_data() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.plane", json!({ "name": "floor", "advanced": { "segments": [100, 100] } }))]);
    let after_create = s.undo_bytes();
    assert_eq!(after_create, 0, "the previous state had no meshes");
    exec(&mut s, &[call("scene.set_transform", json!({ "target": "floor", "position": [1, 0, 0] }))]);
    assert_eq!(s.undo_bytes(), 0, "moving an object shares the mesh");
    exec(&mut s, &[call("scene.apply_transform", json!({ "target": "floor" }))]);
    assert!(s.undo_bytes() > 100 * 100 * 12, "editing the mesh keeps the old copy alive: {}", s.undo_bytes());
}

#[test]
fn undo_and_redo_are_fast() {
    let mut s = Session::new();
    for i in 0..200 {
        exec(&mut s, &[call("prim.box", json!({ "name": format!("b{i}"), "advanced": { "segments": [8, 8, 8] } }))]);
    }
    let t = Instant::now();
    for _ in 0..50 {
        s.undo().unwrap();
        s.redo().unwrap();
    }
    let per = t.elapsed() / 100;
    assert!(per.as_millis() < 5, "undo or redo took {per:?}");
}

/// An op that panics: the batch must roll back and report INTERNAL.
struct Boom;

impl DynOp for Boom {
    fn id(&self) -> &'static str {
        "test.boom"
    }
    fn meta(&self) -> OpMeta {
        p3d_ops::Registry::builtin().get("scene.tag").unwrap().meta()
    }
    fn params_schema(&self) -> Value {
        json!({})
    }
    fn default_params(&self) -> Value {
        json!({})
    }
    fn check(&self, _doc: &Document, _args: &Value) -> Result<Value, OpError> {
        Ok(json!({}))
    }
    fn run(&self, tx: &mut Tx, _args: &Value, _ctx: &OpCtx) -> Result<Applied, OpError> {
        tx.doc_mut().create_object("half_done", None, p3d_scene::ObjectKind::Empty).unwrap();
        panic!("kaboom");
    }
}

#[test]
fn panics_roll_back() {
    let mut r = Registry::builtin();
    r.register(Arc::new(Boom)).unwrap();
    let mut s = Session::with_registry(Arc::new(r));
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    let e = s.execute(&[call("prim.box", json!({ "name": "b" })), call("test.boom", json!({}))], &BatchOptions::default()).unwrap_err();
    assert_eq!((e.index, e.error.code), (1, ErrorCode::Internal));
    assert!(e.error.message.contains("kaboom"));
    assert_eq!(s.doc().object_count(), 1);
    assert!(s.doc().resolve("half_done").is_err());
}

#[test]
fn jobs_commit_and_rerun_when_the_document_changed() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    let mut job = s.spawn_job(vec![call("prim.quad_sphere", json!({ "name": "ball", "segments": 32 }))], "add ball");
    // Change the document while the job may still run.
    exec(&mut s, &[call("prim.box", json!({ "name": "b" }))]);
    let r = s.wait_job(&mut job).unwrap();
    assert_eq!(r.revision, s.revision());
    for name in ["a", "b", "ball"] {
        assert!(s.doc().resolve(name).is_ok(), "{name} missing");
    }
    assert_eq!(s.history().last().unwrap().label, "add ball");
}

#[test]
fn cancelled_jobs_fail_cleanly() {
    let mut s = Session::new();
    let mut job = s.spawn_job(vec![call("prim.box", json!({}))], "box");
    job.handle().cancel();
    // The op may have finished before the cancel landed; both outcomes are clean.
    match s.wait_job(&mut job) {
        Ok(_) => assert_eq!(s.doc().object_count(), 1),
        Err(e) => {
            assert_eq!(e.error.code, ErrorCode::Cancelled);
            assert_eq!(s.doc().object_count(), 0);
        }
    }
}

#[test]
fn same_calls_give_identical_documents() {
    let calls = vec![
        call("prim.box", json!({ "name": "crate", "bevel": { "width": "1cm", "segments": 3 } })),
        call("prim.cylinder", json!({ "name": "post", "parent": "crate" })),
        call("scene.duplicate", json!({ "target": "post", "offset": [0.2, 0, 0] })),
        call("scene.join", json!({ "targets": ["post", "post_1"] })),
    ];
    let mut a = Session::new();
    let mut b = Session::new();
    exec(&mut a, &calls);
    exec(&mut b, &calls);
    let dump = |s: &Session| -> Vec<(String, Vec<[u32; 3]>)> {
        s.doc()
            .objects()
            .map(|(k, o)| {
                let pos = s.doc().object_mesh(k).map(|m| m.positions.iter().map(|p| p.to_array().map(f32::to_bits)).collect()).unwrap_or_default();
                (format!("{} {}", o.public_id(), s.doc().path(k).unwrap()), pos)
            })
            .collect()
    };
    assert_eq!(dump(&a), dump(&b));
}

#[test]
fn edit_ops_undo_and_redo_exactly() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "bin", "advanced": { "segments": [2, 2, 2] } }))]);
    let steps = [
        vec![call("edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "group": "top" } } }))],
        vec![OpCall { label: Some("cap".into()), ..call("edit.fill", json!({ "target": "bin" })) }],
        vec![call("edit.recalc_normals", json!({ "target": "bin", "inside": true }))],
    ];
    let mut states = vec![Arc::clone(s.doc())];
    for step in &steps {
        exec(&mut s, step);
        states.push(Arc::clone(s.doc()));
    }
    let key = s.doc().resolve("bin").unwrap();
    assert!(s.doc().object_mesh(key).unwrap().group("cap.faces").is_some(), "the call label names the group");
    for i in (0..steps.len()).rev() {
        s.undo().unwrap();
        assert!(Arc::ptr_eq(s.doc(), &states[i]), "undo to state {i}");
    }
    for (i, state) in states.iter().enumerate().skip(1) {
        s.redo().unwrap();
        assert!(Arc::ptr_eq(s.doc(), state), "redo to state {i}");
    }
}

#[test]
fn revisions_and_checkpoints_are_reachable() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    s.checkpoint("one");
    exec(&mut s, &[call("prim.box", json!({ "name": "b" }))]);
    exec(&mut s, &[call("prim.box", json!({ "name": "c" }))]);
    s.undo().unwrap();
    for rev in 0..=3 {
        assert_eq!(s.doc_at(rev).map(|d| d.revision), Some(rev), "revision {rev}");
    }
    assert!(s.doc_at(9).is_none());
    assert_eq!(s.checkpoint_doc("one").unwrap().object_count(), 1);
}

#[test]
fn revisions_are_never_reused_after_undo() {
    let mut s = Session::new();
    exec(&mut s, &[call("prim.box", json!({ "name": "a" }))]);
    exec(&mut s, &[call("prim.box", json!({ "name": "b" }))]);
    s.undo().unwrap();
    assert_eq!(s.revision(), 1);
    exec(&mut s, &[call("prim.box", json!({ "name": "c" }))]);
    assert_eq!(s.revision(), 3, "rev 2 was another state");
    assert!(s.doc_at(2).is_none(), "the undone branch is gone");
    let revs: Vec<u64> = s.history().iter().map(|h| h.revision).collect();
    assert_eq!(revs, vec![1, 3]);
}
