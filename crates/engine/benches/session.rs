//! Session overheads: undo/redo on a heavy scene and a tiny op's transaction cost.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use p3d_engine::{BatchOptions, OpCall, Session};
use serde_json::json;

/// A 1M-triangle grid, 200 small boxes and a few undo steps of each kind.
fn heavy() -> Session {
    let mut s = Session::new();
    let opts = BatchOptions::default();
    s.execute(&[OpCall::new("prim.plane", json!({ "name": "ground", "advanced": { "segments": [707, 707] } }))], &opts).ok();
    for i in 0..200 {
        s.execute(&[OpCall::new("prim.box", json!({ "name": format!("b{i}"), "advanced": { "segments": [4, 4, 4] } }))], &opts).ok();
    }
    s.execute(&[OpCall::new("scene.set_transform", json!({ "target": "ground", "position": [0, -1, 0] }))], &opts).ok();
    s
}

fn bench(c: &mut Criterion) {
    let mut s = heavy();
    c.bench_function("session/undo_redo_1m_tris_201_objects", |b| {
        b.iter(|| {
            black_box(s.undo().ok());
            black_box(s.redo().ok());
        })
    });
    let mut s = Session::new();
    s.execute(&[OpCall::new("prim.box", json!({ "name": "crate" }))], &BatchOptions::default()).ok();
    let call = [OpCall::new("scene.tag", json!({ "target": "crate", "add": ["low"] }))];
    let dry = BatchOptions { dry_run: true, ..BatchOptions::default() };
    c.bench_function("session/tiny_op_transaction", |b| b.iter(|| black_box(s.execute(&call, &dry).ok())));
}

criterion_group!(benches, bench);
criterion_main!(benches);
