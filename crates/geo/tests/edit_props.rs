//! Property tests for the edit kernels on random meshes: the result is
//! always a valid mesh, and a refused edit changes nothing.

use p3d_geo::edit::{ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, KernelError, extrude, inset};
use p3d_mesh::Mesh;
use p3d_testdata::strategy;
use proptest::prelude::*;

fn pick_faces(m: &Mesh, picks: &[u32]) -> Vec<u32> {
    let live: Vec<u32> = m.faces().collect();
    let mut out: Vec<u32> = picks.iter().filter(|_| !live.is_empty()).map(|&p| live[p as usize % live.len()]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Valid after success, untouched after a refusal.
fn check(before: &Mesh, after: &Mesh, r: Result<(), KernelError>) -> Result<(), TestCaseError> {
    match r {
        Ok(()) => prop_assert_eq!(after.validate(), Ok(())),
        Err(_) => prop_assert_eq!(after, before),
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn extrusions_stay_valid(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12), individual in any::<bool>(), d in -0.5f32..0.5) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty() && d.abs() > 1e-3);
        let mode = if individual { ExtrudeMode::Individual } else { ExtrudeMode::Region };
        let mut n = m.clone();
        let r = extrude(&mut n, &faces, &ExtrudeSpec { mode, distance: d, direction: None });
        prop_assert!(r.is_ok(), "{:?}", r);
        if let Ok(r) = &r {
            prop_assert_eq!(r.cap.len(), faces.len());
            prop_assert_eq!(n.face_count(), m.face_count() + r.side.len());
        }
        check(&m, &n, r.map(|_| ()))?;
    }

    #[test]
    fn insets_stay_valid_or_change_nothing(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12), individual in any::<bool>(), t in 0.001f32..0.3, depth in -0.2f32..0.2) {
        let faces = pick_faces(&m, &picks);
        prop_assume!(!faces.is_empty());
        let mode = if individual { InsetMode::Individual } else { InsetMode::Region };
        let mut n = m.clone();
        let r = inset(&mut n, &faces, &InsetSpec { mode, thickness: t, depth });
        if let Ok(r) = &r {
            prop_assert_eq!(r.inner.len(), faces.len());
            prop_assert_eq!(n.face_count(), m.face_count() + r.rim.len());
        }
        check(&m, &n, r.map(|_| ()))?;
    }
}
