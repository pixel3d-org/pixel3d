//! Sweeps: a cross section carried along a path on rotation-minimizing
//! frames (double reflection, Wang et al. 2008), with twist, scale, mitered
//! joints and caps.

use glam::{DVec2, DVec3};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};

use super::shape::{area2, fill};
use super::{MAX_GEN_FACES, invalid, strip};
use crate::prim::soup::{Soup, tag_groups_and_sharp};
use crate::prim::{PrimError, quad_grid_cap};

/// Joints that turn more than this many degrees get sharp edges.
pub const SWEEP_CORNER_DEG: f64 = 30.0;
/// Largest stretch of the cross section at a mitered joint (1 / cos of half
/// the turn); sharper joints are flattened to it.
pub const MAX_MITER: f64 = 4.0;
/// Most path points.
pub const MAX_PATH_POINTS: usize = 200_000;

const CAP_START: u16 = 0;
const CAP_END: u16 = 1;
const SPAN0: u16 = 2;

/// How the open ends of a closed cross section on an open path are filled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SweepCaps {
    /// Open ends.
    None,
    /// Constrained Delaunay fill merged into quads.
    #[default]
    Fill,
    /// Quads only; needs a round cross section from [`round_profile`].
    QuadGrid,
}

/// Sweep parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct SweepSpec<'a> {
    /// Cross section seen looking along the path: x to the right, y up.
    /// A closed one may wind either way; its faces point outward.
    pub profile: &'a [DVec2],
    pub profile_closed: bool,
    /// Span of each profile segment for groups `profile_<span>`; no profile
    /// groups when empty.
    pub profile_spans: &'a [u32],
    pub path: &'a [DVec3],
    pub path_closed: bool,
    /// Turn of the cross section from start to end (radians). A closed path
    /// needs whole turns.
    pub twist: f64,
    /// Cross section scale at the start and at the end (linear in length).
    pub scale: [f64; 2],
    pub caps: SweepCaps,
    /// Direction the cross section's y leans toward at the start.
    pub up: DVec3,
}

/// A circle cross section of `segments` points (a multiple of 4) placed so
/// that [`SweepCaps::QuadGrid`] can fill its ends.
///
/// Complexity: O(segments).
pub fn round_profile(radius: f64, segments: u32) -> Vec<DVec2> {
    (0..segments)
        .map(|k| {
            let a = std::f64::consts::FRAC_PI_4 + std::f64::consts::TAU * f64::from(k) / f64::from(segments);
            DVec2::new(a.cos(), a.sin()) * radius
        })
        .collect()
}

/// A frame along the path: right, up, tangent, and the miter stretch.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    pub(crate) r: DVec3,
    pub(crate) s: DVec3,
    pub(crate) t: DVec3,
    /// Bend direction and stretch factor at a joint.
    bend: DVec3,
    stretch: f64,
}

/// Reflects `v` in the plane through the origin perpendicular to `n`
/// (`c` = n.n).
fn reflect(v: DVec3, n: DVec3, c: f64) -> DVec3 {
    v - n * (2.0 * n.dot(v) / c)
}

/// Unit tangents (joint bisectors) with bend directions and miter stretch.
///
/// Complexity: O(points).
fn tangents(path: &[DVec3], closed: bool) -> Result<Vec<(DVec3, DVec3, f64)>, PrimError> {
    let n = path.len();
    let dir = |i: usize| (path[(i + 1) % n] - path[i]).normalize();
    (0..n)
        .map(|i| {
            let prev = if i > 0 || closed { Some(dir((i + n - 1) % n)) } else { None };
            let next = if i + 1 < n || closed { Some(dir(i)) } else { None };
            match (prev, next) {
                (Some(a), Some(b)) => {
                    let sum = a + b;
                    if sum.length() < 1e-6 {
                        return Err(invalid("path", format!("point {i}"), "turns straight back on itself"));
                    }
                    let t = sum.normalize();
                    let cos = t.dot(b);
                    let bend = (b - a).normalize_or_zero();
                    Ok((t, bend, (1.0 / cos).min(MAX_MITER)))
                }
                (Some(d), None) | (None, Some(d)) => Ok((d, DVec3::ZERO, 1.0)),
                (None, None) => Err(invalid("path", "1 point", "needs at least 2 points")),
            }
        })
        .collect()
}

/// Rotation-minimizing frames by double reflection; a closed path spreads
/// the leftover turn evenly by length so the seam meets.
///
/// Complexity: O(points).
pub(crate) fn frames(path: &[DVec3], closed: bool, up: DVec3) -> Result<Vec<Frame>, PrimError> {
    let tans = tangents(path, closed)?;
    let n = path.len();
    let t0 = tans[0].0;
    let up = if up.normalize_or_zero().dot(t0).abs() > 0.999 || up.length_squared() == 0.0 {
        if t0.y.abs() > 0.999 { DVec3::Z } else { DVec3::Y }
    } else {
        up
    };
    let s0 = (up - t0 * up.dot(t0)).normalize();
    let mut rs = Vec::with_capacity(n + 1);
    rs.push(t0.cross(s0));
    let step = |r: DVec3, i: usize, j: usize| {
        let v1 = path[j] - path[i];
        let c1 = v1.length_squared();
        let (r_l, t_l) = (reflect(r, v1, c1), reflect(tans[i].0, v1, c1));
        let v2 = tans[j].0 - t_l;
        let c2 = v2.length_squared();
        if c2 < 1e-18 { r_l } else { reflect(r_l, v2, c2) }
    };
    for i in 0..n - 1 {
        let r = step(rs[i], i, i + 1);
        rs.push(r);
    }
    let mut out: Vec<Frame> = (0..n)
        .map(|i| {
            let (t, bend, stretch) = tans[i];
            let r = (rs[i] - t * rs[i].dot(t)).normalize();
            Frame { r, s: r.cross(t), t, bend, stretch }
        })
        .collect();
    if closed {
        // Carry the last frame around to the start and spread the mismatch.
        let back = step(rs[n - 1], n - 1, 0);
        let back = (back - t0 * back.dot(t0)).normalize();
        let gap = back.cross(out[0].r).dot(t0).atan2(back.dot(out[0].r));
        let lens = cumulative(path, true);
        let total = lens[n];
        for (i, f) in out.iter_mut().enumerate() {
            let a = gap * lens[i] / total;
            let (r, s) = (f.r * a.cos() + f.s * a.sin(), f.s * a.cos() - f.r * a.sin());
            f.r = r;
            f.s = s;
        }
    }
    Ok(out)
}

/// Arc length at each point (and the total at index `n` for closed paths).
fn cumulative(path: &[DVec3], closed: bool) -> Vec<f64> {
    let mut out = Vec::with_capacity(path.len() + 1);
    let mut acc = 0.0;
    out.push(0.0);
    for w in path.windows(2) {
        acc += w[0].distance(w[1]);
        out.push(acc);
    }
    if closed && let (Some(a), Some(b)) = (path.last(), path.first()) {
        out.push(acc + a.distance(*b));
    }
    out
}

/// Sweeps a cross section along a path. Groups: `side`, `cap_start` and
/// `cap_end` for filled ends, and `profile_<i>` per profile span when spans
/// are given. Cap rims, rails at cross-section corners and rings at path
/// joints sharper than [`SWEEP_CORNER_DEG`] are sharp.
///
/// Complexity: O(path points x profile points) plus O(p log p) per filled cap.
pub fn sweep(spec: &SweepSpec) -> Result<Mesh, PrimError> {
    let (np, nq) = (spec.path.len(), spec.profile.len());
    let min_profile = if spec.profile_closed { 3 } else { 2 };
    if nq < min_profile {
        return Err(invalid("profile", format!("{nq} points"), format!("needs at least {min_profile}")));
    }
    let min_path = if spec.path_closed { 3 } else { 2 };
    if np < min_path || np > MAX_PATH_POINTS {
        return Err(invalid("path", format!("{np} points"), format!("needs {min_path} to {MAX_PATH_POINTS} points")));
    }
    if spec.profile.iter().any(|p| !p.is_finite()) || spec.path.iter().any(|p| !p.is_finite()) {
        return Err(invalid("path", "a point", "is not finite"));
    }
    for i in 0..np {
        let j = (i + 1) % np;
        if (j > i || spec.path_closed) && spec.path[i].distance(spec.path[j]) <= 1e-9 {
            return Err(invalid("path", format!("point {i}"), format!("coincides with point {j}")));
        }
    }
    let tau = std::f64::consts::TAU;
    if !spec.twist.is_finite() || (spec.path_closed && ((spec.twist / tau).round() * tau - spec.twist).abs() > 1e-6) {
        return Err(invalid("twist", format!("{:.2} deg", spec.twist.to_degrees()), "a closed path needs whole turns (0, 360, ...)"));
    }
    if spec.scale.iter().any(|s| !(s.is_finite() && *s > 1e-6)) {
        return Err(invalid("scale", format!("{:?}", spec.scale), "must be above 0"));
    }
    let segs = if spec.path_closed { np } else { np - 1 };
    if segs.saturating_mul(nq) > MAX_GEN_FACES {
        return Err(invalid("path", format!("{np} points"), format!("x {nq} profile points would exceed {MAX_GEN_FACES} faces")));
    }
    if !spec.profile_spans.is_empty() && spec.profile_spans.len() != if spec.profile_closed { nq } else { nq - 1 } {
        return Err(invalid("profile_spans", spec.profile_spans.len(), "needs one span per profile segment"));
    }

    // A closed cross section runs counter-clockwise so faces point outward.
    let mut profile = spec.profile.to_vec();
    let mut spans = spec.profile_spans.to_vec();
    if spec.profile_closed && area2(&profile) < 0.0 {
        profile.reverse();
        if !spans.is_empty() {
            let last = spans[nq - 1];
            spans.truncate(nq - 1);
            spans.reverse();
            spans.push(last);
        }
    }
    let caps = if spec.profile_closed && !spec.path_closed { spec.caps } else { SweepCaps::None };
    if caps == SweepCaps::QuadGrid {
        let r = profile[0].length();
        let round = round_profile(r, nq as u32);
        if !nq.is_multiple_of(4) || profile.iter().zip(&round).any(|(a, b)| a.distance(*b) > r * 1e-6) {
            return Err(invalid("caps", "quad_grid", "needs a round cross section with a multiple of 4 points"));
        }
    }

    let fr = frames(spec.path, spec.path_closed, spec.up)?;
    let lens = cumulative(spec.path, spec.path_closed);
    let total = if spec.path_closed { lens[np] } else { lens[np - 1] };
    let mut soup = Soup::with_capacity(np * nq, segs * nq + 2, 4 * segs * nq);
    let mut rows: Vec<Vec<u32>> = Vec::with_capacity(np);
    let mut axes: Vec<(DVec3, DVec3, f64)> = Vec::with_capacity(np);
    for (i, f) in fr.iter().enumerate() {
        let u = lens[i] / total;
        let a = spec.twist * u;
        let (r, s) = (f.r * a.cos() + f.s * a.sin(), f.s * a.cos() - f.r * a.sin());
        let k = spec.scale[0] + (spec.scale[1] - spec.scale[0]) * u;
        axes.push((r, s, k));
        rows.push(
            profile
                .iter()
                .map(|p| {
                    let q = (r * p.x + s * p.y) * k;
                    let q = q + f.bend * (q.dot(f.bend) * (f.stretch - 1.0));
                    soup.push_vert(spec.path[i] + q)
                })
                .collect(),
        );
    }
    let profile_segs = if spec.profile_closed { nq } else { nq - 1 };
    for i in 0..segs {
        let (a, b) = (&rows[i], &rows[(i + 1) % np]);
        if spans.is_empty() {
            strip(&mut soup, a, b, spec.profile_closed, SPAN0);
        } else {
            // One column at a time so each keeps its span's tag.
            for k in 0..profile_segs {
                let k1 = (k + 1) % nq;
                let tag = u16::try_from(u32::from(SPAN0) + spans[k]).map_err(|_| invalid("profile_spans", spans[k], "at most 65,000 spans"))?;
                soup.poly(&[a[k], b[k], b[k1], a[k1]], tag);
            }
        }
    }
    let mut has_cap = [false; 2];
    if caps != SweepCaps::None {
        let polys = if caps == SweepCaps::Fill { fill(std::slice::from_ref(&profile))? } else { Vec::new() };
        for (end, tag) in [(0usize, CAP_START), (np - 1, CAP_END)] {
            has_cap[tag as usize] = true;
            let ring = &rows[end];
            let out = if tag == CAP_START { -fr[end].t } else { fr[end].t };
            match caps {
                SweepCaps::Fill => {
                    for p in &polys {
                        // Counter-clockwise in (x, y) faces back along the path: right for the start.
                        let mut v: Vec<u32> = p.iter().map(|&i| ring[i as usize]).collect();
                        if tag == CAP_END {
                            v.reverse();
                        }
                        soup.poly(&v, tag);
                    }
                }
                SweepCaps::QuadGrid => {
                    let (r, s, k) = axes[end];
                    let radius = profile[0].length();
                    let dirs: Vec<DVec3> = profile.iter().map(|p| DVec3::new(p.x, 0.0, p.y) / radius).collect();
                    let center = spec.path[end];
                    quad_grid_cap(&mut soup, ring, &dirs, radius, tag, |x, z| center + (r * x + s * z) * k, |_| out);
                }
                SweepCaps::None => {}
            }
        }
    }

    let (mut mesh, tags) = soup.finish()?;
    let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
    let faces_where =
        |pred: &dyn Fn(u16) -> bool| -> Vec<u32> { tags.iter().enumerate().filter(|(_, t)| pred(**t)).map(|(f, _)| f as u32).collect() };
    groups.push(("side".into(), faces_where(&|t| t >= SPAN0)));
    if has_cap[0] {
        groups.push(("cap_start".into(), faces_where(&|t| t == CAP_START)));
    }
    if has_cap[1] {
        groups.push(("cap_end".into(), faces_where(&|t| t == CAP_END)));
    }
    if !spans.is_empty() {
        let max = spans.iter().copied().max().unwrap_or(0);
        for span in 0..=max {
            let fs = faces_where(&|t| t == SPAN0 + span as u16);
            if !fs.is_empty() {
                groups.push((format!("profile_{span}"), fs));
            }
        }
    }
    for (name, faces) in groups {
        // Faces of a fresh mesh are all live, so this cannot fail.
        let _ = mesh.set_group(&name, ElementKind::Face, faces);
    }
    tag_groups_and_sharp(&mut mesh, &tags, &[], |a, b| (a < SPAN0) != (b < SPAN0));
    let corner_cos = SWEEP_CORNER_DEG.to_radians().cos();
    let sharpen = |mesh: &mut Mesh, a: u32, b: u32| {
        if let Some(h) = mesh.find_halfedge(a, b) {
            let e = mesh.edge_of(h);
            mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
    };
    // Rails along the path at cross-section corners.
    for k in 0..nq {
        let interior = spec.profile_closed || (k > 0 && k + 1 < nq);
        if !interior {
            continue;
        }
        let (a, b, c) = (profile[(k + nq - 1) % nq], profile[k], profile[(k + 1) % nq]);
        if (b - a).normalize().dot((c - b).normalize()) < corner_cos {
            for i in 0..segs {
                sharpen(&mut mesh, rows[i][k], rows[(i + 1) % np][k]);
            }
        }
    }
    // Rings at path joints.
    for i in 0..np {
        let interior = spec.path_closed || (i > 0 && i + 1 < np);
        if interior && fr[i].stretch > 1.0 / (SWEEP_CORNER_DEG.to_radians() * 0.5).cos() {
            for k in 0..profile_segs {
                sharpen(&mut mesh, rows[i][k], rows[i][(k + 1) % nq]);
            }
        }
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed_outward(m: &Mesh) -> f64 {
        m.validate().unwrap();
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
        let vol: f32 = m.faces().map(|f| m.face_normal(f).dot(m.face_centroid(f)) * m.face_area(f)).sum::<f32>() / 3.0;
        assert!(vol > 0.0, "faces point inward ({vol})");
        f64::from(vol)
    }

    fn square(h: f64) -> Vec<DVec2> {
        vec![DVec2::new(-h, -h), DVec2::new(h, -h), DVec2::new(h, h), DVec2::new(-h, h)]
    }

    fn spec<'a>(profile: &'a [DVec2], path: &'a [DVec3]) -> SweepSpec<'a> {
        SweepSpec {
            profile,
            profile_closed: true,
            profile_spans: &[],
            path,
            path_closed: false,
            twist: 0.0,
            scale: [1.0; 2],
            caps: SweepCaps::Fill,
            up: DVec3::Y,
        }
    }

    #[test]
    fn straight_bar_matches_its_box_either_winding() {
        let path = [DVec3::ZERO, DVec3::new(0.0, 0.0, -2.0)];
        for profile in [square(0.1), square(0.1).into_iter().rev().collect()] {
            let m = sweep(&spec(&profile, &path)).unwrap();
            let vol = closed_outward(&m);
            assert!((vol - 0.2 * 0.2 * 2.0).abs() < 1e-5, "{vol}");
            assert_eq!(m.face_count(), 4 + 2);
            // Looking along -Z with y up, the cross section's y is world +Y.
            let (lo, hi) = m.bounds().unwrap();
            assert!((hi.y - 0.1).abs() < 1e-6 && (lo.y + 0.1).abs() < 1e-6);
        }
    }

    #[test]
    fn mitered_frame_keeps_its_thickness() {
        // A square picture frame swept along a closed square path.
        let path = [DVec3::new(-1.0, 0.0, -1.0), DVec3::new(1.0, 0.0, -1.0), DVec3::new(1.0, 0.0, 1.0), DVec3::new(-1.0, 0.0, 1.0)];
        let profile = square(0.05);
        let m = sweep(&SweepSpec { path_closed: true, ..spec(&profile, &path) }).unwrap();
        let vol = closed_outward(&m);
        // Four mitered bars: the outer square of side 2.1 minus the inner one of side 1.9, 0.1 thick.
        assert!((vol - 0.1 * (2.1 * 2.1 - 1.9 * 1.9)).abs() < 1e-4, "{vol}");
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 0);
        // Rails at the 4 profile corners (4 x 4 edges) and the 4 mitered rings (4 x 4 edges).
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 32);
    }

    #[test]
    fn frames_do_not_twist_on_a_helix_and_closed_seams_meet() {
        let helix: Vec<DVec3> = (0..200)
            .map(|i| {
                let a = i as f64 * 0.1;
                DVec3::new(a.cos(), i as f64 * 0.01, a.sin())
            })
            .collect();
        let fr = frames(&helix, false, DVec3::Y).unwrap();
        for w in fr.windows(2) {
            // Rotation-minimizing: consecutive right vectors stay close (no spinning about the tangent).
            assert!(w[0].r.dot(w[1].r) > 0.99);
            assert!(w[1].r.dot(w[1].t).abs() < 1e-9 && (w[1].s.length() - 1.0).abs() < 1e-9);
        }
        let ring: Vec<DVec3> = (0..24)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 24.0;
                DVec3::new(a.cos(), 0.3 * (3.0 * a).sin(), a.sin())
            })
            .collect();
        let profile = round_profile(0.05, 8);
        let m = sweep(&SweepSpec { path_closed: true, caps: SweepCaps::None, ..spec(&profile, &ring) }).unwrap();
        closed_outward(&m);
        assert!(sweep(&SweepSpec { path_closed: true, twist: 1.0, ..spec(&profile, &ring) }).is_err(), "partial twist on a closed path");
    }

    #[test]
    fn round_caps_twist_scale_and_spans() {
        let path = [DVec3::ZERO, DVec3::new(0.0, 1.0, 0.0), DVec3::new(1.0, 1.0, 0.0)];
        let profile = round_profile(0.1, 12);
        let m = sweep(&SweepSpec { caps: SweepCaps::QuadGrid, twist: 1.0, scale: [1.0, 0.5], ..spec(&profile, &path) }).unwrap();
        closed_outward(&m);
        assert!(m.faces().all(|f| m.face_len(f) == 4));
        assert!(m.group("cap_start").unwrap().iter().all(|f| m.face_normal(f).dot(glam::Vec3::NEG_Y) > 0.999));
        assert!(m.group("cap_end").unwrap().iter().all(|f| m.face_normal(f).dot(glam::Vec3::X) > 0.999));
        assert!(sweep(&SweepSpec { caps: SweepCaps::QuadGrid, ..spec(&square(0.1), &path) }).is_err());
        // An open profile (an L-shaped trim) with two spans.
        let trim = [DVec2::new(0.0, 0.0), DVec2::new(0.05, 0.0), DVec2::new(0.05, 0.02)];
        let m = sweep(&SweepSpec { profile_closed: false, profile_spans: &[0, 1], ..spec(&trim, &path) }).unwrap();
        m.validate().unwrap();
        assert_eq!((m.group("profile_0").unwrap().count(), m.group("profile_1").unwrap().count()), (2, 2));
        assert!(m.group("cap_start").is_none());
        let back = [DVec3::ZERO, DVec3::X, DVec3::ZERO];
        assert!(sweep(&spec(&profile, &back)).is_err(), "a path that reverses");
    }
}
