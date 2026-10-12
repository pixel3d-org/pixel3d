//! `subd.*`: smooth subdivision.

use p3d_geo::subd::{MAX_SUBD_FACES, MAX_SUBD_LEVELS, SubdSpec, catmull_clark, subdivided_face_count};
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{label, thousands};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput, Warning};
use crate::tx::Tx;

fn mesh_target(doc: &Document, target: &str) -> Result<ObjectKey, OpError> {
    let key = doc.resolve(target).map_err(|e| OpError::from_scene(e, "/target"))?;
    if doc.object_mesh(key).is_none() {
        return Err(OpError::invalid("/target", format!("{} is not a mesh", label(doc, key))));
    }
    Ok(key)
}

/// Advanced settings of `subd.catmull_clark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct SubdAdvanced {
    /// Open-border corners (vertices with two edges) stay in place.
    pub keep_corners: bool,
    /// Treat edges marked sharp like fully creased edges.
    pub sharp_as_crease: bool,
}

impl Default for SubdAdvanced {
    fn default() -> Self {
        SubdAdvanced { keep_corners: true, sharp_as_crease: false }
    }
}

/// Parameters of `subd.catmull_clark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatmullClarkParams {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Subdivision levels (1 to 6); each makes about 4 times as many faces.
    #[serde(default = "one")]
    pub levels: u32,
    /// Rarely needed settings.
    #[serde(default)]
    pub advanced: SubdAdvanced,
}

fn one() -> u32 {
    1
}

impl Default for CatmullClarkParams {
    fn default() -> Self {
        CatmullClarkParams { target: String::new(), levels: 1, advanced: SubdAdvanced::default() }
    }
}

/// `subd.catmull_clark`.
pub struct CatmullClark;

impl Op for CatmullClark {
    const ID: &'static str = "subd.catmull_clark";
    type Params = CatmullClarkParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Catmull-Clark subdivision",
            category: Category::Subd,
            summary: "Smooths the whole mesh by Catmull-Clark subdivision, keeping creased edges sharp.",
            docs: "Every face becomes quads and the surface rounds toward a smooth limit shape; `levels` repeats it. Edge creases \
                   (`shade.crease`) keep edges sharp: weight 1 stays sharp at every level, smaller weights soften after a few \
                   levels. Open borders stay borders. UVs are interpolated, materials and groups follow the faces, custom normals \
                   are dropped. The result replaces the mesh (it is applied, not a live modifier).\n\n\
                   **Use for** smooth organic and stylized shapes from a low-poly cage, rounded hard surface with support loops or \
                   creases. Check the triangle budget: 2 levels make 16 times the faces.",
            examples: &[OpExample {
                args: r#"{"target": "post", "levels": 2}"#,
                result: "post (o:3): subdivided 2 levels: 32 -> 512 faces (1,024 tris; budget 6,000)",
            }],
            aliases: &["subdivision surface", "subsurf", "smooth mesh", "catmull clark", "polySmooth", "turbosmooth", "meshsmooth"],
            complexity: "O(4^levels x (V + E + F))",
            flags: OpFlags::UNDOABLE,
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &CatmullClarkParams) -> Result<(), OpError> {
        let key = mesh_target(doc, &p.target)?;
        crate::common::check_count("/levels", p.levels, 1, MAX_SUBD_LEVELS)?;
        if let Some(m) = doc.object_mesh(key) {
            let n = subdivided_face_count(m, p.levels);
            if n > MAX_SUBD_FACES {
                return Err(OpError::new(ErrorCode::BudgetExceeded, format!("{} levels would make {} faces", p.levels, thousands(n)))
                    .at("/levels")
                    .with_hint(format!("at most {} faces; use fewer levels", thousands(MAX_SUBD_FACES))));
            }
        }
        Ok(())
    }

    fn apply(tx: &mut Tx, p: &CatmullClarkParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let key = mesh_target(tx.doc(), &p.target)?;
        let spec = SubdSpec { levels: p.levels, keep_corners: p.advanced.keep_corners, sharp_as_crease: p.advanced.sharp_as_crease };
        let before = tx.mesh(key, "/target")?.face_count();
        let new = catmull_clark(tx.mesh(key, "/target")?, &spec).map_err(|e| OpError::internal(e.to_string()))?;
        let (faces, tris) = (new.face_count(), new.triangle_count());
        *tx.mesh_mut(key, "/target")? = new;
        let budget = tx.doc().settings.tri_budget() as usize;
        let mut out = OpOutput::summary(format!(
            "{}: subdivided {} level{}: {} -> {} faces ({} tris; budget {})",
            label(tx.doc(), key),
            p.levels,
            if p.levels == 1 { "" } else { "s" },
            thousands(before),
            thousands(faces),
            thousands(tris),
            thousands(budget)
        ));
        out.changed.push(key);
        if tris > budget {
            out.warnings.push(Warning {
                message: format!("{} triangles exceed the asset class budget of {}", thousands(tris), thousands(budget)),
                fix: None,
            });
        }
        Ok(out)
    }
}
