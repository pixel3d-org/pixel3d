//! Plain-array form of a mesh for file formats and IPC.

use fixedbitset::FixedBitSet;
use glam::{Vec2, Vec3, Vec4};
use smallvec::SmallVec;

use crate::attrs::{EdgeFlags, MAX_UV_LAYERS};
use crate::groups::{ElementKind, Group};
use crate::mesh::Mesh;
use crate::validate::ValidationError;

/// A named group as a sorted member list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawGroup {
    pub name: String,
    /// 0 vertices, 1 edges, 2 faces.
    pub kind: u8,
    pub members: Vec<u32>,
}

/// Every array of a compacted mesh, without dead slots.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawMesh {
    pub he_next: Vec<u32>,
    pub he_prev: Vec<u32>,
    pub he_vert: Vec<u32>,
    pub he_face: Vec<u32>,
    pub vert_he: Vec<u32>,
    pub face_he: Vec<u32>,
    pub positions: Vec<[f32; 3]>,
    pub corner_uv: Vec<Vec<[f32; 2]>>,
    pub corner_normal: Option<Vec<[f32; 3]>>,
    pub corner_color: Option<Vec<[f32; 4]>>,
    pub face_material: Vec<u16>,
    pub edge_flags: Vec<u8>,
    pub edge_crease: Option<Vec<f32>>,
    pub groups: Vec<RawGroup>,
}

fn kind_code(k: ElementKind) -> u8 {
    match k {
        ElementKind::Vert => 0,
        ElementKind::Edge => 1,
        ElementKind::Face => 2,
    }
}

impl Mesh {
    /// Plain arrays of this mesh (compacted first if it has dead slots, so
    /// equal meshes give equal arrays).
    ///
    /// Complexity: O(V + H + F) plus O(groups x slots).
    pub fn to_raw(&self) -> RawMesh {
        let compacted;
        let m = if self.has_holes_in_storage() {
            let mut c = self.clone();
            c.compact();
            compacted = c;
            &compacted
        } else {
            self
        };
        RawMesh {
            he_next: m.he_next.clone(),
            he_prev: m.he_prev.clone(),
            he_vert: m.he_vert.clone(),
            he_face: m.he_face.clone(),
            vert_he: m.vert_he.clone(),
            face_he: m.face_he.clone(),
            positions: m.positions.iter().map(|p| p.to_array()).collect(),
            corner_uv: m.corner_uv.iter().map(|l| l.iter().map(|u| u.to_array()).collect()).collect(),
            corner_normal: m.corner_normal.as_ref().map(|n| n.iter().map(|x| x.to_array()).collect()),
            corner_color: m.corner_color.as_ref().map(|c| c.iter().map(|x| x.to_array()).collect()),
            face_material: m.face_material.clone(),
            edge_flags: m.edge_flags.iter().map(|f| f.bits()).collect(),
            edge_crease: m.edge_crease.clone(),
            groups: m.groups().map(|(name, g)| RawGroup { name: name.to_owned(), kind: kind_code(g.kind()), members: g.iter().collect() }).collect(),
        }
    }

    /// Rebuilds a mesh from plain arrays and checks every invariant.
    ///
    /// Complexity: O(V + H + F) plus O(groups x slots).
    pub fn from_raw(raw: RawMesh) -> Result<Mesh, ValidationError> {
        let bad = |what: &str| ValidationError(what.to_owned());
        if raw.corner_uv.len() > MAX_UV_LAYERS {
            return Err(bad("too many UV layers"));
        }
        let mut m = Mesh {
            he_next: raw.he_next,
            he_prev: raw.he_prev,
            he_vert: raw.he_vert,
            he_face: raw.he_face,
            vert_he: raw.vert_he,
            face_he: raw.face_he,
            positions: raw.positions.into_iter().map(Vec3::from).collect(),
            corner_uv: raw.corner_uv.into_iter().map(|l| l.into_iter().map(Vec2::from).collect()).collect::<SmallVec<_>>(),
            corner_normal: raw.corner_normal.map(|n| n.into_iter().map(Vec3::from).collect()),
            corner_color: raw.corner_color.map(|c| c.into_iter().map(Vec4::from).collect()),
            face_material: raw.face_material,
            edge_flags: raw.edge_flags.into_iter().map(EdgeFlags::from_bits).collect(),
            edge_crease: raw.edge_crease,
            ..Mesh::default()
        };
        // Index ranges are checked before any navigation so validate() cannot
        // index out of bounds.
        let (nh, nv, nf) = (m.he_vert.len(), m.vert_he.len(), m.face_he.len());
        let in_range = |v: &[u32], n: usize| v.iter().all(|&x| x == crate::INVALID || (x as usize) < n);
        if !(in_range(&m.he_next, nh)
            && in_range(&m.he_prev, nh)
            && in_range(&m.he_vert, nv)
            && in_range(&m.he_face, nf)
            && in_range(&m.vert_he, nh)
            && in_range(&m.face_he, nh)
            && m.he_next.len() == nh
            && m.he_prev.len() == nh
            && m.he_face.len() == nh)
        {
            return Err(bad("topology index out of range"));
        }
        for g in raw.groups {
            let kind = match g.kind {
                0 => ElementKind::Vert,
                1 => ElementKind::Edge,
                2 => ElementKind::Face,
                _ => return Err(bad("unknown group kind")),
            };
            let n = m.slots(kind);
            let mut bits = FixedBitSet::with_capacity(n);
            for x in g.members {
                if x as usize >= n {
                    return Err(ValidationError(format!("group `{}` member out of range", g.name)));
                }
                bits.insert(x as usize);
            }
            if g.name.is_empty() || m.groups.contains_key(&g.name) {
                return Err(ValidationError(format!("group name `{}` is empty or repeated", g.name)));
            }
            m.groups.insert(g.name, Group::from_bits(kind, bits));
        }
        m.validate()?;
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_attributes_and_holes() {
        let p = (0..9).map(|i| Vec3::new((i % 3) as f32, (i / 3) as f32, 0.0)).collect();
        let corners = [0, 1, 4, 3, 1, 2, 5, 4, 3, 4, 7, 6, 4, 5, 8, 7];
        let mut m = Mesh::from_polygons(p, &[4; 4], &corners).unwrap();
        m.add_uv_layer().unwrap();
        m.corner_color = Some(vec![Vec4::ONE; m.he_slots()]);
        m.edge_crease = Some(vec![0.5; m.edge_slots()]);
        m.face_material[2] = 7;
        m.set_group("left", ElementKind::Face, [0, 2]).unwrap();
        m.set_group("center", ElementKind::Vert, [4]).unwrap();
        m.delete_faces(&[1]).unwrap();
        let raw = m.to_raw();
        let back = Mesh::from_raw(raw.clone()).unwrap();
        assert_eq!(back.to_raw(), raw, "raw form is canonical");
        assert_eq!((back.face_count(), back.face_material[1]), (3, 7));
        assert_eq!(back.group("left").unwrap().count(), 2);
    }

    #[test]
    fn corrupt_arrays_are_rejected() {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        let m = Mesh::from_polygons(p, &[3], &[0, 1, 2]).unwrap();
        let mut raw = m.to_raw();
        raw.he_next[0] = 99;
        assert!(Mesh::from_raw(raw).is_err());
        let mut raw = m.to_raw();
        raw.he_next.swap(0, 1);
        assert!(Mesh::from_raw(raw).is_err());
        let mut raw = m.to_raw();
        raw.groups.push(RawGroup { name: "g".into(), kind: 2, members: vec![5] });
        assert!(Mesh::from_raw(raw).is_err());
    }
}
