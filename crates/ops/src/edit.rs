//! `edit.*`: mesh edits addressed by selectors and groups.

use p3d_mesh::{ElementKind, Mesh};
use p3d_query::{Filter, Selector, select as evaluate};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{group_label, label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, GroupInfo, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::tx::Tx;

const EDIT: OpFlags = OpFlags::UNDOABLE;

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

fn empty_selection(what: &str, hint: &str) -> OpError {
    OpError::new(ErrorCode::EmptySelection, what.to_owned()).at("/select").with_hint(hint.to_owned())
}

/// Boundary half-edges picked by a selector: on a selected edge, starting at
/// a selected vertex, or bordering a selected face. All of them without one.
///
/// Complexity: O(H) plus the selector.
fn boundary_starts(mesh: &Mesh, select: Option<&Selector>) -> Result<Vec<u32>, OpError> {
    let sel = match select {
        Some(s) => Some(evaluate(mesh, s).map_err(|e| OpError::from_query(e, "/select"))?),
        None => None,
    };
    Ok((0..mesh.he_slots() as u32)
        .filter(|&h| mesh.is_he_alive(h) && mesh.is_boundary_he(h))
        .filter(|&h| match &sel {
            None => true,
            Some(s) => match s.kind {
                ElementKind::Edge => s.bits.contains((h >> 1) as usize),
                ElementKind::Vert => s.bits.contains(mesh.origin(h) as usize) || s.bits.contains(mesh.dest(h) as usize),
                ElementKind::Face => mesh.face(h ^ 1).is_some_and(|f| s.bits.contains(f as usize)),
            },
        })
        .collect())
}

// ---- edit.fill -------------------------------------------------------------------------

/// Parameters of `edit.fill`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Holes to fill: those with a selected boundary edge or vertex, or
    /// bordering a selected face. Every hole when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
}

/// `edit.fill`.
pub struct Fill;

impl Op for Fill {
    const ID: &'static str = "edit.fill";
    type Params = FillParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Fill holes",
            category: Category::Edit,
            summary: "Closes holes (loops of boundary edges) with one face each.",
            docs: "Each hole becomes a single polygon over its border vertices, facing the same way as the surrounding surface. \
                   New faces take the material of a neighbouring face and go into the group `<label>.faces`.\n\n\
                   **Use for** holes found by `diagnose` (`OPEN_BOUNDARY`), or to cap an open tube. Large or curved holes give \
                   n-gons that shade poorly: keep them on flat areas.\n\n\
                   **Not for** surfaces that are meant to be open (a plane, a cloth flap): their border is a hole too.",
            examples: &[OpExample { args: r#"{"target": "bin"}"#, result: "bin (o:5): filled 1 hole with 1 face (4 corners), group fill_1.faces" }],
            aliases: &["fill", "f", "fill hole", "close hole", "cap holes", "make face"],
            complexity: "O(H) to find holes plus O(loop length) per hole",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FillParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &FillParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let starts = boundary_starts(tx.mesh(key, "/target")?, p.select.as_ref())?;
        if starts.is_empty() {
            return Err(if p.select.is_some() {
                empty_selection("the selection touches no hole", "select boundary edges: {\"edges\": {\"boundary\": true}}")
            } else {
                empty_selection(&format!("{} has no holes", label(tx.doc(), key)), "diagnose lists open boundaries")
            });
        }
        let group = group_label(tx.mesh(key, "/target")?, ctx, "fill");
        let mesh = tx.mesh_mut(key, "/target")?;
        let (mut faces, mut too_short, mut largest) = (Vec::new(), 0usize, 0usize);
        for h in starts {
            // Filling a loop takes all of its half-edges off the boundary.
            if !mesh.is_boundary_he(h) {
                continue;
            }
            match mesh.fill_boundary_loop(h) {
                Ok(f) => {
                    largest = largest.max(mesh.face_len(f));
                    faces.push(f);
                }
                Err(_) => too_short += 1,
            }
        }
        let name = format!("{group}.faces");
        if !faces.is_empty() {
            mesh.set_group(&name, ElementKind::Face, faces.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
        }
        let n = faces.len();
        let mut out = OpOutput::summary(format!(
            "{}: filled {} hole{} with {} face{} ({largest} corners{}), group {name}",
            label(tx.doc(), key),
            thousands(n),
            if n == 1 { "" } else { "s" },
            thousands(n),
            if n == 1 { "" } else { "s" },
            if n > 1 { " at most" } else { "" },
        ));
        out.changed.push(key);
        if n > 0 {
            out.groups.push(GroupInfo { object: key, name, kind: ElementKind::Face, count: n as u32 });
        }
        if too_short > 0 {
            out.warnings.push(Warning { message: format!("{too_short} hole(s) of 2 edges cannot become faces"), fix: None });
        }
        if largest > 4 {
            out.warnings.push(Warning {
                message: format!("a {largest}-corner face was made; on curved areas it will shade poorly (triangulate or grid fill later)"),
                fix: None,
            });
        }
        Ok(out)
    }
}

// ---- edit.recalc_normals ------------------------------------------------------------------

/// Parameters of `edit.recalc_normals`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecalcNormalsParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Point normals inward instead (for interiors seen from inside, such as rooms).
    #[serde(default)]
    pub inside: bool,
}

/// `edit.recalc_normals`.
pub struct RecalcNormals;

/// Does a part point outward? Closed parts by the sign of their volume; open
/// parts by whether their faces face away from the part's center on
/// average (area weighted).
///
/// Complexity: O(c) for the part's corners.
fn points_outward(mesh: &Mesh, labels: &[u32], part: &p3d_analyze::Part, id: u32) -> bool {
    if part.closed {
        return part.volume >= 0.0;
    }
    let center = (part.min + part.max) / 2.0;
    let score: f64 =
        mesh.faces().filter(|&f| labels[f as usize] == id).map(|f| f64::from(mesh.face_normal_area2(f).dot(mesh.face_centroid(f) - center))).sum();
    score >= 0.0
}

impl Op for RecalcNormals {
    const ID: &'static str = "edit.recalc_normals";
    type Params = RecalcNormalsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Recalculate normals",
            category: Category::Edit,
            summary: "Makes every connected part of a mesh face outward (or inward), flipping parts that face the wrong way.",
            docs: "Closed parts are judged by their volume (a part whose faces point inward has negative volume); open parts by \
                   whether their faces point away from the part's center on average. Parts that already face the right way are \
                   left alone, and so are the inner walls of hollow parts (cavities face inward by design). Within a part, faces \
                   always agree with their neighbours, so whole parts are flipped.\n\n\
                   **Use for** `INVERTED_NORMALS` findings, or after mirroring or importing.",
            examples: &[OpExample { args: r#"{"target": "crate"}"#, result: "crate (o:1): all 1 part already faces outward" }],
            aliases: &["recalculate normals", "recalculate outside", "shift n", "conform normals", "unify normals", "fix normals"],
            complexity: "O(V + F + C) plus O(c log c) per flipped part",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &RecalcNormalsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &RecalcNormalsParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let parts = p3d_analyze::parts(mesh);
        let wrong: Vec<u32> = parts
            .parts
            .iter()
            .enumerate()
            .filter(|(i, part)| !part.cavity && points_outward(mesh, &parts.labels, part, *i as u32) == p.inside)
            .map(|(_, part)| part.first_face)
            .collect();
        let way = if p.inside { "inward" } else { "outward" };
        let total = parts.parts.len();
        let mut out = OpOutput::default();
        if wrong.is_empty() {
            out.summary =
                format!("{}: all {total} part{} already face{} {way}", label(tx.doc(), key), plural(total), if total == 1 { "s" } else { "" });
            return Ok(out);
        }
        let mesh = tx.mesh_mut(key, "/target")?;
        let mut faces = 0usize;
        for f in &wrong {
            faces += mesh.flip_part(*f).map_err(|e| OpError::internal(e.to_string()))?;
        }
        out.summary = format!(
            "{}: flipped {} of {total} part{} ({} faces); normals face {way}",
            label(tx.doc(), key),
            wrong.len(),
            plural(total),
            thousands(faces)
        );
        out.changed.push(key);
        Ok(out)
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

// ---- edit.delete_elements ---------------------------------------------------------------

/// Parameters of `edit.delete_elements`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteElementsParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to delete. Selecting edges or vertices deletes every face that
    /// uses them.
    pub select: Selector,
}

impl Default for DeleteElementsParams {
    fn default() -> Self {
        DeleteElementsParams { target: String::new(), select: Selector::Faces(Filter::default()) }
    }
}

/// `edit.delete_elements`.
pub struct DeleteElements;

impl Op for DeleteElements {
    const ID: &'static str = "edit.delete_elements";
    type Params = DeleteElementsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Delete elements",
            category: Category::Edit,
            summary: "Deletes the selected faces (or every face using the selected edges or vertices) and anything left unused.",
            docs: "Edges and vertices left without faces are removed too, so the mesh never keeps loose geometry. Where the \
                   remaining faces around a vertex fall apart, the vertex is split so the mesh stays manifold. Deleting faces of \
                   a closed mesh opens holes (`edit.fill` closes them).\n\n\
                   **Use for** removing hidden faces (the bottom of a crate that stands on the floor), duplicate faces and \
                   debris found by `diagnose`.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "bottom"}}}"#,
                result: "crate (o:1): deleted 1 face, 0 edges and 0 vertices; 1 hole opened",
            }],
            aliases: &["delete faces", "delete vertices", "delete edges", "x", "remove faces", "dissolve faces"],
            complexity: "O(n) for the selector plus O(c log c) for the deleted corners",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &DeleteElementsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &DeleteElementsParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let sel = evaluate(mesh, &p.select).map_err(|e| OpError::from_query(e, "/select"))?;
        let faces: Vec<u32> = match sel.kind {
            ElementKind::Face => sel.bits.ones().map(|f| f as u32).collect(),
            ElementKind::Edge => {
                let mut f: Vec<u32> = sel.bits.ones().flat_map(|e| mesh.edge_faces(e as u32)).flatten().collect();
                f.sort_unstable();
                f.dedup();
                f
            }
            ElementKind::Vert => {
                let mut f: Vec<u32> = sel.bits.ones().flat_map(|v| mesh.vert_faces(v as u32).collect::<Vec<_>>()).collect();
                f.sort_unstable();
                f.dedup();
                f
            }
        };
        if faces.is_empty() {
            return Err(empty_selection("the selector matched no faces", "check group names; perceive structure lists the groups"));
        }
        let boundary_before = p3d_analyze::boundary_loop_count(mesh);
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = mesh.delete_faces(&faces).map_err(|e| OpError::internal(e.to_string()))?;
        let empty = mesh.face_count() == 0;
        let opened = p3d_analyze::boundary_loop_count(mesh).saturating_sub(boundary_before);
        let mut out = OpOutput::summary(format!(
            "{}: deleted {} face{}, {} edge{} and {} vert{}{}",
            label(tx.doc(), key),
            thousands(r.faces),
            plural(r.faces),
            thousands(r.edges),
            plural(r.edges),
            thousands(r.verts),
            if r.verts == 1 { "ex" } else { "ices" },
            match opened {
                0 => String::new(),
                1 => "; 1 hole opened".into(),
                n => format!("; {n} holes opened"),
            }
        ));
        out.changed.push(key);
        if empty {
            out.warnings.push(Warning {
                message: "the mesh is now empty".into(),
                fix: Some(crate::op::OpCall { op: "scene.delete".into(), args: serde_json::json!({ "target": p.target }) }),
            });
        }
        Ok(out)
    }
}
