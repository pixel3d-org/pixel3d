//! Property tests on random valid meshes (primitives with random parameters
//! and random face deletions).

use p3d_mesh::{Mesh, TriOptions, Vec3};
use p3d_testdata::strategy;
use proptest::prelude::*;

fn euler(m: &Mesh) -> i64 {
    m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn primitives_are_valid(m in strategy::primitive()) {
        prop_assert!(m.face_count() > 0, "generator failed on valid parameters");
        prop_assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn deletions_keep_the_mesh_valid(m in strategy::mesh()) {
        prop_assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn compact_preserves_everything_live(mut m in strategy::mesh()) {
        let counts = (m.vert_count(), m.edge_count(), m.face_count(), m.triangle_count(), euler(&m));
        let positions: Vec<Vec3> = m.verts().map(|v| m.positions[v as usize]).collect();
        let groups: Vec<(String, usize)> = m.groups().map(|(n, g)| (n.to_owned(), g.count())).collect();
        m.compact();
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert!(!m.has_holes_in_storage());
        prop_assert_eq!((m.vert_count(), m.edge_count(), m.face_count(), m.triangle_count(), euler(&m)), counts);
        prop_assert_eq!(m.positions.clone(), positions);
        prop_assert_eq!(m.groups().map(|(n, g)| (n.to_owned(), g.count())).collect::<Vec<_>>(), groups);
    }

    #[test]
    fn triangle_buffers_are_consistent(m in strategy::mesh()) {
        let b = m.to_tri_buffers(&TriOptions::default());
        prop_assert_eq!(b.triangle_count(), m.triangle_count());
        prop_assert!(b.indices.iter().all(|&i| (i as usize) < b.positions.len()));
        prop_assert_eq!(b.normals.len(), b.positions.len());
        prop_assert!(b.vert_source.iter().all(|&v| m.is_vert_alive(v)));
        prop_assert!(b.tri_face.iter().all(|&f| m.is_face_alive(f)));
        // No more output vertices than corners, no fewer than used vertices.
        let corners: usize = m.faces().map(|f| m.face_len(f)).sum();
        prop_assert!(b.positions.len() <= corners && b.positions.len() >= m.vert_count());
        // Submeshes tile the index buffer.
        let covered: u32 = b.submeshes.iter().map(|s| s.count).sum();
        prop_assert_eq!(covered as usize, b.indices.len());
        // Triangulated area matches face area (quads of spheres are slightly non-planar).
        let face_area: f32 = m.faces().map(|f| m.face_area(f)).sum();
        let tri_area: f32 = b.indices.chunks(3).map(|t| {
            let [a, c, d] = [0, 1, 2].map(|i| b.positions[t[i] as usize]);
            0.5 * (c - a).cross(d - a).length()
        }).sum();
        prop_assert!((tri_area - face_area).abs() <= 0.02 * face_area + 1e-6, "{} vs {}", tri_area, face_area);
        // Deterministic.
        prop_assert_eq!(m.to_tri_buffers(&TriOptions::default()), b);
    }

    #[test]
    fn corner_normals_are_unit(m in strategy::mesh()) {
        let n = m.compute_corner_normals();
        for f in m.faces() {
            for h in m.face_halfedges(f) {
                prop_assert!((n[h as usize].length() - 1.0).abs() < 1e-4, "corner {} has normal {}", h, n[h as usize]);
            }
        }
    }
}
