//! Remeshing: field-aligned quad remeshing of closed surfaces.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.

pub mod field;
pub mod isotropic;
pub mod poles;
pub mod quad;

pub use field::Guide;
pub use isotropic::{IsotropicSpec, isotropic};
pub use quad::{MAX_SAMPLES, QuadReport, QuadSpec, RemeshError, quad_remesh};
