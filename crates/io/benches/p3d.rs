//! `.p3d` save and load at 100k and 1M triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_io::p3d;
use p3d_scene::Document;
use p3d_testdata::terrain;

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("p3d");
    g.sample_size(10);
    for tris in [100_000, 1_000_000] {
        let mut d = Document::new();
        d.create_mesh_object("terrain", None, terrain(tris, 1)).ok();
        let Ok((bytes, _)) = p3d::to_bytes(&d) else { return };
        g.bench_with_input(BenchmarkId::new("save", tris), &d, |b, d| b.iter(|| p3d::to_bytes(black_box(d)).ok()));
        g.bench_with_input(BenchmarkId::new("load", tris), &bytes, |b, bytes| b.iter(|| p3d::from_bytes(black_box(bytes)).ok()));
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
