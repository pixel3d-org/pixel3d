//! Shells (project spec 17.4): a flat outline turned into a curved sheet,
//! for ears, leaves, petals, wings, fins, feathers and cloth flaps. The
//! outline (x across, y along its length, base at the lowest y) is filled
//! with an even quad-dominant sheet, then cupped across its width, bent
//! along its length and twisted toward the tip. With a thickness it becomes
//! a closed solid (front, back and rim); without, a one-sided card facing
//! +Z (alpha cards for foliage).

use glam::{DVec2, Vec3};
use p3d_mesh::{ElementKind, Mesh};
use rustc_hash::FxHashMap;

use super::invalid;
use super::shape::{fill_tris_with, orient};
use crate::edit::{QuadSpec, tris_to_quads};
use crate::prim::PrimError;

/// Most sheet points (outline plus interior).
pub const MAX_SHEET_POINTS: usize = 200_000;

/// A shell.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellSpec {
    /// Closed contours, even-odd (holes allowed), in meters.
    pub contours: Vec<Vec<DVec2>>,
    /// Target edge length of the sheet.
    pub spacing: f64,
    /// Total thickness; 0 makes a one-sided card.
    pub thickness: f64,
    /// Height of the edges across the width, as a share of the half width
    /// (positive curls toward +Z).
    pub cup: f64,
    /// Height of the tip, as a share of the length (positive toward +Z).
    pub bend: f64,
    /// Turn about the length axis at the tip, radians.
    pub twist: f64,
}

/// Contours with no edge longer than `spacing` (original points kept).
///
/// Complexity: O(output points).
fn resample(c: &[DVec2], spacing: f64) -> Vec<DVec2> {
    let mut out = Vec::with_capacity(c.len());
    for i in 0..c.len() {
        let (a, b) = (c[i], c[(i + 1) % c.len()]);
        let n = (a.distance(b) / spacing).ceil().max(1.0) as usize;
        for k in 0..n {
            out.push(a + (b - a) * (k as f64 / n as f64));
        }
    }
    out
}

/// Interior points on a staggered grid, kept away from the contours by
/// half a spacing: scanlines find the inside spans, an edge hash the
/// nearby edges.
///
/// Complexity: O(rows x edges + points).
fn interior(contours: &[Vec<DVec2>], spacing: f64) -> Result<Vec<DVec2>, PrimError> {
    let (mut lo, mut hi) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
    for p in contours.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let edges: Vec<(DVec2, DVec2)> = contours.iter().flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()]))).collect();
    // Edges by grid cell, for the clearance test.
    let cell = |p: DVec2| (((p.x - lo.x) / spacing).floor() as i64, ((p.y - lo.y) / spacing).floor() as i64);
    let mut hash: FxHashMap<(i64, i64), Vec<u32>> = FxHashMap::default();
    for (k, &(a, b)) in edges.iter().enumerate() {
        let (ca, cb) = (cell(a.min(b)), cell(a.max(b)));
        for x in ca.0..=cb.0 {
            for y in ca.1..=cb.1 {
                hash.entry((x, y)).or_default().push(k as u32);
            }
        }
    }
    let clear = |p: DVec2| {
        let (cx, cy) = cell(p);
        for x in cx - 1..=cx + 1 {
            for y in cy - 1..=cy + 1 {
                for &k in hash.get(&(x, y)).map_or(&[][..], Vec::as_slice) {
                    let (a, b) = edges[k as usize];
                    let ab = b - a;
                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
                    if (a + ab * t).distance(p) < spacing * 0.5 {
                        return false;
                    }
                }
            }
        }
        true
    };
    let row = spacing * 3f64.sqrt() / 2.0;
    let rows = ((hi.y - lo.y) / row) as usize + 1;
    let mut out = Vec::new();
    for r in 0..rows {
        let y = lo.y + row * (r as f64 + 0.5);
        // Inside spans of this row (even-odd).
        let mut xs: Vec<f64> =
            edges.iter().filter(|(a, b)| (a.y > y) != (b.y > y)).map(|(a, b)| a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x)).collect();
        xs.sort_by(f64::total_cmp);
        let shift = if r % 2 == 1 { spacing * 0.5 } else { 0.0 };
        let (pairs, _) = xs.as_chunks::<2>();
        for pair in pairs {
            let mut x = lo.x + shift + ((pair[0] - lo.x - shift) / spacing).ceil() * spacing;
            while x < pair[1] {
                let p = DVec2::new(x, y);
                if clear(p) {
                    out.push(p);
                    if out.len() > MAX_SHEET_POINTS {
                        return Err(invalid("spacing", spacing, format!("needs more than {MAX_SHEET_POINTS} points; use a larger spacing")));
                    }
                }
                x += spacing;
            }
        }
    }
    Ok(out)
}

/// Builds a shell.
///
/// Complexity: O(n log n) in sheet points (the triangulation), O(n) for
/// the deformation and thickness.
pub fn shell(spec: &ShellSpec) -> Result<Mesh, PrimError> {
    if !(spec.spacing.is_finite() && spec.spacing > 0.0) {
        return Err(invalid("spacing", spec.spacing, "must be positive"));
    }
    if !(spec.thickness.is_finite() && spec.thickness >= 0.0) {
        return Err(invalid("thickness", spec.thickness, "must be zero (a card) or positive"));
    }
    for (name, v) in [("cup", spec.cup), ("bend", spec.bend), ("twist", spec.twist)] {
        if !v.is_finite() || v.abs() > 10.0 {
            return Err(invalid(name, v, "must be between -10 and 10"));
        }
    }
    let (contours, _) = orient(&spec.contours)?;
    let rings: Vec<Vec<DVec2>> = contours.iter().map(|c| resample(c, spec.spacing)).collect();
    let extra = interior(&rings, spec.spacing)?;
    let tris = fill_tris_with(&rings, &extra)?;
    let flat: Vec<DVec2> = rings.iter().flatten().copied().chain(extra.iter().copied()).collect();
    let positions: Vec<Vec3> = flat.iter().map(|p| Vec3::new(p.x as f32, p.y as f32, 0.0)).collect();
    let corners: Vec<u32> = tris.iter().flatten().copied().collect();
    let mut sheet = Mesh::from_polygons(positions, &vec![3; tris.len()], &corners)?;
    let faces: Vec<u32> = sheet.faces().collect();
    let quads = QuadSpec { max_face_angle_deg: 1.0, max_shape_angle_deg: 40.0, ignore_boundaries: false };
    tris_to_quads(&mut sheet, &faces, &quads).map_err(|e| invalid("shape", "fill", e.to_string()))?;
    sheet.compact();
    // Deform: cup across x, bend along y, twist about the length axis.
    let (lo, hi) = sheet.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let cx = (lo.x + hi.x) * 0.5;
    let half = ((hi.x - lo.x) * 0.5).max(1e-9);
    let len = (hi.y - lo.y).max(1e-9);
    let (cup, bend, twist) = (spec.cup as f32, spec.bend as f32, spec.twist as f32);
    for v in sheet.verts().collect::<Vec<_>>() {
        let p = sheet.positions[v as usize];
        let u = (p.x - cx) / half;
        let t = (p.y - lo.y) / len;
        let z = cup * half * u * u + bend * len * t * t;
        let a = twist * t;
        let (s, c) = a.sin_cos();
        let x = p.x - cx;
        sheet.positions[v as usize] = Vec3::new(cx + x * c + z * s, p.y, -x * s + z * c);
    }
    let faces: Vec<u32> = sheet.faces().collect();
    if spec.thickness == 0.0 {
        sheet.set_group("front", ElementKind::Face, faces).map_err(|e| invalid("shape", "groups", e.to_string()))?;
        return Ok(sheet);
    }
    thicken(&sheet, spec.thickness as f32)
}

/// Quads across the rim: a half round in steps of 45 degrees, so smooth
/// shading never folds sharply where the front turns into the back.
const RIM_SEGMENTS: u32 = 4;

/// A closed solid from a sheet: front and back offset by half the
/// thickness along the vertex normals, joined by a half-round rim along the
/// boundary (bulging out by half the thickness).
///
/// Complexity: O(V + C).
fn thicken(sheet: &Mesh, thickness: f32) -> Result<Mesh, PrimError> {
    let n = sheet.vert_slots();
    let mut normal = vec![Vec3::ZERO; n];
    for f in sheet.faces() {
        let fn_ = sheet.face_normal_area2(f);
        for v in sheet.face_verts(f) {
            normal[v as usize] += fn_;
        }
    }
    let h = thickness * 0.5;
    let mut positions = Vec::with_capacity(2 * n);
    for side in [h, -h] {
        positions.extend(normal.iter().zip(&sheet.positions).map(|(nv, p)| *p + nv.normalize_or(Vec3::Z) * side));
    }
    let back = n as u32;
    let mut sizes = Vec::new();
    let mut corners = Vec::new();
    let mut front_faces = Vec::new();
    let mut back_faces = Vec::new();
    let mut rim_faces = Vec::new();
    for f in sheet.faces() {
        let vs: Vec<u32> = sheet.face_verts(f).collect();
        front_faces.push(sizes.len() as u32);
        sizes.push(vs.len() as u32);
        corners.extend(&vs);
        back_faces.push(sizes.len() as u32);
        sizes.push(vs.len() as u32);
        corners.extend(vs.iter().rev().map(|v| v + back));
    }
    // Boundary edges, and the direction away from the sheet at each of
    // their vertices (in the sheet's tangent plane).
    let mut rim = Vec::new();
    let mut outward = vec![Vec3::ZERO; n];
    let mut ring = vec![u32::MAX; n];
    let mut rim_verts = Vec::new();
    for hh in 0..sheet.he_slots() as u32 {
        if !sheet.is_he_alive(hh) || !sheet.is_boundary_he(hh) || sheet.face(hh).is_some() {
            continue;
        }
        // The boundary half-edge runs opposite to its face's edge a -> b,
        // so the face lies to the left of a -> b and outward is to the right.
        let (b, a) = (sheet.origin(hh), sheet.dest(hh));
        let d = sheet.positions[b as usize] - sheet.positions[a as usize];
        let o = d.cross((normal[a as usize] + normal[b as usize]).normalize_or(Vec3::Z));
        for v in [a, b] {
            outward[v as usize] += o;
            if ring[v as usize] == u32::MAX {
                ring[v as usize] = rim_verts.len() as u32;
                rim_verts.push(v);
            }
        }
        rim.push((b, a));
    }
    let middle = positions.len() as u32;
    for j in 1..RIM_SEGMENTS {
        let th = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * j as f32 / RIM_SEGMENTS as f32;
        positions.extend(rim_verts.iter().map(|&v| {
            let nv = normal[v as usize].normalize_or(Vec3::Z);
            let o = outward[v as usize] - nv * outward[v as usize].dot(nv);
            let o = o.normalize_or(nv.any_orthonormal_vector());
            sheet.positions[v as usize] + nv * (h * th.sin()) + o * (h * th.cos())
        }));
    }
    let count = rim_verts.len() as u32;
    let at = |j: u32, v: u32| match j {
        0 => v,
        RIM_SEGMENTS => v + back,
        _ => middle + (j - 1) * count + ring[v as usize],
    };
    for &(b, a) in &rim {
        for j in 0..RIM_SEGMENTS {
            rim_faces.push(sizes.len() as u32);
            sizes.push(4);
            corners.extend([at(j, b), at(j, a), at(j + 1, a), at(j + 1, b)]);
        }
    }
    let mut mesh = Mesh::from_polygons(positions, &sizes, &corners)?;
    mesh.compact();
    for (name, list) in [("front", front_faces), ("back", back_faces), ("rim", rim_faces)] {
        mesh.set_group(name, ElementKind::Face, list).map_err(|e| invalid("shape", "groups", e.to_string()))?;
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn leaf() -> Vec<DVec2> {
        // A pointed leaf 4 cm wide and 10 cm long, base at y = 0.
        (0..40)
            .map(|i| {
                let t = i as f64 / 40.0 * std::f64::consts::TAU;
                let y = 0.05 * (1.0 - t.cos());
                let x = 0.02 * t.sin() * (1.0 - y / 0.12);
                DVec2::new(x, y)
            })
            .collect()
    }

    #[test]
    fn a_card_is_an_even_quad_dominant_sheet() {
        let m = shell(&ShellSpec { contours: vec![leaf()], spacing: 0.004, thickness: 0.0, cup: 0.0, bend: 0.0, twist: 0.0 }).unwrap();
        assert_eq!(m.validate(), Ok(()));
        let quads = m.faces().filter(|&f| m.face_len(f) == 4).count();
        assert!(quads * 2 > m.face_count(), "{quads} of {}", m.face_count());
        assert!(m.verts().all(|v| m.positions[v as usize].z == 0.0));
        assert!(m.faces().all(|f| m.face_normal(f).z > 0.99), "faces +Z");
        assert!(m.group("front").is_some());
    }

    #[test]
    fn cup_bend_and_twist_curve_it_and_thickness_closes_it() {
        let m = shell(&ShellSpec { contours: vec![leaf()], spacing: 0.004, thickness: 0.002, cup: 0.4, bend: 0.3, twist: 0.3 }).unwrap();
        assert_eq!(m.validate(), Ok(()));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed");
        let (lo, hi) = m.bounds().unwrap();
        assert!(hi.z - lo.z > 0.02, "bent: {lo} {hi}");
        for g in ["front", "back", "rim"] {
            assert!(m.group(g).is_some_and(|g| g.count() > 0), "{g}");
        }
        // Volume is about area x thickness: outward faces.
        let mut vol = 0.0f64;
        for f in m.faces() {
            for t in m.face_triangles(f) {
                let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
                vol += a.dot(b.cross(c)) / 6.0;
            }
        }
        assert!(vol > 0.0, "{vol}");
    }

    #[test]
    fn the_rim_is_a_half_round_without_sharp_folds() {
        let disc: Vec<DVec2> = (0..48).map(|i| DVec2::from_angle(i as f64 / 48.0 * std::f64::consts::TAU) * 0.03).collect();
        let m = shell(&ShellSpec { contours: vec![disc], spacing: 0.004, thickness: 0.004, cup: 0.3, bend: 0.0, twist: 0.0 }).unwrap();
        assert_eq!(m.validate(), Ok(()));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed");
        let rim = m.group("rim").unwrap();
        assert_eq!(rim.count() as u32 % RIM_SEGMENTS, 0);
        let on_rim = |f: u32| rim.contains(f);
        let mut checked = 0;
        for h in 0..m.he_slots() as u32 {
            if !m.is_he_alive(h) {
                continue;
            }
            let (Some(f), Some(g)) = (m.face(h), m.face(m.twin(h))) else { continue };
            if on_rim(f) || on_rim(g) {
                let fold = m.face_normal(f).angle_between(m.face_normal(g)).to_degrees();
                assert!(fold < 50.0, "rim edge folds {fold} degrees");
                checked += 1;
            }
        }
        assert!(checked > 48 * 4, "{checked}");
        // The rim bulges out by about half the thickness.
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.x - lo.x - 0.064).abs() < 0.0015, "{lo} {hi}");
    }

    #[test]
    fn holes_and_bad_parameters() {
        let outer = vec![DVec2::ZERO, DVec2::new(0.1, 0.0), DVec2::new(0.1, 0.1), DVec2::new(0.0, 0.1)];
        let hole = vec![DVec2::new(0.03, 0.03), DVec2::new(0.07, 0.03), DVec2::new(0.07, 0.07), DVec2::new(0.03, 0.07)];
        let m = shell(&ShellSpec { contours: vec![outer.clone(), hole], spacing: 0.01, thickness: 0.0, cup: 0.0, bend: 0.0, twist: 0.0 }).unwrap();
        let area: f32 = m.faces().map(|f| m.face_area(f)).sum();
        assert!((area - 0.0084).abs() < 1e-5, "{area}");
        assert!(shell(&ShellSpec { contours: vec![outer.clone()], spacing: 0.0, thickness: 0.0, cup: 0.0, bend: 0.0, twist: 0.0 }).is_err());
        assert!(
            shell(&ShellSpec { contours: vec![outer], spacing: 1e-6, thickness: 0.0, cup: 0.0, bend: 0.0, twist: 0.0 }).is_err(),
            "too many points"
        );
    }
}
