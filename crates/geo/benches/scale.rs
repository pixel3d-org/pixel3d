//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::{Affine3A, DVec3, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_geo::prim::{BoxSpec, CapKind, ConeSpec, Origin, PlaneSpec, QuadSphereSpec, cone, cuboid, plane, quad_sphere};
use p3d_testdata::{scale_sizes, sphere};

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
    let mut g = c.benchmark_group("scale_prim_cone");
    for &tris in &sizes {
        // Side 2 s x s / 2 and caps about s^2 / 8 triangles: s = sqrt(tris)
        // keeps both proportional to tris.
        let s = (((tris as f64).sqrt() / 4.0).round().max(1.0) as u32) * 4;
        let spec = ConeSpec { radius_bottom: 0.5, radius_top: 0.2, height: 1.0, segments: s, height_segments: s / 2, cap: CapKind::QuadGrid };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| cone(black_box(spec), Origin::Center)));
    }
    g.finish();

    // Tree queries start at 16k triangles: below that the whole tree sits in
    // L1 and L2 cache and the first step measures the cache, not the algorithm.
    let sizes = scale_sizes(16_000, 100_000);
    let meshes: Vec<(usize, TriBvh)> = sizes.iter().map(|&t| (t, TriBvh::from_mesh(&sphere(t), Affine3A::IDENTITY))).collect();
    let mut g = c.benchmark_group("scale_geo_bvh_build");
    for &t in &sizes {
        let m = sphere(t);
        g.bench_with_input(BenchmarkId::from_parameter(t), &m, |b, m| b.iter(|| TriBvh::from_mesh(black_box(m), Affine3A::IDENTITY)));
    }
    g.finish();
    // Closest points for the vertices of a nearby part: every 10th vertex of
    // the sphere, in vertex order, pushed up to one edge length off the
    // surface (contact scale). The cost is output-sensitive: O(log n + k) for
    // the k leaves within the found distance, so far points cost more and a
    // sphere center (every triangle equally far) is the O(n) worst case.
    let mut g = c.benchmark_group("scale_geo_bvh_closest_near");
    for (t, bvh) in &meshes {
        let h = (std::f32::consts::TAU / *t as f32).sqrt();
        let pts: Vec<Vec3> = sphere(*t).positions.iter().step_by(10).enumerate().map(|(i, p)| *p * (1.0 + h * (i as f32 * 2.1).sin())).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &pts, |b, pts| b.iter(|| pts.iter().filter_map(|p| bvh.closest(*p, 1.0)).count()));
    }
    g.finish();
    // Two spheres half an edge length apart (contact scale, as for parts that
    // touch); output-sensitive like the closest-point query.
    let mut g = c.benchmark_group("scale_geo_bvh_nearest");
    for (t, bvh) in &meshes {
        let h = (std::f32::consts::TAU / *t as f32).sqrt();
        let other = TriBvh::from_mesh(&sphere(*t), Affine3A::from_translation(Vec3::new(1.0 + 0.5 * h, 0.0, 0.0)));
        g.bench_with_input(BenchmarkId::from_parameter(t), &other, |b, other| b.iter(|| bvh.nearest(black_box(other), 10.0)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_geo_bvh_self_crossings");
    for (t, bvh) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), bvh, |b, bvh| {
            b.iter(|| {
                let mut n = 0u32;
                bvh.self_crossings(|_, _| {
                    n += 1;
                    true
                });
                n
            })
        });
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
