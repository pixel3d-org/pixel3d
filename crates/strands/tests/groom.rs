//! Grooming genes (ADR 0011): braids with beads and ties, clump profiles,
//! clump layers, regions around a point and obstacles strands drape over.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use glam::{Affine3A, Vec3};
use p3d_geo::prim::{BoxSpec, Origin, QuadSphereSpec, cuboid, quad_sphere};
use p3d_mesh::Mesh;
use p3d_strands::{Braids, Kind, Output, Region, StrandSpec, Tie, generate, generate_around};
use proptest::prelude::*;

const R: f64 = 0.1;

fn head() -> Mesh {
    quad_sphere(&QuadSphereSpec { radius: R, segments: 16 }, Origin::Center).unwrap()
}

/// A chin patch: the front lower part of the sphere.
fn chin() -> Region {
    Region::Around { center: Vec3::new(0.0, -0.06, 0.08), radii: Vec3::new(0.07, 0.05, 0.06) }
}

fn closed(m: &Mesh) -> bool {
    m.edges().all(|e| !m.is_boundary_edge(e))
}

fn braided(count: u32, beads: u32, tie: Tie) -> StrandSpec {
    StrandSpec {
        region: chin(),
        clumps: 24,
        braids: Braids { count, beads, tie, length: 0.25, ..Braids::default() },
        ..StrandSpec::preset(Kind::Beard)
    }
}

#[test]
fn braids_hang_from_the_chin_with_beads_and_ties() {
    let s = generate(&head(), &braided(2, 3, Tie::Bead)).unwrap();
    assert_eq!(s.mesh.validate(), Ok(()));
    assert!(closed(&s.mesh), "braids, beads and ties are closed");
    assert_eq!((s.report.braids, s.report.beads), (2, 6));
    for g in ["clumps", "braids", "beads", "ties"] {
        assert!(s.mesh.group(g).is_some(), "group {g}");
    }
    // The braid guides hang down about their length and stay outside.
    let braids: Vec<_> = s.guides.iter().rev().take(2).collect();
    for g in braids {
        let drop = g.points[0].y - g.points.last().unwrap().y;
        assert!(drop > 0.12, "a braid hangs: {drop}");
    }
    assert_eq!(s.report.penetrating, 0);
    // The two braids start apart, one on each side of the chin.
    let n = s.guides.len();
    let (a, b) = (s.guides[n - 2].points[0], s.guides[n - 1].points[0]);
    assert!((a.x - b.x).abs() > 0.02, "{a} {b}");
    // Without braids the grooming fields change nothing.
    let plain = generate(&head(), &braided(0, 3, Tie::Bead)).unwrap();
    assert_eq!(plain.report.braids, 0);
    assert!(plain.mesh.group("braids").is_none());
    // Braids start where they are asked to.
    let at = Vec3::new(0.05, -0.05, 0.08);
    let spec = StrandSpec { braids: Braids { at: vec![at], length: 0.2, ..Braids::default() }, ..braided(0, 0, Tie::None) };
    let s = generate(&head(), &spec).unwrap();
    assert_eq!(s.report.braids, 1);
    assert!(s.guides.last().unwrap().points[0].distance(at) < 0.04);
}

#[test]
fn clump_profiles_layers_and_regions() {
    let m = head();
    let base = StrandSpec { region: chin(), clumps: 16, ..StrandSpec::preset(Kind::Beard) };
    let one = generate(&m, &base).unwrap();
    let three = generate(&m, &StrandSpec { clump_layers: 3, ..base.clone() }).unwrap();
    assert_eq!(three.report.guides, one.report.guides * 3);
    assert!(closed(&three.mesh));
    // A leaf-shaped clump is narrower at the root than in the middle.
    let leaf = generate(&m, &StrandSpec { root_width: 0.3, bulge: 0.4, flatness: 0.8, twist: 0.5, ..base.clone() }).unwrap();
    assert_eq!(leaf.mesh.validate(), Ok(()));
    assert!(closed(&leaf.mesh));
    // The region around a point is the faces near it.
    let faces = p3d_strands::region_faces(&m, &chin()).unwrap();
    assert!(!faces.is_empty() && faces.len() < m.face_count() / 4);
    assert!(faces.iter().all(|&f| m.face_centroid(f).z > 0.0));
    let e = p3d_strands::region_faces(&m, &Region::Around { center: Vec3::new(1.0, 0.0, 0.0), radii: Vec3::splat(0.01) }).unwrap_err();
    assert_eq!(e.param, "region");
    let e = generate(&m, &StrandSpec { bulge: 2.0, ..base }).unwrap_err();
    assert_eq!(e.param, "bulge");
}

#[test]
fn strands_drape_over_obstacles() {
    let m = head();
    // A slab under the chin: a beard falls onto it and stays above it.
    let mut chest = cuboid(&BoxSpec { size: glam::DVec3::new(0.4, 0.1, 0.4), ..BoxSpec::default() }, Origin::Center).unwrap();
    chest.transform(Affine3A::from_translation(Vec3::new(0.0, -0.2, 0.1)));
    let spec = StrandSpec { region: chin(), clumps: 20, length: 0.25, ..StrandSpec::preset(Kind::Beard) };
    let free = generate(&m, &spec).unwrap();
    let low = |s: &p3d_strands::Strands| s.guides.iter().flat_map(|g| g.points.iter()).map(|p| p.y).fold(f32::MAX, f32::min);
    assert!(low(&free) < -0.15, "without the slab the beard hangs past it");
    let draped = generate_around(&m, Some(&chest), &spec).unwrap();
    for g in &draped.guides {
        for p in &g.points {
            let inside = p.x.abs() < 0.2 && (p.y + 0.2).abs() < 0.05 && (p.z - 0.1).abs() < 0.2;
            assert!(!inside, "a strand point is inside the slab: {p}");
        }
    }
    assert_eq!(draped.mesh.validate(), Ok(()));
    let _ = Output::Clumps;
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 10 } else { 48 }))]

    #[test]
    fn braids_keep_clean_topology(
        count in 1u32..5, beads in 0u32..5, tie in 0usize..3, length in 0.05f32..0.5, width in 0.0f32..0.05,
        bead_size in 0.0f32..0.06, clumps in 6u32..40, segments in 2u32..16, seed in 0u64..1000,
    ) {
        let tie = [Tie::None, Tie::Band, Tie::Bead][tie];
        let spec = StrandSpec {
            region: chin(), clumps, segments, seed,
            braids: Braids { count, beads, tie, length, width, bead_size, at: Vec::new() },
            ..StrandSpec::preset(Kind::Beard)
        };
        let s = generate(&head(), &spec).unwrap();
        prop_assert_eq!(s.mesh.validate(), Ok(()));
        prop_assert!(closed(&s.mesh));
        prop_assert!(s.mesh.positions.iter().all(|p| p.is_finite()));
        prop_assert_eq!(s.report.beads, (s.report.braids as u32 * beads) as usize);
    }
}
