//! `section`: how thick, how round, what is inside? Closed loops of a
//! planar slice through the parts, per part, in world millimeters.

use glam::{Vec2, Vec3};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::Report;
use crate::error::PerceiveError;
use crate::fmt::{area, mm, mm0};
use crate::silhouette::simplify;
use crate::view::{Part, View};

/// Default number of points over all loops.
pub const DEFAULT_POINTS: usize = 48;

/// Slice plane normal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SliceAxis {
    /// A vertical slice across X; loops in (z, y).
    X,
    /// A horizontal slice; loops in (x, z).
    #[default]
    Y,
    /// A vertical slice across Z; loops in (x, y).
    Z,
}

impl SliceAxis {
    fn index(self) -> usize {
        match self {
            SliceAxis::X => 0,
            SliceAxis::Y => 1,
            SliceAxis::Z => 2,
        }
    }

    /// Output axes (u, v) and the orientation sign that makes outer loops
    /// counter-clockwise in them.
    fn plane(self) -> (usize, usize, f32) {
        match self {
            SliceAxis::X => (2, 1, 1.0),
            SliceAxis::Y => (0, 2, 1.0),
            SliceAxis::Z => (0, 1, -1.0),
        }
    }

    pub fn name(self) -> &'static str {
        ["x", "y", "z"][self.index()]
    }
}

/// Options of the `section` aspect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionOptions {
    pub axis: SliceAxis,
    /// Plane position in meters; the middle of the view when `None`.
    pub at: Option<f32>,
    /// Points over all loops (8 to 160).
    pub points: usize,
}

impl Default for SectionOptions {
    fn default() -> Self {
        SectionOptions { axis: SliceAxis::Y, at: None, points: DEFAULT_POINTS }
    }
}

/// One loop of the slice.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SectionLoop {
    pub part: String,
    /// `outer`, `hole`, or `open` (the slice crosses an open surface).
    pub kind: &'static str,
    /// Enclosed area in square meters (0 for open curves).
    pub area: f32,
    pub length: f32,
    pub min: Vec2,
    pub max: Vec2,
    /// Simplified points (world meters in the section's axes).
    pub points: Vec<Vec2>,
    /// For holes: the smallest distance to the enclosing outer loop of the
    /// same part (the wall thickness).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wall: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Section {
    pub axis: SliceAxis,
    pub at: f32,
    /// World axes of the loop coordinates.
    pub axes: [&'static str; 2],
    pub loops: Vec<SectionLoop>,
    /// Small loops left out for lack of points.
    pub more_loops: usize,
}

/// A slice of one part: chains of crossing points.
struct Chain {
    pts: Vec<Vec2>,
    closed: bool,
}

/// Slices a part's faces with the plane `p[axis] = at`. Vertices exactly on
/// the plane count as above it, so every crossing is a proper sign change.
/// Crossing points are keyed by mesh edge, so neighbouring faces share them
/// and segments chain into loops.
///
/// Complexity: O(F + C) for the part's faces and corners.
fn slice_part(part: &Part, axis: SliceAxis, at: f32) -> Vec<Chain> {
    let mesh = part.mesh;
    let k = axis.index();
    let (u, v, _) = axis.plane();
    let world = |vert: u32| part.world.transform_point3(mesh.positions[vert as usize]);
    let above = |p: Vec3| p[k] >= at;
    let flipped = part.world.matrix3.determinant() < 0.0;
    // Crossing point of an edge, computed in a canonical vertex order.
    let crossing = |e: u32| -> Vec2 {
        let [a, b] = mesh.edge_verts(e);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let (pa, pb) = (world(a), world(b));
        let t = (at - pa[k]) / (pb[k] - pa[k]);
        let p = pa + (pb - pa) * t.clamp(0.0, 1.0);
        Vec2::new(p[u], p[v])
    };
    // Segments (start edge, end edge): from where a face loop climbs above
    // the plane to where it drops below again.
    let mut segs: Vec<(u32, u32)> = Vec::new();
    let mut ups: Vec<u32> = Vec::new();
    let mut downs: Vec<u32> = Vec::new();
    for f in mesh.faces() {
        ups.clear();
        downs.clear();
        for h in mesh.face_halfedges(f) {
            let (a, b) = (above(world(mesh.origin(h))), above(world(mesh.dest(h))));
            if a != b {
                if b { ups.push(h >> 1) } else { downs.push(h >> 1) }
            }
        }
        if ups.is_empty() || ups.len() != downs.len() {
            continue;
        }
        if ups.len() == 1 {
            let s = if flipped { (downs[0], ups[0]) } else { (ups[0], downs[0]) };
            segs.push(s);
            continue;
        }
        // A concave face crosses the plane several times: pair crossings in
        // order along the face's line of intersection.
        let n = mesh.face_normal(f);
        let dir = part.world.transform_vector3(n).cross(Vec3::from(match k {
            0 => [1.0, 0.0, 0.0],
            1 => [0.0, 1.0, 0.0],
            _ => [0.0, 0.0, 1.0],
        }));
        let key = |e: u32| {
            let c = crossing(e);
            let mut p = Vec3::ZERO;
            p[u] = c.x;
            p[v] = c.y;
            p.dot(dir)
        };
        let mut all: Vec<(f32, u32, bool)> = ups.iter().map(|&e| (key(e), e, true)).chain(downs.iter().map(|&e| (key(e), e, false))).collect();
        all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for pair in all.as_chunks::<2>().0 {
            let (x, y) = (pair[0], pair[1]);
            let (s, t) = if x.2 { (x.1, y.1) } else { (y.1, x.1) };
            segs.push(if flipped { (t, s) } else { (s, t) });
        }
    }
    // Chain segments through shared edges.
    let mut by_start: FxHashMap<u32, usize> = FxHashMap::default();
    let mut has_pred: FxHashMap<u32, ()> = FxHashMap::default();
    for (i, &(s, t)) in segs.iter().enumerate() {
        by_start.insert(s, i);
        has_pred.insert(t, ());
    }
    let mut used = vec![false; segs.len()];
    let mut chains = Vec::new();
    // Open chains first (they start where nothing leads in), then loops.
    let starts: Vec<usize> = (0..segs.len()).filter(|&i| !has_pred.contains_key(&segs[i].0)).chain(0..segs.len()).collect();
    for first in starts {
        if used[first] {
            continue;
        }
        let mut pts = vec![crossing(segs[first].0)];
        let mut cur = first;
        let closed = loop {
            used[cur] = true;
            let end = segs[cur].1;
            pts.push(crossing(end));
            match by_start.get(&end) {
                Some(&next) if next == first => break true,
                Some(&next) if !used[next] => cur = next,
                _ => break false,
            }
        };
        if closed {
            pts.pop();
        }
        if pts.len() >= 2 {
            chains.push(Chain { pts, closed });
        }
    }
    chains
}

/// Twice the signed area of a closed polygon.
fn area2(pts: &[Vec2]) -> f32 {
    let n = pts.len();
    (0..n).map(|i| pts[i].perp_dot(pts[(i + 1) % n])).sum()
}

fn point_in(poly: &[Vec2], p: Vec2) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// The section of a view.
///
/// Complexity: O(F + C) over the parts, plus O(m log m) simplification and
/// O(p^2) wall measurements on simplified loops of at most p points.
pub fn section(view: &View, opts: &SectionOptions) -> Result<Section, PerceiveError> {
    if !(8..=160).contains(&opts.points) {
        return Err(PerceiveError::new("OUT_OF_RANGE", format!("points is {}, outside 8 to 160", opts.points)).at("/points"));
    }
    let k = opts.axis.index();
    let at = opts.at.unwrap_or((view.lo[k] + view.hi[k]) / 2.0);
    let (u, v, sign) = opts.axis.plane();
    let names = ["x", "y", "z"];
    let mut raw: Vec<(usize, Chain, f32)> = Vec::new();
    for (i, part) in view.parts.iter().enumerate() {
        if at < part.lo[k] || at > part.hi[k] {
            continue;
        }
        for c in slice_part(part, opts.axis, at) {
            let a = if c.closed { sign * area2(&c.pts) / 2.0 } else { 0.0 };
            raw.push((i, c, a));
        }
    }
    // Largest first; drop slivers below a millionth of the largest.
    raw.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()).then(a.0.cmp(&b.0)));
    let biggest = raw.first().map_or(0.0, |r| r.2.abs());
    raw.retain(|r| !r.1.closed || r.2.abs() > biggest * 1e-6);
    let shown = raw.len().min(opts.points / 4);
    let corners: Vec<usize> = raw.iter().take(shown).map(|r| r.1.pts.len()).collect();
    let extra: usize = corners.iter().map(|c| c.saturating_sub(4)).sum::<usize>().max(1);
    let spare = opts.points - 4 * shown;
    let mut loops: Vec<SectionLoop> = Vec::new();
    for (idx, (i, c, a)) in raw.iter().take(shown).enumerate() {
        let share = 4 + spare * corners[idx].saturating_sub(4) / extra;
        let mut pts = c.pts.clone();
        // Outer loops counter-clockwise, holes clockwise, in the output axes.
        if c.closed && sign < 0.0 {
            pts.reverse();
        }
        let keep: Vec<Vec2> = if c.closed { simplify(&pts, share, 0.0).into_iter().map(|j| pts[j]).collect() } else { thin_open(&pts, share) };
        let length: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>() + if c.closed { pts[pts.len() - 1].distance(pts[0]) } else { 0.0 };
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in &pts {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        loops.push(SectionLoop {
            part: view.parts[*i].label.clone(),
            kind: if !c.closed {
                "open"
            } else if *a >= 0.0 {
                "outer"
            } else {
                "hole"
            },
            area: a.abs(),
            length,
            min: lo,
            max: hi,
            points: keep,
            wall: None,
        });
    }
    // Wall thickness of each hole: distance to the enclosing outer loop of
    // the same part, measured on the simplified loops.
    for h in 0..loops.len() {
        if loops[h].kind != "hole" {
            continue;
        }
        let probe = loops[h].points[0];
        let outer = (0..loops.len()).find(|&o| loops[o].kind == "outer" && loops[o].part == loops[h].part && point_in(&loops[o].points, probe));
        if let Some(o) = outer {
            let op = &loops[o].points;
            let wall = loops[h]
                .points
                .iter()
                .map(|p| (0..op.len()).map(|j| seg_dist(*p, op[j], op[(j + 1) % op.len()])).fold(f32::INFINITY, f32::min))
                .fold(f32::INFINITY, f32::min);
            loops[h].wall = Some(wall);
        }
    }
    Ok(Section { axis: opts.axis, at, axes: [names[u], names[v]], more_loops: raw.len() - shown, loops })
}

/// Every k-th point of an open polyline, ends kept.
fn thin_open(pts: &[Vec2], target: usize) -> Vec<Vec2> {
    if pts.len() <= target {
        return pts.to_vec();
    }
    let step = (pts.len() - 1).div_ceil(target - 1);
    let mut out: Vec<Vec2> = pts.iter().step_by(step).copied().collect();
    if out.last() != pts.last() {
        out.extend(pts.last());
    }
    out
}

/// Compact text of a section.
pub fn section_text(view: &View, s: &Section) -> String {
    let [u, v] = s.axes;
    let what = match s.axis {
        SliceAxis::Y => "horizontal",
        _ => "vertical",
    };
    let mut t = format!("section {}={} of {} ({what} slice, mm, ({u}, {v}))\n", s.axis.name(), mm(s.at), view.title());
    if s.loops.is_empty() {
        t.push_str("the plane misses every part\n");
        return t;
    }
    for l in &s.loops {
        let size = l.max - l.min;
        let mut line = format!("{} {}: {} x {} mm", l.part, l.kind, mm(size.x), mm(size.y));
        if l.kind != "open" {
            line.push_str(&format!(", area {}", area(f64::from(l.area))));
        }
        if let Some(w) = l.wall {
            line.push_str(&format!(", wall {} mm", mm(w)));
        }
        let pts: Vec<String> = l.points.iter().map(|p| format!("({},{})", mm0(p.x), mm0(p.y))).collect();
        line.push_str(&format!(": {}", pts.join(" ")));
        t.push_str(&line);
        t.push('\n');
    }
    if s.more_loops > 0 {
        t.push_str(&format!("+{} smaller loops (more points, or a target, show them)\n", s.more_loops));
    }
    t
}

/// The `section` report of a view.
pub fn report(view: &View, opts: &SectionOptions) -> Result<Report, PerceiveError> {
    let s = section(view, opts)?;
    Ok(Report::new("section", section_text(view, &s), &s))
}
