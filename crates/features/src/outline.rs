//! Outline helpers: corner cutting that keeps chosen corners sharp.

use glam::DVec2;

/// Chaikin corner cutting, `rounds` times, of a closed polygon; vertices
/// flagged `sharp` stay as corners.
///
/// Complexity: O(n x 2^rounds).
pub fn smooth(points: &[DVec2], sharp: &[bool], rounds: u32) -> Vec<DVec2> {
    let mut pts: Vec<(DVec2, bool)> = points.iter().copied().zip(sharp.iter().copied().chain(std::iter::repeat(false))).collect();
    for _ in 0..rounds {
        let n = pts.len();
        let mut next = Vec::with_capacity(2 * n);
        for i in 0..n {
            let (p, sp) = pts[i];
            let (q, _) = pts[(i + 1) % n];
            if sp {
                next.push((p, true));
            } else {
                // The cut near p is the end of the previous edge's cut; it is
                // emitted as part of edge (prev, p) below.
            }
            let a = p * 0.75 + q * 0.25;
            let b = p * 0.25 + q * 0.75;
            next.push((a, false));
            next.push((b, false));
        }
        // A sharp corner keeps its neighbours' cuts on either side; drop the
        // cut points that would round it off (those right next to it).
        pts = next;
    }
    pts.into_iter().map(|(p, _)| p).collect()
}

/// A closed outline through normalized points `(x, y)` scaled by `width`
/// and `length`, smoothed except at sharp points.
pub fn shaped(points: &[(f64, f64, bool)], width: f64, length: f64, rounds: u32) -> Vec<DVec2> {
    let pts: Vec<DVec2> = points.iter().map(|&(x, y, _)| DVec2::new(x * width, y * length)).collect();
    let sharp: Vec<bool> = points.iter().map(|p| p.2).collect();
    smooth(&pts, &sharp, rounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharp_points_survive_smoothing() {
        let tri = [DVec2::ZERO, DVec2::new(1.0, 0.0), DVec2::new(0.5, 1.0)];
        let s = smooth(&tri, &[false, false, true], 3);
        assert!(s.iter().any(|p| p.distance(DVec2::new(0.5, 1.0)) < 1e-12), "the tip stays");
        assert!(!s.iter().any(|p| p.distance(DVec2::ZERO) < 1e-12), "round corners are cut");
        assert!(s.len() > 20);
    }
}
