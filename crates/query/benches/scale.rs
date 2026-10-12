//! Scaling check inputs for `cargo xtask bench-scale`: selector evaluation
//! at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_mesh::{ElementKind, Mesh};
use p3d_query::{Selector, select};
use p3d_testdata::{scale_sizes, sphere};

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let meshes: Vec<(usize, Mesh)> = sizes
        .iter()
        .map(|&t| {
            let mut m = sphere(t);
            let half: Vec<u32> = m.faces().filter(|f| f % 2 == 0).collect();
            let _ = m.set_group("half", ElementKind::Face, half);
            (t, m)
        })
        .collect();
    let selectors: [(&str, &str); 3] = [
        ("scale_query_and_normal", r#"{"and": [{"faces": {"group": "half"}}, {"faces": {"normal": {"dir": [0, 1, 0], "within_deg": 60}}}]}"#),
        ("scale_query_grow", r#"{"grow": {"steps": 2, "of": {"faces": {"normal": {"dir": [1, 0, 0], "within_deg": 20}}}}}"#),
        ("scale_query_edges", r#"{"or": [{"edges": {"sharper_than_deg": 5}}, {"edges": {"in_box": {"min": [0, 0, 0], "max": [1, 1, 1]}}}]}"#),
    ];
    for (name, json) in selectors {
        let Ok(s) = serde_json::from_str::<Selector>(json) else { continue };
        let mut g = c.benchmark_group(name);
        for (t, m) in &meshes {
            g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| select(black_box(m), &s).ok()));
        }
        g.finish();
    }
}

criterion_group!(benches, scale);
criterion_main!(benches);
