//! The signed distance field of a closed mesh: the distance to the nearest
//! triangle (a BVH query), negative inside (ray parity). Meshing it with
//! [`crate::mesh`] resamples any closed mesh into even quads: a voxel
//! remesh that also repairs bad topology and evens out long thin faces.

use glam::{Affine3A, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;

use crate::mesh::Field;

/// The distance field of a closed mesh.
pub struct MeshField {
    bvh: TriBvh,
    lo: Vec3,
    hi: Vec3,
    reach: f32,
}

impl MeshField {
    /// Builds the BVH of the mesh's triangles. `None` for an empty mesh.
    ///
    /// Complexity: O(n log n) for n triangles.
    pub fn new(mesh: &Mesh) -> Option<MeshField> {
        let bvh = TriBvh::from_mesh(mesh, Affine3A::IDENTITY);
        let (lo, hi) = bvh.bounds()?;
        Some(MeshField { reach: (hi - lo).length() * 2.0 + 1.0, bvh, lo, hi })
    }
}

impl Field for MeshField {
    /// Complexity: O(log n) typical (a closest-point query and a parity test).
    fn eval(&self, p: Vec3) -> f32 {
        let d = self.bvh.closest(p, self.reach).map_or(self.reach, |c| c.dist);
        if self.bvh.contains(p) { -d } else { d }
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::mesh::{MeshSpec, mesh};
    use glam::DVec3;
    use p3d_geo::prim::{CylinderSpec, Origin, cylinder};

    #[test]
    fn a_thin_faced_prism_resamples_into_even_quads() {
        let post = cylinder(&CylinderSpec { radius: 0.05, height: 1.0, segments: 8, ..CylinderSpec::default() }, Origin::Bottom).unwrap();
        let f = MeshField::new(&post).unwrap();
        assert!(f.eval(Vec3::new(0.0, 0.5, 0.0)) < 0.0);
        assert!((f.eval(Vec3::new(0.2, 0.5, 0.0)) - 0.15).abs() < 0.01);
        let r = mesh(&f, &MeshSpec { voxel: 0.01 }).unwrap();
        let m = &r.mesh;
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // Edges about one voxel long everywhere: no long thin faces left.
        let longest = m
            .edges()
            .map(|e| {
                let [a, b] = m.edge_verts(e);
                m.positions[a as usize].distance(m.positions[b as usize])
            })
            .fold(0.0f32, f32::max);
        assert!(longest < 0.025, "{longest}");
        let _ = DVec3::ZERO;
    }
}
