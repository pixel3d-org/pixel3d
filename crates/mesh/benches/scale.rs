//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_mesh::{Mesh, Patch, TriOptions};
use p3d_testdata::{scale_sizes, sphere};

/// A patch replacing each given face by a fan of triangles around its centroid.
fn poke(m: &Mesh, faces: impl Iterator<Item = u32>) -> Patch {
    let mut p = Patch::new(m);
    for f in faces {
        p.remove_face(f);
        let vs: Vec<u32> = m.face_verts(f).collect();
        let c = p.new_vert(m.face_centroid(f), None);
        for i in 0..vs.len() {
            p.add_face(&[vs[i], vs[(i + 1) % vs.len()], c], &[], 0, Some(f));
        }
    }
    p
}

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

    let mut g = c.benchmark_group("scale_mesh_corner_normals");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| black_box(m).compute_corner_normals()));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_delete_faces");
    for (t, m) in &meshes {
        let del: Vec<u32> = m.faces().step_by(3).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &(m.clone(), del), |b, (m, del)| {
            b.iter_batched(|| m.clone(), |mut x| x.delete_faces(del).ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    // Patch cost grows with the patch: poke every third face.
    let mut g = c.benchmark_group("scale_mesh_patch_poke");
    for (t, m) in &meshes {
        let patch = poke(m, m.faces().step_by(3));
        g.bench_with_input(BenchmarkId::from_parameter(t), &(m.clone(), patch), |b, (m, p)| {
            b.iter_batched(|| m.clone(), |mut x| x.apply_patch(p).ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    // ... and not with the mesh: poke the same 16 faces of ever larger meshes
    // (a slope near 0). The columns have room, as after the first edit of a
    // transaction; growing a full column is amortized O(1) per element.
    let mut g = c.benchmark_group("scale_mesh_patch_local");
    for (t, m) in &meshes {
        let patch = poke(m, m.faces().take(16));
        let roomy = || {
            let mut x = m.clone();
            x.reserve(64, 256, 128);
            x
        };
        g.bench_with_input(BenchmarkId::from_parameter(t), &patch, |b, p| {
            b.iter_batched(roomy, |mut x| x.apply_patch(p).ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_flip");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter_batched(|| m.clone(), |mut x| x.flip_all(), BatchSize::LargeInput));
    }
    g.finish();

    // Worst case for part count: every face its own part (n meshes out).
    let mut g = c.benchmark_group("scale_mesh_split_parts");
    for (t, m) in &meshes {
        let labels: Vec<u32> = (0..m.face_slots() as u32).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &labels, |b, l| b.iter(|| black_box(m).split_by_face_labels(l)));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_append");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter_batched(|| m.clone(), |mut x| x.append(m), BatchSize::LargeInput));
    }
    g.finish();

    // Every 7th face deleted, then every hole filled again.
    let mut g = c.benchmark_group("scale_mesh_fill_holes");
    for (t, m) in &meshes {
        let mut holed = m.clone();
        let del: Vec<u32> = holed.faces().step_by(7).collect();
        let _ = holed.delete_faces(&del);
        let starts: Vec<u32> = (0..holed.he_slots() as u32).filter(|&h| holed.is_he_alive(h) && holed.is_boundary_he(h)).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &holed, |b, h| {
            b.iter_batched(
                || h.clone(),
                |mut x| {
                    for &s in &starts {
                        if x.is_boundary_he(s) {
                            let _ = x.fill_boundary_loop(s);
                        }
                    }
                    x
                },
                BatchSize::LargeInput,
            )
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_mesh_flip_part");
    for (t, m) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter_batched(|| m.clone(), |mut x| x.flip_part(0), BatchSize::LargeInput));
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
