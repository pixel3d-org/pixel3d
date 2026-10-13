//! Surfaces of placed parts for anchors, landmarks and constraints (ADR
//! 0010), queried in the root's space. A mesh keeps its BVH in part space
//! and an SDF part is sphere traced on its field, so both move with their
//! placement for free (the constraint solver moves parts many times), and
//! anchoring on an SDF part never needs its mesh.

use std::sync::Arc;

use glam::{Affine3A, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use p3d_sdf::Sdf;

/// A surface point and its outward normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
}

/// A placed part's surface.
#[derive(Clone)]
pub struct Surface {
    kind: Kind,
    to_root: Affine3A,
    to_part: Affine3A,
    /// Bounds in the part's own space.
    lo: Vec3,
    hi: Vec3,
}

#[derive(Clone)]
enum Kind {
    /// Triangles in part space.
    Mesh(Arc<TriBvh>),
    /// The field in part space.
    Sdf(Arc<Sdf>),
}

/// Sphere-tracing steps before giving up.
const MAX_STEPS: u32 = 400;
/// Rays per side of the grid that finds an SDF part's extreme point.
const EXTREME_GRID: usize = 9;

impl Surface {
    /// Complexity: O(n log n) for the BVH over n triangles.
    pub fn of_mesh(mesh: &Mesh, to_root: Affine3A) -> Option<Surface> {
        let (lo, hi) = mesh.bounds()?;
        let bvh = TriBvh::from_mesh(mesh, Affine3A::IDENTITY);
        Some(Surface { kind: Kind::Mesh(Arc::new(bvh)), to_root, to_part: to_root.inverse(), lo, hi })
    }

    /// Complexity: O(1); queries cost O(steps x items).
    pub fn of_sdf(sdf: Arc<Sdf>, to_root: Affine3A) -> Surface {
        let (lo, hi) = sdf.bounds();
        Surface { kind: Kind::Sdf(sdf), to_root, to_part: to_root.inverse(), lo, hi }
    }

    /// The same surface at another placement.
    ///
    /// Complexity: O(1).
    pub fn placed(&self, to_root: Affine3A) -> Surface {
        Surface { kind: self.kind.clone(), to_root, to_part: to_root.inverse(), lo: self.lo, hi: self.hi }
    }

    /// Bounds in the part's own space.
    pub fn part_bounds(&self) -> (Vec3, Vec3) {
        (self.lo, self.hi)
    }

    pub fn to_root(&self) -> Affine3A {
        self.to_root
    }

    /// The size of the part (its bounds diagonal, part space).
    fn size(&self) -> f32 {
        (self.hi - self.lo).length().max(1e-6)
    }

    fn flipped(&self) -> bool {
        self.to_root.matrix3.determinant() < 0.0
    }

    /// A part-space normal in root space.
    fn normal_to_root(&self, n: Vec3) -> Vec3 {
        let w = (self.to_part.matrix3.transpose() * n).normalize_or(Vec3::Y);
        if self.flipped() { -w } else { w }
    }

    /// The first hit of a ray (root space).
    ///
    /// Complexity: O(log n) for meshes, O(steps x items) for SDFs.
    pub fn ray(&self, origin: Vec3, dir: Vec3) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        let o = self.to_part.transform_point3(origin);
        let d = self.to_part.transform_vector3(dir).try_normalize()?;
        match &self.kind {
            Kind::Mesh(bvh) => {
                let h = bvh.raycast(o, d, f32::MAX)?;
                // Mesh normals already point out; mirroring flips the winding.
                let n = self.to_part.matrix3.transpose() * bvh.normal(h.tri);
                Some(Hit { point: self.to_root.transform_point3(h.point), normal: n.normalize_or(Vec3::Y) })
            }
            Kind::Sdf(sdf) => {
                let (t0, t1) = ray_box(o, d, self.lo, self.hi)?;
                let eps = self.size() * 1e-5;
                let lip = sdf.lipschitz().max(1.0);
                let mut t = t0.max(0.0);
                for _ in 0..MAX_STEPS {
                    if t > t1 {
                        return None;
                    }
                    let p = o + d * t;
                    let dist = sdf.eval(p);
                    if dist < eps {
                        return Some(self.sdf_hit(sdf, p));
                    }
                    t += (dist / lip).max(eps * 0.5);
                }
                None
            }
        }
    }

    /// The surface point nearest `p` (root space; measured in part space,
    /// so non-uniform scale bends it slightly).
    ///
    /// Complexity: O(log n) typical for meshes, O(items) for SDFs.
    pub fn nearest(&self, p: Vec3) -> Option<Hit> {
        let q = self.to_part.transform_point3(p);
        match &self.kind {
            Kind::Mesh(bvh) => {
                let c = bvh.closest(q, f32::MAX)?;
                let n = self.to_part.matrix3.transpose() * bvh.normal(c.tri);
                Some(Hit { point: self.to_root.transform_point3(c.point), normal: n.normalize_or(Vec3::Y) })
            }
            Kind::Sdf(sdf) => {
                let mut q = q;
                let h = self.size() * 1e-4;
                for _ in 0..16 {
                    let d = sdf.eval(q);
                    let g = sdf.gradient(q, h).try_normalize()?;
                    q -= g * d;
                    if d.abs() < h * 0.1 {
                        break;
                    }
                }
                Some(self.sdf_hit(sdf, q))
            }
        }
    }

    fn sdf_hit(&self, sdf: &Sdf, p: Vec3) -> Hit {
        let g = sdf.gradient(p, self.size() * 1e-4);
        let mut n = self.normal_to_root(g);
        // A mirrored field still has its outside where it is positive.
        if self.flipped() {
            n = -n;
        }
        Hit { point: self.to_root.transform_point3(p), normal: n }
    }

    /// The outermost surface point in direction `dir` (part space) from the
    /// part's center, scaled to its bounding ellipsoid: a ray from outside
    /// through that ellipsoid point toward the center.
    ///
    /// Complexity: one ray query.
    pub fn toward(&self, dir: Vec3) -> Option<Hit> {
        let center = (self.lo + self.hi) * 0.5;
        let half = ((self.hi - self.lo) * 0.5).max(Vec3::splat(1e-6));
        let target = center + dir.try_normalize()? * half;
        let outside = center + (target - center) * 3.0;
        let (o, c) = (self.to_root.transform_point3(outside), self.to_root.transform_point3(center));
        self.ray(o, c - o)
    }

    /// The surface point furthest along root direction `d`: every triangle
    /// corner for meshes, a grid of rays for SDF parts.
    ///
    /// Complexity: O(triangles) for meshes, O(81 rays) for SDFs.
    pub fn extreme(&self, d: Vec3) -> Option<Vec3> {
        let d = d.try_normalize()?;
        match &self.kind {
            Kind::Mesh(bvh) => (0..bvh.len() as u32)
                .flat_map(|t| bvh.triangle(t))
                .map(|p| self.to_root.transform_point3(p))
                .max_by(|a, b| a.dot(d).total_cmp(&b.dot(d))),
            Kind::Sdf(_) => {
                let (lo, hi) = self.root_bounds();
                let span = (hi - lo).length() + 1.0;
                let (u, v) = (d.any_orthonormal_vector(), d.cross(d.any_orthonormal_vector()));
                let center = (lo + hi) * 0.5;
                let half = (hi - lo).length() * 0.5;
                let mut best: Option<Vec3> = None;
                for i in 0..EXTREME_GRID {
                    for j in 0..EXTREME_GRID {
                        let s = |k: usize| (k as f32 / (EXTREME_GRID - 1) as f32 * 2.0 - 1.0) * half;
                        let origin = center + u * s(i) + v * s(j) + d * span;
                        if let Some(h) = self.ray(origin, -d)
                            && best.is_none_or(|b| h.point.dot(d) > b.dot(d))
                        {
                            best = Some(h.point);
                        }
                    }
                }
                best
            }
        }
    }

    /// About `n` points spread over the surface (root space): triangle
    /// centroids for meshes, rays from a Fibonacci sphere for SDF parts.
    ///
    /// Complexity: O(n) queries.
    pub fn samples(&self, n: usize) -> Vec<Vec3> {
        match &self.kind {
            Kind::Mesh(bvh) => {
                let step = (bvh.len() / n.max(1)).max(1);
                (0..bvh.len() as u32).step_by(step).map(|t| self.to_root.transform_point3(bvh.triangle(t).iter().sum::<Vec3>() / 3.0)).collect()
            }
            Kind::Sdf(_) => {
                let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
                (0..n)
                    .filter_map(|i| {
                        let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
                        let r = (1.0 - y * y).sqrt();
                        let a = golden * i as f32;
                        self.toward(Vec3::new(r * a.cos(), y, r * a.sin())).map(|h| h.point)
                    })
                    .collect()
            }
        }
    }

    /// The part's bounds corners in root space.
    pub fn root_bounds(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for i in 0..8 {
            let c = Vec3::new(
                if i & 1 == 0 { self.lo.x } else { self.hi.x },
                if i & 2 == 0 { self.lo.y } else { self.hi.y },
                if i & 4 == 0 { self.lo.z } else { self.hi.z },
            );
            let w = self.to_root.transform_point3(c);
            lo = lo.min(w);
            hi = hi.max(w);
        }
        (lo, hi)
    }
}

/// Entry and exit distances of a ray through a box.
fn ray_box(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> Option<(f32, f32)> {
    let inv = d.recip();
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let t0 = a.min(b).max_element();
    let t1 = a.max(b).min_element();
    (t1 >= t0.max(0.0)).then_some((t0, t1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use glam::Quat;
    use p3d_sdf::{Item, Prim};

    use super::*;

    fn ball() -> Mesh {
        p3d_geo::prim::uv_sphere(&p3d_geo::prim::UvSphereSpec { radius: 0.5, segments: 64, rings: 32 }, p3d_geo::prim::Origin::Center).unwrap()
    }

    #[test]
    fn sdf_and_mesh_surfaces_agree() {
        let sdf = Arc::new(Sdf::new(vec![Item::prim(Prim::Sphere { radius: 0.5 }, Vec3::ZERO)]).unwrap());
        let place = Affine3A::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(1.0, 2.0, 0.0));
        let s = Surface::of_sdf(sdf, place);
        let m = Surface::of_mesh(&ball(), place).unwrap();
        for surf in [&s, &m] {
            let hit = surf.ray(Vec3::new(1.0, 5.0, 0.0), Vec3::NEG_Y).unwrap();
            assert!(hit.point.distance(Vec3::new(1.0, 2.5, 0.0)) < 1e-3, "{hit:?}");
            assert!(hit.normal.y > 0.99, "{hit:?}");
            let top = surf.toward(Vec3::Y).unwrap();
            assert!(top.point.distance(Vec3::new(1.0, 2.5, 0.0)) < 1e-3);
            let near = surf.nearest(Vec3::new(3.0, 2.0, 0.0)).unwrap();
            assert!(near.point.distance(Vec3::new(1.5, 2.0, 0.0)) < 5e-3, "{near:?}");
            let low = surf.extreme(Vec3::NEG_Y).unwrap();
            assert!((low.y - 1.5).abs() < 2e-3, "{low}");
            assert!(surf.samples(32).iter().all(|p| (p.distance(Vec3::new(1.0, 2.0, 0.0)) - 0.5).abs() < 2e-2));
        }
        // Moving is free and queries follow.
        let moved = m.placed(Affine3A::from_translation(Vec3::new(0.0, 10.0, 0.0)));
        assert!((moved.extreme(Vec3::Y).unwrap().y - 10.5).abs() < 1e-3);
    }

    #[test]
    fn mirrored_surfaces_keep_normals_out() {
        let mirror = Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let m = Surface::of_mesh(&ball(), mirror).unwrap();
        let hit = m.ray(Vec3::new(2.0, 0.0, 0.0), Vec3::NEG_X).unwrap();
        assert!(hit.normal.x > 0.9, "{hit:?}");
        let sdf = Arc::new(Sdf::new(vec![Item::prim(Prim::Sphere { radius: 0.5 }, Vec3::ZERO)]).unwrap());
        let s = Surface::of_sdf(sdf, mirror);
        let hit = s.ray(Vec3::new(2.0, 0.0, 0.0), Vec3::NEG_X).unwrap();
        assert!(hit.normal.x > 0.9, "{hit:?}");
    }
}
