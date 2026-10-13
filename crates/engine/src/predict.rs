//! Diagnostics with predicted fix results: each fix runs on a copy of the
//! document and the copy is diagnosed again (project spec 17.10).

use std::collections::BTreeSet;
use std::time::Instant;

use p3d_ops::{ErrorCode, OpError, Registry};
use p3d_perceive::diagnose::{Finding, diagnose};
use p3d_perceive::{PerceiveError, View};
use p3d_scene::Document;

use crate::run::{OpCall, run_batch};
use crate::session::{BatchOptions, BatchResult, Session};

/// Fixes predicted per call; later ones say how to get a prediction.
pub const MAX_PREDICTIONS: usize = 8;

/// A perception error as an op error (same codes).
pub fn op_error(e: PerceiveError) -> OpError {
    let code = match e.code {
        "NOT_FOUND" => ErrorCode::NotFound,
        "AMBIGUOUS_NAME" => ErrorCode::AmbiguousName,
        "OUT_OF_RANGE" => ErrorCode::OutOfRange,
        "EMPTY_SELECTION" => ErrorCode::EmptySelection,
        _ => ErrorCode::InvalidParam,
    };
    let mut o = OpError::new(code, e.message).with_suggestions(e.suggestions);
    o.path = e.path;
    o.hint = e.hint;
    o
}

type Key = (&'static str, String, Option<String>);

fn key(f: &Finding) -> Key {
    (f.code, f.part.clone(), f.other.clone())
}

/// What a fix would do to the findings, in one line.
///
/// Complexity: the fix op plus one [`diagnose`] of the view.
fn predict(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64, f: &Finding, k: usize, before: &BTreeSet<Key>) -> String {
    let fix = &f.fixes[k];
    let call = OpCall { op: fix.op.clone(), args: fix.args.clone(), label: Some(format!("fix_{}", f.id.to_lowercase())) };
    let start = Instant::now();
    let after = match run_batch(registry, doc, &[call], seed, None, None, None) {
        Ok((d, _)) => d,
        Err(e) => return format!("fails: {} {}", e.error.code.as_str(), e.error.message),
    };
    let view = match View::new(&after, target) {
        Ok(v) => v,
        Err(e) => return format!("the target is gone ({})", e.message),
    };
    let found = diagnose(&view);
    let mut out = match found.iter().find(|g| key(g) == key(f)) {
        None => "resolved".to_owned(),
        Some(g) => format!("remains: {}", g.message),
    };
    let new: Vec<String> = found.iter().filter(|g| !before.contains(&key(g))).take(2).map(|g| format!("{} {}", g.code, g.part)).collect();
    if !new.is_empty() {
        out.push_str(&format!("; new {}", new.join(", ")));
    }
    tracing::debug!(finding = %f.id, ms = start.elapsed().as_millis() as u64, "fix predicted");
    out
}

/// Findings of a view of `doc`, with the predicted result of the first
/// fix of the first [`MAX_PREDICTIONS`] findings that have one.
///
/// Complexity: one [`diagnose`] plus, per predicted fix, the op and one
/// more [`diagnose`].
pub fn diagnose_predicted(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64) -> Result<Vec<Finding>, PerceiveError> {
    let view = View::new(doc, target)?;
    let mut findings = diagnose(&view);
    let before: BTreeSet<Key> = findings.iter().map(key).collect();
    let mut budget = MAX_PREDICTIONS;
    for f in &mut findings {
        if f.fixes.is_empty() {
            continue;
        }
        let p = if budget > 0 {
            budget -= 1;
            predict(registry, doc, target, seed, f, 0, &before)
        } else {
            format!("not predicted (diagnose with target {} predicts it)", f.part)
        };
        f.fixes[0].predicted = Some(p);
    }
    Ok(findings)
}

/// Predicts one fix of one finding (for `diagnose explain`).
///
/// Complexity: one [`diagnose`] plus the op and one more [`diagnose`].
pub fn explain(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64, id: &str) -> Result<Finding, OpError> {
    let view = View::new(doc, target).map_err(op_error)?;
    let findings = diagnose(&view);
    let before: BTreeSet<Key> = findings.iter().map(key).collect();
    let mut f = find(&findings, id)?.clone();
    for k in 0..f.fixes.len() {
        let p = predict(registry, doc, target, seed, &f, k, &before);
        f.fixes[k].predicted = Some(p);
    }
    Ok(f)
}

fn find<'a>(findings: &'a [Finding], id: &str) -> Result<&'a Finding, OpError> {
    findings.iter().find(|f| f.id.eq_ignore_ascii_case(id)).ok_or_else(|| {
        OpError::new(ErrorCode::NotFound, format!("no finding {id} at this revision"))
            .at("/fix")
            .with_suggestions(findings.iter().take(5).map(|f| format!("{} {}", f.id, f.code)).collect())
            .with_hint("finding ids are per revision: run diagnose again after an edit")
    })
}

/// Applies fix `option` (0 is the first) of finding `id` of the current
/// revision as one undoable transaction.
///
/// Complexity: one [`diagnose`] plus the fix op.
pub fn apply_fix(session: &mut Session, target: Option<&str>, id: &str, option: usize) -> Result<(Finding, BatchResult), OpError> {
    let doc = std::sync::Arc::clone(session.doc());
    let view = View::new(&doc, target).map_err(op_error)?;
    let findings = diagnose(&view);
    let f = find(&findings, id)?.clone();
    let fix = f.fixes.get(option).ok_or_else(|| {
        OpError::new(ErrorCode::OutOfRange, format!("{} has {} fix option(s)", f.id, f.fixes.len()))
            .at("/option")
            .with_hint(f.hint.clone().unwrap_or_else(|| "options are numbered from 0".into()))
    })?;
    let call = OpCall { op: fix.op.clone(), args: fix.args.clone(), label: Some(format!("fix_{}", f.id.to_lowercase())) };
    let opts = BatchOptions { label: Some(format!("fix {} {} {}", f.id, f.code, f.part)), ..BatchOptions::default() };
    let r = session.execute(&[call], &opts).map_err(|e| *e.error)?;
    Ok((f, r))
}
