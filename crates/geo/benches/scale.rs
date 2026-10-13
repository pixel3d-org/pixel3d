//! Scaling check inputs for `cargo xtask bench-scale`: each `scale_*` group
//! runs one kernel at n, 4n and 16n triangles.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use glam::{Affine3A, DVec3, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_geo::prim::{
    BoxSpec, CapKind, CapsuleSpec, ConeSpec, IcoSphereSpec, Origin, PlaneSpec, QuadSphereSpec, TorusSpec, TubeSpec, UvSphereSpec, capsule, cone,
    cuboid, ico_sphere, plane, quad_sphere, torus, tube, uv_sphere,
};
use p3d_testdata::{scale_sizes, sphere};

#[path = "workloads/edit.rs"]
mod workloads;

fn scale(c: &mut Criterion) {
    let sizes = scale_sizes(4_000, 100_000);
    let mut g = c.benchmark_group("scale_prim_plane");
    for &tris in &sizes {
        let n = ((tris as f64 / 2.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::from_parameter(tris), &n, |b, &n| {
            b.iter(|| plane(black_box(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: n, segments_z: n })))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_quad_sphere");
    for &tris in &sizes {
        let n = ((tris as f64 / 12.0).sqrt().round()) as u32;
        g.bench_with_input(BenchmarkId::from_parameter(tris), &n, |b, &n| {
            b.iter(|| quad_sphere(black_box(&QuadSphereSpec { radius: 0.5, segments: n }), Origin::Center))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_box");
    for &tris in &sizes {
        let n = ((tris as f64 / 12.0).sqrt().round()) as u32;
        let spec = BoxSpec { size: DVec3::ONE, segments: [n; 3], bevel_width: 0.05, bevel_segments: 3 };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| cuboid(black_box(spec), Origin::Center)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_uv_sphere");
    for &tris in &sizes {
        let s = ((tris as f64).sqrt().round()) as u32;
        let spec = UvSphereSpec { radius: 0.5, segments: s, rings: (s / 2).max(2) };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| uv_sphere(black_box(spec), Origin::Center)));
    }
    g.finish();
    // Levels give exactly 4x steps; the parameter is the real triangle count.
    let mut g = c.benchmark_group("scale_prim_ico_sphere");
    let level = ((sizes[0] as f64 / 20.0).log(4.0).round().max(1.0)) as u32;
    for l in level..level + 3 {
        let spec = IcoSphereSpec { radius: 0.5, subdivisions: l };
        g.bench_with_input(BenchmarkId::from_parameter(20 * 4usize.pow(l)), &spec, |b, spec| b.iter(|| ico_sphere(black_box(spec), Origin::Center)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_cone");
    for &tris in &sizes {
        // Side 2 s x s / 2 and caps about s^2 / 8 triangles: s = sqrt(tris)
        // keeps both proportional to tris.
        let s = (((tris as f64).sqrt() / 4.0).round().max(1.0) as u32) * 4;
        let spec = ConeSpec { radius_bottom: 0.5, radius_top: 0.2, height: 1.0, segments: s, height_segments: s / 2, cap: CapKind::QuadGrid };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| cone(black_box(spec), Origin::Center)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_torus");
    for &tris in &sizes {
        let t = ((tris as f64 / 2.0).sqrt().round()) as u32;
        let spec = TorusSpec { major_radius: 0.5, minor_radius: 0.1, segments: t, sides: t };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| torus(black_box(spec), Origin::Center)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_tube");
    for &tris in &sizes {
        // 4 s (rows + 1) triangles with rows = s / 4.
        let s = ((tris as f64).sqrt().round()) as u32;
        let spec = TubeSpec { outer_radius: 0.5, inner_radius: 0.4, height: 1.0, segments: s, height_segments: (s / 4).max(1) };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| tube(black_box(spec), Origin::Center)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_prim_capsule");
    for &tris in &sizes {
        // Ends about s^2 / 4 triangles plus 2 s rows on the side, rows = s / 8.
        let s = (((tris as f64 * 2.0).sqrt() / 8.0).round().max(1.0) as u32) * 8;
        let spec = CapsuleSpec { radius: 0.2, height: 1.0, segments: s, height_segments: s / 8 };
        g.bench_with_input(BenchmarkId::from_parameter(tris), &spec, |b, spec| b.iter(|| capsule(black_box(spec), Origin::Center)));
    }
    g.finish();

    // Shading over whole meshes.
    let spheres: Vec<(usize, p3d_mesh::Mesh)> = sizes.iter().map(|&t| (t, sphere(t))).collect();
    let mut g = c.benchmark_group("scale_shade_sharp_by_angle");
    for (t, m) in &spheres {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| {
            b.iter_batched(|| m.clone(), |mut x| p3d_geo::shade::sharp_by_angle(&mut x, 30.0, true), BatchSize::LargeInput)
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_shade_weighted_normals");
    for (t, m) in &spheres {
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| {
            b.iter_batched(|| m.clone(), |mut x| p3d_geo::shade::weighted_normals(&mut x, p3d_geo::shade::Weighting::Area), BatchSize::LargeInput)
        });
    }
    g.finish();

    // Deformers over every vertex.
    let mut g = c.benchmark_group("scale_deform_bend");
    for (t, m) in &spheres {
        let all: Vec<u32> = m.verts().collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &all, |b, all| {
            b.iter_batched(
                || m.clone(),
                |mut x| p3d_geo::deform::bend(&mut x, all, p3d_geo::deform::DeformAxis::Y, p3d_geo::deform::DeformAxis::X, 1.0),
                BatchSize::LargeInput,
            )
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_deform_displace");
    for (t, m) in &spheres {
        let all: Vec<u32> = m.verts().collect();
        let spec = p3d_geo::deform::DisplaceSpec { strength: 0.01, scale: 0.1, seed: 1, octaves: 3 };
        g.bench_with_input(BenchmarkId::from_parameter(t), &all, |b, all| {
            b.iter_batched(|| m.clone(), |mut x| p3d_geo::deform::displace(&mut x, all, &spec), BatchSize::LargeInput)
        });
    }
    g.finish();

    // Element transform with proportional editing: a tenth of the vertices
    // selected, the rest within reach follow (straight distances, k-d tree).
    let mut g = c.benchmark_group("scale_edit_transform_proportional");
    for (t, m) in &spheres {
        let sel: Vec<u32> = m.verts().step_by(10).collect();
        let spec = p3d_geo::edit::TransformSpec {
            translate: Vec3::Y * 0.01,
            proportional: Some(p3d_geo::edit::Proportional { radius: 0.05, falloff: p3d_geo::edit::Falloff::Smooth, connected_only: false, seed: 0 }),
            ..p3d_geo::edit::TransformSpec::default()
        };
        g.bench_with_input(BenchmarkId::from_parameter(t), &sel, |b, sel| {
            b.iter_batched(|| m.clone(), |mut x| p3d_geo::edit::transform_verts(&mut x, sel, &spec), BatchSize::LargeInput)
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_edit_edge_slide");
    for (t, m) in &spheres {
        let edges: Vec<u32> = m.edges().step_by(7).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &edges, |b, edges| {
            b.iter_batched(|| m.clone(), |mut x| p3d_geo::edit::edge_slide(&mut x, edges, 0.3), BatchSize::LargeInput)
        });
    }
    g.finish();

    // One level of Catmull-Clark subdivision.
    let mut g = c.benchmark_group("scale_subd_catmull_clark");
    for (t, m) in &spheres {
        let spec = p3d_geo::subd::SubdSpec::default();
        g.bench_with_input(BenchmarkId::from_parameter(t), m, |b, m| b.iter(|| p3d_geo::subd::catmull_clark(black_box(m), &spec)));
    }
    g.finish();

    // A wavy lathe profile around 64 segments with quad-grid caps.
    let mut g = c.benchmark_group("scale_gen_lathe");
    for &t in &sizes {
        let n = (t / 128).max(2);
        let profile: Vec<glam::DVec2> = (0..n).map(|i| glam::DVec2::new(0.3 + 0.05 * (i as f64 * 0.3).sin(), i as f64 * 0.01)).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &profile, |b, profile| {
            let spec = p3d_geo::generate::LatheSpec {
                profile,
                spans: &[],
                closed: false,
                segments: 64,
                angle: std::f64::consts::TAU,
                cap: CapKind::QuadGrid,
                axis: p3d_geo::edit::Axis::Y,
            };
            b.iter(|| p3d_geo::generate::lathe(black_box(&spec)))
        });
    }
    g.finish();

    // A wavy outline with a hole, extruded with a round bevel on both rims.
    let mut g = c.benchmark_group("scale_gen_extrude_shape");
    for &t in &sizes {
        let ring = |n: usize, r: f64| -> Vec<glam::DVec2> {
            (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    glam::DVec2::new(a.cos(), a.sin()) * r * (1.0 + 0.05 * (a * 9.0).sin())
                })
                .collect()
        };
        let n = (t / 32).max(8);
        let contours = vec![ring(n, 1.0), ring(n / 2, 0.4)];
        g.bench_with_input(BenchmarkId::from_parameter(t), &contours, |b, contours| {
            let spec = p3d_geo::generate::ShapeSpec {
                contours,
                depth: 0.2,
                bevel: 0.01,
                bevel_segments: 3,
                bevel_front: true,
                bevel_back: true,
                axis: p3d_geo::edit::Axis::Z,
            };
            b.iter(|| p3d_geo::generate::extrude_shape(black_box(&spec)))
        });
    }
    g.finish();

    // A 16-sided tube along a helix with quad-grid caps.
    let mut g = c.benchmark_group("scale_gen_sweep");
    for &t in &sizes {
        let n = (t / 32).max(2);
        let path: Vec<glam::DVec3> = (0..n).map(|i| glam::DVec3::new((i as f64 * 0.05).cos(), i as f64 * 0.01, (i as f64 * 0.05).sin())).collect();
        let profile = p3d_geo::generate::round_profile(0.02, 16);
        g.bench_with_input(BenchmarkId::from_parameter(t), &path, |b, path| {
            let spec = p3d_geo::generate::SweepSpec {
                profile: &profile,
                profile_closed: true,
                profile_spans: &[],
                path,
                path_closed: false,
                twist: 0.0,
                scale: [1.0, 1.0],
                caps: p3d_geo::generate::SweepCaps::QuadGrid,
                up: glam::DVec3::Y,
            };
            b.iter(|| p3d_geo::generate::sweep(black_box(&spec)))
        });
    }
    g.finish();

    // Rings of 64 points stacked up, 4 smooth rows between sections, capped.
    let mut g = c.benchmark_group("scale_gen_loft");
    for &t in &sizes {
        let n = (t / 512).max(2);
        let sections: Vec<Vec<glam::DVec3>> = (0..n)
            .map(|i| {
                let r = 0.5 + 0.1 * (i as f64 * 0.7).sin();
                (0..64)
                    .map(|k| {
                        let a = std::f64::consts::TAU * k as f64 / 64.0;
                        glam::DVec3::new(a.cos() * r, i as f64 * 0.1, -a.sin() * r)
                    })
                    .collect()
            })
            .collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &sections, |b, sections| {
            let spec = p3d_geo::generate::LoftSpec { sections, sections_closed: true, points: 0, steps: 4, smooth: true, closed: false, caps: true };
            b.iter(|| p3d_geo::generate::loft(black_box(&spec)))
        });
    }
    g.finish();

    // Curve kernels on a wavy closed polyline of `t` points.
    let wave = |t: usize| -> Vec<Vec3> {
        (0..t)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / t as f32;
                Vec3::new(a.cos(), 0.0, -a.sin()) * (1.0 + 0.1 * (a * 7.0).sin())
            })
            .collect()
    };
    let mut g = c.benchmark_group("scale_curve_sample_bezier");
    for &t in &sizes {
        let pts = wave(t / 8);
        let h = p3d_geo::curve::auto_handles(&pts, true);
        g.bench_with_input(BenchmarkId::from_parameter(t), &(pts, h), |b, (pts, h)| {
            b.iter(|| p3d_geo::curve::sample_bezier(black_box(pts), h, true, 8))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_curve_resample");
    for &t in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(t), &wave(t), |b, pts| b.iter(|| p3d_geo::curve::resample(black_box(pts), true, t)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_curve_offset");
    for &t in &sizes {
        g.bench_with_input(BenchmarkId::from_parameter(t), &wave(t), |b, pts| {
            b.iter(|| p3d_geo::curve::offset(black_box(pts), true, 0.01, None, 4.0))
        });
    }
    g.finish();
    let mut g = c.benchmark_group("scale_curve_fillet");
    for &t in &sizes {
        let pts = wave(t / 4);
        g.bench_with_input(BenchmarkId::from_parameter(t), &pts, |b, pts| b.iter(|| p3d_geo::curve::fillet(black_box(pts), true, 0.01, 3)));
    }
    g.finish();

    // Edit kernels: the selection grows with the mesh.
    let edit_sets: Vec<(usize, Vec<_>)> = sizes.iter().map(|&t| (t, workloads::workloads(t))).collect();
    for i in 0..edit_sets[0].1.len() {
        let mut g = c.benchmark_group(format!("scale_edit_{}", edit_sets[0].1[i].0));
        for (t, sets) in &edit_sets {
            let (name, m, faces) = &sets[i];
            g.bench_with_input(BenchmarkId::from_parameter(t), &(m, faces), |b, (m, faces)| {
                b.iter_batched(|| (*m).clone(), |x| workloads::run(name, black_box(x), faces), BatchSize::LargeInput)
            });
        }
        g.finish();
    }

    // Tree queries start at 16k triangles: below that the whole tree sits in
    // L1 and L2 cache and the first step measures the cache, not the algorithm.
    let sizes = scale_sizes(16_000, 100_000);
    let meshes: Vec<(usize, TriBvh)> = sizes.iter().map(|&t| (t, TriBvh::from_mesh(&sphere(t), Affine3A::IDENTITY))).collect();
    let mut g = c.benchmark_group("scale_geo_bvh_build");
    for &t in &sizes {
        let m = sphere(t);
        g.bench_with_input(BenchmarkId::from_parameter(t), &m, |b, m| b.iter(|| TriBvh::from_mesh(black_box(m), Affine3A::IDENTITY)));
    }
    g.finish();
    // Closest points for the vertices of a nearby part: every 10th vertex of
    // the sphere, in vertex order, pushed up to one edge length off the
    // surface (contact scale). The cost is output-sensitive: O(log n + k) for
    // the k leaves within the found distance, so far points cost more and a
    // sphere center (every triangle equally far) is the O(n) worst case.
    let mut g = c.benchmark_group("scale_geo_bvh_closest_near");
    for (t, bvh) in &meshes {
        let h = (std::f32::consts::TAU / *t as f32).sqrt();
        let pts: Vec<Vec3> = sphere(*t).positions.iter().step_by(10).enumerate().map(|(i, p)| *p * (1.0 + h * (i as f32 * 2.1).sin())).collect();
        g.bench_with_input(BenchmarkId::from_parameter(t), &pts, |b, pts| b.iter(|| pts.iter().filter_map(|p| bvh.closest(*p, 1.0)).count()));
    }
    g.finish();
    // Two spheres half an edge length apart (contact scale, as for parts that
    // touch); output-sensitive like the closest-point query.
    let mut g = c.benchmark_group("scale_geo_bvh_nearest");
    for (t, bvh) in &meshes {
        let h = (std::f32::consts::TAU / *t as f32).sqrt();
        let other = TriBvh::from_mesh(&sphere(*t), Affine3A::from_translation(Vec3::new(1.0 + 0.5 * h, 0.0, 0.0)));
        g.bench_with_input(BenchmarkId::from_parameter(t), &other, |b, other| b.iter(|| bvh.nearest(black_box(other), 10.0)));
    }
    g.finish();
    let mut g = c.benchmark_group("scale_geo_bvh_self_crossings");
    for (t, bvh) in &meshes {
        g.bench_with_input(BenchmarkId::from_parameter(t), bvh, |b, bvh| {
            b.iter(|| {
                let mut n = 0u32;
                bvh.self_crossings(|_, _| {
                    n += 1;
                    true
                });
                n
            })
        });
    }
    g.finish();
}

criterion_group!(benches, scale);
criterion_main!(benches);
