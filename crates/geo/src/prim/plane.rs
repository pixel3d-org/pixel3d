//! Flat grid in the XZ plane, facing +Y.

use glam::DVec3;
use p3d_mesh::Mesh;

use super::{MAX_SEGMENTS, PrimError, check_segments, check_size, even_grid};

/// A grid of quads centered on the origin in the XZ plane, normal +Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneSpec {
    /// Size along X and Z in meters.
    pub size_x: f64,
    pub size_z: f64,
    /// Quads along X and Z.
    pub segments_x: u32,
    pub segments_z: u32,
}

impl Default for PlaneSpec {
    fn default() -> Self {
        PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: 1, segments_z: 1 }
    }
}

/// Generates a plane. Vertices are row-major along X; no groups, no sharp
/// edges.
///
/// Complexity: O(segments_x x segments_z).
pub fn plane(spec: &PlaneSpec) -> Result<Mesh, PrimError> {
    check_size("size_x", spec.size_x)?;
    check_size("size_z", spec.size_z)?;
    check_segments("segments_x", spec.segments_x, 1, MAX_SEGMENTS)?;
    check_segments("segments_z", spec.segments_z, 1, MAX_SEGMENTS)?;
    let (nx, nz) = (spec.segments_x, spec.segments_z);
    let xs = even_grid(spec.size_x / 2.0, nx);
    let zs = even_grid(spec.size_z / 2.0, nz);
    let w = nx + 1;
    let mut positions = Vec::with_capacity((w * (nz + 1)) as usize);
    for &z in &zs {
        for &x in &xs {
            positions.push(DVec3::new(x, 0.0, z).as_vec3());
        }
    }
    let quads = (nx * nz) as usize;
    let mut corners = Vec::with_capacity(4 * quads);
    for j in 0..nz {
        for i in 0..nx {
            let a = j * w + i;
            // (i, j), (i, j + 1), (i + 1, j + 1), (i + 1, j): normal +Y.
            corners.extend([a, a + w, a + w + 1, a + 1]);
        }
    }
    Ok(Mesh::from_polygons(positions, &vec![4; quads], &corners)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn counts_and_orientation() {
        let m = plane(&PlaneSpec { size_x: 2.0, size_z: 1.0, segments_x: 4, segments_z: 2 }).unwrap();
        m.validate().unwrap();
        assert_eq!((m.vert_count(), m.face_count(), m.triangle_count()), (15, 8, 16));
        assert!(m.faces().all(|f| m.face_normal(f).abs_diff_eq(Vec3::Y, 1e-6)));
        let (lo, hi) = m.bounds().unwrap();
        assert_eq!((lo, hi), (Vec3::new(-1.0, 0.0, -0.5), Vec3::new(1.0, 0.0, 0.5)));
    }

    #[test]
    fn rejects_bad_params() {
        assert!(plane(&PlaneSpec { segments_x: 0, ..PlaneSpec::default() }).is_err());
        assert!(plane(&PlaneSpec { size_z: 0.0, ..PlaneSpec::default() }).is_err());
        assert!(plane(&PlaneSpec { size_x: f64::NAN, ..PlaneSpec::default() }).is_err());
    }
}
