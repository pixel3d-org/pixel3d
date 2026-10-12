//! `select.*`: named groups, the stable way to refer to geometry.

use p3d_mesh::ElementKind;
use p3d_query::{Filter, Selector, select as evaluate};
use p3d_scene::name::suggest;
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, GroupInfo, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::tx::Tx;

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

fn check_group_name(path: &str, name: &str) -> Result<(), OpError> {
    if name.is_empty() || name.trim() != name || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(OpError::invalid(path, "group names must be 1 to 128 characters without surrounding spaces"));
    }
    Ok(())
}

fn no_group(doc: &Document, key: ObjectKey, name: &str, path: &str) -> OpError {
    let names: Vec<String> = doc.object_mesh(key).map(|m| m.groups().map(|(n, _)| n.to_owned()).collect()).unwrap_or_default();
    OpError::new(ErrorCode::NotFound, format!("{} has no group `{name}`", label(doc, key))).at(path).with_suggestions(suggest(
        name,
        names.iter().map(String::as_str),
        3,
    ))
}

fn group_out(doc: &Document, key: ObjectKey, name: &str, verb: &str) -> OpOutput {
    let (kind, count) = doc.object_mesh(key).and_then(|m| m.group(name)).map_or((ElementKind::Face, 0), |g| (g.kind(), g.count()));
    let mut out = OpOutput::summary(format!("{}: {verb} group {name} ({} {})", label(doc, key), thousands(count), kind.as_str()));
    out.changed.push(key);
    out.groups.push(GroupInfo { object: key, name: name.to_owned(), kind, count: count as u32 });
    out
}

// ---- select.save ----------------------------------------------------------------------

/// Parameters of `select.save`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Elements to store.
    pub select: Selector,
    /// Group name; an existing group of that name is replaced.
    pub name: String,
}

impl Default for SaveParams {
    fn default() -> Self {
        SaveParams { target: String::new(), select: Selector::Faces(Filter::default()), name: String::new() }
    }
}

/// `select.save`.
pub struct Save;

impl Op for Save {
    const ID: &'static str = "select.save";
    type Params = SaveParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Save selection",
            category: Category::Select,
            summary: "Stores the elements a selector picks as a named group on the mesh.",
            docs: "Groups survive later edits, so ops and agents can refer to geometry by name instead of indices. Selectors combine \
                   groups, normals, materials, flags and regions (see the query docs). An empty result is an error, because it \
                   almost always means a wrong selector.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"normal": {"dir": [0, 0, 1], "within_deg": 10}}}, "name": "front_faces"}"#,
                result: "crate (o:1): saved group front_faces (1 faces)",
            }],
            aliases: &["vertex group", "face map", "selection set", "quick select", "save selection"],
            complexity: "O(n) per selector term in elements of its kind",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SaveParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        check_group_name("/name", &p.name)
    }

    fn apply(tx: &mut Tx, p: &SaveParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let sel = evaluate(tx.mesh(key, "/target")?, &p.select).map_err(|e| OpError::from_query(e, "/select"))?;
        if sel.is_empty() {
            return Err(OpError::new(ErrorCode::EmptySelection, "the selector matched nothing")
                .at("/select")
                .with_hint("check group names and normal directions; perceive structure lists the groups"));
        }
        tx.mesh_mut(key, "/target")?.set_group_bits(&p.name, sel.kind, &sel.bits).map_err(|e| OpError::invalid("/name", e.to_string()))?;
        Ok(group_out(tx.doc(), key, &p.name, "saved"))
    }
}

// ---- select.grow / select.shrink -----------------------------------------------------------

/// Parameters of `select.grow` and `select.shrink`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrowParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Group to grow or shrink.
    pub group: String,
    /// Rings of neighbours to add or remove.
    #[serde(default = "one")]
    pub steps: u32,
    /// Store the result under this name instead of changing the group.
    #[serde(default)]
    pub save_as: Option<String>,
}

fn one() -> u32 {
    1
}

impl Default for GrowParams {
    fn default() -> Self {
        GrowParams { target: String::new(), group: String::new(), steps: 1, save_as: None }
    }
}

fn grow_or_shrink(tx: &mut Tx, p: &GrowParams, grow: bool) -> Result<OpOutput, OpError> {
    let key = mesh_target(tx.doc(), &p.target)?;
    let mesh = tx.mesh(key, "/target")?;
    let kind = mesh.group(&p.group).map(|g| g.kind()).ok_or_else(|| no_group(tx.doc(), key, &p.group, "/group"))?;
    let base = Box::new(match kind {
        ElementKind::Face => Selector::Faces(Filter { group: Some(p.group.clone()), ..Filter::default() }),
        ElementKind::Edge => Selector::Edges(Filter { group: Some(p.group.clone()), ..Filter::default() }),
        ElementKind::Vert => Selector::Verts(Filter { group: Some(p.group.clone()), ..Filter::default() }),
    });
    let s = if grow { Selector::Grow { steps: p.steps, of: base } } else { Selector::Shrink { steps: p.steps, of: base } };
    let sel = evaluate(mesh, &s).map_err(|e| OpError::from_query(e, ""))?;
    let name = p.save_as.clone().unwrap_or_else(|| p.group.clone());
    tx.mesh_mut(key, "/target")?.set_group_bits(&name, sel.kind, &sel.bits).map_err(|e| OpError::invalid("/save_as", e.to_string()))?;
    Ok(group_out(tx.doc(), key, &name, if grow { "grew" } else { "shrank" }))
}

fn validate_grow(doc: &Document, p: &GrowParams) -> Result<(), OpError> {
    let key = mesh_target(doc, &p.target)?;
    if doc.object_mesh(key).and_then(|m| m.group(&p.group)).is_none() {
        return Err(no_group(doc, key, &p.group, "/group"));
    }
    if p.steps == 0 || p.steps > 1000 {
        return Err(OpError::out_of_range("/steps", format!("{} is outside 1 to 1000", p.steps)));
    }
    if let Some(n) = &p.save_as {
        check_group_name("/save_as", n)?;
    }
    Ok(())
}

/// `select.grow`.
pub struct Grow;

impl Op for Grow {
    const ID: &'static str = "select.grow";
    type Params = GrowParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Grow group",
            category: Category::Select,
            summary: "Adds the neighbouring rings of elements to a group (faces across edges, edges and vertices through vertices).",
            docs: "Grows a group by `steps` rings. Use `save_as` to keep the original group.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "group": "top", "save_as": "top_ring"}"#,
                result: "crate (o:1): grew group top_ring (5 faces)",
            }],
            aliases: &["select more", "grow selection", "ctrl plus"],
            complexity: "O(n + H) for any step count",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &GrowParams) -> Result<(), OpError> {
        validate_grow(doc, p)
    }

    fn apply(tx: &mut Tx, p: &GrowParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        grow_or_shrink(tx, p, true)
    }
}

/// `select.shrink`.
pub struct Shrink;

impl Op for Shrink {
    const ID: &'static str = "select.shrink";
    type Params = GrowParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Shrink group",
            category: Category::Select,
            summary: "Removes the outermost rings of elements from a group.",
            docs: "The inverse of `select.grow`: removes elements that touch anything outside the group, `steps` times.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "group": "top", "save_as": "top_inner"}"#,
                result: "crate (o:1): shrank group top_inner (0 faces)",
            }],
            aliases: &["select less", "shrink selection", "ctrl minus"],
            complexity: "O(n + H) for any step count",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &GrowParams) -> Result<(), OpError> {
        validate_grow(doc, p)
    }

    fn apply(tx: &mut Tx, p: &GrowParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        grow_or_shrink(tx, p, false)
    }
}

// ---- select.group_delete / select.group_rename ------------------------------------------------

/// Parameters of `select.group_delete`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupDeleteParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Group to remove (the geometry stays).
    pub group: String,
}

/// `select.group_delete`.
pub struct GroupDelete;

impl Op for GroupDelete {
    const ID: &'static str = "select.group_delete";
    type Params = GroupDeleteParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Delete group",
            category: Category::Select,
            summary: "Removes a named group; the geometry stays.",
            docs: "Removes the group only. To delete geometry use `edit.delete_elements` (Phase 3).",
            examples: &[OpExample { args: r#"{"target": "crate", "group": "bottom"}"#, result: "crate (o:1): deleted group bottom" }],
            aliases: &["remove vertex group", "delete face map"],
            complexity: "O(groups)",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &GroupDeleteParams) -> Result<(), OpError> {
        let key = mesh_target(doc, &p.target)?;
        if doc.object_mesh(key).and_then(|m| m.group(&p.group)).is_none() {
            return Err(no_group(doc, key, &p.group, "/group"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &GroupDeleteParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        if !tx.mesh_mut(key, "/target")?.remove_group(&p.group) {
            return Err(no_group(tx.doc(), key, &p.group, "/group"));
        }
        let mut out = OpOutput::summary(format!("{}: deleted group {}", label(tx.doc(), key), p.group));
        out.changed.push(key);
        Ok(out)
    }
}

/// Parameters of `select.group_rename`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupRenameParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Current group name.
    pub from: String,
    /// New group name; must be free.
    pub to: String,
}

/// `select.group_rename`.
pub struct GroupRename;

impl Op for GroupRename {
    const ID: &'static str = "select.group_rename";
    type Params = GroupRenameParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Rename group",
            category: Category::Select,
            summary: "Renames a named group, keeping its elements and position in the list.",
            docs: "Use meaningful names (`lid_rim`, `handle_l`) so later ops and agents can find geometry.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "from": "top", "to": "lid_face"}"#,
                result: "crate (o:1): renamed group top to lid_face",
            }],
            aliases: &["rename vertex group", "rename face map"],
            complexity: "O(groups)",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &GroupRenameParams) -> Result<(), OpError> {
        let key = mesh_target(doc, &p.target)?;
        check_group_name("/to", &p.to)?;
        let mesh = doc.object_mesh(key).ok_or_else(|| OpError::internal("mesh vanished"))?;
        if mesh.group(&p.from).is_none() {
            return Err(no_group(doc, key, &p.from, "/from"));
        }
        if p.from != p.to && mesh.group(&p.to).is_some() {
            return Err(OpError::invalid("/to", format!("a group `{}` already exists", p.to)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &GroupRenameParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        tx.mesh_mut(key, "/target")?.rename_group(&p.from, &p.to).map_err(|e| OpError::invalid("/to", e.to_string()))?;
        let mut out = OpOutput::summary(format!("{}: renamed group {} to {}", label(tx.doc(), key), p.from, p.to));
        out.changed.push(key);
        Ok(out)
    }
}
