//! Arrays and mirrors: copies of a mesh placed by transforms and joined.

use glam::{Affine3A, DVec3, Mat3A, Quat, Vec3, Vec3A};
use p3d_mesh::Mesh;

use super::sweep::frames;
use super::{MAX_GEN_FACES, invalid};
use crate::curve::resample;
use crate::edit::{Axis, KernelError, merge_by_distance};
use crate::prim::PrimError;

/// Most copies in one array.
pub const MAX_COPIES: u32 = 10_000;

/// `count` copies on a line, each `step` after the previous; the first
/// stays in place.
///
/// Complexity: O(count).
pub fn linear_copies(count: u32, step: Vec3) -> Vec<Affine3A> {
    (0..count).map(|i| Affine3A::from_translation(step * i as f32)).collect()
}

/// `count` copies turned about the axis line through `center`: a full turn
/// spaces them evenly, a partial turn puts the last one at `angle`.
///
/// Complexity: O(count).
pub fn radial_copies(count: u32, axis: Vec3, center: Vec3, angle: f32) -> Vec<Affine3A> {
    let full = (angle.abs() - std::f32::consts::TAU).abs() < 1e-4;
    let div = if full || count < 2 { count.max(1) } else { count - 1 };
    (0..count)
        .map(|i| {
            let r = Quat::from_axis_angle(axis.normalize(), angle * i as f32 / div as f32);
            Affine3A::from_translation(center) * Affine3A::from_quat(r) * Affine3A::from_translation(-center)
        })
        .collect()
}

/// `count` copies with their origins at points evenly spaced along a path.
/// With `align`, each copy's +X follows the path and its +Y leans toward
/// `up` on rotation-minimizing frames (no spinning along the path).
///
/// Complexity: O(path points + count).
pub fn path_copies(count: u32, path: &[Vec3], closed: bool, align: bool, up: Vec3) -> Result<Vec<Affine3A>, PrimError> {
    let pts = resample(path, closed, count as usize).map_err(|e| invalid("path", "the path", e.to_string()))?;
    if !align {
        return Ok(pts.iter().map(|p| Affine3A::from_translation(*p)).collect());
    }
    let dpts: Vec<DVec3> = pts.iter().map(|p| p.as_dvec3()).collect();
    let fr = frames(&dpts, closed, up.as_dvec3())?;
    Ok(fr
        .iter()
        .zip(&pts)
        .map(|(f, p)| {
            // +X along the path, +Y up, +Z = X x Y (the frame's right vector).
            let m = Mat3A::from_cols(Vec3A::from(f.t.as_vec3()), Vec3A::from(f.s.as_vec3()), Vec3A::from(f.r.as_vec3()));
            Affine3A { matrix3: m, translation: Vec3A::from(*p) }
        })
        .collect())
}

/// The boundary vertices of a mesh.
fn boundary_verts(mesh: &Mesh) -> Vec<u32> {
    mesh.verts().filter(|&v| mesh.is_boundary_vert(v)).collect()
}

/// Copies of a mesh, one per transform, joined into one mesh. With `weld`,
/// open-edge vertices of touching copies within that distance merge.
/// Returns the mesh and the number of vertices welded away.
///
/// Complexity: O(copies x mesh size) plus O(b log b) for b boundary
/// vertices when welding.
pub fn array(mesh: &Mesh, copies: &[Affine3A], weld: Option<f32>) -> Result<(Mesh, usize), PrimError> {
    if copies.is_empty() || copies.len() > MAX_COPIES as usize {
        return Err(invalid("count", copies.len(), format!("must be between 1 and {MAX_COPIES}")));
    }
    if mesh.face_count().saturating_mul(copies.len()) > MAX_GEN_FACES {
        return Err(invalid("count", copies.len(), format!("x {} faces would exceed {MAX_GEN_FACES} faces", mesh.face_count())));
    }
    let mut out = mesh.clone();
    out.transform(copies[0]);
    for &m in &copies[1..] {
        let mut c = mesh.clone();
        c.transform(m);
        out.append(&c);
    }
    let welded = weld_open(&mut out, weld)?;
    Ok((out, welded))
}

fn weld_open(mesh: &mut Mesh, weld: Option<f32>) -> Result<usize, PrimError> {
    let Some(d) = weld else { return Ok(0) };
    let open = boundary_verts(mesh);
    if open.is_empty() {
        return Ok(0);
    }
    let r = merge_by_distance(mesh, Some(&open), d).map_err(|e: KernelError| invalid("weld", d, e.to_string()))?;
    mesh.compact();
    Ok(r.merged)
}

/// Adds the mirror image of a mesh across the plane normal to `axis` at
/// `offset` along it, keeping the original; open-edge vertices within
/// `weld` of the plane merge with their mirror twins. Returns the welded
/// count.
///
/// Complexity: O(mesh size) plus O(b log b) for b boundary vertices.
pub fn mirror_copy(mesh: &mut Mesh, axis: Axis, offset: f32, weld: Option<f32>) -> Result<usize, PrimError> {
    let n = axis.unit();
    let m = Affine3A::from_translation(n * offset) * Affine3A::from_scale(Vec3::ONE - 2.0 * n) * Affine3A::from_translation(-n * offset);
    if mesh.face_count().saturating_mul(2) > MAX_GEN_FACES {
        return Err(invalid("target", mesh.face_count(), format!("doubling would exceed {MAX_GEN_FACES} faces")));
    }
    let mut c = mesh.clone();
    c.transform(m);
    mesh.append(&c);
    let Some(d) = weld else { return Ok(0) };
    let near: Vec<u32> = mesh.verts().filter(|&v| mesh.is_boundary_vert(v) && (mesh.positions[v as usize].dot(n) - offset).abs() <= d).collect();
    if near.is_empty() {
        return Ok(0);
    }
    let r = merge_by_distance(mesh, Some(&near), d).map_err(|e| invalid("weld", d, e.to_string()))?;
    mesh.compact();
    Ok(r.merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{Origin, PlaneSpec, plane};

    fn tile() -> Mesh {
        plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: 2, segments_z: 2 }).unwrap()
    }

    #[test]
    fn linear_copies_weld_into_one_strip() {
        let (m, welded) = array(&tile(), &linear_copies(3, Vec3::X), Some(1e-4)).unwrap();
        m.validate().unwrap();
        assert_eq!(welded, 2 * 3, "two seams of three vertices");
        assert_eq!(m.face_count(), 12);
        assert_eq!(m.vert_count(), 7 * 3);
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.x - lo.x - 3.0).abs() < 1e-6);
        let (loose, none) = array(&tile(), &linear_copies(3, Vec3::X * 2.0), Some(1e-4)).unwrap();
        assert_eq!((none, loose.vert_count()), (0, 27), "gaps stay apart");
    }

    #[test]
    fn radial_and_path_copies() {
        let ring = radial_copies(4, Vec3::Y, Vec3::ZERO, std::f32::consts::TAU);
        assert!(ring[1].transform_point3(Vec3::X).abs_diff_eq(Vec3::NEG_Z, 1e-6));
        let half = radial_copies(3, Vec3::Y, Vec3::ZERO, std::f32::consts::PI);
        assert!(half[2].transform_point3(Vec3::X).abs_diff_eq(Vec3::NEG_X, 1e-6), "a partial turn ends at the angle");
        let path = [Vec3::ZERO, Vec3::new(0.0, 0.0, -4.0)];
        let along = path_copies(5, &path, false, true, Vec3::Y).unwrap();
        assert!(along[4].translation.abs_diff_eq(Vec3A::new(0.0, 0.0, -4.0), 1e-5));
        // +X follows the path (-Z here), +Y stays up.
        assert!(along[2].transform_vector3(Vec3::X).abs_diff_eq(Vec3::NEG_Z, 1e-5));
        assert!(along[2].transform_vector3(Vec3::Y).abs_diff_eq(Vec3::Y, 1e-5));
        assert!(array(&tile(), &[], None).is_err());
    }

    #[test]
    fn mirror_copies_close_open_halves() {
        // Half a cylinder side (open) mirrored into a full ring.
        let mut half =
            crate::prim::cylinder(&crate::prim::CylinderSpec { cap: crate::prim::CapKind::None, ..Default::default() }, Origin::Center).unwrap();
        crate::edit::plane_cut(
            &mut half,
            &crate::edit::PlaneCutSpec { point: Vec3::ZERO, normal: Vec3::X, keep: crate::edit::Keep::Above, fill: false },
            None,
        )
        .unwrap();
        half.compact();
        let faces = half.face_count();
        let welded = mirror_copy(&mut half, Axis::X, 0.0, Some(1e-4)).unwrap();
        half.validate().unwrap();
        assert!(welded > 0);
        assert_eq!(half.face_count(), 2 * faces);
        assert_eq!(half.vert_count() as i64 - half.edge_count() as i64 + half.face_count() as i64, 0, "an open tube");
    }
}
