//! Property tests for booleans on random closed primitives, rotated and
//! moved: results are valid closed meshes, volumes add up (inclusion and
//! exclusion), and the target's face groups survive.

use glam::{Affine3A, DVec3, Quat, Vec3};
use p3d_geo::boolean::{BoolOp, BooleanError, BooleanSpec, ManifoldEngine, boolean};
use p3d_geo::prim::{BoxSpec, CylinderSpec, IcoSphereSpec, Origin, QuadSphereSpec, TorusSpec, cuboid, cylinder, ico_sphere, quad_sphere, torus};
use p3d_mesh::{ElementKind, Mesh};
use proptest::prelude::*;

fn volume(m: &Mesh) -> f64 {
    m.faces()
        .flat_map(|f| m.face_triangles(f))
        .map(|t| {
            let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn solid(kind: u8, size: f64, segments: u32) -> Mesh {
    let made = match kind % 5 {
        0 => {
            cuboid(&BoxSpec { size: DVec3::new(size, size * 0.7, size * 1.3), segments: [1; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center)
        }
        1 => quad_sphere(&QuadSphereSpec { radius: size * 0.5, segments: segments.min(8) }, Origin::Center),
        2 => cylinder(
            &CylinderSpec { radius: size * 0.4, height: size, segments: 4 * (segments / 4).max(2), ..CylinderSpec::default() },
            Origin::Center,
        ),
        3 => ico_sphere(&IcoSphereSpec { radius: size * 0.5, subdivisions: 1 + segments % 2 }, Origin::Center),
        _ => torus(&TorusSpec { major_radius: size * 0.4, minor_radius: size * 0.15, segments, ..TorusSpec::default() }, Origin::Center),
    };
    let mut m = made.unwrap_or_default();
    let faces: Vec<u32> = m.faces().collect();
    let _ = m.set_group("all", ElementKind::Face, faces);
    m
}

fn run(a: &Mesh, b: &Mesh, op: BoolOp) -> Result<Option<Mesh>, BooleanError> {
    match boolean(a, &[b], &BooleanSpec { op, ..BooleanSpec::default() }, &ManifoldEngine) {
        Ok(r) => Ok(Some(r.mesh)),
        Err(BooleanError::Empty) => Ok(None),
        Err(e) => Err(e),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn booleans_are_closed_and_volumes_add_up(
        ka in 0u8..5,
        kb in 0u8..5,
        sa in 0.5f64..2.0,
        sb in 0.5f64..2.0,
        segments in 6u32..20,
        turn in prop::array::uniform3(-3.0f32..3.0),
        shift in prop::array::uniform3(-1.0f32..1.0),
    ) {
        let a = solid(ka, sa, segments);
        let mut b = solid(kb, sb, segments + 3);
        b.transform(Affine3A::from_rotation_translation(Quat::from_euler(glam::EulerRot::XYZ, turn[0], turn[1], turn[2]), Vec3::from(shift)));
        let (va, vb) = (volume(&a), volume(&b));
        prop_assert!(va > 0.0 && vb > 0.0, "a primitive failed to build");
        let mut vols = [0.0; 3];
        for (i, op) in [BoolOp::Union, BoolOp::Difference, BoolOp::Intersect].into_iter().enumerate() {
            let r = run(&a, &b, op);
            prop_assert!(r.is_ok(), "{op:?}: {r:?}");
            if let Ok(Some(m)) = r {
                prop_assert_eq!(m.validate(), Ok(()), "{:?}", op);
                prop_assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "{op:?} left open edges");
                // No flat leftovers: every piece is thicker on average (volume
                // over area) than a quarter of the weld distance.
                let (label, count) = m.face_components();
                let mut pieces = vec![(0.0f64, 0.0f64); count as usize];
                for f in m.faces() {
                    for t in m.face_triangles(f) {
                        let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
                        let p = &mut pieces[label[f as usize] as usize];
                        p.0 += a.dot(b.cross(c)) / 6.0;
                        p.1 += (b - a).cross(c - a).length() / 2.0;
                    }
                }
                prop_assert!(pieces.iter().all(|(v, a)| v.abs() > 0.25e-4 * a), "{op:?} left a flat piece: {pieces:?}");
                // What is left of the target keeps its group (a union or an
                // intersection may consist of tool faces only).
                let all = m.group("all").map_or(0, |g| g.count());
                prop_assert!(all > 0 || op != BoolOp::Difference, "the difference lost the group");
                vols[i] = volume(&m);
            }
        }
        let [u, d, x] = vols;
        let tol = 2e-3 * (va + vb);
        prop_assert!((va + vb - u - x).abs() < tol, "|A| + |B| = {} but union + intersection = {}", va + vb, u + x);
        prop_assert!((va - d - x).abs() < tol, "|A| = {va} but difference + intersection = {}", d + x);
    }
}
