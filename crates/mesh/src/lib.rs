//! Half-edge mesh kernel: attributes, groups, topology queries, validation and triangle buffers.
//!
//! Layer L0 (foundation). See `docs/architecture.md`.

mod attrs;
mod build;
mod combine;
mod edit;
mod fill;
mod groups;
mod mesh;
mod normals;
mod patch;
mod raw;
mod tri;
mod validate;

pub use attrs::{EdgeFlags, MAX_UV_LAYERS};
pub use build::BuildError;
pub use edit::{DeleteReport, EditError, Remap};
pub use fixedbitset::FixedBitSet;
pub use glam::{Vec2, Vec3, Vec4};
pub use groups::{ElementKind, Group, GroupError};
pub use mesh::{INVALID, Loop, Mesh};
pub use patch::{CornerSrc, Patch, PatchResult};
pub use raw::{RawGroup, RawMesh};
pub use tri::{MAX_EAR_CLIP_CORNERS, Submesh, TriBuffers, TriOptions};
pub use validate::ValidationError;
