//! A static k-d tree over points for nearest-neighbour queries.

use glam::Vec3;

/// Points arranged as an implicit balanced k-d tree: the median of every
/// index range splits it, cycling through X, Y and Z by depth.
#[derive(Debug, Clone, Default)]
pub struct PointTree {
    /// Points in tree order (contiguous for cache-friendly queries).
    pts: Vec<Vec3>,
    /// Original id of each point in tree order.
    order: Vec<u32>,
}

impl PointTree {
    /// Builds the tree.
    ///
    /// Complexity: O(n log n).
    pub fn new(points: &[Vec3]) -> PointTree {
        let mut order: Vec<u32> = (0..points.len() as u32).collect();
        let mut stack = vec![(0usize, order.len(), 0usize)];
        while let Some((lo, hi, depth)) = stack.pop() {
            if hi - lo <= 1 {
                continue;
            }
            let mid = (lo + hi) / 2;
            let axis = depth % 3;
            order[lo..hi].select_nth_unstable_by(mid - lo, |&a, &b| points[a as usize][axis].total_cmp(&points[b as usize][axis]));
            stack.push((lo, mid, depth + 1));
            stack.push((mid + 1, hi, depth + 1));
        }
        let pts = order.iter().map(|&i| points[i as usize]).collect();
        PointTree { pts, order }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The nearest point within `max_dist` of `p`: its id and distance.
    ///
    /// Complexity: O(log n) expected.
    pub fn nearest(&self, p: Vec3, max_dist: f32) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        let mut best_d2 = max_dist * max_dist;
        // Each range carries the squared distance from p to its splitting
        // plane side, re-checked when popped: a closer point found meanwhile
        // prunes it.
        let mut stack = vec![(0usize, self.order.len(), 0usize, 0.0f32)];
        while let Some((lo, hi, depth, plane_d2)) = stack.pop() {
            if lo >= hi || plane_d2 > best_d2 {
                continue;
            }
            let mid = (lo + hi) / 2;
            let id = self.order[mid];
            let q = self.pts[mid];
            let d2 = q.distance_squared(p);
            if d2 <= best_d2 {
                best_d2 = d2;
                best = Some((id, d2));
            }
            let axis = depth % 3;
            let diff = p[axis] - q[axis];
            let (near, far) = if diff < 0.0 { ((lo, mid), (mid + 1, hi)) } else { ((mid + 1, hi), (lo, mid)) };
            // The far side matters only if the splitting plane is closer than the best so far.
            if diff * diff <= best_d2 {
                stack.push((far.0, far.1, depth + 1, diff * diff));
            }
            stack.push((near.0, near.1, depth + 1, plane_d2));
        }
        best.map(|(id, d2)| (id, d2.sqrt()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_matches_brute_force() {
        let pts: Vec<Vec3> = (0..500)
            .map(|i| {
                let t = i as f32;
                Vec3::new((t * 0.37).sin() * 3.0, (t * 0.91).cos() * 2.0, (t * 0.13).sin())
            })
            .collect();
        let tree = PointTree::new(&pts);
        for j in 0..200 {
            let q = Vec3::new((j as f32 * 0.7).cos() * 3.0, (j as f32 * 0.3).sin() * 2.0, 0.2);
            let brute = pts.iter().enumerate().map(|(i, p)| (i as u32, p.distance(q))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            let got = tree.nearest(q, f32::INFINITY).unwrap();
            assert!((got.1 - brute.1).abs() < 1e-6, "{got:?} vs {brute:?}");
        }
        assert!(tree.nearest(Vec3::splat(100.0), 1.0).is_none());
        assert!(PointTree::new(&[]).nearest(Vec3::ZERO, 1.0).is_none());
    }
}
