//! Property tests for the edit kernels on random meshes: the result is
//! always a valid mesh, and a refused edit changes nothing.

use p3d_geo::edit::{
    Axis, BevelSpec, ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, Keep, KernelError, MergeMode, PlaneCutSpec, QuadSpec, SmoothSpec, ThickenSpec,
    bevel, bridge, dissolve_edges, dissolve_limited, dissolve_verts, extrude, inset, loop_cut, merge, merge_by_distance, plane_cut, poke, smooth,
    split, subdivide, symmetrize, thicken, triangulate, tris_to_quads,
};
use p3d_mesh::Mesh;
use p3d_testdata::strategy;
use proptest::prelude::*;

fn pick_faces(m: &Mesh, picks: &[u32]) -> Vec<u32> {
    let live: Vec<u32> = m.faces().collect();
    let mut out: Vec<u32> = picks.iter().filter(|_| !live.is_empty()).map(|&p| live[p as usize % live.len()]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Valid after success, untouched after a refusal.
fn check(before: &Mesh, after: &Mesh, r: Result<(), KernelError>) -> Result<(), TestCaseError> {
    match r {
        Ok(()) => prop_assert_eq!(after.validate(), Ok(())),
        Err(_) => prop_assert_eq!(after, before),
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn extrusions_stay_valid(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12), individual in any::<bool>(), d in -0.5f32..0.5) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty() && d.abs() > 1e-3);
        let mode = if individual { ExtrudeMode::Individual } else { ExtrudeMode::Region };
        let mut n = m.clone();
        let r = extrude(&mut n, &faces, &ExtrudeSpec { mode, distance: d, direction: None });
        prop_assert!(r.is_ok(), "{:?}", r);
        if let Ok(r) = &r {
            prop_assert_eq!(r.cap.len(), faces.len());
            prop_assert_eq!(n.face_count(), m.face_count() + r.side.len());
        }
        check(&m, &n, r.map(|_| ()))?;
    }

    #[test]
    fn insets_stay_valid_or_change_nothing(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12), individual in any::<bool>(), t in 0.001f32..0.3, depth in -0.2f32..0.2) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty());
        let mode = if individual { InsetMode::Individual } else { InsetMode::Region };
        let mut n = m.clone();
        let r = inset(&mut n, &faces, &InsetSpec { mode, thickness: t, depth });
        if let Ok(r) = &r {
            prop_assert_eq!(r.inner.len(), faces.len());
            prop_assert_eq!(n.face_count(), m.face_count() + r.rim.len());
        }
        check(&m, &n, r.map(|_| ()))?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn poke_triangulate_and_requad(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12), offset in -0.1f32..0.1) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty());
        let mut n = m.clone();
        let tris = poke(&mut n, &faces, offset).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        let corners: usize = faces.iter().map(|&f| m.face_len(f)).sum();
        prop_assert_eq!(tris.len(), corners);

        let mut t = m.clone();
        let all: Vec<u32> = t.faces().collect();
        let (out, _) = triangulate(&mut t, &all).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(t.validate(), Ok(()));
        prop_assert!(t.faces().all(|f| t.face_len(f) == 3));
        prop_assert_eq!(out.len(), m.triangle_count());
        prop_assert_eq!(t.triangle_count(), m.triangle_count());

        let all: Vec<u32> = t.faces().collect();
        let quads = tris_to_quads(&mut t, &all, &QuadSpec::default()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(t.validate(), Ok(()));
        prop_assert_eq!(t.face_count(), m.triangle_count() - quads.len());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn split_and_subdivide(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12)) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty());
        let mut n = m.clone();
        let r = split(&mut n, &faces).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!(n.face_count(), m.face_count());
        // The unselected faces around a vertex can fall apart too (then the patch splits it).
        prop_assert!(n.vert_count() >= m.vert_count() + r.duplicated);
        // No selected face shares a vertex with an unselected one any more.
        let sel: std::collections::BTreeSet<u32> = r.faces.iter().copied().collect();
        for &f in &r.faces {
            for v in n.face_verts(f) {
                prop_assert!(n.vert_faces(v).all(|g| sel.contains(&g)));
            }
        }

        let mut s = m.clone();
        let r = subdivide(&mut s, &faces, 1).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(s.validate(), Ok(()));
        let corners: usize = faces.iter().map(|&f| m.face_len(f)).sum();
        prop_assert_eq!(r.faces.len(), corners);
        let area = |x: &Mesh| x.faces().map(|f| x.face_area(f)).sum::<f32>();
        prop_assert!((area(&s) - area(&m)).abs() <= 1e-3 * area(&m).max(1e-3), "{} vs {}", area(&s), area(&m));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn loop_cuts_stay_valid_or_change_nothing(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..3), cuts in 1u32..4, offset in -1.0f32..1.0) {
        let live: Vec<u32> = m.edges().collect();
        prop_assume!(!live.is_empty());
        let edges: Vec<u32> = picks.iter().map(|&p| live[p as usize % live.len()]).collect();
        let mut n = m.clone();
        match loop_cut(&mut n, &edges, cuts, offset) {
            Ok(r) => {
                prop_assert_eq!(n.validate(), Ok(()));
                prop_assert!(r.rings >= 1);
                prop_assert_eq!(r.faces.len() % (cuts as usize + 1), 0);
            }
            Err(_) => prop_assert_eq!(&n, &m),
        }
    }

    #[test]
    fn bridges_join_two_tubes(sa in 3u32..24, sb in 3u32..24, segments in 1u32..4, gap in 0.1f32..2.0) {
        use p3d_geo::prim::{CapKind, CylinderSpec, Origin, cylinder};
        let tube = |s: u32, y: f32| {
            let mut t = cylinder(&CylinderSpec { radius: 0.5, height: 1.0, segments: s, height_segments: 1, cap: CapKind::None }, Origin::Center).unwrap_or_default();
            t.transform(glam::Affine3A::from_translation(glam::Vec3::new(0.0, y, 0.0)));
            t
        };
        let mut m = tube(sa, 0.0);
        m.append(&tube(sb, 1.0 + gap));
        let hole = |m: &Mesh, y: f32| (0..m.he_slots() as u32).find(|&h| m.is_he_alive(h) && m.is_boundary_he(h) && (m.positions[m.origin(h) as usize].y - y).abs() < 1e-4);
        let (Some(a), Some(b)) = (hole(&m, 0.5), hole(&m, 0.5 + gap)) else { return Err(TestCaseError::fail("holes not found")) };
        let r = bridge(&mut m, a, b, segments).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert_eq!(m.face_components().1, 1);
        prop_assert_eq!(r.faces.len(), (sa.max(sb) * segments) as usize);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn merges_and_dissolves_stay_valid_or_change_nothing(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..8), kind in 0usize..6) {
        let mut n = m.clone();
        let verts: Vec<u32> = { let live: Vec<u32> = m.verts().collect(); picks.iter().map(|&p| live[p as usize % live.len().max(1)]).collect() };
        let edges: Vec<u32> = { let live: Vec<u32> = m.edges().collect(); picks.iter().map(|&p| live[p as usize % live.len().max(1)]).collect() };
        let faces = pick_faces(&m, &picks);
        let r: Result<(), KernelError> = match kind {
            0 => merge(&mut n, &verts, MergeMode::Center).map(|_| ()),
            1 => merge(&mut n, &verts, MergeMode::Collapse).map(|_| ()),
            2 => merge_by_distance(&mut n, None, 0.05).map(|_| ()),
            3 => dissolve_verts(&mut n, &verts).map(|_| ()),
            4 => dissolve_edges(&mut n, &edges, true).map(|_| ()),
            _ => dissolve_limited(&mut n, &faces, 10.0).map(|_| ()),
        };
        match r {
            Ok(()) => prop_assert_eq!(n.validate(), Ok(())),
            // Sequential dissolves may stop part way; single patches change nothing.
            Err(e) => {
                prop_assert_eq!(n.validate(), Ok(()));
                if kind <= 2 { prop_assert_eq!(&n, &m, "{}", e); }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn plane_cuts_symmetrize_thicken_and_smooth(m in strategy::mesh(), nx in -1.0f32..1.0, ny in -1.0f32..1.0, nz in -1.0f32..1.0, keep in 0usize..3, fill in any::<bool>(), axis in 0usize..3) {
        let normal = glam::Vec3::new(nx, ny, nz);
        prop_assume!(normal.length() > 0.1);
        let center = m.bounds().map_or(glam::Vec3::ZERO, |(lo, hi)| (lo + hi) / 2.0);
        let keep = [Keep::Both, Keep::Above, Keep::Below][keep];
        let mut c = m.clone();
        let r = plane_cut(&mut c, &PlaneCutSpec { point: center, normal, keep, fill }, None);
        prop_assert!(r.is_ok(), "{:?}", r);
        prop_assert_eq!(c.validate(), Ok(()));
        if keep != Keep::Both {
            let n = normal.normalize() * if keep == Keep::Above { 1.0 } else { -1.0 };
            let size = m.bounds().map_or(1.0, |(lo, hi)| (hi - lo).max_element());
            prop_assert!(c.verts().all(|v| (c.positions[v as usize] - center).dot(n) >= -1e-4 * size.max(1.0)));
        }

        let mut s = m.clone();
        let axis = [Axis::X, Axis::Y, Axis::Z][axis];
        if symmetrize(&mut s, axis, true, 1e-5).is_ok() {
            prop_assert_eq!(s.validate(), Ok(()));
        }

        let mut t = m.clone();
        let all: Vec<u32> = t.faces().collect();
        let r = thicken(&mut t, &all, &ThickenSpec { thickness: 0.01, offset: -1.0 });
        prop_assert!(r.is_ok(), "{:?}", r);
        prop_assert_eq!(t.validate(), Ok(()));
        prop_assert!(t.edges().all(|e| !t.is_boundary_edge(e)), "thickened surfaces are closed");

        let mut sm = m.clone();
        let verts: Vec<u32> = sm.verts().collect();
        smooth(&mut sm, &verts, &SmoothSpec::default()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(sm.validate(), Ok(()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn bevels_stay_valid_or_change_nothing(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..10), w in 0.001f32..0.2, segments in 1u32..5, profile in 0.0f32..1.0, clamp in any::<bool>()) {
        let live: Vec<u32> = m.edges().collect();
        prop_assume!(!live.is_empty());
        let edges: Vec<u32> = picks.iter().map(|&p| live[p as usize % live.len()]).collect();
        let mut n = m.clone();
        match bevel(&mut n, &edges, &BevelSpec { width: w, segments, profile, clamp }) {
            Ok(r) => {
                prop_assert_eq!(n.validate(), Ok(()));
                prop_assert!(r.width <= w);
                // Closed meshes stay closed.
                let open = |x: &Mesh| x.edges().filter(|&e| x.is_boundary_edge(e)).count();
                if open(&m) == 0 {
                    prop_assert_eq!(open(&n), 0);
                }
            }
            Err(_) => prop_assert_eq!(&n, &m),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn catmull_clark_stays_valid(m in strategy::mesh(), levels in 1u32..3, crease in 0.0f32..1.0, sharp in any::<bool>()) {
        use p3d_geo::subd::{SubdSpec, catmull_clark, subdivided_face_count};
        let mut c = m.clone();
        c.edge_crease = Some(vec![crease; c.edge_slots()]);
        let s = catmull_clark(&c, &SubdSpec { levels, keep_corners: true, sharp_as_crease: sharp }).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(s.validate(), Ok(()));
        prop_assert_eq!(s.face_count(), subdivided_face_count(&m, levels));
        prop_assert!(s.faces().all(|f| s.face_len(f) == 4));
        let euler = |x: &Mesh| x.vert_count() as i64 - x.edge_count() as i64 + x.face_count() as i64;
        prop_assert_eq!(euler(&s), euler(&m), "topology type is kept");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn deformers_keep_topology(m in strategy::mesh(), angle in -3.0f32..3.0, factor in 0.0f32..2.0, seed in any::<u64>()) {
        use p3d_geo::deform::{DeformAxis, DisplaceSpec, bend, displace, taper, twist};
        let all: Vec<u32> = m.verts().collect();
        let mut n = m.clone();
        twist(&mut n, &all, DeformAxis::Y, angle).map_err(|e| TestCaseError::fail(e.to_string()))?;
        taper(&mut n, &all, DeformAxis::X, factor).map_err(|e| TestCaseError::fail(e.to_string()))?;
        bend(&mut n, &all, DeformAxis::Z, DeformAxis::Y, angle).map_err(|e| TestCaseError::fail(e.to_string()))?;
        displace(&mut n, &all, &DisplaceSpec { strength: 0.01, scale: 0.1, seed, octaves: 2 }).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!((n.vert_count(), n.edge_count(), n.face_count()), (m.vert_count(), m.edge_count(), m.face_count()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn element_transforms_keep_topology(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..10), radius in 0.01f32..1.0, connected in any::<bool>(), factor in -1.0f32..1.0) {
        use p3d_geo::edit::{Falloff, Pivot, Proportional, TransformSpec, edge_slide, shrink_fatten, transform_verts};
        let live: Vec<u32> = m.verts().collect();
        let verts: Vec<u32> = picks.iter().map(|&p| live[p as usize % live.len()]).collect();
        let mut n = m.clone();
        let spec = TransformSpec {
            translate: glam::Vec3::new(0.1, 0.2, -0.1),
            rotate: glam::Quat::from_rotation_y(0.4),
            scale: glam::Vec3::splat(0.8),
            pivot: Pivot::Individual,
            proportional: Some(Proportional { radius, falloff: Falloff::Smooth, connected_only: connected, seed: 1 }),
            ..TransformSpec::default()
        };
        transform_verts(&mut n, &verts, &spec).map_err(|e| TestCaseError::fail(e.to_string()))?;
        shrink_fatten(&mut n, &verts, 0.01, true).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let edges: Vec<u32> = { let le: Vec<u32> = n.edges().collect(); picks.iter().map(|&p| le[p as usize % le.len()]).collect() };
        edge_slide(&mut n, &edges, factor).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!((n.vert_count(), n.edge_count(), n.face_count()), (m.vert_count(), m.edge_count(), m.face_count()));
    }
}
