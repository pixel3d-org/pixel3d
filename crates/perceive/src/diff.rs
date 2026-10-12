//! `diff`: what did my edit do? Objects added, removed, changed and moved,
//! size, volume and topology deltas, how the silhouette changed, and
//! findings added and resolved.

use std::collections::BTreeSet;

use glam::{Vec2, Vec3};
use serde::Serialize;

use crate::Report;
use crate::diagnose::{Finding, diagnose};
use crate::error::PerceiveError;
use crate::fmt::{dims, mm, point, thousands, volume};
use crate::silhouette::{ViewDir, rasterize};
use crate::view::View;

/// Changed objects described in the text before it is cut short.
const MAX_CHANGED: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObjectChange {
    pub id: u32,
    pub name: String,
    /// What changed, as short phrases.
    pub changes: Vec<String>,
}

/// How the front silhouette changed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SilhouetteDelta {
    pub view: ViewDir,
    /// Covered area before and after, square meters.
    pub area_before: f32,
    pub area_after: f32,
    /// Largest distance the outline moved (meters) and where (view axes).
    pub max_shift: f32,
    pub at: Vec2,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FindingRef {
    pub code: String,
    pub part: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diff {
    pub before: u64,
    pub after: u64,
    pub added: Vec<ObjectChange>,
    pub removed: Vec<ObjectChange>,
    pub changed: Vec<ObjectChange>,
    pub silhouette: Option<SilhouetteDelta>,
    pub findings_added: Vec<FindingRef>,
    pub findings_resolved: Vec<FindingRef>,
}

/// Chamfer distance (in cells) from every cell to the nearest filled cell
/// of a mask; `f32::INFINITY` where the mask is empty.
///
/// Complexity: O(cells).
fn distance_to(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    const A: f32 = 1.0;
    const B: f32 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> = mask.iter().map(|&m| if m { 0.0 } else { f32::INFINITY }).collect();
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + A);
            }
            if j > 0 {
                v = v.min(d[k - w] + A);
                if i > 0 {
                    v = v.min(d[k - w - 1] + B);
                }
                if i + 1 < w {
                    v = v.min(d[k - w + 1] + B);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..h).rev() {
        for i in (0..w).rev() {
            let k = j * w + i;
            let mut v = d[k];
            if i + 1 < w {
                v = v.min(d[k + 1] + A);
            }
            if j + 1 < h {
                v = v.min(d[k + w] + A);
                if i + 1 < w {
                    v = v.min(d[k + w + 1] + B);
                }
                if i > 0 {
                    v = v.min(d[k + w - 1] + B);
                }
            }
            d[k] = v;
        }
    }
    d
}

/// Front silhouettes of two views rasterized in a common frame, compared
/// cell by cell.
///
/// Complexity: O(triangles + cells).
pub fn silhouette_delta(before: &View, after: &View, dir: ViewDir) -> Option<SilhouetteDelta> {
    if before.parts.is_empty() && after.parts.is_empty() {
        return None;
    }
    let proj = |p: Vec3| crate::silhouette::project(dir, p);
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for v in [before, after] {
        if !v.parts.is_empty() {
            let (a, b) = (proj(v.lo), proj(v.hi));
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        }
    }
    let ra = rasterize(before, dir, 256, Some((lo, hi)));
    let rb = rasterize(after, dir, 256, Some((lo, hi)));
    let (w, h) = (ra.w, ra.h);
    let ma: Vec<bool> = (0..w * h).map(|k| ra.owner(k % w, k / w).is_some()).collect();
    let mb: Vec<bool> = (0..w * h).map(|k| rb.owner(k % w, k / w).is_some()).collect();
    let da = distance_to(&ma, w, h);
    let db = distance_to(&mb, w, h);
    let (mut shift, mut at_cell) = (0.0f32, None);
    for k in 0..w * h {
        // Added cells: how far from the old outline; removed: from the new.
        let d = match (ma[k], mb[k]) {
            (false, true) => da[k],
            (true, false) => db[k],
            _ => continue,
        };
        if d.is_finite() && d > shift {
            shift = d;
            at_cell = Some(k);
        }
    }
    let cell_area = ra.cell * ra.cell;
    let at = at_cell.map_or(Vec2::ZERO, |k| ra.origin + Vec2::new((k % w) as f32 + 0.5, (k / w) as f32 + 0.5) * ra.cell);
    Some(SilhouetteDelta {
        view: dir,
        area_before: ma.iter().filter(|&&m| m).count() as f32 * cell_area,
        area_after: mb.iter().filter(|&&m| m).count() as f32 * cell_area,
        max_shift: shift * ra.cell,
        at,
    })
}

/// Differences between two documents seen through the same target, with
/// the findings of both sides (callers cache them per revision; see
/// [`report`] for a version that computes them).
///
/// Complexity: O(objects + vertices) for both views plus two silhouette
/// rasters.
pub fn diff(before: &View, after: &View, findings_before: &[Finding], findings_after: &[Finding]) -> Diff {
    let ids = |v: &View| -> BTreeSet<u32> { v.parts.iter().map(|p| p.id).chain(v.others.iter().map(|o| o.id)).collect() };
    let (ia, ib) = (ids(before), ids(after));
    let label = |v: &View, id: u32| {
        v.parts
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.label.clone())
            .or_else(|| v.others.iter().find(|o| o.id == id).map(|o| o.label.clone()))
            .unwrap_or_default()
    };
    let mut d = Diff {
        before: before.doc.revision,
        after: after.doc.revision,
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        silhouette: None,
        findings_added: Vec::new(),
        findings_resolved: Vec::new(),
    };
    for &id in ib.difference(&ia) {
        let what =
            after.parts.iter().find(|p| p.id == id).map_or_else(|| "empty".to_owned(), |p| format!("{} at {}", dims(p.size()), point(p.center())));
        d.added.push(ObjectChange { id, name: label(after, id), changes: vec![what] });
    }
    for &id in ia.difference(&ib) {
        d.removed.push(ObjectChange { id, name: label(before, id), changes: Vec::new() });
    }
    for &id in ia.intersection(&ib) {
        let (Some(ka), Some(kb)) = (before.doc.object_by_id(id), after.doc.object_by_id(id)) else { continue };
        let (Some(oa), Some(ob)) = (before.doc.object(ka), after.doc.object(kb)) else { continue };
        let mut changes = Vec::new();
        let (pa, pb) = (before.parts.iter().find(|p| p.id == id), after.parts.iter().find(|p| p.id == id));
        if oa.name() != ob.name() {
            changes.push(format!("renamed from {}", oa.name()));
        }
        let mesh_same = match (oa.mesh_key(), ob.mesh_key()) {
            (Some(ma), Some(mb)) => matches!((before.doc.mesh_arc(ma), after.doc.mesh_arc(mb)), (Some(x), Some(y)) if std::sync::Arc::ptr_eq(x, y)),
            (None, None) => true,
            _ => false,
        };
        match (pa, pb) {
            (Some(a), Some(b)) => {
                let moved = b.center() - a.center();
                let resized = b.size() - a.size();
                if resized.abs().max_element() > 1e-5 {
                    changes.push(format!("size {} -> {}", dims(a.size()).trim_end_matches(" mm"), dims(b.size())));
                } else if moved.length() > 1e-5 {
                    changes.push(format!("moved by {} mm", point(moved)));
                }
                if !mesh_same {
                    let (ta, tb) = (a.topology(), b.topology());
                    if ta.tris != tb.tris {
                        changes.push(format!("tris {} -> {} ({:+})", thousands(ta.tris), thousands(tb.tris), tb.tris as i64 - ta.tris as i64));
                    }
                    if (ta.volume - tb.volume).abs() > 1e-7 && ta.is_closed() && tb.is_closed() {
                        changes.push(format!("volume {} -> {}", volume(ta.volume), volume(tb.volume)));
                    }
                    if ta.boundary_loops != tb.boundary_loops {
                        changes.push(format!("holes {} -> {}", ta.boundary_loops, tb.boundary_loops));
                    }
                    if ta.parts != tb.parts {
                        changes.push(format!("pieces {} -> {}", ta.parts, tb.parts));
                    }
                    let ga: BTreeSet<&str> = a.mesh.groups().map(|(n, _)| n).collect();
                    let gb: BTreeSet<&str> = b.mesh.groups().map(|(n, _)| n).collect();
                    let plus: Vec<&str> = gb.difference(&ga).copied().collect();
                    let minus: Vec<&str> = ga.difference(&gb).copied().collect();
                    if !plus.is_empty() {
                        changes.push(format!("groups +{}", plus.join(" +")));
                    }
                    if !minus.is_empty() {
                        changes.push(format!("groups -{}", minus.join(" -")));
                    }
                    if changes.is_empty() {
                        changes.push("mesh edited (same size and counts)".into());
                    }
                }
            }
            (None, Some(b)) => changes.push(format!("now has geometry: {}", dims(b.size()))),
            (Some(_), None) => changes.push("lost its geometry".into()),
            (None, None) => {
                let moved = ob.transform.translation - oa.transform.translation;
                if moved.length() > 1e-5 {
                    changes.push(format!("moved by {} mm", point(moved)));
                }
            }
        }
        if oa.parent().and_then(|p| before.doc.object(p)).map(|o| o.id()) != ob.parent().and_then(|p| after.doc.object(p)).map(|o| o.id()) {
            changes.push("parent changed".into());
        }
        if !changes.is_empty() {
            d.changed.push(ObjectChange { id, name: label(after, id), changes });
        }
    }
    d.silhouette = silhouette_delta(before, after, ViewDir::Front);
    let key = |f: &Finding| (f.code.to_owned(), f.part.clone());
    let fa: BTreeSet<(String, String)> = findings_before.iter().map(key).collect();
    let fb: BTreeSet<(String, String)> = findings_after.iter().map(key).collect();
    d.findings_added = fb.difference(&fa).map(|(code, part)| FindingRef { code: code.clone(), part: part.clone() }).collect();
    d.findings_resolved = fa.difference(&fb).map(|(code, part)| FindingRef { code: code.clone(), part: part.clone() }).collect();
    d
}

/// Compact text of a diff.
pub fn diff_text(title: &str, d: &Diff) -> String {
    let mut t = format!("diff rev {} -> rev {} of {title}\n", d.before, d.after);
    if d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty() {
        t.push_str("no changes\n");
    }
    for o in &d.added {
        t.push_str(&format!("added {} (o:{}): {}\n", o.name, o.id, o.changes.join("; ")));
    }
    for o in &d.removed {
        t.push_str(&format!("removed {} (o:{})\n", o.name, o.id));
    }
    for o in d.changed.iter().take(MAX_CHANGED) {
        t.push_str(&format!("changed {} (o:{}): {}\n", o.name, o.id, o.changes.join("; ")));
    }
    if d.changed.len() > MAX_CHANGED {
        t.push_str(&format!("+{} more changed objects\n", d.changed.len() - MAX_CHANGED));
    }
    if let Some(s) = &d.silhouette
        && s.max_shift > 0.0
    {
        let pct = if s.area_before > 0.0 { 100.0 * (s.area_after - s.area_before) / s.area_before } else { 100.0 };
        t.push_str(&format!(
            "silhouette front: area {:+.1}%, outline moved up to {} mm (near x={}, y={})\n",
            pct,
            mm(s.max_shift),
            mm(s.at.x),
            mm(s.at.y)
        ));
    }
    let refs = |v: &[FindingRef]| v.iter().map(|f| format!("{} {}", f.code, f.part)).collect::<Vec<_>>().join(", ");
    if !d.findings_added.is_empty() || !d.findings_resolved.is_empty() {
        let mut parts = Vec::new();
        if !d.findings_added.is_empty() {
            parts.push(format!("new {}", refs(&d.findings_added)));
        }
        if !d.findings_resolved.is_empty() {
            parts.push(format!("resolved {}", refs(&d.findings_resolved)));
        }
        t.push_str(&format!("findings: {}\n", parts.join("; ")));
    }
    t
}

/// The `diff` report between two documents, both seen through `target`.
///
/// Complexity: [`diff`] plus two [`diagnose`] runs.
pub fn report(before: &p3d_scene::Document, after: &p3d_scene::Document, target: Option<&str>) -> Result<Report, PerceiveError> {
    // A target that exists on one side only (added or deleted) is empty on the other.
    let (vb, va) = match (View::new(before, target), View::new(after, target)) {
        (Ok(b), Ok(a)) => (b, a),
        (Err(_), Ok(a)) => (View::empty(before), a),
        (Ok(b), Err(_)) => (b, View::empty(after)),
        (Err(e), Err(_)) => return Err(e),
    };
    let d = diff(&vb, &va, &diagnose(&vb), &diagnose(&va));
    Ok(Report::new("diff", diff_text(&va.title(), &d), &d))
}
