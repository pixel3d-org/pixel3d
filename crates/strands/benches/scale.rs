//! Scaling check inputs for `cargo xtask bench-scale`: hair cards and
//! clumps on a sphere with n, 4n and 16n guides, and braids among them.

// A measurement program: setup errors stop it.
#![allow(clippy::expect_used)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_geo::prim::{Origin, QuadSphereSpec, quad_sphere};
use p3d_strands::{Braids, Kind, Output, Region, StrandSpec, Tie, generate};
use p3d_testdata::scale_sizes;

fn scale(c: &mut Criterion) {
    let head = quad_sphere(&QuadSphereSpec { radius: 0.1, segments: 32 }, Origin::Center).expect("a sphere");
    for (name, output) in [("scale_strands_cards", Output::Cards), ("scale_strands_clumps", Output::Clumps)] {
        let mut g = c.benchmark_group(name);
        for &n in &scale_sizes(64, 256) {
            let spec = StrandSpec { region: Region::All, output, clumps: n as u32, cap: false, ..StrandSpec::preset(Kind::Hair) };
            g.bench_with_input(BenchmarkId::from_parameter(n), &spec, |b, spec| b.iter(|| generate(black_box(&head), spec)));
        }
        g.finish();
    }
    // Braids: n braids (with beads and a bead tie) among 4n clumps.
    let mut g = c.benchmark_group("scale_strands_braids");
    for &n in &scale_sizes(4, 16) {
        let spec = StrandSpec {
            region: Region::All,
            output: Output::Clumps,
            clumps: 4 * n as u32,
            braids: Braids { count: n as u32, beads: 2, tie: Tie::Bead, length: 0.2, ..Braids::default() },
            ..StrandSpec::preset(Kind::Beard)
        };
        g.bench_with_input(BenchmarkId::from_parameter(n), &spec, |b, spec| b.iter(|| generate(black_box(&head), spec)));
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(3));
    targets = scale
}
criterion_main!(benches);
