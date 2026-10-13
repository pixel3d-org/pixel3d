//! Scaling check inputs for `cargo xtask bench-scale`: a broadleaf tree
//! grown from n, 4n and 16n attractors. The growth step shrinks with the
//! attractor count (step ~ n^(-1/3)) and the budget grows with it, so the
//! skeleton, the wood and the cards all scale with n.

// A measurement program: setup errors stop it.
#![allow(clippy::expect_used)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_vegetation::{Species, TreeSpec, tree};

fn sizes() -> [usize; 3] {
    let n = if std::env::var("P3D_BENCH_SCALE").is_ok_and(|v| v == "large") { 1600 } else { 400 };
    [n, 4 * n, 16 * n]
}

fn spec(n: usize) -> TreeSpec {
    let base = TreeSpec::preset(Species::Broadleaf);
    let k = (n as f32 / 1600.0).cbrt();
    TreeSpec { attractors: n as u32, branch_spacing: 0.3 / k, card_size: 0.6 / k.sqrt(), budget: (n as u32 * 6).max(2_000), ..base }
}

fn scale(c: &mut Criterion) {
    let mut g = c.benchmark_group("scale_tree");
    for &n in &sizes() {
        let s = spec(n);
        g.bench_with_input(BenchmarkId::from_parameter(n), &s, |b, s| b.iter(|| tree(black_box(s)).expect("a tree")));
    }
    g.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(std::time::Duration::from_millis(500)).measurement_time(std::time::Duration::from_secs(3));
    targets = scale
}
criterion_main!(benches);
