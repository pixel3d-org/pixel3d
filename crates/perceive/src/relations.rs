//! `relations`: do the parts fit together? Contacts, gaps, penetrations,
//! parts inside others, and floating parts.

use glam::Vec3;
use p3d_geo::bvh::box_pairs;
use serde::Serialize;

use crate::fmt::{mm, point};
use crate::view::View;

/// Candidate pairs examined at most; beyond it the report says so. Only
/// scenes with hundreds of parts packed within a quarter of their size of
/// each other come close.
pub const MAX_PAIRS: usize = 20_000;

/// Vertices sampled per part when estimating penetration depth.
const DEPTH_SAMPLES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Closer than the contact tolerance, not crossing.
    Touching,
    /// Apart by more than the contact tolerance, but near.
    Gap,
    /// Surfaces cross.
    Penetrating,
    /// `a` lies entirely inside `b` (no crossing).
    Inside,
}

/// How two parts relate.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairRelation {
    /// Part indices in the view, `a < b` except for `inside` (inner first).
    pub a: usize,
    pub b: usize,
    pub kind: RelationKind,
    /// Gap for `touching` and `gap`; penetration depth for `penetrating`
    /// (0 when unknown because neither part is closed); 0 for `inside`.
    pub dist: f32,
    /// For `penetrating`: the part whose surface sinks deepest into the other.
    pub sinks: Option<usize>,
    /// Where: the closest point on `a`, or the deepest point.
    pub at: Vec3,
    /// The search ran out of budget: `dist` is an upper bound.
    pub approximate: bool,
}

/// A part that touches nothing and does not stand on the ground.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Floating {
    pub part: usize,
    /// The nearest part within the search radius and the gap to it.
    pub nearest: Option<(usize, f32)>,
}

/// The contact graph of a view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Relations {
    pub tolerance: f32,
    /// Pairs within the near distance, ordered by (a, b).
    pub pairs: Vec<PairRelation>,
    pub floating: Vec<Floating>,
    /// Parts whose lowest point is on the ground plane (y = 0).
    pub grounded: Vec<usize>,
    /// More candidate pairs than [`MAX_PAIRS`]; the rest were skipped.
    pub truncated: bool,
}

impl Relations {
    /// Relations involving part `i`.
    pub fn of(&self, i: usize) -> impl Iterator<Item = &PairRelation> {
        self.pairs.iter().filter(move |r| r.a == i || r.b == i)
    }

    pub fn in_contact(&self, i: usize) -> bool {
        self.of(i).any(|r| r.kind != RelationKind::Gap)
    }
}

/// Deepest point of part `a`'s vertices inside closed part `b`: (depth, point).
///
/// Complexity: O(s (log n + k)) for s <= [`DEPTH_SAMPLES`] sampled vertices.
fn deepest_inside(view: &View, a: usize, b: usize) -> Option<(f32, Vec3)> {
    let (pa, pb) = (&view.parts[a], &view.parts[b]);
    if !pb.topology().is_watertight() {
        return None;
    }
    let n = pa.mesh.vert_slots();
    let stride = n.div_ceil(DEPTH_SAMPLES).max(1);
    let mut best: Option<(f32, Vec3)> = None;
    for v in (0..n as u32).step_by(stride).filter(|&v| pa.mesh.is_vert_alive(v)) {
        let p = pa.world.transform_point3(pa.mesh.positions[v as usize]);
        if !(p.cmpge(pb.lo).all() && p.cmple(pb.hi).all()) || !pb.bvh().contains(p) {
            continue;
        }
        if let Some(c) = pb.bvh().closest(p, f32::INFINITY)
            && best.is_none_or(|(d, _)| c.dist > d)
        {
            best = Some((c.dist, p));
        }
    }
    best
}

/// The contact graph: every pair of parts within 5% of the view's size is
/// classified; parts touching nothing are reported with their nearest
/// neighbour within a quarter of the view's size.
///
/// Complexity: O(k log k + pairs x (log n + leaf pairs near the contact))
/// for k parts, plus building each involved part's BVH once.
pub fn relations(view: &View) -> Relations {
    let tol = view.contact_tolerance();
    let near = 0.05 * view.diagonal() + tol;
    let search = 0.25 * view.diagonal() + tol;
    let boxes: Vec<(Vec3, Vec3)> = view.parts.iter().map(|p| (p.lo, p.hi)).collect();
    let (candidates, truncated) = box_pairs(&boxes, search / 2.0, MAX_PAIRS);

    let mut pairs = Vec::new();
    let mut nearest: Vec<Option<(usize, f32)>> = vec![None; view.parts.len()];
    let note = |i: usize, j: usize, d: f32, nearest: &mut Vec<Option<(usize, f32)>>| {
        for (x, y) in [(i, j), (j, i)] {
            if nearest[x].is_none_or(|(_, e)| d < e) {
                nearest[x] = Some((y, d));
            }
        }
    };
    for (i, j) in candidates {
        let (pi, pj) = (&view.parts[i], &view.parts[j]);
        let Some(n) = pi.bvh().nearest(pj.bvh(), search) else { continue };
        note(i, j, n.dist, &mut nearest);
        // Distance 0 is also plain contact (faces lying on each other): only
        // triangles passing through each other make a penetration.
        let mut crossing = None;
        if n.dist <= tol {
            pi.bvh().crossing_pairs(pj.bvh(), |t, _| {
                crossing = Some(pi.bvh().triangle(t)[0]);
                false
            });
        }
        if let Some(cross_at) = crossing {
            let di = deepest_inside(view, i, j);
            let dj = deepest_inside(view, j, i);
            let (depth, at, sinks) = match (di, dj) {
                (Some((x, p)), Some((y, q))) => {
                    if x >= y {
                        (x, p, Some(i))
                    } else {
                        (y, q, Some(j))
                    }
                }
                (Some((x, p)), None) => (x, p, Some(i)),
                (None, Some((y, q))) => (y, q, Some(j)),
                (None, None) => (0.0, cross_at, None),
            };
            // Crossing by less than the tolerance is contact.
            if sinks.is_none() || depth > tol {
                pairs.push(PairRelation { a: i, b: j, kind: RelationKind::Penetrating, dist: depth, sinks, at, approximate: n.approximate });
                continue;
            }
        }
        // Not crossing: one part may still lie wholly inside the other.
        let inside = |x: usize, y: usize| {
            let (px, py) = (&view.parts[x], &view.parts[y]);
            px.lo.cmpge(py.lo).all() && px.hi.cmple(py.hi).all() && py.topology().is_watertight() && py.bvh().contains(n_point(px))
        };
        if inside(i, j) || inside(j, i) {
            let (a, b) = if inside(i, j) { (i, j) } else { (j, i) };
            pairs.push(PairRelation { a, b, kind: RelationKind::Inside, dist: 0.0, sinks: None, at: view.parts[a].center(), approximate: false });
        } else if n.dist <= tol {
            pairs.push(PairRelation { a: i, b: j, kind: RelationKind::Touching, dist: n.dist, sinks: None, at: n.a, approximate: n.approximate });
        } else if n.dist <= near {
            pairs.push(PairRelation { a: i, b: j, kind: RelationKind::Gap, dist: n.dist, sinks: None, at: n.a, approximate: n.approximate });
        }
    }
    pairs.sort_by_key(|r| (r.a.min(r.b), r.a.max(r.b)));

    let grounded: Vec<usize> = (0..view.parts.len()).filter(|&i| view.parts[i].lo.y.abs() <= tol).collect();
    let mut rel = Relations { tolerance: tol, pairs, floating: Vec::new(), grounded, truncated };
    if view.parts.len() > 1 {
        for (i, near) in nearest.iter().enumerate() {
            if !rel.in_contact(i) && rel.grounded.binary_search(&i).is_err() {
                rel.floating.push(Floating { part: i, nearest: *near });
            }
        }
    }
    rel
}

/// A vertex of a part in world space (for containment tests).
fn n_point(p: &crate::view::Part) -> Vec3 {
    p.mesh.verts().next().map_or(p.center(), |v| p.world.transform_point3(p.mesh.positions[v as usize]))
}

/// One line per relation, at most `max_lines` lines, then floating parts
/// and grounded parts.
pub fn relation_lines(view: &View, rel: &Relations, max_lines: usize) -> Vec<String> {
    let name = |i: usize| view.parts[i].label.clone();
    let mut out = Vec::new();
    for r in rel.pairs.iter().take(max_lines) {
        let approx = if r.approximate { " (at most)" } else { "" };
        out.push(match r.kind {
            RelationKind::Touching => format!("{} - {}: touching at {}", name(r.a), name(r.b), point(r.at)),
            RelationKind::Gap => format!("{} - {}: gap {} mm{approx} at {}", name(r.a), name(r.b), mm(r.dist), point(r.at)),
            RelationKind::Penetrating => match r.sinks {
                Some(s) => {
                    let other = if s == r.a { r.b } else { r.a };
                    format!("{} - {}: {} sinks {} mm into {} at {}", name(r.a), name(r.b), name(s), mm(r.dist), name(other), point(r.at))
                }
                None => format!("{} - {}: surfaces cross at {} (depth unknown: open meshes)", name(r.a), name(r.b), point(r.at)),
            },
            RelationKind::Inside => format!("{} is inside {}", name(r.a), name(r.b)),
        });
    }
    if rel.pairs.len() > max_lines {
        out.push(format!("+{} more relations (perceive relations with a target narrows the view)", rel.pairs.len() - max_lines));
    }
    for f in &rel.floating {
        out.push(match f.nearest {
            Some((j, d)) => format!("floating: {} touches nothing; nearest {} at {} mm", name(f.part), name(j), mm(d)),
            None => format!("floating: {} touches nothing within {} mm", name(f.part), mm(0.25 * view.diagonal())),
        });
    }
    if !rel.grounded.is_empty() {
        let g: Vec<String> = rel.grounded.iter().take(8).map(|&i| name(i)).collect();
        out.push(format!("on the ground (y = 0): {}{}", g.join(", "), if rel.grounded.len() > 8 { ", ..." } else { "" }));
    }
    if rel.truncated {
        out.push(format!("note: over {} nearby pairs; some were skipped", crate::fmt::thousands(MAX_PAIRS)));
    }
    out
}

/// Lines of relations listed before the text is cut short.
const MAX_RELATION_LINES: usize = 20;

/// The `relations` report of a view.
pub fn report(view: &View) -> crate::Report {
    let rel = relations(view);
    let mut text = format!(
        "relations of {}: {} part{}, contact within {} mm\n",
        view.title(),
        view.parts.len(),
        if view.parts.len() == 1 { "" } else { "s" },
        mm(rel.tolerance)
    );
    let lines = relation_lines(view, &rel, MAX_RELATION_LINES);
    if lines.is_empty() {
        text.push_str(if view.parts.len() < 2 { "a single part: nothing to relate\n" } else { "no parts near each other\n" });
    }
    for l in lines {
        text.push_str(&l);
        text.push('\n');
    }
    crate::Report::new("relations", text, &named(view, &rel))
}

/// Relations with part names instead of view indices, for JSON output.
pub fn named(view: &View, rel: &Relations) -> serde_json::Value {
    let name = |i: usize| view.parts[i].label.clone();
    serde_json::json!({
        "tolerance": rel.tolerance,
        "pairs": rel.pairs.iter().map(|r| serde_json::json!({
            "a": name(r.a),
            "b": name(r.b),
            "kind": r.kind,
            "dist": r.dist,
            "sinks": r.sinks.map(name),
            "at": r.at,
            "approximate": r.approximate,
        })).collect::<Vec<_>>(),
        "floating": rel.floating.iter().map(|f| serde_json::json!({
            "part": name(f.part),
            "nearest": f.nearest.map(|(j, _)| name(j)),
            "gap": f.nearest.map(|(_, d)| d),
        })).collect::<Vec<_>>(),
        "grounded": rel.grounded.iter().map(|&i| name(i)).collect::<Vec<_>>(),
        "truncated": rel.truncated,
    })
}
