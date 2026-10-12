//! Catmull-Clark subdivision with semi-sharp creases (DeRose, Kass, Truong
//! 1998).

use glam::{Vec2, Vec3, Vec4};
use p3d_mesh::{EdgeFlags, ElementKind, Mesh};

use crate::edit::KernelError;

/// Subdivision settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubdSpec {
    pub levels: u32,
    /// Open-border vertices with only two edges (corners of a plane) stay put.
    pub keep_corners: bool,
    /// Treat edges marked sharp like fully creased edges.
    pub sharp_as_crease: bool,
}

impl Default for SubdSpec {
    fn default() -> Self {
        SubdSpec { levels: 1, keep_corners: true, sharp_as_crease: false }
    }
}

/// Largest number of levels in one call.
pub const MAX_SUBD_LEVELS: u32 = 6;

/// Largest number of faces a subdivision may produce.
pub const MAX_SUBD_FACES: usize = 8_000_000;

/// Faces after `levels` levels: each k-gon becomes k quads, then quads
/// become 4 each.
///
/// Complexity: O(F).
pub fn subdivided_face_count(mesh: &Mesh, levels: u32) -> usize {
    if levels == 0 {
        return mesh.face_count();
    }
    let corners: usize = mesh.faces().map(|f| mesh.face_len(f)).sum();
    corners.saturating_mul(4usize.saturating_pow(levels - 1))
}

/// Subdivides the whole mesh `levels` times. Edge crease weights in [0, 1]
/// map to sharpness 0 to 10 (each level lowers it by 1), open borders act as
/// infinitely sharp creases. Corner UVs and colors are interpolated
/// linearly per face; custom normals are dropped; materials, groups, sharp
/// and seam flags follow the faces and edges they came from.
///
/// Complexity: O(4^levels x (V + E + F + C)).
pub fn catmull_clark(mesh: &Mesh, spec: &SubdSpec) -> Result<Mesh, KernelError> {
    if !(1..=MAX_SUBD_LEVELS).contains(&spec.levels) {
        return Err(KernelError::OutOfRange {
            param: "levels",
            message: format!("{} levels is outside 1 to {MAX_SUBD_LEVELS}", spec.levels),
            hint: "1 or 2 levels are typical; each level makes 4 times the faces".into(),
        });
    }
    let n = subdivided_face_count(mesh, spec.levels);
    if n > MAX_SUBD_FACES {
        return Err(KernelError::OutOfRange {
            param: "levels",
            message: format!("{} levels would make {n} faces (more than {MAX_SUBD_FACES})", spec.levels),
            hint: "use fewer levels".into(),
        });
    }
    let mut m = mesh.clone();
    m.compact();
    for _ in 0..spec.levels {
        m = once(&m, spec)?;
    }
    Ok(m)
}

/// One level on a compacted mesh.
fn once(m: &Mesh, spec: &SubdSpec) -> Result<Mesh, KernelError> {
    let (nv, ne, nf) = (m.vert_slots(), m.edge_slots(), m.face_slots());
    let pos = |v: u32| m.positions[v as usize];
    // Sharpness per edge: crease x 10, infinite on the border (and on sharp edges if asked).
    let sharp: Vec<f32> = (0..ne as u32)
        .map(|e| {
            if m.is_boundary_edge(e) || (spec.sharp_as_crease && m.edge_flags(e).contains(EdgeFlags::SHARP)) {
                f32::INFINITY
            } else {
                m.edge_crease(e) * 10.0
            }
        })
        .collect();
    let face_pt: Vec<Vec3> = (0..nf as u32).map(|f| m.face_centroid(f)).collect();
    let edge_pt: Vec<Vec3> = (0..ne as u32)
        .map(|e| {
            let [a, b] = m.edge_verts(e);
            let mid = (pos(a) + pos(b)) * 0.5;
            let s = sharp[e as usize];
            if s >= 1.0 {
                return mid;
            }
            let [Some(f0), Some(f1)] = m.edge_faces(e) else { return mid };
            let smooth = (pos(a) + pos(b) + face_pt[f0 as usize] + face_pt[f1 as usize]) * 0.25;
            smooth.lerp(mid, s)
        })
        .collect();
    let vert_pt: Vec<Vec3> = (0..nv as u32)
        .map(|v| {
            let p = pos(v);
            let ring: Vec<u32> = m.vert_outgoing(v).collect();
            let n = ring.len() as f32;
            let creases: Vec<u32> = ring.iter().copied().filter(|&h| sharp[(h >> 1) as usize] > 0.0).collect();
            let boundary = m.is_boundary_vert(v);
            if boundary && spec.keep_corners && ring.len() == 2 {
                return p;
            }
            let smooth = || {
                let f = ring.iter().filter_map(|&h| m.face(h)).map(|f| face_pt[f as usize]).sum::<Vec3>() / n;
                let r = ring.iter().map(|&h| (p + pos(m.dest(h))) * 0.5).sum::<Vec3>() / n;
                (f + r * 2.0 + p * (n - 3.0)) / n
            };
            match creases.len() {
                0 | 1 => smooth(),
                2 => {
                    let crease = (pos(m.dest(creases[0])) + pos(m.dest(creases[1])) + p * 6.0) / 8.0;
                    let s = (sharp[(creases[0] >> 1) as usize] + sharp[(creases[1] >> 1) as usize]) * 0.5;
                    if s >= 1.0 { crease } else { smooth().lerp(crease, s) }
                }
                _ => {
                    let s = creases.iter().map(|&h| sharp[(h >> 1) as usize].min(1.0)).sum::<f32>() / creases.len() as f32;
                    if s >= 1.0 { p } else { smooth().lerp(p, s) }
                }
            }
        })
        .collect();

    // New vertices: vertex points, then edge points, then face points.
    let mut positions = vert_pt;
    positions.extend(edge_pt);
    positions.extend(face_pt);
    let (eo, fo) = (nv as u32, (nv + ne) as u32);
    let corners_total: usize = (0..nf as u32).map(|f| m.face_len(f)).sum();
    let mut sizes = Vec::with_capacity(corners_total);
    let mut corners = Vec::with_capacity(4 * corners_total);
    // Source corners for attributes: (corner, next corner, previous corner, face).
    let mut src: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(corners_total);
    for f in 0..nf as u32 {
        let hs: Vec<u32> = m.face_halfedges(f).collect();
        let k = hs.len();
        for i in 0..k {
            let (h, prev) = (hs[i], hs[(i + k - 1) % k]);
            sizes.push(4);
            corners.extend([m.origin(h), eo + (h >> 1), fo + f, eo + (prev >> 1)]);
            src.push((h, hs[(i + 1) % k], prev, f));
        }
    }
    let mut out = Mesh::from_polygons(positions, &sizes, &corners).map_err(|e| KernelError::OutOfRange {
        param: "levels",
        message: format!("subdivision produced an invalid mesh: {e}"),
        hint: "run cleanup.auto first".into(),
    })?;

    // Corner attributes, in polygon order: corner, edge middle, face middle, previous edge middle.
    let face_avg = |col: &[Vec2], f: u32| -> Vec2 { m.face_halfedges(f).map(|h| col[h as usize]).sum::<Vec2>() / m.face_len(f) as f32 };
    for col in &m.corner_uv {
        let values: Vec<Vec2> = src
            .iter()
            .flat_map(|&(h, next, prev, f)| {
                let c = col[h as usize];
                [c, (c + col[next as usize]) * 0.5, face_avg(col, f), (c + col[prev as usize]) * 0.5]
            })
            .collect();
        out.add_uv_layer_from_polygon_order(&values);
    }
    if let Some(col) = &m.corner_color {
        let avg = |f: u32| m.face_halfedges(f).map(|h| col[h as usize]).sum::<Vec4>() / m.face_len(f) as f32;
        let values: Vec<Vec4> = src
            .iter()
            .flat_map(|&(h, next, prev, f)| {
                let c = col[h as usize];
                [c, (c + col[next as usize]) * 0.5, avg(f), (c + col[prev as usize]) * 0.5]
            })
            .collect();
        let mut column = vec![Vec4::ONE; out.he_slots()];
        out.corner_values_from_polygon_order(&mut column, &values);
        out.corner_color = Some(column);
    }
    // Materials and face groups: child faces of face f are consecutive.
    let mut child_start = Vec::with_capacity(nf + 1);
    let mut acc = 0u32;
    for f in 0..nf as u32 {
        child_start.push(acc);
        acc += m.face_len(f) as u32;
    }
    child_start.push(acc);
    for f in 0..nf {
        for c in child_start[f]..child_start[f + 1] {
            out.face_material[c as usize] = m.face_material[f];
        }
    }
    // Edge flags, creases and edge groups on the two halves of every old edge.
    let halves = |out: &Mesh, e: u32| -> [Option<u32>; 2] {
        let [a, b] = m.edge_verts(e);
        [out.find_halfedge(a, eo + e).map(|h| h >> 1), out.find_halfedge(eo + e, b).map(|h| h >> 1)]
    };
    let any_crease = m.edge_crease.is_some();
    if any_crease {
        out.edge_crease = Some(vec![0.0; out.edge_slots()]);
    }
    for e in 0..ne as u32 {
        let fl = m.edge_flags(e);
        let child_crease = ((m.edge_crease(e) * 10.0 - 1.0).max(0.0) / 10.0).min(1.0);
        if fl == EdgeFlags::NONE && child_crease == 0.0 {
            continue;
        }
        for c in halves(&out, e).into_iter().flatten() {
            out.edge_flags[c as usize] = fl;
            if let Some(col) = &mut out.edge_crease {
                // A full crease (1.0) stays full; partial ones lose one level.
                col[c as usize] = if m.edge_crease(e) >= 1.0 { 1.0 } else { child_crease };
            }
        }
    }
    for (name, g) in m.groups() {
        let members: Vec<u32> = match g.kind() {
            ElementKind::Face => g.iter().flat_map(|f| child_start[f as usize]..child_start[f as usize + 1]).collect(),
            ElementKind::Vert => {
                // Vertex points, edge points of edges inside the group, face points of faces inside it.
                let mut v: Vec<u32> = g.iter().collect();
                v.extend((0..ne as u32).filter(|&e| m.edge_verts(e).iter().all(|&x| g.contains(x))).map(|e| eo + e));
                v.extend((0..nf as u32).filter(|&f| m.face_verts(f).all(|x| g.contains(x))).map(|f| fo + f));
                v
            }
            ElementKind::Edge => g.iter().flat_map(|e| halves(&out, e).into_iter().flatten().collect::<Vec<_>>()).collect(),
        };
        let _ = out.set_group(name, g.kind(), members);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    fn cube() -> Mesh {
        cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap()
    }

    #[test]
    fn cube_becomes_round() {
        let m = cube();
        let s = catmull_clark(&m, &SubdSpec { levels: 3, ..SubdSpec::default() }).unwrap();
        s.validate().unwrap();
        assert_eq!(s.face_count(), 6 * 64);
        assert!(s.faces().all(|f| s.face_len(f) == 4));
        // Pulled in from the cube's corners toward a sphere-like shape.
        let r: Vec<f32> = s.verts().map(|v| s.positions[v as usize].length()).collect();
        let (lo, hi) = (r.iter().fold(f32::MAX, |a, &b| a.min(b)), r.iter().fold(0.0f32, |a, &b| a.max(b)));
        assert!(hi < 0.866 && lo > 0.3 && hi / lo < 1.6, "{lo} {hi}");
        assert_eq!(s.group("top").unwrap().count(), 64);
        // Each level of a closed mesh keeps Euler 2.
        assert_eq!(s.vert_count() as i64 - s.edge_count() as i64 + s.face_count() as i64, 2);
    }

    #[test]
    fn full_creases_keep_the_box_sharp() {
        let mut m = cube();
        m.edge_crease = Some(vec![1.0; m.edge_slots()]);
        let s = catmull_clark(&m, &SubdSpec { levels: 2, ..SubdSpec::default() }).unwrap();
        s.validate().unwrap();
        let (lo, hi) = s.bounds().unwrap();
        assert!((hi - lo).abs_diff_eq(Vec3::ONE, 1e-5));
        // Every vertex stays on the cube's surface.
        assert!(s.verts().all(|v| s.positions[v as usize].abs().max_element() > 0.5 - 1e-5));
        // Crease 0.3 is sharpness 3: still sharp after two levels.
        let spread = |x: &Mesh| x.verts().map(|v| x.positions[v as usize].abs().max_element()).fold(0.0f32, f32::max);
        let crease = |w: f32| {
            let mut c = cube();
            c.edge_crease = Some(vec![w; c.edge_slots()]);
            catmull_clark(&c, &SubdSpec { levels: 2, ..SubdSpec::default() }).unwrap()
        };
        assert!((spread(&crease(0.3)) - 0.5).abs() < 1e-5);
        // Crease 0.05 (sharpness 0.5) softens: between round and sharp.
        let plain = catmull_clark(&cube(), &SubdSpec { levels: 2, ..SubdSpec::default() }).unwrap();
        let semi = spread(&crease(0.05));
        assert!(semi > spread(&plain) + 1e-3 && semi < 0.5 - 1e-3, "{} < {semi} < 0.5", spread(&plain));
    }

    #[test]
    fn open_planes_keep_their_corners() {
        let m = plane(&PlaneSpec { segments_x: 2, segments_z: 2, ..PlaneSpec::default() }).unwrap();
        let s = catmull_clark(&m, &SubdSpec { levels: 2, ..SubdSpec::default() }).unwrap();
        s.validate().unwrap();
        assert_eq!(s.face_count(), 64);
        let (lo, hi) = s.bounds().unwrap();
        assert!((hi.x - lo.x - 1.0).abs() < 1e-5, "corners kept, borders straight");
        let mut uv = m.clone();
        let n: usize = uv.faces().map(|f| uv.face_len(f)).sum();
        uv.add_uv_layer_from_polygon_order(&vec![Vec2::new(0.25, 0.75); n]);
        let s = catmull_clark(&uv, &SubdSpec::default()).unwrap();
        assert!(s.faces().all(|f| s.face_halfedges(f).all(|h| s.corner_uv(0, h) == Some(Vec2::new(0.25, 0.75)))));
    }

    #[test]
    fn limits() {
        assert!(catmull_clark(&cube(), &SubdSpec { levels: 0, ..SubdSpec::default() }).is_err());
        assert_eq!(subdivided_face_count(&cube(), 2), 96);
        let big = plane(&PlaneSpec { segments_x: 1000, segments_z: 1000, ..PlaneSpec::default() }).unwrap();
        assert!(catmull_clark(&big, &SubdSpec { levels: 2, ..SubdSpec::default() }).is_err(), "16M faces is too many");
    }
}
