//! `scene.*`: objects, hierarchy, transforms, tags, joining and separating.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3};
use p3d_math::Trs;
use p3d_math::transform::rotation_from_euler_deg;
use p3d_mesh::INVALID;
use p3d_scene::{Document, ObjectKey, ObjectKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_new_object, label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::{Angle, Length, vec3};
use crate::tx::Tx;

const EDIT: OpFlags = OpFlags::UNDOABLE;

fn resolve(doc: &Document, target: &str, path: &str) -> Result<ObjectKey, OpError> {
    doc.resolve(target).map_err(|e| OpError::from_scene(e, path))
}

fn scene_err(path: &str) -> impl Fn(p3d_scene::SceneError) -> OpError + '_ {
    move |e| OpError::from_scene(e, path)
}

/// Components in meters with up to 4 decimals: `-0.3 0 0.05`.
fn fmt_vec(v: Vec3) -> String {
    let f = |x: f32| {
        let s = format!("{:.4}", x);
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".to_owned() } else { s.to_owned() }
    };
    format!("{} {} {}", f(v.x), f(v.y), f(v.z))
}

// ---- scene.create_empty -----------------------------------------------------------

/// Parameters of `scene.create_empty`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CreateEmptyParams {
    /// Object name; a free name such as `empty_1` is picked when omitted.
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    pub parent: Option<String>,
    /// Position in the parent's space.
    pub position: [Length; 3],
}

/// `scene.create_empty`.
pub struct CreateEmpty;

impl Op for CreateEmpty {
    const ID: &'static str = "scene.create_empty";
    type Params = CreateEmptyParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Create empty",
            category: Category::Scene,
            summary: "Creates an empty object: a transform for grouping parts or marking a pivot.",
            docs: "An empty has a transform but no geometry. **Use for** grouping parts under one parent (move them together), pivots \
                   (door hinges, lids), and attachment points.",
            examples: &[OpExample {
                args: r#"{"name": "pivot", "parent": "crate", "position": [0, 0.6, -0.3]}"#,
                result: "created crate/pivot (o:5)",
            }],
            aliases: &["null", "locator", "group", "add empty", "dummy"],
            complexity: "O(depth + log n)",
            flags: EDIT.union(OpFlags::CREATES_OBJECTS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CreateEmptyParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref()).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &CreateEmptyParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let parent = check_new_object(tx.doc(), p.name.as_deref(), p.parent.as_deref())?;
        let name = p.name.clone().unwrap_or_else(|| tx.doc().unique_name("empty", parent));
        let key = tx.doc_mut().create_object(&name, parent, ObjectKind::Empty).map_err(scene_err("/name"))?;
        if let Some(o) = tx.doc_mut().object_mut(key) {
            o.transform.translation = vec3(&p.position);
        }
        let mut out = OpOutput::summary(format!("created {}", label(tx.doc(), key)));
        out.created.push(key);
        Ok(out)
    }
}

// ---- scene.delete -----------------------------------------------------------------------

/// Parameters of `scene.delete`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteParams {
    /// Object to delete (name, path or id).
    pub target: String,
    /// Delete children too (default); otherwise they move up to the parent.
    #[serde(default = "yes")]
    pub recursive: bool,
}

fn yes() -> bool {
    true
}

impl Default for DeleteParams {
    fn default() -> Self {
        DeleteParams { target: String::new(), recursive: true }
    }
}

/// `scene.delete`.
pub struct Delete;

impl Op for Delete {
    const ID: &'static str = "scene.delete";
    type Params = DeleteParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Delete object",
            category: Category::Scene,
            summary: "Deletes an object, with its children unless recursive is false.",
            docs: "Deletes an object. With `recursive: false` its children move to its parent and keep their world positions. Meshes \
                   no other object uses are freed. Undo restores everything.",
            examples: &[OpExample { args: r#"{"target": "crate/lid"}"#, result: "deleted crate/lid (o:2) and 1 child" }],
            aliases: &["remove", "erase", "delete object"],
            complexity: "O(subtree x log n); O(children x depth) when children move up",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &DeleteParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target").map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &DeleteParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let what = label(tx.doc(), key);
        let ids = tx.doc_mut().delete(key, p.recursive).map_err(scene_err("/target"))?;
        let extra = ids.len().saturating_sub(1);
        let summary = match extra {
            0 => format!("deleted {what}"),
            1 => format!("deleted {what} and 1 child"),
            n => format!("deleted {what} and {n} descendants"),
        };
        let mut out = OpOutput::summary(summary);
        out.deleted = ids;
        Ok(out)
    }
}

// ---- scene.duplicate ----------------------------------------------------------------------

/// Copy or instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateMode {
    /// Independent mesh (shares data until edited).
    #[default]
    Copy,
    /// Same mesh: editing one edits all instances.
    Instance,
}

/// Parameters of `scene.duplicate`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DuplicateParams {
    /// Object to duplicate (name, path or id).
    pub target: String,
    /// Name of the duplicate; a free variant of the source name when omitted.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub mode: DuplicateMode,
    /// Added to the duplicate's position.
    #[serde(default)]
    pub offset: [Length; 3],
}

/// `scene.duplicate`.
pub struct Duplicate;

impl Op for Duplicate {
    const ID: &'static str = "scene.duplicate";
    type Params = DuplicateParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Duplicate",
            category: Category::Scene,
            summary: "Duplicates an object as an independent copy or as an instance sharing its mesh.",
            docs: "Creates a sibling with the same transform (plus `offset`), tags and metadata. `copy` gets its own mesh (free until \
                   edited); `instance` shares the mesh, so edits apply to all instances and exports can reuse it. Children are not duplicated.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "mode": "instance", "offset": [1, 0, 0]}"#,
                result: "duplicated crate (o:1) as crate_1 (o:5), instance",
            }],
            aliases: &["copy", "clone", "instance", "linked duplicate", "shift d", "alt d"],
            complexity: "O(depth + log n)",
            flags: EDIT.union(OpFlags::CREATES_OBJECTS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &DuplicateParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target")?;
        if let Some(n) = &p.name {
            p3d_scene::name::check_name(n).map_err(|e| OpError::invalid("/name", e.to_string()))?;
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &DuplicateParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let src = resolve(tx.doc(), &p.target, "/target")?;
        let o = tx.doc().object(src).cloned().ok_or_else(|| OpError::internal("object vanished"))?;
        let kind = match (o.kind(), p.mode) {
            (ObjectKind::Empty, _) => ObjectKind::Empty,
            (k @ ObjectKind::Mesh { .. }, DuplicateMode::Instance) => k,
            (ObjectKind::Mesh { mesh }, DuplicateMode::Copy) => {
                let shared = Arc::clone(tx.doc().mesh_arc(mesh).ok_or_else(|| OpError::internal("mesh vanished"))?);
                ObjectKind::Mesh { mesh: tx.doc_mut().add_shared_mesh(shared) }
            }
        };
        let name = p.name.clone().unwrap_or_else(|| tx.doc().unique_name(o.name(), o.parent()));
        let key = tx.doc_mut().create_object(&name, o.parent(), kind).map_err(scene_err("/name"))?;
        if let Some(d) = tx.doc_mut().object_mut(key) {
            d.transform = o.transform;
            d.transform.translation += vec3(&p.offset);
            d.tags = o.tags.clone();
            d.metadata = o.metadata.clone();
            d.visible = o.visible;
        }
        let mode = match p.mode {
            DuplicateMode::Copy => "copy",
            DuplicateMode::Instance => "instance",
        };
        let mut out = OpOutput::summary(format!("duplicated {} as {}, {mode}", label(tx.doc(), src), label(tx.doc(), key)));
        out.created.push(key);
        Ok(out)
    }
}

// ---- scene.rename -------------------------------------------------------------------------

/// Parameters of `scene.rename`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameParams {
    /// Object to rename (name, path or id).
    pub target: String,
    /// New leaf name; must be free among its siblings.
    pub name: String,
}

/// `scene.rename`.
pub struct Rename;

impl Op for Rename {
    const ID: &'static str = "scene.rename";
    type Params = RenameParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Rename",
            category: Category::Scene,
            summary: "Renames an object; paths of its children change with it.",
            docs: "Leaf names are unique among siblings and may not contain `/`. Ids (`o:12`) never change, so they are the safest references.",
            examples: &[OpExample { args: r#"{"target": "crate", "name": "chest"}"#, result: "renamed crate (o:1) to chest" }],
            aliases: &["name", "set name"],
            complexity: "O(subtree x path length + log n)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &RenameParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target")?;
        p3d_scene::name::check_name(&p.name).map_err(|e| OpError::invalid("/name", e.to_string()))
    }

    fn apply(tx: &mut Tx, p: &RenameParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let before = label(tx.doc(), key);
        tx.doc_mut().rename(key, &p.name).map_err(scene_err("/name"))?;
        Ok(OpOutput::summary(format!("renamed {before} to {}", p.name)))
    }
}

// ---- scene.parent / scene.unparent ----------------------------------------------------------

/// Parameters of `scene.parent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParentParams {
    /// Object to move (name, path or id).
    pub target: String,
    /// New parent (name, path or id).
    pub parent: String,
    /// Keep the world position (default) instead of the local transform.
    #[serde(default = "yes")]
    pub keep_world: bool,
}

impl Default for ParentParams {
    fn default() -> Self {
        ParentParams { target: String::new(), parent: String::new(), keep_world: true }
    }
}

/// `scene.parent`.
pub struct Parent;

impl Op for Parent {
    const ID: &'static str = "scene.parent";
    type Params = ParentParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set parent",
            category: Category::Scene,
            summary: "Moves an object under a parent so it follows the parent's transform.",
            docs: "By default the object keeps its world position (its local transform is recomputed). A parent cannot be the object \
                   itself or one of its descendants.",
            examples: &[OpExample { args: r#"{"target": "post", "parent": "crate"}"#, result: "post (o:3) is now crate/post" }],
            aliases: &["parent to", "ctrl p", "attach", "link"],
            complexity: "O(depth + siblings + subtree x path length)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ParentParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target")?;
        resolve(doc, &p.parent, "/parent").map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &ParentParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let parent = resolve(tx.doc(), &p.parent, "/parent")?;
        let before = label(tx.doc(), key);
        tx.doc_mut().set_parent(key, Some(parent), p.keep_world).map_err(scene_err("/parent"))?;
        Ok(OpOutput::summary(format!("{before} is now {}", tx.doc().path(key).unwrap_or_default())))
    }
}

/// Parameters of `scene.unparent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnparentParams {
    /// Object to move to the top level (name, path or id).
    pub target: String,
    /// Keep the world position (default) instead of the local transform.
    #[serde(default = "yes")]
    pub keep_world: bool,
}

impl Default for UnparentParams {
    fn default() -> Self {
        UnparentParams { target: String::new(), keep_world: true }
    }
}

/// `scene.unparent`.
pub struct Unparent;

impl Op for Unparent {
    const ID: &'static str = "scene.unparent";
    type Params = UnparentParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Clear parent",
            category: Category::Scene,
            summary: "Moves an object to the top level of the scene.",
            docs: "By default the object keeps its world position.",
            examples: &[OpExample { args: r#"{"target": "crate/lid"}"#, result: "crate/lid (o:2) is now lid" }],
            aliases: &["clear parent", "alt p", "detach"],
            complexity: "O(depth + siblings + subtree x path length)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &UnparentParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target").map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &UnparentParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let before = label(tx.doc(), key);
        tx.doc_mut().set_parent(key, None, p.keep_world).map_err(scene_err("/target"))?;
        Ok(OpOutput::summary(format!("{before} is now {}", tx.doc().path(key).unwrap_or_default())))
    }
}

// ---- scene.set_transform -------------------------------------------------------------------

/// Uniform or per-axis scale.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Scale {
    Uniform(f64),
    Axes([f64; 3]),
}

impl Scale {
    fn vec(self) -> Vec3 {
        match self {
            Scale::Uniform(s) => Vec3::splat(s as f32),
            Scale::Axes([x, y, z]) => Vec3::new(x as f32, y as f32, z as f32),
        }
    }
}

/// Parameters of `scene.set_transform`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTransformParams {
    /// Object to move (name, path or id).
    pub target: String,
    /// Position in the parent's space.
    #[serde(default)]
    pub position: Option<[Length; 3]>,
    /// Euler angles in degrees, applied about X, then Y, then Z.
    #[serde(default)]
    pub rotation: Option<[Angle; 3]>,
    /// Uniform factor or per-axis factors.
    #[serde(default)]
    pub scale: Option<Scale>,
    /// Add to the position, rotate further and multiply the scale instead of replacing them.
    #[serde(default)]
    pub relative: bool,
}

/// `scene.set_transform`.
pub struct SetTransform;

impl Op for SetTransform {
    const ID: &'static str = "scene.set_transform";
    type Params = SetTransformParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set transform",
            category: Category::Scene,
            summary: "Sets or adjusts an object's position, rotation (degrees) and scale relative to its parent.",
            docs: "Only the given parts change. Rotation is Euler degrees about X, then Y, then Z. With `relative: true`, position is \
                   added, rotation applied on top and scale multiplied. Mesh data does not change; see `scene.apply_transform` to bake it in.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "rotation": [0, 45, 0]}"#,
                result: "crate (o:1): position 0 0 0, rotation 0 45 0 deg, scale 1 1 1",
            }],
            aliases: &["move", "rotate", "scale", "translate", "grab", "set position"],
            complexity: "O(depth)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SetTransformParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target")?;
        if let Some(s) = p.scale {
            let v = s.vec();
            if !v.is_finite() || v.abs().min_element() < 1e-6 {
                return Err(OpError::out_of_range("/scale", "scale factors must be finite and non-zero").with_hint("use a negative factor to mirror"));
            }
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SetTransformParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let o = tx.doc_mut().object_mut(key).ok_or_else(|| OpError::internal("object vanished"))?;
        let t = &mut o.transform;
        if let Some(pos) = &p.position {
            t.translation = if p.relative { t.translation + vec3(pos) } else { vec3(pos) };
        }
        if let Some([x, y, z]) = p.rotation {
            let q = rotation_from_euler_deg(x.degrees() as f32, y.degrees() as f32, z.degrees() as f32);
            t.rotation = if p.relative { (q * t.rotation).normalize() } else { q };
        }
        if let Some(s) = p.scale {
            t.scale = if p.relative { t.scale * s.vec() } else { s.vec() };
        }
        let t = *t;
        let e = p3d_math::transform::rotation_to_euler_deg(t.rotation).map(|d| ((d * 100.0).round() / 100.0 + 0.0).to_string());
        let s = [t.scale.x, t.scale.y, t.scale.z].map(|c| ((c * 1000.0).round() / 1000.0).to_string());
        Ok(OpOutput::summary(format!(
            "{}: position {}, rotation {} {} {} deg, scale {} {} {}",
            label(tx.doc(), key),
            fmt_vec(t.translation),
            e[0],
            e[1],
            e[2],
            s[0],
            s[1],
            s[2]
        )))
    }
}

// ---- scene.apply_transform -----------------------------------------------------------------

/// Parameters of `scene.apply_transform`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyTransformParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Bake the position into the mesh.
    #[serde(default = "yes")]
    pub position: bool,
    /// Bake the rotation into the mesh.
    #[serde(default = "yes")]
    pub rotation: bool,
    /// Bake the scale into the mesh.
    #[serde(default = "yes")]
    pub scale: bool,
}

impl Default for ApplyTransformParams {
    fn default() -> Self {
        ApplyTransformParams { target: String::new(), position: true, rotation: true, scale: true }
    }
}

/// `scene.apply_transform`.
pub struct ApplyTransform;

impl Op for ApplyTransform {
    const ID: &'static str = "scene.apply_transform";
    type Params = ApplyTransformParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Apply transform",
            category: Category::Scene,
            summary: "Bakes an object's position, rotation and/or scale into its mesh, leaving the world shape unchanged.",
            docs: "The chosen parts of the transform are reset to identity and applied to the vertices instead. Children keep their \
                   world positions. Mirroring scales flip the faces so normals stay outward. Engines expect identity transforms with \
                   scale baked in; export profiles do this automatically.",
            examples: &[OpExample { args: r#"{"target": "crate", "position": false}"#, result: "applied rotation and scale to crate (o:1)" }],
            aliases: &["ctrl a", "freeze transformations", "reset xform", "bake transform"],
            complexity: "O(V + H) of the mesh plus O(children)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ApplyTransformParams) -> Result<(), OpError> {
        let key = resolve(doc, &p.target, "/target")?;
        if doc.object(key).and_then(|o| o.mesh_key()).is_none() {
            return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &ApplyTransformParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let o = tx.doc().object(key).ok_or_else(|| OpError::internal("object vanished"))?;
        let t = o.transform;
        let remaining = Trs {
            translation: if p.position { Vec3::ZERO } else { t.translation },
            rotation: if p.rotation { Quat::IDENTITY } else { t.rotation },
            scale: if p.scale { Vec3::ONE } else { t.scale },
        };
        let rem = remaining.to_affine();
        if rem.matrix3.determinant().abs() < f32::EPSILON {
            return Err(OpError::new(ErrorCode::Unsupported, "the remaining transform is singular"));
        }
        let applied: Affine3A = rem.inverse() * t.to_affine();
        // Children keep their world transforms: their locals absorb the applied part.
        let children: Vec<ObjectKey> = o.children().to_vec();
        let mut new_child = Vec::with_capacity(children.len());
        for c in &children {
            let ct = tx.doc().object(*c).map(|x| x.transform).unwrap_or_default();
            let n = Trs::from_affine(&(applied * ct.to_affine())).ok_or_else(|| {
                OpError::new(ErrorCode::Unsupported, format!("{} would need a sheared transform", label(tx.doc(), *c)))
                    .with_hint("apply scale and rotation separately, or unparent the child first")
            })?;
            new_child.push((*c, n));
        }
        tx.doc_mut().make_mesh_unique(key).map_err(scene_err("/target"))?;
        tx.mesh_mut(key, "/target")?.transform(applied);
        if let Some(ob) = tx.doc_mut().object_mut(key) {
            ob.transform = remaining;
        }
        for (c, n) in new_child {
            if let Some(co) = tx.doc_mut().object_mut(c) {
                co.transform = n;
            }
        }
        let parts: Vec<&str> =
            [(p.position, "position"), (p.rotation, "rotation"), (p.scale, "scale")].into_iter().filter(|(on, _)| *on).map(|(_, n)| n).collect();
        let what = match parts.as_slice() {
            [] => "nothing".to_owned(),
            [a] => (*a).to_owned(),
            [a, b] => format!("{a} and {b}"),
            _ => "position, rotation and scale".to_owned(),
        };
        let mut out = OpOutput::summary(format!("applied {what} to {}", label(tx.doc(), key)));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- scene.set_origin --------------------------------------------------------------------

/// Where to put the origin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OriginMode {
    /// Bounding box center.
    Center,
    /// Center of the bounding box's bottom face.
    #[default]
    BottomCenter,
    /// A point of the bounding box given by `bbox` fractions.
    Bbox,
    /// A point in object space given by `point`.
    Point,
}

/// Parameters of `scene.set_origin`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetOriginParams {
    /// Mesh object (name, path or id).
    pub target: String,
    #[serde(default)]
    pub to: OriginMode,
    /// For `bbox`: fractions of the bounding box per axis (0 is the minimum,
    /// 1 the maximum). For a modular wall's corner pivot: [0, 0, 0].
    #[serde(default = "half_bottom")]
    pub bbox: [f32; 3],
    /// For `point`: the new origin in object space.
    #[serde(default)]
    pub point: [Length; 3],
}

fn half_bottom() -> [f32; 3] {
    [0.5, 0.0, 0.5]
}

impl Default for SetOriginParams {
    fn default() -> Self {
        SetOriginParams { target: String::new(), to: OriginMode::BottomCenter, bbox: half_bottom(), point: [Length(0.0); 3] }
    }
}

/// `scene.set_origin`.
pub struct SetOrigin;

impl Op for SetOrigin {
    const ID: &'static str = "scene.set_origin";
    type Params = SetOriginParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set origin",
            category: Category::Scene,
            summary: "Moves an object's origin (pivot) without moving its geometry in the world.",
            docs: "Game assets usually want the origin at the bottom center (props) or a corner (modular pieces, `to: bbox` with \
                   `bbox: [0, 0, 0]`). Vertices shift the other way and children keep their world positions.",
            examples: &[OpExample { args: r#"{"target": "crate", "to": "center"}"#, result: "origin of crate (o:1) moved by 0 0.3 0 m" }],
            aliases: &["set pivot", "origin to geometry", "center pivot", "affect pivot only"],
            complexity: "O(V) of the mesh plus O(children)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SetOriginParams) -> Result<(), OpError> {
        let key = resolve(doc, &p.target, "/target")?;
        if doc.object_mesh(key).is_none() {
            return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
        }
        if !p.bbox.iter().all(|f| f.is_finite()) {
            return Err(OpError::invalid("/bbox", "fractions must be finite"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SetOriginParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let (lo, hi) = tx.mesh(key, "/target")?.bounds().ok_or_else(|| OpError::new(ErrorCode::Topology, "the mesh is empty"))?;
        let c = (lo + hi) / 2.0;
        let point = match p.to {
            OriginMode::Center => c,
            OriginMode::BottomCenter => Vec3::new(c.x, lo.y, c.z),
            OriginMode::Bbox => lo + (hi - lo) * Vec3::from(p.bbox),
            OriginMode::Point => vec3(&p.point),
        };
        tx.doc_mut().make_mesh_unique(key).map_err(scene_err("/target"))?;
        tx.mesh_mut(key, "/target")?.transform(Affine3A::from_translation(-point));
        let o = tx.doc_mut().object_mut(key).ok_or_else(|| OpError::internal("object vanished"))?;
        o.transform.translation += o.transform.rotation * (o.transform.scale * point);
        let children: Vec<ObjectKey> = o.children().to_vec();
        for ch in children {
            if let Some(co) = tx.doc_mut().object_mut(ch) {
                co.transform.translation -= point;
            }
        }
        let mut out = OpOutput::summary(format!("origin of {} moved by {} m", label(tx.doc(), key), fmt_vec(point)));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- scene.tag / scene.set_metadata ----------------------------------------------------------

/// Parameters of `scene.tag`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TagParams {
    /// Object (name, path or id).
    pub target: String,
    /// Tags to add, for example `high`, `low`, `collision`, `lod1`.
    #[serde(default)]
    pub add: Vec<String>,
    /// Tags to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// `scene.tag`.
pub struct Tag;

impl Op for Tag {
    const ID: &'static str = "scene.tag";
    type Params = TagParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Tag",
            category: Category::Scene,
            summary: "Adds or removes tags such as high, low, collision or lod1 on an object.",
            docs: "Tags drive pipelines: bake pairs (`high` and `low`), collision proxies (`collision`), LOD meshes (`lod1`...).",
            examples: &[OpExample { args: r#"{"target": "crate", "add": ["low"]}"#, result: "crate (o:1) tags: low" }],
            aliases: &["label", "mark"],
            complexity: "O(tags x (added + removed))",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TagParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target")?;
        if let Some(i) = p.add.iter().position(|t| t.trim().is_empty() || t.trim() != t) {
            return Err(OpError::invalid(&format!("/add/{i}"), "tags must be non-empty without surrounding spaces"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &TagParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let o = tx.doc_mut().object_mut(key).ok_or_else(|| OpError::internal("object vanished"))?;
        o.tags.retain(|t| !p.remove.contains(t));
        for t in &p.add {
            if !o.tags.contains(t) {
                o.tags.push(t.clone());
            }
        }
        let tags = if o.tags.is_empty() { "none".to_owned() } else { o.tags.join(", ") };
        Ok(OpOutput::summary(format!("{} tags: {tags}", label(tx.doc(), key))))
    }
}

/// Parameters of `scene.set_metadata`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMetadataParams {
    /// Object (name, path or id).
    pub target: String,
    /// Keys to set to JSON values.
    #[serde(default)]
    pub set: BTreeMap<String, serde_json::Value>,
    /// Keys to remove.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// `scene.set_metadata`.
pub struct SetMetadata;

impl Op for SetMetadata {
    const ID: &'static str = "scene.set_metadata";
    type Params = SetMetadataParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Set metadata",
            category: Category::Scene,
            summary: "Sets or removes free-form JSON metadata on an object.",
            docs: "Metadata travels with the object in `.p3d` files and, where the format allows, into exports (glTF extras).",
            examples: &[OpExample { args: r#"{"target": "crate", "set": {"material_hint": "oak"}}"#, result: "crate (o:1) metadata: material_hint" }],
            aliases: &["custom properties", "extras", "user data"],
            complexity: "O(keys log keys)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SetMetadataParams) -> Result<(), OpError> {
        resolve(doc, &p.target, "/target").map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SetMetadataParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let o = tx.doc_mut().object_mut(key).ok_or_else(|| OpError::internal("object vanished"))?;
        for k in &p.remove {
            o.metadata.remove(k);
        }
        o.metadata.extend(p.set.iter().map(|(k, v)| (k.clone(), v.clone())));
        let keys: Vec<&str> = o.metadata.keys().map(String::as_str).collect();
        let keys = if keys.is_empty() { "none".to_owned() } else { keys.join(", ") };
        Ok(OpOutput::summary(format!("{} metadata: {keys}", label(tx.doc(), key))))
    }
}

// ---- scene.join ------------------------------------------------------------------------------

/// Parameters of `scene.join`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JoinParams {
    /// Mesh objects to join (at least two).
    pub targets: Vec<String>,
    /// The object that receives the others; the first target when omitted.
    #[serde(default)]
    pub into: Option<String>,
}

/// `scene.join`.
pub struct Join;

impl Op for Join {
    const ID: &'static str = "scene.join";
    type Params = JoinParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Join",
            category: Category::Scene,
            summary: "Merges several mesh objects into one, keeping their world shapes.",
            docs: "The other objects' meshes move into the receiving object's space and are appended as separate parts; the other \
                   objects are removed and their children move to the receiver. Groups with the same name merge.",
            examples: &[OpExample { args: r#"{"targets": ["crate", "post"]}"#, result: "joined 2 objects into crate (o:1): 38 faces" }],
            aliases: &["ctrl j", "combine", "merge objects", "attach"],
            complexity: "O(total mesh size + targets x log n + children x depth)",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &JoinParams) -> Result<(), OpError> {
        if p.targets.len() < 2 {
            return Err(OpError::invalid("/targets", "join needs at least two objects"));
        }
        let mut keys = std::collections::BTreeSet::new();
        for (i, t) in p.targets.iter().enumerate() {
            let path = format!("/targets/{i}");
            let k = resolve(doc, t, &path)?;
            if doc.object_mesh(k).is_none() {
                return Err(OpError::invalid(&path, format!("{} is not a mesh", label(doc, k))));
            }
            if !keys.insert(k) {
                return Err(OpError::invalid(&path, format!("{} is listed twice", label(doc, k))));
            }
        }
        let into = match &p.into {
            Some(t) => {
                let k = resolve(doc, t, "/into")?;
                if !keys.contains(&k) {
                    return Err(OpError::invalid("/into", "`into` must be one of the targets"));
                }
                k
            }
            None => resolve(doc, &p.targets[0], "/targets/0")?,
        };
        // The receiver cannot sit inside another target: that target's children
        // (one of them holding the receiver) would have to move into it.
        let mut cur = doc.object(into).and_then(|o| o.parent());
        while let Some(a) = cur {
            if keys.contains(&a) {
                return Err(OpError::invalid("/into", format!("{} is inside {}, another target", label(doc, into), label(doc, a)))
                    .with_hint("join into the outermost object instead"));
            }
            cur = doc.object(a).and_then(|o| o.parent());
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &JoinParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let keys: Vec<ObjectKey> =
            p.targets.iter().enumerate().map(|(i, t)| resolve(tx.doc(), t, &format!("/targets/{i}"))).collect::<Result<_, _>>()?;
        let into = match &p.into {
            Some(t) => resolve(tx.doc(), t, "/into")?,
            None => keys[0],
        };
        let world_into = tx.doc().world_affine(into).ok_or_else(|| OpError::internal("object vanished"))?;
        if world_into.matrix3.determinant().abs() < f32::EPSILON {
            return Err(OpError::new(ErrorCode::Unsupported, format!("{} has a singular transform", label(tx.doc(), into))));
        }
        let inv = world_into.inverse();
        tx.doc_mut().make_mesh_unique(into).map_err(scene_err("/into"))?;
        let others: Vec<ObjectKey> = keys.iter().copied().filter(|&k| k != into).collect();
        for &k in &others {
            let rel = inv * tx.doc().world_affine(k).ok_or_else(|| OpError::internal("object vanished"))?;
            let mut part = tx.mesh(k, "/targets")?.clone();
            part.transform(rel);
            tx.mesh_mut(into, "/into")?.append(&part);
            // Children move to the receiver in one bulk step (never one by one).
            tx.doc_mut().move_children(k, Some(into)).map_err(scene_err("/targets"))?;
        }
        let deleted = tx.doc_mut().delete_leaves(&others).map_err(scene_err("/targets"))?;
        let faces = tx.mesh(into, "/into")?.face_count();
        let mut out = OpOutput::summary(format!("joined {} objects into {}: {} faces", keys.len(), label(tx.doc(), into), thousands(faces)));
        out.changed.push(into);
        out.deleted = deleted;
        Ok(out)
    }
}

// ---- scene.separate --------------------------------------------------------------------------

/// How to split a mesh.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SeparateBy {
    /// Each connected part becomes an object.
    #[default]
    LooseParts,
    /// Faces of each material slot become an object.
    Material,
    /// Faces of each listed face group become an object named after the group.
    Groups,
}

/// Parameters of `scene.separate`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeparateParams {
    /// Mesh object (name, path or id).
    pub target: String,
    #[serde(default)]
    pub by: SeparateBy,
    /// Face groups for `by: groups`; faces in none of them stay in the target.
    #[serde(default)]
    pub groups: Vec<String>,
}

/// `scene.separate`.
pub struct Separate;

impl Op for Separate {
    const ID: &'static str = "scene.separate";
    type Params = SeparateParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Separate",
            category: Category::Scene,
            summary: "Splits a mesh object into several objects by loose part, material or face group.",
            docs: "The first part stays in the target object; the others become siblings with the same transform. Vertices on the \
                   border between parts are duplicated. With `by: groups` each listed face group becomes an object named after it.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "by": "groups", "groups": ["top"]}"#,
                result: "separated crate (o:1) into 2 objects: crate (o:1), top (o:5)",
            }],
            aliases: &["p", "split", "separate by loose parts", "extract", "detach"],
            complexity: "O(V + H + F) plus O(groups x slots)",
            flags: EDIT.union(OpFlags::CREATES_OBJECTS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SeparateParams) -> Result<(), OpError> {
        let key = resolve(doc, &p.target, "/target")?;
        let mesh = doc.object_mesh(key).ok_or_else(|| OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))))?;
        if p.by == SeparateBy::Groups {
            if p.groups.is_empty() {
                return Err(OpError::invalid("/groups", "list the face groups to separate"));
            }
            for (i, g) in p.groups.iter().enumerate() {
                match mesh.group(g) {
                    Some(gr) if gr.kind() == p3d_mesh::ElementKind::Face => {}
                    Some(_) => return Err(OpError::invalid(&format!("/groups/{i}"), format!("group `{g}` does not hold faces"))),
                    None => {
                        return Err(OpError::new(ErrorCode::NotFound, format!("no group `{g}`"))
                            .at(&format!("/groups/{i}"))
                            .with_suggestions(p3d_scene::name::suggest(g, mesh.groups().map(|(n, _)| n), 3)));
                    }
                }
            }
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SeparateParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = resolve(tx.doc(), &p.target, "/target")?;
        let mesh = tx.mesh(key, "/target")?;
        let base = tx.doc().object(key).map(|o| o.name().to_owned()).unwrap_or_default();
        let (labels, names): (Vec<u32>, Vec<String>) = match p.by {
            SeparateBy::LooseParts => {
                let (l, n) = mesh.face_components();
                (l, (0..n).map(|i| if i == 0 { base.clone() } else { format!("{base}_part_{i}") }).collect())
            }
            SeparateBy::Material => {
                let mut mats: Vec<u16> = mesh.faces().map(|f| mesh.face_material(f)).collect();
                mats.sort_unstable();
                mats.dedup();
                let mut slot = vec![INVALID; mats.last().map_or(0, |&m| m as usize + 1)];
                for (i, &m) in mats.iter().enumerate() {
                    slot[m as usize] = i as u32;
                }
                let labels = (0..mesh.face_slots() as u32)
                    .map(|f| if mesh.is_face_alive(f) { slot[mesh.face_material(f) as usize] } else { INVALID })
                    .collect();
                (labels, mats.iter().enumerate().map(|(i, m)| if i == 0 { base.clone() } else { format!("{base}_mat_{m}") }).collect())
            }
            SeparateBy::Groups => {
                let mut labels: Vec<u32> = (0..mesh.face_slots() as u32).map(|f| if mesh.is_face_alive(f) { 0 } else { INVALID }).collect();
                // First listed group wins where groups overlap.
                for (i, g) in p.groups.iter().enumerate().rev() {
                    if let Some(gr) = mesh.group(g) {
                        for f in gr.iter() {
                            labels[f as usize] = i as u32 + 1;
                        }
                    }
                }
                (labels, std::iter::once(base.clone()).chain(p.groups.iter().cloned()).collect())
            }
        };
        let mut parts = mesh.split_by_face_labels(&labels);
        let used: Vec<usize> = (0..parts.len()).filter(|&i| parts[i].face_count() > 0).collect();
        if used.len() <= 1 {
            let mut out = OpOutput::summary(format!("{} has nothing to separate", label(tx.doc(), key)));
            out.warnings.push(Warning { message: "the mesh has a single part for this split".into(), fix: None });
            return Ok(out);
        }
        let (parent, transform, tags) = {
            let o = tx.doc().object(key).ok_or_else(|| OpError::internal("object vanished"))?;
            (o.parent(), o.transform, o.tags.clone())
        };
        let mut out = OpOutput::default();
        tx.doc_mut().make_mesh_unique(key).map_err(scene_err("/target"))?;
        let mut first = true;
        let mut made = Vec::new();
        for i in used {
            let part = std::mem::take(&mut parts[i]);
            if first {
                *tx.mesh_mut(key, "/target")? = part;
                out.changed.push(key);
                made.push(label(tx.doc(), key));
                first = false;
                continue;
            }
            let name = tx.doc().unique_name(&names[i], parent);
            let k = tx.doc_mut().create_mesh_object(&name, parent, part).map_err(scene_err("/target"))?;
            if let Some(o) = tx.doc_mut().object_mut(k) {
                o.transform = transform;
                o.tags = tags.clone();
            }
            out.created.push(k);
            made.push(label(tx.doc(), k));
        }
        out.summary = format!("separated {} into {} objects: {}", label(tx.doc(), key), made.len(), made.join(", "));
        Ok(out)
    }
}
