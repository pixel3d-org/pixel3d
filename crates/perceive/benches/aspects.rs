//! Perception aspects and diagnostics on a 100k-triangle scene (project
//! spec 21.8: any aspect under 50 ms, diagnose under 200 ms).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use p3d_perceive::probe::SurfacePoint;
use p3d_perceive::section::SectionOptions;
use p3d_perceive::silhouette::SilhouetteOptions;
use p3d_perceive::{View, diagnose, diff, probe, relations, section, shape, silhouette, structure};
use p3d_scene::Document;
use p3d_testdata::{scenes, sphere};

/// A 100k-triangle body with a lid, two handles and a floating badge.
fn scene() -> Document {
    let mut doc = Document::new();
    let body = scenes::add(&mut doc, "body", None, sphere(100_000), [0.0, 0.5, 0.0]);
    scenes::add(&mut doc, "lid", Some(body), scenes::box_mesh([0.4, 0.05, 0.4], 0.0), [0.0, 0.49, 0.0]);
    scenes::add(&mut doc, "handle_l", Some(body), scenes::box_mesh([0.05, 0.05, 0.2], 0.0), [-0.5, 0.0, 0.0]);
    scenes::add(&mut doc, "handle_r", Some(body), scenes::box_mesh([0.05, 0.05, 0.2], 0.0), [0.5, 0.0, 0.0]);
    scenes::add(&mut doc, "badge", None, scenes::box_mesh([0.1, 0.1, 0.01], 0.0), [0.0, 0.5, 0.55]);
    doc
}

/// The view of a target (benches cannot unwrap; a missing target is a bug).
fn view<'a>(doc: &'a Document, target: Option<&str>) -> View<'a> {
    View::new(doc, target).unwrap_or_else(|_| View::empty(doc))
}

fn aspects(c: &mut Criterion) {
    let doc = scene();
    let mut g = c.benchmark_group("perceive_100k");
    g.sample_size(10);
    // Each iteration builds its view (and BVHs) from scratch: no caching.
    g.bench_function("structure", |b| b.iter(|| structure::report(&view(black_box(&doc), None))));
    g.bench_function("shape", |b| b.iter(|| shape::report(&view(black_box(&doc), Some("body")))));
    g.bench_function("silhouette", |b| b.iter(|| silhouette::report(&view(black_box(&doc), None), &SilhouetteOptions::default())));
    g.bench_function("section", |b| b.iter(|| section::report(&view(black_box(&doc), None), &SectionOptions::default())));
    g.bench_function("relations", |b| b.iter(|| relations::report(&view(black_box(&doc), None))));
    let at = SurfacePoint { on: "body".into(), landmark: Some("top".into()), ..SurfacePoint::default() };
    g.bench_function("probe", |b| b.iter(|| probe::report(&view(black_box(&doc), None), &at)));
    let mut after = doc.clone();
    if let Some(o) = after.resolve("lid").ok().and_then(|lid| after.object_mut(lid)) {
        o.transform.translation.y += 0.01;
    }
    // Findings are cached per revision by the frontends, as here.
    let (vb, va) = (view(&doc, None), view(&after, None));
    let (fb, fa) = (diagnose::diagnose(&vb), diagnose::diagnose(&va));
    g.bench_function("diff", |b| b.iter(|| diff::diff(&view(black_box(&doc), None), &view(black_box(&after), None), &fb, &fa)));
    g.bench_function("diagnose", |b| b.iter(|| diagnose::report(&view(black_box(&doc), None))));
    g.finish();
}

criterion_group!(benches, aspects);
criterion_main!(benches);
