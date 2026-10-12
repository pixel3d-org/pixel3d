//! Edit kernels at 1k, 10k, 100k and 1M triangles.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use p3d_testdata::SIZES;

#[path = "workloads/edit.rs"]
mod workloads;

fn edits(c: &mut Criterion) {
    let mut g = c.benchmark_group("edit");
    g.sample_size(10);
    for &tris in &SIZES {
        g.throughput(Throughput::Elements(tris as u64));
        for (name, m, faces) in workloads::workloads(tris) {
            g.bench_with_input(BenchmarkId::new(name, tris), &(m, faces), |b, (m, faces)| {
                b.iter_batched(|| m.clone(), |x| workloads::run(name, black_box(x), faces), BatchSize::LargeInput)
            });
        }
    }
    g.finish();
}

criterion_group!(benches, edits);
criterion_main!(benches);
