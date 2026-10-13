//! Property tests for the generators on random profiles and shapes.

use glam::{DQuat, DVec2, DVec3};
use p3d_geo::edit::Axis;
use p3d_geo::generate::{LatheSpec, LoftSpec, ShapeSpec, SweepCaps, SweepSpec, extrude_shape, lathe, loft, round_profile, sweep};
use p3d_geo::prim::{CapKind, PrimError};
use p3d_mesh::Mesh;
use proptest::prelude::*;

/// Valid, closed when `closed`, faces pointing out (positive signed volume).
fn check(m: &Mesh, closed: bool) -> Result<f32, TestCaseError> {
    prop_assert_eq!(m.validate(), Ok(()));
    if closed {
        prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
    }
    let vol: f32 = m.faces().map(|f| m.face_normal(f).dot(m.face_centroid(f)) * m.face_area(f)).sum::<f32>() / 3.0;
    if closed {
        prop_assert!(vol > 0.0, "faces point inward ({})", vol);
    }
    Ok(vol)
}

/// A star polygon around the origin with radii in `lo..hi` and at least
/// `min` points (6 points of radius 0.5 keep everything within 0.43 inside).
fn star(lo: f64, hi: f64, min: usize) -> impl Strategy<Value = Vec<DVec2>> {
    prop::collection::vec(lo..hi, min..24).prop_map(|radii| {
        let n = radii.len();
        radii
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                DVec2::new(a.cos() * r, a.sin() * r)
            })
            .collect()
    })
}

fn cap() -> impl Strategy<Value = CapKind> {
    prop_oneof![Just(CapKind::QuadGrid), Just(CapKind::Ngon), Just(CapKind::Fan), Just(CapKind::None)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn lathes_of_rising_profiles(
        radii in prop::collection::vec(0.05f64..1.0, 2..20),
        pole_start in any::<bool>(),
        pole_end in any::<bool>(),
        quarter in 1u32..12,
        cap in cap(),
        down in any::<bool>(),
    ) {
        let mut profile: Vec<DVec2> = radii.iter().enumerate().map(|(i, r)| DVec2::new(*r, i as f64 * 0.1)).collect();
        let n = profile.len();
        if pole_start && n > 2 { profile[0].x = 0.0; }
        if pole_end && n > 2 { profile[n - 1].x = 0.0; }
        if down { profile.reverse(); }
        let spec = LatheSpec { profile: &profile, spans: &[], closed: false, segments: 4 * quarter, angle: std::f64::consts::TAU, cap, axis: Axis::Y };
        let m = lathe(&spec).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let ends_closed = (profile[0].x == 0.0 || cap != CapKind::None) && (profile[n - 1].x == 0.0 || cap != CapKind::None);
        check(&m, ends_closed)?;
        prop_assert_eq!(m.group("side").map(|g| g.count()), Some((n - 1) * 4 * quarter as usize));
    }

    #[test]
    fn extruded_stars_with_holes(
        outer in star(0.5, 1.0, 6),
        hole in prop::option::of(star(0.1, 0.3, 3)),
        flip in any::<bool>(),
        depth in 0.05f64..1.0,
        bevel in prop::option::of((0.001f64..0.02, 1u32..5)),
    ) {
        let mut contours = vec![outer];
        if let Some(h) = hole { contours.push(h); }
        if flip { for c in &mut contours { c.reverse(); } }
        let (w, segs) = bevel.unwrap_or((0.0, 1));
        let spec = ShapeSpec { contours: &contours, depth, bevel: w, bevel_segments: segs, bevel_front: true, bevel_back: true, axis: Axis::Z };
        match extrude_shape(&spec) {
            Ok(m) => {
                let vol = check(&m, true)?;
                let euler = m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64;
                prop_assert_eq!(euler, if contours.len() == 2 { 0 } else { 2 });
                if w == 0.0 {
                    let area = |c: &[DVec2]| (0..c.len()).map(|i| c[i].perp_dot(c[(i + 1) % c.len()])).sum::<f64>().abs() / 2.0;
                    let expect = depth * (area(&contours[0]) - contours.get(1).map_or(0.0, |h| area(h)));
                    prop_assert!((f64::from(vol) - expect).abs() < 1e-4 * expect.max(1.0), "{} vs {}", vol, expect);
                }
            }
            // Spiky stars can be too narrow for a bevel; anything else is a bug.
            Err(PrimError::InvalidParam { param: "bevel", .. }) if w > 0.0 => {}
            Err(e) => return Err(TestCaseError::fail(e.to_string())),
        }
    }
}

/// A wandering path: steps of 0.5 to 1 m turning at most 50 degrees.
fn wander() -> impl Strategy<Value = Vec<DVec3>> {
    prop::collection::vec((0.5f64..1.0, -0.87f64..0.87, -0.87f64..0.87), 1..20).prop_map(|steps| {
        let (mut p, mut d) = (DVec3::ZERO, DVec3::Y);
        let mut out = vec![p];
        for (len, yaw, pitch) in steps {
            let side = d.any_orthonormal_vector();
            d = (DQuat::from_axis_angle(side, pitch) * DQuat::from_axis_angle(d.cross(side).normalize(), yaw) * d).normalize();
            p += d * len;
            out.push(p);
        }
        out
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn sweeps_along_wandering_paths(
        path in wander(),
        star in star(0.03, 0.08, 3),
        round in any::<bool>(),
        twist in -6.0f64..6.0,
        taper in 0.2f64..2.0,
        caps in any::<bool>(),
    ) {
        let profile = if round { round_profile(0.06, 12) } else { star };
        let caps = match (caps, round) {
            (false, _) => SweepCaps::None,
            (true, true) => SweepCaps::QuadGrid,
            (true, false) => SweepCaps::Fill,
        };
        let spec = SweepSpec {
            profile: &profile,
            profile_closed: true,
            profile_spans: &[],
            path: &path,
            path_closed: false,
            twist,
            scale: [1.0, taper],
            caps,
            up: DVec3::Y,
        };
        let m = sweep(&spec).map_err(|e| TestCaseError::fail(e.to_string()))?;
        check(&m, caps != SweepCaps::None)?;
        prop_assert_eq!(m.group("side").map(|g| g.count()), Some((path.len() - 1) * profile.len()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn lofts_through_stacked_stars(
        stars in prop::collection::vec((star(0.3, 1.0, 3), any::<bool>(), -1.0f64..1.0), 2..6),
        points in prop::option::of(8usize..40),
        steps in 1u32..5,
        smooth in any::<bool>(),
    ) {
        // Star sections stacked 0.5 m apart, some wound the other way, each turned a little.
        let sections: Vec<Vec<DVec3>> = stars
            .iter()
            .enumerate()
            .map(|(i, (s, flip, turn))| {
                let mut ring: Vec<DVec3> = s.iter().map(|p| {
                    let q = DVec2::from_angle(*turn).rotate(*p);
                    DVec3::new(q.x, i as f64 * 0.5, -q.y)
                }).collect();
                if *flip { ring.reverse(); }
                ring
            })
            .collect();
        let spec = LoftSpec { sections: &sections, sections_closed: true, points: points.unwrap_or(24), steps, smooth, closed: false, caps: true };
        let m = loft(&spec).map_err(|e| TestCaseError::fail(e.to_string()))?;
        check(&m, true)?;
    }
}
