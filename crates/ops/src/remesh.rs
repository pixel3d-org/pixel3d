//! `remesh.*`: new topology for an existing surface (kernels in
//! `p3d_remesh`).

use glam::Affine3A;
use p3d_geo::bvh::TriBvh;
use p3d_mesh::{ElementKind, Mesh};
use p3d_remesh::{QuadReport, QuadSpec, RemeshError, quad_remesh};
use p3d_scene::Document;
use p3d_sdf::{MeshField, MeshSpec, mesh as sdf_mesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_count, check_length, label, mesh_target, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length};
use crate::tx::Tx;

/// Parameters of `remesh.quad`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuadRemeshParams {
    /// Mesh object (name, path or id); it must be closed.
    pub target: String,
    /// Target edge length; about a fortieth of the largest side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<Length>,
    /// Smoothing sweeps of the direction and position fields per level (more is smoother and slower).
    #[serde(default = "twelve")]
    pub iterations: u32,
    /// Relaxation sweeps onto the original surface.
    #[serde(default = "four")]
    pub relax: u32,
    /// Edge loops circling points (eyes, mouth, joints): `{"at": "eye_socket", "radius": "3cm"}`; `at` is world coordinates or a group's center.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loops: Vec<LoopParam>,
    /// Flow along an object axis (limbs, blades, tails).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<crate::edit::AxisParam>,
}

/// Edge loops around a point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoopParam {
    pub at: crate::deform::PointParam,
    /// Outer radius of the loops; an eighth of the mesh's largest side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<Length>,
}

// Forms and the op docs start from these, so they match serde's defaults.
impl Default for QuadRemeshParams {
    fn default() -> Self {
        QuadRemeshParams { target: String::new(), edge: None, iterations: twelve(), relax: four(), loops: Vec::new(), align: None }
    }
}

fn twelve() -> u32 {
    12
}

fn four() -> u32 {
    4
}

/// `remesh.quad`.
pub struct QuadRemesh;

fn remesh_error(e: RemeshError) -> OpError {
    match e {
        RemeshError::BadEdge => OpError::out_of_range("/edge", e.to_string()),
        RemeshError::Empty => OpError::new(ErrorCode::EmptySelection, "the mesh has no faces").at("/target"),
        RemeshError::TooLarge(n) => {
            OpError::new(ErrorCode::BudgetExceeded, format!("{} samples would be needed", thousands(n))).at("/edge").with_hint("use a longer edge")
        }
        RemeshError::Build(m) => OpError::new(ErrorCode::Topology, m).at("/edge").with_hint("try a shorter edge, or cleanup.auto the mesh first"),
    }
}

/// Face groups and materials of the old faces, carried to the new faces by
/// the nearest old face.
///
/// Complexity: O(F log n) for F new faces over n old triangles, plus
/// O(F x G) for G face groups.
fn carry(old: &Mesh, new: &mut Mesh) {
    let bvh = TriBvh::from_mesh(old, Affine3A::IDENTITY);
    let reach = old.bounds().map_or(1.0, |(lo, hi)| (hi - lo).length());
    let faces: Vec<u32> = new.faces().collect();
    let src: Vec<Option<u32>> = faces
        .iter()
        .map(|&f| {
            let c = new.face_verts(f).map(|v| new.positions[v as usize]).sum::<glam::Vec3>() / new.face_len(f) as f32;
            bvh.closest(c, reach).map(|h| h.face)
        })
        .collect();
    for (&f, s) in faces.iter().zip(&src) {
        if let Some(s) = s {
            new.face_material[f as usize] = old.face_material(*s);
        }
    }
    for (name, g) in old.groups().filter(|(_, g)| g.kind() == ElementKind::Face) {
        let list: Vec<u32> = faces.iter().zip(&src).filter(|(_, s)| s.is_some_and(|s| g.contains(s))).map(|(&f, _)| f).collect();
        if !list.is_empty() {
            // The faces are live, so this cannot fail.
            let _ = new.set_group(name, ElementKind::Face, list);
        }
    }
}

impl Op for QuadRemesh {
    const ID: &'static str = "remesh.quad";
    type Params = QuadRemeshParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Quad remesh",
            category: Category::Remesh,
            summary: "Rebuilds a closed mesh as all quads with even edges that follow a smooth direction field.",
            docs: "Field-aligned quad remeshing (Instant Meshes, Jakob et al. 2015): a smooth direction field and a lattice of \
                   points about `edge` apart are solved on the surface, quads are extracted from the lattice and subdivided \
                   once, so every face is a quad. Before subdividing, local edits (merging triangle pairs, turning edges, splitting \
                   vertices of valence 6 or more and big faces) lower the poles, so none has valence 2 or above 6. The new vertices are relaxed onto \
                   the old surface; face groups and materials carry over by the nearest old face. UVs, creases and edge groups \
                   do not.\n\n\
                   **Use for** turning `sdf.mesh` results, boolean-heavy blockouts and scans into clean, even quads before \
                   UVs, subdivision or rigging. Hard edges round off: remesh organic parts, model hard-surface parts directly. \
                   `loops` lay edge loops around points (eyes, mouth, joints) and `align` runs the flow along an axis; a \
                   spec part with `humanoid_head` landmarks gets loops around its eyes and mouth by itself.",
            examples: &[OpExample {
                args: r#"{"target": "post", "edge": "8mm"}"#,
                result: "post (o:3): remeshed into 5,196 quads (5,196 faces, edge 8 mm); poles 150 of valence 3, 142 of 5, 0 of 6, 0 of 7 or more",
            }],
            aliases: &["quad remesh", "quadriflow", "instant meshes", "retopology", "retopo", "zremesher", "autoretopo"],
            complexity: "O(S x iterations) for the S samples (about area / (edge / 1.25)^2), plus O(S log S) for the hierarchy",
            flags: OpFlags::UNDOABLE.union(OpFlags::LONG_RUNNING),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &QuadRemeshParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if let Some(e) = p.edge {
            check_length("/edge", e, 1e-4, 10.0)?;
        }
        check_count("/iterations", p.iterations, 1, 100)?;
        check_count("/relax", p.relax, 0, 50)
    }

    fn apply(tx: &mut Tx, p: &QuadRemeshParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let old = tx.mesh(key, "/target")?;
        let open = old.edges().filter(|&e| old.is_boundary_edge(e)).count();
        if open > 0 {
            return Err(OpError::new(
                ErrorCode::NonManifoldInput,
                format!("{} has {} open edges; quad remesh needs a closed surface", label(tx.doc(), key), thousands(open)),
            )
            .at("/target")
            .with_hint("close it with edit.fill or cleanup.auto first"));
        }
        let size = old.bounds().map_or(1.0, |(lo, hi)| (hi - lo).max_element());
        let edge = p.edge.map_or(size / 40.0, |e| e.0 as f32);
        // Design rules for the flow, in the mesh's space.
        let to_local = tx.doc().world_affine(key).unwrap_or_default().inverse();
        let mut guides = Vec::new();
        for (i, l) in p.loops.iter().enumerate() {
            let path = format!("/loops/{i}/at");
            let center = match &l.at {
                crate::deform::PointParam::Group(g) => crate::deform::group_center(old, g, &path)?,
                crate::deform::PointParam::At(c) => to_local.transform_point3(glam::Vec3::new(c[0].0 as f32, c[1].0 as f32, c[2].0 as f32)),
            };
            guides.push(p3d_remesh::Guide::Ring { center, radius: l.radius.map_or(size / 8.0, |r| r.0 as f32) });
        }
        if let Some(a) = p.align {
            let mut d = glam::Vec3::ZERO;
            d[a as usize] = 1.0;
            guides.push(p3d_remesh::Guide::Axis(d));
        }
        let spec = QuadSpec { edge, iterations: p.iterations, relax: p.relax, seed: ctx.seed, guides, ..QuadSpec::default() };
        ctx.check_cancel()?;
        ctx.report(0.05);
        // Long thin faces (primitives, CAD-like meshes) are resampled evenly
        // first: the direction field needs an even sampling.
        let longest = old
            .edges()
            .map(|e| {
                let [a, b] = old.edge_verts(e);
                old.positions[a as usize].distance(old.positions[b as usize])
            })
            .fold(0.0f32, f32::max);
        let resampled = if longest > edge {
            let field = MeshField::new(old).ok_or_else(|| OpError::new(ErrorCode::EmptySelection, "the mesh has no faces").at("/target"))?;
            Some(sdf_mesh(&field, &MeshSpec { voxel: edge * 0.6 }).map_err(|e| OpError::new(ErrorCode::Topology, e.to_string()).at("/edge"))?.mesh)
        } else {
            None
        };
        // Progress by stage: the kernel runs as one call.
        ctx.check_cancel()?;
        ctx.report(0.25);
        let (mut mesh, r): (Mesh, QuadReport) = quad_remesh(resampled.as_ref().unwrap_or(old), &spec).map_err(remesh_error)?;
        ctx.check_cancel()?;
        ctx.report(0.85);
        carry(old, &mut mesh);
        ctx.check_cancel()?;
        ctx.report(0.95);
        let faces = mesh.face_count();
        *tx.mesh_mut(key, "/target")? = mesh;
        let two = if r.valence_two > 0 { format!(", {} of 2", thousands(r.valence_two)) } else { String::new() };
        let mut out = OpOutput::summary(format!(
            "{}: remeshed into {} quads ({} faces, edge {} mm); poles {} of valence 3, {} of 5, {} of 6, {} of 7 or more{two}",
            label(tx.doc(), key),
            thousands(r.quads),
            thousands(faces),
            (f64::from(edge) * 1000.0).round(),
            thousands(r.poles[0]),
            thousands(r.poles[1]),
            thousands(r.poles[2]),
            thousands(r.poles[3])
        ));
        out.changed.push(key);
        Ok(out)
    }
}

// ---- remesh.isotropic ------------------------------------------------------------------

/// Parameters of `remesh.isotropic`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IsotropicParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Target edge length; about a fortieth of the largest side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<Length>,
    /// Rounds of split, collapse, flip, relax and project.
    #[serde(default = "six")]
    pub iterations: u32,
    /// Edges bent more than this are features and keep their place.
    #[serde(default = "forty")]
    pub feature_angle: Angle,
    /// Join triangle pairs into quads afterwards (quad-dominant).
    #[serde(default)]
    pub quads: bool,
}

fn six() -> u32 {
    6
}

fn forty() -> Angle {
    Angle::from_degrees(40.0)
}

impl Default for IsotropicParams {
    fn default() -> Self {
        IsotropicParams { target: String::new(), edge: None, iterations: six(), feature_angle: forty(), quads: false }
    }
}

/// `remesh.isotropic`.
pub struct IsotropicRemesh;

impl Op for IsotropicRemesh {
    const ID: &'static str = "remesh.isotropic";
    type Params = IsotropicParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Isotropic remesh",
            category: Category::Remesh,
            summary: "Rebuilds a mesh as even triangles about `edge` long (valence near 6), keeping boundaries and sharp features.",
            docs: "Edges longer than 4/3 of `edge` are split, shorter than 4/5 collapsed, edges flipped toward valence 6, and \
                   vertices relaxed in their tangent planes and projected back onto the original surface, `iterations` times \
                   (Botsch and Kobbelt 2004). Boundaries and edges bent more than `feature_angle` stay put, so hard-surface \
                   edges survive. Open meshes work. `quads` joins triangle pairs into a quad-dominant mesh afterwards. Face \
                   groups and materials carry over by the nearest old face.\n\n\
                   **Use for** evening out scans, booleans and decimated meshes before sculpting, deformation or simulation; \
                   `remesh.quad` makes all-quad, flow-aligned topology for organic parts.",
            examples: &[OpExample {
                args: r#"{"target": "crate", "edge": "5cm"}"#,
                result: "crate (o:1): remeshed into 3,072 faces (3,072 tris), edges about 50 mm",
            }],
            aliases: &["isotropic remesh", "uniform remesh", "triangle remesh", "even triangles", "remesh"],
            complexity: "O(iterations x T log T) for T output triangles",
            flags: OpFlags::UNDOABLE.union(OpFlags::LONG_RUNNING),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &IsotropicParams) -> Result<(), OpError> {
        mesh_target(doc, &p.target)?;
        if let Some(e) = p.edge {
            check_length("/edge", e, 1e-4, 10.0)?;
        }
        check_count("/iterations", p.iterations, 1, 50)
    }

    fn apply(tx: &mut Tx, p: &IsotropicParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let old = tx.mesh(key, "/target")?;
        let size = old.bounds().map_or(1.0, |(lo, hi)| (hi - lo).max_element());
        let edge = p.edge.map_or(size / 40.0, |e| e.0 as f32);
        let spec = p3d_remesh::IsotropicSpec { edge, iterations: p.iterations, feature_deg: p.feature_angle.0.to_degrees() as f32 };
        ctx.check_cancel()?;
        let mut mesh = p3d_remesh::isotropic(old, &spec).map_err(remesh_error)?;
        if p.quads {
            let faces: Vec<u32> = mesh.faces().collect();
            let q = p3d_geo::edit::QuadSpec { max_face_angle_deg: 10.0, max_shape_angle_deg: 45.0, ignore_boundaries: false };
            p3d_geo::edit::tris_to_quads(&mut mesh, &faces, &q).map_err(|e| OpError::new(ErrorCode::Topology, e.to_string()))?;
            mesh.compact();
        }
        carry(old, &mut mesh);
        let (faces, tris) = (mesh.face_count(), mesh.triangle_count());
        *tx.mesh_mut(key, "/target")? = mesh;
        let mut out = OpOutput::summary(format!(
            "{}: remeshed into {} faces ({} tris), edges about {} mm",
            label(tx.doc(), key),
            thousands(faces),
            thousands(tris),
            (f64::from(edge) * 1000.0).round()
        ));
        out.changed.push(key);
        Ok(out)
    }
}
