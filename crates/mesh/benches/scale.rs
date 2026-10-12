//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_mesh::{Mesh, TriOptions};
use p3d_testdata::{scale_sizes, sphere};

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let meshes: Vec<(usize, Mesh)> = sizes.iter().map(|&t| (t, sphere(t))).collect();

    let mut g = c.benchmark_group("scale_mesh_from_polygons");
    for (t, m) in &meshes {
        let sizes: Vec<u32> = m.faces().map(|f| m.face_len(f) as u32).collect();
        let corners: Vec<u32> = m.faces().flat_map(|f| m.face_verts(f).collect::<Vec<_>>()).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &(m.positions.clone(), sizes, corners), |b, (p, s, k)| {
            b.iter(|| Mesh::from_polygons(black_box(p.clone()), black_box(s), black_box(k)))
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_validate");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| black_box(m).validate()));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_tri_buffers");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| black_box(m).to_tri_buffers(&TriOptions::default())));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_compact");
    for (t, m) in &meshes {
        let mut holed = m.clone();
        let del: Vec<u32> = holed.faces().step_by(7).collect();
        let _ = holed.delete_faces(&del);
        g.bench_with_input(BenchmarkId::from_parameter(t), &holed, |b, h| b.iter(|| black_box(h.clone()).compact()));
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
