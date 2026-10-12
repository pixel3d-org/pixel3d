//! Transaction working copy handed to ops.

use std::collections::BTreeSet;

use p3d_mesh::Mesh;
use p3d_scene::{Document, ObjectKey};

use crate::error::{ErrorCode, OpError};

/// The working copy of a document during one transaction. Ops mutate it;
/// the engine commits it atomically or drops it. Copy-on-write means only
/// touched objects and meshes are copied.
#[derive(Debug, Clone)]
pub struct Tx {
    doc: Document,
    touched: BTreeSet<ObjectKey>,
}

impl Tx {
    pub fn new(doc: Document) -> Self {
        Tx { doc, touched: BTreeSet::new() }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Mutable document for scene-level edits (names, hierarchy, transforms).
    pub fn doc_mut(&mut self) -> &mut Document {
        &mut self.doc
    }

    pub fn into_doc(self) -> Document {
        self.doc
    }

    /// Resolves a target, reporting errors at `path` in the request.
    pub fn resolve(&self, target: &str, path: &str) -> Result<ObjectKey, OpError> {
        self.doc.resolve(target).map_err(|e| OpError::from_scene(e, path))
    }

    /// The mesh of a mesh object for editing (copied first if shared with a
    /// snapshot). Edits reach every instance that shares the mesh.
    pub fn mesh_mut(&mut self, key: ObjectKey, path: &str) -> Result<&mut Mesh, OpError> {
        let mk = self.mesh_key(key, path)?;
        self.touched.insert(key);
        self.doc.mesh_mut(mk).ok_or_else(|| OpError::internal("mesh vanished"))
    }

    /// The mesh of a mesh object.
    pub fn mesh(&self, key: ObjectKey, path: &str) -> Result<&Mesh, OpError> {
        let mk = self.mesh_key(key, path)?;
        self.doc.mesh(mk).ok_or_else(|| OpError::internal("mesh vanished"))
    }

    fn mesh_key(&self, key: ObjectKey, path: &str) -> Result<p3d_scene::MeshKey, OpError> {
        let o = self.doc.object(key).ok_or_else(|| OpError::internal("object vanished"))?;
        o.mesh_key().ok_or_else(|| {
            OpError::new(ErrorCode::InvalidParam, format!("`{}` is an {}, not a mesh", self.doc.path(key).unwrap_or_default(), o.kind().as_str()))
                .at(path)
        })
    }

    /// Records that an op touched an object's mesh.
    pub fn touch(&mut self, key: ObjectKey) {
        self.touched.insert(key);
    }

    /// Objects whose meshes were edited through [`Tx::mesh_mut`] or [`Tx::touch`].
    pub fn touched(&self) -> &BTreeSet<ObjectKey> {
        &self.touched
    }
}
