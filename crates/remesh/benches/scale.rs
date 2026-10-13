//! Scaling check inputs for `cargo xtask bench-scale`: quad remeshing a
//! sphere with edge lengths giving about n, 4n and 16n output triangles.

// A measurement program: setup errors stop it.
#![allow(clippy::expect_used)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_geo::prim::{IcoSphereSpec, Origin, ico_sphere};
use p3d_remesh::{QuadSpec, quad_remesh};
use p3d_testdata::scale_sizes;

fn scale(c: &mut Criterion) {
    let sphere = ico_sphere(&IcoSphereSpec { radius: 0.5, subdivisions: 3 }, Origin::Center).expect("a sphere");
    let mut g = c.benchmark_group("scale_remesh_quad");
    for &t in &scale_sizes(4_000, 100_000) {
        // About pi / edge^2 quads, two triangles each.
        let edge = (std::f32::consts::PI * 2.0 / t as f32).sqrt();
        let spec = QuadSpec { edge, ..QuadSpec::default() };
        g.bench_with_input(BenchmarkId::from_parameter(t), &spec, |b, spec| b.iter(|| quad_remesh(black_box(&sphere), spec)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_remesh_isotropic");
    for &t in &scale_sizes(4_000, 100_000) {
        // An equilateral triangle of side e covers about 0.433 e^2.
        let edge = (std::f32::consts::PI / (0.433 * t as f32)).sqrt();
        let spec = p3d_remesh::IsotropicSpec { edge, ..p3d_remesh::IsotropicSpec::default() };
        g.bench_with_input(BenchmarkId::from_parameter(t), &spec, |b, spec| b.iter(|| p3d_remesh::isotropic(black_box(&sphere), spec)));
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(3));
    targets = scale
}
criterion_main!(benches);
