//! One bench per op: every registered op runs its documented examples on
//! the fixture its category's examples are written for (the same scenes as
//! the example tests), from a fresh copy each time. New ops are covered
//! automatically. Kernel scaling is checked separately by
//! `cargo xtask bench-scale`; this measures whole op calls (parse, validate,
//! apply, summary), whose small ones should stay under 1 ms.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use p3d_ops::{OpCtx, Registry};
use serde_json::Value;

#[path = "../tests/fixtures/mod.rs"]
mod fixtures;

fn ops(c: &mut Criterion) {
    let registry = Registry::builtin();
    let (_dir, files) = fixtures::io_folder();
    let mut g = c.benchmark_group("op");
    for op in registry.iter() {
        let base = fixtures::for_category(op.meta().category);
        for (i, ex) in op.meta().examples.iter().enumerate() {
            let Ok(args) = serde_json::from_str::<Value>(ex.args) else { continue };
            let name = if i == 0 { op.id().to_owned() } else { format!("{}#{}", op.id(), i + 1) };
            g.bench_function(name, |b| {
                b.iter_batched(
                    || base.clone(),
                    |mut tx| op.run(&mut tx, &args, &OpCtx { files: Some(&files), ..OpCtx::default() }),
                    BatchSize::SmallInput,
                )
            });
        }
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20).warm_up_time(std::time::Duration::from_millis(200)).measurement_time(std::time::Duration::from_millis(600));
    targets = ops
}
criterion_main!(benches);
