//! Scaling check inputs for `cargo xtask bench-scale`: meshing a blended
//! head-like SDF (ellipsoids and a capsule, smooth unions) at voxel sizes
//! giving about n, 4n and 16n triangles.

// A measurement program: setup errors stop it.
#![allow(clippy::expect_used)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::Vec3;
use p3d_sdf::{Combine, Item, MeshSpec, Prim, Sdf, mesh};
use p3d_testdata::scale_sizes;

fn head() -> Sdf {
    let mut nose = Item::prim(Prim::Capsule { a: Vec3::new(0.0, 0.13, 0.08), b: Vec3::new(0.0, 0.1, 0.11), ra: 0.012, rb: 0.016 }, Vec3::ZERO);
    nose.combine = Combine::SmoothUnion(0.02);
    let items = vec![
        Item::prim(Prim::Ellipsoid { radii: Vec3::new(0.075, 0.105, 0.09) }, Vec3::new(0.0, 0.13, 0.0)),
        Item::prim(Prim::Ellipsoid { radii: Vec3::new(0.055, 0.045, 0.055) }, Vec3::new(0.0, 0.06, 0.035)).with(Combine::SmoothUnion(0.025)),
        nose,
    ];
    Sdf::new(items).expect("a valid tree")
}

fn scale(c: &mut Criterion) {
    let sdf = head();
    // The surface is about 0.1 m^2; quads of side h make 2 triangles each.
    let area = 0.105f32;
    let mut g = c.benchmark_group("scale_sdf_mesh");
    for &t in &scale_sizes(4_000, 100_000) {
        let voxel = (2.0 * area / t as f32).sqrt();
        g.bench_with_input(BenchmarkId::from_parameter(t), &voxel, |b, &voxel| b.iter(|| mesh(black_box(&sdf), &MeshSpec { voxel })));
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(2));
    targets = scale
}
criterion_main!(benches);
