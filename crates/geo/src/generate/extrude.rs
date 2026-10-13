//! Extruded shapes: flat contours with holes turned into solids, with
//! optional round or chamfered rims.

use glam::{DVec2, DVec3, Vec3};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};

use super::shape::{fill, inset, orient};
use super::{MAX_GEN_FACES, invalid, strip};
use crate::edit::Axis;
use crate::prim::soup::{Soup, tag_groups_and_sharp};
use crate::prim::{MAX_BEVEL_SEGMENTS, PrimError};

/// Contour joints that turn more than this many degrees get sharp side edges.
pub const SHAPE_CORNER_DEG: f64 = 30.0;

const FRONT: u16 = 0;
const BACK: u16 = 1;
const SIDE: u16 = 2;
const BEVEL: u16 = 3;

/// Shape extrusion parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeSpec<'a> {
    /// Closed contours in the shape's plane; a contour inside an odd number
    /// of others is a hole. Either winding works.
    pub contours: &'a [Vec<DVec2>],
    /// Thickness: the back sits at 0 and the front at `depth` along the axis.
    pub depth: f64,
    /// Rim bevel width (0 for sharp rims).
    pub bevel: f64,
    /// Bevel segments: 1 is a chamfer, more make it round.
    pub bevel_segments: u32,
    pub bevel_front: bool,
    pub bevel_back: bool,
    /// Extrusion direction: the shape's (x, y) plane is turned to face it.
    pub axis: Axis,
}

/// `(inset, height)` of each bevel ring from the cap edge (`w`, 0) to the
/// side wall (0, `w`), on a quarter circle (a straight line for one segment).
fn bevel_rings(w: f64, m: u32) -> Vec<(f64, f64)> {
    (0..=m)
        .map(|i| {
            let t = std::f64::consts::FRAC_PI_2 * f64::from(i) / f64::from(m);
            (w * (1.0 - t.sin()), w * (1.0 - t.cos()))
        })
        .collect()
}

/// Extrudes closed contours into a solid. Groups: `front` (+Z before the
/// turn), `back`, `side`, and `bevel` when beveled. Caps are constrained
/// Delaunay fills merged into quads; side walls are quads. Cap rims (sharp
/// rims or chamfers) and side edges at contour corners sharper than
/// [`SHAPE_CORNER_DEG`] are sharp.
///
/// Complexity: O(n log n) in contour points for the caps, plus O(n x bevel
/// segments) for the walls.
pub fn extrude_shape(spec: &ShapeSpec) -> Result<Mesh, PrimError> {
    let (contours, _holes) = orient(spec.contours)?;
    let d = spec.depth;
    if !(d.is_finite() && d > 0.0) {
        return Err(invalid("depth", d, "must be above 0"));
    }
    let w = spec.bevel;
    let sides = u32::from(spec.bevel_front) + u32::from(spec.bevel_back);
    let beveled = w > 0.0 && sides > 0;
    if beveled {
        if !w.is_finite() || w * f64::from(sides) >= d {
            return Err(invalid("bevel", w, format!("must be less than the depth{}", if sides == 2 { " / 2" } else { "" })));
        }
        if !(1..=MAX_BEVEL_SEGMENTS).contains(&spec.bevel_segments) {
            return Err(invalid("bevel_segments", spec.bevel_segments, format!("must be between 1 and {MAX_BEVEL_SEGMENTS}")));
        }
    } else if w < 0.0 || !w.is_finite() {
        return Err(invalid("bevel", w, "must be 0 or more"));
    }
    let m = if beveled { spec.bevel_segments } else { 0 };
    let total: usize = contours.iter().map(Vec::len).sum();
    let rings_per = 2 + if beveled { (m as usize) * sides as usize } else { 0 };
    if total.saturating_mul(rings_per) > MAX_GEN_FACES {
        return Err(invalid("shape", format!("{total} points"), format!("would exceed {MAX_GEN_FACES} faces")));
    }

    // Rings from the back cap edge to the front cap edge: (inset, z).
    let profile = bevel_rings(w, m.max(1));
    let mut rings: Vec<(f64, f64)> = Vec::with_capacity(rings_per);
    if beveled && spec.bevel_back {
        rings.extend(profile.iter().copied());
    } else {
        rings.push((0.0, 0.0));
    }
    let side_strip = rings.len() - 1;
    if beveled && spec.bevel_front {
        rings.extend(profile.iter().rev().map(|&(i, h)| (i, d - h)));
    } else {
        rings.push((0.0, d));
    }

    // Inset contours per distinct inset value.
    let mut shapes: Vec<(f64, Vec<Vec<DVec2>>)> = Vec::new();
    for &(i, _) in &rings {
        if shapes.iter().any(|(x, _)| *x == i) {
            continue;
        }
        let set = if i == 0.0 {
            contours.clone()
        } else {
            contours
                .iter()
                .map(|c| inset(c, i))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| invalid("bevel", w, "is wider than the shape allows; use a smaller bevel"))?
        };
        shapes.push((i, set));
    }
    let shape_of = |i: f64| shapes.iter().find(|(x, _)| *x == i).map(|(_, s)| s);

    let mut soup = Soup::with_capacity(total * rings.len(), total * rings.len(), 4 * total * rings.len());
    // ids[ring][contour] = vertex ids around that contour.
    let mut ids: Vec<Vec<Vec<u32>>> = Vec::with_capacity(rings.len());
    for &(i, z) in &rings {
        let set = shape_of(i).ok_or_else(|| invalid("bevel", w, "internal: missing inset"))?;
        ids.push(set.iter().map(|c| c.iter().map(|p| soup.push_vert(DVec3::new(p.x, p.y, z))).collect()).collect());
    }
    for (r, pair) in ids.windows(2).enumerate() {
        let tag = if r == side_strip { SIDE } else { BEVEL };
        for (lower, upper) in pair[0].iter().zip(&pair[1]) {
            // Upper ring first: quads come out facing away from the solid.
            strip(&mut soup, upper, lower, true, tag);
        }
    }
    let concat = |ring: &[Vec<u32>]| -> Vec<u32> { ring.iter().flatten().copied().collect() };
    let last = rings.len() - 1;
    let front_ids = concat(&ids[last]);
    let back_ids = concat(&ids[0]);
    let front_polys = fill(shape_of(rings[last].0).ok_or_else(|| invalid("bevel", w, "internal: missing inset"))?)?;
    let back_polys =
        if rings[0].0 == rings[last].0 { front_polys.clone() } else { fill(shape_of(rings[0].0).ok_or_else(|| invalid("bevel", w, "internal"))?)? };
    for p in &front_polys {
        let v: Vec<u32> = p.iter().map(|&i| front_ids[i as usize]).collect();
        soup.poly(&v, FRONT);
    }
    for p in &back_polys {
        let v: Vec<u32> = p.iter().rev().map(|&i| back_ids[i as usize]).collect();
        soup.poly(&v, BACK);
    }

    let (mut mesh, tags) = soup.finish()?;
    let mut names: Vec<(&str, u16)> = vec![("front", FRONT), ("back", BACK), ("side", SIDE)];
    if beveled {
        names.push(("bevel", BEVEL));
    }
    for (name, tag) in names {
        let faces = tags.iter().enumerate().filter(|(_, t)| **t == tag).map(|(f, _)| f as u32);
        // Faces of a fresh mesh are all live, so this cannot fail.
        let _ = mesh.set_group(name, ElementKind::Face, faces);
    }
    let round = beveled && m > 1;
    tag_groups_and_sharp(&mut mesh, &tags, &[], |a, b| a != b && !(round && (a == BEVEL || b == BEVEL)));
    // Side and bevel edges at contour corners.
    let corner_cos = SHAPE_CORNER_DEG.to_radians().cos();
    for (c, pts) in contours.iter().enumerate() {
        let n = pts.len();
        for k in 0..n {
            let (a, b, q) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
            if (b - a).normalize().dot((q - b).normalize()) >= corner_cos {
                continue;
            }
            for r in 0..last {
                if let Some(h) = mesh.find_halfedge(ids[r][c][k], ids[r + 1][c][k]) {
                    let e = mesh.edge_of(h);
                    mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
                }
            }
        }
    }
    turn_z(&mut mesh, spec.axis);
    Ok(mesh)
}

/// Turns a mesh built along +Z so that +Z lands on `axis`, keeping the
/// shape plane's x and y as the curve planes `yz` (X), `xz` (Y) and `xy`
/// (Z) map them. A proper rotation, so winding is kept.
///
/// Complexity: O(vertices).
pub(crate) fn turn_z(mesh: &mut Mesh, axis: Axis) {
    let map: fn(Vec3) -> Vec3 = match axis {
        Axis::Z => return,
        // x -> X, y -> -Z, z -> Y.
        Axis::Y => |p| Vec3::new(p.x, p.z, -p.y),
        // x -> Y, y -> Z, z -> X.
        Axis::X => |p| Vec3::new(p.z, p.x, p.y),
    };
    for p in &mut mesh.positions {
        *p = map(*p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(w: f64, h: f64) -> Vec<DVec2> {
        vec![DVec2::new(-w / 2.0, -h / 2.0), DVec2::new(w / 2.0, -h / 2.0), DVec2::new(w / 2.0, h / 2.0), DVec2::new(-w / 2.0, h / 2.0)]
    }

    fn spec(contours: &[Vec<DVec2>]) -> ShapeSpec<'_> {
        ShapeSpec { contours, depth: 0.1, bevel: 0.0, bevel_segments: 1, bevel_front: true, bevel_back: true, axis: Axis::Z }
    }

    fn closed_outward(m: &Mesh) {
        m.validate().unwrap();
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "open edges");
        // Signed volume by the divergence theorem is positive for outward faces.
        let vol: f32 = m.faces().map(|f| m.face_normal(f).dot(m.face_centroid(f)) * m.face_area(f)).sum::<f32>() / 3.0;
        assert!(vol > 0.0, "faces point inward ({vol})");
    }

    #[test]
    fn frame_with_a_hole() {
        let contours = [rect(1.0, 0.8), rect(0.6, 0.4)];
        let m = extrude_shape(&spec(&contours)).unwrap();
        closed_outward(&m);
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 0, "genus 1");
        assert!(m.faces().all(|f| m.face_len(f) == 4), "quads only");
        let names: Vec<&str> = m.groups().map(|(n, _)| n).collect();
        assert_eq!(names, ["front", "back", "side"]);
        assert!(m.group("front").unwrap().iter().all(|f| m.face_normal(f).abs_diff_eq(Vec3::Z, 1e-5)));
        // 16 cap rim edges plus 8 corner edges.
        assert_eq!(m.edges().filter(|&e| m.edge_flags(e).contains(EdgeFlags::SHARP)).count(), 24);
        let vol: f32 = m.faces().map(|f| m.face_normal(f).dot(m.face_centroid(f)) * m.face_area(f)).sum::<f32>() / 3.0;
        assert!((vol - 0.1 * (0.8 - 0.24)).abs() < 1e-5, "{vol}");
    }

    #[test]
    fn bevels_round_or_chamfer_and_turn() {
        let contours = [rect(1.0, 0.5)];
        for (segs, axis) in [(1, Axis::Z), (3, Axis::Y), (2, Axis::X)] {
            let s = ShapeSpec { bevel: 0.02, bevel_segments: segs, axis, ..spec(&contours) };
            let m = extrude_shape(&s).unwrap();
            closed_outward(&m);
            assert_eq!(m.group("bevel").unwrap().count(), 2 * 4 * segs as usize);
            let (lo, hi) = m.bounds().unwrap();
            let size = hi - lo;
            let depth_axis = match axis {
                Axis::X => size.x,
                Axis::Y => size.y,
                Axis::Z => size.z,
            };
            assert!((depth_axis - 0.1).abs() < 1e-6, "{axis:?} {size}");
        }
        let only_front = ShapeSpec { bevel: 0.02, bevel_segments: 2, bevel_back: false, ..spec(&contours) };
        closed_outward(&extrude_shape(&only_front).unwrap());
        assert!(extrude_shape(&ShapeSpec { bevel: 0.06, ..spec(&contours) }).is_err(), "two bevels deeper than the depth");
        assert!(extrude_shape(&ShapeSpec { bevel: 0.03, depth: 1.0, ..spec(&[rect(0.05, 1.0)]) }).is_err(), "wider than the shape");
    }
}
