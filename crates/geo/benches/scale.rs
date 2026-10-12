//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::DVec3;
use p3d_geo::prim::{BoxSpec, Origin, PlaneSpec, QuadSphereSpec, cuboid, plane, quad_sphere};
use p3d_testdata::scale_sizes;

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let mut g = c.benchmark_group("scale_prim_plane");
    for &tris in &sizes {
        let n = ((tris as f64 / 2.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::from_parameter(tris), &n, |b, &n| {
            b.iter(|| plane(black_box(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: n, segments_z: n })))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_quad_sphere");
    for &tris in &sizes {
        let n = ((tris as f64 / 12.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::from_parameter(tris), &n, |b, &n| {
            b.iter(|| quad_sphere(black_box(&QuadSphereSpec { radius: 0.5, segments: n }), Origin::Center))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_box");
    for &tris in &sizes {
        let n = ((tris as f64 / 12.0).sqrt().round()) as u32;
        let spec = BoxSpec { size: DVec3::ONE, segments: [n; 3], bevel_width: 0.05, bevel_segments: 3 };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| cuboid(black_box(spec), Origin::Center)));
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
