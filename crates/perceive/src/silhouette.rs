//! `silhouette`: how does the outline read from a view? Orthographic
//! outlines as short polylines in world millimeters, labelled with the part
//! that forms each stretch, plus extremes and holes.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::Report;
use crate::error::PerceiveError;
use crate::fmt::{mm, mm0};
use crate::view::View;

/// Default number of outline points.
pub const DEFAULT_POINTS: usize = 32;
/// Default raster cells along the longer side.
pub const DEFAULT_RESOLUTION: usize = 256;
/// Outlines and holes smaller than this many cells are raster noise.
const MIN_LOOP_CELLS: i64 = 4;

/// Where the orthographic camera sits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewDir {
    /// From +Z (the asset's front): outline in (x, y).
    #[default]
    Front,
    /// From -Z: outline in (x, y).
    Back,
    /// From +X (the asset's right side): outline in (z, y).
    Right,
    /// From -X: outline in (z, y).
    Left,
    /// From +Y: outline in (x, z).
    Top,
    /// From -Y: outline in (x, z).
    Bottom,
}

impl ViewDir {
    pub fn name(self) -> &'static str {
        match self {
            ViewDir::Front => "front",
            ViewDir::Back => "back",
            ViewDir::Right => "right",
            ViewDir::Left => "left",
            ViewDir::Top => "top",
            ViewDir::Bottom => "bottom",
        }
    }

    /// World axes of the outline's (u, v) and of the depth toward the camera.
    fn axes(self) -> (usize, usize, usize, f32) {
        match self {
            ViewDir::Front => (0, 1, 2, 1.0),
            ViewDir::Back => (0, 1, 2, -1.0),
            ViewDir::Right => (2, 1, 0, 1.0),
            ViewDir::Left => (2, 1, 0, -1.0),
            ViewDir::Top => (0, 2, 1, 1.0),
            ViewDir::Bottom => (0, 2, 1, -1.0),
        }
    }

    fn axis_names(self) -> (&'static str, &'static str) {
        let (u, v, _, _) = self.axes();
        let n = |k: usize| ["x", "y", "z"][k];
        (n(u), n(v))
    }
}

/// Options of the `silhouette` aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilhouetteOptions {
    pub view: ViewDir,
    /// Outline points over all loops (8 to 128).
    pub points: usize,
    /// Raster cells along the longer side (64 to 1024).
    pub resolution: usize,
}

impl Default for SilhouetteOptions {
    fn default() -> Self {
        SilhouetteOptions { view: ViewDir::Front, points: DEFAULT_POINTS, resolution: DEFAULT_RESOLUTION }
    }
}

/// One outline point: position (world meters in the view's axes) and the
/// part forming the outline there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutlinePoint {
    pub at: Vec2,
    pub part: String,
}

/// A closed outline: counter-clockwise for outer outlines, clockwise for
/// holes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outline {
    pub hole: bool,
    /// Area in square meters.
    pub area: f32,
    pub min: Vec2,
    pub max: Vec2,
    pub points: Vec<OutlinePoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Extreme {
    pub at: Vec2,
    pub part: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Silhouette {
    pub view: ViewDir,
    /// World axes of the outline coordinates, for example `["x", "y"]`.
    pub axes: [&'static str; 2],
    /// Raster cell size in meters (the outline's precision).
    pub cell: f32,
    pub min: Vec2,
    pub max: Vec2,
    pub outlines: Vec<Outline>,
    /// Small outlines left out for lack of points.
    pub more_outlines: usize,
    pub left: Option<Extreme>,
    pub right: Option<Extreme>,
    pub top: Option<Extreme>,
    pub bottom: Option<Extreme>,
}

/// A raster of the view: per cell the frontmost depth and its part.
pub struct Raster {
    pub w: usize,
    pub h: usize,
    pub cell: f32,
    /// World (u, v) of cell (0, 0)'s lower corner.
    pub origin: Vec2,
    depth: Vec<f32>,
    /// Part index + 1; 0 for empty.
    owner: Vec<u32>,
}

impl Raster {
    #[inline]
    pub fn filled(&self, i: isize, j: isize) -> bool {
        i >= 0 && j >= 0 && (i as usize) < self.w && (j as usize) < self.h && self.owner[j as usize * self.w + i as usize] != 0
    }

    /// Part index owning a cell.
    #[inline]
    pub fn owner(&self, i: usize, j: usize) -> Option<usize> {
        let o = self.owner[j * self.w + i];
        (o != 0).then(|| o as usize - 1)
    }

    pub fn filled_count(&self) -> usize {
        self.owner.iter().filter(|&&o| o != 0).count()
    }
}

/// Rasterizes every part's triangles projected along the view direction.
/// A cell is covered when its center lies in a triangle; cells holding a
/// triangle corner are covered too, so parts thinner than a cell still show.
/// `bounds` (u, v) fixes the raster frame (the view's own bounds when
/// `None`), so two rasters can be compared cell by cell.
///
/// Complexity: O(triangles + covered cells).
pub fn rasterize(view: &View, dir: ViewDir, resolution: usize, bounds: Option<(Vec2, Vec2)>) -> Raster {
    let (ua, va, da, sign) = dir.axes();
    let (lo, hi) = bounds.unwrap_or((Vec2::new(view.lo[ua], view.lo[va]), Vec2::new(view.hi[ua], view.hi[va])));
    let ext = (hi - lo).max(Vec2::splat(1e-6));
    let cell = ext.max_element() / resolution as f32;
    // One empty cell of margin on every side keeps outlines closed.
    let origin = lo - Vec2::splat(cell);
    let w = (ext.x / cell).ceil() as usize + 2;
    let h = (ext.y / cell).ceil() as usize + 2;
    let mut r = Raster { w, h, cell, origin, depth: vec![f32::NEG_INFINITY; w * h], owner: vec![0; w * h] };
    for (pi, part) in view.parts.iter().enumerate() {
        let bvh = part.bvh();
        for t in 0..bvh.len() as u32 {
            let tri = bvh.triangle(t);
            let p: [Vec2; 3] = tri.map(|q| (Vec2::new(q[ua], q[va]) - origin) / cell);
            let d: [f32; 3] = tri.map(|q| q[da] * sign);
            r.fill_triangle(p, d, pi as u32 + 1);
        }
    }
    r
}

impl Raster {
    fn put(&mut self, i: usize, j: usize, depth: f32, owner: u32) {
        let k = j * self.w + i;
        if self.owner[k] == 0 || depth > self.depth[k] || (depth == self.depth[k] && owner < self.owner[k]) {
            self.depth[k] = depth;
            self.owner[k] = owner;
        }
    }

    /// Covers cells whose centers lie in the triangle (cell units), plus the
    /// cells of its corners.
    fn fill_triangle(&mut self, p: [Vec2; 3], d: [f32; 3], owner: u32) {
        let mut covered = 0usize;
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if area.abs() >= 1e-12 {
            let lo = p[0].min(p[1]).min(p[2]);
            let hi = p[0].max(p[1]).max(p[2]);
            let (i0, j0) = ((lo.x - 0.5).ceil().max(0.0) as isize, (lo.y - 0.5).ceil().max(0.0) as isize);
            let (i1, j1) = (((hi.x - 0.5).floor() as isize).min(self.w as isize - 1), ((hi.y - 0.5).floor() as isize).min(self.h as isize - 1));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let c = Vec2::new(i as f32 + 0.5, j as f32 + 0.5);
                    let w0 = (p[2] - p[1]).perp_dot(c - p[1]) / area;
                    let w1 = (p[0] - p[2]).perp_dot(c - p[2]) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                        self.put(i as usize, j as usize, w0 * d[0] + w1 * d[1] + w2 * d[2], owner);
                        covered += 1;
                    }
                }
            }
        }
        if covered == 0 {
            // Thinner than a cell (a wire, a sliver, an edge-on face): cover
            // the cells along its edges so it still shows.
            for k in 0..3 {
                let (a, b) = (p[k], p[(k + 1) % 3]);
                let steps = ((b - a).length() * 2.0).ceil().max(1.0) as usize;
                for s in 0..=steps {
                    let t = s as f32 / steps as f32;
                    let q = a + (b - a) * t;
                    let (i, j) = (q.x.floor(), q.y.floor());
                    if i >= 0.0 && j >= 0.0 && (i as usize) < self.w && (j as usize) < self.h {
                        self.put(i as usize, j as usize, d[k] + (d[(k + 1) % 3] - d[k]) * t, owner);
                    }
                }
            }
        }
    }
}

/// An extreme run candidate: (sort key, lower corner, upper corner).
type Run = ((i64, i64), (i32, i32), (i32, i32));

/// A traced outline on the cell lattice: corner coordinates (cell units).
struct Loop {
    pts: Vec<(i32, i32)>,
    /// Twice the signed area in cells (positive: outer, counter-clockwise).
    area2: i64,
}

/// Traces every boundary between filled and empty cells into closed loops,
/// filled side on the left: outer outlines counter-clockwise, holes
/// clockwise. Diagonal neighbours count as connected. Straight runs are
/// merged.
///
/// Complexity: O(cells).
fn trace(r: &Raster) -> Vec<Loop> {
    let lw = r.w + 1;
    let vid = |i: i32, j: i32| j as usize * lw + i as usize;
    // Directed boundary edges: start corner and direction.
    let mut edges: Vec<((i32, i32), (i32, i32))> = Vec::new();
    for j in 0..r.h as isize {
        for i in 0..r.w as isize {
            if !r.filled(i, j) {
                continue;
            }
            let (x, y) = (i as i32, j as i32);
            if !r.filled(i, j - 1) {
                edges.push(((x, y), (1, 0)));
            }
            if !r.filled(i + 1, j) {
                edges.push(((x + 1, y), (0, 1)));
            }
            if !r.filled(i, j + 1) {
                edges.push(((x + 1, y + 1), (-1, 0)));
            }
            if !r.filled(i - 1, j) {
                edges.push(((x, y + 1), (0, -1)));
            }
        }
    }
    let mut starts = vec![[u32::MAX; 2]; lw * (r.h + 1)];
    for (k, (s, _)) in edges.iter().enumerate() {
        let slot = &mut starts[vid(s.0, s.1)];
        if slot[0] == u32::MAX {
            slot[0] = k as u32;
        } else {
            slot[1] = k as u32;
        }
    }
    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();
    for first in 0..edges.len() {
        if used[first] {
            continue;
        }
        let mut pts: Vec<(i32, i32)> = Vec::new();
        let mut k = first;
        let mut area2 = 0i64;
        loop {
            used[k] = true;
            let (s, d) = edges[k];
            let e = (s.0 + d.0, s.1 + d.1);
            area2 += i64::from(s.0) * i64::from(e.1) - i64::from(e.0) * i64::from(s.1);
            pts.push(s);
            let cands = starts[vid(e.0, e.1)];
            let next = match (cands[0], cands[1]) {
                (a, u32::MAX) if a != u32::MAX && !used[a as usize] => Some(a as usize),
                (a, b) if a != u32::MAX && b != u32::MAX => {
                    // Saddle: turn right, joining diagonal neighbours.
                    let right = (d.1, -d.0);
                    let pick = if edges[a as usize].1 == right { a } else { b };
                    let other = if pick == a { b } else { a };
                    if !used[pick as usize] {
                        Some(pick as usize)
                    } else if !used[other as usize] {
                        Some(other as usize)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            match next {
                Some(n) => k = n,
                None => break,
            }
        }
        // Keep only corners where the direction changes.
        let n = pts.len();
        if n >= 3 {
            let mut keep = Vec::with_capacity(n);
            for idx in 0..n {
                let (a, b, c) = (pts[(idx + n - 1) % n], pts[idx], pts[(idx + 1) % n]);
                let cross = (i64::from(b.0) - i64::from(a.0)) * (i64::from(c.1) - i64::from(b.1))
                    - (i64::from(b.1) - i64::from(a.1)) * (i64::from(c.0) - i64::from(b.0));
                if cross != 0 {
                    keep.push(b);
                }
            }
            pts = keep;
        }
        if pts.len() >= 3 {
            loops.push(Loop { pts, area2 });
        }
    }
    loops
}

/// Visvalingam-Whyatt simplification of a closed polygon: repeatedly drops
/// the point spanning the smallest triangle with its neighbours while more
/// than `target` points remain or that triangle is smaller than `noise`
/// (twice its area, in squared units). Ties go to the lower index.
///
/// Complexity: O(m log m).
pub fn simplify(pts: &[Vec2], target: usize, noise: f32) -> Vec<usize> {
    let m = pts.len();
    let target = target.max(3);
    if m <= 3 {
        return (0..m).collect();
    }
    let mut prev: Vec<usize> = (0..m).map(|i| (i + m - 1) % m).collect();
    let mut next: Vec<usize> = (0..m).map(|i| (i + 1) % m).collect();
    let mut alive = vec![true; m];
    let area = |p: &[usize], n: &[usize], i: usize| ((pts[i] - pts[p[i]]).perp_dot(pts[n[i]] - pts[p[i]])).abs();
    let mut heap: BinaryHeap<Reverse<(u32, usize)>> = (0..m).map(|i| Reverse((area(&prev, &next, i).to_bits(), i))).collect();
    let mut stamp: Vec<u32> = (0..m).map(|i| area(&prev, &next, i).to_bits()).collect();
    let mut count = m;
    while count > 3 {
        let Some(Reverse((key, i))) = heap.pop() else { break };
        if !alive[i] || key != stamp[i] {
            continue;
        }
        if count <= target && f32::from_bits(key) >= noise {
            break;
        }
        alive[i] = false;
        count -= 1;
        let (p, n) = (prev[i], next[i]);
        next[p] = n;
        prev[n] = p;
        for j in [p, n] {
            let a = area(&prev, &next, j).to_bits();
            stamp[j] = a;
            heap.push(Reverse((a, j)));
        }
    }
    (0..m).filter(|&i| alive[i]).collect()
}

/// The part forming the outline at a lattice corner: the frontmost of the
/// filled cells around it.
fn corner_owner(r: &Raster, (x, y): (i32, i32)) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (di, dj) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
        let (i, j) = (x as isize + di, y as isize + dj);
        if r.filled(i, j) {
            let k = j as usize * r.w + i as usize;
            let (d, o) = (r.depth[k], r.owner[k] as usize - 1);
            if best.is_none_or(|(bd, bo)| d > bd || (d == bd && o < bo)) {
                best = Some((d, o));
            }
        }
    }
    best.map(|b| b.1)
}

/// The silhouette of a view.
///
/// Complexity: O(triangles + cells + m log m) for m traced corners.
pub fn silhouette(view: &View, opts: &SilhouetteOptions) -> Result<Silhouette, PerceiveError> {
    if !(8..=128).contains(&opts.points) {
        return Err(PerceiveError::new("OUT_OF_RANGE", format!("points is {}, outside 8 to 128", opts.points)).at("/points"));
    }
    if !(64..=1024).contains(&opts.resolution) {
        return Err(PerceiveError::new("OUT_OF_RANGE", format!("resolution is {}, outside 64 to 1024", opts.resolution)).at("/resolution"));
    }
    let (ua, va) = opts.view.axis_names();
    let r = rasterize(view, opts.view, opts.resolution, None);
    let (ui, vi, _, _) = opts.view.axes();
    let (blo, bhi) = (Vec2::new(view.lo[ui], view.lo[vi]), Vec2::new(view.hi[ui], view.hi[vi]));
    // Corners within 1.5 cells of the exact projected bounds sit on them: the
    // outermost numbers (widths, heights, the ground) are exact.
    let snap = |p: Vec2| {
        let s = |x: f32, lo: f32, hi: f32| {
            if (x - lo).abs() < 1.5 * r.cell {
                lo
            } else if (x - hi).abs() < 1.5 * r.cell {
                hi
            } else {
                x
            }
        };
        Vec2::new(s(p.x, blo.x, bhi.x), s(p.y, blo.y, bhi.y))
    };
    let to_world = |(x, y): (i32, i32)| snap(r.origin + Vec2::new(x as f32, y as f32) * r.cell);
    let label = |o: Option<usize>| o.map_or_else(String::new, |i| view.parts[i].label.clone());
    let mut loops = trace(&r);
    // Pockets of a few cells are raster noise, not holes or parts.
    loops.retain(|l| l.area2.abs() >= 2 * MIN_LOOP_CELLS);
    // Largest outlines first; deterministic.
    loops.sort_by(|a, b| b.area2.abs().cmp(&a.area2.abs()).then(a.pts.cmp(&b.pts)));

    // Extremes: outlines are made of axis-aligned runs, so the extreme in
    // each direction is a run. Report the midpoint of the one nearest the
    // middle of the view, and the part in the cell just inside it.
    let (w, h) = (i64::from(r.w as i32), i64::from(r.h as i32));
    // (key, inside cell) per direction: left, right, top, bottom.
    let mut ext: [Option<Run>; 4] = [None; 4];
    for l in loops.iter().filter(|l| l.area2 > 0) {
        let n = l.pts.len();
        for k in 0..n {
            let (a, b) = (l.pts[k], l.pts[(k + 1) % n]);
            let cands: [(usize, (i64, i64)); 2] = if a.0 == b.0 {
                let off = (i64::from(a.1) + i64::from(b.1) - h).abs();
                [(0, (i64::from(a.0), off)), (1, (-i64::from(a.0), off))]
            } else {
                let off = (i64::from(a.0) + i64::from(b.0) - w).abs();
                [(2, (-i64::from(a.1), off)), (3, (i64::from(a.1), off))]
            };
            for (d, key) in cands {
                let (lo, hi) = (a.min(b), a.max(b));
                if ext[d].is_none_or(|(c, cl, ch)| (key, lo, hi) < (c, cl, ch)) {
                    ext[d] = Some((key, lo, hi));
                }
            }
        }
    }
    let extreme = |d: usize| {
        ext[d].map(|(_, lo, hi)| {
            let mid = Vec2::new(lo.0 as f32 + hi.0 as f32, lo.1 as f32 + hi.1 as f32) * 0.5;
            let (mi, mj) = ((lo.0 + hi.0) / 2, (lo.1 + hi.1) / 2);
            let cell = match d {
                0 => (lo.0, mj.min(hi.1 - 1)),
                1 => (lo.0 - 1, mj.min(hi.1 - 1)),
                2 => (mi.min(hi.0 - 1), lo.1 - 1),
                _ => (mi.min(hi.0 - 1), lo.1),
            };
            let owner = if r.filled(cell.0 as isize, cell.1 as isize) { r.owner(cell.0 as usize, cell.1 as usize) } else { corner_owner(&r, lo) };
            Extreme { at: snap(r.origin + mid * r.cell), part: label(owner) }
        })
    };

    // Staircase steps of the raster (triangles under one cell) go first;
    // then the point budget is shared: 4 per outline, the rest in proportion
    // to how many corners each outline still has.
    let noise = 2.0 * r.cell * r.cell;
    let worlds: Vec<Vec<Vec2>> = loops.iter().map(|l| l.pts.iter().map(|&p| to_world(p)).collect()).collect();
    let shown = loops.len().min(opts.points / 4);
    let corners: Vec<usize> = worlds.iter().take(shown).map(|wp| simplify(wp, usize::MAX, noise).len()).collect();
    let extra: usize = corners.iter().map(|c| c.saturating_sub(4)).sum::<usize>().max(1);
    let spare = opts.points - 4 * shown;
    let mut outlines = Vec::new();
    for (k, l) in loops.iter().take(shown).enumerate() {
        let share = 4 + spare * corners[k].saturating_sub(4) / extra;
        let world = &worlds[k];
        let keep = simplify(world, share, noise);
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in world {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        outlines.push(Outline {
            hole: l.area2 < 0,
            area: (l.area2.abs() as f32 / 2.0) * r.cell * r.cell,
            min: lo,
            max: hi,
            points: keep.iter().map(|&k| OutlinePoint { at: world[k], part: label(corner_owner(&r, l.pts[k])) }).collect(),
        });
    }
    let more = loops.len() - shown;
    let (lo, hi) = outlines
        .iter()
        .filter(|o| !o.hole)
        .fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(l, h), o| (l.min(o.min), h.max(o.max)));
    let (lo, hi) = if outlines.is_empty() { (Vec2::ZERO, Vec2::ZERO) } else { (lo, hi) };
    Ok(Silhouette {
        view: opts.view,
        axes: [ua, va],
        cell: r.cell,
        min: lo,
        max: hi,
        outlines,
        more_outlines: more,
        left: extreme(0),
        right: extreme(1),
        top: extreme(2),
        bottom: extreme(3),
    })
}

/// Compact text of a silhouette.
pub fn silhouette_text(view: &View, s: &Silhouette) -> String {
    let pts: usize = s.outlines.iter().map(|o| o.points.len()).sum();
    let [u, v] = s.axes;
    let mut t = format!("silhouette {} of {} ({pts} pts, mm, ({u}, {v}), precision {} mm)\n", s.view.name(), view.title(), mm(s.cell));
    if s.outlines.is_empty() {
        t.push_str("empty: no geometry in view\n");
        return t;
    }
    let size = s.max - s.min;
    t.push_str(&format!(
        "  width {} ({u} {} to {}), height {} ({v} {} to {})\n",
        mm(size.x),
        mm(s.min.x),
        mm(s.max.x),
        mm(size.y),
        mm(s.min.y),
        mm(s.max.y)
    ));
    for (k, o) in s.outlines.iter().enumerate() {
        let kind = if o.hole { "hole" } else { "outline" };
        let mut line = format!("  {kind} {}: ", k + 1);
        let mut last = "";
        for p in &o.points {
            if p.part != last {
                line.push_str(&format!("({},{} {}) ", mm0(p.at.x), mm0(p.at.y), p.part));
                last = &p.part;
            } else {
                line.push_str(&format!("({},{}) ", mm0(p.at.x), mm0(p.at.y)));
            }
        }
        t.push_str(line.trim_end());
        t.push('\n');
    }
    if s.more_outlines > 0 {
        t.push_str(&format!("  +{} smaller outlines (more points, or a target, show them)\n", s.more_outlines));
    }
    let e = |name: &str, x: &Option<Extreme>| x.as_ref().map(|x| format!("{name} {} ({},{})", x.part, mm0(x.at.x), mm0(x.at.y)));
    let ex: Vec<String> = [e("left", &s.left), e("right", &s.right), e("top", &s.top), e("bottom", &s.bottom)].into_iter().flatten().collect();
    t.push_str(&format!("  extremes: {}\n", ex.join(", ")));
    t
}

/// The `silhouette` report of a view.
pub fn report(view: &View, opts: &SilhouetteOptions) -> Result<Report, PerceiveError> {
    let s = silhouette(view, opts)?;
    Ok(Report::new("silhouette", silhouette_text(view, &s), &s))
}

/// Projects a world point into a view's outline coordinates.
pub fn project(dir: ViewDir, p: Vec3) -> Vec2 {
    let (u, v, _, _) = dir.axes();
    Vec2::new(p[u], p[v])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_keeps_the_corners_of_a_square() {
        let mut pts = Vec::new();
        for i in 0..10 {
            pts.push(Vec2::new(i as f32, 0.0));
        }
        for i in 0..10 {
            pts.push(Vec2::new(10.0, i as f32));
        }
        for i in 0..10 {
            pts.push(Vec2::new(10.0 - i as f32, 10.0));
        }
        for i in 0..10 {
            pts.push(Vec2::new(0.0, 10.0 - i as f32));
        }
        let keep = simplify(&pts, 4, 0.0);
        let kept: Vec<Vec2> = keep.iter().map(|&k| pts[k]).collect();
        assert_eq!(kept, vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(0.0, 10.0)]);
        assert_eq!(simplify(&pts[..3], 8, 0.0).len(), 3);
        // Below the noise floor even a generous budget drops the collinear points.
        assert_eq!(simplify(&pts, 100, 0.5).len(), 4);
    }
}
