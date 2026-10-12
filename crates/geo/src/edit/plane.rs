//! Cutting a mesh with a plane (bisect, with optional clearing and capping)
//! and making it mirror-symmetric across a plane.

use glam::{Affine3A, Vec3};
use p3d_mesh::{CornerSrc, ElementKind, Mesh};
use rustc_hash::FxHashMap;

use super::merge::merge_by_distance;
use super::{KernelError, patch};

/// Which side of the plane survives a cut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Keep {
    /// Both sides (just cut).
    #[default]
    Both,
    /// The side the normal points to.
    Above,
    /// The other side.
    Below,
}

/// A plane cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneCutSpec {
    /// A point on the plane.
    pub point: Vec3,
    /// The plane normal (need not be unit length).
    pub normal: Vec3,
    pub keep: Keep,
    /// Close the holes the cut opens on the plane (only with one side kept).
    pub fill: bool,
}

/// What a plane cut did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaneCutResult {
    /// Edges lying on the plane after the cut (the cut line).
    pub edges: Vec<u32>,
    /// Caps closing the holes (with `fill`).
    pub caps: Vec<u32>,
    pub faces_cut: usize,
    pub faces_removed: usize,
}

/// Vertices closer to the plane than this (relative to the mesh size) count
/// as on it.
const ON_PLANE: f32 = 1e-6;

/// Side of each corner of a polygon: -1, 0 or 1.
fn side(d: f32, eps: f32) -> i8 {
    if d > eps {
        1
    } else if d < -eps {
        -1
    } else {
        0
    }
}

/// Cuts every face that crosses the plane in two along it, with new vertices
/// where edges cross (shared by the faces on both sides). Faces that cross
/// it more than once (concave faces) are triangulated first. Optionally
/// deletes one side and caps the holes left on the plane.
///
/// Complexity: O(V + F + C) to classify, plus O(c log c) for the c corners
/// of the faces cut or removed.
pub fn plane_cut(mesh: &mut Mesh, spec: &PlaneCutSpec) -> Result<PlaneCutResult, KernelError> {
    let n = spec.normal;
    if !(n.is_finite() && n.length_squared() > 1e-12 && spec.point.is_finite()) {
        return Err(KernelError::OutOfRange {
            param: "normal",
            message: "the plane needs a finite, non-zero normal".into(),
            hint: "for example [1, 0, 0] for a YZ plane".into(),
        });
    }
    let n = n.normalize();
    let size = mesh.bounds().map_or(1.0, |(lo, hi)| (hi - lo).max_element().max(1e-6));
    let eps = ON_PLANE * size;
    let dist = |v: u32| (mesh.positions[v as usize] - spec.point).dot(n);
    let mut p = patch(mesh);
    // One new vertex per crossing edge.
    let mut cross: FxHashMap<u64, u32> = FxHashMap::default();
    let mut result = PlaneCutResult::default();
    let mut chords: Vec<(u32, u32)> = Vec::new();
    let faces: Vec<u32> = mesh.faces().collect();
    for f in faces {
        let sides: Vec<i8> = mesh.face_verts(f).map(|v| side(dist(v), eps)).collect();
        let (has_pos, has_neg) = (sides.contains(&1), sides.contains(&-1));
        let removed_side = match spec.keep {
            Keep::Both => 0,
            Keep::Above => -1,
            Keep::Below => 1,
        };
        if !(has_pos && has_neg) {
            // Entirely on one side (or on the plane).
            let s = if has_pos {
                1
            } else if has_neg {
                -1
            } else {
                0
            };
            if s != 0 && s == removed_side {
                p.remove_face(f);
                result.faces_removed += 1;
            }
            continue;
        }
        // Concave faces may cross more than once: split their triangles instead.
        let hs: Vec<u32> = mesh.face_halfedges(f).collect();
        let changes = {
            let nz: Vec<i8> = sides.iter().copied().filter(|&s| s != 0).collect();
            (0..nz.len()).filter(|&i| nz[i] != nz[(i + 1) % nz.len()]).count()
        };
        let pieces: Vec<Vec<u32>> = if changes == 2 { vec![hs.clone()] } else { mesh.face_triangles(f).iter().map(|t| t.to_vec()).collect() };
        p.remove_face(f);
        result.faces_cut += 1;
        for piece in pieces {
            // Corners of the piece with crossing points inserted:
            // (vertex, source a, source b, t, side).
            let mut ring: Vec<(u32, CornerSrc, i8)> = Vec::new();
            for (i, &h) in piece.iter().enumerate() {
                let g = piece[(i + 1) % piece.len()];
                let (a, b) = (mesh.origin(h), mesh.origin(g));
                let (da, db) = (dist(a), dist(b));
                let (sa, sb) = (side(da, eps), side(db, eps));
                ring.push((a, CornerSrc::copy(h), sa));
                if sa * sb < 0 {
                    let t = da / (da - db);
                    // Keyed by the vertex pair: an edge, or the diagonal of a
                    // triangulated concave face, shared by both of its sides.
                    let key = (u64::from(a.min(b)) << 32) | u64::from(a.max(b));
                    let v = *cross.entry(key).or_insert_with(|| {
                        let pa = mesh.positions[a as usize];
                        p.new_vert(pa.lerp(mesh.positions[b as usize], t), None)
                    });
                    ring.push((v, CornerSrc::lerp(h, g, t), 0));
                }
            }
            // Split the ring at its two on-plane corners where the sides change.
            let k = ring.len();
            let zeros: Vec<usize> = (0..k)
                .filter(|&i| ring[i].2 == 0)
                .filter(|&i| {
                    let before = (1..k).map(|j| ring[(i + k - j) % k].2).find(|&s| s != 0).unwrap_or(0);
                    let after = (1..k).map(|j| ring[(i + j) % k].2).find(|&s| s != 0).unwrap_or(0);
                    before != after
                })
                .collect();
            let keep_piece = |s: i8| s != removed_side;
            if zeros.len() != 2 {
                // Touching only: keep the piece whole on its side.
                let s = ring.iter().map(|c| c.2).find(|&s| s != 0).unwrap_or(0);
                if keep_piece(s) {
                    let vs: Vec<u32> = ring.iter().map(|c| c.0).collect();
                    let src: Vec<CornerSrc> = ring.iter().map(|c| c.1).collect();
                    p.add_face(&vs, &src, mesh.face_material(f), Some(f));
                }
                continue;
            }
            let (i, j) = (zeros[0], zeros[1]);
            for (from, to) in [(i, j), (j, i)] {
                let mut part: Vec<(u32, CornerSrc, i8)> = Vec::new();
                let mut x = from;
                loop {
                    part.push(ring[x]);
                    if x == to {
                        break;
                    }
                    x = (x + 1) % k;
                }
                let s = part.iter().map(|c| c.2).find(|&s| s != 0).unwrap_or(0);
                if part.len() >= 3 && keep_piece(s) {
                    let vs: Vec<u32> = part.iter().map(|c| c.0).collect();
                    let src: Vec<CornerSrc> = part.iter().map(|c| c.1).collect();
                    p.add_face(&vs, &src, mesh.face_material(f), Some(f));
                }
            }
            chords.push((ring[i].0, ring[j].0));
        }
    }
    let r = mesh.apply_patch(&p)?;
    let id = |v: u32| if p.is_new_vert(v) { r.verts[p.new_vert_index(v)] } else { v };
    let mut edges: Vec<u32> = chords.iter().filter_map(|&(a, b)| mesh.find_halfedge(id(a), id(b)).map(|h| h >> 1)).collect();
    edges.sort_unstable();
    edges.dedup();
    result.edges = edges;
    if spec.fill && spec.keep != Keep::Both {
        // Holes whose vertices all lie on the plane.
        let mut done: FxHashMap<u32, ()> = FxHashMap::default();
        let starts: Vec<u32> = (0..mesh.he_slots() as u32).filter(|&h| mesh.is_he_alive(h) && mesh.is_boundary_he(h)).collect();
        for h in starts {
            if done.contains_key(&h) || !mesh.is_boundary_he(h) {
                continue;
            }
            let Ok(lp) = mesh.boundary_loop(h) else { continue };
            for &x in &lp {
                done.insert(x, ());
            }
            let on = lp.iter().all(|&x| (mesh.positions[mesh.origin(x) as usize] - spec.point).dot(n).abs() <= eps * 10.0);
            if on
                && lp.len() >= 3
                && let Ok(f) = mesh.fill_boundary_loop(h)
            {
                result.caps.push(f);
            }
        }
    }
    Ok(result)
}

/// Mirror axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }
}

/// What a symmetrize did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymmetrizeResult {
    /// Faces of the mirrored half.
    pub mirrored: Vec<u32>,
    /// Vertices welded along the mirror plane.
    pub welded: usize,
}

/// Makes a mesh mirror-symmetric across the plane through the origin
/// normal to `axis`: the half on the `keep_positive` side is kept, the
/// other half is replaced by its mirror image, and the two are welded along
/// the plane (within `weld` meters).
///
/// Complexity: O(V + F + C) plus the weld's O(n log n) on the seam.
pub fn symmetrize(mesh: &mut Mesh, axis: Axis, keep_positive: bool, weld: f32) -> Result<SymmetrizeResult, KernelError> {
    let n = axis.unit() * if keep_positive { 1.0 } else { -1.0 };
    plane_cut(mesh, &PlaneCutSpec { point: Vec3::ZERO, normal: n, keep: Keep::Above, fill: false })?;
    mesh.compact();
    if mesh.face_count() == 0 {
        return Err(KernelError::Empty("nothing lies on the kept side of the mirror plane".into()));
    }
    let mut half = mesh.clone();
    let mut s = Vec3::ONE;
    s[axis_index(axis)] = -1.0;
    half.transform(Affine3A::from_scale(s));
    let [_, _, fo] = mesh.append(&half);
    // A temporary group follows the mirrored faces through the weld (which
    // replaces the faces along the seam).
    const TRACK: &str = "__symmetrize_mirrored";
    let half_faces: Vec<u32> = (fo..mesh.face_slots() as u32).filter(|&f| mesh.is_face_alive(f)).collect();
    let _ = mesh.set_group(TRACK, ElementKind::Face, half_faces);
    let seam: Vec<u32> = mesh.verts().filter(|&v| mesh.positions[v as usize].dot(n).abs() <= weld.max(1e-7)).collect();
    let r = merge_by_distance(mesh, Some(&seam), weld.max(1e-7));
    let mirrored: Vec<u32> = mesh.group(TRACK).map(|g| g.iter().collect()).unwrap_or_default();
    mesh.remove_group(TRACK);
    let r = r?;
    // Seam vertices sit exactly on the plane.
    for v in mesh.verts().collect::<Vec<_>>() {
        if mesh.positions[v as usize].dot(n).abs() <= weld.max(1e-7) {
            mesh.positions[v as usize][axis_index(axis)] = 0.0;
        }
    }
    Ok(SymmetrizeResult { mirrored, welded: r.merged })
}

fn axis_index(a: Axis) -> usize {
    match a {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, CapKind, CylinderSpec, Origin, QuadSphereSpec, cuboid, cylinder, quad_sphere};
    use glam::DVec3;

    #[test]
    fn cutting_a_box_in_half() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let r = plane_cut(&mut m, &PlaneCutSpec { point: Vec3::ZERO, normal: Vec3::Y, keep: Keep::Both, fill: false }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces_cut, r.edges.len(), m.face_count()), (4, 4, 10));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // Keep the top half and cap it.
        let r = plane_cut(&mut m, &PlaneCutSpec { point: Vec3::ZERO, normal: Vec3::Y, keep: Keep::Above, fill: true }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.faces_removed, r.caps.len()), (5, 1));
        let (lo, hi) = m.bounds().unwrap();
        assert!(lo.y.abs() < 1e-6 && (hi.y - 0.5).abs() < 1e-6);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "capped");
        assert!(m.face_normal(r.caps[0]).abs_diff_eq(Vec3::NEG_Y, 1e-5));
    }

    #[test]
    fn slanted_cut_through_a_sphere() {
        let mut m = quad_sphere(&QuadSphereSpec { radius: 1.0, segments: 6 }, Origin::Center).unwrap();
        let normal = Vec3::new(1.0, 2.0, 0.5);
        let r = plane_cut(&mut m, &PlaneCutSpec { point: Vec3::new(0.0, 0.1, 0.0), normal, keep: Keep::Below, fill: true }).unwrap();
        m.validate().unwrap();
        assert_eq!(r.caps.len(), 1);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        let n = normal.normalize();
        assert!(m.verts().all(|v| (m.positions[v as usize] - Vec3::new(0.0, 0.1, 0.0)).dot(n) <= 1e-4));
    }

    #[test]
    fn symmetrize_a_shifted_cylinder() {
        let mut m = cylinder(&CylinderSpec { segments: 12, cap: CapKind::Ngon, ..CylinderSpec::default() }, Origin::Center).unwrap();
        m.transform(Affine3A::from_translation(Vec3::new(0.2, 0.0, 0.0)));
        let r = symmetrize(&mut m, Axis::X, true, 1e-4).unwrap();
        m.validate().unwrap();
        assert!(r.welded > 0);
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "welded closed");
        let (lo, hi) = m.bounds().unwrap();
        assert!((lo.x + hi.x).abs() < 1e-5, "symmetric: {lo} {hi}");
        for v in m.verts() {
            let p = m.positions[v as usize];
            let q = Vec3::new(-p.x, p.y, p.z);
            assert!(m.verts().any(|w| m.positions[w as usize].abs_diff_eq(q, 1e-4)), "{p} has no mirror");
        }
    }
}
