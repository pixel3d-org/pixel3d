//! proptest strategies for random valid meshes.

use glam::DVec3;
use p3d_geo::prim::{BoxSpec, CapKind, CylinderSpec, Origin, PlaneSpec, QuadSphereSpec, cuboid, cylinder, plane, quad_sphere};
use p3d_mesh::Mesh;
use proptest::prelude::*;

/// A random primitive with random (small) parameters, always valid.
pub fn primitive() -> impl Strategy<Value = Mesh> {
    prop_oneof![
        (1u32..12, 1u32..12, 0.01f64..10.0, 0.01f64..10.0)
            .prop_map(|(sx, sz, x, z)| { plane(&PlaneSpec { size_x: x, size_z: z, segments_x: sx, segments_z: sz }).unwrap_or_default() }),
        (prop::array::uniform3(1u32..5), prop::array::uniform3(0.05f64..3.0), 0.0f64..0.45, 0u32..6).prop_map(|(seg, size, bevel, bs)| {
            let size = DVec3::from(size);
            let spec = BoxSpec { size, segments: seg, bevel_width: bevel * size.min_element(), bevel_segments: bs };
            cuboid(&spec, Origin::Bottom).unwrap_or_default()
        }),
        (1u32..17, 1u32..4, 0.01f64..2.0, 0.01f64..2.0, 0usize..4).prop_map(|(quarter, rows, r, h, cap)| {
            let cap = [CapKind::QuadGrid, CapKind::Ngon, CapKind::Fan, CapKind::None][cap];
            let segments = if cap == CapKind::QuadGrid { 4 * quarter } else { 2 + quarter };
            cylinder(&CylinderSpec { radius: r, height: h, segments, height_segments: rows, cap }, Origin::Center).unwrap_or_default()
        }),
        (1u32..8, 0.01f64..5.0).prop_map(|(n, r)| quad_sphere(&QuadSphereSpec { radius: r, segments: n }, Origin::Center).unwrap_or_default()),
    ]
}

/// A random primitive with a random subset of its faces deleted (and,
/// sometimes, compacted): exercises boundaries, split vertices and free slots.
pub fn mesh() -> impl Strategy<Value = Mesh> {
    (primitive(), prop::collection::vec(any::<u32>(), 0..12), any::<bool>()).prop_map(|(mut m, picks, compact)| {
        for p in picks {
            let live: Vec<u32> = m.faces().collect();
            if live.len() <= 1 {
                break;
            }
            let f = live[p as usize % live.len()];
            let _ = m.delete_faces(&[f]);
        }
        if compact {
            m.compact();
        }
        m
    })
}
