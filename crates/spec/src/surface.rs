//! Surfaces of placed parts for anchors and landmarks (ADR 0010), queried
//! in the root's space: a mesh through its BVH, an SDF part by sphere
//! tracing its field (so anchoring on an SDF part never needs its mesh).

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
pub struct Surface {
    kind: Kind,
    to_root: Affine3A,
    to_part: Affine3A,
    /// Bounds in the part's own space.
    lo: Vec3,
    hi: Vec3,
}

enum Kind {
    /// Triangles in root space; `flip` when the placement mirrors.
    Mesh { bvh: Box<TriBvh>, flip: bool },
    /// The field in part space.
    Sdf(Arc<Sdf>),
}

/// Sphere-tracing steps before giving up.
const MAX_STEPS: u32 = 400;

impl Surface {
    /// Complexity: O(n log n) for the BVH over n triangles.
    pub fn of_mesh(mesh: &Mesh, to_root: Affine3A) -> Option<Surface> {
        let (lo, hi) = mesh.bounds()?;
        let bvh = TriBvh::from_mesh(mesh, to_root);
        Some(Surface {
            kind: Kind::Mesh { bvh: Box::new(bvh), flip: to_root.matrix3.determinant() < 0.0 },
            to_root,
            to_part: to_root.inverse(),
            lo,
            hi,
        })
    }

    /// Complexity: O(1); queries cost O(steps x items).
    pub fn of_sdf(sdf: Arc<Sdf>, to_root: Affine3A) -> Surface {
        let (lo, hi) = sdf.bounds();
        Surface { kind: Kind::Sdf(sdf), to_root, to_part: to_root.inverse(), lo, hi }
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

    /// The first hit of a ray (root space).
    ///
    /// Complexity: O(log n) for meshes, O(steps x items) for SDFs.
    pub fn ray(&self, origin: Vec3, dir: Vec3) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        match &self.kind {
            Kind::Mesh { bvh, flip } => {
                let h = bvh.raycast(origin, dir, f32::MAX)?;
                let n = bvh.normal(h.tri);
                Some(Hit { point: h.point, normal: if *flip { -n } else { n } })
            }
            Kind::Sdf(sdf) => {
                let o = self.to_part.transform_point3(origin);
                let d = self.to_part.transform_vector3(dir);
                let scale = d.length();
                let d = d / scale;
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

    /// The surface point nearest `p` (root space).
    ///
    /// Complexity: O(log n) typical for meshes, O(items) for SDFs.
    pub fn nearest(&self, p: Vec3) -> Option<Hit> {
        match &self.kind {
            Kind::Mesh { bvh, flip } => {
                let c = bvh.closest(p, f32::MAX)?;
                let n = bvh.normal(c.tri);
                Some(Hit { point: c.point, normal: if *flip { -n } else { n } })
            }
            Kind::Sdf(sdf) => {
                let mut q = self.to_part.transform_point3(p);
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
        // Normals move by the inverse transpose of the placement.
        let n = self.to_part.matrix3.transpose() * g;
        Hit { point: self.to_root.transform_point3(p), normal: n.normalize_or(Vec3::Y) }
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

    #[test]
    fn sdf_and_mesh_surfaces_agree() {
        let sdf = Arc::new(Sdf::new(vec![Item::prim(Prim::Sphere { radius: 0.5 }, Vec3::ZERO)]).unwrap());
        let place = Affine3A::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(1.0, 2.0, 0.0));
        let s = Surface::of_sdf(sdf, place);
        let hit = s.ray(Vec3::new(1.0, 5.0, 0.0), Vec3::NEG_Y).unwrap();
        assert!(hit.point.distance(Vec3::new(1.0, 2.5, 0.0)) < 1e-3, "{hit:?}");
        assert!(hit.normal.distance(Vec3::Y) < 1e-2);
        let top = s.toward(Vec3::Y).unwrap();
        assert!(top.point.distance(Vec3::new(1.0, 2.5, 0.0)) < 1e-3);
        let near = s.nearest(Vec3::new(3.0, 2.0, 0.0)).unwrap();
        assert!(near.point.distance(Vec3::new(1.5, 2.0, 0.0)) < 1e-3, "{near:?}");

        let ball =
            p3d_geo::prim::uv_sphere(&p3d_geo::prim::UvSphereSpec { radius: 0.5, segments: 64, rings: 32 }, p3d_geo::prim::Origin::Center).unwrap();
        let m = Surface::of_mesh(&ball, place).unwrap();
        let hit = m.ray(Vec3::new(1.0, 5.0, 0.0), Vec3::NEG_Y).unwrap();
        assert!(hit.point.distance(Vec3::new(1.0, 2.5, 0.0)) < 1e-3, "{hit:?}");
        assert!(hit.normal.y > 0.99);
    }

    #[test]
    fn mirrored_mesh_normals_point_out() {
        let ball =
            p3d_geo::prim::uv_sphere(&p3d_geo::prim::UvSphereSpec { radius: 0.5, segments: 16, rings: 8 }, p3d_geo::prim::Origin::Center).unwrap();
        let m = Surface::of_mesh(&ball, Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0))).unwrap();
        let hit = m.ray(Vec3::new(2.0, 0.0, 0.0), Vec3::NEG_X).unwrap();
        assert!(hit.normal.x > 0.9, "{hit:?}");
    }
}
