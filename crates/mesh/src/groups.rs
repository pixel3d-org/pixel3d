//! Named element groups: how agents and the node graph refer to geometry
//! robustly. A group is a set of vertices, edges or faces stored as a bitset
//! over element slots.

use fixedbitset::FixedBitSet;

use crate::mesh::Mesh;

/// The kind of element a group holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ElementKind {
    Vert,
    Edge,
    Face,
}

impl ElementKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ElementKind::Vert => "verts",
            ElementKind::Edge => "edges",
            ElementKind::Face => "faces",
        }
    }
}

/// A set of elements of one kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    kind: ElementKind,
    bits: FixedBitSet,
}

impl Group {
    pub fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Number of elements.
    ///
    /// Complexity: O(slots / 64).
    pub fn count(&self) -> usize {
        self.bits.count_ones(..)
    }

    pub fn is_empty(&self) -> bool {
        self.bits.is_clear()
    }

    #[inline]
    pub fn contains(&self, i: u32) -> bool {
        self.bits.contains(i as usize)
    }

    /// Elements in index order.
    ///
    /// Complexity: O(slots / 64 + count).
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.ones().map(|i| i as u32)
    }

    pub fn bits(&self) -> &FixedBitSet {
        &self.bits
    }

    #[inline]
    pub(crate) fn insert(&mut self, i: u32) {
        self.bits.insert(i as usize);
    }

    #[inline]
    pub(crate) fn remove(&mut self, i: u32) {
        self.bits.set(i as usize, false);
    }

    pub(crate) fn grow(&mut self, slots: usize) {
        self.bits.grow(slots);
    }

    /// Renumbers members through an old-to-new map (dead elements map to
    /// `INVALID` and are dropped) into a bitset of `slots` bits.
    ///
    /// Complexity: O(old slots / 64 + count).
    pub(crate) fn remap(&mut self, map: &[u32], slots: usize) {
        let mut bits = FixedBitSet::with_capacity(slots);
        for i in self.bits.ones() {
            if let Some(&n) = map.get(i)
                && (n as usize) < slots
            {
                bits.insert(n as usize);
            }
        }
        self.bits = bits;
    }
}

/// Why a group operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GroupError {
    #[error("group name is empty")]
    EmptyName,
    #[error("no group named `{0}`")]
    NotFound(String),
    #[error("a group named `{0}` already exists")]
    Exists(String),
    #[error("{kind} element {index} does not exist")]
    NoSuchElement { kind: &'static str, index: u32 },
}

impl Mesh {
    /// Number of slots for elements of a kind.
    pub fn slots(&self, kind: ElementKind) -> usize {
        match kind {
            ElementKind::Vert => self.vert_slots(),
            ElementKind::Edge => self.edge_slots(),
            ElementKind::Face => self.face_slots(),
        }
    }

    /// Whether an element of a kind is live.
    pub fn is_alive(&self, kind: ElementKind, i: u32) -> bool {
        match kind {
            ElementKind::Vert => self.is_vert_alive(i),
            ElementKind::Edge => self.is_edge_alive(i),
            ElementKind::Face => self.is_face_alive(i),
        }
    }

    /// Groups in creation order.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &Group)> {
        self.groups.iter().map(|(n, g)| (n.as_str(), g))
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.get(name)
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Creates or replaces a group with the given live elements. Returns the
    /// element count.
    ///
    /// Complexity: O(slots / 64 + elements).
    pub fn set_group(&mut self, name: &str, kind: ElementKind, elements: impl IntoIterator<Item = u32>) -> Result<usize, GroupError> {
        if name.is_empty() {
            return Err(GroupError::EmptyName);
        }
        let mut bits = FixedBitSet::with_capacity(self.slots(kind));
        for i in elements {
            if !self.is_alive(kind, i) {
                return Err(GroupError::NoSuchElement { kind: kind.as_str(), index: i });
            }
            bits.insert(i as usize);
        }
        let count = bits.count_ones(..);
        self.groups.insert(name.to_owned(), Group { kind, bits });
        self.revision += 1;
        Ok(count)
    }

    /// Creates or replaces a group from a bitset over element slots. Bits of
    /// dead or out-of-range elements are dropped.
    ///
    /// Complexity: O(slots / 64 + set bits).
    pub fn set_group_bits(&mut self, name: &str, kind: ElementKind, bits: &FixedBitSet) -> Result<usize, GroupError> {
        if name.is_empty() {
            return Err(GroupError::EmptyName);
        }
        let n = self.slots(kind);
        let mut clean = FixedBitSet::with_capacity(n);
        for i in bits.ones().take_while(|&i| i < n) {
            if self.is_alive(kind, i as u32) {
                clean.insert(i);
            }
        }
        let count = clean.count_ones(..);
        self.groups.insert(name.to_owned(), Group { kind, bits: clean });
        self.revision += 1;
        Ok(count)
    }

    /// Removes a group. Returns whether it existed. Later groups keep their order.
    ///
    /// Complexity: O(groups).
    pub fn remove_group(&mut self, name: &str) -> bool {
        let removed = self.groups.shift_remove(name).is_some();
        if removed {
            self.revision += 1;
        }
        removed
    }

    /// Renames a group, keeping its position in the order.
    ///
    /// Complexity: O(groups).
    pub fn rename_group(&mut self, from: &str, to: &str) -> Result<(), GroupError> {
        if to.is_empty() {
            return Err(GroupError::EmptyName);
        }
        if from == to {
            return if self.groups.contains_key(from) { Ok(()) } else { Err(GroupError::NotFound(from.to_owned())) };
        }
        if self.groups.contains_key(to) {
            return Err(GroupError::Exists(to.to_owned()));
        }
        let index = self.groups.get_index_of(from).ok_or_else(|| GroupError::NotFound(from.to_owned()))?;
        let (_, group) = self.groups.shift_remove_index(index).ok_or_else(|| GroupError::NotFound(from.to_owned()))?;
        self.groups.shift_insert(index, to.to_owned(), group);
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

    fn strip() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y, Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 0.0)];
        Mesh::from_polygons(p, &[4, 4], &[0, 1, 2, 3, 1, 4, 5, 2]).unwrap()
    }

    #[test]
    fn create_query_rename_remove() {
        let mut m = strip();
        assert_eq!(m.set_group("left", ElementKind::Face, [0]).unwrap(), 1);
        assert_eq!(m.set_group("corners", ElementKind::Vert, [0, 3, 4, 5]).unwrap(), 4);
        assert_eq!(m.set_group("all", ElementKind::Face, [0, 1]).unwrap(), 2);
        let g = m.group("corners").unwrap();
        assert_eq!(g.kind(), ElementKind::Vert);
        assert_eq!(g.iter().collect::<Vec<_>>(), vec![0, 3, 4, 5]);
        assert!(g.contains(4) && !g.contains(1));

        m.rename_group("corners", "rim").unwrap();
        let names: Vec<&str> = m.groups().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["left", "rim", "all"], "rename keeps the order");
        assert_eq!(m.rename_group("rim", "all"), Err(GroupError::Exists("all".into())));
        assert_eq!(m.rename_group("nope", "x"), Err(GroupError::NotFound("nope".into())));

        assert!(m.remove_group("left"));
        assert!(!m.remove_group("left"));
        assert_eq!(m.groups().map(|(n, _)| n).collect::<Vec<_>>(), vec!["rim", "all"]);
        m.validate().unwrap();
    }

    #[test]
    fn rejects_bad_elements_and_names() {
        let mut m = strip();
        assert_eq!(m.set_group("", ElementKind::Face, [0]), Err(GroupError::EmptyName));
        assert_eq!(m.set_group("x", ElementKind::Face, [7]), Err(GroupError::NoSuchElement { kind: "faces", index: 7 }));
        assert!(m.group("x").is_none());
    }

    #[test]
    fn bits_are_clamped_to_live_slots() {
        let mut m = strip();
        let mut bits = FixedBitSet::with_capacity(100);
        bits.insert(1);
        bits.insert(99);
        assert_eq!(m.set_group_bits("b", ElementKind::Face, &bits).unwrap(), 1);
        assert_eq!(m.group("b").unwrap().bits().len(), m.face_slots());
        m.validate().unwrap();
    }
}
