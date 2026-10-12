//! Property tests for the primitive generators over their parameter ranges.

use glam::DVec3;
use p3d_geo::prim::{BoxSpec, CapKind, ConeSpec, CylinderSpec, Origin, QuadSphereSpec, cone, cuboid, cylinder, quad_sphere};
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
