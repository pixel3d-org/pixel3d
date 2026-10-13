//! Scene statistics, as Blender's viewport overlay shows them (View >
//! Statistics): objects, vertices, edges, faces and triangles, the selected
//! part against the total. Objects hidden in the viewport are left out.
//! Counting walks every face for triangles, so results are cached per
//! document state, hidden set and selection, never recounted per frame.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use p3d_scene::{Document, ObjectKey};

/// Element counts of a set of objects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub objects: usize,
    pub verts: usize,
    pub edges: usize,
    pub faces: usize,
    pub tris: usize,
}

impl Counts {
    fn add(&mut self, doc: &Document, key: ObjectKey) {
        self.objects += 1;
        if let Some(m) = doc.object_mesh(key) {
            self.verts += m.vert_count();
            self.edges += m.edge_count();
            self.faces += m.face_count();
            self.tris += m.triangle_count();
        }
    }
}

/// The selected and the visible totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub selected: Counts,
    pub total: Counts,
    /// Triangles of every object, hidden ones too (the asset budget counts
    /// what exports, not what the viewport shows).
    pub budget_tris: usize,
}

/// Counts the visible objects and the selected ones among them.
///
/// Complexity: O(objects + faces of the visible meshes).
pub fn count(doc: &Document, selected: &BTreeSet<ObjectKey>, hidden: &BTreeSet<ObjectKey>) -> Stats {
    count_with_objects(doc, selected, hidden).0
}

/// [`count`], plus the triangles of every object (hidden ones too).
///
/// Complexity: O(objects log objects + faces of all meshes).
pub fn count_with_objects(doc: &Document, selected: &BTreeSet<ObjectKey>, hidden: &BTreeSet<ObjectKey>) -> (Stats, BTreeMap<ObjectKey, usize>) {
    let mut s = Stats::default();
    let mut per_object = BTreeMap::new();
    for (key, _) in doc.objects() {
        if hidden.contains(&key) {
            let tris = doc.object_mesh(key).map_or(0, |m| m.triangle_count());
            s.budget_tris += tris;
            per_object.insert(key, tris);
            continue;
        }
        let before = s.total;
        s.total.add(doc, key);
        s.budget_tris += s.total.tris - before.tris;
        per_object.insert(key, s.total.tris - before.tris);
        if selected.contains(&key) {
            // Reuse the object's counts instead of walking it twice.
            let d = |a: usize, b: usize| a - b;
            s.selected.objects += 1;
            s.selected.verts += d(s.total.verts, before.verts);
            s.selected.edges += d(s.total.edges, before.edges);
            s.selected.faces += d(s.total.faces, before.faces);
            s.selected.tris += d(s.total.tris, before.tris);
        }
    }
    (s, per_object)
}

/// Statistics for the last document state and selection seen.
#[derive(Debug, Default)]
pub struct StatsCache {
    key: Option<u64>,
    stats: Stats,
    /// Triangles per object, counted with the stats.
    tris: BTreeMap<ObjectKey, usize>,
}

impl StatsCache {
    /// The statistics for `scene_key` with this selection and hidden set,
    /// counted again only when one of them changed.
    ///
    /// Complexity: O(selected + hidden) to check; a recount as [`count`].
    pub fn get(&mut self, scene_key: u64, doc: &Document, selected: &BTreeSet<ObjectKey>, hidden: &BTreeSet<ObjectKey>) -> Stats {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        scene_key.hash(&mut h);
        selected.hash(&mut h);
        hidden.hash(&mut h);
        let key = h.finish();
        if self.key != Some(key) {
            (self.stats, self.tris) = count_with_objects(doc, selected, hidden);
            self.key = Some(key);
        }
        self.stats
    }

    /// The triangles of `key` as last counted (call [`StatsCache::get`]
    /// first in the frame).
    pub fn tris_of(&self, key: ObjectKey) -> Option<usize> {
        self.tris.get(&key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_follow_selection_and_hiding() {
        let mut app = crate::PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "a" })).unwrap();
        app.run("prim.box", json!({ "name": "b" })).unwrap();
        let (a, b) = (app.doc().resolve("a").unwrap(), app.doc().resolve("b").unwrap());
        let doc = std::sync::Arc::clone(app.session.doc());
        let sel: BTreeSet<_> = [b].into();
        let s = count(&doc, &sel, &BTreeSet::new());
        assert_eq!(s.total, Counts { objects: 2, verts: 16, edges: 24, faces: 12, tris: 24 });
        assert_eq!(s.selected, Counts { objects: 1, verts: 8, edges: 12, faces: 6, tris: 12 });
        let s = count(&doc, &sel, &[a].into());
        assert_eq!(s.total.objects, 1, "hidden objects are left out");
        assert_eq!(s.budget_tris, 24, "but count toward the budget");
        // The cache recounts only when something changed.
        let mut cache = StatsCache::default();
        let first = cache.get(1, &doc, &sel, &BTreeSet::new());
        assert_eq!(cache.get(1, &doc, &sel, &BTreeSet::new()), first);
        assert_eq!(cache.get(1, &doc, &BTreeSet::new(), &BTreeSet::new()).selected, Counts::default());
    }
}
