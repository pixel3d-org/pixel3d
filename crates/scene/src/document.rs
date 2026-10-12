//! The document: objects in a hierarchy, meshes, history graphs, settings.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use glam::Affine3A;
use p3d_math::Trs;
use p3d_mesh::Mesh;
use slotmap::{SecondaryMap, SlotMap};
use smallvec::SmallVec;

use crate::expect::Expectation;
use crate::history::{OpGraph, OpNode};
use crate::name::{NameError, check_name, suggest};
use crate::object::{GraphKey, MeshKey, Object, ObjectKey, ObjectKind};
use crate::settings::DocSettings;

/// Why a document edit or lookup failed. Nothing changes when an edit fails.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SceneError {
    #[error("no object `{target}`")]
    NotFound { target: String, suggestions: Vec<String> },
    #[error("`{name}` matches {} objects; use a path or an id: {}", candidates.len(), candidates.join(", "))]
    Ambiguous { name: String, candidates: Vec<String> },
    #[error("an object `{path}` already exists")]
    NameTaken { path: String, suggestion: String },
    #[error(transparent)]
    InvalidName(#[from] NameError),
    #[error("cannot put `{child}` under `{parent}`, which is inside it")]
    Cycle { child: String, parent: String },
    #[error("the parent's transform is singular, so `{child}` cannot keep its world transform")]
    SingularParent { child: String },
    #[error("object id {0} is already used")]
    IdTaken(u32),
    #[error("`{0}` still has children")]
    HasChildren(String),
    #[error("the object no longer exists")]
    StaleObject,
    #[error("the mesh no longer exists")]
    StaleMesh,
}

/// Children of one parent by leaf name.
type Siblings = BTreeMap<String, ObjectKey>;

/// The document. Large tables sit behind `Arc` and are copied on write, so
/// cloning a document (an undo snapshot) is O(1) and a transaction copies
/// only the tables and objects it touches.
///
/// Names are indexed by (parent, leaf name) and by leaf name alone, never by
/// full path: renaming or moving an object updates one entry, and path
/// lookups walk the segments, so cost never grows with depth x name length.
#[derive(Debug, Clone)]
pub struct Document {
    objects: Arc<SlotMap<ObjectKey, Arc<Object>>>,
    by_name: Arc<BTreeMap<Option<ObjectKey>, Siblings>>,
    by_leaf: Arc<BTreeMap<String, BTreeSet<ObjectKey>>>,
    by_id: Arc<BTreeMap<u32, ObjectKey>>,
    meshes: Arc<SlotMap<MeshKey, Arc<Mesh>>>,
    mesh_users: Arc<SecondaryMap<MeshKey, u32>>,
    graphs: Arc<SlotMap<GraphKey, Arc<OpGraph>>>,
    expectations: Arc<Vec<Expectation>>,
    next_id: u32,
    pub settings: DocSettings,
    /// Unknown top-level fields of the file this document came from, kept so
    /// newer files survive a load and save by an older version.
    pub extras: BTreeMap<String, serde_json::Value>,
    /// Set by the engine on every committed transaction.
    pub revision: u64,
}

impl Default for Document {
    /// An empty document; the first object gets id 1 (`o:1`).
    fn default() -> Self {
        Document {
            objects: Arc::default(),
            by_name: Arc::default(),
            by_leaf: Arc::default(),
            by_id: Arc::default(),
            meshes: Arc::default(),
            mesh_users: Arc::default(),
            graphs: Arc::default(),
            expectations: Arc::default(),
            next_id: 1,
            settings: DocSettings::default(),
            extras: BTreeMap::new(),
            revision: 0,
        }
    }
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- objects: reading ------------------------------------------------

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn object(&self, key: ObjectKey) -> Option<&Object> {
        self.objects.get(key).map(|o| &**o)
    }

    /// Mutable access to the editable fields of an object (transform, tags,
    /// metadata, visibility, lock). Copies the object if a snapshot shares it.
    pub fn object_mut(&mut self, key: ObjectKey) -> Option<&mut Object> {
        Arc::make_mut(&mut self.objects).get_mut(key).map(Arc::make_mut)
    }

    /// Whether two documents share the same object (no copy happened).
    pub fn shares_object(&self, other: &Document, key: ObjectKey) -> bool {
        matches!((self.objects.get(key), other.objects.get(key)), (Some(a), Some(b)) if Arc::ptr_eq(a, b))
    }

    pub fn object_by_id(&self, id: u32) -> Option<ObjectKey> {
        self.by_id.get(&id).copied()
    }

    /// All objects in creation order (by id).
    ///
    /// Complexity: O(n).
    pub fn objects(&self) -> impl Iterator<Item = (ObjectKey, &Object)> {
        self.by_id.values().filter_map(|&k| self.objects.get(k).map(|o| (k, &**o)))
    }

    /// Top-level objects in creation order.
    ///
    /// Complexity: O(n).
    pub fn roots(&self) -> impl Iterator<Item = ObjectKey> + '_ {
        self.objects().filter(|(_, o)| o.parent.is_none()).map(|(k, _)| k)
    }

    /// Path such as `chest/lid`.
    ///
    /// Complexity: O(depth).
    pub fn path(&self, key: ObjectKey) -> Option<String> {
        let mut names: Vec<&str> = Vec::new();
        let mut cur = Some(key);
        while let Some(k) = cur {
            let o = self.objects.get(k)?;
            names.push(&o.name);
            cur = o.parent;
            if names.len() > self.objects.len() {
                return None;
            }
        }
        names.reverse();
        Some(names.join("/"))
    }

    fn child_path(&self, parent: Option<ObjectKey>, name: &str) -> Option<String> {
        match parent {
            None => Some(name.to_owned()),
            Some(p) => Some(format!("{}/{name}", self.path(p)?)),
        }
    }

    /// The key and every descendant, parents before children.
    ///
    /// Complexity: O(subtree).
    pub fn subtree(&self, key: ObjectKey) -> Vec<ObjectKey> {
        let mut out = Vec::new();
        let mut stack = vec![key];
        while let Some(k) = stack.pop() {
            if let Some(o) = self.objects.get(k) {
                out.push(k);
                stack.extend(o.children.iter().rev());
            }
        }
        out
    }

    /// World transform: parents' transforms applied over the local one.
    ///
    /// Complexity: O(depth).
    pub fn world_affine(&self, key: ObjectKey) -> Option<Affine3A> {
        let mut m = self.objects.get(key)?.transform.to_affine();
        let mut cur = self.objects.get(key)?.parent;
        let mut steps = 0;
        while let Some(p) = cur {
            let o = self.objects.get(p)?;
            m = o.transform.to_affine() * m;
            cur = o.parent;
            steps += 1;
            if steps > self.objects.len() {
                return None;
            }
        }
        Some(m)
    }

    /// The child of `parent` (or top-level object) with a leaf name.
    ///
    /// Complexity: O(log n).
    pub fn child_named(&self, parent: Option<ObjectKey>, name: &str) -> Option<ObjectKey> {
        self.by_name.get(&parent).and_then(|s| s.get(name)).copied()
    }

    /// Finds an object by id (`o:12`), by path (`chest/lid`), or by a leaf
    /// name: a top-level object with that name, else the only object with it.
    ///
    /// Complexity: O(depth x log n) for paths, O(log n) otherwise; O(n x
    /// depth) on errors (suggestions).
    pub fn resolve(&self, target: &str) -> Result<ObjectKey, SceneError> {
        if let Some(n) = target.strip_prefix("o:") {
            return n.parse::<u32>().ok().and_then(|id| self.object_by_id(id)).ok_or_else(|| self.not_found(target));
        }
        if target.contains('/') {
            let mut cur: Option<ObjectKey> = None;
            for seg in target.split('/') {
                cur = Some(self.child_named(cur, seg).ok_or_else(|| self.not_found(target))?);
            }
            return cur.ok_or_else(|| self.not_found(target));
        }
        if let Some(k) = self.child_named(None, target) {
            return Ok(k);
        }
        let matches: Vec<ObjectKey> = self.by_leaf.get(target).map(|s| s.iter().copied().collect()).unwrap_or_default();
        match matches.as_slice() {
            [k] => Ok(*k),
            [] => Err(self.not_found(target)),
            many => {
                let mut candidates: Vec<String> = many.iter().filter_map(|&k| self.path(k)).collect();
                candidates.sort();
                Err(SceneError::Ambiguous { name: target.to_owned(), candidates })
            }
        }
    }

    /// Error with suggestions from leaf names and paths (error path only).
    ///
    /// Complexity: O(n x (depth + name length^2)).
    fn not_found(&self, target: &str) -> SceneError {
        let paths: Vec<String> = self.objects().filter_map(|(k, _)| self.path(k)).collect();
        let names = self.by_leaf.keys().map(String::as_str).chain(paths.iter().map(String::as_str));
        SceneError::NotFound { target: target.to_owned(), suggestions: suggest(target, names, 3) }
    }

    /// `base`, or `base_1`, `base_2`, ... whichever is free under `parent`.
    /// Creating many unnamed objects with one base under one parent costs
    /// O(k) probes each (k = names taken so far); agents normally name objects.
    ///
    /// Complexity: O(k x log n).
    pub fn unique_name(&self, base: &str, parent: Option<ObjectKey>) -> String {
        let siblings = self.by_name.get(&parent);
        let free = |n: &str| siblings.is_none_or(|s| !s.contains_key(n));
        if free(base) {
            return base.to_owned();
        }
        (1u32..).map(|i| format!("{base}_{i}")).find(|n| free(n)).unwrap_or_else(|| base.to_owned())
    }

    fn name_error(&self, parent: Option<ObjectKey>, name: &str) -> SceneError {
        SceneError::NameTaken { path: self.child_path(parent, name).unwrap_or_else(|| name.to_owned()), suggestion: self.unique_name(name, parent) }
    }

    /// Adds an object to both name indexes.
    ///
    /// Complexity: O(log n).
    fn index_insert(&mut self, parent: Option<ObjectKey>, name: &str, key: ObjectKey) {
        Arc::make_mut(&mut self.by_name).entry(parent).or_default().insert(name.to_owned(), key);
        Arc::make_mut(&mut self.by_leaf).entry(name.to_owned()).or_default().insert(key);
    }

    /// Removes an object from both name indexes, dropping empty entries.
    ///
    /// Complexity: O(log n).
    fn index_remove(&mut self, parent: Option<ObjectKey>, name: &str, key: ObjectKey) {
        let by_name = Arc::make_mut(&mut self.by_name);
        if let Some(s) = by_name.get_mut(&parent) {
            s.remove(name);
            if s.is_empty() {
                by_name.remove(&parent);
            }
        }
        let by_leaf = Arc::make_mut(&mut self.by_leaf);
        if let Some(s) = by_leaf.get_mut(name) {
            s.remove(&key);
            if s.is_empty() {
                by_leaf.remove(name);
            }
        }
    }

    // ---- objects: editing ------------------------------------------------------

    /// Creates an object with the next id.
    ///
    /// Complexity: O(log n), plus an O(n) table copy on the first write after
    /// a snapshot.
    pub fn create_object(&mut self, name: &str, parent: Option<ObjectKey>, kind: ObjectKind) -> Result<ObjectKey, SceneError> {
        let id = self.next_id;
        self.create_object_with_id(id, name, parent, kind)
    }

    /// Creates an object with a given id (used when loading files).
    ///
    /// Complexity: O(log n), plus an O(n) table copy on the first write after
    /// a snapshot.
    pub fn create_object_with_id(&mut self, id: u32, name: &str, parent: Option<ObjectKey>, kind: ObjectKind) -> Result<ObjectKey, SceneError> {
        check_name(name)?;
        if self.by_id.contains_key(&id) {
            return Err(SceneError::IdTaken(id));
        }
        if let Some(p) = parent
            && !self.objects.contains_key(p)
        {
            return Err(SceneError::StaleObject);
        }
        if let ObjectKind::Mesh { mesh } = kind
            && !self.meshes.contains_key(mesh)
        {
            return Err(SceneError::StaleMesh);
        }
        if self.child_named(parent, name).is_some() {
            return Err(self.name_error(parent, name));
        }
        let object = Object {
            id,
            name: name.to_owned(),
            parent,
            children: SmallVec::new(),
            kind,
            history: None,
            transform: Trs::IDENTITY,
            tags: SmallVec::new(),
            metadata: BTreeMap::new(),
            visible: true,
            locked: false,
        };
        let key = Arc::make_mut(&mut self.objects).insert(Arc::new(object));
        if let Some(p) = parent
            && let Some(po) = self.object_mut(p)
        {
            po.children.push(key);
        }
        if let ObjectKind::Mesh { mesh } = kind {
            bump(Arc::make_mut(&mut self.mesh_users), mesh);
        }
        self.index_insert(parent, name, key);
        Arc::make_mut(&mut self.by_id).insert(id, key);
        self.next_id = self.next_id.max(id + 1);
        Ok(key)
    }

    /// Next id that [`Document::create_object`] will use. Ids never repeat,
    /// even after deletions.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Raises the next id (used when loading files); never lowers it.
    pub fn reserve_ids_below(&mut self, next: u32) {
        self.next_id = self.next_id.max(next);
    }

    /// Renames an object; the paths of its subtree change with it.
    ///
    /// Complexity: O(log n).
    pub fn rename(&mut self, key: ObjectKey, name: &str) -> Result<(), SceneError> {
        check_name(name)?;
        let o = self.object(key).ok_or(SceneError::StaleObject)?;
        if o.name == name {
            return Ok(());
        }
        let (parent, old) = (o.parent, o.name.clone());
        if self.child_named(parent, name).is_some() {
            return Err(self.name_error(parent, name));
        }
        self.index_remove(parent, &old, key);
        if let Some(o) = self.object_mut(key) {
            o.name = name.to_owned();
        }
        self.index_insert(parent, name, key);
        Ok(())
    }

    /// Moves an object under another parent (`None`: to the top level). With
    /// `keep_world` the local transform is recomputed so the object does not
    /// move in the world.
    ///
    /// Complexity: O(depth + siblings + log n).
    pub fn set_parent(&mut self, key: ObjectKey, parent: Option<ObjectKey>, keep_world: bool) -> Result<(), SceneError> {
        let o = self.object(key).ok_or(SceneError::StaleObject)?;
        if o.parent == parent {
            return Ok(());
        }
        let name = o.name.clone();
        let old_parent = o.parent;
        if let Some(p) = parent {
            self.check_not_inside(&[key], p)?;
        }
        if self.child_named(parent, &name).is_some() {
            return Err(self.name_error(parent, &name));
        }
        let new_local = if keep_world {
            let world = self.world_affine(key).ok_or(SceneError::StaleObject)?;
            let inv = self.inverse_world(parent)?.ok_or_else(|| SceneError::SingularParent { child: name.clone() })?;
            Some(Trs::from_affine(&(inv * world)).ok_or(SceneError::SingularParent { child: name.clone() })?)
        } else {
            None
        };
        if let Some(op) = old_parent
            && let Some(po) = self.object_mut(op)
        {
            po.children.retain(|c| *c != key);
        }
        if let Some(p) = parent
            && let Some(po) = self.object_mut(p)
        {
            po.children.push(key);
        }
        if let Some(o) = self.object_mut(key) {
            o.parent = parent;
            if let Some(t) = new_local {
                o.transform = t;
            }
        }
        self.index_remove(old_parent, &name, key);
        self.index_insert(parent, &name, key);
        Ok(())
    }

    /// Fails if `target` is one of `keys` or lies inside one of them.
    ///
    /// Complexity: O(depth x log k).
    fn check_not_inside(&self, keys: &[ObjectKey], target: ObjectKey) -> Result<(), SceneError> {
        let set: BTreeSet<ObjectKey> = keys.iter().copied().collect();
        let mut cur = Some(target);
        while let Some(c) = cur {
            if set.contains(&c) {
                return Err(SceneError::Cycle { child: self.path(c).unwrap_or_default(), parent: self.path(target).unwrap_or_default() });
            }
            cur = self.object(c).ok_or(SceneError::StaleObject)?.parent;
        }
        Ok(())
    }

    /// Inverse world transform of a parent (identity for the top level);
    /// `None` when it is singular.
    fn inverse_world(&self, parent: Option<ObjectKey>) -> Result<Option<Affine3A>, SceneError> {
        Ok(match parent {
            None => Some(Affine3A::IDENTITY),
            Some(p) => {
                let w = self.world_affine(p).ok_or(SceneError::StaleObject)?;
                (w.matrix3.determinant().abs() >= f32::EPSILON).then(|| w.inverse())
            }
        })
    }

    /// Moves every child of `from` under `to`, keeping world transforms
    /// (local ones are kept where `to` is singular) and adding `_N` suffixes
    /// where names are taken. Everything is planned first, so nothing changes
    /// on failure. Leaves `from` without children.
    ///
    /// Complexity: O(children x (depth + log n)); never O(children^2).
    pub fn move_children(&mut self, from: ObjectKey, to: Option<ObjectKey>) -> Result<(), SceneError> {
        let children: Vec<ObjectKey> = self.object(from).ok_or(SceneError::StaleObject)?.children.to_vec();
        if children.is_empty() {
            return Ok(());
        }
        if let Some(t) = to {
            self.check_not_inside(&children, t)?;
        }
        let inverse = self.inverse_world(to)?;
        let mut taken: BTreeSet<String> = BTreeSet::new();
        let mut plan = Vec::with_capacity(children.len());
        for &c in &children {
            let co = self.object(c).ok_or(SceneError::StaleObject)?;
            let mut name = co.name.clone();
            let mut i = 1u32;
            while self.child_named(to, &name).is_some() || taken.contains(&name) {
                name = format!("{}_{i}", co.name);
                i += 1;
            }
            taken.insert(name.clone());
            let world = self.world_affine(c).ok_or(SceneError::StaleObject)?;
            let local = inverse.and_then(|inv| Trs::from_affine(&(inv * world))).unwrap_or(co.transform);
            plan.push((c, co.name.clone(), name, local));
        }
        for (c, old, name, local) in plan {
            self.index_remove(Some(from), &old, c);
            if let Some(co) = self.object_mut(c) {
                co.parent = to;
                co.name = name.clone();
                co.transform = local;
            }
            if let Some(t) = to
                && let Some(po) = self.object_mut(t)
            {
                po.children.push(c);
            }
            self.index_insert(to, &name, c);
        }
        if let Some(o) = self.object_mut(from) {
            o.children.clear();
        }
        Ok(())
    }

    /// Deletes an object. With `recursive` its descendants go too; otherwise
    /// its children move to its parent, keeping their world transforms (and
    /// get a `_N` suffix if their name is taken there). Meshes no other
    /// object uses are deleted. Returns the ids of deleted objects.
    ///
    /// Complexity: O(subtree x log n) recursive, O(children x (depth + log
    /// n)) otherwise, plus O(siblings) to unlink it from its parent.
    pub fn delete(&mut self, key: ObjectKey, recursive: bool) -> Result<Vec<u32>, SceneError> {
        let parent = self.object(key).ok_or(SceneError::StaleObject)?.parent;
        if recursive {
            let doomed = self.subtree(key);
            let mut ids = Vec::with_capacity(doomed.len());
            // Children first; only the root's parent survives and needs updating.
            for &k in doomed.iter().rev() {
                ids.push(self.remove_one(k, k == key));
            }
            ids.reverse();
            return Ok(ids);
        }
        self.move_children(key, parent)?;
        Ok(vec![self.remove_one(key, true)])
    }

    /// Deletes objects that have no children, unlinking each parent once.
    /// Returns their ids in the given order.
    ///
    /// Complexity: O(k log n + siblings of the affected parents).
    pub fn delete_leaves(&mut self, keys: &[ObjectKey]) -> Result<Vec<u32>, SceneError> {
        for &k in keys {
            let o = self.object(k).ok_or(SceneError::StaleObject)?;
            if !o.children.is_empty() {
                return Err(SceneError::HasChildren(self.path(k).unwrap_or_default()));
            }
        }
        let doomed: BTreeSet<ObjectKey> = keys.iter().copied().collect();
        let parents: BTreeSet<ObjectKey> = keys.iter().filter_map(|&k| self.object(k).and_then(|o| o.parent)).collect();
        for p in parents {
            if let Some(po) = self.object_mut(p) {
                po.children.retain(|c| !doomed.contains(c));
            }
        }
        Ok(keys.iter().map(|&k| self.remove_one(k, false)).collect())
    }

    /// Removes one object that has no children left. `update_parent` is false
    /// when the caller unlinks the parent itself (or deletes it too).
    ///
    /// Complexity: O(log n), plus O(siblings) when updating the parent.
    fn remove_one(&mut self, key: ObjectKey, update_parent: bool) -> u32 {
        let Some(o) = Arc::make_mut(&mut self.objects).remove(key) else { return 0 };
        if update_parent
            && let Some(p) = o.parent
            && let Some(po) = self.object_mut(p)
        {
            po.children.retain(|c| *c != key);
        }
        self.index_remove(o.parent, &o.name, key);
        Arc::make_mut(&mut self.by_id).remove(&o.id);
        if let ObjectKind::Mesh { mesh } = o.kind {
            self.release_mesh(mesh);
        }
        if let Some(g) = o.history {
            Arc::make_mut(&mut self.graphs).remove(g);
        }
        o.id
    }

    // ---- meshes ---------------------------------------------------------------

    /// Adds a mesh no object uses yet.
    pub fn add_mesh(&mut self, mesh: Mesh) -> MeshKey {
        self.add_shared_mesh(Arc::new(mesh))
    }

    /// Adds a mesh that shares its data with other keys until edited (copies).
    pub fn add_shared_mesh(&mut self, mesh: Arc<Mesh>) -> MeshKey {
        let key = Arc::make_mut(&mut self.meshes).insert(mesh);
        Arc::make_mut(&mut self.mesh_users).insert(key, 0);
        key
    }

    /// Adds a mesh and an object showing it.
    pub fn create_mesh_object(&mut self, name: &str, parent: Option<ObjectKey>, mesh: Mesh) -> Result<ObjectKey, SceneError> {
        let mk = self.add_mesh(mesh);
        match self.create_object(name, parent, ObjectKind::Mesh { mesh: mk }) {
            Ok(k) => Ok(k),
            Err(e) => {
                self.release_mesh(mk);
                Err(e)
            }
        }
    }

    pub fn mesh(&self, key: MeshKey) -> Option<&Mesh> {
        self.meshes.get(key).map(|m| &**m)
    }

    /// The shared mesh, for cheap copies and identity checks.
    pub fn mesh_arc(&self, key: MeshKey) -> Option<&Arc<Mesh>> {
        self.meshes.get(key)
    }

    /// Mutable mesh; copies it first if a snapshot or another key shares it.
    pub fn mesh_mut(&mut self, key: MeshKey) -> Option<&mut Mesh> {
        Arc::make_mut(&mut self.meshes).get_mut(key).map(Arc::make_mut)
    }

    /// Number of objects using a mesh.
    pub fn mesh_users(&self, key: MeshKey) -> u32 {
        self.mesh_users.get(key).copied().unwrap_or(0)
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Every mesh with its key, in slot order (for memory accounting and IO).
    pub fn mesh_entries(&self) -> impl Iterator<Item = (MeshKey, &Arc<Mesh>)> {
        self.meshes.iter()
    }

    /// The mesh of a mesh object.
    pub fn object_mesh(&self, key: ObjectKey) -> Option<&Mesh> {
        self.object(key)?.mesh_key().and_then(|m| self.mesh(m))
    }

    /// Points an object at a mesh (or makes it an empty), releasing the old one.
    pub fn set_kind(&mut self, key: ObjectKey, kind: ObjectKind) -> Result<(), SceneError> {
        if let ObjectKind::Mesh { mesh } = kind
            && !self.meshes.contains_key(mesh)
        {
            return Err(SceneError::StaleMesh);
        }
        let old = self.object(key).ok_or(SceneError::StaleObject)?.kind;
        if old == kind {
            return Ok(());
        }
        if let ObjectKind::Mesh { mesh } = kind {
            bump(Arc::make_mut(&mut self.mesh_users), mesh);
        }
        if let Some(o) = self.object_mut(key) {
            o.kind = kind;
        }
        if let ObjectKind::Mesh { mesh } = old {
            self.release_mesh(mesh);
        }
        Ok(())
    }

    /// Gives a mesh object its own copy of a mesh shared with other objects
    /// (the copy shares data until edited). Returns the object's mesh key.
    pub fn make_mesh_unique(&mut self, key: ObjectKey) -> Result<MeshKey, SceneError> {
        let mk = self.object(key).ok_or(SceneError::StaleObject)?.mesh_key().ok_or(SceneError::StaleMesh)?;
        if self.mesh_users(mk) <= 1 {
            return Ok(mk);
        }
        let shared = Arc::clone(self.meshes.get(mk).ok_or(SceneError::StaleMesh)?);
        let copy = Arc::make_mut(&mut self.meshes).insert(shared);
        Arc::make_mut(&mut self.mesh_users).insert(copy, 0);
        self.set_kind(key, ObjectKind::Mesh { mesh: copy })?;
        Ok(copy)
    }

    fn release_mesh(&mut self, mesh: MeshKey) {
        let users = Arc::make_mut(&mut self.mesh_users);
        let left = users.get(mesh).copied().unwrap_or(0).saturating_sub(1);
        if left == 0 {
            users.remove(mesh);
            Arc::make_mut(&mut self.meshes).remove(mesh);
        } else {
            users.insert(mesh, left);
        }
    }

    // ---- expectations ----------------------------------------------------------

    /// Expectations in the order they were added.
    pub fn expectations(&self) -> &[Expectation] {
        &self.expectations
    }

    /// Mutable expectations (copied first if a snapshot shares them).
    ///
    /// Complexity: O(expectations) on the first write after a snapshot.
    pub fn expectations_mut(&mut self) -> &mut Vec<Expectation> {
        Arc::make_mut(&mut self.expectations)
    }

    /// `E<n>` for the smallest n not used by an expectation.
    ///
    /// Complexity: O(expectations x n).
    pub fn next_expectation_name(&self) -> String {
        (1u32..).map(|n| format!("E{n}")).find(|n| !self.expectations.iter().any(|e| &e.name == n)).unwrap_or_default()
    }

    // ---- history ---------------------------------------------------------------

    pub fn history(&self, key: ObjectKey) -> Option<&OpGraph> {
        self.object(key)?.history.and_then(|g| self.graphs.get(g)).map(|g| &**g)
    }

    /// Appends an op node to an object's history.
    ///
    /// Complexity: O(1) amortized, or O(history length) when an undo snapshot
    /// shares the history (it is copied first; see ADR 0002).
    pub fn push_history(&mut self, key: ObjectKey, node: OpNode) -> Result<(), SceneError> {
        let existing = self.object(key).ok_or(SceneError::StaleObject)?.history;
        let g = match existing {
            Some(g) => g,
            None => {
                let g = Arc::make_mut(&mut self.graphs).insert(Arc::new(OpGraph::default()));
                if let Some(o) = self.object_mut(key) {
                    o.history = Some(g);
                }
                g
            }
        };
        if let Some(graph) = Arc::make_mut(&mut self.graphs).get_mut(g) {
            Arc::make_mut(graph).push(node);
        }
        Ok(())
    }

    /// Replaces an object's history (used when loading files and freezing).
    pub fn set_history(&mut self, key: ObjectKey, graph: OpGraph) -> Result<(), SceneError> {
        let existing = self.object(key).ok_or(SceneError::StaleObject)?.history;
        match existing {
            Some(g) => {
                if let Some(slot) = Arc::make_mut(&mut self.graphs).get_mut(g) {
                    *slot = Arc::new(graph);
                }
            }
            None => {
                let g = Arc::make_mut(&mut self.graphs).insert(Arc::new(graph));
                if let Some(o) = self.object_mut(key) {
                    o.history = Some(g);
                }
            }
        }
        Ok(())
    }

    // ---- consistency ------------------------------------------------------------

    /// Checks that the indexes agree with the objects: names, ids,
    /// parent/child links, reachability from the top level, mesh user counts.
    /// For tests and debug builds.
    ///
    /// Complexity: O(n log n + meshes).
    pub fn check(&self) -> Result<(), String> {
        let n = self.objects.len();
        let named: usize = self.by_name.values().map(BTreeMap::len).sum();
        let leafed: usize = self.by_leaf.values().map(BTreeSet::len).sum();
        if self.by_id.len() != n || named != n || leafed != n {
            return Err(format!("{n} objects, {} ids, {named} sibling names, {leafed} leaf names", self.by_id.len()));
        }
        let mut users: SecondaryMap<MeshKey, u32> = SecondaryMap::new();
        let mut child_links = 0usize;
        let mut with_parent = 0usize;
        for (k, o) in self.objects.iter() {
            if self.by_id.get(&o.id) != Some(&k) {
                return Err(format!("id {} not indexed", o.id));
            }
            if o.id >= self.next_id {
                return Err(format!("id {} not below next id {}", o.id, self.next_id));
            }
            if self.child_named(o.parent, &o.name) != Some(k) || !self.by_leaf.get(&o.name).is_some_and(|s| s.contains(&k)) {
                return Err(format!("o:{} `{}` not indexed by name", o.id, o.name));
            }
            if let Some(p) = o.parent {
                if !self.objects.contains_key(p) {
                    return Err(format!("o:{} has a missing parent", o.id));
                }
                with_parent += 1;
            }
            // Each child points back here; together with the counts below, every
            // object appears exactly once in its parent's list.
            let mut kids: Vec<ObjectKey> = o.children.to_vec();
            kids.sort_unstable();
            kids.dedup();
            if kids.len() != o.children.len() {
                return Err(format!("o:{} lists a child twice", o.id));
            }
            for &c in &o.children {
                if self.objects.get(c).and_then(|co| co.parent) != Some(k) {
                    return Err(format!("a child of o:{} points elsewhere", o.id));
                }
            }
            child_links += o.children.len();
            if let ObjectKind::Mesh { mesh } = o.kind {
                if !self.meshes.contains_key(mesh) {
                    return Err(format!("o:{} uses a missing mesh", o.id));
                }
                bump(&mut users, mesh);
            }
        }
        if child_links != with_parent {
            return Err(format!("{child_links} child links for {with_parent} objects with a parent"));
        }
        // No cycles: everything is reachable from the top level.
        let reachable: usize = self.roots().map(|r| self.subtree(r).len()).sum();
        if reachable != n {
            return Err(format!("{} objects are unreachable from the top level", n - reachable));
        }
        for (m, _) in self.meshes.iter() {
            let counted = users.get(m).copied().unwrap_or(0);
            if counted == 0 || self.mesh_users(m) != counted {
                return Err(format!("mesh has {} users, counted {counted}", self.mesh_users(m)));
            }
        }
        Ok(())
    }
}

/// Increments a mesh's user count.
fn bump(users: &mut SecondaryMap<MeshKey, u32>, mesh: MeshKey) {
    let n = users.get(mesh).copied().unwrap_or(0);
    users.insert(mesh, n + 1);
}
