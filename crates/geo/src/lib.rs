//! Geometry kernels: generators, edit operations, normals, subdivision, deformers and cleanup.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.

pub mod bvh;
pub mod curve;
pub mod deform;
pub mod edit;
pub mod kdtree;
pub mod prim;
pub mod shade;
pub mod subd;
