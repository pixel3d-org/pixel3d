//! Polygon accumulator used by the primitive generators.

use glam::{DVec3, Vec3};
use p3d_mesh::{BuildError, EdgeFlags, ElementKind, Mesh};
use rustc_hash::FxHashMap;

/// Collects vertices and polygons, then builds a [`Mesh`].
///
/// Generators compute every shared vertex from the same canonical inputs, so
/// shared vertices are bit-identical and [`Soup::vert`] welds them with an
/// exact hash lookup (no tolerance).
#[derive(Default)]
pub(crate) struct Soup {
    positions: Vec<Vec3>,
    weld: FxHashMap<[u32; 3], u32>,
    sizes: Vec<u32>,
    corners: Vec<u32>,
    tags: Vec<u16>,
}

#[inline]
fn bits(p: Vec3) -> [u32; 3] {
    // Fold -0.0 into 0.0 so both spellings of zero weld.
    p.to_array().map(|c| if c == 0.0 { 0 } else { c.to_bits() })
}

impl Soup {
    pub(crate) fn with_capacity(verts: usize, faces: usize, corners: usize) -> Self {
        Soup {
            positions: Vec::with_capacity(verts),
            weld: FxHashMap::default(),
            sizes: Vec::with_capacity(faces),
            corners: Vec::with_capacity(corners),
            tags: Vec::with_capacity(faces),
        }
    }

    /// Adds a vertex without welding.
    #[inline]
    pub(crate) fn push_vert(&mut self, p: DVec3) -> u32 {
        self.positions.push(p.as_vec3());
        (self.positions.len() - 1) as u32
    }

    /// Adds a vertex, or returns the existing one with the same position.
    ///
    /// Complexity: O(1) expected.
    #[inline]
    pub(crate) fn vert(&mut self, p: DVec3) -> u32 {
        let q = p.as_vec3();
        let next = self.positions.len() as u32;
        let id = *self.weld.entry(bits(q)).or_insert(next);
        if id == next {
            self.positions.push(q);
        }
        id
    }

    /// Adds a polygon, reversing it if its normal points away from `outward`.
    ///
    /// Complexity: O(polygon size).
    pub(crate) fn poly_facing(&mut self, verts: &[u32], outward: DVec3, tag: u16) {
        let mut n = Vec3::ZERO;
        for i in 0..verts.len() {
            let a = self.positions[verts[i] as usize];
            let b = self.positions[verts[(i + 1) % verts.len()] as usize];
            n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        self.sizes.push(verts.len() as u32);
        if f64::from(n.dot(outward.as_vec3())) < 0.0 {
            self.corners.extend(verts.iter().rev());
        } else {
            self.corners.extend_from_slice(verts);
        }
        self.tags.push(tag);
    }

    /// Builds the mesh; returns it with the tag of every face (face index =
    /// polygon order).
    ///
    /// Complexity: O(V + C) expected.
    pub(crate) fn finish(self) -> Result<(Mesh, Vec<u16>), BuildError> {
        let mesh = Mesh::from_polygons(self.positions, &self.sizes, &self.corners)?;
        Ok((mesh, self.tags))
    }
}

/// Creates one face group per named tag (in `names` order; `None` names are
/// skipped) and marks edges between faces whose tags `sharp` separates as
/// sharp.
///
/// Complexity: O(groups x F + E).
pub(crate) fn tag_groups_and_sharp(mesh: &mut Mesh, tags: &[u16], names: &[Option<&str>], sharp: impl Fn(u16, u16) -> bool) {
    for (t, name) in names.iter().enumerate() {
        if let Some(name) = name {
            let faces = tags.iter().enumerate().filter(|(_, x)| **x == t as u16).map(|(f, _)| f as u32);
            // Faces of a fresh mesh are all live, so this cannot fail.
            let _ = mesh.set_group(name, ElementKind::Face, faces);
        }
    }
    for e in mesh.edges().collect::<Vec<_>>() {
        if let [Some(a), Some(b)] = mesh.edge_faces(e)
            && sharp(tags[a as usize], tags[b as usize])
        {
            mesh.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
    }
}
