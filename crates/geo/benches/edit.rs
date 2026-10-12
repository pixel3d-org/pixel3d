//! Edit kernels at 1k, 10k, 100k and 1M triangles.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use p3d_geo::edit::bridge;
use p3d_geo::prim::{CapKind, CylinderSpec, Origin, cylinder};
use p3d_testdata::SIZES;

#[path = "workloads/edit.rs"]
mod workloads;

fn edits(c: &mut Criterion) {
    let mut g = c.benchmark_group("edit");
    g.sample_size(10);
    for &tris in &SIZES {
        g.throughput(Throughput::Elements(tris as u64));
        for (name, m, faces) in workloads::workloads(tris) {
            g.bench_with_input(BenchmarkId::new(name, tris), &(m, faces), |b, (m, faces)| {
                b.iter_batched(|| m.clone(), |x| workloads::run(name, black_box(x), faces), BatchSize::LargeInput)
            });
        }
    }
    // Bridging two tube ends: the alignment is O(a x b) in the hole sizes,
    // which are bounded by MAX_BRIDGE_LOOP.
    for s in [64u32, 256, 1024] {
        let tube = |y: f32| {
            let spec = CylinderSpec { radius: 0.5, height: 1.0, segments: s, height_segments: 1, cap: CapKind::None };
            let mut t = cylinder(&spec, Origin::Center).unwrap_or_default();
            t.transform(glam::Affine3A::from_translation(glam::Vec3::new(0.0, y, 0.0)));
            t
        };
        let mut m = tube(0.0);
        m.append(&tube(2.0));
        let hole = |y: f32| {
            (0..m.he_slots() as u32).find(|&h| m.is_he_alive(h) && m.is_boundary_he(h) && (m.positions[m.origin(h) as usize].y - y).abs() < 1e-4)
        };
        if let (Some(a), Some(b)) = (hole(0.5), hole(1.5)) {
            g.bench_with_input(BenchmarkId::new("bridge_hole", s), &m, |bch, m| {
                bch.iter_batched(|| m.clone(), |mut x| bridge(&mut x, a, b, 1).map(|r| r.faces.len()), BatchSize::LargeInput)
            });
        }
    }
    g.finish();
}

criterion_group!(benches, edits);
criterion_main!(benches);
