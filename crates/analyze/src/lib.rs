//! Asset metrics and validation.
//!
//! Layer L3 (IO and analysis). See `docs/architecture.md`.
//!
//! [`topology`] measures a mesh the way the benchmark analyzer does: element
//! counts, face kinds, poles, angles, holes, parts, orientation and
//! degenerate elements. Every function here is a pure read of its input.

mod topology;

pub use topology::{BoundaryLoop, Part, Parts, SHORT_EDGE, SLIVER_RATIO, Topology, boundary_loops, face_degenerate, parts, topology};
