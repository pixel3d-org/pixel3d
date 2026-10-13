//! Edge flow and deformation (project spec 17.10) on parts tagged
//! `deforming`: triangles (`DEFORM_TRIANGLES`), poles of valence 6 or more
//! (`DEFORM_POLES`) and sudden changes of face size (`DENSITY_JUMP`). Also
//! the kernels the spec layer runs at landmarks: edge loops across a joint
//! ([`joint_loops`]) and edge flow circling an eye or the mouth
//! ([`ring_flow`]). Everything walks the half-edge mesh: O(n) or
//! O(n log n) per part.

use glam::{Mat3, Vec3};
use serde_json::json;

use super::source::{SpecPart, edit_fix, spec_part};
use super::{Draft, Fix, Severity, decal, tagged};
use crate::fmt::thousands;
use crate::view::{Part, View};

/// The tag that marks a part as bending in animation.
pub const DEFORMING: &str = "deforming";
/// Triangles above this share of a deforming part's faces are flagged.
pub const MAX_TRI_SHARE: f32 = 0.05;
/// Interior vertices with this many edges or more pinch when bent.
pub const POLE_VALENCE: usize = 6;
/// Neighbouring faces whose sizes (square roots of areas) differ by more
/// than this factor make a density jump.
pub const DENSITY_RATIO: f32 = 4.0;
/// Edge loops a joint needs within one limb diameter to bend cleanly.
pub const MIN_JOINT_LOOPS: f32 = 3.0;
/// Edge flow around an eye or the mouth circles it when [`ring_flow`] is
/// under this: a grid or triangles that ignore the point measure about
/// 0.64, `remesh.quad` loops laid with edges a tenth of their radius about
/// 0.4.
pub const RING_FLOW: f32 = 0.5;

/// The fix that rebuilds a part's topology as even, flowing quads: a
/// `remesh.quad` edit layer on a spec part (heads get eye and mouth loops by
/// themselves), else the op itself. Open parts get a quad-dominant
/// isotropic remesh instead.
///
/// Complexity: O(size of the part's edit list).
pub fn remesh_fix(view: &View, i: usize, sp: Option<&SpecPart>, extra: serde_json::Value, summary: &str) -> Fix {
    let part = &view.parts[i];
    let closed = part.topology().is_closed();
    let (op, mut args) = if closed { ("remesh.quad", extra) } else { ("remesh.isotropic", json!({ "quads": true })) };
    match sp {
        Some(sp) => edit_fix(view.doc, sp, op, args, format!("{summary} (a {op} edit layer)")),
        None => {
            if let Some(a) = args.as_object_mut() {
                a.insert("target".into(), json!(part.path));
            }
            Fix::op(op, args, summary)
        }
    }
}

/// Edge-flow findings of part `i` (parts tagged `deforming` only).
///
/// Complexity: O(V + E + F) over the part's mesh.
pub fn check(view: &View, i: usize, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    if !tagged(view, part, DEFORMING) || decal(view, part) || tagged(view, part, "strands") {
        return;
    }
    let m = part.mesh;
    let sp = spec_part(view.doc, part.key);
    let (mut faces, mut tris, mut tri_sum) = (0usize, 0usize, Vec3::ZERO);
    for f in m.faces() {
        faces += 1;
        if m.face_len(f) == 3 {
            tris += 1;
            tri_sum += m.face_centroid(f);
        }
    }
    let share = tris as f32 / faces.max(1) as f32;
    if share > MAX_TRI_SHARE {
        let at = part.world.transform_point3(tri_sum / tris.max(1) as f32);
        let mut d = Draft::new(
            "DEFORM_TRIANGLES",
            Severity::Warn,
            i,
            at,
            format!(
                "{}% of its faces are triangles ({} of {}): they crease and pinch when it bends (limit {}%)",
                (share * 100.0).round() as i32,
                thousands(tris),
                thousands(faces),
                (MAX_TRI_SHARE * 100.0) as i32
            ),
        )
        .measure(share * 100.0, MAX_TRI_SHARE * 100.0, "%");
        if share < 0.5 {
            // A few triangles in a quad mesh: pair them up.
            let fix = match &sp {
                Some(sp) => edit_fix(view.doc, sp, "edit.tris_to_quads", json!({}), "join the triangles into quads (an edit layer)"),
                None => Fix::op("edit.tris_to_quads", json!({ "target": part.path }), "join the triangles into quads"),
            };
            d.fixes.push(fix);
        }
        d.fixes.push(remesh_fix(view, i, sp.as_ref(), json!({}), "rebuild it as flowing quads"));
        out.push(d);
    }
    // Poles only mean something in a quad-dominant mesh.
    if share < 0.5 {
        let (mut poles, mut worst) = (0usize, None::<(usize, u32)>);
        for v in m.verts() {
            if m.is_boundary_vert(v) {
                continue;
            }
            let k = m.valence(v);
            if k >= POLE_VALENCE {
                poles += 1;
                if worst.is_none_or(|(w, _)| k > w) {
                    worst = Some((k, v));
                }
            }
        }
        if let Some((k, v)) = worst {
            let at = part.world.transform_point3(m.positions[v as usize]);
            let mut d = Draft::new(
                "DEFORM_POLES",
                Severity::Warn,
                i,
                at,
                format!(
                    "{poles} pole{} with {POLE_VALENCE} or more edges (worst {k}): they pinch and shade badly when it bends",
                    if poles == 1 { "" } else { "s" }
                ),
            )
            .count(poles, 0);
            d.fixes.push(remesh_fix(view, i, sp.as_ref(), json!({}), "rebuild it with poles of at most 5 edges"));
            out.push(d);
        }
    }
    density_jump(view, i, sp.as_ref(), out);
}

/// Neighbouring faces of very different sizes.
///
/// Complexity: O(E + F).
fn density_jump(view: &View, i: usize, sp: Option<&SpecPart>, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    let m = part.mesh;
    let (mut jumps, mut worst) = (0usize, None::<(f32, u32)>);
    for e in m.edges() {
        let [Some(f), Some(g)] = m.edge_faces(e) else { continue };
        let (a, b) = (m.face_area(f), m.face_area(g));
        if a <= 0.0 || b <= 0.0 {
            continue;
        }
        let r = (a.max(b) / a.min(b)).sqrt();
        if r > DENSITY_RATIO {
            jumps += 1;
            if worst.is_none_or(|(w, _)| r > w) {
                worst = Some((r, e));
            }
        }
    }
    let Some((r, e)) = worst else { return };
    let [a, b] = m.edge_verts(e);
    let at = part.world.transform_point3((m.positions[a as usize] + m.positions[b as usize]) / 2.0);
    let mut d = Draft::new(
        "DENSITY_JUMP",
        Severity::Info,
        i,
        at,
        format!(
            "{} edge{} between faces whose sizes differ more than {DENSITY_RATIO}x (worst {:.1}x): it bends unevenly there",
            thousands(jumps),
            if jumps == 1 { "" } else { "s" },
            r
        ),
    )
    .measure(r, DENSITY_RATIO, "ratio");
    d.fixes.push(remesh_fix(view, i, sp, json!({}), "even out the face sizes"));
    out.push(d);
}

/// Edge loops across a joint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointLoops {
    /// Loops within one limb diameter of the joint (rounded down to tenths).
    pub loops: f32,
    /// The limb's axis at the joint (world).
    pub axis: Vec3,
    /// The limb's radius at the joint.
    pub radius: f32,
    /// Median length of the faces along the axis there.
    pub spacing: f32,
}

/// The smallest eigenvector of a symmetric positive semi-definite matrix,
/// by power iteration on `trace * I - m`.
///
/// Complexity: O(1) (a fixed number of iterations).
fn smallest_eigenvector(m: Mat3) -> Vec3 {
    let t = m.x_axis.x + m.y_axis.y + m.z_axis.z;
    let s = Mat3::from_diagonal(Vec3::splat(t)) - m;
    let mut v = Vec3::new(0.577, 0.578, 0.576);
    for _ in 0..64 {
        let w = s * v;
        let n = w.length();
        if n < 1e-20 {
            break;
        }
        v = w / n;
    }
    v
}

/// How many edge loops cross the limb at a joint point on the surface:
/// the limb's axis is the direction the nearby surface normals avoid, its
/// radius comes from the cross-section through the joint, and loops are one
/// limb diameter over the median length of the faces along the axis there.
///
/// Complexity: O(log n + k) for the k triangles near the joint (BVH box
/// query), plus O(k log k) for the median.
pub fn joint_loops(part: &Part, at: Vec3) -> Option<JointLoops> {
    let bvh = part.bvh();
    let reach = 0.25 * part.size().max_element();
    let mut near: Vec<u32> = Vec::new();
    bvh.query_box(at - Vec3::splat(reach), at + Vec3::splat(reach), |t| {
        let tri = bvh.triangle(t);
        if p3d_geo::bvh::closest_on_triangle(at, tri).distance(at) <= reach {
            near.push(t);
        }
        true
    });
    if near.is_empty() {
        return None;
    }
    // The axis: the direction the surface normals around the joint avoid.
    let mut cov = Mat3::ZERO;
    for &t in &near {
        let [a, b, c] = bvh.triangle(t);
        let n = (b - a).cross(c - a);
        let w = n.length();
        if w > 0.0 {
            let u = n / w;
            cov += Mat3::from_cols(u * u.x, u * u.y, u * u.z) * (w * 0.5);
        }
    }
    let axis = smallest_eigenvector(cov).normalize_or_zero();
    if axis == Vec3::ZERO {
        return None;
    }
    // The cross-section through the joint: where triangles cross its plane.
    let (mut ring, mut sum) = (Vec::new(), Vec3::ZERO);
    for &t in &near {
        let tri = bvh.triangle(t);
        let d = tri.map(|p| (p - at).dot(axis));
        let mut cut = SmallCut::default();
        for k in 0..3 {
            let (p, q) = (tri[k], tri[(k + 1) % 3]);
            let (dp, dq) = (d[k], d[(k + 1) % 3]);
            if (dp <= 0.0) != (dq <= 0.0) {
                cut.push(p + (q - p) * (dp / (dp - dq)));
            }
        }
        if let Some(mid) = cut.mid() {
            ring.push(mid);
            sum += mid;
        }
    }
    if ring.len() < 3 {
        return None;
    }
    let center = sum / ring.len() as f32;
    let radius = ring.iter().map(|p| p.distance(center)).sum::<f32>() / ring.len() as f32;
    if radius <= 0.0 {
        return None;
    }
    // Faces within one radius of the joint plane and near the limb: their
    // lengths along the axis.
    let m = part.mesh;
    let mut faces: Vec<u32> = near.iter().map(|&t| bvh.face(t)).collect();
    faces.sort_unstable();
    faces.dedup();
    let mut extents: Vec<f32> = Vec::with_capacity(faces.len());
    for f in faces {
        let (mut lo, mut hi, mut radial) = (f32::INFINITY, f32::NEG_INFINITY, 0.0f32);
        for v in m.face_verts(f) {
            let p = part.world.transform_point3(m.positions[v as usize]);
            let t = (p - at).dot(axis);
            lo = lo.min(t);
            hi = hi.max(t);
            let off = p - center - axis * (p - center).dot(axis);
            radial = radial.max(off.length());
        }
        if hi >= -radius && lo <= radius && radial <= 2.5 * radius {
            extents.push((hi - lo).max(1e-6));
        }
    }
    if extents.is_empty() {
        return None;
    }
    extents.sort_unstable_by(f32::total_cmp);
    let spacing = extents[extents.len() / 2];
    let loops = ((2.0 * radius / spacing) * 10.0).floor() / 10.0;
    Some(JointLoops { loops, axis, radius, spacing })
}

/// Up to two crossing points of a triangle with a plane.
#[derive(Default)]
struct SmallCut {
    pts: [Vec3; 2],
    n: usize,
}

impl SmallCut {
    fn push(&mut self, p: Vec3) {
        if self.n < 2 {
            self.pts[self.n] = p;
            self.n += 1;
        }
    }

    fn mid(&self) -> Option<Vec3> {
        (self.n == 2).then(|| (self.pts[0] + self.pts[1]) / 2.0)
    }
}

/// How far the edge flow around a point is from circling it: the
/// length-weighted mean of `|sin 2a|` over the edges in the ring from 0.4 to
/// 1 times `radius` around `at`, where `a` is the angle between an edge and
/// the circle through it (0 when every edge runs around the point or
/// straight out from it, as in concentric loops; about 0.64 for a grid or
/// triangles that ignore the point). The band is where `remesh.quad` lays
/// its loops. None when no edge lies in the band.
///
/// Complexity: O(E + F) over the part's mesh.
pub fn ring_flow(part: &Part, at: Vec3, radius: f32) -> Option<f32> {
    let m = part.mesh;
    let w = |v: u32| part.world.transform_point3(m.positions[v as usize]);
    let normals = m.face_normals();
    let (lo, hi) = (0.4 * radius, radius);
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for e in m.edges() {
        let [a, b] = m.edge_verts(e);
        let (pa, pb) = (w(a), w(b));
        let mid = (pa + pb) / 2.0;
        let d = mid - at;
        let r = d.length();
        if !(lo..=hi).contains(&r) {
            continue;
        }
        let n = m.edge_faces(e).into_iter().flatten().map(|f| normals[f as usize]).sum::<Vec3>();
        let n = part.world.transform_vector3(n).normalize_or_zero();
        let tangent = n.cross(d).normalize_or_zero();
        let out = tangent.cross(n);
        let dir = pb - pa;
        let len = dir.length();
        let (c, s) = (dir.dot(tangent).abs(), dir.dot(out).abs());
        let q = c * c + s * s;
        if tangent == Vec3::ZERO || q <= 0.0 {
            continue;
        }
        sum += len * 2.0 * c * s / q;
        weight += len;
    }
    (weight > 0.0).then(|| sum / weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smallest_eigenvector_of_a_flat_spread_is_its_normal() {
        let m = Mat3::from_diagonal(Vec3::new(2.0, 0.01, 1.0));
        let v = smallest_eigenvector(m);
        assert!(v.y.abs() > 0.99, "{v}");
    }
}
