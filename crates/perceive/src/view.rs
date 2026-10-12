//! The perceived scene: the target object's subtree (or the whole scene)
//! as parts in world space, with lazily built BVHs and topology.

use std::sync::OnceLock;

use glam::{Affine3A, Vec3};
use p3d_analyze::Topology;
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;
use p3d_scene::{Document, ObjectKey, SceneError};

use crate::error::PerceiveError;

/// A mesh object seen in world space. Until Model Specs exist, every mesh
/// object is a part.
pub struct Part<'a> {
    pub key: ObjectKey,
    /// Public id number (`o:12` has id 12).
    pub id: u32,
    /// Leaf name when unique in the view, else the full path.
    pub label: String,
    pub path: String,
    pub mesh: &'a Mesh,
    pub world: Affine3A,
    /// World-space bounds of the mesh's vertices.
    pub lo: Vec3,
    pub hi: Vec3,
    /// Depth below the view's root (0 for the root or top-level objects).
    pub depth: usize,
    bvh: OnceLock<TriBvh>,
    topo: OnceLock<Topology>,
}

impl Part<'_> {
    pub fn size(&self) -> Vec3 {
        self.hi - self.lo
    }

    pub fn center(&self) -> Vec3 {
        (self.lo + self.hi) / 2.0
    }

    /// Triangles in world space (built on first use).
    ///
    /// Complexity: O(n log n) on first use, O(1) after.
    pub fn bvh(&self) -> &TriBvh {
        self.bvh.get_or_init(|| TriBvh::from_mesh(self.mesh, self.world))
    }

    /// Topology metrics (computed on first use).
    ///
    /// Complexity: O(n) on first use, O(1) after.
    pub fn topology(&self) -> &Topology {
        self.topo.get_or_init(|| p3d_analyze::topology(self.mesh))
    }

    /// `crate (o:1)`.
    pub fn name_id(&self) -> String {
        format!("{} (o:{})", self.label, self.id)
    }
}

/// An object of the view that is not a part (an empty or a mesh without
/// faces), listed by `structure`.
pub struct Other {
    pub key: ObjectKey,
    pub id: u32,
    pub label: String,
    pub kind: &'static str,
    pub depth: usize,
    pub world: Affine3A,
}

/// What the perception aspects look at.
pub struct View<'a> {
    pub doc: &'a Document,
    /// The target object, or `None` for the whole scene.
    pub root: Option<ObjectKey>,
    /// Mesh objects with faces, parents before children, siblings in order.
    pub parts: Vec<Part<'a>>,
    /// Empties and empty meshes, in the same order.
    pub others: Vec<Other>,
    /// Bounds of all parts (zero when there are none).
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Maps a scene lookup error.
pub fn scene_error(e: SceneError, path: &str) -> PerceiveError {
    match e {
        SceneError::NotFound { target, suggestions } => PerceiveError::new("NOT_FOUND", format!("no object `{target}`"))
            .at(path)
            .with_suggestions(suggestions)
            .with_hint("refer to objects by name, path (parent/child) or id (o:12)"),
        SceneError::Ambiguous { name, candidates } => PerceiveError::new("AMBIGUOUS_NAME", format!("`{name}` matches {} objects", candidates.len()))
            .at(path)
            .with_suggestions(candidates)
            .with_hint("use one of the paths or an id"),
        other => PerceiveError::new("INVALID_PARAM", other.to_string()).at(path),
    }
}

impl<'a> View<'a> {
    /// The subtree of `target` (name, path or id), or the whole scene.
    ///
    /// Complexity: O(objects + vertices of the parts).
    pub fn new(doc: &'a Document, target: Option<&str>) -> Result<View<'a>, PerceiveError> {
        let root = match target {
            Some(t) => Some(doc.resolve(t).map_err(|e| scene_error(e, "/target"))?),
            None => None,
        };
        let order: Vec<(ObjectKey, usize)> = match root {
            Some(r) => depth_first(doc, std::iter::once(r)),
            None => depth_first(doc, doc.roots()),
        };
        let mut parts = Vec::new();
        let mut others = Vec::new();
        for (key, depth) in order {
            let Some(o) = doc.object(key) else { continue };
            let world = doc.world_affine(key).unwrap_or(Affine3A::IDENTITY);
            let path = doc.path(key).unwrap_or_default();
            match doc.object_mesh(key) {
                Some(mesh) if mesh.face_count() > 0 => {
                    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                    for v in mesh.verts() {
                        let p = world.transform_point3(mesh.positions[v as usize]);
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    parts.push(Part {
                        key,
                        id: o.id(),
                        label: o.name().to_owned(),
                        path,
                        mesh,
                        world,
                        lo,
                        hi,
                        depth,
                        bvh: OnceLock::new(),
                        topo: OnceLock::new(),
                    });
                }
                Some(_) => others.push(Other { key, id: o.id(), label: path, kind: "empty mesh", depth, world }),
                None => others.push(Other { key, id: o.id(), label: path, kind: o.kind().as_str(), depth, world }),
            }
        }
        // Leaf names are labels unless two parts share one; then paths are.
        let mut names: Vec<String> = parts.iter().map(|p| p.label.clone()).collect();
        names.sort_unstable();
        let mut dup: Vec<String> = names.windows(2).filter(|w| w[0] == w[1]).map(|w| w[0].clone()).collect();
        dup.dedup();
        for p in &mut parts {
            if dup.binary_search(&p.label).is_ok() {
                p.label = p.path.clone();
            }
        }
        for o in &mut others {
            if let Some(leaf) = o.label.rsplit('/').next()
                && names.binary_search_by(|n| n.as_str().cmp(leaf)).is_err()
            {
                o.label = leaf.to_owned();
            }
        }
        let (lo, hi) = parts.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(p.lo), h.max(p.hi)));
        let (lo, hi) = if parts.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { (lo, hi) };
        Ok(View { doc, root, parts, others, lo, hi })
    }

    /// A view with nothing in it.
    pub fn empty(doc: &'a Document) -> View<'a> {
        View { doc, root: None, parts: Vec::new(), others: Vec::new(), lo: Vec3::ZERO, hi: Vec3::ZERO }
    }

    /// Size of everything in view.
    pub fn size(&self) -> Vec3 {
        self.hi - self.lo
    }

    /// Diagonal of everything in view, at least 1 mm.
    pub fn diagonal(&self) -> f32 {
        self.size().length().max(1e-3)
    }

    /// `crate (o:1)` for the target, or `scene`.
    pub fn title(&self) -> String {
        match self.root {
            None => "scene".into(),
            Some(r) => match (self.doc.object(r), self.parts.iter().find(|p| p.key == r)) {
                (_, Some(p)) => p.name_id(),
                (Some(o), None) => format!("{} (o:{})", o.name(), o.id()),
                _ => "?".into(),
            },
        }
    }

    /// The part for an object (name, path or id).
    pub fn part(&self, target: &str, path: &str) -> Result<usize, PerceiveError> {
        let key = self.doc.resolve(target).map_err(|e| scene_error(e, path))?;
        self.parts.iter().position(|p| p.key == key).ok_or_else(|| {
            PerceiveError::new("INVALID_PARAM", format!("`{target}` is not a mesh part in view"))
                .at(path)
                .with_suggestions(self.parts.iter().take(5).map(|p| p.label.clone()).collect())
        })
    }

    /// Contact and gap tolerance: 1 mm, or 0.1% of the view's diagonal for
    /// large scenes.
    pub fn contact_tolerance(&self) -> f32 {
        (self.diagonal() * 1e-3).max(1e-3)
    }
}

/// Objects under `roots` with their depth, parents first, children in order.
///
/// Complexity: O(objects in the subtrees).
fn depth_first(doc: &Document, roots: impl Iterator<Item = ObjectKey>) -> Vec<(ObjectKey, usize)> {
    let mut out = Vec::new();
    let mut stack: Vec<(ObjectKey, usize)> = roots.map(|k| (k, 0)).collect();
    stack.reverse();
    while let Some((k, d)) = stack.pop() {
        out.push((k, d));
        if let Some(o) = doc.object(k) {
            stack.extend(o.children().iter().rev().map(|c| (*c, d + 1)));
        }
    }
    out
}
