//! Braids: three plies interleaved along a hanging guide (a figure-eight
//! crossing, the classic three-strand plait), beads threaded on them and a
//! band or bead tie with a short tuft at the end. Grooming genes of ADR
//! 0011: a dwarf's braided beard is `braids: 2, beads: 2, tie: bead`.

use std::f32::consts::TAU;

use glam::Vec3;
use p3d_geo::bvh::TriBvh;
use p3d_mesh::{ElementKind, Mesh};

use crate::grow::{Growth, integrate, rng_for};
use crate::{Guide, StrandSpec, StrandsError, Style, Tie};

/// Most points along one braid after resampling.
const MAX_BRAID_POINTS: usize = 512;

/// Closed tubes under construction.
#[derive(Default)]
struct Tubes {
    pos: Vec<Vec3>,
    sizes: Vec<u32>,
    corners: Vec<u32>,
    groups: Vec<(&'static str, Vec<u32>)>,
}

impl Tubes {
    fn face(&mut self, group: &'static str, corners: &[u32]) {
        let f = self.sizes.len() as u32;
        match self.groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, list)) => list.push(f),
            None => self.groups.push((group, vec![f])),
        }
        self.sizes.push(corners.len() as u32);
        self.corners.extend_from_slice(corners);
    }

    /// A closed tube through `pts` with one ring per point: an elliptic
    /// section (`radii` across `across`, times `flat` along the other
    /// axis), a flat cap at the start and a point or a flat cap at the end.
    ///
    /// Complexity: O(points x sides).
    #[allow(clippy::too_many_arguments)]
    fn tube(&mut self, group: &'static str, pts: &[Vec3], across: &[Vec3], radii: &[f32], flat: f32, sides: u32, pointed: bool) {
        let m = pts.len();
        if m < 2 {
            return;
        }
        let rings = if pointed { m - 1 } else { m };
        let base = self.pos.len() as u32;
        for i in 0..rings {
            let t = (pts[(i + 1).min(m - 1)] - pts[i.saturating_sub(1)]).normalize_or(Vec3::Y);
            let a = (across[i] - t * across[i].dot(t)).normalize_or(t.any_orthonormal_vector());
            let b = t.cross(a);
            for k in 0..sides {
                let th = k as f32 / sides as f32 * TAU;
                self.pos.push(pts[i] + a * (th.cos() * radii[i]) + b * (th.sin() * radii[i] * flat));
            }
        }
        for i in 0..rings as u32 - 1 {
            for k in 0..sides {
                let (p, q) = (base + i * sides + k, base + i * sides + (k + 1) % sides);
                self.face(group, &[p, q, q + sides, p + sides]);
            }
        }
        let last = base + (rings as u32 - 1) * sides;
        if pointed {
            let apex = self.pos.len() as u32;
            self.pos.push(pts[m - 1]);
            for k in 0..sides {
                self.face(group, &[last + k, last + (k + 1) % sides, apex]);
            }
        } else {
            let cap: Vec<u32> = (0..sides).map(|k| last + k).collect();
            self.face(group, &cap);
        }
        let start: Vec<u32> = (0..sides).rev().map(|k| base + k).collect();
        self.face(group, &start);
    }

    fn finish(self) -> Result<Mesh, StrandsError> {
        let mut m = Mesh::from_polygons(self.pos, &self.sizes, &self.corners).map_err(|e| StrandsError::new("braids", e.to_string()))?;
        for (name, faces) in self.groups {
            m.set_group(name, ElementKind::Face, faces).map_err(|e| StrandsError::new("braids", e.to_string()))?;
        }
        Ok(m)
    }
}

/// A braid's centerline resampled evenly, with an outward direction per
/// point.
struct Line {
    pts: Vec<Vec3>,
    out: Vec<Vec3>,
    len: f32,
}

/// Resamples a guide to points about `step` apart (at most
/// [`MAX_BRAID_POINTS`]).
///
/// Complexity: O(guide points + output points).
fn resample(g: &Guide, step: f32) -> Line {
    let mut along = Vec::with_capacity(g.points.len());
    let mut acc = 0.0f32;
    for (i, p) in g.points.iter().enumerate() {
        if i > 0 {
            acc += p.distance(g.points[i - 1]);
        }
        along.push(acc);
    }
    let n = ((acc / step.max(1e-6)).ceil() as usize).clamp(4, MAX_BRAID_POINTS);
    let mut pts = Vec::with_capacity(n + 1);
    let mut out = Vec::with_capacity(n + 1);
    let mut j = 0usize;
    for i in 0..=n {
        let s = acc * i as f32 / n as f32;
        while j + 2 < along.len() && along[j + 1] < s {
            j += 1;
        }
        let span = (along[j + 1] - along[j]).max(1e-9);
        let t = ((s - along[j]) / span).clamp(0.0, 1.0);
        pts.push(g.points[j].lerp(g.points[j + 1], t));
        out.push(g.outward[j].lerp(g.outward[j + 1], t).normalize_or(g.outward[j]));
    }
    Line { pts, out, len: acc }
}

/// The roots braids start from: the roots nearest the given points, or
/// spread across the third of the roots lowest and most to the front (+Z),
/// such as the chin of a beard or the temples of a head of hair.
///
/// Complexity: O(roots log roots + count x roots).
fn pick_roots(guides: &[Guide], spec: &StrandSpec) -> Vec<usize> {
    let roots: Vec<usize> = (0..guides.len()).filter(|&i| guides[i].layer == 0 && !guides[i].flyaway).collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let b = &spec.braids;
    let count = if b.count > 0 { b.count as usize } else { b.at.len() };
    if !b.at.is_empty() {
        return (0..count)
            .filter_map(|i| {
                let p = b.at[i % b.at.len()];
                roots
                    .iter()
                    .copied()
                    .min_by(|&x, &y| guides[x].points[0].distance_squared(p).total_cmp(&guides[y].points[0].distance_squared(p)).then(x.cmp(&y)))
            })
            .collect();
    }
    let mut low = roots;
    let score = |i: usize| guides[i].points[0].y - 0.5 * guides[i].points[0].z;
    low.sort_by(|&x, &y| score(x).total_cmp(&score(y)).then(x.cmp(&y)));
    low.truncate(count.max(low.len().div_ceil(3)));
    low.sort_by(|&x, &y| guides[x].points[0].x.total_cmp(&guides[y].points[0].x).then(x.cmp(&y)));
    (0..count).map(|i| low[((i as f32 + 0.5) / count as f32 * low.len() as f32) as usize % low.len()]).collect()
}

/// Rings of a barrel (a bead or a band) around a braid's axis.
fn barrel(t: &mut Tubes, group: &'static str, line: &Line, at: f32, length: f32, radius: f32, sides: u32) {
    let s0 = (at - length * 0.5).max(0.0);
    let point = |s: f32| {
        let x = (s / line.len.max(1e-9) * (line.pts.len() - 1) as f32).clamp(0.0, (line.pts.len() - 1) as f32);
        let i = (x as usize).min(line.pts.len() - 2);
        (line.pts[i].lerp(line.pts[i + 1], x - i as f32), line.out[i])
    };
    let profile = [(0.0f32, 0.84f32), (0.15, 1.0), (0.85, 1.0), (1.0, 0.84)];
    let mut pts = Vec::with_capacity(4);
    let mut across = Vec::with_capacity(4);
    let mut radii = Vec::with_capacity(4);
    for (u, r) in profile {
        let (p, o) = point(s0 + u * length);
        pts.push(p);
        across.push(o);
        radii.push(radius * r);
    }
    t.tube(group, &pts, &across, &radii, 1.0, sides, false);
}

/// Grows the braids and builds them: plies, beads, ties and tufts. Returns
/// the mesh, the braid guides and the bead count.
///
/// Complexity: O(braids x (segments x log F + points x sides)).
pub(crate) fn braids(bvh: &TriBvh, center: Vec3, guides: &[Guide], width: f32, spec: &StrandSpec) -> Result<(Mesh, Vec<Guide>, usize), StrandsError> {
    let b = &spec.braids;
    let picks = pick_roots(guides, spec);
    if picks.is_empty() {
        return Ok((Mesh::default(), Vec::new(), 0));
    }
    let w = if b.width > 0.0 { b.width } else { width * 1.2 };
    let length = if b.length > 0.0 { b.length } else { spec.length * 1.6 };
    // Braids hang: little noise, no curl, full gravity.
    let hang = StrandSpec {
        length,
        length_variation: 0.0,
        gravity: spec.gravity.max(0.9),
        curl: 0.0,
        noise: spec.noise * 0.3,
        segments: ((length / (w * 0.8)).ceil() as u32).clamp(spec.segments, 64),
        style: match spec.style {
            Style::Spiky | Style::Upright | Style::WindBlown(_) => Style::Straight,
            s => s,
        },
        ..spec.clone()
    };
    let sides = spec.sides.max(6);
    let period = w * 1.15;
    let mut t = Tubes::default();
    let mut out_guides = Vec::with_capacity(picks.len());
    let mut beads = 0usize;
    for (bi, &gi) in picks.iter().enumerate() {
        let root = guides[gi].points[0];
        let n0 = guides[gi].outward[0];
        let mut rng = rng_for(spec.seed ^ 0xB7A1_D5EE, 7, bi);
        let lift = spec.lift + spec.volume * 0.5;
        let (points, outward) = integrate(bvh, root - n0 * spec.lift, n0, center, &hang, &mut rng, &Growth { lift, noise: hang.noise, length: 1.0 });
        let g = Guide { points, outward, width: 1.0, layer: 0, slot: 0, flyaway: false };
        let line = resample(&g, period / 12.0);
        let m = line.pts.len();
        let tie_len = match b.tie {
            Tie::None => 0.0,
            Tie::Band => w * 0.45,
            Tie::Bead => w * 0.8,
        };
        let tuft = if b.tie == Tie::None { 0.0 } else { w * 1.1 };
        let plait_end = (line.len - tuft - tie_len * 0.5).max(line.len * 0.5);
        let end = ((plait_end / line.len.max(1e-9)) * (m - 1) as f32).ceil() as usize;
        let end = end.clamp(2, m - 1);
        // Three plies on a figure-eight path around the line.
        for k in 0..3 {
            let mut pts = Vec::with_capacity(end + 1);
            let mut across = Vec::with_capacity(end + 1);
            let mut radii = Vec::with_capacity(end + 1);
            for i in 0..=end {
                let tan = (line.pts[(i + 1).min(m - 1)] - line.pts[i.saturating_sub(1)]).normalize_or(Vec3::NEG_Y);
                let out = (line.out[i] - tan * line.out[i].dot(tan)).normalize_or(tan.any_orthonormal_vector());
                let side = tan.cross(out);
                let s = line.len * i as f32 / (m - 1) as f32;
                let phi = TAU * s / period + TAU * k as f32 / 3.0;
                // Narrower toward the end; the root starts gathered.
                let taper = 1.0 - 0.25 * (s / line.len.max(1e-9));
                let gather = (s / (w * 0.6)).min(1.0);
                pts.push(line.pts[i] + side * (w * 0.3 * phi.sin() * taper * gather) + out * (w * 0.12 * (2.0 * phi).sin() * taper * gather));
                across.push(side);
                radii.push(w * 0.27 * taper);
            }
            t.tube("braids", &pts, &across, &radii, 0.8, sides, false);
        }
        let s_tie = plait_end;
        match b.tie {
            Tie::None => {}
            Tie::Band => barrel(&mut t, "ties", &line, s_tie, tie_len, w * 0.4, sides),
            Tie::Bead => barrel(&mut t, "ties", &line, s_tie, tie_len, w * 0.55, sides.max(8)),
        }
        if tuft > 0.0 {
            // A short tuft fanning out of the tie.
            let a = ((s_tie / line.len.max(1e-9)) * (m - 1) as f32).round() as usize;
            let a = a.min(m - 2);
            let tan = (line.pts[a + 1] - line.pts[a]).normalize_or(Vec3::NEG_Y);
            let out = (line.out[a] - tan * line.out[a].dot(tan)).normalize_or(tan.any_orthonormal_vector());
            let side = tan.cross(out);
            for k in 0..4 {
                let th = TAU * (k as f32 + 0.5) / 4.0;
                let radial = side * th.cos() + out * th.sin();
                let start = line.pts[a] + radial * w * 0.12;
                let dir = (tan + radial * 0.35).normalize_or(tan);
                let l = tuft * (0.85 + 0.3 * rng.unit());
                let pts: Vec<Vec3> =
                    (0..5).map(|j| start + dir * (l * j as f32 / 4.0) + Vec3::NEG_Y * (l * 0.1 * (j as f32 / 4.0).powi(2))).collect();
                let radii: Vec<f32> = (0..5).map(|j| w * 0.2 * (1.0 - 0.2 * j as f32)).collect();
                t.tube("ties", &pts, &[side; 5], &radii, 0.7, sides.min(8), true);
            }
        }
        // Beads spread between a fifth and four fifths of the plait.
        for j in 0..b.beads {
            let at = plait_end * (0.2 + 0.6 * (j as f32 + 0.5) / b.beads as f32);
            let size = if b.bead_size > 0.0 { b.bead_size } else { w * 1.25 };
            barrel(&mut t, "beads", &line, at, size * 0.8, size * 0.5, sides.max(8));
            beads += 1;
        }
        out_guides.push(g);
    }
    Ok((t.finish()?, out_guides, beads))
}
