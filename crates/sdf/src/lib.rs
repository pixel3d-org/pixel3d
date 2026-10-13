//! Signed distance field trees, evaluation and narrow-band meshing.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.
//!
//! Agents compose organic forms from a list of named primitives joined by
//! hard or smooth operators ([`tree`]); [`mesh`] turns the zero set into a
//! closed quad mesh, sampling only near the surface.

pub mod from_mesh;
pub mod mesh;
pub mod tree;

pub use from_mesh::MeshField;
pub use mesh::{BLOCK, Field, MeshSpec, SdfMesh, mesh};
pub use tree::{Combine, Item, MAX_DEPTH, MAX_ITEMS, Modifiers, Noise, Prim, Sdf, SdfError, Shape};
