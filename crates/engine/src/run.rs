//! The batch runner: applies op calls to a working copy, all or nothing.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::time::{Duration, Instant};

use p3d_ops::{ErrorCode, OpCtx, OpError, OpOutput, Registry, Tx};
use p3d_scene::{Document, ObjectKey, OpNode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One op call in a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpCall {
    /// Op id, for example `prim.box`.
    pub op: String,
    /// Op parameters; `null` or missing means all defaults.
    #[serde(default)]
    pub args: Value,
    /// History label; `<op name>_<n>` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl OpCall {
    pub fn new(op: &str, args: Value) -> Self {
        OpCall { op: op.to_owned(), args, label: None }
    }
}

/// The result of one op in a committed (or dry-run) batch.
#[derive(Debug, Clone, PartialEq)]
pub struct OpResult {
    pub op: String,
    pub label: String,
    pub output: OpOutput,
    /// Canonical parameters (defaults filled in, units converted).
    pub params: Value,
    pub elapsed: Duration,
}

/// A failed batch: which op failed and why. Nothing was committed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("op {index} ({op}) failed: {error}")]
pub struct BatchError {
    pub index: usize,
    pub op: String,
    /// The error, with its path prefixed by `/ops/<index>/args`.
    pub error: Box<OpError>,
}

/// Totals over a set of objects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Totals {
    verts: i64,
    faces: i64,
    tris: i64,
}

impl Totals {
    fn add(&mut self, o: Totals) {
        self.verts += o.verts;
        self.faces += o.faces;
        self.tris += o.tris;
    }
}

/// Complexity: O(objects x (faces + corners)) over the given objects.
fn totals(doc: &Document, keys: impl Iterator<Item = ObjectKey>) -> Totals {
    let mut t = Totals::default();
    for k in keys {
        if let Some(m) = doc.object_mesh(k) {
            t.verts += m.vert_count() as i64;
            t.faces += m.face_count() as i64;
            t.tris += m.triangle_count() as i64;
        }
    }
    t
}

fn short_name(op: &str) -> &str {
    op.rsplit('.').next().unwrap_or(op)
}

/// Text of a panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// Seed of op `index` in the batch committed as `revision`: the same calls
/// on the same document always get the same seeds.
pub fn op_seed(session_seed: u64, revision: u64, index: usize) -> u64 {
    // SplitMix64 step over the inputs.
    let mut z = session_seed ^ revision.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (index as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Runs a batch on a copy of `base`. Every op either succeeds or the whole
/// batch fails; panics inside ops become `INTERNAL` errors. In debug builds
/// every touched mesh and the document indexes are validated after each op.
///
/// Complexity: the ops' own costs plus, per op, O(size of each affected mesh)
/// for the metric deltas and O(history length) per affected object (the
/// history is copied when a snapshot shares it; see ADR 0002).
pub fn run_batch(
    registry: &Registry,
    base: &Document,
    calls: &[OpCall],
    seed: u64,
    cancel: Option<&AtomicBool>,
    progress: Option<&AtomicU32>,
) -> Result<(Document, Vec<OpResult>), BatchError> {
    let mut tx = Tx::new(base.clone());
    let mut results = Vec::with_capacity(calls.len());
    let mut known: BTreeMap<ObjectKey, Totals> = BTreeMap::new();
    for (i, call) in calls.iter().enumerate() {
        let span = tracing::info_span!("op", id = %call.op, index = i);
        let _guard = span.enter();
        let fail = |error: OpError| BatchError { index: i, op: call.op.clone(), error: Box::new(error) };
        let op = registry.get(&call.op).map_err(|e| fail(e.at(&format!("/ops/{i}/op"))))?;
        let ctx = OpCtx { seed: op_seed(seed, base.revision, i), cancel, progress };
        ctx.check_cancel().map_err(|e| fail(e.at(&format!("/ops/{i}"))))?;

        let objects_before = tx.doc().object_count() as i64;
        let start = Instant::now();
        let mut applied = match catch_unwind(AssertUnwindSafe(|| op.run(&mut tx, &call.args, &ctx))) {
            Ok(Ok(a)) => a,
            Ok(Err(e)) => return Err(fail(e.prefixed(&format!("/ops/{i}/args")))),
            Err(payload) => {
                let msg = panic_message(&*payload);
                tracing::error!(op = %call.op, "op panicked: {msg}");
                return Err(fail(OpError::new(ErrorCode::Internal, format!("op panicked: {msg}")).at(&format!("/ops/{i}"))));
            }
        };
        let elapsed = start.elapsed();

        // Every object this op changed, created or edited through the Tx.
        let mut affected: Vec<ObjectKey> = applied.output.changed.iter().chain(applied.output.created.iter()).copied().collect();
        affected.extend(tx.touched().iter().copied());
        affected.sort_unstable();
        affected.dedup();
        affected.retain(|k| tx.doc().object(*k).is_some());

        if cfg!(debug_assertions) {
            for &k in &affected {
                if let Some(m) = tx.doc().object_mesh(k) {
                    m.validate().map_err(|e| fail(OpError::internal(format!("{} left an invalid mesh: {e}", call.op)).at(&format!("/ops/{i}"))))?;
                }
            }
            tx.doc().check().map_err(|e| fail(OpError::internal(format!("{} left inconsistent indexes: {e}", call.op)).at(&format!("/ops/{i}"))))?;
        }

        // Metric deltas against the state before this op. An object changed by an
        // earlier op in the batch has cached totals; otherwise it still matches
        // the base (or is new). No snapshot is taken, so no mesh is copied twice.
        let mut before = Totals::default();
        let mut after = Totals::default();
        for &k in &affected {
            let b = known.get(&k).copied().unwrap_or_else(|| totals(base, std::iter::once(k).filter(|k| base.object(*k).is_some())));
            let a = totals(tx.doc(), std::iter::once(k));
            known.insert(k, a);
            before.add(b);
            after.add(a);
        }
        applied.output.metrics.verts = after.verts - before.verts;
        applied.output.metrics.faces = after.faces - before.faces;
        applied.output.metrics.tris = after.tris - before.tris;
        applied.output.metrics.objects = tx.doc().object_count() as i64 - objects_before;

        let label = call.label.clone().unwrap_or_else(|| format!("{}_{}", short_name(&call.op), i + 1));
        let version = op.meta().version;
        for &k in &affected {
            let node = OpNode { op: call.op.clone(), version, params: applied.params.clone(), label: label.clone() };
            tx.doc_mut().push_history(k, node).map_err(|e| fail(OpError::internal(e.to_string())))?;
        }
        tracing::debug!(op = %call.op, ?elapsed, "applied");
        results.push(OpResult { op: call.op.clone(), label, output: applied.output, params: applied.params, elapsed });
    }
    Ok((tx.into_doc(), results))
}
