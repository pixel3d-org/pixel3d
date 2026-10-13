//! Output meshes (cards, clumps, shells) and the coverage check.

use std::f32::consts::TAU;

use glam::{IVec3, Vec2, Vec3};
use p3d_mesh::{ElementKind, Mesh};
use rustc_hash::FxHashMap;

use crate::{Guide, StrandSpec, StrandsError};

/// Polygons under construction, with face groups in creation order.
#[derive(Default)]
struct Builder {
    pos: Vec<Vec3>,
    sizes: Vec<u32>,
    corners: Vec<u32>,
    uvs: Vec<Vec2>,
    groups: Vec<(String, Vec<u32>)>,
}

impl Builder {
    fn face(&mut self, group: &str, corners: &[u32]) {
        let f = self.sizes.len() as u32;
        match self.groups.last_mut() {
            Some((g, list)) if g == group => list.push(f),
            _ => match self.groups.iter_mut().find(|(g, _)| g == group) {
                Some((_, list)) => list.push(f),
                None => self.groups.push((group.to_owned(), vec![f])),
            },
        }
        self.sizes.push(corners.len() as u32);
        self.corners.extend_from_slice(corners);
    }

    fn finish(self, with_uvs: bool) -> Result<Mesh, StrandsError> {
        let mut m = Mesh::from_polygons(self.pos, &self.sizes, &self.corners).map_err(|e| StrandsError::new("output", e.to_string()))?;
        for (name, faces) in self.groups {
            m.set_group(&name, ElementKind::Face, faces).map_err(|e| StrandsError::new("output", e.to_string()))?;
        }
        if with_uvs && m.add_uv_layer_from_polygon_order(&self.uvs).is_none() {
            return Err(StrandsError::new("output", "card UVs do not match the faces"));
        }
        Ok(m)
    }
}

/// The region's faces as their own patch: positions, area-weighted vertex
/// normals, sizes and corners.
///
/// Complexity: O(corners of the region).
#[allow(clippy::type_complexity)]
fn patch(parent: &Mesh, faces: &[u32]) -> (Vec<Vec3>, Vec<Vec3>, Vec<u32>, Vec<u32>) {
    let mut map = vec![u32::MAX; parent.vert_slots()];
    let (mut pos, mut normals, mut sizes, mut corners) = (Vec::new(), Vec::new(), Vec::with_capacity(faces.len()), Vec::new());
    for &f in faces {
        let n2 = parent.face_normal_area2(f);
        let start = corners.len();
        for v in parent.face_verts(f) {
            if map[v as usize] == u32::MAX {
                map[v as usize] = pos.len() as u32;
                pos.push(parent.positions[v as usize]);
                normals.push(Vec3::ZERO);
            }
            let id = map[v as usize];
            normals[id as usize] += n2;
            corners.push(id);
        }
        sizes.push((corners.len() - start) as u32);
    }
    for n in &mut normals {
        *n = n.normalize_or_zero();
    }
    (pos, normals, sizes, corners)
}

/// Appends the region patch offset along its normals.
fn add_patch(b: &mut Builder, patch: &(Vec<Vec3>, Vec<Vec3>, Vec<u32>, Vec<u32>), offset: f32, group: &str, uv: Option<Vec2>) {
    let (pos, normals, sizes, corners) = patch;
    let base = b.pos.len() as u32;
    b.pos.extend(pos.iter().zip(normals).map(|(p, n)| *p + *n * offset));
    let mut at = 0usize;
    let mut face = Vec::new();
    for &s in sizes {
        face.clear();
        face.extend(corners[at..at + s as usize].iter().map(|c| c + base));
        at += s as usize;
        b.face(group, &face);
        if let Some(uv) = uv {
            b.uvs.extend(std::iter::repeat_n(uv, s as usize));
        }
    }
}

/// Unit width directions along a guide: across the strand and along the
/// parent's surface (tangent x outward).
///
/// Complexity: O(points).
fn across(g: &Guide) -> Vec<Vec3> {
    let m = g.points.len();
    let mut out: Vec<Vec3> = Vec::with_capacity(m);
    for i in 0..m {
        let t = (g.points[(i + 1).min(m - 1)] - g.points[i.saturating_sub(1)]).normalize_or(Vec3::Y);
        let s = t.cross(g.outward[i]).try_normalize().unwrap_or_else(|| out.last().copied().unwrap_or_else(|| t.any_orthonormal_vector()));
        // Exactly across the strand.
        out.push((s - t * s.dot(t)).normalize_or(t.any_orthonormal_vector()));
    }
    out
}

fn tangent(g: &Guide, i: usize) -> Vec3 {
    let m = g.points.len();
    (g.points[(i + 1).min(m - 1)] - g.points[i.saturating_sub(1)]).normalize_or(Vec3::Y)
}

/// Width of a guide at point `i`.
fn width_at(g: &Guide, i: usize, width: f32, spec: &StrandSpec) -> f32 {
    let t = i as f32 / (g.points.len() - 1).max(1) as f32;
    width * g.width * (1.0 + (spec.tip_width - 1.0) * t)
}

/// Cards: an optional scalp cap, then a tapered strip per guide, inner
/// layers first and flyaways last (alpha sorting order). Groups `cap`,
/// `cards.<layer>` and `flyaways`; UVs into 4 atlas columns, v from root
/// 0 to tip 1.
///
/// Complexity: O(region corners + guide points).
pub(crate) fn cards(parent: &Mesh, faces: &[u32], guides: &[Guide], width: f32, spec: &StrandSpec) -> Result<Mesh, StrandsError> {
    let mut b = Builder::default();
    if spec.cap {
        add_patch(&mut b, &patch(parent, faces), spec.lift * 0.5, "cap", Some(Vec2::new(0.125, 0.02)));
    }
    for g in guides {
        let s = across(g);
        let m = g.points.len();
        let base = b.pos.len() as u32;
        for (i, (p, s)) in g.points.iter().zip(&s).enumerate() {
            let half = *s * width_at(g, i, width, spec) * 0.5;
            b.pos.push(*p - half);
            b.pos.push(*p + half);
        }
        let group = if g.flyaway { "flyaways".to_owned() } else { format!("cards.{}", g.layer) };
        let (u0, u1) = (g.slot as f32 / 4.0, (g.slot + 1) as f32 / 4.0);
        for i in 0..m - 1 {
            let k = base + 2 * i as u32;
            // Left, right, right ahead, left ahead: faces outward.
            b.face(&group, &[k, k + 1, k + 3, k + 2]);
            let (v0, v1) = (i as f32 / (m - 1) as f32, (i + 1) as f32 / (m - 1) as f32);
            b.uvs.extend([Vec2::new(u0, v0), Vec2::new(u1, v0), Vec2::new(u1, v1), Vec2::new(u0, v1)]);
        }
    }
    b.finish(true)
}

/// Clumps: a closed tube per guide with a lens section (flattened across
/// the surface), sunk slightly at the root and pointed at the tip. Group
/// `clumps`.
///
/// Complexity: O(guide points x sides).
pub(crate) fn clumps(guides: &[Guide], width: f32, spec: &StrandSpec) -> Result<Mesh, StrandsError> {
    let mut b = Builder::default();
    let n = spec.sides;
    for g in guides {
        let s = across(g);
        let m = g.points.len();
        let rings = (m - 1) as u32;
        let base = b.pos.len() as u32;
        for (i, &si) in s.iter().enumerate().take(rings as usize) {
            let t = tangent(g, i);
            let bi = t.cross(si);
            let r = width_at(g, i, width, spec) * 0.5;
            let center = if i == 0 { g.points[0] - g.outward[0] * r * 0.5 } else { g.points[i] };
            for k in 0..n {
                let a = k as f32 / n as f32 * TAU;
                b.pos.push(center + si * (a.cos() * r) + bi * (a.sin() * r * 0.45));
            }
        }
        let apex = b.pos.len() as u32;
        b.pos.push(g.points[m - 1]);
        for i in 0..rings - 1 {
            for k in 0..n {
                let (a, c) = (base + i * n + k, base + i * n + (k + 1) % n);
                b.face("clumps", &[a, c, c + n, a + n]);
            }
        }
        let last = base + (rings - 1) * n;
        for k in 0..n {
            b.face("clumps", &[last + k, last + (k + 1) % n, apex]);
        }
        let cap: Vec<u32> = (0..n).rev().map(|k| base + k).collect();
        b.face("clumps", &cap);
    }
    b.finish(false)
}

/// Shells: the region copied `shells` times up to the strand length.
/// Groups `shell_<i>`, innermost first.
///
/// Complexity: O(region corners x shells).
pub(crate) fn shells(parent: &Mesh, faces: &[u32], spec: &StrandSpec) -> Result<Mesh, StrandsError> {
    let mut b = Builder::default();
    let p = patch(parent, faces);
    for j in 0..spec.shells {
        let off = spec.lift + spec.length * (j + 1) as f32 / spec.shells as f32;
        add_patch(&mut b, &p, off, &format!("shell_{j}"), None);
    }
    b.finish(false)
}

/// Total card area.
///
/// Complexity: O(guide points).
pub(crate) fn card_area(guides: &[Guide], width: f32, spec: &StrandSpec) -> f32 {
    guides
        .iter()
        .map(|g| {
            (0..g.points.len() - 1)
                .map(|i| (width_at(g, i, width, spec) + width_at(g, i + 1, width, spec)) * 0.5 * g.points[i].distance(g.points[i + 1]))
                .sum::<f32>()
        })
        .sum()
}

/// Share of the region within reach of a card or clump, and the center and
/// area of the largest uncovered patch. Reach is half the width plus the
/// distance off the surface and half a step, on a hash grid of guide
/// points (no pairwise loops). The region is sampled evenly (about four
/// samples per reach disk, at most [`p3d_geo::generate::MAX_SCATTER`]), so
/// large faces count by how much of them is covered; faces too small to
/// get a sample count by their centroid.
///
/// Complexity: O(F + samples + guide points) expected.
pub(crate) fn coverage(parent: &Mesh, faces: &[u32], guides: &[Guide], width: f32, spec: &StrandSpec) -> (f32, Option<(Vec3, f32)>) {
    let mut pts: Vec<(Vec3, f32)> = Vec::new();
    for g in guides.iter().filter(|g| !g.flyaway) {
        let m = g.points.len();
        for i in 0..m {
            let step = g.points[i].distance(g.points[(i + 1).min(m - 1)]);
            let off = spec.lift + spec.volume * i as f32 / (m - 1) as f32;
            pts.push((g.points[i], width_at(g, i, width, spec) * 0.5 + off + step * 0.5));
        }
    }
    let cell = pts.iter().map(|p| p.1).fold(1e-6f32, f32::max);
    let key = |p: Vec3| (p / cell).floor().as_ivec3();
    let mut grid: FxHashMap<IVec3, Vec<u32>> = FxHashMap::default();
    for (i, (p, _)) in pts.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i as u32);
    }
    let reached = |c: Vec3| {
        let k = key(c);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(k + IVec3::new(dx, dy, dz)))
                        && list.iter().any(|&i| pts[i as usize].0.distance(c) <= pts[i as usize].1)
                    {
                        return true;
                    }
                }
            }
        }
        false
    };
    let total: f32 = faces.iter().map(|&f| parent.face_area(f)).sum();
    let mean_reach = if pts.is_empty() { cell } else { pts.iter().map(|p| p.1).sum::<f32>() / pts.len() as f32 };
    let want = (4.0 * total / (std::f32::consts::PI * mean_reach * mean_reach)).ceil();
    let count = (want.min(p3d_geo::generate::MAX_SCATTER as f32) as u32).max(64);
    let samples = p3d_geo::generate::scatter(parent, faces, &p3d_geo::generate::ScatterSpec { count, min_distance: 0.0, seed: spec.seed ^ 0xC0FE })
        .unwrap_or_default();
    let mut seen_n = vec![0u32; parent.face_slots()];
    let mut hit_n = vec![0u32; parent.face_slots()];
    for s in &samples {
        seen_n[s.face as usize] += 1;
        if reached(s.position) {
            hit_n[s.face as usize] += 1;
        }
    }
    // Covered share of each region face.
    let mut share = vec![-1.0f32; parent.face_slots()];
    let mut hit = 0.0f32;
    for &f in faces {
        let s = match seen_n[f as usize] {
            0 => f32::from(u8::from(reached(parent.face_centroid(f)))),
            n => hit_n[f as usize] as f32 / n as f32,
        };
        share[f as usize] = s;
        hit += parent.face_area(f) * s;
    }
    // The largest mostly uncovered patch, by flood fill across region faces.
    let open = |f: u32| share[f as usize] >= 0.0 && share[f as usize] < 0.5;
    let mut seen = vec![false; parent.face_slots()];
    let mut best: Option<(Vec3, f32)> = None;
    let mut stack = Vec::new();
    for &f in faces {
        if !open(f) || seen[f as usize] {
            continue;
        }
        let (mut area, mut sum) = (0.0f32, Vec3::ZERO);
        seen[f as usize] = true;
        stack.push(f);
        while let Some(g) = stack.pop() {
            let a = parent.face_area(g) * (1.0 - share[g as usize]);
            area += a;
            sum += parent.face_centroid(g) * a;
            for h in parent.face_halfedges(g) {
                if let Some(nf) = parent.face(parent.twin(h))
                    && open(nf)
                    && !seen[nf as usize]
                {
                    seen[nf as usize] = true;
                    stack.push(nf);
                }
            }
        }
        if area > 0.0 && best.is_none_or(|(_, b)| area > b) {
            best = Some((sum / area, area));
        }
    }
    (if total > 0.0 { hit / total } else { 0.0 }, best)
}
