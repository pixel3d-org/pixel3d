//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_mesh::Mesh;
use p3d_testdata::{beveled_box, scale_sizes};

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let meshes: Vec<(usize, Mesh)> = sizes.iter().map(|&t| (t, beveled_box(t))).collect();

    let mut g = c.benchmark_group("scale_analyze_topology");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| p3d_analyze::topology(black_box(m))));
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
