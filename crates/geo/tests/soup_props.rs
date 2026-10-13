//! Property tests for soup repair: meshes taken apart the way files store
//! them (a vertex per corner, faces flipped, repeated, with extra faces on
//! edges) and random triangle soup always repair into a valid mesh, and a
//! clean mesh comes back with all its faces.

use glam::Vec3;
use p3d_geo::soup::{SoupSpec, repair};
use p3d_mesh::Mesh;
use p3d_testdata::strategy;
use proptest::prelude::*;

/// The mesh as soup with a separate vertex for every corner.
fn split(m: &Mesh) -> (Vec<Vec3>, Vec<u32>, Vec<Vec<u32>>) {
    let mut pos = Vec::new();
    let mut faces = Vec::new();
    for f in m.faces() {
        let mut face = Vec::new();
        for v in m.face_verts(f) {
            face.push(pos.len() as u32);
            pos.push(m.positions[v as usize]);
        }
        faces.push(face);
    }
    let sizes = faces.iter().map(|f| f.len() as u32).collect();
    (pos, sizes, faces)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn split_meshes_come_back_whole(m in strategy::mesh()) {
        let (pos, sizes, faces) = split(&m);
        let corners: Vec<u32> = faces.concat();
        let r = repair(&pos, &sizes, &corners, &SoupSpec::default()).unwrap();
        prop_assert_eq!(r.mesh.validate(), Ok(()));
        prop_assert_eq!(r.mesh.face_count(), m.face_count());
        prop_assert_eq!(r.report.flipped + r.report.duplicates + r.report.detached, 0, "{:?}", r.report);
    }

    #[test]
    fn broken_soup_always_repairs(
        m in strategy::mesh(),
        flips in prop::collection::vec(any::<u32>(), 0..8),
        repeats in prop::collection::vec(any::<u32>(), 0..4),
        extra in prop::collection::vec((any::<u32>(), any::<u32>()), 0..4),
    ) {
        let (mut pos, _, mut faces) = split(&m);
        let n = faces.len();
        for f in flips {
            faces[f as usize % n].reverse();
        }
        for f in repeats {
            let copy = faces[f as usize % n].clone();
            faces.push(copy);
        }
        // Extra triangles on existing edges (books) and across faces.
        for (a, b) in extra {
            let fa = &faces[a as usize % n];
            let (u, v) = (fa[0], fa[1 % fa.len()]);
            pos.push(pos[u as usize] + Vec3::new(0.0, 0.3, 0.1) * (b % 5) as f32);
            faces.push(vec![u, v, pos.len() as u32 - 1]);
        }
        let sizes: Vec<u32> = faces.iter().map(|f| f.len() as u32).collect();
        let corners: Vec<u32> = faces.concat();
        let r = repair(&pos, &sizes, &corners, &SoupSpec::default());
        prop_assert!(r.is_ok(), "{:?}", r.err());
        let r = r.unwrap_or_default();
        prop_assert_eq!(r.mesh.validate(), Ok(()));
        prop_assert_eq!(r.corner_src.len(), r.mesh.faces().map(|f| r.mesh.face_len(f)).sum::<usize>());
        prop_assert_eq!(r.face_src.len(), r.mesh.face_count());
    }

    #[test]
    fn random_triangles_repair(
        points in prop::collection::vec(prop::array::uniform3(0u8..4), 4..24),
        tris in prop::collection::vec(prop::array::uniform3(any::<u8>()), 1..60),
    ) {
        let pos: Vec<Vec3> = points.iter().map(|p| Vec3::new(f32::from(p[0]), f32::from(p[1]), f32::from(p[2]))).collect();
        let corners: Vec<u32> = tris.iter().flat_map(|t| t.map(|i| u32::from(i) % pos.len() as u32)).collect();
        let sizes = vec![3; tris.len()];
        match repair(&pos, &sizes, &corners, &SoupSpec::default()) {
            Ok(r) => prop_assert_eq!(r.mesh.validate(), Ok(())),
            Err(e) => prop_assert_eq!(e, p3d_geo::soup::SoupError::Empty),
        }
    }
}
