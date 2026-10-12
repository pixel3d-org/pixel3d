//! Expectation checks: tests for models (project spec 17.11). Every check
//! is a pure read of the document with a clear pass or fail message that
//! carries the measured values.

use p3d_scene::{Check, Document, Expectation, ObjectRef, Range};
use serde::Serialize;

use crate::Report;
use crate::fmt::{dims, mm, thousands};
use crate::relations::{RelationKind, relate};
use crate::sample::surface_samples;
use crate::view::View;

/// Most parts one `not_intersecting` check compares pairwise.
pub const MAX_NOT_INTERSECTING: usize = 64;
/// Surface samples per side for symmetry checks.
const SYMMETRY_SAMPLES: usize = 512;

/// The result of one expectation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub name: String,
    /// `attached(lid, crate)`.
    pub label: String,
    pub pass: bool,
    /// Measured values, and why it fails.
    pub message: String,
}

/// The current path of a referenced object: by id (survives renames), else
/// by the written name.
fn path(doc: &Document, r: &ObjectRef) -> Result<String, String> {
    doc.object_by_id(r.id)
        .and_then(|k| doc.path(k))
        .or_else(|| doc.resolve(&r.name).ok().and_then(|k| doc.path(k)))
        .ok_or_else(|| format!("{} (o:{}) no longer exists", r.name, r.id))
}

fn range_text(r: &Range) -> String {
    match (r.min, r.max) {
        (Some(a), Some(b)) => format!("{} to {} mm", mm(a as f32), mm(b as f32)),
        (Some(a), None) => format!("at least {} mm", mm(a as f32)),
        (None, Some(b)) => format!("at most {} mm", mm(b as f32)),
        (None, None) => "any".into(),
    }
}

/// Largest distance from surface samples of `from` parts, mirrored across
/// the plane `axis = at`, to the nearest surface of `onto` parts (capped).
///
/// Complexity: O(samples x parts x (log n + k)).
fn mirror_deviation(view: &View, from: &[usize], onto: &[usize], axis: usize, at: f32, cap: f32) -> f32 {
    let per = (SYMMETRY_SAMPLES / from.len().max(1)).max(16);
    let mut worst = 0.0f32;
    for &i in from {
        for s in surface_samples(&view.parts[i], per) {
            let mut m = s.point;
            m[axis] = 2.0 * at - m[axis];
            let d = onto.iter().filter_map(|&j| view.parts[j].bvh().closest(m, cap).map(|c| c.dist)).fold(cap, f32::min);
            worst = worst.max(d);
            if worst >= cap {
                return cap;
            }
        }
    }
    worst
}

/// Runs one expectation.
///
/// Complexity: bounded sampled queries for `symmetric`, one pair relation
/// for `attached`, O(p^2) pair relations for `not_intersecting` (p is at
/// most [`MAX_NOT_INTERSECTING`]), O(n) for `dims`, `budget` and
/// `watertight`.
pub fn check(doc: &Document, e: &Expectation) -> Outcome {
    let label = e.check.label();
    let result = run(doc, &e.check);
    let (pass, message) = match result {
        Ok((pass, m)) => (pass, m),
        Err(m) => (false, m),
    };
    Outcome { name: e.name.clone(), label, pass, message }
}

fn run(doc: &Document, c: &Check) -> Result<(bool, String), String> {
    match c {
        Check::Dims { target, width, height, depth } => {
            let p = path(doc, target)?;
            let view = View::new(doc, Some(&p)).map_err(|e| e.message)?;
            let size = view.size();
            let mut bad = Vec::new();
            for (name, r, v) in [("width", width, size.x), ("height", height, size.y), ("depth", depth, size.z)] {
                if let Some(r) = r
                    && !r.contains(f64::from(v))
                {
                    bad.push(format!("{name} {} mm, want {}", mm(v), range_text(r)));
                }
            }
            let m = if bad.is_empty() { dims(size) } else { format!("{}; {}", dims(size), bad.join("; ")) };
            Ok((bad.is_empty(), m))
        }
        Check::Symmetric { a, b, axis, at, tolerance } => {
            let k = match axis {
                'x' => 0,
                'y' => 1,
                _ => 2,
            };
            let tol = *tolerance as f32;
            match b {
                Some(b) => {
                    let (pa, pb) = (path(doc, a)?, path(doc, b)?);
                    let view = View::new(doc, None).map_err(|e| e.message)?;
                    let i = view.part(&pa, "/a").map_err(|e| e.message)?;
                    let j = view.part(&pb, "/b").map_err(|e| e.message)?;
                    let plane = at.map_or(0.0, |v| v as f32);
                    let cap = view.diagonal() * 0.1 + tol;
                    let d = mirror_deviation(&view, &[i], &[j], k, plane, cap).max(mirror_deviation(&view, &[j], &[i], k, plane, cap));
                    let over = if d >= cap { "over " } else { "" };
                    Ok((d <= tol, format!("mirror across {axis}={} mm deviates {over}{} mm (tolerance {} mm)", mm(plane), mm(d), mm(tol))))
                }
                None => {
                    let pa = path(doc, a)?;
                    let view = View::new(doc, Some(&pa)).map_err(|e| e.message)?;
                    let all: Vec<usize> = (0..view.parts.len()).collect();
                    let plane = at.map_or((view.lo[k] + view.hi[k]) / 2.0, |v| v as f32);
                    let cap = view.diagonal() * 0.1 + tol;
                    let d = mirror_deviation(&view, &all, &all, k, plane, cap);
                    let over = if d >= cap { "over " } else { "" };
                    Ok((d <= tol, format!("mirror across {axis}={} mm deviates {over}{} mm (tolerance {} mm)", mm(plane), mm(d), mm(tol))))
                }
            }
        }
        Check::Attached { part, to, max_gap, max_penetration } => {
            let (pp, pt) = (path(doc, part)?, path(doc, to)?);
            let view = View::new(doc, None).map_err(|e| e.message)?;
            let i = view.part(&pp, "/part").map_err(|e| e.message)?;
            let j = view.part(&pt, "/to").map_err(|e| e.message)?;
            let tol = (*max_gap as f32).max(1e-6);
            let far = view.diagonal() * 4.0 + 1.0;
            let Some((n, rel)) = relate(&view, i, j, tol, far, far) else {
                return Ok((false, "no surface to measure".into()));
            };
            let (gap, depth) = match &rel {
                Some(r) if r.kind == RelationKind::Penetrating => (0.0, r.dist),
                Some(r) if r.kind == RelationKind::Inside => (0.0, f32::INFINITY),
                _ => (n.dist, 0.0),
            };
            let mut bad = Vec::new();
            if f64::from(gap) > *max_gap {
                bad.push(format!("gap {} mm > {} mm", mm(gap), mm(*max_gap as f32)));
            }
            if let Some(mp) = max_penetration
                && f64::from(depth) > *mp
            {
                if depth.is_infinite() {
                    bad.push("one lies inside the other".into());
                } else {
                    bad.push(format!("sinks {} mm > {} mm", mm(depth), mm(*mp as f32)));
                }
            }
            let state = if depth > 0.0 && depth.is_finite() { format!("sinks {} mm", mm(depth)) } else { format!("gap {} mm", mm(gap)) };
            let m = if bad.is_empty() { state } else { bad.join("; ") };
            Ok((bad.is_empty(), m))
        }
        Check::NotIntersecting { parts } => {
            if parts.len() > MAX_NOT_INTERSECTING {
                return Err(format!("{} parts; at most {MAX_NOT_INTERSECTING} per check", parts.len()));
            }
            let view = View::new(doc, None).map_err(|e| e.message)?;
            let mut idx = Vec::new();
            for r in parts {
                let p = path(doc, r)?;
                idx.push(view.part(&p, "/parts").map_err(|e| e.message)?);
            }
            let tol = view.contact_tolerance();
            let far = view.diagonal() * 4.0 + 1.0;
            let mut bad = Vec::new();
            for x in 0..idx.len() {
                for y in x + 1..idx.len() {
                    let (i, j) = (idx[x], idx[y]);
                    if let Some((_, Some(r))) = relate(&view, i, j, tol, 0.0, far)
                        && matches!(r.kind, RelationKind::Penetrating | RelationKind::Inside)
                    {
                        let how = if r.kind == RelationKind::Inside { "inside".to_owned() } else { format!("cross by {} mm", mm(r.dist)) };
                        bad.push(format!("{} and {} {how}", view.parts[i].label, view.parts[j].label));
                    }
                }
            }
            let m = if bad.is_empty() { format!("{} parts, none pass through each other", idx.len()) } else { bad.join("; ") };
            Ok((bad.is_empty(), m))
        }
        Check::Budget { target, min_tris, max_tris } => {
            let p = target.as_ref().map(|t| path(doc, t)).transpose()?;
            let view = View::new(doc, p.as_deref()).map_err(|e| e.message)?;
            let tris: u64 = view.parts.iter().map(|p| p.topology().tris as u64).sum();
            let ok = tris <= *max_tris && min_tris.is_none_or(|m| tris >= m);
            let want = match min_tris {
                Some(m) => format!("{} to {}", thousands(*m as usize), thousands(*max_tris as usize)),
                None => format!("at most {}", thousands(*max_tris as usize)),
            };
            Ok((ok, format!("{} tris, want {want}", thousands(tris as usize))))
        }
        Check::Watertight { target } => {
            let p = path(doc, target)?;
            let view = View::new(doc, Some(&p)).map_err(|e| e.message)?;
            let mut bad = Vec::new();
            for part in &view.parts {
                let t = part.topology();
                if t.boundary_loops > 0 {
                    bad.push(format!("{} has {} hole{}", part.label, t.boundary_loops, if t.boundary_loops == 1 { "" } else { "s" }));
                }
                if t.inverted_parts > 0 {
                    bad.push(format!("{} has {} inward piece{}", part.label, t.inverted_parts, if t.inverted_parts == 1 { "" } else { "s" }));
                }
            }
            if view.parts.is_empty() {
                bad.push("no geometry".into());
            }
            let m = if bad.is_empty() {
                format!("{} part{} closed and facing out", view.parts.len(), if view.parts.len() == 1 { "" } else { "s" })
            } else {
                bad.join("; ")
            };
            Ok((bad.is_empty(), m))
        }
    }
}

/// Runs every expectation of the document, in order.
///
/// Complexity: the sum of the checks.
pub fn check_all(doc: &Document) -> Vec<Outcome> {
    doc.expectations().iter().map(|e| check(doc, e)).collect()
}

/// The line every response ends with: `tests 3/4 pass; failing:
/// attached(lid, crate)`. Empty when there are no expectations.
pub fn tests_line(outcomes: &[Outcome]) -> String {
    if outcomes.is_empty() {
        return String::new();
    }
    let pass = outcomes.iter().filter(|o| o.pass).count();
    let failing: Vec<String> = outcomes.iter().filter(|o| !o.pass).take(4).map(|o| format!("{} {}", o.name, o.label)).collect();
    let more = outcomes.iter().filter(|o| !o.pass).count().saturating_sub(4);
    let mut t = format!("tests {pass}/{} pass", outcomes.len());
    if !failing.is_empty() {
        t.push_str(&format!("; failing: {}", failing.join(", ")));
        if more > 0 {
            t.push_str(&format!(" +{more}"));
        }
    }
    t
}

/// The `expect check` report: one line per expectation.
pub fn report(doc: &Document) -> Report {
    let outcomes = check_all(doc);
    let mut t = String::new();
    if outcomes.is_empty() {
        t.push_str("no expectations (expect add creates them)\n");
    }
    for o in &outcomes {
        t.push_str(&format!("{} {} {}: {}\n", if o.pass { "pass" } else { "FAIL" }, o.name, o.label, o.message));
    }
    let line = tests_line(&outcomes);
    if !line.is_empty() {
        t.push_str(&line);
        t.push('\n');
    }
    Report::new("expect", t, &outcomes)
}
