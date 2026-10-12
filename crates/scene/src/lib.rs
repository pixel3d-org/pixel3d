//! Document model: objects, hierarchy, materials, textures, skeletons, clips, ids, names and settings.
//!
//! Layer L1 (document). See `docs/architecture.md`.

mod document;
mod history;
pub mod name;
mod object;
mod settings;
#[cfg(test)]
mod tests;

pub use document::{Document, SceneError};
pub use history::{OpGraph, OpNode};
pub use object::{GraphKey, MeshKey, Object, ObjectKey, ObjectKind};
pub use settings::{AssetClass, DisplayUnits, DocSettings};
