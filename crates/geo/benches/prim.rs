//! Primitive generators at 1k, 10k, 100k and 1M triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use glam::DVec3;
use p3d_geo::prim::{
    BoxSpec, CapKind, CapsuleSpec, ConeSpec, CylinderSpec, IcoSphereSpec, Origin, PlaneSpec, QuadSphereSpec, TorusSpec, TubeSpec, UvSphereSpec,
    WedgeSpec, capsule, cone, cuboid, cylinder, ico_sphere, plane, quad_sphere, torus, tube, uv_sphere, wedge,
};
use p3d_testdata::SIZES;

fn prims(c: &mut Criterion) {
    let mut g = c.benchmark_group("prim");
    g.sample_size(10);
    for &tris in &SIZES {
        g.throughput(Throughput::Elements(tris as u64));
        let n = ((tris as f64 / 2.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::new("plane", tris), &n, |b, &n| {
            b.iter(|| plane(black_box(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: n, segments_z: n })))
        });
        if tris > 100_000 {
            continue;
        }
        let n = ((tris as f64 / 12.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::new("quad_sphere", tris), &n, |b, &n| {
            b.iter(|| quad_sphere(black_box(&QuadSphereSpec { radius: 0.5, segments: n }), Origin::Center))
        });
        let s = ((tris as f64).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::new("uv_sphere", tris), &s, |b, &s| {
            b.iter(|| uv_sphere(black_box(&UvSphereSpec { radius: 0.5, segments: s, rings: (s / 2).max(2) }), Origin::Center))
        });
        let level = ((tris as f64 / 20.0).log(4.0).round().max(0.0)) as u32;
        g.bench_with_input(BenchmarkId::new("ico_sphere", tris), &level, |b, &l| {
            b.iter(|| ico_sphere(black_box(&IcoSphereSpec { radius: 0.5, subdivisions: l }), Origin::Center))
        });
        g.bench_with_input(BenchmarkId::new("box_bevel3", tris), &n, |b, &n| {
            let spec = BoxSpec { size: DVec3::ONE, segments: [n; 3], bevel_width: 0.05, bevel_segments: 3 };
            b.iter(|| cuboid(black_box(&spec), Origin::Center))
        });
        let s = (((tris as f64 / 2.0).sqrt() / 4.0).round().max(1.0) as u32) * 4;
        g.bench_with_input(BenchmarkId::new("cylinder", tris), &s, |b, &s| {
            let spec = CylinderSpec { radius: 0.5, height: 1.0, segments: s, height_segments: s / 2, cap: CapKind::QuadGrid };
            b.iter(|| cylinder(black_box(&spec), Origin::Center))
        });
        g.bench_with_input(BenchmarkId::new("cone", tris), &s, |b, &s| {
            let spec = ConeSpec { radius_bottom: 0.5, radius_top: 0.2, height: 1.0, segments: s, height_segments: s / 2, cap: CapKind::QuadGrid };
            b.iter(|| cone(black_box(&spec), Origin::Center))
        });
        let t = ((tris as f64 / 2.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::new("torus", tris), &t, |b, &t| {
            b.iter(|| torus(black_box(&TorusSpec { major_radius: 0.5, minor_radius: 0.1, segments: t, sides: t }), Origin::Center))
        });
        let s = ((tris as f64).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::new("tube", tris), &s, |b, &s| {
            let spec = TubeSpec { outer_radius: 0.5, inner_radius: 0.4, height: 1.0, segments: s, height_segments: (s / 4).max(1) };
            b.iter(|| tube(black_box(&spec), Origin::Center))
        });
        let s = (((tris as f64 * 2.0).sqrt() / 8.0).round().max(1.0) as u32) * 8;
        g.bench_with_input(BenchmarkId::new("capsule", tris), &s, |b, &s| {
            let spec = CapsuleSpec { radius: 0.2, height: 1.0, segments: s, height_segments: s / 8 };
            b.iter(|| capsule(black_box(&spec), Origin::Center))
        });
    }
    g.bench_function("wedge", |b| b.iter(|| wedge(black_box(&WedgeSpec::default()), Origin::Center)));
    g.finish();
}

criterion_group!(benches, prims);
criterion_main!(benches);
