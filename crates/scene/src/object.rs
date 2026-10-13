//! Scene objects.

use std::collections::BTreeMap;

use p3d_math::Trs;
use slotmap::new_key_type;
use smallvec::SmallVec;

new_key_type! {
    /// Key of an object in a document (internal; agents use names and ids).
    pub struct ObjectKey;
    /// Key of a mesh in a document; several objects may share one (instances).
    pub struct MeshKey;
    /// Key of an object's history graph.
    pub struct GraphKey;
    /// Key of a curve; each curve belongs to exactly one object.
    pub struct CurveKey;
}

/// What an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A transform only (grouping, pivots, anchors).
    Empty,
    /// A polygon mesh.
    Mesh { mesh: MeshKey },
    /// Polylines and Bezier splines (profiles and paths for generators).
    Curve { curve: CurveKey },
}

impl ObjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectKind::Empty => "empty",
            ObjectKind::Mesh { .. } => "mesh",
            ObjectKind::Curve { .. } => "curve",
        }
    }
}

/// An object in the scene hierarchy. Name, parent, children, kind and id
/// change only through [`crate::Document`] so its indexes stay consistent.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) parent: Option<ObjectKey>,
    pub(crate) children: SmallVec<[ObjectKey; 4]>,
    pub(crate) kind: ObjectKind,
    pub(crate) history: Option<GraphKey>,
    /// Local transform relative to the parent.
    pub transform: Trs,
    /// Free-form tags such as `high`, `low`, `collision`, `lod1`.
    pub tags: SmallVec<[String; 2]>,
    /// Free-form metadata, sorted by key.
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// The Model Spec this object is the root of (ADR 0010).
    pub spec: Option<std::sync::Arc<crate::spec::SpecRecord>>,
    pub visible: bool,
    pub locked: bool,
}

impl Object {
    /// Numeric public id; never reused within a document.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Public id as agents write it: `o:<n>`.
    pub fn public_id(&self) -> String {
        format!("o:{}", self.id)
    }

    /// Leaf name, unique among siblings.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parent(&self) -> Option<ObjectKey> {
        self.parent
    }

    /// Children in insertion order.
    pub fn children(&self) -> &[ObjectKey] {
        &self.children
    }

    pub fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn mesh_key(&self) -> Option<MeshKey> {
        match self.kind {
            ObjectKind::Mesh { mesh } => Some(mesh),
            ObjectKind::Empty | ObjectKind::Curve { .. } => None,
        }
    }

    pub fn curve_key(&self) -> Option<CurveKey> {
        match self.kind {
            ObjectKind::Curve { curve } => Some(curve),
            _ => None,
        }
    }

    pub fn history_key(&self) -> Option<GraphKey> {
        self.history
    }
}
