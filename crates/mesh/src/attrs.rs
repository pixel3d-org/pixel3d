//! Attribute columns: per corner (half-edge), per face and per edge.

use glam::{Vec2, Vec3, Vec4};

use crate::mesh::Mesh;

/// Per-edge flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EdgeFlags(u8);

impl EdgeFlags {
    pub const NONE: EdgeFlags = EdgeFlags(0);
    /// UV seam: UV islands are cut along this edge.
    pub const SEAM: EdgeFlags = EdgeFlags(1);
    /// Hard edge: normals are split across it.
    pub const SHARP: EdgeFlags = EdgeFlags(1 << 1);

    #[inline]
    pub fn contains(self, other: EdgeFlags) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    pub fn insert(&mut self, other: EdgeFlags) {
        self.0 |= other.0;
    }

    #[inline]
    pub fn remove(&mut self, other: EdgeFlags) {
        self.0 &= !other.0;
    }

    #[inline]
    pub fn set(&mut self, other: EdgeFlags, on: bool) {
        if on { self.insert(other) } else { self.remove(other) }
    }

    #[inline]
    pub fn bits(self) -> u8 {
        self.0
    }

    #[inline]
    pub fn from_bits(bits: u8) -> EdgeFlags {
        EdgeFlags(bits)
    }
}

/// Maximum number of UV layers (UV0 for textures, UV1 for lightmaps).
pub const MAX_UV_LAYERS: usize = 2;

impl Mesh {
    /// Number of UV layers.
    pub fn uv_layer_count(&self) -> usize {
        self.corner_uv.len()
    }

    /// Adds a UV layer filled with zeros and returns its index, or `None`
    /// when [`MAX_UV_LAYERS`] layers exist already.
    ///
    /// Complexity: O(half-edges).
    pub fn add_uv_layer(&mut self) -> Option<usize> {
        if self.corner_uv.len() >= MAX_UV_LAYERS {
            return None;
        }
        self.corner_uv.push(vec![Vec2::ZERO; self.he_slots()]);
        self.revision += 1;
        Some(self.corner_uv.len() - 1)
    }

    /// UV of a corner (half-edge `h`, at its origin vertex) in a layer.
    #[inline]
    pub fn corner_uv(&self, layer: usize, h: u32) -> Option<Vec2> {
        self.corner_uv.get(layer).map(|l| l[h as usize])
    }

    /// Sets per-corner values of a column from a list in polygon order:
    /// live faces in index order, corners from `face_he(f)` in winding order
    /// (the corner order given to [`Mesh::from_polygons`]).
    ///
    /// Returns the number of corners, or `None` when `values` has another
    /// length (the column is unchanged then).
    ///
    /// Complexity: O(F + C).
    pub fn corner_values_from_polygon_order<T: Copy>(&self, column: &mut [T], values: &[T]) -> Option<usize> {
        let corners: usize = self.faces().map(|f| self.face_len(f)).sum();
        if corners != values.len() || column.len() != self.he_slots() {
            return None;
        }
        let mut i = 0;
        for f in self.faces() {
            for h in self.face_halfedges(f) {
                column[h as usize] = values[i];
                i += 1;
            }
        }
        Some(corners)
    }

    /// Adds a UV layer from per-corner UVs in polygon order (see
    /// [`Mesh::corner_values_from_polygon_order`]). Returns the layer index.
    ///
    /// Complexity: O(F + C).
    pub fn add_uv_layer_from_polygon_order(&mut self, uvs: &[Vec2]) -> Option<usize> {
        if self.corner_uv.len() >= MAX_UV_LAYERS {
            return None;
        }
        let mut column = vec![Vec2::ZERO; self.he_slots()];
        self.corner_values_from_polygon_order(&mut column, uvs)?;
        self.corner_uv.push(column);
        self.revision += 1;
        Some(self.corner_uv.len() - 1)
    }

    /// Custom split normal of a corner, if custom normals are set.
    #[inline]
    pub fn corner_normal(&self, h: u32) -> Option<Vec3> {
        self.corner_normal.as_ref().map(|n| n[h as usize])
    }

    /// Vertex color of a corner, if colors are set.
    #[inline]
    pub fn corner_color(&self, h: u32) -> Option<Vec4> {
        self.corner_color.as_ref().map(|c| c[h as usize])
    }

    /// Material slot of a face.
    #[inline]
    pub fn face_material(&self, f: u32) -> u16 {
        self.face_material[f as usize]
    }

    /// Flags of an edge.
    #[inline]
    pub fn edge_flags(&self, e: u32) -> EdgeFlags {
        self.edge_flags[e as usize]
    }

    /// Sets or clears a flag on an edge.
    #[inline]
    pub fn set_edge_flag(&mut self, e: u32, flag: EdgeFlags, on: bool) {
        self.edge_flags[e as usize].set(flag, on);
        self.revision += 1;
    }

    /// Crease weight of an edge in [0, 1] (0 when no creases are set).
    #[inline]
    pub fn edge_crease(&self, e: u32) -> f32 {
        self.edge_crease.as_ref().map_or(0.0, |c| c[e as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_tris() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        Mesh::from_polygons(p, &[3, 3], &[0, 1, 2, 2, 1, 3]).unwrap()
    }

    #[test]
    fn flags() {
        let mut f = EdgeFlags::NONE;
        f.insert(EdgeFlags::SHARP);
        assert!(f.contains(EdgeFlags::SHARP) && !f.contains(EdgeFlags::SEAM));
        f.set(EdgeFlags::SEAM, true);
        f.remove(EdgeFlags::SHARP);
        assert_eq!(f, EdgeFlags::SEAM);
        assert_eq!(EdgeFlags::from_bits(f.bits()), f);
    }

    #[test]
    fn default_columns() {
        let m = two_tris();
        assert_eq!(m.uv_layer_count(), 0);
        assert_eq!(m.face_material(1), 0);
        assert_eq!(m.edge_flags(0), EdgeFlags::NONE);
        assert_eq!(m.edge_crease(0), 0.0);
        assert!(m.corner_normal(0).is_none() && m.corner_color(0).is_none());
        m.validate().unwrap();
    }

    #[test]
    fn uvs_follow_polygon_order() {
        let mut m = two_tris();
        let uvs: Vec<Vec2> = (0..6).map(|i| Vec2::new(i as f32, 0.0)).collect();
        let layer = m.add_uv_layer_from_polygon_order(&uvs).unwrap();
        // Corner 3 is face 1's first corner, at vertex 2.
        let h = m.face_he(1);
        assert_eq!(m.origin(h), 2);
        assert_eq!(m.corner_uv(layer, h), Some(Vec2::new(3.0, 0.0)));
        assert!(m.add_uv_layer_from_polygon_order(&uvs[..5]).is_none());
        m.add_uv_layer().unwrap();
        assert!(m.add_uv_layer().is_none(), "at most two layers");
        m.validate().unwrap();
    }
}
