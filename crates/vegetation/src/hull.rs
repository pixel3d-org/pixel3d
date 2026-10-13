//! The crown hull: a surface of revolution around a vertical axis that the
//! branches grow toward and the foliage fills. Its profile gives the
//! radius share at each height share of the crown.

use glam::{Vec2, Vec3};
use p3d_geo::deform::fbm;
use p3d_geo::generate::Rng;

/// The crown's outline, turned around the vertical axis.
#[derive(Debug, Clone, PartialEq)]
pub enum CrownShape {
    /// Round, as tall as wide.
    Sphere,
    /// Round, with its own height and width.
    Ellipsoid,
    /// Widest low down, a rounded top and a flatter underside (mature
    /// broadleaf trees).
    Dome,
    /// Widest at the bottom, pointed at the top (conifers).
    Cone,
    /// Flat underside and a low dome (acacia, stone pine, palms).
    Umbrella,
    /// A rounded cylinder (cypress, poplar).
    Column,
    /// A silhouette: points (radius share 0 to 1, height share 0 to 1 from
    /// the crown base), in order from the bottom to the top.
    Outline(Vec<Vec2>),
}

impl CrownShape {
    /// Shape words for parameters (outlines are given as points).
    pub const WORDS: [&'static str; 6] = ["sphere", "ellipsoid", "dome", "cone", "umbrella", "column"];

    pub fn as_str(&self) -> &'static str {
        match self {
            CrownShape::Sphere => "sphere",
            CrownShape::Ellipsoid => "ellipsoid",
            CrownShape::Dome => "dome",
            CrownShape::Cone => "cone",
            CrownShape::Umbrella => "umbrella",
            CrownShape::Column => "column",
            CrownShape::Outline(_) => "outline",
        }
    }

    pub fn parse(s: &str) -> Option<CrownShape> {
        Some(match s {
            "sphere" => CrownShape::Sphere,
            "ellipsoid" => CrownShape::Ellipsoid,
            "dome" => CrownShape::Dome,
            "cone" => CrownShape::Cone,
            "umbrella" => CrownShape::Umbrella,
            "column" => CrownShape::Column,
            _ => return None,
        })
    }

    /// Radius share at height share `t` (0 at the crown base, 1 at the
    /// top); at most 1.
    ///
    /// Complexity: O(1), O(points) for outlines.
    pub fn radius_at(&self, t: f32) -> f32 {
        if !(0.0..=1.0).contains(&t) {
            return 0.0;
        }
        let superellipse = |x: f32, p: f32| (1.0 - x.abs().powf(p)).max(0.0).powf(1.0 / p);
        match self {
            CrownShape::Sphere | CrownShape::Ellipsoid => superellipse(2.0 * t - 1.0, 2.0),
            CrownShape::Dome => {
                const WIDEST: f32 = 0.36;
                if t < WIDEST { superellipse((WIDEST - t) / WIDEST, 2.6) } else { superellipse((t - WIDEST) / (1.0 - WIDEST), 2.0) }
            }
            CrownShape::Cone => {
                const WIDEST: f32 = 0.08;
                if t < WIDEST { (t / WIDEST).sqrt() } else { (1.0 - t) / (1.0 - WIDEST) }
            }
            CrownShape::Umbrella => superellipse(t, 2.5) * (t / 0.04).min(1.0).sqrt(),
            CrownShape::Column => superellipse(2.0 * t - 1.0, 4.0),
            CrownShape::Outline(points) => outline_radius(points, t),
        }
    }
}

/// Piecewise linear radius of an outline, scaled so its widest point is 1.
///
/// Complexity: O(points).
fn outline_radius(points: &[Vec2], t: f32) -> f32 {
    let widest = points.iter().map(|p| p.x).fold(0.0f32, f32::max);
    if widest <= 0.0 || points.len() < 2 {
        return 0.0;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t >= a.y && t <= b.y {
            let s = if b.y - a.y > 1e-6 { (t - a.y) / (b.y - a.y) } else { 0.0 };
            return (a.x + (b.x - a.x) * s).max(0.0) / widest;
        }
    }
    0.0
}

/// A placed crown hull.
#[derive(Debug, Clone, PartialEq)]
pub struct Hull {
    /// Axis position (x, z).
    pub axis: Vec2,
    /// Height of the crown base and top.
    pub base: f32,
    pub top: f32,
    /// Widest radius.
    pub radius: f32,
    pub shape: CrownShape,
    /// Lumps of the crown's outline, as a share of the radius (0 smooth).
    pub lumps: f32,
    pub seed: u64,
}

impl Hull {
    pub fn height(&self) -> f32 {
        (self.top - self.base).max(1e-6)
    }

    /// Height share of `y` (0 at the base, 1 at the top).
    pub fn t(&self, y: f32) -> f32 {
        (y - self.base) / self.height()
    }

    /// Radius of the smooth profile at height `y` (0 outside the crown).
    ///
    /// Complexity: O(1), O(points) for outlines (as every query below).
    pub fn radius_at(&self, y: f32) -> f32 {
        self.radius * self.shape.radius_at(self.t(y))
    }

    /// Radius at height `y` toward the horizontal unit direction `dir`,
    /// with the lumps.
    pub fn radius_toward(&self, y: f32, dir: Vec2) -> f32 {
        let r = self.radius_at(y);
        if self.lumps <= 0.0 {
            return r;
        }
        let t = self.t(y);
        let q = Vec3::new(dir.x * 1.6, t * 2.4, dir.y * 1.6);
        r * (1.0 + self.lumps * (fbm(q, self.seed, 2) * 1.6).clamp(-1.0, 1.0))
    }

    /// The point on the axis at height `y`.
    pub fn axis_point(&self, y: f32) -> Vec3 {
        Vec3::new(self.axis.x, y, self.axis.y)
    }

    /// The crown's center (on the axis, halfway up).
    pub fn center(&self) -> Vec3 {
        self.axis_point((self.base + self.top) / 2.0)
    }

    /// Horizontal distance from the axis over the hull radius at that
    /// height: below 1 inside, 1 on the hull, above 1 outside (and 2 or
    /// more above or below the crown).
    pub fn share(&self, p: Vec3) -> f32 {
        let t = self.t(p.y);
        let d = Vec2::new(p.x, p.z).distance(self.axis);
        if !(0.0..=1.0).contains(&t) {
            let beyond = if t < 0.0 { -t } else { t - 1.0 };
            return 2.0 + beyond + d / self.radius.max(1e-6);
        }
        let dir = if d > 1e-9 { (Vec2::new(p.x, p.z) - self.axis) / d } else { Vec2::X };
        let r = self.radius_toward(p.y, dir);
        if r < 1e-6 { 2.0 + d / self.radius.max(1e-6) } else { d / r }
    }

    pub fn contains(&self, p: Vec3) -> bool {
        self.share(p) <= 1.0
    }

    /// Outward unit normal of the smooth hull's level sets at `p` (the
    /// gradient of the horizontal distance over the radius, minus the
    /// profile); the lumps are left out, so normals stay soft.
    pub fn normal(&self, p: Vec3) -> Vec3 {
        let h = 0.01;
        let t = self.t(p.y).clamp(0.0, 1.0);
        let (a, b) = ((t - h).max(0.0), (t + h).min(1.0));
        let slope = (self.shape.radius_at(b) - self.shape.radius_at(a)) / (b - a).max(1e-6);
        let off = Vec2::new(p.x, p.z) - self.axis;
        let d = off.length();
        let radial = if d > 1e-6 { off / d } else { Vec2::ZERO };
        // The gradient of d / R - profile(t): radial / R and -profile'(t) / height.
        let n = Vec3::new(radial.x / self.radius.max(1e-6), -slope / self.height(), radial.y / self.radius.max(1e-6));
        let n = n.normalize_or_zero();
        if n == Vec3::ZERO { if t > 0.5 { Vec3::Y } else { Vec3::NEG_Y } } else { n }
    }

    /// Volume, by integrating discs over the height.
    ///
    /// Complexity: O(1) (64 steps; outlines O(points) per step).
    pub fn volume(&self) -> f32 {
        const STEPS: usize = 64;
        let mut sum = 0.0;
        for i in 0..STEPS {
            let t = (i as f32 + 0.5) / STEPS as f32;
            let r = self.radius * self.shape.radius_at(t);
            sum += std::f32::consts::PI * r * r;
        }
        sum * self.height() / STEPS as f32
    }

    /// Area of the side silhouette (the profile's region in a vertical
    /// plane through the axis).
    ///
    /// Complexity: O(1).
    pub fn side_area(&self) -> f32 {
        const STEPS: usize = 64;
        let sum: f32 = (0..STEPS).map(|i| self.shape.radius_at((i as f32 + 0.5) / STEPS as f32)).sum();
        2.0 * self.radius * sum * self.height() / STEPS as f32
    }

    /// A uniformly random point inside, by rejection from the bounding
    /// cylinder (at most 64 tries; the center when every try misses).
    ///
    /// Complexity: O(1) expected.
    pub fn sample(&self, rng: &mut Rng) -> Vec3 {
        for _ in 0..64 {
            let r = self.radius * (1.0 + self.lumps) * rng.unit().sqrt();
            let a = rng.unit() * std::f32::consts::TAU;
            let y = self.base + rng.unit() * self.height();
            let p = Vec3::new(self.axis.x + r * a.cos(), y, self.axis.y + r * a.sin());
            if self.contains(p) {
                return p;
            }
        }
        self.center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(shape: CrownShape) -> Hull {
        Hull { axis: Vec2::new(1.0, 2.0), base: 4.0, top: 12.0, radius: 6.0, shape, lumps: 0.0, seed: 1 }
    }

    #[test]
    fn profiles_peak_at_one_and_close_at_the_ends() {
        for s in [CrownShape::Sphere, CrownShape::Ellipsoid, CrownShape::Dome, CrownShape::Cone, CrownShape::Umbrella, CrownShape::Column] {
            let peak = (0..=200).map(|i| s.radius_at(i as f32 / 200.0)).fold(0.0f32, f32::max);
            assert!((peak - 1.0).abs() < 0.02, "{s:?} peak {peak}");
            assert!(s.radius_at(1.0) < 0.05, "{s:?} top");
            assert_eq!(s.radius_at(1.2), 0.0);
        }
        let o = CrownShape::Outline(vec![Vec2::new(0.0, 0.0), Vec2::new(2.0, 0.4), Vec2::new(1.0, 1.0)]);
        assert!((o.radius_at(0.4) - 1.0).abs() < 1e-6);
        assert!((o.radius_at(0.7) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn shares_normals_and_samples_agree() {
        let h = hull(CrownShape::Dome);
        let c = h.center();
        assert!(h.contains(c));
        assert!(!h.contains(Vec3::new(20.0, 8.0, 0.0)));
        assert!(!h.contains(Vec3::new(1.0, 1.0, 2.0)), "below the crown");
        let side = Vec3::new(1.0 + h.radius_at(7.0), 7.0, 2.0);
        assert!((h.share(side) - 1.0).abs() < 1e-4);
        assert!(h.normal(side).x > 0.8, "{:?}", h.normal(side));
        assert!(h.normal(Vec3::new(1.0, 11.9, 2.0)).y > 0.9);
        let mut rng = Rng::new(3);
        for _ in 0..200 {
            assert!(h.contains(h.sample(&mut rng)));
        }
        let s = hull(CrownShape::Sphere);
        let want = 4.0 / 3.0 * std::f32::consts::PI * 36.0 * 4.0;
        assert!((s.volume() - want).abs() / want < 0.01, "{} vs {want}", s.volume());
        let area = std::f32::consts::PI * 6.0 * 4.0;
        assert!((s.side_area() - area).abs() / area < 0.01);
    }
}
