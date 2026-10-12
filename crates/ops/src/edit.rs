//! `edit.*`: mesh edits addressed by selectors and groups.

use p3d_geo::edit::{
    Axis, ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, Keep, KernelError, MergeMode, PlaneCutSpec, QuadSpec, SmoothSpec, ThickenSpec, extrude,
    inset,
};
use p3d_mesh::{ElementKind, Mesh};
use p3d_query::{Filter, Selector, select as evaluate};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{group_label, label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, GroupInfo, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::params::{Angle, Length};
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

/// Maps an edit kernel refusal to an op error.
fn kernel_error(e: KernelError) -> OpError {
    match e {
        KernelError::Topology(t) => OpError::new(ErrorCode::Topology, t.to_string())
            .with_hint("the result would not be a clean manifold mesh; select fewer or different faces"),
        KernelError::OutOfRange { param, message, hint } => OpError::out_of_range(&format!("/{param}"), message).with_hint(hint),
        KernelError::Empty(m) => empty_selection(&m, "select faces, for example {\"faces\": {\"group\": \"top\"}}"),
    }
}

/// Faces picked by a selector; other element kinds are refused.
///
/// Complexity: O(n) for the selector.
fn selected_faces(mesh: &Mesh, select: &Selector, op: &str) -> Result<Vec<u32>, OpError> {
    let sel = evaluate(mesh, select).map_err(|e| OpError::from_query(e, "/select"))?;
    if sel.kind != ElementKind::Face {
        return Err(OpError::invalid("/select", format!("{op} works on faces, but the selector picks {}", sel.kind.as_str()))
            .with_hint("select faces, for example {\"faces\": {\"group\": \"top\"}}"));
    }
    if sel.is_empty() {
        return Err(empty_selection("the selector matched no faces", "check group names; perceive structure lists the groups"));
    }
    Ok(sel.iter().collect())
}

/// Creates or replaces a face group and reports it.
fn face_group(mesh: &mut Mesh, key: ObjectKey, name: String, faces: &[u32]) -> Result<GroupInfo, OpError> {
    let n = mesh.set_group(&name, ElementKind::Face, faces.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
    Ok(GroupInfo { object: key, name, kind: ElementKind::Face, count: n as u32 })
}

/// A non-zero finite length check that allows negative values.
fn check_signed_length(path: &str, l: Length, what: &str) -> Result<(), OpError> {
    if l.0.is_finite() && l.0.abs() <= p3d_geo::prim::MAX_SIZE {
        Ok(())
    } else {
        Err(OpError::out_of_range(path, format!("{what} {} is outside -10 km to 10 km", l.0)))
    }
}

// ---- edit.extrude --------------------------------------------------------------------

/// How several faces are extruded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtrudeModeParam {
    /// Connected faces move together with walls only around the outside.
    #[default]
    Region,
    /// Every face moves on its own with walls all around it.
    Individual,
}

/// Parameters of `edit.extrude`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtrudeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to extrude.
    pub select: Selector,
    /// How far the faces move; negative pushes them in.
    #[serde(default = "default_extrude_distance")]
    pub distance: Length,
    /// `region` (default) or `individual`.
    #[serde(default)]
    pub mode: ExtrudeModeParam,
    /// Move along this object-space direction instead of the face normals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f32; 3]>,
}

fn default_extrude_distance() -> Length {
    Length(0.1)
}

impl Default for ExtrudeParams {
    fn default() -> Self {
        ExtrudeParams {
            target: String::new(),
            select: Selector::Faces(Filter::default()),
            distance: default_extrude_distance(),
            mode: ExtrudeModeParam::Region,
            direction: None,
        }
    }
}

/// `edit.extrude`.
pub struct Extrude;

impl Op for Extrude {
    const ID: &'static str = "edit.extrude";
    type Params = ExtrudeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Extrude",
            category: Category::Edit,
            summary: "Pulls faces out along their normals (or a direction) and joins them to the surface with side walls.",
            docs: "The selected faces move by `distance`; new side walls connect them to where they were. In `region` mode connected \
                   faces move together (even thickness: every face moves the full distance, creases stretch); in `individual` mode \
                   each face moves along its own normal with walls all around.\n\n\
                   **Groups:** the moved faces keep their groups and are also `<label>.cap`; the walls are `<label>.side`. New edges \
                   where faces meet at more than 30 degrees become sharp.\n\n\
                   **Use for** raised panels, buttons, legs, bosses, wall thickness on open shells (negative distance pushes in: \
                   recesses, vents). For a recess with a border, `edit.inset` first, then extrude the inner faces with a negative \
                   distance. Typical distances: 2 to 10 mm for panel details on 0.5 to 1 m props.\n\n\
                   **Not for** whole closed objects (that inflates them: use `scene.set_transform` scale) or making solid walls from \
                   a plane (use `edit.thicken`).",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}, "distance": "5cm"}"#,
                result: "crate (o:1): extruded 1 face by 50 mm: +4 side faces; groups extrude_1.cap, extrude_1.side",
            }],
            aliases: &["extrude region", "extrude faces", "e", "extrude individual", "push pull", "polyExtrudeFacet", "inflate"],
            complexity: "O(c log c) for the c corners of the selected faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ExtrudeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        check_signed_length("/distance", p.distance, "distance")?;
        if p.distance.0.abs() < 1e-7 {
            return Err(
                OpError::out_of_range("/distance", "extrusion distance must not be 0").with_hint("for example \"5mm\", or \"-5mm\" to push in")
            );
        }
        if let Some(d) = p.direction
            && !(d.iter().all(|x| x.is_finite()) && d.iter().any(|x| *x != 0.0))
        {
            return Err(OpError::invalid("/direction", "direction must be a finite, non-zero vector").with_hint("for example [0, 1, 0] for up"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &ExtrudeParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = selected_faces(tx.mesh(key, "/target")?, &p.select, "edit.extrude")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "extrude");
        let mode = match p.mode {
            ExtrudeModeParam::Region => ExtrudeMode::Region,
            ExtrudeModeParam::Individual => ExtrudeMode::Individual,
        };
        let spec = ExtrudeSpec { mode, distance: p.distance.0 as f32, direction: p.direction.map(glam::Vec3::from) };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = extrude(mesh, &faces, &spec).map_err(kernel_error)?;
        let cap = face_group(mesh, key, format!("{glabel}.cap"), &r.cap)?;
        let side = face_group(mesh, key, format!("{glabel}.side"), &r.side)?;
        let mut out = OpOutput::summary(format!(
            "{}: extruded {} face{} by {}: +{} side faces; groups {}, {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            p3d_math::format_length(p.distance.0),
            thousands(r.side.len()),
            cap.name,
            side.name
        ));
        out.changed.push(key);
        out.groups = vec![cap, side];
        Ok(out)
    }
}

// ---- edit.inset ------------------------------------------------------------------------

/// How several faces are inset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InsetModeParam {
    /// Connected faces shrink together with one rim around the outside.
    #[default]
    Region,
    /// Every face shrinks on its own with a rim all around it.
    Individual,
}

/// Parameters of `edit.inset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InsetParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to inset.
    pub select: Selector,
    /// Width of the rim, measured inside the faces.
    #[serde(default = "default_inset_thickness")]
    pub thickness: Length,
    /// Moves the inner faces along their normals; negative makes a recess.
    #[serde(default)]
    pub depth: Length,
    /// `region` (default) or `individual`.
    #[serde(default)]
    pub mode: InsetModeParam,
}

fn default_inset_thickness() -> Length {
    Length(0.01)
}

impl Default for InsetParams {
    fn default() -> Self {
        InsetParams {
            target: String::new(),
            select: Selector::Faces(Filter::default()),
            thickness: default_inset_thickness(),
            depth: Length(0.0),
            mode: InsetModeParam::Region,
        }
    }
}

/// `edit.inset`.
pub struct Inset;

impl Op for Inset {
    const ID: &'static str = "edit.inset";
    type Params = InsetParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Inset",
            category: Category::Edit,
            summary: "Shrinks faces inward within their surface, leaving a rim of quads, and optionally pushes the inner part in or out.",
            docs: "Each selected region (or face, in `individual` mode) gets a rim of width `thickness` along its border; the inner \
                   faces replace the originals. `depth` moves the inner part along the normals: negative for recessed panels, \
                   positive for raised plates.\n\n\
                   **Groups:** inner faces are `<label>.inner`, the rim `<label>.rim`; both keep the groups of the original faces.\n\n\
                   **Use for** panel borders, door and window frames, screen bezels, vents (inset then extrude in), keeping a \
                   crisp edge loop before subdivision. Typical thickness: 5 to 30 mm on 0.5 to 2 m props.\n\n\
                   **Refuses** a thickness that would fold the rim over (more than about half the shortest edge).",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "front"}}, "thickness": "4cm", "depth": "-1cm"}"#,
                result: "crate (o:1): inset 1 face by 40 mm, depth -10 mm: +4 rim faces; groups inset_1.inner, inset_1.rim",
            }],
            aliases: &["inset faces", "i", "inset region", "inset individual", "polyExtrudeFacet offset", "frame", "border"],
            complexity: "O(c log c) for the c corners of the selected faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &InsetParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_length("/thickness", p.thickness, 1e-6, p3d_geo::prim::MAX_SIZE)?;
        check_signed_length("/depth", p.depth, "depth")
    }

    fn apply(tx: &mut Tx, p: &InsetParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = selected_faces(tx.mesh(key, "/target")?, &p.select, "edit.inset")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "inset");
        let mode = match p.mode {
            InsetModeParam::Region => InsetMode::Region,
            InsetModeParam::Individual => InsetMode::Individual,
        };
        let spec = InsetSpec { mode, thickness: p.thickness.0 as f32, depth: p.depth.0 as f32 };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = inset(mesh, &faces, &spec).map_err(kernel_error)?;
        let inner = face_group(mesh, key, format!("{glabel}.inner"), &r.inner)?;
        let rim = face_group(mesh, key, format!("{glabel}.rim"), &r.rim)?;
        let depth = if p.depth.0 == 0.0 { String::new() } else { format!(", depth {}", p3d_math::format_length(p.depth.0)) };
        let mut out = OpOutput::summary(format!(
            "{}: inset {} face{} by {}{depth}: +{} rim faces; groups {}, {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            p3d_math::format_length(p.thickness.0),
            thousands(r.rim.len()),
            inner.name,
            rim.name
        ));
        out.changed.push(key);
        out.groups = vec![inner, rim];
        Ok(out)
    }
}

/// Faces picked by an optional selector: every face without one.
///
/// Complexity: O(n).
fn faces_or_all(mesh: &Mesh, select: Option<&Selector>, op: &str) -> Result<Vec<u32>, OpError> {
    match select {
        Some(s) => selected_faces(mesh, s, op),
        None => Ok(mesh.faces().collect()),
    }
}

// ---- edit.poke -------------------------------------------------------------------------

/// Parameters of `edit.poke`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PokeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to poke; every face when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Moves each new center vertex along its face normal (a pyramid when positive).
    #[serde(default)]
    pub offset: Length,
}

/// `edit.poke`.
pub struct Poke;

impl Op for Poke {
    const ID: &'static str = "edit.poke";
    type Params = PokeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Poke",
            category: Category::Edit,
            summary: "Splits each face into a fan of triangles around a new center vertex, optionally raised into a pyramid.",
            docs: "Every selected face is replaced by one triangle per edge, meeting at a new vertex at the face center; `offset` moves \
                   that vertex along the normal (spikes, studs, low-poly gems, pyramid roofs). Triangles keep the face's groups and \
                   are also `<label>.faces`.\n\n\
                   **Not for** general triangulation (use `edit.triangulate`: it adds no vertices).",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}, "offset": "5cm"}"#,
                result: "crate (o:1): poked 1 face into 4 triangles; group poke_1.faces",
            }],
            aliases: &["poke faces", "pyramid", "spike", "fan triangulate", "poke"],
            complexity: "O(c log c) for the c corners of the selected faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &PokeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        check_signed_length("/offset", p.offset, "offset")
    }

    fn apply(tx: &mut Tx, p: &PokeParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = faces_or_all(tx.mesh(key, "/target")?, p.select.as_ref(), "edit.poke")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "poke");
        let mesh = tx.mesh_mut(key, "/target")?;
        let tris = p3d_geo::edit::poke(mesh, &faces, p.offset.0 as f32).map_err(kernel_error)?;
        let g = face_group(mesh, key, format!("{glabel}.faces"), &tris)?;
        let mut out = OpOutput::summary(format!(
            "{}: poked {} face{} into {} triangles; group {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            thousands(tris.len()),
            g.name
        ));
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.triangulate --------------------------------------------------------------------

/// Parameters of `edit.triangulate`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriangulateParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to triangulate; every face when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
}

/// `edit.triangulate`.
pub struct Triangulate;

impl Op for Triangulate {
    const ID: &'static str = "edit.triangulate";
    type Params = TriangulateParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Triangulate",
            category: Category::Edit,
            summary: "Splits quads and n-gons into triangles with the diagonals that keep angles largest (beauty), adding no vertices.",
            docs: "Each face with more than three corners becomes triangles; diagonals are flipped until the smallest angles are as \
                   large as possible. Triangles keep their face's groups and material and are also `<label>.faces`.\n\n\
                   **Use for** fixing how a non-planar quad or a concave n-gon is cut before export, or preparing a mesh for an \
                   engine that needs exact triangulation control. Exporters triangulate anyway, so do not triangulate meshes you will \
                   still edit (quads are easier to cut, loop and subdivide).",
            examples: &[OpExample {
                args: r#"{"target": "crate"}"#,
                result: "crate (o:1): split 6 faces into 12 triangles; group triangulate_1.faces",
            }],
            aliases: &["triangulate faces", "ctrl t", "quads to tris", "polyTriangulate", "beauty fill"],
            complexity: "O(c log c) for the c corners, plus O(k^2) per face of k corners (k <= 256)",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TriangulateParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &TriangulateParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = faces_or_all(tx.mesh(key, "/target")?, p.select.as_ref(), "edit.triangulate")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "triangulate");
        let mesh = tx.mesh_mut(key, "/target")?;
        let (tris, split) = p3d_geo::edit::triangulate(mesh, &faces).map_err(kernel_error)?;
        let mut out = OpOutput::default();
        if split == 0 {
            out.summary = format!("{}: the {} selected faces are triangles already", label(tx.doc(), key), thousands(faces.len()));
            return Ok(out);
        }
        let g = face_group(mesh, key, format!("{glabel}.faces"), &tris)?;
        out.summary = format!(
            "{}: split {} face{} into {} triangles; group {}",
            label(tx.doc(), key),
            thousands(split),
            plural(split),
            thousands(tris.len() - (faces.len() - split)),
            g.name
        );
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.tris_to_quads -------------------------------------------------------------------

/// Advanced settings of `edit.tris_to_quads`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct TrisToQuadsAdvanced {
    /// Also join across sharp edges, UV seams and material borders.
    pub ignore_boundaries: bool,
}

/// Parameters of `edit.tris_to_quads`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrisToQuadsParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Triangles to join; every face when omitted (other faces are skipped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Largest angle between the two triangles (degrees); larger folds stay triangles.
    #[serde(default = "forty_deg")]
    pub max_face_angle: Angle,
    /// Largest difference of a quad corner from a right angle (degrees).
    #[serde(default = "forty_deg")]
    pub max_shape_angle: Angle,
    /// Rarely needed settings.
    #[serde(default)]
    pub advanced: TrisToQuadsAdvanced,
}

fn forty_deg() -> Angle {
    Angle::from_degrees(40.0)
}

impl Default for TrisToQuadsParams {
    fn default() -> Self {
        TrisToQuadsParams {
            target: String::new(),
            select: None,
            max_face_angle: forty_deg(),
            max_shape_angle: forty_deg(),
            advanced: TrisToQuadsAdvanced::default(),
        }
    }
}

/// `edit.tris_to_quads`.
pub struct TrisToQuads;

impl Op for TrisToQuads {
    const ID: &'static str = "edit.tris_to_quads";
    type Params = TrisToQuadsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Tris to quads",
            category: Category::Edit,
            summary: "Joins pairs of adjacent triangles into quads, best-shaped pairs first, where the result is flat and square enough.",
            docs: "Candidate pairs are ranked by how close the quad's corners are to right angles and how flat the pair is; the best \
                   pairs join first (greedy matching). Sharp edges, UV seams and material borders are kept unless \
                   `advanced.ignore_boundaries` is true. New quads are `<label>.faces`.\n\n\
                   **Use for** cleaning imported or triangulated meshes before more modelling, UV straightening or subdivision. \
                   Loosen the angles (up to 60 to 90 degrees) for curved, coarse meshes.",
            examples: &[OpExample {
                args: r#"{"target": "tiles"}"#,
                result: "tiles (o:6): joined 16 triangle pairs into quads; 0 triangles left; group quads_1.faces",
            }],
            aliases: &["tris to quads", "alt j", "join triangles", "quadrangulate", "polyQuad"],
            complexity: "O(n log n) for n selected triangles, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TrisToQuadsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        for (path, a, max) in [("/max_face_angle", p.max_face_angle, 180.0), ("/max_shape_angle", p.max_shape_angle, 90.0)] {
            if !(0.0..=max).contains(&a.degrees()) {
                return Err(OpError::out_of_range(path, format!("{} degrees is outside 0 to {max}", a.degrees())));
            }
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &TrisToQuadsParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = faces_or_all(tx.mesh(key, "/target")?, p.select.as_ref(), "edit.tris_to_quads")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "quads");
        let spec = QuadSpec {
            max_face_angle_deg: p.max_face_angle.degrees() as f32,
            max_shape_angle_deg: p.max_shape_angle.degrees() as f32,
            ignore_boundaries: p.advanced.ignore_boundaries,
        };
        let mesh = tx.mesh_mut(key, "/target")?;
        let quads = p3d_geo::edit::tris_to_quads(mesh, &faces, &spec).map_err(kernel_error)?;
        let mut out = OpOutput::default();
        if quads.is_empty() {
            out.summary = format!("{}: the {} selected faces have no triangle pairs to join", label(tx.doc(), key), thousands(faces.len()));
            return Ok(out);
        }
        let left = mesh.faces().filter(|&f| mesh.face_len(f) == 3).count();
        let g = face_group(mesh, key, format!("{glabel}.faces"), &quads)?;
        out.summary = format!(
            "{}: joined {} triangle pairs into quads; {} triangles left; group {}",
            label(tx.doc(), key),
            thousands(quads.len()),
            thousands(left),
            g.name
        );
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.split ------------------------------------------------------------------------

/// Parameters of `edit.split`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SplitParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to detach from the rest of the mesh.
    pub select: Selector,
}

impl Default for SplitParams {
    fn default() -> Self {
        SplitParams { target: String::new(), select: Selector::Faces(Filter::default()) }
    }
}

/// `edit.split`.
pub struct Split;

impl Op for Split {
    const ID: &'static str = "edit.split";
    type Params = SplitParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Split",
            category: Category::Edit,
            summary: "Detaches faces from the rest of the mesh by duplicating the vertices along the border, leaving them in place.",
            docs: "The selected faces become their own piece inside the same object: vertices where they meet unselected faces are \
                   duplicated, nothing moves. Faces keep their groups and are also `<label>.faces`.\n\n\
                   **Use for** a lid, door or panel that must move separately (follow with `scene.separate` by groups to make it its \
                   own object), hard splits for shading or UVs, or before flipping part of a surface.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}}"#,
                result: "crate (o:1): split 1 face off (4 vertices duplicated); group split_1.faces",
            }],
            aliases: &["split", "y", "detach", "rip faces", "polyChipOff", "extract"],
            complexity: "O(c log c) for the c corners of the selected faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SplitParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &SplitParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = selected_faces(tx.mesh(key, "/target")?, &p.select, "edit.split")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "split");
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::split(mesh, &faces).map_err(kernel_error)?;
        let mut out = OpOutput::default();
        if r.duplicated == 0 {
            out.summary = format!("{}: the {} selected faces are a separate piece already", label(tx.doc(), key), thousands(faces.len()));
            return Ok(out);
        }
        let g = face_group(mesh, key, format!("{glabel}.faces"), &r.faces)?;
        out.summary = format!(
            "{}: split {} face{} off ({} vertices duplicated); group {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            thousands(r.duplicated),
            g.name
        );
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.flip -------------------------------------------------------------------------

/// Parameters of `edit.flip`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlipParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces whose connected parts are flipped; every part when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
}

/// `edit.flip`.
pub struct Flip;

impl Op for Flip {
    const ID: &'static str = "edit.flip";
    type Params = FlipParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Flip normals",
            category: Category::Edit,
            summary: "Reverses the facing of every connected part that contains a selected face.",
            docs: "Faces always agree with their neighbours, so whole connected parts flip: the inside becomes the outside. To flip \
                   only some faces, `edit.split` them off first.\n\n\
                   **Use for** rooms and interiors seen from inside, or deliberately inward shells. To make everything face outward, \
                   use `edit.recalc_normals` instead.",
            examples: &[OpExample { args: r#"{"target": "bin"}"#, result: "bin (o:5): flipped 1 part (5 faces)" }],
            aliases: &["flip", "flip normals", "reverse normals", "invert normals", "polyNormal reverse"],
            complexity: "O(V + F + C) to find parts plus O(c log c) per flipped part",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FlipParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &FlipParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let faces = faces_or_all(mesh, p.select.as_ref(), "edit.flip")?;
        let (labels, _) = mesh.face_components();
        let mut parts: Vec<u32> = faces.iter().map(|&f| labels[f as usize]).collect();
        parts.sort_unstable();
        parts.dedup();
        // First face of each touched part, in face order.
        let mut firsts: Vec<u32> = Vec::new();
        for f in mesh.faces() {
            if let Ok(i) = parts.binary_search(&labels[f as usize]) {
                if firsts.len() <= i {
                    firsts.resize(i + 1, u32::MAX);
                }
                if firsts[i] == u32::MAX {
                    firsts[i] = f;
                }
            }
        }
        let mesh = tx.mesh_mut(key, "/target")?;
        let mut flipped = 0usize;
        for f in firsts {
            flipped += mesh.flip_part(f).map_err(|e| OpError::internal(e.to_string()))?;
        }
        let mut out = OpOutput::summary(format!(
            "{}: flipped {} part{} ({} faces)",
            label(tx.doc(), key),
            parts.len(),
            plural(parts.len()),
            thousands(flipped)
        ));
        out.changed.push(key);
        if flipped > faces.len() {
            out.warnings.push(Warning {
                message: format!(
                    "{} faces outside the selection flipped too: faces cannot face against their neighbours (edit.split the selection first to flip only it)",
                    thousands(flipped - faces.len())
                ),
                fix: None,
            });
        }
        Ok(out)
    }
}

// ---- edit.subdivide -----------------------------------------------------------------------

/// Parameters of `edit.subdivide`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubdivideParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Faces to subdivide; every face when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Times to split (1 to 6); each level makes about 4 times as many faces.
    #[serde(default = "one_level")]
    pub levels: u32,
}

fn one_level() -> u32 {
    1
}

impl Default for SubdivideParams {
    fn default() -> Self {
        SubdivideParams { target: String::new(), select: None, levels: 1 }
    }
}

/// `edit.subdivide`.
pub struct Subdivide;

impl Op for Subdivide {
    const ID: &'static str = "edit.subdivide";
    type Params = SubdivideParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Subdivide",
            category: Category::Edit,
            summary: "Splits faces into quads at edge and face middles without moving anything (flat subdivision).",
            docs: "Every selected face is split into one quad per corner (quads into 4, triangles into 3, n-gons into n) through new \
                   vertices at the middle of each edge and face; the shape stays exactly the same. Unselected neighbours gain the \
                   new edge vertices so no cracks open. New faces keep their groups and are also `<label>.faces`; sharp edges and \
                   seams stay on both halves.\n\n\
                   **Use for** more resolution before deforming (`deform.*`) or displacing, and for quad-only meshes from triangles. \
                   For smooth rounding use `subd.catmull_clark`.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "front"}}}"#,
                result: "crate (o:1): subdivided 1 face into 4 (1 level); 4 neighbours gained corners; group subdivide_1.faces",
            }],
            aliases: &["subdivide", "subdivide simple", "polySmooth linear", "tessellate", "split faces"],
            complexity: "O(4^levels x c log c) for the c corners of the selected faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SubdivideParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_count("/levels", p.levels, 1, p3d_geo::edit::MAX_SUBDIVIDE_LEVELS)
    }

    fn apply(tx: &mut Tx, p: &SubdivideParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = faces_or_all(tx.mesh(key, "/target")?, p.select.as_ref(), "edit.subdivide")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "subdivide");
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::subdivide(mesh, &faces, p.levels).map_err(kernel_error)?;
        let g = face_group(mesh, key, format!("{glabel}.faces"), &r.faces)?;
        let neighbours = match r.neighbours {
            0 => String::new(),
            1 => "; 1 neighbour gained corners".into(),
            n => format!("; {} neighbours gained corners", thousands(n)),
        };
        let mut out = OpOutput::summary(format!(
            "{}: subdivided {} face{} into {} ({} level{}){neighbours}; group {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            thousands(r.faces.len()),
            p.levels,
            plural(p.levels as usize),
            g.name
        ));
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.loop_cut -----------------------------------------------------------------------

/// Parameters of `edit.loop_cut`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopCutParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Edges that the cut crosses: each picks the ring of quads running
    /// across it (several edges of one ring cut it once).
    pub select: Selector,
    /// Parallel loops per ring (1 to 64).
    #[serde(default = "one_cut")]
    pub cuts: u32,
    /// Slides a single cut toward either side, from -1 to 1 (0 is the middle).
    #[serde(default)]
    pub offset: f32,
}

fn one_cut() -> u32 {
    1
}

impl Default for LoopCutParams {
    fn default() -> Self {
        LoopCutParams { target: String::new(), select: Selector::Edges(Filter::default()), cuts: 1, offset: 0.0 }
    }
}

/// `edit.loop_cut`.
pub struct LoopCut;

impl Op for LoopCut {
    const ID: &'static str = "edit.loop_cut";
    type Params = LoopCutParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Loop cut",
            category: Category::Edit,
            summary: "Cuts new edge loops across the ring of quads that runs through the selected edges.",
            docs: "From each selected edge the ring of quads across it is followed in both directions until it closes, reaches the \
                   boundary or meets a face that is not a quad (that face gains the cut points as corners). Every ring edge is split \
                   evenly `cuts` times and every ring quad becomes `cuts + 1` quads; `offset` slides a single cut toward one side. \
                   New edges are `<label>.edges`, the split quads `<label>.faces` (they keep their groups); sharp edges and seams \
                   stay on every piece.\n\n\
                   **Use for** support loops next to hard edges before subdivision, extra rows for bending (`deform.bend`), and \
                   splitting a face to extrude only part of it. Select one edge across the ring, for example a vertical side edge \
                   of a cylinder to cut around it.",
            examples: &[OpExample {
                args: r#"{"target": "post", "select": {"edges": {"in_box": {"min": [0.045, 0.2, -0.01], "max": [0.055, 0.8, 0.01]}}}}"#,
                result: "post (o:3): cut 1 ring with 1 loop: +8 faces; groups loop_cut_1.edges (8), loop_cut_1.faces (16)",
            }],
            aliases: &["loop cut", "ctrl r", "insert edge loop", "loop cut and slide", "polySplitRing", "swift loop"],
            complexity: "O(ring length x cuts) plus O(c log c) for the corners touched, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &LoopCutParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_count("/cuts", p.cuts, 1, p3d_geo::edit::MAX_CUTS)?;
        if !(p.offset.is_finite() && (-1.0..=1.0).contains(&p.offset)) {
            return Err(OpError::out_of_range("/offset", format!("offset {} is outside -1 to 1", p.offset)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &LoopCutParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let sel = evaluate(mesh, &p.select).map_err(|e| OpError::from_query(e, "/select"))?;
        if sel.kind != ElementKind::Edge {
            return Err(OpError::invalid(
                "/select",
                format!("edit.loop_cut needs edges across the ring, but the selector picks {}", sel.kind.as_str()),
            )
            .with_hint("select an edge the cut should cross, for example {\"edges\": {\"in_box\": ...}}"));
        }
        if sel.is_empty() {
            return Err(empty_selection("the selector matched no edges", "query shows what a selector picks"));
        }
        let edges: Vec<u32> = sel.iter().collect();
        let glabel = group_label(mesh, ctx, "loop_cut");
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::loop_cut(mesh, &edges, p.cuts, p.offset).map_err(kernel_error)?;
        let en =
            mesh.set_group(&format!("{glabel}.edges"), ElementKind::Edge, r.edges.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
        let eg = GroupInfo { object: key, name: format!("{glabel}.edges"), kind: ElementKind::Edge, count: en as u32 };
        let fg = face_group(mesh, key, format!("{glabel}.faces"), &r.faces)?;
        let mut out = OpOutput::summary(format!(
            "{}: cut {} ring{} with {} loop{}: +{} faces; groups {} ({}), {} ({})",
            label(tx.doc(), key),
            r.rings,
            plural(r.rings),
            p.cuts,
            plural(p.cuts as usize),
            thousands(r.faces.len() - r.faces.len() / (p.cuts as usize + 1)),
            eg.name,
            eg.count,
            fg.name,
            fg.count
        ));
        out.changed.push(key);
        out.groups = vec![eg, fg];
        Ok(out)
    }
}

// ---- edit.bridge ----------------------------------------------------------------------

/// Parameters of `edit.bridge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BridgeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Boundary edges (or vertices, or faces next to them) of exactly two
    /// holes; when omitted the mesh must have exactly two holes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Rings of quads along the bridge (1 to 64).
    #[serde(default = "one_cut")]
    pub segments: u32,
}

impl Default for BridgeParams {
    fn default() -> Self {
        BridgeParams { target: String::new(), select: None, segments: 1 }
    }
}

/// `edit.bridge`.
pub struct Bridge;

impl Op for Bridge {
    const ID: &'static str = "edit.bridge";
    type Params = BridgeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Bridge",
            category: Category::Edit,
            summary: "Joins two holes (boundary loops) with a band of quads.",
            docs: "The two holes are matched vertex to vertex at the rotation with the shortest total connection; the band follows \
                   the surface's facing. Holes with different vertex counts get a few triangles. `segments` adds straight rings \
                   along the band. New faces are `<label>.faces`.\n\n\
                   **Use for** connecting two open tube ends (pipes, limbs), closing a gap between two parts after `scene.join`, \
                   and making a hole through a solid (delete two facing faces, then bridge). Bridge holes that face each other; \
                   far-apart or twisted holes give a stretched band.",
            examples: &[OpExample {
                args: r#"{"target": "duct", "select": {"edges": {"in_box": {"min": [-1, 0.39, -1], "max": [1, 0.61, 1]}}}}"#,
                result: "duct (o:7): bridged 2 holes with 8 faces; group bridge_1.faces",
            }],
            aliases: &["bridge edge loops", "bridge", "connect loops", "polyBridgeEdge", "loft holes"],
            complexity: "O(a x b) to align holes of a and b vertices, plus the patch",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &BridgeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_count("/segments", p.segments, 1, p3d_geo::edit::MAX_CUTS)
    }

    fn apply(tx: &mut Tx, p: &BridgeParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let starts = boundary_starts(mesh, p.select.as_ref())?;
        // One boundary half-edge per hole, in order of first appearance.
        let mut seen = p3d_mesh::FixedBitSet::with_capacity(mesh.he_slots());
        let mut holes: Vec<u32> = Vec::new();
        for h in starts {
            if seen.contains(h as usize) {
                continue;
            }
            for x in mesh.boundary_loop(h).map_err(|e| OpError::internal(e.to_string()))? {
                seen.insert(x as usize);
            }
            holes.push(h);
        }
        if holes.len() != 2 {
            return Err(OpError::new(
                ErrorCode::EmptySelection,
                format!("the selection touches {} hole{}, bridge needs 2", holes.len(), plural(holes.len())),
            )
            .at("/select")
            .with_hint("select boundary edges of exactly two holes; diagnose lists open boundaries"));
        }
        let glabel = group_label(mesh, ctx, "bridge");
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::bridge(mesh, holes[0], holes[1], p.segments).map_err(kernel_error)?;
        let g = face_group(mesh, key, format!("{glabel}.faces"), &r.faces)?;
        let mut out =
            OpOutput::summary(format!("{}: bridged 2 holes with {} faces; group {}", label(tx.doc(), key), thousands(r.faces.len()), g.name));
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

/// Vertices picked by a selector: selected vertices, or the vertices of
/// selected edges or faces.
///
/// Complexity: O(n) plus the selected elements' vertices.
fn selected_verts(mesh: &Mesh, select: &Selector) -> Result<Vec<u32>, OpError> {
    let sel = evaluate(mesh, select).map_err(|e| OpError::from_query(e, "/select"))?;
    let mut out: Vec<u32> = match sel.kind {
        ElementKind::Vert => sel.iter().collect(),
        ElementKind::Edge => sel.iter().flat_map(|e| mesh.edge_verts(e)).collect(),
        ElementKind::Face => sel.iter().flat_map(|f| mesh.face_verts(f).collect::<Vec<_>>()).collect(),
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(empty_selection("the selector matched nothing", "query shows what a selector picks"));
    }
    Ok(out)
}

// ---- edit.merge --------------------------------------------------------------------------

/// How `edit.merge` groups vertices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MergeModeParam {
    /// Every selected vertex becomes one, at their center.
    #[default]
    Center,
    /// Each connected run of selected vertices becomes one, at its own center.
    Collapse,
}

/// Parameters of `edit.merge`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to merge (edges and faces select their vertices).
    pub select: Selector,
    /// `center` (default) or `collapse`.
    #[serde(default)]
    pub mode: MergeModeParam,
}

impl Default for MergeParams {
    fn default() -> Self {
        MergeParams { target: String::new(), select: Selector::Verts(Filter::default()), mode: MergeModeParam::Center }
    }
}

/// `edit.merge`.
pub struct Merge;

impl Op for Merge {
    const ID: &'static str = "edit.merge";
    type Params = MergeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Merge",
            category: Category::Edit,
            summary: "Merges selected vertices into one point (center) or each connected run into its own point (collapse).",
            docs: "`center` merges every selected vertex into one at their average position; `collapse` merges each run of selected \
                   vertices joined by edges separately (collapse a ring of edges into points, a face into its center). Faces that \
                   lose corners shrink, faces left with fewer than three corners disappear, faces pinched at the merged point split \
                   there.\n\n\
                   **Use for** closing the tip of a cone-like shape, collapsing an edge loop to a point, removing a thin face. To \
                   weld seams of mirrored or joined parts use `edit.merge_by_distance`.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "select": {"faces": {"group": "top"}}, "mode": "collapse"}"#,
                result: "crate (o:1): merged 4 vertices into 1 point; 1 face collapsed",
            }],
            aliases: &["merge", "m", "merge at center", "collapse", "polyMergeVertex", "weld"],
            complexity: "O(k + c log c) for k vertices and the c corners of their faces, plus the selector",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &MergeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &MergeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let verts = selected_verts(tx.mesh(key, "/target")?, &p.select)?;
        let mode = match p.mode {
            MergeModeParam::Center => MergeMode::Center,
            MergeModeParam::Collapse => MergeMode::Collapse,
        };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::merge(mesh, &verts, mode).map_err(kernel_error)?;
        let collapsed = match r.faces_removed {
            0 => String::new(),
            1 => "; 1 face collapsed".into(),
            n => format!("; {} faces collapsed", thousands(n)),
        };
        let mut out = OpOutput::summary(format!(
            "{}: merged {} vertices into {} point{}{collapsed}",
            label(tx.doc(), key),
            thousands(r.merged + r.clusters),
            thousands(r.clusters),
            plural(r.clusters)
        ));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- edit.merge_by_distance --------------------------------------------------------------

/// Parameters of `edit.merge_by_distance`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MergeByDistanceParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices that may weld (edges and faces select their vertices); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Vertices closer than this weld together.
    #[serde(default = "weld_distance")]
    pub distance: Length,
}

fn weld_distance() -> Length {
    Length(1e-4)
}

impl Default for MergeByDistanceParams {
    fn default() -> Self {
        MergeByDistanceParams { target: String::new(), select: None, distance: weld_distance() }
    }
}

/// `edit.merge_by_distance`.
pub struct MergeByDistance;

impl Op for MergeByDistance {
    const ID: &'static str = "edit.merge_by_distance";
    type Params = MergeByDistanceParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Merge by distance",
            category: Category::Edit,
            summary: "Welds vertices that lie closer together than a distance, closing seams between joined or mirrored parts.",
            docs: "Each cluster of nearby vertices (chains weld together) becomes one vertex at the cluster's center, so open borders \
                   lying on each other zip into shared edges. The default distance (0.1 mm) welds only coincident vertices.\n\n\
                   **Use for** after `scene.join` of parts that touch along open borders, after mirroring, and on imported meshes \
                   with split vertices (`diagnose` reports open boundaries). Keep the distance well below the smallest edge, or \
                   detail collapses.",
            examples: &[OpExample { args: r#"{"target": "panels"}"#, result: "panels (o:9): welded 2 vertices into 2 others (0 faces collapsed)" }],
            aliases: &["merge by distance", "remove doubles", "weld vertices", "polyMergeVertex distance", "weld"],
            complexity: "O(n log n) for the n vertices taking part (sorted uniform grid), plus the corners of affected faces",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &MergeByDistanceParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_length("/distance", p.distance, 1e-9, p3d_geo::prim::MAX_SIZE)
    }

    fn apply(tx: &mut Tx, p: &MergeByDistanceParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let only = match &p.select {
            Some(s) => Some(selected_verts(tx.mesh(key, "/target")?, s)?),
            None => None,
        };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::merge_by_distance(mesh, only.as_deref(), p.distance.0 as f32).map_err(kernel_error)?;
        let mut out = OpOutput::default();
        if r.merged == 0 {
            out.summary = format!("{}: no vertices within {} of each other", label(tx.doc(), key), p3d_math::format_length(p.distance.0));
            return Ok(out);
        }
        out.summary = format!(
            "{}: welded {} vertices into {} others ({} faces collapsed)",
            label(tx.doc(), key),
            thousands(r.merged),
            thousands(r.clusters),
            thousands(r.faces_removed)
        );
        out.changed.push(key);
        Ok(out)
    }
}

// ---- edit.dissolve -----------------------------------------------------------------------

/// What `edit.dissolve` removes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DissolveModeParam {
    /// Follows the selector: vertices, edges or faces.
    #[default]
    Auto,
    /// Merges faces across edges flatter than `angle`, then drops vertices on straight lines.
    Limited,
}

/// Parameters of `edit.dissolve`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DissolveParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices, edges or faces to dissolve; for `limited`, the faces to
    /// simplify (all when omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// `auto` (by the selector's element kind) or `limited`.
    #[serde(default)]
    pub mode: DissolveModeParam,
    /// For `limited`: faces meeting flatter than this merge (degrees).
    #[serde(default = "five_deg")]
    pub angle: Angle,
}

fn five_deg() -> Angle {
    Angle::from_degrees(5.0)
}

impl Default for DissolveParams {
    fn default() -> Self {
        DissolveParams { target: String::new(), select: None, mode: DissolveModeParam::Auto, angle: five_deg() }
    }
}

/// `edit.dissolve`.
pub struct Dissolve;

impl Op for Dissolve {
    const ID: &'static str = "edit.dissolve";
    type Params = DissolveParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Dissolve",
            category: Category::Edit,
            summary: "Removes vertices, edges or faces while keeping the surface closed, merging the faces around them into polygons.",
            docs: "By the selector's kind: **vertices** with two edges are taken out of their faces, others disappear with their fan \
                   of faces merged into one polygon; **edges** merge their two faces (end vertices left on a straight line go too); \
                   **faces** merge each connected region into one polygon over its border. `mode: \"limited\"` merges faces across \
                   edges flatter than `angle` (not across sharp edges, seams or material borders) and drops vertices on straight \
                   lines. Merged polygons keep groups and are `<label>.faces`. Regions that would need a hole are skipped.\n\n\
                   **Use for** reducing flat areas of hard-surface models (limited, 1 to 5 degrees), removing extra edge loops \
                   (select the loop's edges), and simplifying after booleans.",
            examples: &[OpExample {
                args: r#"{"target": "tiles", "mode": "limited"}"#,
                result: "tiles (o:6): dissolved into 1 polygon (21 vertices and 52 edges fewer); group dissolve_1.faces",
            }],
            aliases: &["dissolve", "limited dissolve", "dissolve edges", "dissolve vertices", "dissolve faces", "delete edge loop", "polyDelEdge"],
            complexity: "O(c log c) for the c corners of the touched faces, plus the selector",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &DissolveParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if p.mode == DissolveModeParam::Auto && p.select.is_none() {
            return Err(OpError::invalid("/select", "dissolve needs a selection, or mode \"limited\" to simplify the whole mesh")
                .with_hint("for example {\"edges\": {\"group\": \"loop_cut_1.edges\"}}"));
        }
        if !(0.0..=180.0).contains(&p.angle.degrees()) {
            return Err(OpError::out_of_range("/angle", format!("{} degrees is outside 0 to 180", p.angle.degrees())));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &DissolveParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let glabel = group_label(mesh, ctx, "dissolve");
        let (verts_before, edges_before) = (mesh.vert_count(), mesh.edge_count());
        let r = match (p.mode, &p.select) {
            (DissolveModeParam::Limited, s) => {
                let faces = faces_or_all(mesh, s.as_ref(), "edit.dissolve")?;
                let mesh = tx.mesh_mut(key, "/target")?;
                p3d_geo::edit::dissolve_limited(mesh, &faces, p.angle.degrees() as f32)
            }
            (DissolveModeParam::Auto, Some(s)) => {
                let sel = evaluate(mesh, s).map_err(|e| OpError::from_query(e, "/select"))?;
                if sel.is_empty() {
                    return Err(empty_selection("the selector matched nothing", "query shows what a selector picks"));
                }
                let items: Vec<u32> = sel.iter().collect();
                let mesh = tx.mesh_mut(key, "/target")?;
                match sel.kind {
                    ElementKind::Vert => p3d_geo::edit::dissolve_verts(mesh, &items),
                    ElementKind::Edge => p3d_geo::edit::dissolve_edges(mesh, &items, true),
                    ElementKind::Face => p3d_geo::edit::dissolve_faces(mesh, &items),
                }
            }
            (DissolveModeParam::Auto, None) => return Err(OpError::invalid("/select", "dissolve needs a selection")),
        }
        .map_err(kernel_error)?;
        let mesh = tx.mesh_mut(key, "/target")?;
        let mut out = OpOutput::default();
        let mut faces = r.faces.clone();
        faces.retain(|&f| mesh.is_face_alive(f));
        let (gone_v, gone_e) = (verts_before.saturating_sub(mesh.vert_count()), edges_before.saturating_sub(mesh.edge_count()));
        if gone_v == 0 && gone_e == 0 {
            out.summary = format!("{}: nothing could be dissolved ({} skipped)", label(tx.doc(), key), r.skipped);
            return Ok(out);
        }
        let g = face_group(mesh, key, format!("{glabel}.faces"), &faces)?;
        out.summary = format!(
            "{}: dissolved into {} polygon{} ({} vertices and {} edges fewer); group {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            thousands(gone_v),
            thousands(gone_e),
            g.name
        );
        out.changed.push(key);
        out.groups = vec![g];
        if r.skipped > 0 {
            out.warnings.push(Warning {
                message: format!("{} selected element(s) were left alone (on the boundary, or a merge would need a hole)", r.skipped),
                fix: None,
            });
        }
        Ok(out)
    }
}

// ---- edit.plane_cut -----------------------------------------------------------------------

/// Which side of the plane `edit.plane_cut` keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeepParam {
    /// Both sides: just cut.
    #[default]
    Both,
    /// The side the normal points to.
    Above,
    /// The other side.
    Below,
}

/// Parameters of `edit.plane_cut`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaneCutParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// A point on the plane, in object space.
    #[serde(default)]
    pub point: [Length; 3],
    /// Plane normal in object space; [0, 1, 0] cuts horizontally.
    #[serde(default = "up")]
    pub normal: [f32; 3],
    /// `both` (default), `above` (the side the normal points to) or `below`.
    #[serde(default)]
    pub keep: KeepParam,
    /// Close the holes the cut leaves on the plane (when one side is removed).
    #[serde(default)]
    pub fill: bool,
}

fn up() -> [f32; 3] {
    [0.0, 1.0, 0.0]
}

impl Default for PlaneCutParams {
    fn default() -> Self {
        PlaneCutParams { target: String::new(), point: [Length(0.0); 3], normal: up(), keep: KeepParam::Both, fill: false }
    }
}

/// `edit.plane_cut`.
pub struct PlaneCut;

impl Op for PlaneCut {
    const ID: &'static str = "edit.plane_cut";
    type Params = PlaneCutParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Plane cut",
            category: Category::Edit,
            summary: "Cuts the mesh along a plane, optionally removing one side and capping the opening.",
            docs: "Every face crossing the plane is split along it (new vertices where edges cross; concave faces are triangulated \
                   first). With `keep: \"above\"` or `\"below\"` the other side is deleted, and `fill` closes the resulting holes \
                   with flat caps. The cut edges are `<label>.edges`, caps `<label>.cap`.\n\n\
                   **Use for** cutting a shape in half (then `edit.symmetrize` or mirror), flattening a base so a prop stands, \
                   adding an edge loop at an exact height, trimming terrain or architecture at a level.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "point": [0, "40cm", 0], "keep": "below", "fill": true}"#,
                result: "crate (o:1): cut 4 faces along the plane; removed 1 face; 1 cap; groups plane_cut_1.edges (4), plane_cut_1.cap (1)",
            }],
            aliases: &["bisect", "plane cut", "slice", "knife project plane", "polyCut", "cut in half"],
            complexity: "O(V + F + C) to classify, plus O(c log c) for the corners of cut and removed faces",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &PlaneCutParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if !(p.normal.iter().all(|x| x.is_finite()) && p.normal.iter().any(|x| *x != 0.0)) {
            return Err(OpError::invalid("/normal", "normal must be a finite, non-zero vector").with_hint("[0, 1, 0] cuts horizontally"));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &PlaneCutParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "plane_cut");
        let keep = match p.keep {
            KeepParam::Both => Keep::Both,
            KeepParam::Above => Keep::Above,
            KeepParam::Below => Keep::Below,
        };
        let spec = PlaneCutSpec { point: crate::params::vec3(&p.point), normal: glam::Vec3::from(p.normal), keep, fill: p.fill };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::plane_cut(mesh, &spec).map_err(kernel_error)?;
        let mut out = OpOutput::default();
        if r.faces_cut == 0 && r.faces_removed == 0 {
            out.summary = format!("{}: the plane misses the mesh; nothing changed", label(tx.doc(), key));
            return Ok(out);
        }
        let en =
            mesh.set_group(&format!("{glabel}.edges"), ElementKind::Edge, r.edges.iter().copied()).map_err(|e| OpError::internal(e.to_string()))?;
        out.groups.push(GroupInfo { object: key, name: format!("{glabel}.edges"), kind: ElementKind::Edge, count: en as u32 });
        if !r.caps.is_empty() {
            out.groups.push(face_group(mesh, key, format!("{glabel}.cap"), &r.caps)?);
        }
        let removed = match r.faces_removed {
            0 => String::new(),
            n => format!("; removed {} face{}", thousands(n), plural(n)),
        };
        let caps = match r.caps.len() {
            0 => String::new(),
            n => format!("; {n} cap{}", plural(n)),
        };
        let groups: Vec<String> = out.groups.iter().map(|g| format!("{} ({})", g.name, g.count)).collect();
        out.summary = format!(
            "{}: cut {} face{} along the plane{removed}{caps}; groups {}",
            label(tx.doc(), key),
            thousands(r.faces_cut),
            plural(r.faces_cut),
            groups.join(", ")
        );
        out.changed.push(key);
        Ok(out)
    }
}

// ---- edit.symmetrize -----------------------------------------------------------------------

/// An axis of object space.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AxisParam {
    #[default]
    X,
    Y,
    Z,
}

/// Parameters of `edit.symmetrize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymmetrizeParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Mirror across the plane through the origin normal to this axis.
    #[serde(default)]
    pub axis: AxisParam,
    /// Keep the positive half (default) and mirror it onto the negative one; false does the opposite.
    #[serde(default = "yes")]
    pub keep_positive: bool,
    /// Vertices this close to the plane weld into the seam.
    #[serde(default = "weld_distance")]
    pub weld: Length,
}

fn yes() -> bool {
    true
}

impl Default for SymmetrizeParams {
    fn default() -> Self {
        SymmetrizeParams { target: String::new(), axis: AxisParam::X, keep_positive: true, weld: weld_distance() }
    }
}

/// `edit.symmetrize`.
pub struct Symmetrize;

impl Op for Symmetrize {
    const ID: &'static str = "edit.symmetrize";
    type Params = SymmetrizeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Symmetrize",
            category: Category::Edit,
            summary: "Makes a mesh mirror-symmetric: keeps one half and replaces the other with its mirror image, welded at the seam.",
            docs: "The mesh is cut at the plane through the object origin normal to `axis`, the kept half (positive by default: +X is \
                   the right side) is mirrored onto the other side and seam vertices within `weld` are merged. Mirrored faces are \
                   `<label>.mirrored` and keep their groups.\n\n\
                   **Use for** characters, vehicles, weapons and props that should be exactly symmetric after editing one side. \
                   The object origin must lie on the symmetry plane (`scene.set_origin` first if needed).",
            examples: &[OpExample {
                args: r#"{"target": "post", "axis": "x"}"#,
                result: "post (o:3): mirrored the +X half onto -X (16 faces); welded 10 seam vertices; group symmetrize_1.mirrored",
            }],
            aliases: &["symmetrize", "mirror mesh", "make symmetric", "polyMirrorFace", "symmetry"],
            complexity: "O(V + F + C) plus the seam weld, O(n log n) in seam vertices",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SymmetrizeParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_length("/weld", p.weld, 1e-9, 1.0)
    }

    fn apply(tx: &mut Tx, p: &SymmetrizeParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "symmetrize");
        let axis = match p.axis {
            AxisParam::X => Axis::X,
            AxisParam::Y => Axis::Y,
            AxisParam::Z => Axis::Z,
        };
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::symmetrize(mesh, axis, p.keep_positive, p.weld.0 as f32).map_err(kernel_error)?;
        let g = face_group(mesh, key, format!("{glabel}.mirrored"), &r.mirrored)?;
        let a = ["X", "Y", "Z"][axis as usize];
        let (from, to) = if p.keep_positive { ("+", "-") } else { ("-", "+") };
        let mut out = OpOutput::summary(format!(
            "{}: mirrored the {from}{a} half onto {to}{a} ({} faces); welded {} seam vertices; group {}",
            label(tx.doc(), key),
            thousands(r.mirrored.len()),
            thousands(r.welded),
            g.name
        ));
        out.changed.push(key);
        out.groups = vec![g];
        Ok(out)
    }
}

// ---- edit.thicken ------------------------------------------------------------------------

/// Parameters of `edit.thicken`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThickenParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Whole surfaces to thicken; every face when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Wall thickness.
    #[serde(default = "default_inset_thickness")]
    pub thickness: Length,
    /// Where the wall goes: -1 behind the surface (default), 0 centered, 1 in front.
    #[serde(default = "minus_one")]
    pub offset: f32,
}

fn minus_one() -> f32 {
    -1.0
}

impl Default for ThickenParams {
    fn default() -> Self {
        ThickenParams { target: String::new(), select: None, thickness: default_inset_thickness(), offset: -1.0 }
    }
}

/// `edit.thicken`.
pub struct Thicken;

impl Op for Thicken {
    const ID: &'static str = "edit.thicken";
    type Params = ThickenParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Thicken",
            category: Category::Edit,
            summary: "Gives surfaces a wall thickness: an offset back side joined to the front along open borders.",
            docs: "A reversed copy of the faces is offset along the vertex normals (stretched at creases so every face keeps the full \
                   thickness) and joined to the original along open borders; closed parts become hollow shells. `offset` -1 keeps \
                   the surface as the outside. Back faces are `<label>.inner`, border walls `<label>.rim`.\n\n\
                   **Use for** sheet metal, cloth and paper with visible edges, walls and floors from planes, cups and containers \
                   from open shells. Typical thickness: 2 to 10 mm for props, 0.1 to 0.3 m for architecture.\n\n\
                   **Select whole surfaces** (or nothing): walls only grow along the mesh's open borders.",
            examples: &[OpExample {
                args: r#"{"target": "bin", "thickness": "1cm"}"#,
                result: "bin (o:5): thickened 5 faces by 10 mm: +5 inner and 4 rim faces; groups thicken_1.inner, thicken_1.rim",
            }],
            aliases: &["solidify", "thicken", "shell", "polyExtrude thickness", "wall thickness"],
            complexity: "O(c log c) for the c corners of the faces",
            flags: EDIT.union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &ThickenParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_length("/thickness", p.thickness, 1e-6, p3d_geo::prim::MAX_SIZE)?;
        if !(p.offset.is_finite() && (-1.0..=1.0).contains(&p.offset)) {
            return Err(OpError::out_of_range("/offset", format!("offset {} is outside -1 to 1", p.offset)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &ThickenParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let faces = faces_or_all(tx.mesh(key, "/target")?, p.select.as_ref(), "edit.thicken")?;
        let glabel = group_label(tx.mesh(key, "/target")?, ctx, "thicken");
        let mesh = tx.mesh_mut(key, "/target")?;
        let r = p3d_geo::edit::thicken(mesh, &faces, &ThickenSpec { thickness: p.thickness.0 as f32, offset: p.offset }).map_err(kernel_error)?;
        let inner = face_group(mesh, key, format!("{glabel}.inner"), &r.inner)?;
        let rim = face_group(mesh, key, format!("{glabel}.rim"), &r.rim)?;
        let mut out = OpOutput::summary(format!(
            "{}: thickened {} face{} by {}: +{} inner and {} rim faces; groups {}, {}",
            label(tx.doc(), key),
            thousands(faces.len()),
            plural(faces.len()),
            p3d_math::format_length(p.thickness.0),
            thousands(r.inner.len()),
            thousands(r.rim.len()),
            inner.name,
            rim.name
        ));
        out.changed.push(key);
        out.groups = vec![inner, rim];
        Ok(out)
    }
}

// ---- edit.smooth ----------------------------------------------------------------------------

/// Advanced settings of `edit.smooth`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SmoothAdvanced {
    /// Keep open-border vertices in place.
    pub pin_boundary: bool,
    /// Keep vertices on sharp edges in place.
    pub pin_sharp: bool,
}

impl Default for SmoothAdvanced {
    fn default() -> Self {
        SmoothAdvanced { pin_boundary: true, pin_sharp: true }
    }
}

/// Parameters of `edit.smooth`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmoothParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Vertices to smooth (edges and faces select their vertices); all when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<Selector>,
    /// Rounds of smoothing (1 to 1000).
    #[serde(default = "ten")]
    pub iterations: u32,
    /// Strength per round, 0 to 1.
    #[serde(default = "half")]
    pub factor: f32,
    /// Rarely needed settings.
    #[serde(default)]
    pub advanced: SmoothAdvanced,
}

fn ten() -> u32 {
    10
}

fn half() -> f32 {
    0.5
}

impl Default for SmoothParams {
    fn default() -> Self {
        SmoothParams { target: String::new(), select: None, iterations: 10, factor: 0.5, advanced: SmoothAdvanced::default() }
    }
}

/// `edit.smooth`.
pub struct Smooth;

impl Op for Smooth {
    const ID: &'static str = "edit.smooth";
    type Params = SmoothParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Smooth",
            category: Category::Edit,
            summary: "Smooths vertex positions without shrinking the shape (Taubin smoothing).",
            docs: "Each round moves every free vertex toward the average of its neighbours, then back by a slightly larger step, which \
                   removes bumps and noise but keeps the overall size. Open borders and sharp edges stay pinned unless turned off \
                   under `advanced`.\n\n\
                   **Use for** relaxing lumpy organic shapes, after `deform.displace` with too much noise, and evening out edge \
                   flow before subdivision. **Not for** rounding hard edges (use `edit.bevel` or `subd.catmull_clark`).",
            examples: &[OpExample { args: r#"{"target": "tiles", "iterations": 5}"#, result: "tiles (o:6): smoothed 9 vertices (5 rounds)" }],
            aliases: &["smooth vertices", "relax", "laplacian smooth", "polyAverageVertex", "taubin"],
            complexity: "O(iterations x sum of valences) of the smoothed vertices",
            flags: EDIT,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &SmoothParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        crate::common::check_count("/iterations", p.iterations, 1, p3d_geo::edit::MAX_SMOOTH_ITERATIONS)?;
        if !(p.factor.is_finite() && p.factor > 0.0 && p.factor <= 1.0) {
            return Err(OpError::out_of_range("/factor", format!("factor {} is outside 0 to 1", p.factor)));
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &SmoothParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let mesh = tx.mesh(key, "/target")?;
        let verts = match &p.select {
            Some(s) => selected_verts(mesh, s)?,
            None => mesh.verts().collect(),
        };
        let spec = SmoothSpec { iterations: p.iterations, factor: p.factor, pin_boundary: p.advanced.pin_boundary, pin_sharp: p.advanced.pin_sharp };
        let mesh = tx.mesh_mut(key, "/target")?;
        let moved = p3d_geo::edit::smooth(mesh, &verts, &spec).map_err(kernel_error)?;
        let mut out = OpOutput::summary(format!(
            "{}: smoothed {} vert{} ({} round{})",
            label(tx.doc(), key),
            thousands(moved),
            if moved == 1 { "ex" } else { "ices" },
            p.iterations,
            plural(p.iterations as usize)
        ));
        if moved == 0 {
            out.warnings.push(Warning { message: "every selected vertex is pinned (border or sharp edge); see advanced settings".into(), fix: None });
        } else {
            out.changed.push(key);
        }
        Ok(out)
    }
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
