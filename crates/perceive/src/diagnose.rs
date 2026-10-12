//! `diagnose`: problems reported like compiler errors (project spec 17.10).
//! Each finding has a stable code, a severity, the part and a location, the
//! measured value and its threshold, a likely cause, and fixes as ready-to-run
//! op calls. The engine fills in each fix's predicted result by running it
//! on a copy.

use glam::Vec3;
use rustc_hash::FxHashMap;
use serde::Serialize;
use serde_json::{Value, json};
use smallvec::SmallVec;

use crate::Report;
use crate::fmt::{mm, point, thousands, volume};
use crate::relations::{RelationKind, Relations, relations};
use crate::view::{Part, View};

/// Findings written out in the text before it is cut short.
const MAX_TEXT_FINDINGS: usize = 12;
/// Crossing triangle pairs counted per part before stopping.
const MAX_CROSSINGS: u32 = 1000;
/// Samples per part for the thin-wall check.
const THIN_SAMPLES: usize = 128;
/// Walls thinner than this are flagged (1 mm).
pub const THIN_WALL: f32 = 1e-3;
/// Faces closer than this to a same-facing face of another part z-fight.
const Z_FIGHT_GAP: f32 = 5e-5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Info => "info",
        }
    }
}

/// A ready-to-run fix.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fix {
    /// What the fix does, in a few words.
    pub summary: String,
    pub op: String,
    pub args: Value,
    /// Predicted result of applying it (filled by the engine's dry run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicted: Option<String>,
}

/// One problem.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// `F1`, `F2`, ... in report order (stable for one revision).
    pub id: String,
    pub code: &'static str,
    pub severity: Severity,
    pub part: String,
    /// The other part, for relation findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// World position and the part's side it is on (`top`, `left`, ...).
    pub at: Vec3,
    pub side: &'static str,
    /// What was measured and the limit, in the units of the message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f32>,
    pub message: String,
    /// Likely cause: the last op that changed the part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    pub fixes: Vec<Fix>,
    /// Why there is no fix yet, when `fixes` is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The side of a part a world point lies on, by its largest relative offset
/// from the part's box center.
pub fn side_of(part: &Part, p: Vec3) -> &'static str {
    let half = part.size() / 2.0;
    // Flat directions (a sheet's thickness) carry no side.
    let rel = |k: usize| if half[k] > 1e-6 { (p[k] - part.center()[k]) / half[k] } else { 0.0 };
    let d = Vec3::new(rel(0), rel(1), rel(2));
    if d.abs().max_element() < 0.25 {
        return "center";
    }
    match d.abs().max_position() {
        0 if d.x < 0.0 => "left",
        0 => "right",
        1 if d.y < 0.0 => "bottom",
        1 => "top",
        _ if d.z < 0.0 => "back",
        _ => "front",
    }
}

fn last_change(view: &View, part: &Part) -> Option<String> {
    let node = view.doc.history(part.key)?.nodes.last()?;
    Some(format!("last changed by {} ({})", node.op, node.label))
}

/// Meters rounded to 0.1 mm for fix arguments.
fn round4(v: Vec3) -> [f64; 3] {
    v.to_array().map(|x| (f64::from(x) * 1e4).round() / 1e4)
}

/// A move of a part by a world-space offset, as a `scene.set_transform`
/// call in its parent's space.
fn move_fix(view: &View, part: &Part, delta: Vec3, summary: String) -> Fix {
    let parent = view.doc.object(part.key).and_then(|o| o.parent());
    let local = parent.and_then(|p| view.doc.world_affine(p)).map_or(delta, |m| m.inverse().transform_vector3(delta));
    Fix {
        summary,
        op: "scene.set_transform".into(),
        args: json!({ "target": part.path, "position": round4(local), "relative": true }),
        predicted: None,
    }
}

struct Draft {
    code: &'static str,
    severity: Severity,
    part: usize,
    other: Option<usize>,
    at: Vec3,
    measured: Option<f32>,
    threshold: Option<f32>,
    message: String,
    fixes: Vec<Fix>,
    hint: Option<String>,
}

/// Geometry findings of one part.
///
/// Complexity: O(n log n) for its n triangles (topology, duplicate hashing,
/// self-crossings), plus bounded sampled ray casts.
fn geometry(view: &View, i: usize, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    let m = part.mesh;
    let topo = part.topology();
    let flat = part.size().min_element() < 1e-3_f32.max(0.01 * part.size().max_element());
    let draft = |code, severity, at, message: String| Draft {
        code,
        severity,
        part: i,
        other: None,
        at,
        measured: None,
        threshold: None,
        message,
        fixes: Vec::new(),
        hint: None,
    };

    // Inward-facing closed pieces.
    let parts = p3d_analyze::parts(m);
    let inverted: Vec<&p3d_analyze::Part> = parts.parts.iter().filter(|p| p.closed && p.volume < 0.0 && !p.cavity).collect();
    // Inner walls of hollow parts are part of the solid, not separate pieces.
    let pieces = parts.parts.iter().filter(|p| !p.cavity).count();
    if let Some(first) = inverted.first() {
        let at = part.world.transform_point3((first.min + first.max) / 2.0);
        let vol: f64 = inverted.iter().map(|p| p.volume).sum();
        let mut d = draft(
            "INVERTED_NORMALS",
            Severity::Error,
            at,
            format!(
                "{} closed piece{} face{} inward (volume {}): it renders inside out and bakes wrong",
                inverted.len(),
                if inverted.len() == 1 { "" } else { "s" },
                if inverted.len() == 1 { "s" } else { "" },
                volume(vol)
            ),
        );
        d.measured = Some(inverted.len() as f32);
        d.fixes.push(Fix {
            summary: "flip the inward pieces".into(),
            op: "edit.recalc_normals".into(),
            args: json!({ "target": part.path }),
            predicted: None,
        });
        out.push(d);
    }

    // Holes in surfaces that are not flat sheets.
    if topo.boundary_loops > 0 && !flat {
        let loops = p3d_analyze::boundary_loops(m, &parts);
        let biggest = loops.iter().max_by(|a, b| a.length.total_cmp(&b.length).then(b.start.cmp(&a.start)));
        if let Some(b) = biggest {
            let at = part.world.transform_point3(b.centroid);
            let mut d = draft(
                "OPEN_BOUNDARY",
                Severity::Warn,
                at,
                format!(
                    "{} hole{} ({} open edges); largest: {} edges, {} mm around",
                    loops.len(),
                    if loops.len() == 1 { "" } else { "s" },
                    thousands(topo.boundary_edges),
                    b.edges,
                    mm(b.length)
                ),
            );
            d.measured = Some(loops.len() as f32);
            d.threshold = Some(0.0);
            d.fixes.push(Fix {
                summary: "close every hole with a face".into(),
                op: "edit.fill".into(),
                args: json!({ "target": part.path }),
                predicted: None,
            });
            d.hint = Some("ignore if the part is meant to be open (cloth, cards, a container)".into());
            out.push(d);
        }
    }

    // Collapsed faces and slivers.
    if topo.degenerate_faces > 0 || topo.short_edges > 0 {
        let first = m.faces().find(|&f| p3d_analyze::face_degenerate(m, f).0);
        let at = first.map_or(part.center(), |f| part.world.transform_point3(m.face_centroid(f)));
        let mut d = draft("DEGENERATE_FACES", Severity::Warn, at, {
            let plural = |n: usize| if n == 1 { "" } else { "s" };
            let mut what = Vec::new();
            if topo.degenerate_faces > 0 {
                what.push(format!("{} collapsed or sliver face{}", topo.degenerate_faces, plural(topo.degenerate_faces)));
            }
            if topo.short_edges > 0 {
                what.push(format!("{} edge{} under 1 micrometer", topo.short_edges, plural(topo.short_edges)));
            }
            format!("{}: they break normals, UVs and bakes", what.join(" and "))
        });
        d.measured = Some(topo.degenerate_faces as f32);
        d.hint = Some("no automatic fix yet; merging their vertices (cleanup.auto) will remove them".into());
        out.push(d);
    }

    // Faces lying exactly on top of other faces (same corners).
    let mut seen: FxHashMap<SmallVec<[[i64; 3]; 4]>, u32> = FxHashMap::default();
    let (mut dups, mut first_dup) = (0usize, None);
    let q = |p: Vec3| (p * 1e6).round().as_i64vec3().to_array();
    for f in m.faces() {
        let mut key: SmallVec<[[i64; 3]; 4]> = m.face_verts(f).map(|v| q(m.positions[v as usize])).collect();
        key.sort_unstable();
        if let std::collections::hash_map::Entry::Vacant(slot) = seen.entry(key) {
            slot.insert(f);
        } else {
            dups += 1;
            first_dup.get_or_insert(f);
        }
    }
    if let Some(f) = first_dup {
        let mut d = draft(
            "DUPLICATE_FACES",
            Severity::Warn,
            part.world.transform_point3(m.face_centroid(f)),
            format!("{dups} face{} duplicate other faces exactly: they flicker and double the cost", if dups == 1 { "" } else { "s" }),
        );
        d.measured = Some(dups as f32);
        d.hint = Some("no automatic fix yet; remove the extra copy (cleanup.auto will)".into());
        out.push(d);
    }

    // The surface passing through itself.
    // Located at the mean of the crossing triangles: where the overlap is.
    let mut crossings = 0u32;
    let mut sum = Vec3::ZERO;
    part.bvh().self_crossings(|t, u| {
        crossings += 1;
        let c = |x: u32| part.bvh().triangle(x).iter().copied().sum::<Vec3>() / 3.0;
        sum += (c(t) + c(u)) / 2.0;
        crossings < MAX_CROSSINGS
    });
    if crossings > 0 {
        let at = sum / crossings as f32;
        let mut d = draft(
            "SELF_INTERSECTION",
            Severity::Warn,
            at,
            format!(
                "{}{} triangle pairs pass through each other{}",
                if crossings >= MAX_CROSSINGS { "over " } else { "" },
                thousands(crossings as usize),
                if pieces > 1 { format!(" ({pieces} loose pieces overlap)") } else { String::new() }
            ),
        );
        d.measured = Some(crossings as f32);
        if pieces > 1 {
            d.fixes.push(Fix {
                summary: "make each loose piece its own part".into(),
                op: "scene.separate".into(),
                args: json!({ "target": part.path, "by": "loose_parts" }),
                predicted: None,
            });
        } else {
            d.hint = Some("no automatic fix yet; reshape the folded area".into());
        }
        out.push(d);
    } else if pieces > 1 {
        // Several separate pieces in one part: fine, but each could be a part.
        let mut d = draft("LOOSE_PIECES", Severity::Info, part.center(), format!("{pieces} separate pieces in one part"));
        d.measured = Some(pieces as f32);
        d.fixes.push(Fix {
            summary: "make each piece its own part".into(),
            op: "scene.separate".into(),
            args: json!({ "target": part.path, "by": "loose_parts" }),
            predicted: None,
        });
        out.push(d);
    }

    // Walls thinner than 1 mm in closed parts.
    if topo.is_closed() {
        let eps = part.size().length() * 1e-5;
        let mut thinnest: Option<(f32, Vec3)> = None;
        for s in crate::sample::surface_samples(part, THIN_SAMPLES) {
            if s.normal == Vec3::ZERO {
                continue;
            }
            if let Some(h) = part.bvh().raycast(s.point - s.normal * eps, -s.normal, THIN_WALL)
                && thinnest.is_none_or(|(t, _)| h.t + eps < t)
            {
                thinnest = Some((h.t + eps, s.point));
            }
        }
        if let Some((t, at)) = thinnest {
            let mut d = draft(
                "THIN_WALL",
                Severity::Warn,
                at,
                format!("wall only {} mm thick (limit {} mm): it z-fights and bakes badly", mm(t), mm(THIN_WALL)),
            );
            d.measured = Some(t);
            d.threshold = Some(THIN_WALL);
            d.hint = Some("no automatic fix yet; thicken the wall or remove the inner surface".into());
            out.push(d);
        }
    }
}

/// Relation findings between parts.
///
/// Complexity: [`relations`] plus O(s log n) per touching pair for the
/// z-fighting samples.
fn relation_findings(view: &View, rel: &Relations, out: &mut Vec<Draft>) {
    let tol = rel.tolerance;
    for f in &rel.floating {
        let part = &view.parts[f.part];
        let (message, fixes, at, other, gap) = match &f.nearest {
            Some(n) => (
                format!("touches nothing: {} mm from {}", mm(n.gap), view.parts[n.part].label),
                vec![move_fix(view, part, n.to - n.from, format!("move it {} mm onto {}", mm(n.gap), view.parts[n.part].label))],
                n.from,
                Some(n.part),
                Some(n.gap),
            ),
            None => (format!("touches nothing within {} mm", mm(0.25 * view.diagonal())), Vec::new(), part.center(), None, None),
        };
        out.push(Draft {
            code: "FLOATING_PART",
            severity: Severity::Warn,
            part: f.part,
            other,
            at,
            measured: gap,
            threshold: Some(tol),
            message,
            fixes,
            hint: None,
        });
    }
    for r in &rel.pairs {
        match r.kind {
            RelationKind::Penetrating => {
                let Some(s) = r.sinks else { continue };
                let other = if s == r.a { r.b } else { r.a };
                let sinker = &view.parts[s];
                // Embedding up to a quarter of the sinking part's thinnest size
                // reads as attached; deeper looks like a modelling slip.
                let allowed = (0.25 * sinker.size().min_element()).max(tol);
                if r.dist <= allowed {
                    continue;
                }
                let dir = (r.to - r.at).normalize_or_zero();
                let by = r.dist - allowed / 2.0;
                out.push(Draft {
                    code: "PENETRATION",
                    severity: Severity::Warn,
                    part: s,
                    other: Some(other),
                    at: r.at,
                    measured: Some(r.dist),
                    threshold: Some(allowed),
                    message: format!("sinks {} mm into {} (allowed {} mm)", mm(r.dist), view.parts[other].label, mm(allowed)),
                    fixes: vec![move_fix(view, sinker, dir * by, format!("pull it out {} mm", mm(by)))],
                    hint: None,
                });
            }
            RelationKind::Touching => {
                if let Some(d) = z_fighting(view, r.a, r.b) {
                    out.push(d);
                }
            }
            _ => {}
        }
    }
}

/// Faces of one part lying on same-facing faces of another (both render at
/// the same depth and flicker).
///
/// Complexity: O(s log n) for s <= 256 samples of the smaller part.
fn z_fighting(view: &View, a: usize, b: usize) -> Option<Draft> {
    let (small, big) = if view.parts[a].bvh().len() <= view.parts[b].bvh().len() { (a, b) } else { (b, a) };
    let (ps, pb) = (&view.parts[small], &view.parts[big]);
    let samples = crate::sample::surface_samples(ps, 256);
    let (mut area, mut total, mut sum) = (0.0f32, 0.0f32, Vec3::ZERO);
    for s in &samples {
        total += s.weight;
        if let Some(c) = pb.bvh().closest(s.point, Z_FIGHT_GAP)
            && pb.bvh().normal(c.tri).dot(s.normal) > 0.999
        {
            area += s.weight;
            sum += s.point * s.weight;
        }
    }
    if area <= 0.0 {
        return None;
    }
    // The middle of the flickering area.
    let at = sum / area;
    let shift = ps.bvh().normal(ps.bvh().closest(at, 1e-3).map_or(0, |c| c.tri));
    Some(Draft {
        code: "Z_FIGHTING",
        severity: Severity::Warn,
        part: small,
        other: Some(big),
        at,
        measured: Some(area / total.max(1e-12)),
        threshold: Some(0.0),
        message: format!("{:.0}% of its surface lies on same-facing faces of {}: they flicker", 100.0 * area / total.max(1e-12), pb.label),
        fixes: vec![move_fix(view, ps, shift * 5e-4, "move it 0.5 mm off the shared plane".into())],
        hint: None,
    })
}

/// Every finding in a view, ranked by severity, then part order, then code,
/// and numbered `F1`, `F2`, ...
///
/// Complexity: the sum of the geometry checks over the parts plus
/// [`relations`].
pub fn diagnose(view: &View) -> Vec<Finding> {
    let mut drafts = Vec::new();
    for i in 0..view.parts.len() {
        geometry(view, i, &mut drafts);
    }
    let rel = relations(view);
    relation_findings(view, &rel, &mut drafts);
    drafts.sort_by(|x, y| (x.severity, x.part, x.code, x.other).cmp(&(y.severity, y.part, y.code, y.other)));
    drafts
        .into_iter()
        .enumerate()
        .map(|(k, d)| {
            let part = &view.parts[d.part];
            Finding {
                id: format!("F{}", k + 1),
                code: d.code,
                severity: d.severity,
                part: part.label.clone(),
                other: d.other.map(|o| view.parts[o].label.clone()),
                at: d.at,
                side: side_of(part, d.at),
                measured: d.measured,
                threshold: d.threshold,
                message: d.message,
                cause: last_change(view, part),
                fixes: d.fixes,
                hint: d.hint,
            }
        })
        .collect()
}

/// One block per finding, like a compiler's output.
pub fn findings_text(title: &str, findings: &[Finding]) -> String {
    let count = |s: Severity| findings.iter().filter(|f| f.severity == s).count();
    let (e, w, i) = (count(Severity::Error), count(Severity::Warn), count(Severity::Info));
    let mut t = format!("diagnose {title}: {e} error{}, {w} warning{}, {i} info\n", if e == 1 { "" } else { "s" }, if w == 1 { "" } else { "s" });
    if findings.is_empty() {
        t.push_str("no problems found\n");
    }
    for f in findings.iter().take(MAX_TEXT_FINDINGS) {
        let other = f.other.as_ref().map_or(String::new(), |o| format!(" -> {o}"));
        t.push_str(&format!("{} {} {} {}{other}: {}\n", f.id, f.severity.as_str(), f.code, f.part, f.message));
        t.push_str(&format!("    at {} ({} of {})", point(f.at), f.side, f.part));
        if let Some(c) = &f.cause {
            t.push_str(&format!("; {c}"));
        }
        t.push('\n');
        for fix in &f.fixes {
            let args = serde_json::to_string(&fix.args).unwrap_or_default();
            t.push_str(&format!("    fix: {} {args} ({})", fix.op, fix.summary));
            if let Some(p) = &fix.predicted {
                t.push_str(&format!("  predicted: {p}"));
            }
            t.push('\n');
        }
        if let Some(h) = &f.hint {
            t.push_str(&format!("    note: {h}\n"));
        }
    }
    if findings.len() > MAX_TEXT_FINDINGS {
        t.push_str(&format!("+{} more findings (diagnose with a target narrows the view)\n", findings.len() - MAX_TEXT_FINDINGS));
    }
    t
}

/// The `diagnose` report of a view (fixes without predictions).
pub fn report(view: &View) -> Report {
    let f = diagnose(view);
    Report::new("diagnose", findings_text(&view.title(), &f), &f)
}
