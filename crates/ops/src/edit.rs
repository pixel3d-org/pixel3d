//! `edit.*`: mesh edits addressed by selectors and groups.

use p3d_geo::edit::{ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, KernelError, QuadSpec, extrude, inset};
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
