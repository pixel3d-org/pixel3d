//! Flat shapes bounded by closed contours: nesting (holes and islands),
//! constrained Delaunay fills merged into quads, and miter insets.

use std::collections::VecDeque;

use glam::{DVec2, Vec3};
use p3d_mesh::Mesh;
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};

use super::invalid;
use crate::edit::{QuadSpec, tris_to_quads};
use crate::prim::PrimError;

/// Most contours in one shape (nesting is quadratic in contours, linear in points).
pub const MAX_CONTOURS: usize = 1024;

/// Twice the signed area (positive when counter-clockwise).
///
/// Complexity: O(points).
pub(crate) fn area2(pts: &[DVec2]) -> f64 {
    (0..pts.len()).map(|i| pts[i].perp_dot(pts[(i + 1) % pts.len()])).sum()
}

/// Whether `p` is inside the closed polygon (even-odd rule).
///
/// Complexity: O(points).
fn contains(poly: &[DVec2], p: DVec2) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

/// Validates contours and orients them so the solid lies on the left of
/// every contour: outer boundaries counter-clockwise, holes clockwise (a
/// contour inside an odd number of others is a hole). Returns the oriented
/// contours and whether each one is a hole.
///
/// Complexity: O(contours x points).
pub(crate) fn orient(contours: &[Vec<DVec2>]) -> Result<(Vec<Vec<DVec2>>, Vec<bool>), PrimError> {
    if contours.is_empty() || contours.len() > MAX_CONTOURS {
        return Err(invalid("shape", format!("{} contours", contours.len()), format!("needs 1 to {MAX_CONTOURS} closed contours")));
    }
    for (i, c) in contours.iter().enumerate() {
        if c.len() < 3 || c.iter().any(|p| !p.is_finite()) {
            return Err(invalid("shape", format!("contour {i}"), "needs at least 3 finite points"));
        }
        if area2(c).abs() <= 1e-12 {
            return Err(invalid("shape", format!("contour {i}"), "encloses no area"));
        }
    }
    let mut out = Vec::with_capacity(contours.len());
    let mut holes = Vec::with_capacity(contours.len());
    for (i, c) in contours.iter().enumerate() {
        let depth = contours.iter().enumerate().filter(|(j, o)| *j != i && contains(o, c[0])).count();
        let hole = depth % 2 == 1;
        let ccw = area2(c) > 0.0;
        let mut c = c.clone();
        if ccw == hole {
            c.reverse();
        }
        out.push(c);
        holes.push(hole);
    }
    Ok((out, holes))
}

/// Fills the area the contours enclose (even-odd) with a constrained
/// Delaunay triangulation, then joins triangle pairs into quads where the
/// quad is convex and near-rectangular. Polygons index the contours' points
/// concatenated in order and run counter-clockwise.
///
/// Complexity: O(n log n) in contour points.
pub(crate) fn fill(contours: &[Vec<DVec2>]) -> Result<Vec<Vec<u32>>, PrimError> {
    let total: usize = contours.iter().map(Vec::len).sum();
    let mut verts = Vec::with_capacity(total);
    let mut edges = Vec::with_capacity(total);
    for c in contours {
        let base = verts.len();
        verts.extend(c.iter().map(|p| Point2::new(p.x, p.y)));
        edges.extend((0..c.len()).map(|i| [base + i, base + (i + 1) % c.len()]));
    }
    let mut crossing = false;
    let cdt = ConstrainedDelaunayTriangulation::<Point2<f64>>::try_bulk_load_cdt(verts, edges, |_| crossing = true)
        .map_err(|e| invalid("shape", "a point", format!("cannot be triangulated: {e}")))?;
    if crossing {
        return Err(invalid("shape", "contours", "cross each other or themselves"));
    }
    // A point on another contour's edge splits that constraint: the contours touch.
    if cdt.num_vertices() != total || cdt.num_constraints() != total {
        return Err(invalid("shape", "contours", "touch or repeat a point; keep them apart"));
    }

    // Inside faces: crossing a constraint edge flips inside and outside,
    // starting outside at the convex hull.
    let mut depth = vec![u32::MAX; cdt.num_all_faces()];
    let mut queue = VecDeque::new();
    for e in cdt.convex_hull() {
        if let Some(f) = e.rev().face().as_inner() {
            let d = u32::from(cdt.is_constraint_edge(e.fix().as_undirected()));
            if depth[f.fix().index()] == u32::MAX {
                depth[f.fix().index()] = d;
                queue.push_back(f.fix());
            }
        }
    }
    while let Some(fh) = queue.pop_front() {
        let d = depth[fh.index()];
        for e in cdt.face(fh).adjacent_edges() {
            let Some(nb) = e.rev().face().as_inner() else { continue };
            let i = nb.fix().index();
            if depth[i] == u32::MAX {
                depth[i] = d + u32::from(cdt.is_constraint_edge(e.fix().as_undirected()));
                queue.push_back(nb.fix());
            }
        }
    }
    let tris: Vec<[u32; 3]> =
        cdt.inner_faces().filter(|f| depth[f.fix().index()] % 2 == 1).map(|f| f.vertices().map(|v| v.fix().index() as u32)).collect();
    if tris.is_empty() {
        return Err(invalid("shape", "contours", "enclose nothing"));
    }

    // Join pairs into quads on a flat temporary mesh (vertex ids are kept).
    let positions: Vec<Vec3> = contours.iter().flatten().map(|p| Vec3::new(p.x as f32, p.y as f32, 0.0)).collect();
    let corners: Vec<u32> = tris.iter().flatten().copied().collect();
    let mut mesh = Mesh::from_polygons(positions, &vec![3; tris.len()], &corners)?;
    let faces: Vec<u32> = mesh.faces().collect();
    // Flat caps tolerate rougher quads than curved surfaces (trapezoids between nested contours).
    let spec = QuadSpec { max_face_angle_deg: 1.0, max_shape_angle_deg: 60.0, ignore_boundaries: false };
    tris_to_quads(&mut mesh, &faces, &spec).map_err(|e| invalid("shape", "fill", e.to_string()))?;
    Ok(mesh.faces().map(|f| mesh.face_verts(f).collect()).collect())
}

/// Moves a closed contour sideways by `d` toward its left (into the solid
/// for oriented contours), with miter joins so the point count stays.
/// `None` when an edge would flip (the inset is wider than the shape
/// allows there).
///
/// Complexity: O(points).
pub(crate) fn inset(pts: &[DVec2], d: f64) -> Option<Vec<DVec2>> {
    let n = pts.len();
    let left = |i: usize| (pts[(i + 1) % n] - pts[i]).normalize_or_zero().perp();
    let out: Vec<DVec2> = (0..n)
        .map(|i| {
            let (la, lb) = (left((i + n - 1) % n), left(i));
            let m = (la + lb).normalize_or_zero();
            let cos = m.dot(la);
            if cos > 1e-6 { pts[i] + m * (d / cos) } else { pts[i] + la * d }
        })
        .collect();
    let flipped = (0..n).any(|i| (out[(i + 1) % n] - out[i]).dot(pts[(i + 1) % n] - pts[i]) <= 0.0);
    (!flipped).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: DVec2, h: f64) -> Vec<DVec2> {
        vec![c + DVec2::new(-h, -h), c + DVec2::new(h, -h), c + DVec2::new(h, h), c + DVec2::new(-h, h)]
    }

    fn area_of(polys: &[Vec<u32>], pts: &[DVec2]) -> f64 {
        polys.iter().map(|p| area2(&p.iter().map(|&i| pts[i as usize]).collect::<Vec<_>>()) / 2.0).sum()
    }

    #[test]
    fn holes_and_islands_follow_even_odd() {
        let contours = vec![square(DVec2::ZERO, 1.0), square(DVec2::ZERO, 0.5), square(DVec2::ZERO, 0.2)];
        let (oriented, holes) = orient(&contours).unwrap();
        assert_eq!(holes, [false, true, false]);
        assert!(area2(&oriented[0]) > 0.0 && area2(&oriented[1]) < 0.0 && area2(&oriented[2]) > 0.0);
        let polys = fill(&oriented).unwrap();
        let pts: Vec<DVec2> = oriented.iter().flatten().copied().collect();
        // Area of the ring plus the island, every polygon counter-clockwise.
        assert!((area_of(&polys, &pts) - (4.0 - 1.0 + 0.16)).abs() < 1e-9);
        assert!(polys.iter().all(|p| area2(&p.iter().map(|&i| pts[i as usize]).collect::<Vec<_>>()) > 0.0));
        // A square ring fills with quads only.
        assert!(polys.iter().all(|p| p.len() == 4), "{polys:?}");
    }

    #[test]
    fn crossing_touching_and_flat_contours_fail() {
        let a = square(DVec2::ZERO, 1.0);
        let b = square(DVec2::new(1.0, 0.5), 1.0);
        assert!(fill(&[a.clone(), b]).is_err(), "crossing contours");
        let overlap = square(DVec2::new(1.0, 0.0), 1.0);
        assert!(fill(&[a.clone(), overlap]).is_err(), "contours along each other");
        let bow = vec![DVec2::ZERO, DVec2::new(1.0, 1.0), DVec2::new(1.0, 0.0), DVec2::new(0.0, 1.0)];
        assert!(fill(&[bow]).is_err(), "a contour crossing itself");
        let touch = vec![a[0], DVec2::new(0.0, -2.0), DVec2::new(2.0, -2.0)];
        assert!(fill(&[a.clone(), touch]).is_err(), "contours sharing a point");
        assert!(orient(&[vec![DVec2::ZERO, DVec2::X, DVec2::new(2.0, 0.0)]]).is_err());
    }

    #[test]
    fn insets_keep_counts_and_detect_overlap() {
        let (c, _) = orient(&[square(DVec2::ZERO, 1.0)]).unwrap();
        let i = inset(&c[0], 0.25).unwrap();
        assert!(i.iter().all(|p| (p.x.abs() - 0.75).abs() < 1e-12 && (p.y.abs() - 0.75).abs() < 1e-12));
        assert!(inset(&c[0], 1.2).is_none(), "wider than half the square");
    }
}
