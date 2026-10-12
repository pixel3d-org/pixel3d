//! Curve objects: polylines and cubic Bezier splines used as profiles and
//! paths by generators (lathe, sweep, loft, pipe).

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// One spline of a curve: a polyline, or a cubic Bezier spline when
/// `handles` is set (one in and one out handle per point, absolute
/// positions in object space).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spline {
    pub points: Vec<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<Vec<[[f32; 3]; 2]>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
}

impl Spline {
    /// A polyline through points.
    pub fn polyline(points: impl IntoIterator<Item = Vec3>, closed: bool) -> Spline {
        Spline { points: points.into_iter().map(|p| p.to_array()).collect(), handles: None, closed }
    }

    pub fn is_bezier(&self) -> bool {
        self.handles.is_some()
    }

    /// Control points as vectors.
    ///
    /// Complexity: O(points).
    pub fn positions(&self) -> Vec<Vec3> {
        self.points.iter().map(|&p| Vec3::from(p)).collect()
    }

    /// Number of spans (segments between consecutive points).
    pub fn spans(&self) -> usize {
        match (self.points.len(), self.closed) {
            (0, _) | (1, _) => 0,
            (n, true) => n,
            (n, false) => n - 1,
        }
    }

    /// Whether the data is consistent: at least two points, finite values,
    /// one handle pair per point.
    ///
    /// Complexity: O(points).
    pub fn is_valid(&self) -> bool {
        self.points.len() >= 2
            && self.points.iter().flatten().all(|x| x.is_finite())
            && self.handles.as_ref().is_none_or(|h| h.len() == self.points.len() && h.iter().flatten().flatten().all(|x| x.is_finite()))
    }
}

/// A curve: one or more splines.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    pub splines: Vec<Spline>,
}

impl Curve {
    pub fn single(spline: Spline) -> Curve {
        Curve { splines: vec![spline] }
    }

    /// Total control points.
    ///
    /// Complexity: O(splines).
    pub fn point_count(&self) -> usize {
        self.splines.iter().map(|s| s.points.len()).sum()
    }

    /// Bounds of the control points and handles, `None` when empty.
    ///
    /// Complexity: O(points).
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let pts = self.splines.iter().flat_map(|s| {
            let handles = s.handles.iter().flatten().flat_map(|h| h.iter().copied());
            s.points.iter().copied().chain(handles)
        });
        pts.map(Vec3::from).fold(None, |acc, p| match acc {
            None => Some((p, p)),
            Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
        })
    }

    /// Whether every spline is valid and there is at least one.
    ///
    /// Complexity: O(points).
    pub fn is_valid(&self) -> bool {
        !self.splines.is_empty() && self.splines.iter().all(Spline::is_valid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_bounds_and_validity() {
        let s = Spline::polyline([Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0)], false);
        assert_eq!(s.spans(), 2);
        let c = Curve::single(Spline { closed: true, ..s.clone() });
        assert_eq!(c.splines[0].spans(), 3);
        assert_eq!(c.bounds(), Some((Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0))));
        assert!(c.is_valid());
        assert!(!Curve::default().is_valid());
        let bad = Spline { handles: Some(vec![[[0.0; 3]; 2]]), ..s };
        assert!(!bad.is_valid(), "one handle pair per point");
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Curve>(&json).unwrap(), c);
    }
}
