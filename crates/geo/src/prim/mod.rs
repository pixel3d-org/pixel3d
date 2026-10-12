//! Primitive generators. Every primitive has clean quad topology, real-world
//! dimensions, semantic face groups and sharp flags on its hard edges.
//!
//! Axes: +Y up, +Z front. "Left" and "right" are as seen from the front:
//! right is +X.

mod cuboid;
mod cylinder;
mod plane;
mod soup;
mod sphere;

pub use cuboid::{BoxSpec, cuboid};
pub use cylinder::{CapKind, CylinderSpec, cylinder};
pub use plane::{PlaneSpec, plane};
pub use sphere::{QuadSphereSpec, quad_sphere};

use p3d_mesh::{BuildError, Mesh};

/// Largest segment count along one axis of a primitive.
pub const MAX_SEGMENTS: u32 = 4096;
/// Largest number of bevel segments.
pub const MAX_BEVEL_SEGMENTS: u32 = 64;
/// Smallest size of a primitive along any axis (0.1 mm).
pub const MIN_SIZE: f64 = 1e-4;
/// Largest size of a primitive along any axis (10 km).
pub const MAX_SIZE: f64 = 1e4;

/// Where the object origin sits relative to the primitive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Bounding box center.
    Center,
    /// Center of the bottom face: the primitive stands on the ground plane.
    #[default]
    Bottom,
}

/// Why a primitive could not be generated.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PrimError {
    #[error("{param} is {value}: {reason}")]
    InvalidParam { param: &'static str, value: String, reason: String },
    #[error("generated mesh is invalid: {0}")]
    Build(#[from] BuildError),
}

pub(crate) fn check_size(param: &'static str, v: f64) -> Result<(), PrimError> {
    if v.is_finite() && (MIN_SIZE..=MAX_SIZE).contains(&v) {
        Ok(())
    } else {
        Err(PrimError::InvalidParam { param, value: format!("{v}"), reason: format!("must be between {MIN_SIZE} m and {MAX_SIZE} m") })
    }
}

pub(crate) fn check_segments(param: &'static str, v: u32, min: u32, max: u32) -> Result<(), PrimError> {
    if (min..=max).contains(&v) {
        Ok(())
    } else {
        Err(PrimError::InvalidParam { param, value: v.to_string(), reason: format!("must be between {min} and {max}") })
    }
}

/// Moves a mesh centered on the origin so that `origin` holds, given its
/// half height.
pub(crate) fn place(mesh: &mut Mesh, origin: Origin, half_height: f64) {
    if origin == Origin::Bottom {
        let dy = half_height as f32;
        for p in &mut mesh.positions {
            p.y += dy;
        }
    }
}

/// `n + 1` evenly spaced values from `-a` to `a` with exact, symmetric ends.
pub(crate) fn even_grid(a: f64, n: u32) -> Vec<f64> {
    (0..=n)
        .map(|i| match i {
            0 => -a,
            _ if i == n => a,
            _ if 2 * i > n => a - 2.0 * a * f64::from(n - i) / f64::from(n),
            _ => -a + 2.0 * a * f64::from(i) / f64::from(n),
        })
        .collect()
}
