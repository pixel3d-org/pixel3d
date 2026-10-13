//! Scaling check inputs for `cargo xtask bench-scale`: ornament with n, 4n
//! and 16n motifs (runes and studs) along a band.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::Vec3;
use p3d_features::ornament::{Motif, OrnamentSpec, Outward, band, ornament};
use p3d_testdata::scale_sizes;

fn scale(c: &mut Criterion) {
    let mut g = c.benchmark_group("scale_ornament");
    for &n in &scale_sizes(256, 1024) {
        // A band long enough for n motifs, alternating runes and studs.
        let spec = OrnamentSpec {
            path: band(n as f32 * 0.01, Vec3::ZERO, Vec3::Y, n * 2),
            closed: true,
            outward: Outward::FromAxis { point: Vec3::ZERO, axis: Vec3::Y },
            motif: Motif::Rune,
            count: n as u32,
            size: 0.04,
            ..OrnamentSpec::default()
        };
        g.bench_with_input(BenchmarkId::from_parameter(n), &spec, |b, spec| b.iter(|| ornament(black_box(spec))));
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(3));
    targets = scale
}
criterion_main!(benches);
