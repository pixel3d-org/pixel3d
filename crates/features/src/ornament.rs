//! Ornament along a path (ADR 0011 surface layer, as geometry): a motif
//! repeated along a curve or around a band, raised off the surface and
//! sunk a little into it. Motifs: `stud` (domed rivets), `pyramid`
//! (square studs), `bead` (a beaded rim), `rune` (angular glyphs of raised
//! strokes), `rope` (twisted plies, continuous) and `scroll` (a wave with
//! curls, continuous filigree). Spacing varies with `variation`, so a
//! rhythm is never mechanically even unless asked.
//!
//! Every motif is closed geometry; repeated motifs are separate closed
//! pieces of one mesh, grouped `ornament` and by motif.

use std::f32::consts::{PI, TAU};

use glam::{Quat, Vec2, Vec3};
use p3d_geo::generate::Rng;
use p3d_mesh::{ElementKind, Mesh};

use crate::FeatureError;

/// Most motifs one ornament places.
pub const MAX_MOTIFS: usize = 20_000;
/// Most path points.
pub const MAX_PATH_POINTS: usize = 10_000;

/// What repeats along the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motif {
    Stud,
    Pyramid,
    Bead,
    Rune,
    Rope,
    Scroll,
}

impl Motif {
    pub const ALL: [Motif; 6] = [Motif::Stud, Motif::Pyramid, Motif::Bead, Motif::Rune, Motif::Rope, Motif::Scroll];

    pub fn as_str(self) -> &'static str {
        match self {
            Motif::Stud => "stud",
            Motif::Pyramid => "pyramid",
            Motif::Bead => "bead",
            Motif::Rune => "rune",
            Motif::Rope => "rope",
            Motif::Scroll => "scroll",
        }
    }

    fn group(self) -> &'static str {
        match self {
            Motif::Stud => "studs",
            Motif::Pyramid => "pyramids",
            Motif::Bead => "beads",
            Motif::Rune => "runes",
            Motif::Rope => "rope",
            Motif::Scroll => "scrolls",
        }
    }
}

/// Which way is out of the surface along the path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outward {
    /// A direction, made perpendicular to the path (a path along an edge).
    Along(Vec3),
    /// Away from an axis through a point (a band around a haft).
    FromAxis { point: Vec3, axis: Vec3 },
}

/// An ornament. Lengths in meters.
#[derive(Debug, Clone, PartialEq)]
pub struct OrnamentSpec {
    pub path: Vec<Vec3>,
    pub closed: bool,
    pub outward: Outward,
    pub motif: Motif,
    /// Motifs along the path; 0 fills it at `spacing`.
    pub count: u32,
    /// Distance between motif centers; 0 uses 1.5 times the size.
    pub spacing: f32,
    /// Motif width (rope and scroll: the band's width).
    pub size: f32,
    /// Height above the surface.
    pub relief: f32,
    /// Depth below the surface, so the ornament embeds.
    pub sink: f32,
    /// Random change of spacing, size and turn (0 even to 1 loose).
    pub variation: f32,
    /// Rope plies (2 to 5).
    pub plies: u32,
    pub seed: u64,
}

impl Default for OrnamentSpec {
    fn default() -> Self {
        OrnamentSpec {
            path: band(0.05, Vec3::ZERO, Vec3::Y, 48),
            closed: true,
            outward: Outward::FromAxis { point: Vec3::ZERO, axis: Vec3::Y },
            motif: Motif::Stud,
            count: 0,
            spacing: 0.0,
            size: 0.01,
            relief: 0.004,
            sink: 0.0015,
            variation: 0.0,
            plies: 3,
            seed: 0,
        }
    }
}

/// A circle of `n` points around `axis` through `center`.
///
/// Complexity: O(n).
pub fn band(radius: f32, center: Vec3, axis: Vec3, n: usize) -> Vec<Vec3> {
    let a = axis.normalize_or(Vec3::Y);
    let u = a.any_orthonormal_vector();
    let v = a.cross(u);
    (0..n.max(3)).map(|i| center + (u * (i as f32 / n as f32 * TAU).cos() + v * (i as f32 / n as f32 * TAU).sin()) * radius).collect()
}

fn err(message: impl Into<String>) -> FeatureError {
    FeatureError { path: "/params".into(), message: message.into(), suggestions: Vec::new() }
}

/// A path walked by length: points, cumulative lengths and outward
/// directions.
struct Walk<'a> {
    spec: &'a OrnamentSpec,
    pts: Vec<Vec3>,
    acc: Vec<f32>,
}

impl Walk<'_> {
    fn new(spec: &OrnamentSpec) -> Walk<'_> {
        let mut pts = spec.path.clone();
        if spec.closed {
            pts.push(spec.path[0]);
        }
        let mut acc = Vec::with_capacity(pts.len());
        let mut t = 0.0f32;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                t += p.distance(pts[i - 1]);
            }
            acc.push(t);
        }
        Walk { spec, pts, acc }
    }

    fn len(&self) -> f32 {
        self.acc.last().copied().unwrap_or(0.0)
    }

    /// Point, tangent, outward and across directions at length `s`.
    ///
    /// Complexity: O(log points).
    fn frame(&self, s: f32) -> (Vec3, Vec3, Vec3, Vec3) {
        let total = self.len().max(1e-9);
        let s = if self.spec.closed { s.rem_euclid(total) } else { s.clamp(0.0, total) };
        let i = self.acc.partition_point(|&a| a <= s).clamp(1, self.pts.len() - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let span = (self.acc[i] - self.acc[i - 1]).max(1e-9);
        let p = a.lerp(b, ((s - self.acc[i - 1]) / span).clamp(0.0, 1.0));
        let t = (b - a).normalize_or(Vec3::X);
        let out = match self.spec.outward {
            Outward::Along(d) => d,
            Outward::FromAxis { point, axis } => {
                let ax = axis.normalize_or(Vec3::Y);
                let r = p - point;
                r - ax * r.dot(ax)
            }
        };
        let n = (out - t * out.dot(t)).try_normalize().unwrap_or_else(|| t.any_orthonormal_vector());
        (p, t, n, n.cross(t))
    }
}

/// Polygons under construction.
#[derive(Default)]
struct Build {
    pos: Vec<Vec3>,
    sizes: Vec<u32>,
    corners: Vec<u32>,
}

impl Build {
    fn face(&mut self, c: &[u32]) {
        self.sizes.push(c.len() as u32);
        self.corners.extend_from_slice(c);
    }

    /// A closed solid of revolution in a local frame: rings `(radius,
    /// height)` from the bottom (a flat cap) to the top (a point when its
    /// radius is 0).
    fn revolve(&mut self, origin: Vec3, axes: [Vec3; 3], rings: &[(f32, f32)], sides: u32) {
        let [u, v, w] = axes;
        let base = self.pos.len() as u32;
        let apex = rings.last().is_some_and(|r| r.0 <= 0.0);
        let full = if apex { rings.len() - 1 } else { rings.len() };
        for &(r, h) in &rings[..full] {
            for k in 0..sides {
                let a = k as f32 / sides as f32 * TAU;
                self.pos.push(origin + (u * a.cos() + v * a.sin()) * r + w * h);
            }
        }
        for i in 0..full as u32 - 1 {
            for k in 0..sides {
                let (p, q) = (base + i * sides + k, base + i * sides + (k + 1) % sides);
                self.face(&[p, q, q + sides, p + sides]);
            }
        }
        let last = base + (full as u32 - 1) * sides;
        if apex {
            let top = self.pos.len() as u32;
            let h = rings.last().map_or(0.0, |r| r.1);
            self.pos.push(origin + w * h);
            for k in 0..sides {
                self.face(&[last + k, last + (k + 1) % sides, top]);
            }
        } else {
            let cap: Vec<u32> = (0..sides).map(|k| last + k).collect();
            self.face(&cap);
        }
        let bottom: Vec<u32> = (0..sides).rev().map(|k| base + k).collect();
        self.face(&bottom);
    }

    /// A box in a local frame: half extents along `u`, `v`, `w` around
    /// `center`.
    fn cuboid(&mut self, center: Vec3, axes: [Vec3; 3], half: Vec3) {
        let [u, v, w] = axes;
        let base = self.pos.len() as u32;
        for k in 0..8 {
            let s = |bit: u32| if k & bit == 0 { -1.0 } else { 1.0 };
            self.pos.push(center + u * (half.x * s(1)) + v * (half.y * s(2)) + w * (half.z * s(4)));
        }
        // Corner index bits: 1 = +u, 2 = +v, 4 = +w; outward windings.
        for f in [[0u32, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
            self.face(&f.map(|i| base + i));
        }
    }

    /// A closed tube along points with radii; capped unless `ring` (the
    /// path closes on itself).
    fn tube(&mut self, pts: &[Vec3], radii: &[f32], up: &[Vec3], sides: u32, ring: bool) {
        let m = pts.len();
        if m < 2 {
            return;
        }
        let base = self.pos.len() as u32;
        for i in 0..m {
            let (prev, next) = if ring { (pts[(i + m - 1) % m], pts[(i + 1) % m]) } else { (pts[i.saturating_sub(1)], pts[(i + 1).min(m - 1)]) };
            let t = (next - prev).normalize_or(Vec3::X);
            let a = (up[i] - t * up[i].dot(t)).try_normalize().unwrap_or_else(|| t.any_orthonormal_vector());
            let b = t.cross(a);
            for k in 0..sides {
                let th = k as f32 / sides as f32 * TAU;
                self.pos.push(pts[i] + (a * th.cos() + b * th.sin()) * radii[i]);
            }
        }
        let segs = if ring { m } else { m - 1 };
        for i in 0..segs as u32 {
            let j = (i + 1) % m as u32;
            for k in 0..sides {
                let (p, q) = (base + i * sides + k, base + i * sides + (k + 1) % sides);
                let (r, s) = (base + j * sides + (k + 1) % sides, base + j * sides + k);
                self.face(&[p, q, r, s]);
            }
        }
        if !ring {
            let start: Vec<u32> = (0..sides).rev().map(|k| base + k).collect();
            self.face(&start);
            let last = base + (m as u32 - 1) * sides;
            let end: Vec<u32> = (0..sides).map(|k| last + k).collect();
            self.face(&end);
        }
    }
}

/// Rune strokes in a unit cell (x across -0.3 to 0.3, y up -0.5 to 0.5):
/// angular glyphs of stems and branches, carved-rune style.
const RUNES: [&[[f32; 4]]; 10] = [
    &[[0.0, -0.5, 0.0, 0.5], [0.0, 0.5, 0.3, 0.25], [0.0, 0.15, 0.3, -0.1]],
    &[[0.0, -0.5, 0.0, 0.5], [0.0, 0.3, 0.28, -0.05]],
    &[[-0.22, -0.5, -0.22, 0.5], [0.22, -0.5, 0.22, 0.5], [-0.22, 0.2, 0.22, -0.15]],
    &[[0.0, -0.5, 0.0, 0.5], [-0.28, 0.12, 0.0, 0.5], [0.0, 0.5, 0.28, 0.12]],
    &[[-0.28, -0.5, 0.28, 0.5], [-0.28, 0.5, 0.28, -0.5]],
    &[[0.0, -0.5, 0.0, 0.5], [0.0, 0.0, -0.28, 0.42], [0.0, 0.0, 0.28, 0.42]],
    &[[0.25, 0.5, -0.25, 0.0], [-0.25, 0.0, 0.25, -0.5]],
    &[[-0.2, -0.5, -0.2, 0.5], [-0.2, 0.5, 0.25, 0.2], [0.25, 0.2, -0.2, -0.05]],
    &[[0.0, -0.5, 0.0, 0.5], [-0.28, -0.2, 0.28, 0.2]],
    &[[-0.25, -0.5, -0.25, 0.5], [-0.25, 0.5, 0.25, 0.0], [0.25, 0.0, -0.25, -0.5]],
];

/// Strokes with their ends pulled back where they would run into another
/// stroke, so the bars of a glyph touch without overlapping.
fn trimmed(strokes: &[[f32; 4]], gap: f32) -> Vec<[Vec2; 2]> {
    let seg = |s: &[f32; 4]| [Vec2::new(s[0], s[1]), Vec2::new(s[2], s[3])];
    let dist = |p: Vec2, [a, b]: [Vec2; 2]| {
        let d = b - a;
        let t = ((p - a).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
        p.distance(a + d * t)
    };
    let mut out: Vec<[Vec2; 2]> = strokes.iter().map(seg).collect();
    // Later strokes give way to earlier ones (glyphs have at most 3).
    for (i, stroke) in out.iter_mut().enumerate() {
        for end in 0..2 {
            let [a, b] = *stroke;
            let p = if end == 0 { a } else { b };
            let near = strokes[..i].iter().any(|s| dist(p, seg(s)) < gap);
            if near {
                let dir = (if end == 0 { b - a } else { a - b }).normalize_or(Vec2::X);
                stroke[end] = p + dir * gap;
            }
        }
    }
    out
}

/// Builds an ornament.
///
/// Complexity: O(path points + motifs x motif faces), with a binary search
/// per motif along the path.
pub fn ornament(spec: &OrnamentSpec) -> Result<Mesh, FeatureError> {
    if spec.path.len() < 2 || spec.path.len() > MAX_PATH_POINTS {
        return Err(err(format!("a path takes 2 to {MAX_PATH_POINTS} points, not {}", spec.path.len())));
    }
    if spec.path.iter().any(|p| !p.is_finite()) || !(spec.size.is_finite() && spec.size > 0.0) {
        return Err(err("the path and size must be finite, the size positive"));
    }
    let (relief, sink) = (spec.relief.max(spec.size * 0.02), spec.sink.max(0.0));
    let walk = Walk::new(spec);
    let total = walk.len();
    if total <= spec.size * 0.1 {
        return Err(err("the path is shorter than a motif"));
    }
    let mut b = Build::default();
    let mut rng = Rng::new(spec.seed ^ 0x0A4E_3E47);
    let var = spec.variation.clamp(0.0, 1.0);
    match spec.motif {
        Motif::Rope => rope(&mut b, &walk, spec, sink),
        Motif::Scroll => scroll(&mut b, &walk, spec, relief, sink, &mut rng),
        motif => {
            let spacing = if spec.spacing > 0.0 { spec.spacing } else { spec.size * if motif == Motif::Rune { 1.15 } else { 1.5 } };
            let count = if spec.count > 0 { spec.count as usize } else { ((total / spacing).floor() as usize).max(1) };
            if count > MAX_MOTIFS {
                return Err(err(format!("{count} motifs would exceed {MAX_MOTIFS}; raise spacing or lower count")));
            }
            let step = total / count as f32;
            for i in 0..count {
                let jitter = (rng.unit() - 0.5) * 0.6 * var * step;
                let s = step * (i as f32 + 0.5) + jitter;
                let (p, t, n, across) = walk.frame(s);
                let k = 1.0 + (rng.unit() - 0.5) * 0.5 * var;
                let turn = Quat::from_axis_angle(n, (rng.unit() - 0.5) * 0.7 * var);
                let (t, across) = (turn * t, turn * across);
                let size = spec.size * k;
                match motif {
                    Motif::Stud => {
                        let r = size * 0.5;
                        let rings: Vec<(f32, f32)> = std::iter::once((r, -sink))
                            .chain((0..=4).map(|j| {
                                let a = j as f32 / 4.0 * PI * 0.5;
                                (r * a.cos(), relief * a.sin())
                            }))
                            .collect();
                        let mut rings = rings;
                        if let Some(last) = rings.last_mut() {
                            last.0 = 0.0;
                        }
                        b.revolve(p, [t, across, n], &rings, 12);
                    }
                    Motif::Pyramid => {
                        let r = size * 0.5 * std::f32::consts::SQRT_2;
                        // A square frustum on a skirt, rotated 45 degrees to face the path.
                        b.revolve(
                            p,
                            [(t + across).normalize_or(t), (across - t).normalize_or(across), n],
                            &[(r, -sink), (r, relief * 0.15), (r * 0.12, relief * 0.92), (0.0, relief)],
                            4,
                        );
                    }
                    Motif::Bead => {
                        let r = size * 0.5;
                        let c = p + n * (relief - r);
                        let rings: Vec<(f32, f32)> = (0..=6)
                            .map(|j| {
                                let a = -PI * 0.5 + j as f32 / 6.0 * PI;
                                (r * a.cos(), r * a.sin())
                            })
                            .collect();
                        // A sphere: bottom pole as a tiny flat cap, top as a point.
                        let mut rings = rings;
                        rings[0].0 = r * 0.05;
                        b.revolve(c, [t, across, n], &rings, 10);
                    }
                    Motif::Rune => {
                        let glyph = RUNES[(rng.next_u64() % RUNES.len() as u64) as usize];
                        let width = size * 0.13;
                        for [a, c] in trimmed(glyph, 0.07) {
                            let mid = (a + c) * 0.5 * size;
                            let d = c - a;
                            let len = d.length() * size;
                            if len <= width * 0.5 {
                                continue;
                            }
                            let dir = (t * d.x + across * d.y).normalize_or(t);
                            let side = n.cross(dir);
                            let center = p + t * mid.x + across * mid.y + n * ((relief - sink) * 0.5);
                            b.cuboid(center, [dir, side, n], Vec3::new(len * 0.5 + width * 0.5, width * 0.5, (relief + sink) * 0.5));
                        }
                    }
                    Motif::Rope | Motif::Scroll => {}
                }
            }
        }
    }
    if b.sizes.is_empty() {
        return Err(err("the ornament came out empty; check size and spacing"));
    }
    let mut m = Mesh::from_polygons(b.pos, &b.sizes, &b.corners).map_err(|e| err(e.to_string()))?;
    let all: Vec<u32> = m.faces().collect();
    m.set_group("ornament", ElementKind::Face, all.clone()).map_err(|e| err(e.to_string()))?;
    m.set_group(spec.motif.group(), ElementKind::Face, all).map_err(|e| err(e.to_string()))?;
    Ok(m)
}

/// Twisted plies along the whole path, laid on the surface.
fn rope(b: &mut Build, walk: &Walk<'_>, spec: &OrnamentSpec, sink: f32) {
    let total = walk.len();
    let plies = spec.plies.clamp(2, 5);
    let half = spec.size * 0.5;
    let s = (PI / plies as f32).sin();
    let r = half * s / (1.0 + s);
    let d = half - r;
    let n_pts = ((total / (spec.size * 0.25)).ceil() as usize).clamp(8, MAX_PATH_POINTS);
    // A whole number of turns on a closed band, so the plies meet.
    let mut turns = total / (spec.size * 2.4);
    if spec.closed {
        turns = turns.round().max(1.0);
    }
    // The rope sits on the surface, sunk by `sink`.
    let lift = half - sink;
    for k in 0..plies {
        let mut pts = Vec::with_capacity(n_pts);
        let mut ups = Vec::with_capacity(n_pts);
        let count = if spec.closed { n_pts } else { n_pts + 1 };
        for i in 0..count {
            let sa = total * i as f32 / n_pts as f32;
            let (p, _, n, across) = walk.frame(sa);
            let th = TAU * (turns * sa / total + k as f32 / plies as f32);
            pts.push(p + n * lift + (across * th.cos() + n * th.sin()) * d);
            ups.push(n);
        }
        let radii = vec![r; pts.len()];
        b.tube(&pts, &radii, &ups, 8, spec.closed);
    }
}

/// A wave along the path with a curl at every crest: running filigree.
fn scroll(b: &mut Build, walk: &Walk<'_>, spec: &OrnamentSpec, relief: f32, sink: f32, rng: &mut Rng) {
    let total = walk.len();
    let period = if spec.spacing > 0.0 { spec.spacing } else { spec.size * 1.8 };
    let waves = if spec.closed { (total / period).round().max(1.0) } else { (total / period).max(1.0) };
    let amp = spec.size * 0.3;
    let r = (spec.size * 0.07).max(relief * 0.5);
    let lift = r - sink;
    let n_pts = ((total / (period / 16.0)).ceil() as usize).clamp(8, MAX_PATH_POINTS);
    let count = if spec.closed { n_pts } else { n_pts + 1 };
    let mut pts = Vec::with_capacity(count);
    let mut ups = Vec::with_capacity(count);
    for i in 0..count {
        let s = total * i as f32 / n_pts as f32;
        let (p, _, n, across) = walk.frame(s);
        let ph = TAU * waves * s / total;
        pts.push(p + n * lift + across * (amp * ph.sin()));
        ups.push(n);
    }
    let radii = vec![r; pts.len()];
    b.tube(&pts, &radii, &ups, 6, spec.closed);
    // A curl inside each crest, turning toward the wave.
    let crests = (waves * 2.0).floor() as usize;
    for c in 0..crests {
        let s = total * (c as f32 + 0.25) / (waves * 2.0);
        if !spec.closed && s > total - period * 0.25 {
            break;
        }
        let (p, t, n, across) = walk.frame(s);
        let side = if c % 2 == 0 { 1.0 } else { -1.0 };
        let k = 1.0 + (rng.unit() - 0.5) * 0.3 * spec.variation;
        let center = p + n * lift + across * (side * amp * 0.25);
        let turns = 1.3;
        let m = 18;
        let spiral: Vec<Vec3> = (0..=m)
            .map(|j| {
                let u = j as f32 / m as f32;
                let a = side * TAU * turns * u;
                let rad = amp * 0.75 * k * (1.0 - 0.8 * u);
                center + (across * (side * a.cos()) + t * a.sin()) * rad
            })
            .collect();
        let radii: Vec<f32> = (0..=m).map(|j| r * (1.0 - 0.5 * j as f32 / m as f32)).collect();
        b.tube(&spiral, &radii, &vec![n; spiral.len()], 6, false);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn rune_strokes_are_trimmed_off_each_other() {
        for glyph in RUNES {
            let t = trimmed(glyph, 0.07);
            assert_eq!(t.len(), glyph.len());
            assert!(t.iter().all(|[a, b]| a.is_finite() && b.is_finite()));
        }
    }
}
