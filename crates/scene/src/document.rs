//! The document: objects in a hierarchy, meshes, history graphs, settings.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Affine3A;
use p3d_math::Trs;
use p3d_mesh::Mesh;
use slotmap::{SecondaryMap, SlotMap};
use smallvec::SmallVec;

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
    #[error("the object no longer exists")]
    StaleObject,
    #[error("the mesh no longer exists")]
    StaleMesh,
}

/// The document. Large tables sit behind `Arc` and are copied on write, so
/// cloning a document (an undo snapshot) is O(1) and a transaction copies
/// only the tables and objects it touches.
#[derive(Debug, Clone)]
pub struct Document {
    objects: Arc<SlotMap<ObjectKey, Arc<Object>>>,
    by_path: Arc<BTreeMap<String, ObjectKey>>,
    by_id: Arc<BTreeMap<u32, ObjectKey>>,
    meshes: Arc<SlotMap<MeshKey, Arc<Mesh>>>,
    mesh_users: Arc<SecondaryMap<MeshKey, u32>>,
    graphs: Arc<SlotMap<GraphKey, Arc<OpGraph>>>,
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
            by_path: Arc::default(),
            by_id: Arc::default(),
            meshes: Arc::default(),
            mesh_users: Arc::default(),
            graphs: Arc::default(),
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

    /// Finds an object by id (`o:12`), by path (`chest/lid`) or by a leaf
    /// name that only one object uses.
    ///
    /// Complexity: O(log n) for ids and paths; O(n) for leaf names and on
    /// errors (suggestions).
    pub fn resolve(&self, target: &str) -> Result<ObjectKey, SceneError> {
        if let Some(n) = target.strip_prefix("o:") {
            return n.parse::<u32>().ok().and_then(|id| self.object_by_id(id)).ok_or_else(|| self.not_found(target));
        }
        if let Some(&k) = self.by_path.get(target) {
            return Ok(k);
        }
        let matches: Vec<ObjectKey> = self.objects().filter(|(_, o)| o.name == target).map(|(k, _)| k).collect();
        match matches.as_slice() {
            [k] => Ok(*k),
            [] => Err(self.not_found(target)),
            many => Err(SceneError::Ambiguous { name: target.to_owned(), candidates: many.iter().filter_map(|&k| self.path(k)).collect() }),
        }
    }

    fn not_found(&self, target: &str) -> SceneError {
        let names = self.by_path.keys().map(String::as_str).chain(self.objects.values().map(|o| o.name.as_str()));
        SceneError::NotFound { target: target.to_owned(), suggestions: suggest(target, names, 3) }
    }

    /// `base`, or `base_1`, `base_2`, ... whichever is free under `parent`.
    ///
    /// Complexity: O(k log n) for k taken names.
    pub fn unique_name(&self, base: &str, parent: Option<ObjectKey>) -> String {
        let free = |n: &str| self.child_path(parent, n).is_some_and(|p| !self.by_path.contains_key(&p));
        if free(base) {
            return base.to_owned();
        }
        (1u32..).map(|i| format!("{base}_{i}")).find(|n| free(n)).unwrap_or_else(|| base.to_owned())
    }

    // ---- objects: editing ------------------------------------------------------

    /// Creates an object with the next id.
    ///
    /// Complexity: O(depth + log n).
    pub fn create_object(&mut self, name: &str, parent: Option<ObjectKey>, kind: ObjectKind) -> Result<ObjectKey, SceneError> {
        let id = self.next_id;
        self.create_object_with_id(id, name, parent, kind)
    }

    /// Creates an object with a given id (used when loading files).
    ///
    /// Complexity: O(depth + log n).
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
        let path = self.child_path(parent, name).ok_or(SceneError::StaleObject)?;
        if self.by_path.contains_key(&path) {
            return Err(SceneError::NameTaken { suggestion: self.unique_name(name, parent), path });
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
        Arc::make_mut(&mut self.by_path).insert(path, key);
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

    /// Renames an object (and so the paths of its whole subtree).
    ///
    /// Complexity: O(subtree x path length + depth + log n).
    pub fn rename(&mut self, key: ObjectKey, name: &str) -> Result<(), SceneError> {
        check_name(name)?;
        let o = self.object(key).ok_or(SceneError::StaleObject)?;
        if o.name == name {
            return Ok(());
        }
        let parent = o.parent;
        let old_prefix = self.path(key).ok_or(SceneError::StaleObject)?;
        let new_prefix = self.child_path(parent, name).ok_or(SceneError::StaleObject)?;
        if self.by_path.contains_key(&new_prefix) {
            return Err(SceneError::NameTaken { suggestion: self.unique_name(name, parent), path: new_prefix });
        }
        if let Some(o) = self.object_mut(key) {
            o.name = name.to_owned();
        }
        self.move_paths(key, &old_prefix, &new_prefix);
        Ok(())
    }

    /// Rewrites the path index entries of a subtree after its root moved from
    /// `old_prefix` to `new_prefix` (call after the rename or reparent). Each
    /// old path is the new path with the prefix swapped back.
    ///
    /// Complexity: O(subtree x (depth + path length + log n)).
    fn move_paths(&mut self, key: ObjectKey, old_prefix: &str, new_prefix: &str) {
        let moved: Vec<(String, String, ObjectKey)> = self
            .subtree(key)
            .into_iter()
            .filter_map(|k| {
                let new = self.path(k)?;
                let old = format!("{old_prefix}{}", new.get(new_prefix.len()..)?);
                Some((old, new, k))
            })
            .collect();
        let by_path = Arc::make_mut(&mut self.by_path);
        for (old, _, _) in &moved {
            by_path.remove(old);
        }
        by_path.extend(moved.into_iter().map(|(_, new, k)| (new, k)));
    }

    /// Moves an object under another parent (`None`: to the top level). With
    /// `keep_world` the local transform is recomputed so the object does not
    /// move in the world.
    ///
    /// Complexity: O(depth + siblings + subtree x (path length + log n)).
    pub fn set_parent(&mut self, key: ObjectKey, parent: Option<ObjectKey>, keep_world: bool) -> Result<(), SceneError> {
        let o = self.object(key).ok_or(SceneError::StaleObject)?;
        if o.parent == parent {
            return Ok(());
        }
        let name = o.name.clone();
        let old_parent = o.parent;
        if let Some(p) = parent {
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == key {
                    return Err(SceneError::Cycle { child: self.path(key).unwrap_or_default(), parent: self.path(p).unwrap_or_default() });
                }
                cur = self.object(c).ok_or(SceneError::StaleObject)?.parent;
            }
        }
        let new_prefix = self.child_path(parent, &name).ok_or(SceneError::StaleObject)?;
        if self.by_path.contains_key(&new_prefix) {
            return Err(SceneError::NameTaken { suggestion: self.unique_name(&name, parent), path: new_prefix });
        }
        let new_local = if keep_world {
            let world = self.world_affine(key).ok_or(SceneError::StaleObject)?;
            let parent_world = match parent {
                Some(p) => self.world_affine(p).ok_or(SceneError::StaleObject)?,
                None => Affine3A::IDENTITY,
            };
            if parent_world.matrix3.determinant().abs() < f32::EPSILON {
                return Err(SceneError::SingularParent { child: name });
            }
            Some(Trs::from_affine(&(parent_world.inverse() * world)).ok_or(SceneError::SingularParent { child: name.clone() })?)
        } else {
            None
        };
        let old_prefix = self.path(key).ok_or(SceneError::StaleObject)?;

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
        self.move_paths(key, &old_prefix, &new_prefix);
        Ok(())
    }

    /// Deletes an object. With `recursive` its descendants go too; otherwise
    /// its children move to its parent, keeping their world transforms (and
    /// get a `_N` suffix if their name is taken there). Meshes no other
    /// object uses are deleted. Returns the ids of deleted objects.
    ///
    /// Complexity: O(subtree x (depth + log n)) recursive; O(children x
    /// (subtree x path length + log n)) otherwise.
    pub fn delete(&mut self, key: ObjectKey, recursive: bool) -> Result<Vec<u32>, SceneError> {
        let o = self.object(key).ok_or(SceneError::StaleObject)?;
        let parent = o.parent;
        let children: Vec<ObjectKey> = o.children.to_vec();
        let doomed = if recursive {
            self.subtree(key)
        } else {
            for c in children {
                let name = self.object(c).map(|o| o.name.clone()).ok_or(SceneError::StaleObject)?;
                let free = self.unique_name(&name, parent);
                if free != name {
                    self.rename(c, &free)?;
                }
                self.set_parent(c, parent, true).or_else(|_| self.set_parent(c, parent, false))?;
            }
            vec![key]
        };
        let mut ids = Vec::with_capacity(doomed.len());
        for &k in doomed.iter().rev() {
            ids.push(self.remove_one(k));
        }
        ids.reverse();
        Ok(ids)
    }

    /// Removes one object that has no children left.
    fn remove_one(&mut self, key: ObjectKey) -> u32 {
        let path = self.path(key);
        let Some(o) = Arc::make_mut(&mut self.objects).remove(key) else { return 0 };
        if let Some(p) = o.parent
            && let Some(po) = self.object_mut(p)
        {
            po.children.retain(|c| *c != key);
        }
        if let Some(path) = path {
            Arc::make_mut(&mut self.by_path).remove(&path);
        }
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

    // ---- history ---------------------------------------------------------------

    pub fn history(&self, key: ObjectKey) -> Option<&OpGraph> {
        self.object(key)?.history.and_then(|g| self.graphs.get(g)).map(|g| &**g)
    }

    /// Appends an op node to an object's history.
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

    /// Checks that the indexes agree with the objects: paths, ids,
    /// parent/child links, mesh user counts. For tests and debug builds.
    ///
    /// Complexity: O(n x depth + meshes).
    pub fn check(&self) -> Result<(), String> {
        if self.by_id.len() != self.objects.len() || self.by_path.len() != self.objects.len() {
            return Err(format!("{} objects, {} ids, {} paths", self.objects.len(), self.by_id.len(), self.by_path.len()));
        }
        let mut users: SecondaryMap<MeshKey, u32> = SecondaryMap::new();
        for (k, o) in self.objects.iter() {
            if self.by_id.get(&o.id) != Some(&k) {
                return Err(format!("id {} not indexed", o.id));
            }
            if o.id >= self.next_id {
                return Err(format!("id {} not below next id {}", o.id, self.next_id));
            }
            let path = self.path(k).ok_or("broken parent chain")?;
            if self.by_path.get(&path) != Some(&k) {
                return Err(format!("path `{path}` not indexed"));
            }
            if let Some(p) = o.parent
                && !self.objects.get(p).is_some_and(|po| po.children.contains(&k))
            {
                return Err(format!("`{path}` missing from its parent's children"));
            }
            for &c in &o.children {
                if self.objects.get(c).and_then(|co| co.parent) != Some(k) {
                    return Err(format!("child of `{path}` points elsewhere"));
                }
            }
            if let ObjectKind::Mesh { mesh } = o.kind {
                if !self.meshes.contains_key(mesh) {
                    return Err(format!("`{path}` uses a missing mesh"));
                }
                bump(&mut users, mesh);
            }
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
