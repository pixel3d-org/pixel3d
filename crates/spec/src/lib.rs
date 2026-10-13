//! Brief and Model Spec: parts, anchors, landmarks, joins and the
//! incremental compiler with provenance (project spec 17.3, ADR 0010).
//!
//! Layer L4 (engine). See `docs/architecture.md`.

use std::sync::Arc;

use p3d_ops::{Registry, Typed};

pub mod brief;
pub mod compile;
pub mod landmarks;
pub mod model;
pub mod ops;
pub mod patch;
pub mod surface;

pub use brief::{Brief, Coverage, coverage, parse_brief, proportions};
pub use compile::{CompileReport, LandmarkPoint, PartReport, PartStatus, compile, face_provenance, fingerprint, part_landmarks};
pub use model::{ModelSpec, Part, parse};

/// A problem in a spec, at a JSON pointer into it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct SpecError {
    pub path: String,
    pub message: String,
}

impl SpecError {
    pub fn new(path: &str, message: impl Into<String>) -> SpecError {
        SpecError { path: path.to_owned(), message: message.into() }
    }
}

/// Adds the spec ops to a registry.
///
/// Complexity: O(ops).
pub fn register(r: &mut Registry) {
    for op in [
        Arc::new(Typed::<ops::SpecSet>::new()) as Arc<dyn p3d_ops::DynOp>,
        Arc::new(Typed::<ops::SpecPatch>::new()),
        Arc::new(Typed::<ops::SpecCompile>::new()),
        Arc::new(Typed::<brief::BriefSet>::new()),
    ] {
        // Ids are unique by construction; a clash would be a programming error
        // caught by the registry tests.
        let _ = r.register(op);
    }
}

/// The built-in ops plus the spec ops: the registry sessions use.
///
/// Complexity: O(ops).
pub fn registry() -> Registry {
    let mut r = Registry::builtin();
    register(&mut r);
    r
}
