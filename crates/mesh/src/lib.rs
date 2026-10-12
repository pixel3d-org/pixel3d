//! Half-edge mesh kernel: attributes, groups, topology queries, validation and triangle buffers.
//!
//! Layer L0 (foundation). See `docs/architecture.md`.

mod attrs;
mod build;
mod groups;
mod mesh;
mod validate;

pub use attrs::{EdgeFlags, MAX_UV_LAYERS};
pub use build::BuildError;
pub use fixedbitset::FixedBitSet;
pub use glam::{Vec2, Vec3, Vec4};
pub use groups::{ElementKind, Group, GroupError};
pub use mesh::{INVALID, Loop, Mesh};
pub use validate::ValidationError;
