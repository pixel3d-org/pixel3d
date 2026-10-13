//! Property tests for cleanup on random meshes (primitives with faces
//! deleted, some joined with moved, flipped or triangulated copies).

use glam::{Affine3A, Quat, Vec3};
use p3d_geo::cleanup::{CleanupSpec, cleanup};
use p3d_testdata::strategy;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn cleanup_leaves_a_valid_mesh_and_settles(
        a in strategy::mesh(),
        b in strategy::mesh(),
        join in any::<bool>(),
        flip in any::<bool>(),
        tri in any::<bool>(),
        shift in prop::array::uniform3(-2.0f32..2.0),
    ) {
        let mut m = a;
        if join {
            let mut other = b;
            if flip { other.flip_all(); }
            other.transform(Affine3A::from_rotation_translation(Quat::from_rotation_y(0.3), Vec3::from(shift)));
            m.append(&other);
        }
        if tri {
            let faces: Vec<u32> = m.faces().collect();
            let _ = p3d_geo::edit::triangulate(&mut m, &faces);
        }
        let spec = CleanupSpec::default();
        let r = cleanup(&mut m, &spec);
        prop_assert_eq!(m.validate(), Ok(()), "{:?}", r);
        // A second pass has nothing left to weld, remove or turn.
        let again = cleanup(&mut m, &spec);
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert_eq!((again.welded, again.specks_removed, again.enclosed_removed, again.parts_flipped), (0, 0, 0, 0), "first {:?}, then {:?}", r, again);
    }
}
