//! Lofts: a surface through a sequence of cross sections.

use glam::{DVec2, DVec3, Vec3};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};

use super::shape::fill;
use super::{MAX_GEN_FACES, invalid, strip};
use crate::curve::{plane_normal, resample};
use crate::prim::PrimError;
use crate::prim::soup::{Soup, tag_groups_and_sharp};

/// Section corners that turn more than this many degrees get sharp rails.
pub const LOFT_CORNER_DEG: f64 = 30.0;
/// Most rows between two sections.
pub const MAX_LOFT_STEPS: u32 = 64;
/// Most sections.
pub const MAX_SECTIONS: usize = 4096;

const CAP_START: u16 = 0;
const CAP_END: u16 = 1;
const BAND0: u16 = 2;

/// Loft parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct LoftSpec<'a> {
    /// Cross sections in order (any sizes and point counts).
    pub sections: &'a [Vec<DVec3>],
    /// Whether every section is a closed loop.
    pub sections_closed: bool,
    /// Points per section after resampling by arc length; 0 keeps the
    /// sections as they are when their counts already match and otherwise
    /// uses the largest count.
    pub points: usize,
    /// Rows from one section to the next (1 joins them directly).
    pub steps: u32,
    /// Catmull-Rom curves through corresponding points instead of straight lines.
    pub smooth: bool,
    /// Joins the last section back to the first (a ring).
    pub closed: bool,
    /// Fills the first and last closed sections of an open loft.
    pub caps: bool,
}

fn centroid(pts: &[DVec3]) -> DVec3 {
    pts.iter().copied().sum::<DVec3>() / pts.len() as f64
}

/// Sections resampled to one count, wound the same way around the loft
/// direction and started at matching points so the surface does not twist.
///
/// Complexity: O(sections x points).
fn align(spec: &LoftSpec) -> Result<Vec<Vec<DVec3>>, PrimError> {
    let counts: Vec<usize> = spec.sections.iter().map(Vec::len).collect();
    let max = counts.iter().copied().max().unwrap_or(0);
    let same = counts.iter().all(|&c| c == max);
    let count = if spec.points > 0 { spec.points } else { max };
    let mut out: Vec<Vec<DVec3>> = Vec::with_capacity(spec.sections.len());
    for (i, s) in spec.sections.iter().enumerate() {
        if spec.points == 0 && same {
            out.push(s.clone());
        } else {
            let pts: Vec<Vec3> = s.iter().map(|p| p.as_vec3()).collect();
            let r = resample(&pts, spec.sections_closed, count).map_err(|e| invalid("sections", format!("section {i}"), e.to_string()))?;
            out.push(r.iter().map(|p| p.as_dvec3()).collect());
        }
    }
    let n = out.len();
    let cents: Vec<DVec3> = out.iter().map(|s| centroid(s)).collect();
    // Loft direction at each section.
    let dir = |i: usize| {
        let (a, b) = if i + 1 < n { (cents[i], cents[i + 1]) } else { (cents[i - 1], cents[i]) };
        (b - a).normalize_or_zero()
    };
    for i in 0..n {
        let d = dir(i);
        if spec.sections_closed {
            let pts: Vec<Vec3> = out[i].iter().map(|p| p.as_vec3()).collect();
            if let Some(nrm) = plane_normal(&pts, true)
                && nrm.as_dvec3().dot(d) < 0.0
            {
                out[i].reverse();
            }
        } else if i > 0 {
            let prev = out[i - 1][out[i - 1].len() - 1] - out[i - 1][0];
            let cur = out[i][out[i].len() - 1] - out[i][0];
            if prev.dot(cur) < 0.0 {
                out[i].reverse();
            }
        }
        if spec.sections_closed && i > 0 {
            // Start where the previous section starts, seen from each centroid.
            let want = (out[i - 1][0] - cents[i - 1]).normalize_or_zero();
            let k = (0..out[i].len())
                .max_by(|&a, &b| {
                    let da = (out[i][a] - cents[i]).normalize_or_zero().dot(want);
                    let db = (out[i][b] - cents[i]).normalize_or_zero().dot(want);
                    da.total_cmp(&db).then(b.cmp(&a))
                })
                .unwrap_or(0);
            out[i].rotate_left(k);
        }
    }
    Ok(out)
}

/// Catmull-Rom point between `p1` and `p2` at `t`.
fn catmull(p0: DVec3, p1: DVec3, p2: DVec3, p3: DVec3, t: f64) -> DVec3 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * p1) + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Lofts a surface through sections. Groups: `side`, `band_<i>` (from
/// section i to the next), `cap_start` and `cap_end` for filled ends. Cap
/// rims and rails where both rows turn sharper than [`LOFT_CORNER_DEG`]
/// are sharp.
///
/// Complexity: O(sections x points x steps) plus O(p log p) per cap.
pub fn loft(spec: &LoftSpec) -> Result<Mesh, PrimError> {
    let ns = spec.sections.len();
    let min_sections = if spec.closed { 3 } else { 2 };
    if ns < min_sections || ns > MAX_SECTIONS {
        return Err(invalid("sections", ns, format!("needs {min_sections} to {MAX_SECTIONS} sections")));
    }
    let min_points = if spec.sections_closed { 3 } else { 2 };
    for (i, s) in spec.sections.iter().enumerate() {
        if s.len() < min_points || s.iter().any(|p| !p.is_finite()) {
            return Err(invalid("sections", format!("section {i}"), format!("needs at least {min_points} finite points")));
        }
    }
    if spec.points != 0 && spec.points < min_points {
        return Err(invalid("points", spec.points, format!("needs at least {min_points}")));
    }
    if !(1..=MAX_LOFT_STEPS).contains(&spec.steps) {
        return Err(invalid("steps", spec.steps, format!("must be between 1 and {MAX_LOFT_STEPS}")));
    }
    let secs = align(spec)?;
    let np = secs[0].len();
    let bands = if spec.closed { ns } else { ns - 1 };
    let rows_n = bands * spec.steps as usize + usize::from(!spec.closed);
    if rows_n.saturating_mul(np) > MAX_GEN_FACES {
        return Err(invalid("points", np, format!("x {rows_n} rows would exceed {MAX_GEN_FACES} faces")));
    }

    // Rows: every section, plus interpolated rows between them.
    let at = |i: isize| -> &Vec<DVec3> {
        let i = if spec.closed { i.rem_euclid(ns as isize) } else { i.clamp(0, ns as isize - 1) };
        &secs[i as usize]
    };
    let mut rows: Vec<Vec<DVec3>> = Vec::with_capacity(rows_n);
    let mut row_band: Vec<u16> = Vec::with_capacity(rows_n);
    for b in 0..bands {
        for s in 0..spec.steps {
            let t = f64::from(s) / f64::from(spec.steps);
            let bi = b as isize;
            let (p1, p2) = (at(bi), at(bi + 1));
            let row = if s == 0 {
                p1.clone()
            } else if spec.smooth {
                let (p0, p3) = (at(bi - 1), at(bi + 2));
                (0..np).map(|k| catmull(p0[k], p1[k], p2[k], p3[k], t)).collect()
            } else {
                (0..np).map(|k| p1[k].lerp(p2[k], t)).collect()
            };
            rows.push(row);
            row_band.push(u16::try_from(b).unwrap_or(u16::MAX - BAND0));
        }
    }
    if !spec.closed {
        rows.push(secs[ns - 1].clone());
    }

    let mut soup = Soup::with_capacity(rows.len() * np, rows.len() * np + 2, 4 * rows.len() * np);
    let ids: Vec<Vec<u32>> = rows.iter().map(|r| r.iter().map(|p| soup.push_vert(*p)).collect()).collect();
    let strips = if spec.closed { rows.len() } else { rows.len() - 1 };
    for j in 0..strips {
        // Upper row first: faces point away from the loft axis for sections wound around it.
        strip(&mut soup, &ids[(j + 1) % rows.len()], &ids[j], spec.sections_closed, BAND0 + row_band[j]);
    }
    let caps = spec.caps && spec.sections_closed && !spec.closed;
    if caps {
        for (sec, tag) in [(&secs[0], CAP_START), (&secs[ns - 1], CAP_END)] {
            let ring = if tag == CAP_START { &ids[0] } else { &ids[ids.len() - 1] };
            let c = centroid(sec);
            let pts: Vec<Vec3> = sec.iter().map(|p| p.as_vec3()).collect();
            let n = plane_normal(&pts, true).map_or(DVec3::Y, |n| n.as_dvec3());
            let u = n.any_orthonormal_vector();
            let v = n.cross(u);
            let flat: Vec<DVec2> = sec.iter().map(|p| DVec2::new((*p - c).dot(u), (*p - c).dot(v))).collect();
            for poly in fill(std::slice::from_ref(&flat))? {
                // Fill polygons wind counter-clockwise around n, the loft direction: the end cap faces it.
                let mut vs: Vec<u32> = poly.iter().map(|&i| ring[i as usize]).collect();
                if tag == CAP_START {
                    vs.reverse();
                }
                soup.poly(&vs, tag);
            }
        }
    }

    let (mut mesh, tags) = soup.finish()?;
    let faces_where =
        |pred: &dyn Fn(u16) -> bool| -> Vec<u32> { tags.iter().enumerate().filter(|(_, t)| pred(**t)).map(|(f, _)| f as u32).collect() };
    let mut groups: Vec<(String, Vec<u32>)> = vec![("side".into(), faces_where(&|t| t >= BAND0))];
    if caps {
        groups.push(("cap_start".into(), faces_where(&|t| t == CAP_START)));
        groups.push(("cap_end".into(), faces_where(&|t| t == CAP_END)));
    }
    for b in 0..bands {
        let tag = BAND0 + u16::try_from(b).unwrap_or(u16::MAX - BAND0);
        groups.push((format!("band_{b}"), faces_where(&|t| t == tag)));
    }
    for (name, faces) in groups {
        // Faces of a fresh mesh are all live, so this cannot fail.
        let _ = mesh.set_group(&name, ElementKind::Face, faces);
    }
    tag_groups_and_sharp(&mut mesh, &tags, &[], |a, b| (a < BAND0) != (b < BAND0));
    // Rails at corners both rows share.
    let corner_cos = LOFT_CORNER_DEG.to_radians().cos();
    let corner = |row: &[DVec3], k: usize| {
        let interior = spec.sections_closed || (k > 0 && k + 1 < np);
        interior && {
            let (a, b, c) = (row[(k + np - 1) % np], row[k], row[(k + 1) % np]);
            (b - a).normalize_or_zero().dot((c - b).normalize_or_zero()) < corner_cos
        }
    };
    for j in 0..strips {
        let j1 = (j + 1) % rows.len();
        let (r0, r1) = (&rows[j], &rows[j1]);
        for (k, (&a, &b)) in ids[j].iter().zip(&ids[j1]).enumerate() {
            if corner(r0, k)
                && corner(r1, k)
                && let Some(h) = mesh.find_halfedge(a, b)
            {
                let e = mesh.edge_of(h);
                mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
            }
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(r: f64, y: f64, n: usize, phase: f64) -> Vec<DVec3> {
        (0..n)
            .map(|i| {
                let a = phase + std::f64::consts::TAU * i as f64 / n as f64;
                DVec3::new(a.cos() * r, y, -a.sin() * r)
            })
            .collect()
    }

    fn spec(sections: &[Vec<DVec3>]) -> LoftSpec<'_> {
        LoftSpec { sections, sections_closed: true, points: 0, steps: 1, smooth: false, closed: false, caps: true }
    }

    fn closed_outward(m: &Mesh) -> f32 {
        m.validate().unwrap();
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
        let vol: f32 = m.faces().map(|f| m.face_normal(f).dot(m.face_centroid(f)) * m.face_area(f)).sum::<f32>() / 3.0;
        assert!(vol > 0.0, "faces point inward ({vol})");
        vol
    }

    #[test]
    fn square_to_circle_with_mixed_counts_and_windings() {
        let square = vec![DVec3::new(-0.5, 0.0, -0.5), DVec3::new(0.5, 0.0, -0.5), DVec3::new(0.5, 0.0, 0.5), DVec3::new(-0.5, 0.0, 0.5)];
        let mut circle = ring(0.4, 1.0, 16, 0.3);
        circle.reverse();
        let sections = [square, circle];
        let m = loft(&LoftSpec { points: 16, ..spec(&sections) }).unwrap();
        closed_outward(&m);
        assert_eq!(m.group("side").unwrap().count(), 16);
        let names: Vec<&str> = m.groups().map(|(n, _)| n).collect();
        assert_eq!(names, ["side", "cap_start", "cap_end", "band_0"]);
        assert!(m.group("cap_end").unwrap().iter().all(|f| m.face_normal(f).dot(Vec3::Y) > 0.999));
    }

    #[test]
    fn smooth_steps_and_rings() {
        let sections: Vec<Vec<DVec3>> = (0..4).map(|i| ring(0.3 + 0.1 * (i % 2) as f64, i as f64 * 0.3, 12, 0.0)).collect();
        let m = loft(&LoftSpec { steps: 4, smooth: true, ..spec(&sections) }).unwrap();
        closed_outward(&m);
        assert_eq!(m.group("side").unwrap().count(), 3 * 4 * 12);
        assert_eq!(m.group("band_1").unwrap().count(), 4 * 12);
        // A closed loft around a circle of sections: a torus with no caps.
        let around: Vec<Vec<DVec3>> = (0..8)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 8.0;
                let c = DVec3::new(a.cos(), 0.0, a.sin());
                (0..10)
                    .map(|k| {
                        let b = std::f64::consts::TAU * k as f64 / 10.0;
                        c + (c * b.cos() + DVec3::Y * b.sin()) * 0.2
                    })
                    .collect()
            })
            .collect();
        let t = loft(&LoftSpec { closed: true, ..spec(&around) }).unwrap();
        closed_outward(&t);
        assert_eq!(t.vert_count() as i64 - t.edge_count() as i64 + t.face_count() as i64, 0);
    }

    #[test]
    fn open_sections_make_ribbons_and_bad_input_fails() {
        let a = vec![DVec3::ZERO, DVec3::X];
        let b = vec![DVec3::new(1.0, 1.0, 0.0), DVec3::new(0.0, 1.0, 0.0)];
        let m = loft(&LoftSpec { sections_closed: false, ..spec(&[a.clone(), b]) }).unwrap();
        m.validate().unwrap();
        assert_eq!(m.face_count(), 1);
        // The reversed second section was turned around: the quad is not a bow tie.
        assert!(m.face_area(0) > 0.99);
        assert!(loft(&spec(std::slice::from_ref(&a))).is_err());
        assert!(loft(&LoftSpec { steps: 0, ..spec(&[a.clone(), a.clone()]) }).is_err());
    }
}
