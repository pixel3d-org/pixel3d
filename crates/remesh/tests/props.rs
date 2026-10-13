//! Property tests: random closed primitives remesh into valid, closed,
//! all-quad meshes near the target edge length, the same every time.

use glam::DVec3;
use p3d_geo::prim::{BoxSpec, CapsuleSpec, IcoSphereSpec, Origin, TorusSpec, capsule, cuboid, ico_sphere, torus};
use p3d_mesh::Mesh;
use p3d_remesh::{QuadSpec, quad_remesh};
use proptest::prelude::*;

fn solid(kind: u8, size: f64) -> Mesh {
    let m = match kind % 4 {
        0 => ico_sphere(&IcoSphereSpec { radius: size * 0.5, subdivisions: 3 }, Origin::Center),
        1 => cuboid(
            &BoxSpec { size: DVec3::new(size, size * 0.6, size * 0.8), segments: [2; 3], bevel_width: size * 0.05, bevel_segments: 2 },
            Origin::Center,
        ),
        2 => torus(&TorusSpec { major_radius: size * 0.4, minor_radius: size * 0.15, segments: 32, sides: 16 }, Origin::Center),
        _ => capsule(&CapsuleSpec { radius: size * 0.2, height: size, segments: 16, height_segments: 1 }, Origin::Center),
    };
    m.unwrap_or_default()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn closed_shapes_remesh_into_closed_quads(kind in 0u8..4, size in 0.3f64..2.0, ratio in 12.0f32..30.0, seed in 0u64..1000) {
        let input = solid(kind, size);
        prop_assume!(input.face_count() > 0);
        let edge = size as f32 / ratio;
        let spec = QuadSpec { edge, seed, ..QuadSpec::default() };
        let (m, r) = quad_remesh(&input, &spec).unwrap();
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "{:?}", r);
        prop_assert!(m.faces().all(|f| m.face_len(f) == 4), "{:?}", r);
        // No pole above valence 6, and none of valence 2.
        prop_assert!(r.poles[3] == 0 && r.valence_two == 0, "{:?}", r);
        let mean = m.edges().map(|e| {
            let [a, b] = m.edge_verts(e);
            m.positions[a as usize].distance(m.positions[b as usize])
        }).sum::<f32>() / m.edges().count() as f32;
        prop_assert!(mean > edge * 0.5 && mean < edge * 1.6, "mean edge {} for {}", mean, edge);
        let again = quad_remesh(&input, &spec).unwrap().0;
        prop_assert_eq!(&again.positions, &m.positions);
    }
}
