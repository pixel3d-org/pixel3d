//! The Session: current document, undo and redo, checkpoints, jobs.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use p3d_ops::{ErrorCode, OpError, Registry};
use p3d_scene::Document;

use crate::run::{BatchError, OpCall, OpResult, run_batch};

/// Undo history limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoLimits {
    /// Most undo steps kept.
    pub max_steps: usize,
    /// Approximate bytes of mesh data kept alive only by undo steps.
    pub max_bytes: usize,
}

impl Default for UndoLimits {
    fn default() -> Self {
        UndoLimits { max_steps: 500, max_bytes: 2 << 30 }
    }
}

/// A past state.
#[derive(Debug, Clone)]
struct Snapshot {
    label: String,
    doc: Arc<Document>,
    /// Mesh bytes this snapshot does not share with the next state.
    bytes: usize,
}

/// Options for [`Session::execute`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchOptions {
    /// Run everything, report, then throw the result away.
    pub dry_run: bool,
    /// Store the resulting state under this checkpoint name.
    pub checkpoint: Option<String>,
    /// Undo label; the op labels joined when omitted.
    pub label: Option<String>,
}

/// A committed or dry-run batch.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchResult {
    /// Document revision after the batch (unchanged for dry runs).
    pub revision: u64,
    pub results: Vec<OpResult>,
    pub elapsed: Duration,
    pub dry_run: bool,
}

/// One entry of the session history (undo list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub label: String,
    pub revision: u64,
}

/// Owns the current document and everything that changes it. Synchronous
/// and single-owner: frontends send requests to the thread that owns it.
#[derive(Debug)]
pub struct Session {
    doc: Arc<Document>,
    registry: Arc<Registry>,
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    undo_bytes: usize,
    checkpoints: BTreeMap<String, Arc<Document>>,
    limits: UndoLimits,
    seed: u64,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

/// Mesh bytes in `a` that `b` does not share.
///
/// Complexity: O(meshes).
fn unshared_bytes(a: &Document, b: &Document) -> usize {
    a.mesh_entries().filter(|(k, m)| !b.mesh_arc(*k).is_some_and(|o| Arc::ptr_eq(o, m))).map(|(_, m)| m.approx_bytes()).sum()
}

impl Session {
    /// An empty document with the built-in ops.
    pub fn new() -> Self {
        Self::with_registry(Arc::new(Registry::builtin()))
    }

    pub fn with_registry(registry: Arc<Registry>) -> Self {
        Session {
            doc: Arc::new(Document::new()),
            registry,
            undo: VecDeque::new(),
            redo: Vec::new(),
            undo_bytes: 0,
            checkpoints: BTreeMap::new(),
            limits: UndoLimits::default(),
            seed: 0,
        }
    }

    /// Replaces the document (after loading a file); clears undo, redo and checkpoints.
    pub fn load(&mut self, doc: Document) {
        self.doc = Arc::new(doc);
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.checkpoints.clear();
    }

    pub fn doc(&self) -> &Arc<Document> {
        &self.doc
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn revision(&self) -> u64 {
        self.doc.revision
    }

    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    pub fn set_limits(&mut self, limits: UndoLimits) {
        self.limits = limits;
        self.enforce_limits();
    }

    /// Runs op calls as one transaction: all succeed and commit together, or
    /// nothing changes. Each committed batch is one undo step.
    pub fn execute(&mut self, calls: &[OpCall], opts: &BatchOptions) -> Result<BatchResult, BatchError> {
        let start = Instant::now();
        let (doc, results) = run_batch(&self.registry, &self.doc, calls, self.seed, None, None)?;
        if opts.dry_run {
            return Ok(BatchResult { revision: self.doc.revision, results, elapsed: start.elapsed(), dry_run: true });
        }
        let label = opts.label.clone().unwrap_or_else(|| results.iter().map(|r| r.label.as_str()).collect::<Vec<_>>().join(", "));
        self.commit(doc, label);
        if let Some(name) = &opts.checkpoint {
            self.checkpoints.insert(name.clone(), Arc::clone(&self.doc));
        }
        Ok(BatchResult { revision: self.doc.revision, results, elapsed: start.elapsed(), dry_run: false })
    }

    /// Makes `doc` current as a new revision, pushing the old state to undo.
    fn commit(&mut self, mut doc: Document, label: String) {
        doc.revision = self.doc.revision + 1;
        let new = Arc::new(doc);
        let bytes = unshared_bytes(&self.doc, &new);
        let old = std::mem::replace(&mut self.doc, new);
        self.undo.push_back(Snapshot { label, doc: old, bytes });
        self.undo_bytes += bytes;
        self.redo.clear();
        self.enforce_limits();
    }

    /// Drops the oldest undo steps beyond the limits. Checkpoints are kept.
    fn enforce_limits(&mut self) {
        while self.undo.len() > self.limits.max_steps || (self.undo_bytes > self.limits.max_bytes && self.undo.len() > 1) {
            match self.undo.pop_front() {
                Some(s) => self.undo_bytes = self.undo_bytes.saturating_sub(s.bytes),
                None => break,
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Steps back one transaction. Returns its label.
    ///
    /// Complexity: O(meshes) for memory accounting; the swap itself is O(1).
    pub fn undo(&mut self) -> Result<String, OpError> {
        let s = self.undo.pop_back().ok_or_else(|| OpError::new(ErrorCode::InvalidParam, "nothing to undo"))?;
        self.undo_bytes = self.undo_bytes.saturating_sub(s.bytes);
        let bytes = unshared_bytes(&self.doc, &s.doc);
        let current = std::mem::replace(&mut self.doc, s.doc);
        self.redo.push(Snapshot { label: s.label.clone(), doc: current, bytes });
        Ok(s.label)
    }

    /// Re-applies the last undone transaction. Returns its label.
    ///
    /// Complexity: O(meshes) for memory accounting; the swap itself is O(1).
    pub fn redo(&mut self) -> Result<String, OpError> {
        let s = self.redo.pop().ok_or_else(|| OpError::new(ErrorCode::InvalidParam, "nothing to redo"))?;
        let bytes = unshared_bytes(&self.doc, &s.doc);
        let current = std::mem::replace(&mut self.doc, s.doc);
        self.undo.push_back(Snapshot { label: s.label.clone(), doc: current, bytes });
        self.undo_bytes += bytes;
        Ok(s.label)
    }

    /// Names the current state.
    pub fn checkpoint(&mut self, name: &str) {
        self.checkpoints.insert(name.to_owned(), Arc::clone(&self.doc));
    }

    /// Checkpoint names in sorted order.
    pub fn checkpoints(&self) -> impl Iterator<Item = &str> {
        self.checkpoints.keys().map(String::as_str)
    }

    /// Returns to a checkpoint as a new, undoable transaction.
    pub fn revert(&mut self, name: &str) -> Result<u64, OpError> {
        let target = self.checkpoints.get(name).cloned().ok_or_else(|| {
            OpError::new(ErrorCode::NotFound, format!("no checkpoint `{name}`")).with_suggestions(p3d_scene::name::suggest(
                name,
                self.checkpoints.keys().map(String::as_str),
                3,
            ))
        })?;
        self.commit((*target).clone(), format!("revert to {name}"));
        Ok(self.doc.revision)
    }

    /// The document at a revision still held by the session: the current
    /// one, an undo or redo step, or a checkpoint.
    ///
    /// Complexity: O(undo steps + redo steps + checkpoints).
    pub fn doc_at(&self, revision: u64) -> Option<Arc<Document>> {
        std::iter::once(&self.doc)
            .chain(self.undo.iter().map(|s| &s.doc))
            .chain(self.redo.iter().map(|s| &s.doc))
            .chain(self.checkpoints.values())
            .find(|d| d.revision == revision)
            .cloned()
    }

    /// The document stored under a checkpoint name.
    ///
    /// Complexity: O(log checkpoints).
    pub fn checkpoint_doc(&self, name: &str) -> Option<Arc<Document>> {
        self.checkpoints.get(name).cloned()
    }

    /// The seed ops of the next transaction derive theirs from.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Committed transactions still in the undo list, oldest first.
    pub fn history(&self) -> Vec<HistoryEntry> {
        self.undo.iter().map(|s| HistoryEntry { label: s.label.clone(), revision: s.doc.revision + 1 }).collect()
    }

    /// Bytes of mesh data kept alive only by undo steps (approximate).
    pub fn undo_bytes(&self) -> usize {
        self.undo_bytes
    }

    // ---- jobs ---------------------------------------------------------------------

    /// Starts a batch on the rayon pool against a snapshot of the current
    /// document. Poll it with [`Session::poll_job`].
    pub fn spawn_job(&self, calls: Vec<OpCall>, label: &str) -> Job {
        let (tx, rx) = mpsc::channel();
        let handle = JobHandle { progress: Arc::new(AtomicU32::new(0)), cancel: Arc::new(AtomicBool::new(false)) };
        let base = Arc::clone(&self.doc);
        spawn(Arc::clone(&self.registry), Arc::clone(&base), calls.clone(), self.seed, handle.clone(), tx);
        Job { calls, label: label.to_owned(), base_revision: base.revision, handle, rx, retries: 0 }
    }

    /// Checks a job without blocking. When it has finished on the current
    /// revision its result is committed as one transaction. When the document
    /// changed meanwhile, the job reruns on the new state (up to
    /// [`Job::MAX_RETRIES`] times) and `Pending` is returned.
    pub fn poll_job(&mut self, job: &mut Job) -> JobStatus {
        match job.rx.try_recv() {
            Ok(o) => self.finish_job(job, o),
            Err(mpsc::TryRecvError::Empty) => JobStatus::Pending(job.handle.progress()),
            Err(mpsc::TryRecvError::Disconnected) => JobStatus::Failed(vanished()),
        }
    }

    fn finish_job(&mut self, job: &mut Job, outcome: Outcome) -> JobStatus {
        match outcome {
            Err(e) => JobStatus::Failed(e),
            Ok((doc, results)) => {
                if job.base_revision != self.doc.revision {
                    if job.retries >= Job::MAX_RETRIES {
                        return JobStatus::Failed(BatchError {
                            index: 0,
                            op: String::new(),
                            error: Box::new(OpError::new(ErrorCode::Cancelled, "the document kept changing while the job ran").retryable()),
                        });
                    }
                    job.retries += 1;
                    job.base_revision = self.doc.revision;
                    let (tx, rx) = mpsc::channel();
                    job.rx = rx;
                    spawn(Arc::clone(&self.registry), Arc::clone(&self.doc), job.calls.clone(), self.seed, job.handle.clone(), tx);
                    return JobStatus::Pending(0.0);
                }
                self.commit(doc, job.label.clone());
                JobStatus::Done(BatchResult { revision: self.doc.revision, results, elapsed: Duration::ZERO, dry_run: false })
            }
        }
    }

    /// Blocks until the job finishes (or fails), committing its result.
    pub fn wait_job(&mut self, job: &mut Job) -> Result<BatchResult, BatchError> {
        loop {
            let outcome = job.rx.recv().map_err(|_| vanished())?;
            match self.finish_job(job, outcome) {
                JobStatus::Done(r) => return Ok(r),
                JobStatus::Failed(e) => return Err(e),
                JobStatus::Pending(_) => {}
            }
        }
    }
}

type Outcome = Result<(Document, Vec<OpResult>), BatchError>;

fn vanished() -> BatchError {
    BatchError { index: 0, op: String::new(), error: Box::new(OpError::internal("job thread vanished")) }
}

fn spawn(registry: Arc<Registry>, base: Arc<Document>, calls: Vec<OpCall>, seed: u64, handle: JobHandle, tx: mpsc::Sender<Outcome>) {
    rayon::spawn(move || {
        let _span = tracing::info_span!("job", ops = calls.len()).entered();
        let outcome = run_batch(&registry, &base, &calls, seed, Some(&handle.cancel), Some(&handle.progress));
        // The receiver may be gone (job dropped); nothing to do then.
        let _ = tx.send(outcome);
    });
}

/// Progress and cancellation of a running job; cheap to clone and share.
#[derive(Debug, Clone)]
pub struct JobHandle {
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

impl JobHandle {
    /// Fraction done in [0, 1], as reported by the running op.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1e6
    }

    /// Asks the running op to stop at its next check.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A batch running in the background.
#[derive(Debug)]
pub struct Job {
    calls: Vec<OpCall>,
    label: String,
    base_revision: u64,
    handle: JobHandle,
    rx: mpsc::Receiver<Outcome>,
    retries: u32,
}

impl Job {
    /// Reruns allowed when the document changes under a job.
    pub const MAX_RETRIES: u32 = 3;

    pub fn handle(&self) -> &JobHandle {
        &self.handle
    }
}

/// What [`Session::poll_job`] found.
#[derive(Debug)]
pub enum JobStatus {
    Pending(f32),
    Done(BatchResult),
    Failed(BatchError),
}
