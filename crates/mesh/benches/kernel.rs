//! Mesh kernel at 1k, 10k, 100k and 1M triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use p3d_mesh::{Mesh, TriOptions};
use p3d_testdata::{SIZES, sphere};

/// Polygon soup of a mesh: positions, face sizes, corners.
fn soup(m: &Mesh) -> (Vec<p3d_mesh::Vec3>, Vec<u32>, Vec<u32>) {
    let sizes = m.faces().map(|f| m.face_len(f) as u32).collect();
    let corners = m.faces().flat_map(|f| m.face_verts(f).collect::<Vec<_>>()).collect();
    (m.positions.clone(), sizes, corners)
}

fn kernel(c: &mut Criterion) {
    let mut g = c.benchmark_group("mesh");
    g.sample_size(10);
    for &tris in &SIZES {
        let m = sphere(tris);
        g.throughput(Throughput::Elements(m.triangle_count() as u64));
        let (p, s, k) = soup(&m);
        g.bench_with_input(BenchmarkId::new("from_polygons", tris), &(p, s, k), |b, (p, s, k)| {
            b.iter(|| Mesh::from_polygons(black_box(p.clone()), black_box(s), black_box(k)))
        });
        g.bench_with_input(BenchmarkId::new("validate", tris), &m, |b, m| b.iter(|| black_box(m).validate()));
        g.bench_with_input(BenchmarkId::new("corner_normals", tris), &m, |b, m| b.iter(|| black_box(m).compute_corner_normals()));
        g.bench_with_input(BenchmarkId::new("tri_buffers", tris), &m, |b, m| b.iter(|| black_box(m).to_tri_buffers(&TriOptions::default())));
        let mut holed = m.clone();
        let every_7th: Vec<u32> = holed.faces().step_by(7).collect();
        let _ = holed.delete_faces(&every_7th);
        g.bench_with_input(BenchmarkId::new("compact", tris), &holed, |b, h| b.iter(|| black_box(h.clone()).compact()));
        g.bench_with_input(BenchmarkId::new("delete_faces_1_in_7", tris), &(m, every_7th), |b, (m, del)| {
            b.iter(|| black_box(m.clone()).delete_faces(black_box(del)))
        });
    }
    g.finish();
}

criterion_group!(benches, kernel);
criterion_main!(benches);
