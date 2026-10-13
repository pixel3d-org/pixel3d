//! Mesh cleanup: the "make it clean" pass. Welds seams, collapses
//! degenerate faces, removes specks, fills small holes, removes parts
//! hidden inside closed parts, turns parts outward and rejoins flat
//! triangle pairs into quads.
//!
//! A [`Mesh`] is manifold and consistently wound per part by construction,
//! so non-manifold splits and winding unification happen where polygon soup
//! becomes a mesh (import), not here.

use std::collections::BTreeSet;

use glam::{Affine3A, Vec3};
use p3d_mesh::{Mesh, Patch};

use crate::bvh::TriBvh;
use crate::edit::{MergeMode, QuadSpec, merge, merge_by_distance, tris_to_quads};

/// Faces with less area than this fraction of their longest edge squared
/// are degenerate (collinear corners).
pub const DEGENERATE_RATIO: f32 = 1e-6;
/// Vertices sampled per part for the hidden-part test.
const ENCLOSED_SAMPLES: usize = 8;
/// Weld passes at most (each pass only merges vertices that started close).
const WELD_ROUNDS: usize = 4;
/// Open parts with more faces keep their orientation (two rays per face).
pub const MAX_ORIENT_FACES: usize = 20_000;

/// Which fixes to run and their thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CleanupSpec {
    /// Weld vertices closer than this (0 skips welding).
    pub weld: f32,
    /// Collapse faces with (almost) no area.
    pub degenerate: bool,
    /// Remove parts whose bounding box diagonal is below this (0 keeps all).
    pub min_part_size: f32,
    /// Remove closed, outward parts sitting entirely inside another closed part.
    pub enclosed: bool,
    /// Turn closed parts (and open parts, by ray votes) outward.
    pub orient: bool,
    /// Fill holes of at most this many edges (0 fills none). A loop is only
    /// filled when its part has more faces than the loop has edges, so flat
    /// sheets stay open.
    pub max_hole_edges: u32,
    /// Join coplanar triangle pairs into quads (a limited dissolve that keeps quads).
    pub join_flat: bool,
}

impl Default for CleanupSpec {
    fn default() -> Self {
        CleanupSpec { weld: 1e-4, degenerate: true, min_part_size: 1e-4, enclosed: true, orient: true, max_hole_edges: 4, join_flat: true }
    }
}

/// What a cleanup changed (or, on a copy, would change).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub welded: usize,
    pub degenerate_collapsed: usize,
    /// Degenerate faces that could not be collapsed without breaking the mesh.
    pub degenerate_left: usize,
    pub specks_removed: usize,
    pub enclosed_removed: usize,
    pub parts_flipped: usize,
    pub holes_filled: usize,
    pub triangles_joined: usize,
    /// Fixes that were skipped, and why.
    pub warnings: Vec<String>,
}

impl CleanupReport {
    /// Total number of fixes.
    pub fn fixes(&self) -> usize {
        self.welded
            + self.degenerate_collapsed
            + self.specks_removed
            + self.enclosed_removed
            + self.parts_flipped
            + self.holes_filled
            + self.triangles_joined
    }
}

fn is_degenerate(mesh: &Mesh, f: u32) -> bool {
    let longest = mesh.face_halfedges(f).map(|h| mesh.edge_length(h >> 1)).fold(0.0f32, f32::max);
    longest <= f32::MIN_POSITIVE || mesh.face_area(f) <= DEGENERATE_RATIO * longest * longest
}

/// Faces of each connected part, parts in order of their lowest face.
///
/// Complexity: O(F + H).
fn parts(mesh: &Mesh) -> Vec<Vec<u32>> {
    let (labels, n) = mesh.face_components();
    let mut out = vec![Vec::new(); n as usize];
    for f in mesh.faces() {
        out[labels[f as usize] as usize].push(f);
    }
    out
}

/// Whether a part has no boundary edges, and its signed volume.
///
/// Complexity: O(corners of the part).
fn closed_volume(mesh: &Mesh, faces: &[u32]) -> (bool, f64) {
    let mut closed = true;
    let mut vol = 0.0f64;
    for &f in faces {
        for h in mesh.face_halfedges(f) {
            closed &= !mesh.is_boundary_he(h ^ 1);
        }
        let n = mesh.face_normal(f).as_dvec3();
        let c = mesh.face_centroid(f).as_dvec3();
        vol += n.dot(c) * f64::from(mesh.face_area(f)) / 3.0;
    }
    (closed, vol)
}

/// Deletes whole parts.
fn remove_faces(mesh: &mut Mesh, faces: &[u32]) -> Result<(), String> {
    let mut p = Patch::new(mesh);
    for &f in faces {
        p.remove_face(f);
    }
    mesh.apply_patch(&p).map(|_| ()).map_err(|e| e.to_string())
}

/// Runs the cleanup passes in order, best effort: a pass that cannot run
/// leaves the mesh as it was and adds a warning.
///
/// Complexity: O(n log n) overall: welding and the BVH are O(n log n),
/// part tests O(parts x samples x log n), hole and quad passes O(n log n).
pub fn cleanup(mesh: &mut Mesh, spec: &CleanupSpec) -> CleanupReport {
    let mut r = CleanupReport::default();
    if spec.weld > 0.0 {
        // Welds do not chain within one pass (a merged vertex moves to its
        // group's center and may reach another): repeat until it settles.
        for _ in 0..WELD_ROUNDS {
            let merged = match merge_by_distance(mesh, None, spec.weld) {
                Ok(m) => m.merged,
                Err(_) => {
                    // Welding everything would break the mesh; open seams usually still close.
                    let open: Vec<u32> = mesh.verts().filter(|&v| mesh.is_boundary_vert(v)).collect();
                    match merge_by_distance(mesh, Some(&open), spec.weld) {
                        Ok(m) => m.merged,
                        Err(e) => {
                            r.warnings.push(format!("weld skipped: {e}"));
                            0
                        }
                    }
                }
            };
            r.welded += merged;
            if merged == 0 {
                break;
            }
        }
    }
    if spec.degenerate {
        let bad: Vec<u32> = mesh.faces().filter(|&f| is_degenerate(mesh, f)).collect();
        for f in bad {
            if !mesh.is_face_alive(f) || !is_degenerate(mesh, f) {
                continue;
            }
            // Collapse the shortest edge when it is tiny: the face folds into its
            // neighbours and nothing moves more than half that edge. Slivers with
            // long edges are reported, not moved.
            let h = mesh.face_halfedges(f).min_by(|&a, &b| mesh.edge_length(a >> 1).total_cmp(&mesh.edge_length(b >> 1)).then(a.cmp(&b)));
            let longest = mesh.face_halfedges(f).map(|x| mesh.edge_length(x >> 1)).fold(0.0f32, f32::max);
            let pair = h.filter(|&h| mesh.edge_length(h >> 1) <= (spec.weld * 10.0).max(longest * 0.01)).map(|h| [mesh.origin(h), mesh.dest(h)]);
            let ok = pair.is_some_and(|pair| merge(mesh, &pair, MergeMode::Center).is_ok());
            if ok {
                r.degenerate_collapsed += 1;
            } else {
                r.degenerate_left += 1;
            }
        }
    }
    if spec.min_part_size > 0.0 {
        let specks: Vec<u32> = parts(mesh)
            .into_iter()
            .filter(|fs| {
                let (lo, hi) = fs.iter().flat_map(|&f| mesh.face_verts(f).collect::<Vec<_>>()).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
                    let p = mesh.positions[v as usize];
                    (lo.min(p), hi.max(p))
                });
                lo.distance(hi) < spec.min_part_size
            })
            .inspect(|_| r.specks_removed += 1)
            .flatten()
            .collect();
        if !specks.is_empty()
            && let Err(e) = remove_faces(mesh, &specks)
        {
            r.warnings.push(format!("specks kept: {e}"));
            r.specks_removed = 0;
        }
    }
    // Holes first: a part closed here is then oriented by its volume.
    if spec.max_hole_edges >= 3 {
        fill_small_holes(mesh, spec.max_hole_edges, &mut r);
    }
    if spec.join_flat {
        let tris: Vec<u32> = mesh.faces().filter(|&f| mesh.face_len(f) == 3).collect();
        if !tris.is_empty() {
            let q = QuadSpec { max_face_angle_deg: 1.0, max_shape_angle_deg: 40.0, ignore_boundaries: false };
            match tris_to_quads(mesh, &tris, &q) {
                Ok(quads) => r.triangles_joined = quads.len() * 2,
                Err(e) => r.warnings.push(format!("flat triangles kept: {e}")),
            }
        }
    }
    // Parts last: orientation sees the final faces, so a second pass agrees.
    if spec.enclosed || spec.orient {
        part_passes(mesh, spec, &mut r);
    }
    mesh.compact();
    r
}

/// Hidden parts and orientation, decided with ray parity against the other
/// parts.
fn part_passes(mesh: &mut Mesh, spec: &CleanupSpec, r: &mut CleanupReport) {
    let all = parts(mesh);
    if all.is_empty() {
        return;
    }
    let (labels, _) = mesh.face_components();
    let info: Vec<(bool, f64)> = all.iter().map(|fs| closed_volume(mesh, fs)).collect();
    let bvh = TriBvh::from_mesh(mesh, Affine3A::IDENTITY);
    let mut remove: Vec<u32> = Vec::new();
    let mut flip: BTreeSet<u32> = BTreeSet::new();
    for (i, fs) in all.iter().enumerate() {
        let (closed, vol) = info[i];
        if !closed {
            continue;
        }
        // Inside another closed part when every sampled vertex is.
        let verts: BTreeSet<u32> = fs.iter().flat_map(|&f| mesh.face_verts(f).collect::<Vec<_>>()).collect();
        let verts: Vec<u32> = verts.into_iter().collect();
        let step = (verts.len() / ENCLOSED_SAMPLES).max(1);
        let other_closed = |face: u32| {
            let l = labels[face as usize] as usize;
            l != i && info[l].0
        };
        let inside = verts.iter().step_by(step).all(|&v| bvh.contains_where(mesh.positions[v as usize], other_closed));
        match (inside, vol > 0.0) {
            (true, true) if spec.enclosed => remove.extend(fs),
            // A closed inward part inside another is a cavity (a hollow wall): keep it.
            (false, false) if spec.orient => {
                flip.insert(fs[0]);
            }
            _ => {}
        }
    }
    if spec.orient {
        for (i, fs) in all.iter().enumerate() {
            if info[i].0 {
                continue;
            }
            // Open parts: every face votes by area. A face whose front sees out
            // and back sees in says outward; the reverse says inward. Area weights
            // make the call independent of how the part is cut into faces.
            if fs.len() > MAX_ORIENT_FACES {
                r.warnings.push(format!("open part at face {} has over {MAX_ORIENT_FACES} faces; its orientation was kept", fs[0]));
                continue;
            }
            let (mut outward, mut inward, mut total) = (0.0f64, 0.0f64, 0.0f64);
            for &f in fs {
                let (n, c, area) = (mesh.face_normal(f), mesh.face_centroid(f), mesh.face_area(f));
                let eps = area.sqrt().max(1e-6) * 1e-3;
                let front = bvh.raycast(c + n * eps, n, f32::INFINITY).is_none();
                let back = bvh.raycast(c - n * eps, -n, f32::INFINITY).is_none();
                let a = f64::from(area);
                total += a;
                if front && !back {
                    outward += a;
                } else if back && !front {
                    inward += a;
                }
            }
            // Only a clear call turns an open part: at least a quarter of its area
            // says inward, three times more than says outward. Ambiguous parts
            // stay as they are.
            if inward > 0.0 && 4.0 * inward >= total && inward >= 3.0 * outward {
                flip.insert(fs[0]);
            }
        }
    }
    for f in flip {
        match mesh.flip_part(f) {
            Ok(_) => r.parts_flipped += 1,
            Err(e) => r.warnings.push(format!("part at face {f} not turned: {e}")),
        }
    }
    if !remove.is_empty() {
        let n = all.iter().filter(|fs| remove.contains(&fs[0])).count();
        match remove_faces(mesh, &remove) {
            Ok(()) => r.enclosed_removed = n,
            Err(e) => r.warnings.push(format!("hidden parts kept: {e}")),
        }
    }
}

/// Fills boundary loops of at most `max` edges on parts with more faces
/// than the loop has edges.
fn fill_small_holes(mesh: &mut Mesh, max: u32, r: &mut CleanupReport) {
    let (labels, n) = mesh.face_components();
    let mut part_faces = vec![0usize; n as usize];
    for f in mesh.faces() {
        part_faces[labels[f as usize] as usize] += 1;
    }
    let mut seen = vec![false; mesh.he_slots()];
    let mut holes = Vec::new();
    for h in 0..mesh.he_slots() as u32 {
        if seen[h as usize] || !mesh.is_he_alive(h) || !mesh.is_boundary_he(h) {
            continue;
        }
        let Ok(lp) = mesh.boundary_loop(h) else { continue };
        for &x in &lp {
            seen[x as usize] = true;
        }
        let part = mesh.face(h ^ 1).map(|f| labels[f as usize] as usize);
        if (3..=max as usize).contains(&lp.len()) && part.is_some_and(|p| part_faces[p] > lp.len()) {
            holes.push(h);
        }
    }
    for h in holes {
        match mesh.fill_boundary_loop(h) {
            Ok(_) => r.holes_filled += 1,
            Err(e) => r.warnings.push(format!("hole not filled: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    fn cube(size: f64, at: Vec3) -> Mesh {
        let mut m = cuboid(&BoxSpec { size: DVec3::splat(size), segments: [1; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center).unwrap();
        m.transform(Affine3A::from_translation(at));
        m
    }

    #[test]
    fn removes_hidden_parts_and_specks_and_flips_inward_parts() {
        // A small cube buried in the wall of a hollow box (between the cavity and the outside).
        let mut m = cube(1.0, Vec3::ZERO);
        m.append(&cube(0.1, Vec3::new(0.4, 0.0, 0.0)));
        // A speck far away and an inward cube outside.
        m.append(&cube(0.004, Vec3::splat(5.0)));
        let mut inward = cube(0.5, Vec3::new(3.0, 0.0, 0.0));
        inward.flip_all();
        m.append(&inward);
        // A hollow: an inward cube inside the big one is a cavity and stays.
        let mut cavity = cube(0.6, Vec3::ZERO);
        cavity.flip_all();
        m.append(&cavity);
        let spec = CleanupSpec { min_part_size: 0.01, ..CleanupSpec::default() };
        let r = cleanup(&mut m, &spec);
        m.validate().unwrap();
        assert_eq!((r.enclosed_removed, r.specks_removed, r.parts_flipped), (1, 1, 1), "{r:?}");
        assert_eq!(m.face_count(), 6 * 3);
        let again = cleanup(&mut m, &spec);
        assert_eq!(again.fixes(), 0, "a second pass finds nothing: {again:?}");
    }

    #[test]
    fn welds_seams_fills_small_holes_and_joins_flat_triangles() {
        // Two halves of a box, unwelded, with the lid missing.
        let mut m = cube(1.0, Vec3::ZERO);
        let top: Vec<u32> = m.faces().filter(|&f| m.face_normal(f).y > 0.9).collect();
        remove_faces(&mut m, &top).unwrap();
        let r = cleanup(&mut m, &CleanupSpec::default());
        assert_eq!(r.holes_filled, 1, "{r:?}");
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        // A single quad stays open (a sheet, not a hole).
        let mut sheet = plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: 1, segments_z: 1 }).unwrap();
        assert_eq!(cleanup(&mut sheet, &CleanupSpec::default()).holes_filled, 0);
        // A triangulated grid becomes quads again; duplicated seam vertices weld.
        let mut grid = plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: 2, segments_z: 2 }).unwrap();
        let faces: Vec<u32> = grid.faces().collect();
        crate::edit::triangulate(&mut grid, &faces).unwrap();
        let mut other = grid.clone();
        other.transform(Affine3A::from_translation(Vec3::X));
        grid.append(&other);
        let r = cleanup(&mut grid, &CleanupSpec::default());
        grid.validate().unwrap();
        assert_eq!(r.welded, 3, "{r:?}");
        assert_eq!(r.triangles_joined, 16);
        assert!(grid.faces().all(|f| grid.face_len(f) == 4));
    }

    #[test]
    fn collapses_degenerate_faces_with_a_tiny_edge() {
        // A quad with a flat flap on its bottom edge: triangle (1, 0, 4) has no area and a 0.5 mm edge.
        let pts = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y, Vec3::new(0.9995, 0.0, 0.0)];
        let mut m = Mesh::from_polygons(pts.clone(), &[4, 3], &[0, 1, 2, 3, 1, 0, 4]).unwrap();
        let r = cleanup(&mut m, &CleanupSpec { join_flat: false, ..CleanupSpec::default() });
        m.validate().unwrap();
        assert_eq!((r.degenerate_collapsed, m.face_count()), (1, 1), "{r:?}");
        // The same flap with the extra point in the middle has long edges: reported, not moved.
        let mut pts = pts;
        pts[4] = Vec3::new(0.5, 0.0, 0.0);
        let mut m = Mesh::from_polygons(pts, &[4, 3], &[0, 1, 2, 3, 1, 0, 4]).unwrap();
        let r = cleanup(&mut m, &CleanupSpec { join_flat: false, ..CleanupSpec::default() });
        assert_eq!((r.degenerate_collapsed, r.degenerate_left), (0, 1), "{r:?}");
    }
}
