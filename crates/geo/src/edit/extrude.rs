//! Extrude faces: move them out along their normals (or a direction) and
//! connect them to where they were with side walls.

use glam::Vec3;
use p3d_mesh::{CornerSrc, Mesh};

use super::{
    AUTO_SHARP_DEG, KernelError, corner_edge_flags, corners_normal, face_bits, live_faces, patch, region_fans, restore_edge_flags, sharpen_by_angle,
};

/// How a set of faces is extruded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExtrudeMode {
    /// Connected faces move together; walls only along the region's border.
    #[default]
    Region,
    /// Every face moves on its own, with walls all around it.
    Individual,
}

/// Extrusion parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExtrudeSpec {
    pub mode: ExtrudeMode,
    /// Offset in meters; negative pushes the faces in.
    pub distance: f32,
    /// Move along this direction instead of the normals.
    pub direction: Option<Vec3>,
}

impl Default for ExtrudeSpec {
    fn default() -> Self {
        ExtrudeSpec { mode: ExtrudeMode::Region, distance: 0.1, direction: None }
    }
}

/// Faces an extrusion made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtrudeResult {
    /// The moved faces (they keep the groups and materials of the originals).
    pub cap: Vec<u32>,
    /// The new walls.
    pub side: Vec<u32>,
}

/// Largest stretch of the offset at creases (even thickness), as a multiple
/// of the distance.
const MAX_STRETCH: f32 = 4.0;

/// Extrudes faces. In region mode a vertex gets one copy per fan of
/// selected faces around it, moved along the fan's mean normal and stretched
/// so that every face of the fan moves by the full distance (even
/// thickness); in individual mode each face moves along its own normal. With
/// a direction every copy moves by `direction * distance`.
///
/// Caps keep the groups, materials, corner attributes and edge flags of the
/// faces they replace; walls take the material of their face. New edges
/// where faces meet at more than [`AUTO_SHARP_DEG`] become sharp.
///
/// Complexity: O(c log c + valences of the region's vertices) for the c
/// corners of the selected faces.
pub fn extrude(mesh: &mut Mesh, faces: &[u32], spec: &ExtrudeSpec) -> Result<ExtrudeResult, KernelError> {
    let faces = live_faces(mesh, faces)?;
    if !spec.distance.is_finite() || spec.distance.abs() < 1e-7 {
        return Err(KernelError::OutOfRange {
            param: "distance",
            message: "extrusion distance must not be 0".into(),
            hint: "use a distance such as 0.05 (50 mm), or a negative one to push in".into(),
        });
    }
    let dir = match spec.direction {
        Some(d) if d.is_finite() && d.length_squared() > 1e-12 => Some(d.normalize()),
        Some(_) => {
            return Err(KernelError::OutOfRange {
                param: "direction",
                message: "direction must be a non-zero vector".into(),
                hint: "for example [0, 1, 0] for up".into(),
            });
        }
        None => None,
    };
    let source_flags = corner_edge_flags(mesh, &faces);
    let mut p = patch(mesh);
    let mut side_like: Vec<u32> = Vec::new();
    // Copy of each corner (half-edge of a selected face).
    let mut copy_of: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
    match spec.mode {
        ExtrudeMode::Region => {
            let region = face_bits(mesh, &faces);
            let fans = region_fans(mesh, &region, &faces);
            let copies: Vec<u32> = fans
                .corners
                .iter()
                .map(|fan| {
                    let v = mesh.origin(fan[0]);
                    let offset = match dir {
                        Some(d) => d * spec.distance,
                        None => {
                            let n = corners_normal(mesh, fan);
                            let min_cos = fan.iter().filter_map(|&h| mesh.face(h)).map(|f| mesh.face_normal(f).dot(n)).fold(1.0f32, f32::min);
                            n * spec.distance / min_cos.max(1.0 / MAX_STRETCH)
                        }
                    };
                    p.new_vert(mesh.positions[v as usize] + offset, Some(v))
                })
                .collect();
            for (&h, &fan) in &fans.of_corner {
                copy_of.insert(h, copies[fan as usize]);
            }
            for &f in &faces {
                p.remove_face(f);
            }
            for &f in &faces {
                let vs: Vec<u32> = mesh.face_halfedges(f).map(|h| copy_of[&h]).collect();
                let src: Vec<CornerSrc> = mesh.face_halfedges(f).map(CornerSrc::copy).collect();
                p.add_face(&vs, &src, mesh.face_material(f), Some(f));
            }
            for &f in &faces {
                for h in mesh.face_halfedges(f) {
                    if !mesh.face(h ^ 1).is_some_and(|g| region.contains(g as usize)) {
                        add_wall(mesh, &mut p, h, copy_of[&h], copy_of[&mesh.next(h)]);
                        side_like.push(f);
                    }
                }
            }
        }
        ExtrudeMode::Individual => {
            for &f in &faces {
                let offset = dir.unwrap_or_else(|| mesh.face_normal(f)) * spec.distance;
                for h in mesh.face_halfedges(f) {
                    let v = mesh.origin(h);
                    copy_of.insert(h, p.new_vert(mesh.positions[v as usize] + offset, Some(v)));
                }
                p.remove_face(f);
            }
            for &f in &faces {
                let vs: Vec<u32> = mesh.face_halfedges(f).map(|h| copy_of[&h]).collect();
                let src: Vec<CornerSrc> = mesh.face_halfedges(f).map(CornerSrc::copy).collect();
                p.add_face(&vs, &src, mesh.face_material(f), Some(f));
            }
            for &f in &faces {
                for h in mesh.face_halfedges(f) {
                    add_wall(mesh, &mut p, h, copy_of[&h], copy_of[&mesh.next(h)]);
                    side_like.push(f);
                }
            }
        }
    }
    let r = mesh.apply_patch(&p)?;
    let (cap, side) = r.faces.split_at(faces.len());
    restore_edge_flags(mesh, cap, &source_flags);
    sharpen_by_angle(mesh, side, AUTO_SHARP_DEG);
    Ok(ExtrudeResult { cap: cap.to_vec(), side: side.to_vec() })
}

/// The wall along corner `h` (u to w) of an extruded face, from the old edge
/// up to its copy (u' to w'): u, w, w', u'. Corners copy the face's corners.
fn add_wall(mesh: &Mesh, p: &mut p3d_mesh::Patch, h: u32, u_copy: u32, w_copy: u32) {
    let (u, w) = (mesh.origin(h), mesh.dest(h));
    let n = mesh.next(h);
    let src = [CornerSrc::copy(h), CornerSrc::copy(n), CornerSrc::copy(n), CornerSrc::copy(h)];
    let material = mesh.face(h).map_or(0, |f| mesh.face_material(f));
    p.add_face(&[u, w, w_copy, u_copy], &src, material, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;
    use p3d_mesh::{EdgeFlags, ElementKind};

    fn euler(m: &Mesh) -> i64 {
        m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
    }

    fn unit_box() -> Mesh {
        cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Bottom).unwrap()
    }

    #[test]
    fn extruding_a_box_top_raises_it() {
        let mut m = unit_box();
        let top: Vec<u32> = m.group("top").unwrap().iter().collect();
        let r = extrude(&mut m, &top, &ExtrudeSpec { distance: 0.5, ..ExtrudeSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.cap.len(), r.side.len()), (1, 4));
        assert_eq!((m.vert_count(), m.face_count(), euler(&m)), (12, 10, 2));
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi.y - 1.5).abs() < 1e-6 && lo.y.abs() < 1e-6);
        // The cap stays in `top`; walls face outward.
        assert!(m.group("top").unwrap().contains(r.cap[0]));
        assert!(m.face_normal(r.cap[0]).abs_diff_eq(Vec3::Y, 1e-6));
        for &f in &r.side {
            let c = m.face_centroid(f);
            assert!(m.face_normal(f).dot(Vec3::new(c.x, 0.0, c.z)) > 0.0);
            assert!((m.face_area(f) - 0.5).abs() < 1e-6);
        }
        // Every edge of the raised block is sharp (all meet at 90 degrees).
        assert!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count() >= 20);
    }

    #[test]
    fn region_extrusion_of_an_l_keeps_even_thickness() {
        let mut m = unit_box();
        let faces: Vec<u32> = m.group("top").unwrap().iter().chain(m.group("right").unwrap().iter()).collect();
        let r = extrude(&mut m, &faces, &ExtrudeSpec { distance: 0.1, ..ExtrudeSpec::default() }).unwrap();
        m.validate().unwrap();
        assert_eq!(euler(&m), 2);
        let (_, hi) = m.bounds().unwrap();
        assert!((hi.x - 0.6).abs() < 1e-5 && (hi.y - 1.1).abs() < 1e-5, "{hi}");
        assert_eq!(r.side.len(), 6, "walls along the six border edges of the L");
    }

    #[test]
    fn individual_extrusion_and_direction() {
        let mut m = plane(&PlaneSpec { size_x: 2.0, size_z: 1.0, segments_x: 2, segments_z: 1 }).unwrap();
        let all: Vec<u32> = m.faces().collect();
        let r = extrude(&mut m, &all, &ExtrudeSpec { mode: ExtrudeMode::Individual, distance: 0.2, direction: None }).unwrap();
        m.validate().unwrap();
        assert_eq!((r.cap.len(), r.side.len()), (2, 8));
        // Two separate blocks touching along the middle wall pair.
        assert_eq!(m.vert_count(), 6 + 8);
        let mut m = plane(&PlaneSpec::default()).unwrap();
        let r = extrude(&mut m, &[0], &ExtrudeSpec { direction: Some(Vec3::new(0.0, 0.0, 2.0)), distance: 0.3, ..ExtrudeSpec::default() }).unwrap();
        m.validate().unwrap();
        let c = m.face_centroid(r.cap[0]);
        assert!(c.abs_diff_eq(Vec3::new(0.0, 0.0, 0.3), 1e-6), "{c}");
    }

    #[test]
    fn open_border_and_pushing_in() {
        let mut m = plane(&PlaneSpec { segments_x: 3, segments_z: 3, ..PlaneSpec::default() }).unwrap();
        m.set_group("hole", ElementKind::Face, [4]).unwrap();
        let r = extrude(&mut m, &[4], &ExtrudeSpec { distance: -0.1, ..ExtrudeSpec::default() }).unwrap();
        m.validate().unwrap();
        assert!(m.face_centroid(r.cap[0]).y < -0.099);
        assert!(m.group("hole").unwrap().contains(r.cap[0]));
        // A border face of an open grid: its open edge gets a wall too.
        let r = extrude(&mut m, &[0], &ExtrudeSpec::default()).unwrap();
        m.validate().unwrap();
        assert_eq!(r.side.len(), 4);
    }

    #[test]
    fn regions_touching_at_a_vertex_split_cleanly() {
        let mut m = plane(&PlaneSpec { segments_x: 2, segments_z: 2, ..PlaneSpec::default() }).unwrap();
        // Faces 0 and 3 touch only at the center vertex.
        let r = extrude(&mut m, &[0, 3], &ExtrudeSpec::default()).unwrap();
        m.validate().unwrap();
        assert_eq!(r.side.len(), 8);
    }

    #[test]
    fn errors() {
        let mut m = unit_box();
        assert!(matches!(extrude(&mut m, &[0], &ExtrudeSpec { distance: 0.0, ..ExtrudeSpec::default() }), Err(KernelError::OutOfRange { .. })));
        assert!(matches!(extrude(&mut m, &[], &ExtrudeSpec::default()), Err(KernelError::Empty(_))));
        assert!(matches!(extrude(&mut m, &[77], &ExtrudeSpec::default()), Err(KernelError::Topology(_))));
        let d = ExtrudeSpec { direction: Some(Vec3::ZERO), ..ExtrudeSpec::default() };
        assert!(matches!(extrude(&mut m, &[0], &d), Err(KernelError::OutOfRange { param: "direction", .. })));
    }
}
