//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one diagnostics pass at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::Vec3;
use p3d_perceive::View;
use p3d_perceive::diagnose::{drafts, flow, shading};
use p3d_scene::Document;
use p3d_testdata::{scale_sizes, scenes, sphere, tube};

/// One part tagged `deforming`.
fn doc_of(mesh: p3d_mesh::Mesh) -> Document {
    let mut doc = Document::new();
    let k = scenes::add(&mut doc, "part", None, mesh, [0.0; 3]);
    if let Some(o) = doc.object_mut(k) {
        o.tags.push(flow::DEFORMING.into());
    }
    doc
}

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let spheres: Vec<(usize, Document)> = sizes.iter().map(|&t| (t, doc_of(sphere(t)))).collect();
    let tubes: Vec<(usize, Document)> = sizes.iter().map(|&t| (t, doc_of(tube(t)))).collect();

    let mut g = c.benchmark_group("scale_diagnose_shading");
    for (t, doc) in &spheres {
        let view = View::new(doc, None).unwrap_or_else(|_| View::empty(doc));
        g.bench_with_input(BenchmarkId::from_parameter(t), &view, |b, v| {
            b.iter(|| {
                let mut out = Vec::new();
                shading::check(black_box(v), 0, &mut out);
                out
            })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_diagnose_flow");
    for (t, doc) in &spheres {
        let view = View::new(doc, None).unwrap_or_else(|_| View::empty(doc));
        g.bench_with_input(BenchmarkId::from_parameter(t), &view, |b, v| {
            b.iter(|| {
                let mut out = Vec::new();
                flow::check(black_box(v), 0, &mut out);
                out
            })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_diagnose_ring_flow");
    for (t, doc) in &spheres {
        let view = View::new(doc, None).unwrap_or_else(|_| View::empty(doc));
        let at = Vec3::new(0.0, 0.0, view.hi.z);
        g.bench_with_input(BenchmarkId::from_parameter(t), &view, |b, v| b.iter(|| flow::ring_flow(&v.parts[0], black_box(at), 0.1)));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_diagnose_joint_loops");
    for (t, doc) in &tubes {
        let view = View::new(doc, None).unwrap_or_else(|_| View::empty(doc));
        // Built once, as diagnostics share it with the other checks.
        let _ = view.parts.first().map(|p| p.bvh());
        g.bench_with_input(BenchmarkId::from_parameter(t), &view, |b, v| {
            b.iter(|| flow::joint_loops(&v.parts[0], black_box(Vec3::new(0.0, 0.5, 0.2))))
        });
    }
    g.finish();

    // Every mesh-level check from scratch (view, BVH, topology).
    let mut g = c.benchmark_group("scale_diagnose_drafts");
    for (t, doc) in &spheres {
        g.bench_with_input(BenchmarkId::from_parameter(t), doc, |b, d| {
            b.iter(|| drafts(&View::new(black_box(d), None).unwrap_or_else(|_| View::empty(d))))
        });
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
