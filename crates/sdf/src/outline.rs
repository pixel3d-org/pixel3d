//! Silhouettes (project spec 17.4): "draw it like a blueprint". Two or three
//! orthographic outlines (front, side, top) are extruded through the part
//! and intersected (the visual hull), with edges rounded by eroding the
//! outlines and growing the result back. The field is the maximum of exact
//! 2D distances, so it is 1-Lipschitz and meshes like any SDF.

use glam::{Vec2, Vec3};

use crate::mesh::Field;

/// Most outline edges over all views (each evaluation visits every edge).
pub const MAX_OUTLINE_EDGES: usize = 4096;

/// An orthographic view and its 2D axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// Seen from +Z: horizontal x, vertical y.
    Front,
    /// Seen from +X: horizontal z (front to the right), vertical y.
    Side,
    /// Seen from +Y: horizontal x, vertical z (front up).
    Top,
}

impl View {
    /// World axes of the outline's (horizontal, vertical) coordinates.
    pub fn axes(self) -> (usize, usize) {
        match self {
            View::Front => (0, 1),
            View::Side => (2, 1),
            View::Top => (0, 2),
        }
    }
}

/// Why outlines cannot make a field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutlineError {
    #[error("give at least two views (front and side, say); one outline alone is an extrusion")]
    TooFewViews,
    #[error("the {0} view is given twice")]
    Repeated(&'static str),
    #[error("the {0} outline needs at least three points enclosing an area")]
    Empty(&'static str),
    #[error("the outlines have {0} edges; at most {MAX_OUTLINE_EDGES}")]
    TooManyEdges(usize),
    #[error("the views do not overlap: their shared axis ranges are apart")]
    Disjoint,
}

struct Prepared {
    u: usize,
    v: usize,
    edges: Vec<(Vec2, Vec2)>,
}

/// The visual hull of outlines.
pub struct OutlineField {
    views: Vec<Prepared>,
    round: f32,
    lo: Vec3,
    hi: Vec3,
}

fn name(v: View) -> &'static str {
    match v {
        View::Front => "front",
        View::Side => "side",
        View::Top => "top",
    }
}

/// Signed distance to closed contours (negative inside, even-odd).
///
/// Complexity: O(edges).
fn sd2(edges: &[(Vec2, Vec2)], p: Vec2) -> f32 {
    let mut d = f32::MAX;
    let mut inside = false;
    for &(a, b) in edges {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
        d = d.min((a + ab * t).distance_squared(p));
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * ab.x {
            inside = !inside;
        }
    }
    let d = d.sqrt();
    if inside { -d } else { d }
}

impl OutlineField {
    /// Builds the field; `round` rounds every edge of the hull.
    ///
    /// Complexity: O(points).
    pub fn new(views: &[(View, Vec<Vec<Vec2>>)], round: f32) -> Result<OutlineField, OutlineError> {
        if views.len() < 2 {
            return Err(OutlineError::TooFewViews);
        }
        let mut prepared: Vec<Prepared> = Vec::with_capacity(views.len());
        let mut seen: Vec<View> = Vec::new();
        let (mut lo, mut hi) = (Vec3::splat(f32::MIN), Vec3::splat(f32::MAX));
        let mut total = 0;
        for (view, contours) in views {
            if seen.contains(view) {
                return Err(OutlineError::Repeated(name(*view)));
            }
            seen.push(*view);
            let edges: Vec<(Vec2, Vec2)> =
                contours.iter().filter(|c| c.len() >= 3).flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()]))).collect();
            let area: f32 =
                contours.iter().filter(|c| c.len() >= 3).map(|c| (0..c.len()).map(|i| c[i].perp_dot(c[(i + 1) % c.len()])).sum::<f32>().abs()).sum();
            if edges.is_empty() || area <= 1e-12 || edges.iter().any(|(a, b)| !a.is_finite() || !b.is_finite()) {
                return Err(OutlineError::Empty(name(*view)));
            }
            total += edges.len();
            let (u, v) = view.axes();
            let (mut l2, mut h2) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            for (a, _) in &edges {
                l2 = l2.min(*a);
                h2 = h2.max(*a);
            }
            lo[u] = lo[u].max(l2.x);
            hi[u] = hi[u].min(h2.x);
            lo[v] = lo[v].max(l2.y);
            hi[v] = hi[v].min(h2.y);
            prepared.push(Prepared { u, v, edges });
        }
        if total > MAX_OUTLINE_EDGES {
            return Err(OutlineError::TooManyEdges(total));
        }
        // Two views always cover all three axes (each pair of views shares
        // exactly one axis).
        if (0..3).any(|k| lo[k] > hi[k]) {
            return Err(OutlineError::Disjoint);
        }
        Ok(OutlineField { views: prepared, round: round.max(0.0), lo, hi })
    }
}

impl Field for OutlineField {
    fn eval(&self, p: Vec3) -> f32 {
        let r = self.round;
        self.views.iter().map(|v| sd2(&v.edges, Vec2::new(p[v.u], p[v.v])) + r).fold(f32::MIN, f32::max) - r
    }

    fn lipschitz(&self) -> f32 {
        1.0
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        (self.lo, self.hi)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::mesh::{MeshSpec, mesh};

    fn rect(w: f32, h: f32) -> Vec<Vec<Vec2>> {
        vec![vec![Vec2::new(-w / 2.0, 0.0), Vec2::new(w / 2.0, 0.0), Vec2::new(w / 2.0, h), Vec2::new(-w / 2.0, h)]]
    }

    #[test]
    fn two_rectangles_make_a_box_and_a_triangle_cuts_it() {
        let f = OutlineField::new(&[(View::Front, rect(0.4, 0.2)), (View::Side, rect(0.1, 0.2))], 0.0).unwrap();
        assert!(f.eval(Vec3::new(0.0, 0.1, 0.0)) < 0.0);
        assert!((f.eval(Vec3::new(0.3, 0.1, 0.0)) - 0.1).abs() < 1e-6);
        let m = mesh(&f, &MeshSpec { voxel: 0.01 }).unwrap().mesh;
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - lo - Vec3::new(0.4, 0.2, 0.1)).abs().max_element() < 0.01, "{lo} {hi}");
        // A side profile like a wedge.
        let wedge = vec![vec![Vec2::new(-0.05, 0.0), Vec2::new(0.05, 0.0), Vec2::new(-0.05, 0.2)]];
        let f = OutlineField::new(&[(View::Front, rect(0.4, 0.2)), (View::Side, wedge)], 0.01).unwrap();
        assert!(f.eval(Vec3::new(0.0, 0.18, 0.04)) > 0.0, "outside the slope");
        assert!(f.eval(Vec3::new(0.0, 0.05, -0.02)) < 0.0);
    }

    #[test]
    fn bad_views() {
        assert_eq!(OutlineField::new(&[(View::Front, rect(0.1, 0.1))], 0.0).err(), Some(OutlineError::TooFewViews));
        assert_eq!(
            OutlineField::new(&[(View::Front, rect(0.1, 0.1)), (View::Front, rect(0.1, 0.1))], 0.0).err(),
            Some(OutlineError::Repeated("front"))
        );
        let high = vec![vec![Vec2::new(0.0, 5.0), Vec2::new(0.1, 5.0), Vec2::new(0.1, 6.0)]];
        assert_eq!(OutlineField::new(&[(View::Front, rect(0.1, 0.1)), (View::Side, high)], 0.0).err(), Some(OutlineError::Disjoint));
    }
}
