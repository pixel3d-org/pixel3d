//! Polygons under construction with every per-corner column a tree needs:
//! UVs into the bark or leaf atlas, wind data, vertex colors and custom
//! normals, plus face groups, vertex groups and material slots.

use glam::{Vec2, Vec3, Vec4};
use p3d_mesh::{ElementKind, Mesh};

use crate::VegError;

/// One corner of a face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Corner {
    pub v: u32,
    /// UV layer 0 (bark or leaf atlas).
    pub uv: Vec2,
    /// UV layer 1: wind phase and stiffness.
    pub wind: Vec2,
    /// Vertex color: tint (rgb) and wind branch level (a).
    pub color: Vec4,
    /// Custom normal; zero takes the smooth normal of the surface.
    pub normal: Vec3,
}

#[derive(Debug, Default)]
pub(crate) struct Builder {
    pub pos: Vec<Vec3>,
    sizes: Vec<u32>,
    corners: Vec<Corner>,
    material: Vec<u16>,
    /// Face groups by registration order, with their faces in creation
    /// order.
    groups: Vec<(String, Vec<u32>)>,
    vert_groups: Vec<(String, Vec<u32>)>,
}

impl Builder {
    /// A face group's index, registering it on first use.
    ///
    /// Complexity: O(groups).
    pub fn group(&mut self, name: &str) -> usize {
        match self.groups.iter().position(|(n, _)| n == name) {
            Some(i) => i,
            None => {
                self.groups.push((name.to_owned(), Vec::new()));
                self.groups.len() - 1
            }
        }
    }

    /// Adds a vertex to a vertex group.
    ///
    /// Complexity: O(vertex groups).
    pub fn vert_group(&mut self, name: &str, v: u32) {
        match self.vert_groups.iter_mut().find(|(n, _)| n == name) {
            Some((_, list)) => list.push(v),
            None => self.vert_groups.push((name.to_owned(), vec![v])),
        }
    }

    pub fn vert(&mut self, p: Vec3) -> u32 {
        self.pos.push(p);
        (self.pos.len() - 1) as u32
    }

    /// Triangles of the faces built so far.
    ///
    /// Complexity: O(faces).
    pub fn triangles(&self) -> usize {
        self.sizes.iter().map(|&s| s.saturating_sub(2) as usize).sum()
    }

    /// Adds a face in the groups `groups` (indices from [`Builder::group`]).
    ///
    /// Complexity: O(corners + groups given).
    pub fn face(&mut self, corners: &[Corner], groups: &[usize], material: u16) {
        let f = self.sizes.len() as u32;
        for &g in groups {
            if let Some((_, list)) = self.groups.get_mut(g) {
                list.push(f);
            }
        }
        self.sizes.push(corners.len() as u32);
        self.corners.extend_from_slice(corners);
        self.material.push(material);
    }

    /// The mesh with its groups, UV layers 0 and 1, vertex colors, custom
    /// normals (smooth ones where none was given) and material slots.
    ///
    /// Complexity: O(V + F + C + groups).
    pub fn finish(self) -> Result<Mesh, VegError> {
        let verts: Vec<u32> = self.corners.iter().map(|c| c.v).collect();
        let mut m = Mesh::from_polygons(self.pos, &self.sizes, &verts).map_err(|e| VegError::new("output", e.to_string()))?;
        for (name, faces) in self.groups {
            if !faces.is_empty() {
                m.set_group(&name, ElementKind::Face, faces).map_err(|e| VegError::new("output", e.to_string()))?;
            }
        }
        for (name, verts) in self.vert_groups {
            m.set_group(&name, ElementKind::Vert, verts).map_err(|e| VegError::new("output", e.to_string()))?;
        }
        let uv: Vec<Vec2> = self.corners.iter().map(|c| c.uv).collect();
        let wind: Vec<Vec2> = self.corners.iter().map(|c| c.wind).collect();
        if m.add_uv_layer_from_polygon_order(&uv).is_none() || m.add_uv_layer_from_polygon_order(&wind).is_none() {
            return Err(VegError::new("output", "corner columns do not match the faces"));
        }
        let mut color = vec![Vec4::ONE; m.he_slots()];
        let colors: Vec<Vec4> = self.corners.iter().map(|c| c.color).collect();
        m.corner_values_from_polygon_order(&mut color, &colors).ok_or_else(|| VegError::new("output", "colors do not match the faces"))?;
        let smooth = m.compute_corner_normals();
        let mut normal = vec![Vec3::ZERO; m.he_slots()];
        let given: Vec<Vec3> = self.corners.iter().map(|c| c.normal).collect();
        m.corner_values_from_polygon_order(&mut normal, &given).ok_or_else(|| VegError::new("output", "normals do not match the faces"))?;
        for (n, s) in normal.iter_mut().zip(&smooth) {
            if *n == Vec3::ZERO {
                *n = *s;
            }
        }
        m.corner_color = Some(color);
        m.corner_normal = Some(normal);
        for (f, mat) in self.material.iter().enumerate() {
            if let Some(slot) = m.face_material.get_mut(f) {
                *slot = *mat;
            }
        }
        m.revision += 1;
        Ok(m)
    }
}
