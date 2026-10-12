//! Scaling check inputs for `cargo xtask bench-scale`: document operations
//! at n, 4n and 16n objects.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use p3d_scene::{Document, ObjectKey, ObjectKind};
use p3d_testdata::scale_sizes;

/// One parent with `n` children.
fn flat(n: usize) -> (Document, ObjectKey) {
    let mut d = Document::new();
    let root = d.create_object("root", None, ObjectKind::Empty).unwrap_or_default();
    for i in 0..n {
        let _ = d.create_object(&format!("c{i}"), Some(root), ObjectKind::Empty);
    }
    (d, root)
}

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(1_000, 20_000);

    let mut g = c.benchmark_group("scale_scene_create");
    for &n in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| b.iter(|| flat(black_box(n))));
    }
    g.finish();

    let mut g = c.benchmark_group("scale_scene_delete_subtree");
    for &n in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(|| flat(n), |(mut d, root)| d.delete(root, true).ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_scene_delete_keep_children");
    for &n in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(|| flat(n), |(mut d, root)| d.delete(root, false).ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_scene_rename_subtree");
    for &n in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter_batched(|| flat(n), |(mut d, root)| d.rename(root, "renamed").ok(), BatchSize::LargeInput)
        });
    }
    g.finish();

    // A chain n deep: creating it and resolving the deepest path. Full-path
    // indexes made this quadratic (depth x name length per object).
    let mut g = c.benchmark_group("scale_scene_deep_chain");
    for &n in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                let mut d = Document::new();
                let mut parent = None;
                for i in 0..n {
                    parent = d.create_object(&format!("link_{i}"), parent, ObjectKind::Empty).ok();
                }
                let path: Vec<String> = (0..n).map(|i| format!("link_{i}")).collect();
                d.resolve(&path.join("/")).ok()
            })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("scale_scene_resolve_leaf");
    for &n in &sizes {
        let (d, _) = flat(n);
        let last = format!("c{}", n - 1);
        g.bench_with_input(BenchmarkId::from_parameter(n), &(d, last), |b, (d, last)| b.iter(|| d.resolve(black_box(last)).ok()));
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
