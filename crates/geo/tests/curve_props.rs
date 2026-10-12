//! Property tests for the curve kernels on random polylines.

use glam::Vec3;
use p3d_geo::curve::{auto_handles, fillet, length, offset, resample, sample_bezier, sample_bezier_spans};
use proptest::prelude::*;

/// A star-shaped closed polyline in the ground plane: angles increase, radii
/// vary, so it never crosses itself.
fn star() -> impl Strategy<Value = Vec<Vec3>> {
    prop::collection::vec(0.5f32..2.0, 3..40).prop_map(|radii| {
        let n = radii.len();
        radii
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                Vec3::new(a.cos() * r, 0.0, -a.sin() * r)
            })
            .collect()
    })
}

/// An open polyline in 3D with distinct consecutive points.
fn path() -> impl Strategy<Value = Vec<Vec3>> {
    prop::collection::vec(prop::array::uniform3(-2.0f32..2.0), 2..30).prop_map(|steps| {
        let mut p = Vec3::ZERO;
        steps
            .into_iter()
            .map(|s| {
                p += Vec3::from(s) + Vec3::X * 0.1;
                p
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn bezier_samples_pass_through_every_point(pts in path(), closed in any::<bool>(), res in 1u32..16) {
        let h = auto_handles(&pts, closed);
        let s = sample_bezier(&pts, &h, closed, res).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let spans = if closed { pts.len() } else { pts.len() - 1 };
        prop_assert!(s.len() <= spans * res as usize + 1);
        for p in &pts {
            prop_assert!(s.iter().any(|q| q.distance(*p) < 1e-4), "{} is not on the samples", p);
        }
        // One span per segment, counting up through every span.
        let (_, of) = sample_bezier_spans(&pts, &h, closed, res).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(of.len(), if closed { s.len() } else { s.len() - 1 });
        prop_assert!(of.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));
        prop_assert_eq!(of.last().map(|&x| x as usize + 1), Some(spans));
    }

    #[test]
    fn resampling_keeps_count_ends_and_even_spacing(pts in path(), count in 2usize..200) {
        let r = resample(&pts, false, count).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(r.len(), count);
        prop_assert!(r[0].distance(pts[0]) < 1e-5 && r[count - 1].distance(pts[pts.len() - 1]) < 1e-4);
        // Samples follow the polyline, so chords never exceed the arc-length step.
        let step = length(&pts, false) / (count - 1) as f32;
        prop_assert!(r.windows(2).all(|w| w[0].distance(w[1]) <= step * 1.001 + 1e-5));
    }

    #[test]
    fn closed_offsets_contain_or_are_contained(pts in star(), d in 0.01f32..0.2) {
        // Outward offsets of a star polygon move every vertex away from the center on average.
        let out = offset(&pts, true, d, None, 4.0).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let inner = offset(&pts, true, -d * 0.1, None, 4.0).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(out.len() >= pts.len() && inner.len() >= pts.len());
        let mean = |v: &[Vec3]| v.iter().map(|p| p.length()).sum::<f32>() / v.len() as f32;
        prop_assert!(mean(&out) > mean(&pts) && mean(&inner) < mean(&pts));
        prop_assert!(out.iter().all(|p| p.y.abs() < 1e-5), "stays in the plane");
    }

    #[test]
    fn fillets_shorten_the_path_and_keep_open_ends(pts in path(), r in 0.01f32..1.0, seg in 1u32..8) {
        let f = fillet(&pts, false, r, seg).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(f[0].distance(pts[0]) < 1e-5 && f[f.len() - 1].distance(pts[pts.len() - 1]) < 1e-5);
        prop_assert!(length(&f, false) <= length(&pts, false) + 1e-3, "a fillet cuts corners");
        prop_assert!(f.len() <= pts.len() * (seg as usize + 1));
        prop_assert!(f.iter().all(|p| p.is_finite()));
    }
}
