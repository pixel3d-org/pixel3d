//! `structure`: what is where. The part tree with sizes, positions, groups,
//! contacts and mirror pairs.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::Serialize;

use crate::Report;
use crate::fmt::{dims, mm, point, thousands};
use crate::relations::{RelationKind, Relations, relations};
use crate::view::View;

/// Parts listed in the text before it is cut short.
const MAX_PART_LINES: usize = 24;
/// Groups named per part in the text.
const MAX_GROUPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PartInfo {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub depth: usize,
    /// World-space size and box center, meters.
    pub size: Vec3,
    pub center: Vec3,
    pub tris: usize,
    pub groups: Vec<String>,
    pub tags: Vec<String>,
    /// Short relation phrases: `rests on crate`, `touches crate`, ...
    pub relations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OtherInfo {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub depth: usize,
    pub position: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Structure {
    pub title: String,
    pub size: Vec3,
    pub bottom: f32,
    pub parts: Vec<PartInfo>,
    pub others: Vec<OtherInfo>,
    /// Parts mirrored across the X plane through the view's center, by
    /// matching bounds: (left, right) names.
    pub mirror_pairs: Vec<(String, String)>,
    pub touching: usize,
    pub gaps: usize,
    pub penetrating: usize,
    pub floating: usize,
}

/// Parts whose bounds mirror each other across the YZ plane through the
/// view's center, within a tolerance. A hash grid over mirrored centers
/// keeps this linear in the part count.
///
/// Complexity: O(k log k) for k parts.
pub fn mirror_pairs(view: &View) -> Vec<(usize, usize)> {
    let tol = (view.diagonal() * 0.005).max(view.contact_tolerance());
    let cx = (view.lo.x + view.hi.x) / 2.0;
    let cell = |p: Vec3| ((p / tol).floor()).as_ivec3();
    let mut grid: BTreeMap<(i32, i32, i32), Vec<usize>> = BTreeMap::new();
    for (i, p) in view.parts.iter().enumerate() {
        let c = cell(p.center());
        grid.entry((c.x, c.y, c.z)).or_default().push(i);
    }
    let mut out = Vec::new();
    for (i, p) in view.parts.iter().enumerate() {
        let c = p.center();
        if c.x - cx <= tol {
            continue;
        }
        let m = Vec3::new(2.0 * cx - c.x, c.y, c.z);
        let mc = cell(m);
        let mut best: Option<(f32, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &j in grid.get(&(mc.x + dx, mc.y + dy, mc.z + dz)).into_iter().flatten() {
                        let q = &view.parts[j];
                        let d = q.center().distance(m);
                        if j != i && d <= tol && (q.size() - p.size()).abs().max_element() <= tol && best.is_none_or(|(e, k)| (d, j) < (e, k)) {
                            best = Some((d, j));
                        }
                    }
                }
            }
        }
        if let Some((_, j)) = best {
            out.push((j, i));
        }
    }
    out.sort_unstable();
    out
}

/// Short phrases for a part's relations, nearest first.
fn phrases(view: &View, rel: &Relations, i: usize) -> Vec<String> {
    let tol = rel.tolerance;
    let mut out = Vec::new();
    if rel.grounded.contains(&i) {
        out.push("on the ground".into());
    }
    for r in rel.of(i) {
        let other = if r.a == i { r.b } else { r.a };
        let o = &view.parts[other].label;
        out.push(match r.kind {
            RelationKind::Touching if view.parts[i].lo.y >= view.parts[other].hi.y - tol => format!("rests on {o}"),
            RelationKind::Touching if view.parts[other].lo.y >= view.parts[i].hi.y - tol => format!("under {o}"),
            RelationKind::Touching => format!("touches {o}"),
            RelationKind::Gap => format!("{} mm from {o}", mm(r.dist)),
            RelationKind::Penetrating if r.sinks == Some(i) => format!("sinks {} mm into {o}", mm(r.dist)),
            RelationKind::Penetrating => match r.sinks {
                Some(_) => format!("{o} sinks in {} mm", mm(r.dist)),
                None => format!("crosses {o}"),
            },
            RelationKind::Inside if r.a == i => format!("inside {o}"),
            RelationKind::Inside => format!("contains {o}"),
        });
    }
    if rel.floating.iter().any(|f| f.part == i) {
        out.push("floating".into());
    }
    out
}

/// The structure of a view.
///
/// Complexity: O(objects + vertices) plus [`relations`].
pub fn structure(view: &View) -> Structure {
    let rel = relations(view);
    let parts = view
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| PartInfo {
            id: p.id,
            name: p.label.clone(),
            path: p.path.clone(),
            depth: p.depth,
            size: p.size(),
            center: p.center(),
            tris: p.topology().tris,
            groups: p.mesh.groups().map(|(n, _)| n.to_owned()).collect(),
            tags: view.doc.object(p.key).map(|o| o.tags.iter().cloned().collect()).unwrap_or_default(),
            relations: phrases(view, &rel, i),
        })
        .collect();
    let others = view
        .others
        .iter()
        .map(|o| OtherInfo { id: o.id, name: o.label.clone(), kind: o.kind.into(), depth: o.depth, position: o.world.translation.into() })
        .collect();
    let count = |k: RelationKind| rel.pairs.iter().filter(|r| r.kind == k).count();
    Structure {
        title: view.title(),
        size: view.size(),
        bottom: view.lo.y,
        parts,
        others,
        mirror_pairs: mirror_pairs(view).into_iter().map(|(a, b)| (view.parts[a].label.clone(), view.parts[b].label.clone())).collect(),
        touching: count(RelationKind::Touching),
        gaps: count(RelationKind::Gap),
        penetrating: count(RelationKind::Penetrating),
        floating: rel.floating.len(),
    }
}

/// Compact text of a structure.
pub fn structure_text(s: &Structure) -> String {
    let mut t = format!(
        "structure of {}: {} part{}, world mm (+Y up, front +Z), positions are box centers\n",
        s.title,
        s.parts.len(),
        if s.parts.len() == 1 { "" } else { "s" }
    );
    if s.parts.is_empty() {
        t.push_str("no geometry\n");
    } else {
        t.push_str(&format!("size {}, bottom y={}\n", dims(s.size), mm(s.bottom)));
    }
    for p in s.parts.iter().take(MAX_PART_LINES) {
        let indent = "  ".repeat(p.depth.min(6));
        let mut line =
            format!("{indent}{} (o:{}) {} at {}; {} tris", p.name, p.id, dims(p.size).trim_end_matches(" mm"), point(p.center), thousands(p.tris));
        if !p.groups.is_empty() {
            let shown: Vec<&str> = p.groups.iter().take(MAX_GROUPS).map(String::as_str).collect();
            let more = if p.groups.len() > MAX_GROUPS { format!(" +{}", p.groups.len() - MAX_GROUPS) } else { String::new() };
            line.push_str(&format!("; groups {}{more}", shown.join(" ")));
        }
        if !p.tags.is_empty() {
            line.push_str(&format!("; tags {}", p.tags.join(" ")));
        }
        if !p.relations.is_empty() {
            line.push_str(&format!("; {}", p.relations.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
        }
        t.push_str(&line);
        t.push('\n');
    }
    if s.parts.len() > MAX_PART_LINES {
        t.push_str(&format!("+{} more parts (perceive structure with a target narrows the view)\n", s.parts.len() - MAX_PART_LINES));
    }
    for o in s.others.iter().take(8) {
        t.push_str(&format!("{}{} (o:{}) {} at {}\n", "  ".repeat(o.depth.min(6)), o.name, o.id, o.kind, point(o.position)));
    }
    if !s.mirror_pairs.is_empty() {
        let pairs: Vec<String> = s.mirror_pairs.iter().take(8).map(|(a, b)| format!("{a} | {b}")).collect();
        t.push_str(&format!("mirror pairs (x, by bounds): {}\n", pairs.join(", ")));
    }
    if s.parts.len() > 1 {
        t.push_str(&format!(
            "contacts: {} touching, {} gaps, {} penetrating, {} floating (perceive relations for details)\n",
            s.touching, s.gaps, s.penetrating, s.floating
        ));
    }
    t
}

/// The `structure` report of a view.
pub fn report(view: &View) -> Report {
    let s = structure(view);
    Report::new("structure", structure_text(&s), &s)
}
