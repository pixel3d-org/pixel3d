//! Session, transactions, undo, jobs, events and fix prediction.
//!
//! Layer L4 (engine). See `docs/architecture.md`.
//!
//! The [`Session`] owns the current document. Every [`Session::execute`]
//! call is one transaction: ops run on a copy-on-write working copy, are
//! validated, and commit together or not at all. Undo and redo swap
//! snapshots; jobs run batches on the rayon pool against a snapshot.

pub mod predict;
mod run;
mod session;

pub use run::{BatchError, OpCall, OpResult, op_seed, run_batch};
pub use session::{BatchOptions, BatchResult, Bisected, HistoryEntry, Job, JobHandle, JobStatus, Session, UndoLimits};

/// Every built-in op, the spec ops included: the registry sessions use.
///
/// Complexity: O(ops).
pub fn registry() -> p3d_ops::Registry {
    p3d_spec::registry()
}
