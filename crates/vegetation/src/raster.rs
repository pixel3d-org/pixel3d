//! Coverage rasters: the crown hull's silhouette in a few side views, and
//! how many foliage triangles cover each pixel. Coverage and overdraw are
//! kept up to date as triangles are added.

use glam::{Vec2, Vec3};

use crate::hull::Hull;

/// Pixels across the longer side of a view.
pub(crate) const RESOLUTION: usize = 96;

/// A view's pixel grid: a horizontal axis and +Y.
#[derive(Debug, Clone, Copy)]
struct Grid {
    right: Vec3,
    origin: Vec2,
    pixel: f32,
    w: usize,
    h: usize,
}

impl Grid {
    fn project(&self, q: Vec3) -> Vec2 {
        (Vec2::new(q.dot(self.right), q.y) - self.origin) / self.pixel
    }

    /// Calls `f` with every pixel whose center lies in the triangle.
    ///
    /// Complexity: O(pixels in the triangle's bounding box).
    fn scan(&self, tri: [Vec3; 3], mut f: impl FnMut(usize)) {
        let p = tri.map(|q| self.project(q));
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if area.abs() < 1e-9 {
            return;
        }
        let lo = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
        let hi = p[0].max(p[1]).max(p[2]).ceil().min(Vec2::new(self.w as f32 - 1.0, self.h as f32 - 1.0));
        if lo.x > hi.x || lo.y > hi.y {
            return;
        }
        let s = area.signum();
        for j in lo.y as usize..=hi.y as usize {
            for i in lo.x as usize..=hi.x as usize {
                let c = Vec2::new(i as f32 + 0.5, j as f32 + 0.5);
                let e0 = (p[1] - p[0]).perp_dot(c - p[0]) * s;
                let e1 = (p[2] - p[1]).perp_dot(c - p[1]) * s;
                let e2 = (p[0] - p[2]).perp_dot(c - p[2]) * s;
                if e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 {
                    f(j * self.w + i);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct View {
    grid: Grid,
    inside: Vec<bool>,
    hits: Vec<u16>,
    inside_count: u32,
    covered: u32,
    layers: u64,
    drawn: u32,
}

/// Side views around the crown's vertical axis.
#[derive(Debug, Clone)]
pub(crate) struct Views {
    views: Vec<View>,
}

impl Views {
    /// `count` side views spread over half a turn (opposite views have the
    /// same silhouette).
    ///
    /// Complexity: O(count x RESOLUTION^2).
    pub fn around(hull: &Hull, count: usize) -> Views {
        let views = (0..count)
            .map(|k| {
                let a = k as f32 / count as f32 * std::f32::consts::PI;
                let right = Vec3::new(a.cos(), 0.0, a.sin());
                let center = Vec3::new(hull.axis.x, 0.0, hull.axis.y).dot(right);
                let half_w = hull.radius * (1.3 + hull.lumps);
                let margin = hull.height() * 0.2;
                let (lo, hi) = (Vec2::new(center - half_w, hull.base - margin), Vec2::new(center + half_w, hull.top + margin));
                let size = hi - lo;
                let pixel = size.max_element() / RESOLUTION as f32;
                let (w, h) = ((size.x / pixel).ceil() as usize + 1, (size.y / pixel).ceil() as usize + 1);
                let mut inside = vec![false; w * h];
                let mut inside_count = 0;
                for j in 0..h {
                    let y = lo.y + (j as f32 + 0.5) * pixel;
                    // The cross-section is star-shaped around the axis: its
                    // shadow is the span of its outline's projections.
                    let (mut a, mut b) = (0.0f32, 0.0f32);
                    for s in 0..64 {
                        let th = s as f32 / 64.0 * std::f32::consts::TAU;
                        let dir = Vec2::new(th.cos(), th.sin());
                        let x = hull.radius_toward(y, dir) * (dir.x * right.x + dir.y * right.z);
                        a = a.min(x);
                        b = b.max(x);
                    }
                    for i in 0..w {
                        let x = lo.x + (i as f32 + 0.5) * pixel - center;
                        if x >= a && x <= b && b > a {
                            inside[j * w + i] = true;
                            inside_count += 1;
                        }
                    }
                }
                let grid = Grid { right, origin: lo, pixel, w, h };
                View { grid, inside, hits: vec![0; w * h], inside_count, covered: 0, layers: 0, drawn: 0 }
            })
            .collect();
        Views { views }
    }

    /// Adds a triangle to every view.
    ///
    /// Complexity: O(views x pixels the triangle covers).
    pub fn add(&mut self, tri: [Vec3; 3]) {
        for v in &mut self.views {
            let View { grid, inside, hits, covered, layers, drawn, .. } = v;
            grid.scan(tri, |k| {
                if hits[k] == 0 {
                    *drawn += 1;
                    if inside[k] {
                        *covered += 1;
                    }
                }
                hits[k] = hits[k].saturating_add(1);
                *layers += 1;
            });
        }
    }

    /// Share of the pixels these triangles cover (in every view) that are
    /// still uncovered hull pixels: what adding them would gain.
    ///
    /// Complexity: O(views x pixels the triangles cover).
    pub fn gain(&self, tris: &[[Vec3; 3]]) -> f32 {
        let (mut fresh, mut total) = (0u32, 0u32);
        for v in &self.views {
            for tri in tris {
                v.grid.scan(*tri, |k| {
                    total += 1;
                    if v.inside[k] && v.hits[k] == 0 {
                        fresh += 1;
                    }
                });
            }
        }
        if total == 0 { 0.0 } else { fresh as f32 / total as f32 }
    }

    /// Mean share of the hull silhouette covered.
    ///
    /// Complexity: O(views).
    pub fn coverage(&self) -> f32 {
        let n = self.views.len().max(1) as f32;
        self.views.iter().map(|v| if v.inside_count == 0 { 0.0 } else { v.covered as f32 / v.inside_count as f32 }).sum::<f32>() / n
    }

    /// Mean triangle layers over drawn pixels.
    ///
    /// Complexity: O(views).
    pub fn overdraw(&self) -> f32 {
        let n = self.views.len().max(1) as f32;
        self.views.iter().map(|v| if v.drawn == 0 { 0.0 } else { v.layers as f32 / v.drawn as f32 }).sum::<f32>() / n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::CrownShape;

    #[test]
    fn a_wall_covers_the_hull_once() {
        let hull = Hull { axis: Vec2::ZERO, base: 2.0, top: 6.0, radius: 2.0, shape: CrownShape::Sphere, lumps: 0.0, seed: 1 };
        let mut v = Views::around(&hull, 1);
        assert!(v.coverage() == 0.0);
        let (a, b, c, d) = (Vec3::new(-3.0, 1.0, 0.0), Vec3::new(3.0, 1.0, 0.0), Vec3::new(3.0, 7.0, 0.0), Vec3::new(-3.0, 7.0, 0.0));
        assert!(v.gain(&[[a, b, c], [a, c, d]]) > 0.25, "the hull is a third of the wall");
        v.add([a, b, c]);
        v.add([a, c, d]);
        assert!(v.coverage() > 0.99, "{}", v.coverage());
        assert!((v.overdraw() - 1.0).abs() < 0.02, "{}", v.overdraw());
        assert_eq!(v.gain(&[[a, b, c]]), 0.0);
    }
}
