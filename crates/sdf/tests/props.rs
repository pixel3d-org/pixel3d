//! Property tests: random trees of primitives with random joins and
//! modifiers mesh into valid, closed, outward-facing meshes, the same every
//! time.

use glam::{Quat, Vec3};
use p3d_sdf::{Combine, Item, MeshSpec, Modifiers, Noise, Prim, Sdf, SdfError, mesh};
use proptest::prelude::*;

fn prim() -> impl Strategy<Value = Prim> {
    prop_oneof![
        (0.1f32..0.5).prop_map(|radius| Prim::Sphere { radius }),
        prop::array::uniform3(0.1f32..0.5).prop_map(|r| Prim::Ellipsoid { radii: Vec3::from(r) }),
        (prop::array::uniform3(0.1f32..0.4), 0.0f32..1.0).prop_map(|(h, t)| {
            let half = Vec3::from(h);
            Prim::Box { half, round: t * half.min_element() * 0.9 }
        }),
        (prop::array::uniform3(-0.3f32..0.3), 0.05f32..0.2, 0.0f32..0.2).prop_map(|(b, ra, rb)| Prim::Capsule {
            a: Vec3::ZERO,
            b: Vec3::from(b),
            ra,
            rb
        }),
        (0.2f32..0.6, 0.0f32..0.3, 0.05f32..0.3).prop_map(|(height, bottom, top)| Prim::Cone { height, bottom, top }),
        (0.2f32..0.4, 0.05f32..0.15).prop_map(|(major, minor)| Prim::Torus { major, minor }),
        (0.1f32..0.6, 0.05f32..0.3).prop_map(|(height, radius)| Prim::Cylinder { height, radius }),
    ]
}

fn combine() -> impl Strategy<Value = Combine> {
    prop_oneof![
        Just(Combine::Union),
        (0.02f32..0.15).prop_map(Combine::SmoothUnion),
        Just(Combine::Subtract),
        (0.02f32..0.1).prop_map(Combine::SmoothSubtract),
        Just(Combine::Intersect),
        (0.02f32..0.1).prop_map(Combine::SmoothIntersect),
    ]
}

fn item() -> impl Strategy<Value = Item> {
    (prim(), prop::array::uniform3(-0.3f32..0.3), prop::array::uniform3(-3.0f32..3.0), combine(), 0.0f32..0.03, any::<u8>(), 0u64..100).prop_map(
        |(p, at, turn, combine, round, flags, seed)| {
            let modifiers = Modifiers {
                round,
                shell: if flags & 1 == 1 { 0.04 } else { 0.0 },
                twist: if flags & 2 == 2 { 1.5 } else { 0.0 },
                bend: if flags & 4 == 4 { 0.8 } else { 0.0 },
                mirror: [flags & 8 == 8, false, false],
                noise: (flags & 16 == 16).then_some(Noise { amplitude: 0.01, scale: 0.08, seed, octaves: 2 }),
            };
            Item { rotation: Quat::from_euler(glam::EulerRot::XYZ, turn[0], turn[1], turn[2]), combine, modifiers, ..Item::prim(p, Vec3::from(at)) }
        },
    )
}

fn volume(m: &p3d_mesh::Mesh) -> f64 {
    m.faces()
        .flat_map(|f| m.face_triangles(f))
        .map(|t| {
            let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn random_trees_mesh_closed(items in prop::collection::vec(item(), 1..5)) {
        let sdf = Sdf::new(items).unwrap();
        match mesh(&sdf, &MeshSpec { voxel: 0.025 }) {
            Ok(r) => {
                prop_assert_eq!(r.mesh.validate(), Ok(()));
                let v = volume(&r.mesh);
                prop_assert!(v > 0.0, "volume {v}, faces {}, voxel {}", r.mesh.face_count(), r.voxel);
                prop_assert!(r.mesh.edges().all(|e| !r.mesh.is_boundary_edge(e)), "open edges: {:?}, unresolved {}, voxel {}, specks {}", r.repair, r.unresolved, r.voxel, r.specks);
                prop_assert_eq!(r.face_leaf.len(), r.mesh.face_count());
                let again = mesh(&sdf, &MeshSpec { voxel: 0.025 }).unwrap();
                prop_assert_eq!(&again.mesh.positions, &r.mesh.positions);
            }
            // Cuts and intersections can leave nothing.
            Err(e) => prop_assert_eq!(e, SdfError::Empty),
        }
    }
}
