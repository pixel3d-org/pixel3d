//! Half-edge mesh kernel: attributes, groups, topology queries, validation and triangle buffers.
//!
//! Layer L0 (foundation). See `docs/architecture.md`.

mod build;
mod mesh;
mod validate;

pub use build::BuildError;
pub use glam::{Vec2, Vec3, Vec4};
pub use mesh::{INVALID, Loop, Mesh};
pub use validate::ValidationError;
