//! The `vegetation` aspect (ADR 0009, ADR 0014): how a tree, bush, flower
//! or grass patch reads, from its mesh alone. Per part with a `foliage` or
//! `trunk` group: crown size and proportions, the fork and trunk, card
//! count, the crown silhouette's coverage and its gaps, the overdraw of the
//! cards and how much of the trunk shows through. Generated trees also
//! carry their report as metadata `vegetation` (species, coverage target,
//! repeats), shown when present.

use glam::{Vec2, Vec3};
use p3d_mesh::{ElementKind, Mesh};
use serde::Serialize;

use crate::fmt::thousands;
use crate::view::Part;
use crate::{Report, View};

/// Raster cells across the crown's larger extent.
const RESOLUTION: usize = 128;
/// Height of the trunk diameter measure (breast height), meters.
const BREAST_HEIGHT: f32 = 1.3;

/// What one vegetation part looks like.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Measured {
    pub part: String,
    /// From the generator's metadata, when present.
    pub species: Option<String>,
    pub foliage: Option<String>,
    pub height: f32,
    /// Crown extents along X, Z and Y, and its lowest point above the base.
    pub crown_width: f32,
    pub crown_depth: f32,
    pub crown_height: f32,
    pub crown_base: f32,
    /// Fork height above the base (from `lm.fork`).
    pub fork: Option<f32>,
    /// Trunk diameter at breast height and the root flare's width over it.
    pub trunk_diameter: Option<f32>,
    pub root_spread: Option<f32>,
    /// Scaffold limbs (pieces of `level_1`).
    pub limbs: usize,
    /// Foliage pieces (cards, clumps, fronds) and their triangles.
    pub cards: usize,
    pub leaf_cards: usize,
    pub foliage_tris: usize,
    pub wood_tris: usize,
    /// Covered share of the crown silhouette, front and side.
    pub coverage: [f32; 2],
    /// Uncovered patches inside the silhouette of two cards or more, and the
    /// largest one's area (m2) and center (view coordinates, front).
    pub gaps: usize,
    pub largest_gap: Option<(f32, Vec2)>,
    /// Card layers per covered pixel, front and side, and the most layers
    /// on one pixel.
    pub overdraw: [f32; 2],
    pub overdraw_max: u32,
    /// Share of the trunk's silhouette not hidden by foliage, front and side.
    pub trunk_visible: Option<[f32; 2]>,
    /// Wind data present (vertex color and a second UV layer).
    pub wind: bool,
    /// Target and neighbouring repeats from the generator.
    pub coverage_target: Option<f32>,
    pub repeats: Option<u64>,
}

impl Measured {
    pub fn coverage_mean(&self) -> f32 {
        (self.coverage[0] + self.coverage[1]) * 0.5
    }

    pub fn overdraw_mean(&self) -> f32 {
        (self.overdraw[0] + self.overdraw[1]) * 0.5
    }
}

/// Face membership of a group (all false when missing).
///
/// Complexity: O(F + group size).
fn member(mesh: &Mesh, names: &[&str]) -> Vec<bool> {
    let mut out = vec![false; mesh.face_slots()];
    for n in names {
        if let Some(g) = mesh.group(n).filter(|g| g.kind() == ElementKind::Face) {
            for f in g.iter() {
                if let Some(x) = out.get_mut(f as usize) {
                    *x = true;
                }
            }
        }
    }
    out
}

/// Connected pieces among the selected faces (across shared edges), with
/// each piece's area.
///
/// Complexity: O(F + C).
fn pieces(mesh: &Mesh, sel: &[bool]) -> Vec<f32> {
    let mut seen = vec![false; mesh.face_slots()];
    let mut stack = Vec::new();
    let mut out = Vec::new();
    for f in mesh.faces() {
        if !sel[f as usize] || seen[f as usize] {
            continue;
        }
        seen[f as usize] = true;
        stack.push(f);
        let mut area = 0.0f32;
        while let Some(g) = stack.pop() {
            area += mesh.face_area(g);
            for h in mesh.face_halfedges(g) {
                if let Some(n) = mesh.face(mesh.twin(h))
                    && sel[n as usize]
                    && !seen[n as usize]
                {
                    seen[n as usize] = true;
                    stack.push(n);
                }
            }
        }
        out.push(area);
    }
    out
}

/// A view's cells over the crown, with a margin.
#[derive(Debug, Clone, Copy)]
struct Grid {
    w: usize,
    h: usize,
    cell: f32,
    origin: Vec2,
}

impl Grid {
    fn new(lo: Vec2, hi: Vec2) -> Grid {
        let ext = (hi - lo).max(Vec2::splat(1e-6));
        let cell = ext.max_element() / RESOLUTION as f32;
        let origin = lo - Vec2::splat(cell * 2.0);
        Grid { w: (ext.x / cell).ceil() as usize + 4, h: (ext.y / cell).ceil() as usize + 4, cell, origin }
    }

    /// Calls `f(cell, depth)` for every cell center in the triangle.
    ///
    /// Complexity: O(cells in its bounding box).
    fn scan(&self, p: [Vec2; 3], d: [f32; 3], mut f: impl FnMut(usize, f32)) {
        let p = p.map(|q| (q - self.origin) / self.cell);
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if area.abs() < 1e-12 {
            return;
        }
        let lo = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
        let hi = p[0].max(p[1]).max(p[2]).ceil().min(Vec2::new(self.w as f32 - 1.0, self.h as f32 - 1.0));
        if lo.x > hi.x || lo.y > hi.y {
            return;
        }
        for j in lo.y as usize..=hi.y as usize {
            for i in lo.x as usize..=hi.x as usize {
                let c = Vec2::new(i as f32 + 0.5, j as f32 + 0.5);
                let w0 = (p[2] - p[1]).perp_dot(c - p[1]) / area;
                let w1 = (p[0] - p[2]).perp_dot(c - p[2]) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                f(j * self.w + i, w0 * d[0] + w1 * d[1] + w2 * d[2]);
            }
        }
    }
}

/// Separable square dilation (`grow`) or erosion of a mask by `k` cells.
///
/// Complexity: O(cells x k).
fn morph(mask: &[bool], w: usize, h: usize, k: usize, grow: bool) -> Vec<bool> {
    let pass = |src: &[bool], horizontal: bool| -> Vec<bool> {
        let mut out = vec![!grow; src.len()];
        for j in 0..h {
            for i in 0..w {
                let mut v = !grow;
                let (a, b) = if horizontal { (i.saturating_sub(k), (i + k).min(w - 1)) } else { (j.saturating_sub(k), (j + k).min(h - 1)) };
                for t in a..=b {
                    let s = if horizontal { src[j * w + t] } else { src[t * w + i] };
                    if s == grow {
                        v = grow;
                        break;
                    }
                }
                // Cells near the edge erode as if outside were empty.
                if !grow && ((horizontal && (i < k || i + k >= w)) || (!horizontal && (j < k || j + k >= h))) {
                    v = false;
                }
                out[j * w + i] = v;
            }
        }
        out
    };
    let a = pass(mask, true);
    pass(&a, false)
}

/// Coverage, gaps, overdraw and trunk visibility in one view: `ua` and
/// `va` are the view's world axes, `da` the depth axis toward the viewer.
///
/// Complexity: O(F + cells x k) for F faces and closing radius k.
#[allow(clippy::type_complexity)]
fn view_measure(
    part: &Part,
    leaves: &[bool],
    trunk: &[bool],
    card: f32,
    axes: (usize, usize, usize),
) -> (f32, usize, Option<(f32, Vec2)>, f32, u32, Option<f32>) {
    let (ua, va, da) = axes;
    let mesh = part.mesh;
    let world = |v: u32| part.world.transform_point3(mesh.positions[v as usize]);
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for f in mesh.faces().filter(|&f| leaves[f as usize]) {
        for v in mesh.face_verts(f) {
            let p = world(v);
            lo = lo.min(Vec2::new(p[ua], p[va]));
            hi = hi.max(Vec2::new(p[ua], p[va]));
        }
    }
    if !lo.is_finite() {
        return (0.0, 0, None, 0.0, 0, None);
    }
    let g = Grid::new(lo, hi);
    let n = g.w * g.h;
    // Card layers per cell, and the nearest foliage and trunk depths.
    let mut count = vec![0u32; n];
    let mut leaf_depth = vec![f32::NEG_INFINITY; n];
    let mut trunk_depth = vec![f32::NEG_INFINITY; n];
    let mut trunk_px = 0usize;
    for f in mesh.faces() {
        let (is_leaf, is_trunk) = (leaves[f as usize], trunk[f as usize]);
        if !is_leaf && !is_trunk {
            continue;
        }
        // A fan from the first corner.
        let h0 = mesh.face_he(f);
        let mut h1 = mesh.next(h0);
        loop {
            let h2 = mesh.next(h1);
            if h2 == h0 {
                break;
            }
            let tri = [h0, h1, h2].map(|h| world(mesh.origin(h)));
            let p = tri.map(|q| Vec2::new(q[ua], q[va]));
            let d = tri.map(|q| q[da]);
            if is_leaf {
                g.scan(p, d, |c, z| {
                    count[c] += 1;
                    leaf_depth[c] = leaf_depth[c].max(z);
                });
            } else {
                g.scan(p, d, |c, z| trunk_depth[c] = trunk_depth[c].max(z));
            }
            h1 = h2;
        }
    }
    let covered: Vec<bool> = count.iter().map(|&c| c > 0).collect();
    // The crown silhouette: the covered cells closed over about a card,
    // plus every pocket the outside cannot reach.
    let k = ((card * 0.75 / g.cell).ceil() as usize).clamp(1, RESOLUTION / 6);
    let closed = morph(&morph(&covered, g.w, g.h, k, true), g.w, g.h, k, false);
    let mut outside = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..g.w {
        stack.push(i);
        stack.push((g.h - 1) * g.w + i);
    }
    for j in 0..g.h {
        stack.push(j * g.w);
        stack.push(j * g.w + g.w - 1);
    }
    while let Some(c) = stack.pop() {
        if outside[c] || closed[c] || covered[c] {
            continue;
        }
        outside[c] = true;
        let (i, j) = (c % g.w, c / g.w);
        if i > 0 {
            stack.push(c - 1);
        }
        if i + 1 < g.w {
            stack.push(c + 1);
        }
        if j > 0 {
            stack.push(c - g.w);
        }
        if j + 1 < g.h {
            stack.push(c + g.w);
        }
    }
    let inside: Vec<bool> = (0..n).map(|c| !outside[c]).collect();
    let total = inside.iter().filter(|x| **x).count().max(1);
    let hit = (0..n).filter(|&c| inside[c] && covered[c]).count();
    // Gaps: uncovered pieces inside the silhouette.
    let cell_area = g.cell * g.cell;
    let mut seen = vec![false; n];
    let (mut gaps, mut largest): (usize, Option<(f32, Vec2)>) = (0, None);
    for s in 0..n {
        if seen[s] || !inside[s] || covered[s] {
            continue;
        }
        seen[s] = true;
        stack.push(s);
        let (mut cells, mut sum) = (0usize, Vec2::ZERO);
        while let Some(c) = stack.pop() {
            cells += 1;
            let (i, j) = (c % g.w, c / g.w);
            sum += Vec2::new(i as f32 + 0.5, j as f32 + 0.5);
            let mut push = |d: usize| {
                if !seen[d] && inside[d] && !covered[d] {
                    seen[d] = true;
                    stack.push(d);
                }
            };
            if i > 0 {
                push(c - 1);
            }
            if i + 1 < g.w {
                push(c + 1);
            }
            if j > 0 {
                push(c - g.w);
            }
            if j + 1 < g.h {
                push(c + g.w);
            }
        }
        let area = cells as f32 * cell_area;
        if area >= card * card * 2.0 {
            gaps += 1;
            let center = g.origin + sum / cells as f32 * g.cell;
            if largest.is_none_or(|(a, _)| area > a) {
                largest = Some((area, center));
            }
        }
    }
    let layers: u64 = count.iter().map(|&c| u64::from(c)).sum();
    let drawn = covered.iter().filter(|x| **x).count().max(1);
    let max = count.iter().copied().max().unwrap_or(0);
    let mut seen_trunk = 0usize;
    for (t, l) in trunk_depth.iter().zip(&leaf_depth) {
        if *t > f32::NEG_INFINITY {
            trunk_px += 1;
            if t >= l {
                seen_trunk += 1;
            }
        }
    }
    let trunk_share = (trunk_px > 0).then(|| seen_trunk as f32 / trunk_px as f32);
    (hit as f32 / total as f32, gaps, largest, layers as f32 / drawn as f32, max, trunk_share)
}

/// Width of the trunk's section at height `y` (world), as the mean of its
/// X and Z extents; none when no trunk face crosses it.
///
/// Complexity: O(trunk faces).
fn section_width(part: &Part, trunk: &[bool], y: f32) -> Option<f32> {
    let mesh = part.mesh;
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for f in mesh.faces().filter(|&f| trunk[f as usize]) {
        for h in mesh.face_halfedges(f) {
            let a = part.world.transform_point3(mesh.positions[mesh.origin(h) as usize]);
            let b = part.world.transform_point3(mesh.positions[mesh.dest(h) as usize]);
            if (a.y - y) * (b.y - y) <= 0.0 && (a.y - b.y).abs() > 1e-9 {
                let t = (y - a.y) / (b.y - a.y);
                let q = a.lerp(b, t);
                lo = lo.min(Vec2::new(q.x, q.z));
                hi = hi.max(Vec2::new(q.x, q.z));
            }
        }
    }
    lo.is_finite().then_some(((hi.x - lo.x) + (hi.y - lo.y)) * 0.5)
}

/// Measures one part, or none when it has no foliage or trunk.
///
/// Complexity: O(F + C) plus two views of O(cells x k).
pub fn measure(view: &View, index: usize) -> Option<Measured> {
    let part = view.parts.get(index)?;
    let mesh = part.mesh;
    let leaves = member(mesh, &["foliage"]);
    let trunk = member(mesh, &["trunk", "roots"]);
    let has_leaves = leaves.iter().any(|x| *x);
    if !has_leaves && !trunk.iter().any(|x| *x) {
        return None;
    }
    let landmark =
        |n: &str| mesh.group(&format!("lm.{n}")).and_then(|g| g.iter().next()).map(|v| part.world.transform_point3(mesh.positions[v as usize]));
    let base = landmark("base").map_or(part.lo.y, |p| p.y);
    let mut m = Measured { part: part.label.clone(), height: part.hi.y - base, ..Measured::default() };
    if let Some(meta) = view.doc.object(part.key).and_then(|o| o.metadata.get("vegetation")) {
        m.species = meta.get("species").and_then(|v| v.as_str()).map(str::to_owned);
        m.foliage = meta.get("foliage").and_then(|v| v.as_str()).map(str::to_owned);
        m.coverage_target = meta.get("coverage_target").and_then(serde_json::Value::as_f64).map(|x| x as f32);
        m.repeats = meta.get("repeats").and_then(serde_json::Value::as_u64);
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut foliage_tris = 0usize;
    for f in mesh.faces() {
        let tris = mesh.face_len(f).saturating_sub(2);
        if leaves[f as usize] {
            foliage_tris += tris;
            for v in mesh.face_verts(f) {
                let p = part.world.transform_point3(mesh.positions[v as usize]);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        } else {
            m.wood_tris += tris;
        }
    }
    m.foliage_tris = foliage_tris;
    if has_leaves {
        m.crown_width = hi.x - lo.x;
        m.crown_depth = hi.z - lo.z;
        m.crown_height = hi.y - lo.y;
        m.crown_base = lo.y - base;
    }
    m.fork = landmark("fork").map(|p| p.y - base);
    m.trunk_diameter = section_width(part, &trunk, base + BREAST_HEIGHT.min(m.height * 0.11));
    m.root_spread = match (section_width(part, &trunk, base + m.height * 0.004), m.trunk_diameter) {
        (Some(g), Some(d)) if d > 0.0 => Some(g / d),
        _ => None,
    };
    m.limbs = pieces(mesh, &member(mesh, &["level_1"])).len();
    let cards = pieces(mesh, &leaves);
    m.cards = cards.len();
    m.leaf_cards = pieces(mesh, &member(mesh, &["foliage.leaf_cards"])).len();
    m.wind = mesh.corner_color.is_some() && mesh.uv_layer_count() >= 2;
    if has_leaves {
        let mut areas = cards.clone();
        areas.sort_by(f32::total_cmp);
        let card = areas.get(areas.len() / 2).map_or(0.1, |a| a.sqrt()).max(1e-4);
        let front = view_measure(part, &leaves, &trunk, card, (0, 1, 2));
        let side = view_measure(part, &leaves, &trunk, card, (2, 1, 0));
        m.coverage = [front.0, side.0];
        m.gaps = front.1 + side.1;
        m.largest_gap = front.2;
        m.overdraw = [front.3, side.3];
        m.overdraw_max = front.4.max(side.4);
        m.trunk_visible = match (front.5, side.5) {
            (Some(a), Some(b)) => Some([a, b]),
            _ => None,
        };
    }
    Some(m)
}

fn pct(x: f32) -> String {
    format!("{}%", (x * 100.0).round())
}

fn m1(x: f32) -> String {
    format!("{:.1} m", x)
}

fn m2(x: f32) -> String {
    format!("{:.2} m", x)
}

/// Text lines for one part.
fn lines(m: &Measured) -> Vec<String> {
    let mut out = Vec::new();
    let kind = match (&m.species, &m.foliage) {
        (Some(s), Some(f)) => format!(": {s}, {f}"),
        (Some(s), None) => format!(": {s}"),
        _ => String::new(),
    };
    out.push(format!("{}{kind}", m.part));
    if m.crown_height > 0.0 {
        out.push(format!(
            "  height {}; crown {} x {}, {} tall from {} ({}); width / height {:.2}",
            m1(m.height),
            m1(m.crown_width),
            m1(m.crown_depth),
            m1(m.crown_height),
            m1(m.crown_base),
            pct(m.crown_base / m.height.max(1e-6)),
            m.crown_width.max(m.crown_depth) / m.height.max(1e-6)
        ));
    } else {
        out.push(format!("  height {}; no foliage", m1(m.height)));
    }
    let mut trunk = Vec::new();
    if let Some(f) = m.fork {
        trunk.push(format!("fork at {} ({})", m2(f), pct(f / m.height.max(1e-6))));
    }
    if let Some(d) = m.trunk_diameter {
        trunk.push(format!("diameter {} at breast height ({} of the height)", m2(d), pct(d / m.height.max(1e-6))));
    }
    if let Some(s) = m.root_spread.filter(|s| *s > 1.05) {
        trunk.push(format!("roots spread {s:.1}x"));
    }
    if m.limbs > 0 {
        trunk.push(format!("{} limbs", m.limbs));
    }
    if !trunk.is_empty() {
        out.push(format!("  trunk: {}", trunk.join(", ")));
    }
    let wood_share = m.wood_tris as f32 / (m.wood_tris + m.foliage_tris).max(1) as f32;
    let leaf = if m.leaf_cards > 0 { format!(" ({} single leaves)", thousands(m.leaf_cards)) } else { String::new() };
    out.push(format!(
        "  foliage: {} pieces{leaf}, {} tris; wood {} tris ({})",
        thousands(m.cards),
        thousands(m.foliage_tris),
        thousands(m.wood_tris),
        pct(wood_share)
    ));
    if m.foliage_tris > 0 {
        let target = m.coverage_target.map_or(String::new(), |t| format!(" (target {})", pct(t)));
        let gap = match m.largest_gap {
            Some((a, c)) => format!("; {} gaps of two cards or more, largest {:.2} m2 at ({:.1}, {:.1}) m front", m.gaps, a, c.x, c.y),
            None => "; no gaps of two cards or more".into(),
        };
        out.push(format!("  coverage front {}, side {}{target}{gap}", pct(m.coverage[0]), pct(m.coverage[1])));
        let rep = m.repeats.map_or(String::new(), |r| format!("; {r} neighbouring repeats"));
        out.push(format!("  overdraw front {:.1}, side {:.1} (max {}){rep}", m.overdraw[0], m.overdraw[1], m.overdraw_max));
    }
    if let Some([a, b]) = m.trunk_visible {
        out.push(format!("  trunk visible front {}, side {}", pct(a), pct(b)));
    }
    out.push(format!("  wind data: {}", if m.wind { "level (color a), phase and stiffness (uv1)" } else { "none" }));
    out
}

/// The `vegetation` aspect.
///
/// Complexity: as [`measure`] per part.
pub fn report(view: &View) -> Report {
    let all: Vec<Measured> = (0..view.parts.len()).filter_map(|i| measure(view, i)).collect();
    let mut text = String::new();
    if all.is_empty() {
        text.push_str("vegetation: no part has foliage or a trunk (groups `foliage`, `trunk`); grow one with gen.tree or a tree feature\n");
    } else {
        text.push_str(&format!("vegetation of {} part{}\n", all.len(), if all.len() == 1 { "" } else { "s" }));
        for m in all.iter().take(4) {
            for l in lines(m) {
                text.push_str(&l);
                text.push('\n');
            }
        }
        if all.len() > 4 {
            text.push_str(&format!("... {} more\n", all.len() - 4));
        }
    }
    Report::new("vegetation", text, &all)
}

/// A gene range: `[min, max]`.
pub type Range2 = [f32; 2];

/// The vegetation archetype's genes (ADR 0011): proportions of a tree,
/// measured back from its mesh. A preset word gives the reference ranges
/// of a species; written ranges override them.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VegetationGenes {
    /// `broadleaf`, `conifer`, `palm`, `birch` or `stylized`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Crown width over the tree's height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown_ratio: Option<Range2>,
    /// Height of the crown's lowest point over the tree's height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown_base: Option<Range2>,
    /// Height of the first fork over the tree's height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<Range2>,
    /// Trunk diameter at breast height over the tree's height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk: Option<Range2>,
    /// Scaffold limbs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limbs: Option<Range2>,
    /// Covered share of the crown silhouette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Range2>,
    /// Root flare width over the trunk diameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_spread: Option<Range2>,
}

/// The `vegetation` gene: a preset word or ranges.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum VegetationGene {
    Preset(String),
    Genes(VegetationGenes),
}

/// Reference ranges of a species (ADR 0009's broadleaf profile and its
/// siblings).
fn preset(word: &str) -> Option<VegetationGenes> {
    let g = |crown_ratio: Range2,
             crown_base: Range2,
             fork: Option<Range2>,
             trunk: Range2,
             limbs: Option<Range2>,
             coverage: Range2,
             roots: Option<Range2>| {
        VegetationGenes {
            preset: Some(word.to_owned()),
            crown_ratio: Some(crown_ratio),
            crown_base: Some(crown_base),
            fork,
            trunk: Some(trunk),
            limbs,
            coverage: Some(coverage),
            root_spread: roots,
        }
    };
    Some(match word {
        "broadleaf" => g([0.9, 1.15], [0.2, 0.4], Some([0.33, 0.42]), [0.06, 0.085], Some([3.0, 5.0]), [0.83, 0.93], Some([1.3, 1.9])),
        "conifer" => g([0.3, 0.5], [0.05, 0.2], None, [0.035, 0.07], None, [0.78, 0.95], None),
        "palm" => g([0.55, 1.1], [0.3, 0.8], None, [0.025, 0.06], None, [0.2, 0.8], None),
        "birch" => g([0.35, 0.55], [0.18, 0.35], None, [0.025, 0.045], None, [0.68, 0.88], None),
        "stylized" => g([0.8, 1.35], [0.12, 0.45], Some([0.3, 0.5]), [0.08, 0.16], Some([2.0, 4.0]), [0.82, 1.0], None),
        _ => return None,
    })
}

/// Targets of a gene: the preset's ranges with written ones on top.
///
/// Complexity: O(1).
pub fn gene_targets(gene: &VegetationGene) -> Result<VegetationGenes, (String, String)> {
    let bad = |w: &str| ("/vegetation".to_owned(), format!("`{w}` is not a vegetation preset; use broadleaf, conifer, palm, birch or stylized"));
    match gene {
        VegetationGene::Preset(w) => preset(w).ok_or_else(|| bad(w)),
        VegetationGene::Genes(g) => {
            let base = match &g.preset {
                Some(w) => preset(w).ok_or_else(|| bad(w))?,
                None => VegetationGenes::default(),
            };
            Ok(VegetationGenes {
                preset: g.preset.clone(),
                crown_ratio: g.crown_ratio.or(base.crown_ratio),
                crown_base: g.crown_base.or(base.crown_base),
                fork: g.fork.or(base.fork),
                trunk: g.trunk.or(base.trunk),
                limbs: g.limbs.or(base.limbs),
                coverage: g.coverage.or(base.coverage),
                root_spread: g.root_spread.or(base.root_spread),
            })
        }
    }
}

/// The vegetation genes measured on the view's first vegetation part, as
/// (verdict, line) pairs: `ok`, `LOW` or `HIGH` for genes with a target.
///
/// Complexity: as [`measure`].
pub fn gene_lines(view: &View, gene: &VegetationGene) -> Result<Vec<(&'static str, String)>, (String, String)> {
    let t = gene_targets(gene)?;
    let Some(m) = (0..view.parts.len()).find_map(|i| measure(view, i)) else {
        return Ok(vec![("", "vegetation: no part has foliage or a trunk".to_owned())]);
    };
    let h = m.height.max(1e-6);
    let mut out = Vec::new();
    let mut gene_line = |name: &str, v: Option<f32>, r: Option<Range2>, fmt: &dyn Fn(f32) -> String| {
        let (Some(v), Some(r)) = (v, r) else { return };
        let s = if v < r[0] {
            "LOW"
        } else if v > r[1] {
            "HIGH"
        } else {
            "ok"
        };
        out.push((s, format!("{name} {} (target {} to {}) {s}", fmt(v), fmt(r[0]), fmt(r[1]))));
    };
    let ratio = |x: f32| format!("{x:.2}");
    let count = |x: f32| format!("{}", x.round());
    let crown = m.crown_width.max(m.crown_depth);
    gene_line("crown width / height", (crown > 0.0).then_some(crown / h), t.crown_ratio, &ratio);
    gene_line("crown base / height", (m.crown_height > 0.0).then_some(m.crown_base / h), t.crown_base, &ratio);
    gene_line("fork / height", m.fork.map(|f| f / h), t.fork, &ratio);
    gene_line("trunk diameter / height", m.trunk_diameter.map(|d| d / h), t.trunk, &|x| format!("{x:.3}"));
    gene_line("limbs", Some(m.limbs as f32), t.limbs, &count);
    gene_line("coverage", (m.foliage_tris > 0).then_some(m.coverage_mean()), t.coverage, &ratio);
    gene_line("root spread", m.root_spread, t.root_spread, &ratio);
    Ok(out)
}

/// The `vegetation` expectation on measured values: crown width (the larger
/// of X and Z) and height within ranges (meters), coverage within a range
/// (share, front and side mean) and overdraw at most a budget.
///
/// Complexity: O(1).
pub fn expectation(
    m: &Measured,
    crown_width: Option<&p3d_scene::Range>,
    crown_height: Option<&p3d_scene::Range>,
    coverage: Option<&p3d_scene::Range>,
    max_overdraw: Option<f64>,
) -> (bool, String) {
    let width = m.crown_width.max(m.crown_depth);
    let mut bad = Vec::new();
    let span = |r: &p3d_scene::Range, k: f64, unit: &str| match (r.min, r.max) {
        (Some(a), Some(b)) => format!("{:.2}{unit} to {:.2}{unit}", a * k, b * k),
        (Some(a), None) => format!("at least {:.2}{unit}", a * k),
        (None, Some(b)) => format!("at most {:.2}{unit}", b * k),
        (None, None) => "any".into(),
    };
    if let Some(r) = crown_width.filter(|r| !r.contains(f64::from(width))) {
        bad.push(format!("crown width {width:.2} m, want {}", span(r, 1.0, " m")));
    }
    if let Some(r) = crown_height.filter(|r| !r.contains(f64::from(m.crown_height))) {
        bad.push(format!("crown height {:.2} m, want {}", m.crown_height, span(r, 1.0, " m")));
    }
    let cov = m.coverage_mean();
    if let Some(r) = coverage.filter(|r| !r.contains(f64::from(cov))) {
        bad.push(format!("coverage {}, want {}", pct(cov), span(r, 100.0, "%")));
    }
    let od = m.overdraw_mean();
    if let Some(b) = max_overdraw.filter(|b| f64::from(od) > *b) {
        bad.push(format!("overdraw {od:.1}, budget {b:.1}"));
    }
    let base = format!("crown {:.2} x {:.2} m, coverage {}, overdraw {:.1}, {} cards", width, m.crown_height, pct(cov), od, thousands(m.cards));
    if bad.is_empty() { (true, base) } else { (false, format!("{base}; {}", bad.join("; "))) }
}
