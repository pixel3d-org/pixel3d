//! Ornament along paths: every motif is closed geometry on the surface's
//! outer side, counts and spacing hold, variation loosens the rhythm, and
//! random parameters keep clean topology.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use glam::Vec3;
use p3d_features::ornament::{Motif, OrnamentSpec, Outward, band, ornament};
use p3d_mesh::Mesh;
use proptest::prelude::*;

fn closed(m: &Mesh) -> bool {
    m.edges().all(|e| !m.is_boundary_edge(e))
}

fn ring(motif: Motif, radius: f32, size: f32) -> OrnamentSpec {
    OrnamentSpec {
        path: band(radius, Vec3::ZERO, Vec3::Y, 64),
        closed: true,
        outward: Outward::FromAxis { point: Vec3::ZERO, axis: Vec3::Y },
        motif,
        size,
        relief: size * 0.4,
        sink: size * 0.15,
        ..OrnamentSpec::default()
    }
}

#[test]
fn every_motif_is_closed_and_sits_on_the_surface() {
    for motif in Motif::ALL {
        let spec = ring(motif, 0.05, 0.012);
        let m = ornament(&spec).unwrap();
        assert_eq!(m.validate(), Ok(()), "{motif:?}");
        assert!(closed(&m), "{motif:?} is closed");
        assert!(m.group("ornament").is_some(), "{motif:?}");
        // Everything lies within the sink below and a little above the relief.
        for v in m.verts() {
            let p = m.positions[v as usize];
            let r = Vec3::new(p.x, 0.0, p.z).length();
            assert!(r > 0.05 - spec.size * 0.7 && r < 0.05 + spec.size * 1.1, "{motif:?}: {p} at radius {r}");
        }
    }
}

#[test]
fn counts_spacing_and_variation() {
    // Eight studs: eight closed pieces.
    let mut spec = ring(Motif::Stud, 0.05, 0.01);
    spec.count = 8;
    let m = ornament(&spec).unwrap();
    assert_eq!(m.face_components().1, 8);
    // Studs along an open edge, spaced 1.5 sizes apart by default.
    let edge = OrnamentSpec {
        path: vec![Vec3::ZERO, Vec3::new(0.3, 0.0, 0.0)],
        closed: false,
        outward: Outward::Along(Vec3::Y),
        size: 0.02,
        ..OrnamentSpec::default()
    };
    assert_eq!(ornament(&edge).unwrap().face_components().1, 10);
    // Regular by default, loosened by variation, deterministic per seed.
    let a = ornament(&edge).unwrap();
    let loose = OrnamentSpec { variation: 0.8, seed: 3, ..edge.clone() };
    let b = ornament(&loose).unwrap();
    assert_ne!(a.positions, b.positions);
    assert_eq!(b.positions, ornament(&loose).unwrap().positions);
    // Bad input names the problem.
    assert!(ornament(&OrnamentSpec { path: vec![Vec3::ZERO], ..edge.clone() }).is_err());
    assert!(ornament(&OrnamentSpec { size: 0.0, ..edge }).is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 24 } else { 96 }))]

    #[test]
    fn random_ornaments_keep_clean_topology(
        motif in 0usize..6, radius in 0.01f32..0.5, size_share in 0.05f32..0.6, count in 0u32..40,
        variation in 0.0f32..1.0, plies in 2u32..6, open in any::<bool>(), seed in 0u64..1000,
    ) {
        let size = radius * size_share;
        let mut spec = ring(Motif::ALL[motif], radius, size);
        spec.count = count;
        spec.variation = variation;
        spec.plies = plies;
        spec.seed = seed;
        if open {
            spec.path = vec![Vec3::ZERO, Vec3::new(radius * 3.0, 0.0, 0.0), Vec3::new(radius * 4.0, 0.0, radius)];
            spec.closed = false;
            spec.outward = Outward::Along(Vec3::Y);
        }
        let m = ornament(&spec).unwrap();
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert!(closed(&m));
        prop_assert!(m.positions.iter().all(|p| p.is_finite()));
    }
}
