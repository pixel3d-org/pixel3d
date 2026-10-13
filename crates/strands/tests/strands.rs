//! Strands on a sphere "head": every output is valid, nothing penetrates,
//! cards cover the region, results are deterministic, and random
//! parameters keep these properties.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use glam::Vec3;
use p3d_geo::prim::{Origin, QuadSphereSpec, quad_sphere};
use p3d_mesh::Mesh;
use p3d_strands::{Kind, Output, Region, StrandSpec, Style, generate};
use proptest::prelude::*;

const R: f64 = 0.1;

fn head() -> Mesh {
    quad_sphere(&QuadSphereSpec { radius: R, segments: 16 }, Origin::Center).unwrap()
}

/// A ring at 30 degrees above the equator: the scalp cap above it.
fn scalp() -> Region {
    let (r, y) = (R as f32 * 0.866, R as f32 * 0.5);
    let points = (0..8).map(|i| {
        let a = i as f32 / 8.0 * std::f32::consts::TAU;
        Vec3::new(a.cos() * r, y, a.sin() * r)
    });
    Region::Loop { points: points.collect(), inside: None }
}

fn outside(s: &p3d_strands::Strands) {
    for g in &s.guides {
        for p in &g.points {
            assert!(p.length() > R as f32 * 0.995, "inside the head: {p} ({})", p.length());
        }
    }
    assert_eq!(s.report.penetrating, 0);
}

#[test]
fn hair_cards_cover_the_scalp_without_penetrating() {
    let spec = StrandSpec { region: scalp(), ..StrandSpec::preset(Kind::Hair) };
    let s = generate(&head(), &spec).unwrap();
    assert_eq!(s.mesh.validate(), Ok(()));
    outside(&s);
    let names: Vec<&str> = s.mesh.groups().map(|(n, _)| n).collect();
    for g in ["cap", "cards.0", "cards.1", "flyaways"] {
        assert!(names.contains(&g), "{g} in {names:?}");
    }
    assert_eq!(s.mesh.uv_layer_count(), 1);
    let c = s.report.coverage.unwrap();
    assert!(c > 0.85, "coverage {c}, bald spot {:?}", s.report.bald_spot);
    assert!(s.report.overdraw.unwrap() > 1.0);
    // Swept back: tips end behind and below their roots on average.
    let dz: f32 = s.guides.iter().map(|g| g.points.last().unwrap().z - g.points[0].z).sum::<f32>() / s.guides.len() as f32;
    let dy: f32 = s.guides.iter().map(|g| g.points.last().unwrap().y - g.points[0].y).sum::<f32>() / s.guides.len() as f32;
    assert!(dz < -0.02 && dy < 0.0, "dz {dz} dy {dy}");
    // Cards face out: their normals point away from the head.
    let normals = s.mesh.face_normals();
    let cards = s.mesh.group("cards.0").unwrap();
    let out = cards.iter().filter(|&f| normals[f as usize].dot(s.mesh.face_centroid(f).normalize()) > 0.0).count();
    assert!(out * 10 >= cards.count() * 9, "{out} of {} face out", cards.count());
}

#[test]
fn clumps_are_closed_and_shells_stack() {
    let spec = StrandSpec { region: scalp(), output: Output::Clumps, curl: 0.4, ..StrandSpec::preset(Kind::Hair) };
    let s = generate(&head(), &spec).unwrap();
    assert_eq!(s.mesh.validate(), Ok(()));
    assert!(s.mesh.edges().all(|e| !s.mesh.is_boundary_edge(e)), "clumps are closed");
    outside(&s);
    assert!(s.report.coverage.unwrap() > 0.5);
    let fur = StrandSpec { region: Region::All, ..StrandSpec::preset(Kind::Fur) };
    let m = head();
    let s = generate(&m, &fur).unwrap();
    assert_eq!(s.mesh.face_count(), m.face_count() * fur.shells as usize);
    assert!(s.mesh.group("shell_7").is_some() && s.report.coverage.is_none());
    let (lo, hi) = s.mesh.bounds().unwrap();
    assert!(((hi.x - lo.x) * 0.5 - (R as f32 + fur.length)).abs() < 0.002, "outer shell at radius plus length");
}

#[test]
fn styles_point_where_they_say() {
    let m = head();
    // Eyebrows combed outward: both sides move away from x = 0.
    let brows = StrandSpec { region: scalp(), clumps: 40, ..StrandSpec::preset(Kind::Eyebrows) };
    let s = generate(&m, &brows).unwrap();
    for g in s.guides.iter().filter(|g| g.points[0].x.abs() > 0.03) {
        let dx = g.points.last().unwrap().x - g.points[0].x;
        assert!(dx * g.points[0].x.signum() > 0.0, "outward: {:?}", g.points[0]);
    }
    // Spiky hair stands out along the normal.
    let spiky = StrandSpec { region: scalp(), style: Style::Spiky, gravity: 0.0, noise: 0.0, ..StrandSpec::preset(Kind::Hair) };
    let s = generate(&m, &spiky).unwrap();
    for g in &s.guides {
        let (a, b) = (g.points[0], *g.points.last().unwrap());
        assert!((b - a).normalize().dot(a.normalize()) > 0.9, "{a} -> {b}");
    }
    // A ponytail gathers at its anchor at the back.
    let anchor = Vec3::new(0.0, 0.05, -0.0866);
    let tail =
        StrandSpec { region: scalp(), style: Style::Ponytail(anchor), length: 0.3, segments: 24, gravity: 0.0, ..StrandSpec::preset(Kind::Hair) };
    let s = generate(&m, &tail).unwrap();
    let near = s.guides.iter().filter(|g| g.points.iter().any(|p| p.distance(anchor) < 0.03)).count();
    assert!(near * 10 >= s.guides.len() * 7, "{near} of {} pass the anchor", s.guides.len());
}

#[test]
fn strands_are_deterministic_and_errors_name_the_parameter() {
    let m = head();
    let spec = StrandSpec { region: scalp(), seed: 7, ..StrandSpec::preset(Kind::Beard) };
    let a = generate(&m, &spec).unwrap();
    let b = generate(&m, &spec).unwrap();
    assert_eq!(a.mesh.positions, b.mesh.positions);
    let c = generate(&m, &StrandSpec { seed: 8, ..spec.clone() }).unwrap();
    assert_ne!(a.mesh.positions, c.mesh.positions);
    let e = generate(&m, &StrandSpec { clumps: 0, ..spec.clone() }).unwrap_err();
    assert_eq!(e.param, "clumps");
    let e = generate(&m, &StrandSpec { region: Region::Groups(vec!["scalp".into()]), ..spec }).unwrap_err();
    assert_eq!(e.param, "region");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 12 } else { 64 }))]

    #[test]
    fn random_parameters_stay_valid_and_outside(
        kind in 0usize..5, out in 0usize..3, style in 0usize..6, clumps in 4u32..80, segments in 2u32..16,
        gravity in 0.0f32..1.0, curl in 0.0f32..1.0, noise in 0.0f32..1.0, length in 0.005f32..0.3, seed in 0u64..1000,
    ) {
        let style = [Style::SweptBack, Style::Parted(0.01), Style::Spiky, Style::Straight, Style::Upright, Style::WindBlown(Vec3::X)][style];
        let output = [Output::Cards, Output::Clumps, Output::Shells][out];
        let spec = StrandSpec {
            region: scalp(), style, output, clumps, segments, gravity, curl, noise, length, seed,
            ..StrandSpec::preset(Kind::ALL[kind])
        };
        let s = generate(&head(), &spec).unwrap();
        prop_assert_eq!(s.mesh.validate(), Ok(()));
        prop_assert_eq!(s.report.penetrating, 0);
        prop_assert!(s.mesh.positions.iter().all(|p| p.is_finite()));
        if output == Output::Clumps {
            prop_assert!(s.mesh.edges().all(|e| !s.mesh.is_boundary_edge(e)));
        }
    }
}
