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

/// Rounds every corner of a closed polygon that turns more than `max_turn`
/// (radians) with an arc of `radius` (smaller where the edges beside it are
/// short), in steps of at most `max_turn`: a needle tip becomes a small nub
/// that shades smoothly and keeps a wall.
///
/// Complexity: O(n + added points).
pub fn round_corners(points: &[DVec2], radius: f64, max_turn: f64) -> Vec<DVec2> {
    let n = points.len();
    if n < 3 || radius <= 0.0 || max_turn <= 0.0 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n + 16);
    for i in 0..n {
        let p = points[i];
        let u = p - points[(i + n - 1) % n];
        let w = points[(i + 1) % n] - p;
        let (lu, lw) = (u.length(), w.length());
        if lu < 1e-12 || lw < 1e-12 {
            out.push(p);
            continue;
        }
        let (u, w) = (u / lu, w / lw);
        let turn = u.perp_dot(w).atan2(u.dot(w));
        // Half the inside angle at the corner.
        let half = (std::f64::consts::PI - turn.abs()) * 0.5;
        if turn.abs() <= max_turn || half < 1e-6 {
            out.push(p);
            continue;
        }
        let t = (radius / half.tan()).min(0.45 * lu.min(lw));
        let r = t * half.tan();
        let start = p - u * t;
        let c = start + DVec2::new(-u.y, u.x) * turn.signum() * r;
        let a0 = (start - c).y.atan2((start - c).x);
        let steps = (turn.abs() / max_turn).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let a = a0 + turn * s as f64 / steps as f64;
            out.push(c + DVec2::new(a.cos(), a.sin()) * r);
        }
    }
    out
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

    #[test]
    fn sharp_corners_become_small_arcs() {
        let tri = [DVec2::ZERO, DVec2::new(1.0, 0.0), DVec2::new(0.5, 1.0)];
        let r = round_corners(&tri, 0.05, std::f64::consts::FRAC_PI_4);
        assert!(r.len() > 9, "every corner turns more than 45 degrees: {}", r.len());
        let n = r.len();
        for i in 0..n {
            let (a, p, b) = (r[(i + n - 1) % n], r[i], r[(i + 1) % n]);
            let (u, w) = ((p - a).normalize(), (b - p).normalize());
            assert!(u.perp_dot(w).atan2(u.dot(w)).abs() <= std::f64::consts::FRAC_PI_4 + 1e-9, "turn at {p}");
        }
        // The tip moves back by less than the radius over the sine of half its angle.
        let top = r.iter().map(|p| p.y).fold(f64::MIN, f64::max);
        assert!(top < 1.0 && top > 0.85, "{top}");
    }
}
