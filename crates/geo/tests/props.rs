//! Property tests for the primitive generators over their parameter ranges.

use glam::DVec3;
use p3d_geo::prim::{
    BoxSpec, CapKind, CapsuleSpec, ConeSpec, CylinderSpec, IcoSphereSpec, Origin, QuadSphereSpec, TorusSpec, TubeSpec, UvSphereSpec, WedgeSpec,
    capsule, cone, cuboid, cylinder, ico_sphere, quad_sphere, torus, tube, uv_sphere, wedge,
};
use p3d_mesh::Mesh;
use proptest::prelude::*;

/// Closed, genus 0, every face pointing away from the bounding box center.
fn assert_closed_outward(m: &Mesh) -> Result<(), TestCaseError> {
    prop_assert_eq!(m.validate(), Ok(()));
    prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
    prop_assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
    let (lo, hi) = m.bounds().ok_or_else(|| TestCaseError::fail("empty mesh"))?;
    let c = (lo + hi) / 2.0;
    for f in m.faces() {
        prop_assert!(m.face_normal(f).dot(m.face_centroid(f) - c) > 0.0, "face {} points inward", f);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn boxes(seg in prop::array::uniform3(1u32..6), size in prop::array::uniform3(0.01f64..5.0), bevel in 0.0f64..0.45, bs in 0u32..9) {
        let size = DVec3::from(size);
        let spec = BoxSpec { size, segments: seg, bevel_width: bevel * size.min_element(), bevel_segments: bs };
        let m = cuboid(&spec, Origin::Center).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        let (lo, hi) = m.bounds().ok_or_else(|| TestCaseError::fail("empty"))?;
        prop_assert!((hi - lo).as_dvec3().abs_diff_eq(size, 1e-5 * size.max_element()), "size {} vs {}", hi - lo, size);
        let tris = m.faces().filter(|&f| m.face_len(f) == 3).count();
        let beveled = spec.bevel_width > 0.0 && bs > 0;
        prop_assert_eq!(tris, if beveled && bs % 2 == 1 { 8 } else { 0 });
    }

    #[test]
    fn closed_cylinders(quarter in 1u32..20, rows in 1u32..5, r in 0.01f64..3.0, h in 0.01f64..3.0, cap in 0usize..3) {
        let cap = [CapKind::QuadGrid, CapKind::Ngon, CapKind::Fan][cap];
        let segments = if cap == CapKind::QuadGrid { 4 * quarter } else { 2 + quarter };
        let m = cylinder(&CylinderSpec { radius: r, height: h, segments, height_segments: rows, cap }, Origin::Bottom)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert_eq!(m.group("side").map(|g| g.count()), Some((segments * rows) as usize));
    }

    #[test]
    fn spheres(n in 1u32..24, r in 0.01f64..10.0) {
        let m = quad_sphere(&QuadSphereSpec { radius: r, segments: n }, Origin::Center).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert!(m.verts().all(|v| ((m.positions[v as usize].length() as f64) - r).abs() < 1e-5 * r.max(1.0)));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn cones_and_frustums(quarter in 1u32..12, rows in 1u32..5, rb in 0.0f64..2.0, rt in 0.0f64..2.0, point in 0usize..3, h in 0.01f64..3.0, cap in 0usize..3) {
        let cap = [CapKind::QuadGrid, CapKind::Ngon, CapKind::Fan][cap];
        let segments = if cap == CapKind::QuadGrid { 4 * quarter } else { 2 + quarter };
        // Sometimes a pointed end (bottom or top), otherwise a frustum.
        let (rb, rt) = match point { 0 => (0.0, rt + 0.01), 1 => (rb + 0.01, 0.0), _ => (rb + 0.01, rt + 0.01) };
        let spec = ConeSpec { radius_bottom: rb, radius_top: rt, height: h, segments, height_segments: rows, cap };
        let m = cone(&spec, Origin::Bottom).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert_eq!(m.group("side").map(|g| g.count()), Some((segments * rows) as usize));
        prop_assert_eq!(m.group("cap_top").is_some(), rt > 0.0);
        prop_assert_eq!(m.group("cap_bottom").is_some(), rb > 0.0);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn uv_spheres(s in 3u32..40, n in 2u32..24, r in 0.01f64..10.0) {
        let m = uv_sphere(&UvSphereSpec { radius: r, segments: s, rings: n }, Origin::Center).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert!(m.verts().all(|v| ((m.positions[v as usize].length() as f64) - r).abs() < 1e-5 * r.max(1.0)));
    }

    #[test]
    fn ico_spheres(level in 0u32..5, r in 0.01f64..10.0) {
        let m = ico_sphere(&IcoSphereSpec { radius: r, subdivisions: level }, Origin::Bottom).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert_eq!(m.face_count(), 20 * 4usize.pow(level));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn tori(s in 8u32..48, t in 3u32..24, big in 0.05f64..5.0, ratio in 0.05f64..0.95) {
        let spec = TorusSpec { major_radius: big, minor_radius: big * ratio, segments: s, sides: t };
        let m = torus(&spec, Origin::Center).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 0);
        prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        prop_assert!(m.faces().all(|f| m.face_len(f) == 4));
    }

    #[test]
    fn tubes(s in 3u32..48, rows in 1u32..5, outer in 0.01f64..3.0, ratio in 0.05f64..0.95, h in 0.01f64..3.0) {
        let spec = TubeSpec { outer_radius: outer, inner_radius: outer * ratio, height: h, segments: s, height_segments: rows };
        let m = tube(&spec, Origin::Bottom).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 0);
        prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // Walls face away from the solid: outward on the outer wall, toward the axis inside.
        for f in m.faces() {
            let c = m.face_centroid(f);
            let radial = glam::Vec3::new(c.x, 0.0, c.z);
            let n = m.face_normal(f);
            if m.group("inner").is_some_and(|g| g.contains(f)) {
                prop_assert!(n.dot(radial) < 0.0);
            } else if m.group("outer").is_some_and(|g| g.contains(f)) {
                prop_assert!(n.dot(radial) > 0.0);
            }
        }
    }

    #[test]
    fn capsules(eighth in 1u32..8, rows in 1u32..4, r in 0.01f64..2.0, extra in 0.001f64..3.0) {
        let spec = CapsuleSpec { radius: r, height: 2.0 * r + extra, segments: 8 * eighth, height_segments: rows };
        let m = capsule(&spec, Origin::Center).map_err(|e| TestCaseError::fail(e.to_string()))?;
        assert_closed_outward(&m)?;
        prop_assert!(m.faces().all(|f| m.face_len(f) == 4));
        let (lo, hi) = m.bounds().ok_or_else(|| TestCaseError::fail("empty"))?;
        prop_assert!((f64::from(hi.y - lo.y) - spec.height).abs() < 1e-5 * spec.height.max(1.0));
    }

    #[test]
    fn wedges(size in prop::array::uniform3(0.01f64..5.0)) {
        let m = wedge(&WedgeSpec { size: DVec3::from(size) }, Origin::Bottom).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        let mean = m.verts().map(|v| m.positions[v as usize]).sum::<glam::Vec3>() / 6.0;
        prop_assert!(m.faces().all(|f| m.face_normal(f).dot(m.face_centroid(f) - mean) > 0.0));
    }
}
