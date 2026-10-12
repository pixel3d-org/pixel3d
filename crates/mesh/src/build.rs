//! Building a half-edge mesh from polygons.

use glam::Vec3;
use rustc_hash::FxHashMap;

use crate::mesh::{INVALID, Mesh};

/// Why a polygon list cannot become a 2-manifold half-edge mesh.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    #[error("the face sizes add up to {expected} corners but {got} corner vertices were given")]
    CornerCountMismatch { expected: usize, got: usize },
    #[error("face {face} has {size} corners; a face needs at least 3")]
    FaceTooSmall { face: u32, size: u32 },
    #[error("face {face} references vertex {vert}, but there are only {count} vertices")]
    VertexOutOfRange { face: u32, vert: u32, count: u32 },
    #[error("face {face} uses vertex {vert} more than once")]
    RepeatedVertex { face: u32, vert: u32 },
    #[error("edge {a}-{b} is used by more than two faces (non-manifold edge)")]
    NonManifoldEdge { a: u32, b: u32 },
    #[error("faces {face} and {other} have opposite winding across edge {a}-{b}")]
    InconsistentWinding { a: u32, b: u32, face: u32, other: u32 },
    #[error("vertex {vert} joins separate fans of faces (non-manifold vertex)")]
    NonManifoldVertex { vert: u32 },
    #[error("vertex {vert} is not used by any face")]
    UnreferencedVertex { vert: u32 },
    #[error("vertex {vert} has a non-finite position")]
    NonFinitePosition { vert: u32 },
    #[error("the mesh exceeds the u32 index range")]
    TooLarge,
}

impl Mesh {
    /// Builds a mesh from polygons given as face sizes and a flat list of
    /// corner vertices (the layout of glTF-like and WIT mesh buffers).
    /// Corner `i` of face `f` becomes the half-edge from that corner's
    /// vertex to the next one, and `face_he(f)` is the first corner, so
    /// per-corner data can be copied in input order.
    ///
    /// Fails on non-manifold or inconsistently wound input, unused vertices
    /// and non-finite positions.
    ///
    /// Complexity: O(V + C) expected in vertices and corners (one hash map
    /// lookup per corner).
    pub fn from_polygons(positions: Vec<Vec3>, face_sizes: &[u32], corner_verts: &[u32]) -> Result<Mesh, BuildError> {
        let corners: usize = face_sizes.iter().map(|&s| s as usize).sum();
        if corners != corner_verts.len() {
            return Err(BuildError::CornerCountMismatch { expected: corners, got: corner_verts.len() });
        }
        // Half-edges are at most 2 per corner; keep INVALID out of range.
        if positions.len() >= INVALID as usize || face_sizes.len() >= INVALID as usize || 2 * corners >= INVALID as usize {
            return Err(BuildError::TooLarge);
        }
        let nv = positions.len() as u32;
        if let Some(v) = positions.iter().position(|p| !p.is_finite()) {
            return Err(BuildError::NonFinitePosition { vert: v as u32 });
        }

        let mut mesh = Mesh {
            he_next: Vec::with_capacity(corners + corners / 4),
            he_prev: Vec::new(),
            he_vert: Vec::with_capacity(corners + corners / 4),
            he_face: Vec::with_capacity(corners + corners / 4),
            vert_he: vec![INVALID; positions.len()],
            face_he: Vec::with_capacity(face_sizes.len()),
            positions,
            ..Mesh::default()
        };
        let mut edge_map: FxHashMap<u64, u32> = FxHashMap::default();
        edge_map.reserve(corners / 2 + 16);
        // Last face that used each vertex: O(1) repeated-vertex check.
        let mut seen_in: Vec<u32> = vec![INVALID; nv as usize];
        let mut corner_he: Vec<u32> = Vec::with_capacity(corners);

        let mut start = 0usize;
        for (f, &size) in face_sizes.iter().enumerate() {
            let f = f as u32;
            if size < 3 {
                return Err(BuildError::FaceTooSmall { face: f, size });
            }
            let face = &corner_verts[start..start + size as usize];
            for &v in face {
                if v >= nv {
                    return Err(BuildError::VertexOutOfRange { face: f, vert: v, count: nv });
                }
                if seen_in[v as usize] == f {
                    return Err(BuildError::RepeatedVertex { face: f, vert: v });
                }
                seen_in[v as usize] = f;
            }
            for i in 0..face.len() {
                let a = face[i];
                let b = face[(i + 1) % face.len()];
                let key = (u64::from(a.min(b)) << 32) | u64::from(a.max(b));
                let h = match edge_map.get(&key) {
                    None => {
                        let e = (mesh.he_vert.len() / 2) as u32;
                        edge_map.insert(key, e);
                        mesh.he_vert.extend_from_slice(&[a, b]);
                        mesh.he_face.extend_from_slice(&[f, INVALID]);
                        2 * e
                    }
                    Some(&e) => {
                        let (h0, h1) = (2 * e as usize, 2 * e as usize + 1);
                        if mesh.he_vert[h0] == a {
                            return Err(BuildError::InconsistentWinding { a, b, face: f, other: mesh.he_face[h0] });
                        }
                        if mesh.he_face[h1] != INVALID {
                            return Err(BuildError::NonManifoldEdge { a, b });
                        }
                        mesh.he_face[h1] = f;
                        h1 as u32
                    }
                };
                if mesh.vert_he[a as usize] == INVALID {
                    mesh.vert_he[a as usize] = h;
                }
                corner_he.push(h);
            }
            start += size as usize;
        }

        let nh = mesh.he_vert.len();
        mesh.he_next = vec![INVALID; nh];
        mesh.he_prev = vec![INVALID; nh];
        let mut start = 0usize;
        for &size in face_sizes {
            let hs = &corner_he[start..start + size as usize];
            for i in 0..hs.len() {
                let (h, n) = (hs[i], hs[(i + 1) % hs.len()]);
                mesh.he_next[h as usize] = n;
                mesh.he_prev[n as usize] = h;
            }
            mesh.face_he.push(hs[0]);
            start += size as usize;
        }

        link_boundary(&mut mesh)?;

        if let Some(v) = mesh.vert_he.iter().position(|&h| h == INVALID) {
            return Err(BuildError::UnreferencedVertex { vert: v as u32 });
        }
        check_single_fans(&mesh)?;
        Ok(mesh)
    }
}

/// Links boundary half-edges into loops and makes each boundary vertex's
/// `vert_he` its outgoing boundary half-edge.
///
/// Complexity: O(V + H).
fn link_boundary(mesh: &mut Mesh) -> Result<(), BuildError> {
    let mut boundary_out = vec![INVALID; mesh.vert_slots()];
    for h in 0..mesh.he_slots() as u32 {
        if mesh.he_face[h as usize] == INVALID {
            let u = mesh.he_vert[h as usize];
            if boundary_out[u as usize] != INVALID {
                return Err(BuildError::NonManifoldVertex { vert: u });
            }
            boundary_out[u as usize] = h;
        }
    }
    for h in 0..mesh.he_slots() as u32 {
        if mesh.he_face[h as usize] == INVALID {
            let w = mesh.he_vert[(h ^ 1) as usize];
            // Boundary in-degree equals boundary out-degree at every vertex,
            // so a vertex with an incoming boundary half-edge has an outgoing one.
            let n = boundary_out[w as usize];
            if n == INVALID {
                return Err(BuildError::NonManifoldVertex { vert: w });
            }
            mesh.he_next[h as usize] = n;
            mesh.he_prev[n as usize] = h;
        }
    }
    for (v, &h) in boundary_out.iter().enumerate() {
        if h != INVALID {
            mesh.vert_he[v] = h;
        }
    }
    Ok(())
}

/// Fails if any vertex has more than one fan of faces (for example two cones
/// touching at their tips): the rotation from `vert_he` must reach every
/// outgoing half-edge.
///
/// Complexity: O(V + H).
fn check_single_fans(mesh: &Mesh) -> Result<(), BuildError> {
    let mut degree = vec![0u32; mesh.vert_slots()];
    for &v in &mesh.he_vert {
        degree[v as usize] += 1;
    }
    for v in 0..mesh.vert_slots() as u32 {
        if mesh.vert_outgoing(v).count() != degree[v as usize] as usize {
            return Err(BuildError::NonManifoldVertex { vert: v });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        Mesh::from_polygons(p, &[4], &[0, 1, 2, 3]).unwrap()
    }

    /// Unit cube with outward winding (6 quads, 8 verts, 12 edges).
    pub(crate) fn cube() -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    #[test]
    fn single_quad() {
        let m = quad();
        assert_eq!((m.vert_count(), m.edge_count(), m.face_count()), (4, 4, 1));
        assert_eq!(m.face_verts(0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!((0..4).all(|v| m.is_boundary_vert(v)));
        assert!(m.edges().all(|e| m.is_boundary_edge(e)));
        assert_eq!(m.triangle_count(), 2);
        assert!(m.face_normal(0).abs_diff_eq(Vec3::Z, 1e-6));
        assert!((m.face_area(0) - 1.0).abs() < 1e-6);
        m.validate().unwrap();
    }

    #[test]
    fn closed_cube() {
        let m = cube();
        assert_eq!((m.vert_count(), m.edge_count(), m.face_count()), (8, 12, 6));
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)));
        assert!(m.verts().all(|v| m.valence(v) == 3));
        // Euler characteristic of a sphere.
        assert_eq!(m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64, 2);
        // Outward normals: the face through vertices 0, 2, 3, 1 lies at z = 0.
        assert!(m.face_normal(0).abs_diff_eq(Vec3::NEG_Z, 1e-6));
        m.validate().unwrap();
    }

    #[test]
    fn neighbors_and_lookup() {
        let m = cube();
        let mut n: Vec<u32> = m.vert_neighbors(0).collect();
        n.sort_unstable();
        assert_eq!(n, vec![1, 2, 4]);
        let h = m.find_halfedge(0, 1).unwrap();
        assert_eq!((m.origin(h), m.dest(h)), (0, 1));
        assert!(m.find_halfedge(0, 7).is_none());
        assert_eq!(m.vert_faces(0).count(), 3);
    }

    #[test]
    fn rejects_bad_input() {
        let p = || vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE];
        assert!(matches!(Mesh::from_polygons(p(), &[3], &[0, 1]), Err(BuildError::CornerCountMismatch { .. })));
        assert!(matches!(Mesh::from_polygons(p(), &[2], &[0, 1]), Err(BuildError::FaceTooSmall { .. })));
        assert!(matches!(Mesh::from_polygons(p(), &[3], &[0, 1, 9]), Err(BuildError::VertexOutOfRange { .. })));
        assert!(matches!(Mesh::from_polygons(p(), &[4], &[0, 1, 0, 2]), Err(BuildError::RepeatedVertex { .. })));
        // Two triangles wound the same way across edge 0-1.
        assert!(matches!(Mesh::from_polygons(p(), &[3, 3], &[0, 1, 2, 0, 1, 3]), Err(BuildError::InconsistentWinding { .. })));
        // Three triangles on edge 0-1.
        let r = Mesh::from_polygons(p(), &[3, 3, 3], &[0, 1, 2, 1, 0, 3, 1, 0, 4]);
        assert!(matches!(r, Err(BuildError::NonManifoldEdge { .. }) | Err(BuildError::InconsistentWinding { .. })), "{r:?}");
        assert!(matches!(Mesh::from_polygons(p(), &[3], &[0, 1, 2]), Err(BuildError::UnreferencedVertex { vert: 3 })));
        let mut bad = p();
        bad[1].x = f32::NAN;
        assert!(matches!(Mesh::from_polygons(bad, &[3], &[0, 1, 2]), Err(BuildError::NonFinitePosition { vert: 1 })));
    }

    #[test]
    fn rejects_bowtie_vertex() {
        // Two triangles sharing only vertex 0.
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::NEG_X, Vec3::NEG_Y];
        let r = Mesh::from_polygons(p, &[3, 3], &[0, 1, 2, 0, 3, 4]);
        assert_eq!(r, Err(BuildError::NonManifoldVertex { vert: 0 }));
    }

    #[test]
    fn rejects_two_closed_fans_at_one_vertex() {
        // Two tetrahedra glued at vertex 0 only.
        let mut p = vec![Vec3::ZERO];
        p.extend([Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 1.0), Vec3::NEG_X, Vec3::NEG_Y, Vec3::new(-1.0, -1.0, -1.0)]);
        let tet = |a: u32, b: u32, c: u32| [0, a, b, 0, b, c, 0, c, a, a, c, b];
        let mut faces = tet(1, 2, 3).to_vec();
        faces.extend(tet(4, 5, 6));
        let r = Mesh::from_polygons(p, &[3; 8], &faces);
        assert_eq!(r, Err(BuildError::NonManifoldVertex { vert: 0 }));
    }
}
