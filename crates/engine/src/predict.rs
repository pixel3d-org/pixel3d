//! Diagnostics with predicted fix results: each fix runs on a copy of the
//! document and the copy is diagnosed again (project spec 17.10). Findings
//! are the full set: mesh-level ones from `p3d-perceive` plus intent, edge
//! flow at landmarks and spec-first fixes from `p3d-spec`.

use std::collections::BTreeSet;
use std::time::Instant;

use p3d_ops::{ErrorCode, OpError, OpFlags, Registry};
use p3d_perceive::diagnose::{Finding, Severity, decal};
use p3d_perceive::fmt::thousands;
use p3d_perceive::{PerceiveError, View};
use p3d_scene::Document;
use p3d_spec::diagnose::diagnose;
use serde_json::Value;

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

/// The long-running op a fix runs, if any: the fix's own op, or an op an
/// edit layer it adds to a spec runs (remeshes take seconds).
///
/// Complexity: O(size of the fix's arguments).
fn slow_op(registry: &Registry, fix: &p3d_perceive::diagnose::Fix) -> Option<String> {
    fn ops(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(op) = m.get("op").and_then(Value::as_str) {
                    out.push(op.to_owned());
                }
                m.values().for_each(|x| ops(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| ops(x, out)),
            _ => {}
        }
    }
    let mut all = vec![fix.op.clone()];
    ops(&fix.args, &mut all);
    all.into_iter().find(|op| registry.get(op).is_ok_and(|o| o.meta().flags.contains(OpFlags::LONG_RUNNING)))
}

/// Triangles a game would draw: parts that are not decals or hidden.
///
/// Complexity: O(parts).
fn game_tris(view: &View) -> usize {
    view.parts.iter().filter(|p| !decal(view, p)).map(|p| p.mesh.triangle_count()).sum()
}

/// Strands coverage of a part, from its strands report.
fn coverage(view: &View, part: &str) -> Option<f64> {
    let p = view.parts.iter().find(|p| p.label == part)?;
    view.doc.object(p.key)?.metadata.get("strands")?.get("coverage")?.as_f64()
}

/// The state before the fixes: finding keys and drawn triangles.
struct Before {
    keys: BTreeSet<Key>,
    tris: usize,
}

impl Before {
    fn of(view: &View, findings: &[Finding]) -> Before {
        Before { keys: findings.iter().map(key).collect(), tris: game_tris(view) }
    }
}

/// What a fix would do to the findings, in one line: resolved or what
/// remains, the measure it changes (strands coverage), the change in drawn
/// triangles and findings it would add.
///
/// Complexity: the fix op plus one [`diagnose`] of the view.
fn predict(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64, f: &Finding, k: usize, before: &Before) -> String {
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
    if f.code == "BALD_SPOT"
        && let (Some(was), Some(now)) = (f.measured, coverage(&view, &f.part))
    {
        out.push_str(&format!("; coverage {}% -> {}%", was.round() as i64, (now * 100.0).round() as i64));
    }
    let tris = game_tris(&view);
    if tris != before.tris {
        let sign = if tris > before.tris { "+" } else { "-" };
        out.push_str(&format!("; tris {sign}{}", thousands(tris.abs_diff(before.tris))));
    }
    let new: Vec<String> = found.iter().filter(|g| !before.keys.contains(&key(g))).take(2).map(|g| format!("{} {}", g.code, g.part)).collect();
    if !new.is_empty() {
        out.push_str(&format!("; new {}", new.join(", ")));
    }
    tracing::debug!(finding = %f.id, ms = start.elapsed().as_millis() as u64, "fix predicted");
    out
}

/// Findings of a view of `doc`, with predicted results for the first
/// [`MAX_PREDICTIONS`] fixes of errors and warnings: each finding's first
/// fix, and when that one would not resolve it, its next options until one
/// would, which then moves to the front (the top fix is the best predicted
/// one). Fixes of info findings and long-running fixes (remeshes) are left
/// to `explain`, which predicts every option.
///
/// Complexity: one [`diagnose`] plus, per predicted fix, the op and one
/// more [`diagnose`].
pub fn diagnose_predicted(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64) -> Result<Vec<Finding>, PerceiveError> {
    let view = View::new(doc, target)?;
    let mut findings = diagnose(&view);
    let before = Before::of(&view, &findings);
    let mut budget = MAX_PREDICTIONS;
    for f in &mut findings {
        if f.fixes.is_empty() {
            continue;
        }
        if budget == 0 {
            f.fixes[0].predicted = Some(format!("not predicted (diagnose with target {} predicts it)", f.part));
            continue;
        }
        if f.severity == Severity::Info {
            f.fixes[0].predicted = Some(format!("not predicted for info (explain {} predicts it)", f.id));
            continue;
        }
        for k in 0..f.fixes.len() {
            if budget == 0 {
                break;
            }
            if let Some(op) = slow_op(registry, &f.fixes[k]) {
                f.fixes[k].predicted = Some(format!("not predicted ({op} takes long; explain {} predicts it)", f.id));
                continue;
            }
            budget -= 1;
            let p = predict(registry, doc, target, seed, f, k, &before);
            let resolved = p.starts_with("resolved");
            f.fixes[k].predicted = Some(p);
            if resolved {
                f.fixes[..=k].rotate_right(1);
                break;
            }
        }
    }
    Ok(findings)
}

/// Predicts every fix of one finding (for `diagnose explain`), in the
/// order [`diagnose_predicted`] lists them.
///
/// Complexity: [`diagnose_predicted`] plus, per fix not yet predicted, the
/// op and one more [`diagnose`].
pub fn explain(registry: &Registry, doc: &Document, target: Option<&str>, seed: u64, id: &str) -> Result<Finding, OpError> {
    let view = View::new(doc, target).map_err(op_error)?;
    let findings = diagnose_predicted(registry, doc, target, seed).map_err(op_error)?;
    let before = Before::of(&view, &findings);
    let mut f = find(&findings, id)?.clone();
    for k in 0..f.fixes.len() {
        if f.fixes[k].predicted.as_deref().is_none_or(|p| p.starts_with("not predicted")) {
            let p = predict(registry, doc, target, seed, &f, k, &before);
            f.fixes[k].predicted = Some(p);
        }
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

/// Applies fix `option` (0 is the first, as [`diagnose_predicted`] lists
/// them: the best predicted first) of finding `id` of the current revision
/// as one undoable transaction.
///
/// Complexity: [`diagnose_predicted`] plus the fix op.
pub fn apply_fix(session: &mut Session, target: Option<&str>, id: &str, option: usize) -> Result<(Finding, BatchResult), OpError> {
    let doc = std::sync::Arc::clone(session.doc());
    let findings = diagnose_predicted(session.registry(), &doc, target, session.seed()).map_err(op_error)?;
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
