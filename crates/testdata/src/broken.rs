//! Intentionally broken polygon soups. The kernel must reject each with the
//! listed error; importers must repair them (cleanup, Phase 3).

use glam::Vec3;
use p3d_mesh::BuildError;

/// Polygons given as face sizes plus a flat corner list.
#[derive(Debug, Clone, PartialEq)]
pub struct Soup {
    pub name: &'static str,
    pub positions: Vec<Vec3>,
    pub face_sizes: Vec<u32>,
    pub corners: Vec<u32>,
}

fn square_pair() -> Vec<Vec3> {
    vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z, Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 1.0), Vec3::new(1.0, 1.0, 0.5)]
}

/// Three quads sharing one edge.
pub fn non_manifold_edge() -> Soup {
    Soup { name: "non_manifold_edge", positions: square_pair(), face_sizes: vec![4, 4, 3], corners: vec![0, 3, 2, 1, 1, 2, 5, 4, 1, 6, 2] }
}

/// Two triangles touching at one vertex only.
pub fn bowtie() -> Soup {
    let p = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::NEG_X, Vec3::NEG_Z];
    Soup { name: "bowtie", positions: p, face_sizes: vec![3, 3], corners: vec![0, 2, 1, 0, 4, 3] }
}

/// Two quads with opposite winding across their shared edge.
pub fn flipped_face() -> Soup {
    Soup { name: "flipped_face", positions: square_pair()[..6].to_vec(), face_sizes: vec![4, 4], corners: vec![0, 3, 2, 1, 1, 4, 5, 2] }
}

/// A face that uses one vertex twice.
pub fn repeated_vertex() -> Soup {
    Soup { name: "repeated_vertex", positions: square_pair()[..4].to_vec(), face_sizes: vec![4], corners: vec![0, 1, 0, 3] }
}

/// A vertex no face uses.
pub fn loose_vertex() -> Soup {
    Soup { name: "loose_vertex", positions: square_pair()[..5].to_vec(), face_sizes: vec![4], corners: vec![0, 3, 2, 1] }
}

/// Checks that an error is the one a broken case must produce.
pub type Expect = fn(&BuildError) -> bool;

/// Every broken case with a check for the error the kernel must return.
pub fn all() -> Vec<(Soup, Expect)> {
    vec![
        (non_manifold_edge(), |e| matches!(e, BuildError::NonManifoldEdge { .. } | BuildError::InconsistentWinding { .. })),
        (bowtie(), |e| matches!(e, BuildError::NonManifoldVertex { vert: 0 })),
        (flipped_face(), |e| matches!(e, BuildError::InconsistentWinding { .. })),
        (repeated_vertex(), |e| matches!(e, BuildError::RepeatedVertex { .. })),
        (loose_vertex(), |e| matches!(e, BuildError::UnreferencedVertex { vert: 4 })),
    ]
}

#[cfg(test)]
mod tests {
    use p3d_mesh::Mesh;

    #[test]
    fn every_broken_soup_is_rejected_with_its_error() {
        for (soup, expected) in super::all() {
            let err = Mesh::from_polygons(soup.positions, &soup.face_sizes, &soup.corners).expect_err(soup.name);
            assert!(expected(&err), "{}: unexpected {err}", soup.name);
        }
    }
}
