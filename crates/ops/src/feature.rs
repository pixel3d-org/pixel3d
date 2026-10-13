//! `gen.feature`: a feature template (head base, hand, ear, eye, nose,
//! horn, cable, bolt, rivet, vent grille, panel inset, molding) as a new object
//! (ADR 0012).

use p3d_scene::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{check_length, check_new_object, create_mesh_object};
use crate::error::{ErrorCode, OpError};
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::Length;
use crate::prim::created_summary;
use crate::tx::Tx;

/// Parameters of `gen.feature`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeatureParams {
    /// Object name; the feature's id is used when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The template: `head_base`, `hand`, `ear`, `eye`, `nose`, `horn`, `cable`, `bolt`, `rivet`, `vent_grille`, `panel_inset` or `molding` (the `features` tool describes them).
    pub feature: String,
    /// The template's parameters, for example `{"variant": "elf", "length": "9cm"}`; defaults for the rest.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
    /// Voxel size for SDF features (`nose`); a 48th of the size when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Length>,
    /// Origin position in the parent's space.
    #[serde(default)]
    pub position: [Length; 3],
}

/// A feature error as an op error, keeping its path and suggestions.
pub fn feature_error(e: p3d_features::FeatureError) -> OpError {
    let mut o = OpError::invalid(&e.path, e.message);
    o.suggestions = e.suggestions;
    if e.path == "/feature" {
        o.code = ErrorCode::NotFound;
    }
    o
}

/// Builds a feature into a mesh: designed meshes as they are, SDF features
/// meshed with their landmarks baked onto the nearest vertices.
///
/// Complexity: the feature's own build, plus O(active voxels) for SDF ones.
pub fn feature_mesh(id: &str, params: &Value, detail: Option<f32>, ctx: &OpCtx) -> Result<p3d_mesh::Mesh, OpError> {
    let built = p3d_features::build(id, params).map_err(feature_error)?;
    match built.shape {
        p3d_features::Shape::Mesh(m) => Ok(m),
        p3d_features::Shape::Sdf(items) => {
            let mut m = crate::sdf::mesh_items(items, detail, "/params", ctx)?.mesh;
            p3d_features::bake_landmarks(&mut m, &built.landmarks);
            Ok(m)
        }
    }
}

/// `gen.feature`.
pub struct GenFeature;

impl Op for GenFeature {
    const ID: &'static str = "gen.feature";
    type Params = FeatureParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Feature",
            category: Category::Gen,
            summary: "Builds a feature template (head base, hand, ear, eye, nose, horn, cable, bolt, rivet, vent grille, panel inset, molding) from semantic parameters.",
            docs: "Feature templates are details agents describe instead of drawing: an elf ear is `{\"variant\": \"elf\", \
                   \"pointiness\": 0.9}`, a ram horn `{\"curl\": 300, \"spiral\": 0.4}`. Each has documented parameters with \
                   ranges (the `features` tool describes them), clean topology for every value in range, and landmarks kept as \
                   vertex groups `lm.<name>` (an ear's `root` and `tip`) for anchors and handles. Organic features (`nose`) are \
                   SDF shapes, meshed here at `detail`; in a Model Spec they blend into SDF parents.\n\n\
                   **Use for** head blockouts, hands, ears, eyes, noses, horns, hanging cables and ropes, and hard-surface details (bolts, rivets, vents, panels, moldings).",
            examples: &[OpExample {
                args: r#"{"feature": "ear", "params": {"variant": "elf", "length": "9cm"}}"#,
                result: "created ear (o:4) ear feature 46.53 x 76.18 x 52.22 mm: 524 faces (954 tris; budget 6,000); landmarks lobe, rim_back, root, tip",
            }],
            aliases: &["feature", "template", "kitbash", "detail", "elf ear", "horn", "bolt", "rivet", "vent", "molding"],
            complexity: "the template's build, about O(output faces)",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &FeatureParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        if let Some(d) = p.detail {
            check_length("/detail", d, 1e-4, 1.0)?;
        }
        let f = p3d_features::get(&p.feature).map_err(feature_error)?;
        p3d_features::resolve(&f.info(), &p.params).map(|_| ()).map_err(feature_error)
    }

    fn apply(tx: &mut Tx, p: &FeatureParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let m = feature_mesh(&p.feature, &p.params, p.detail.map(|d| d.0 as f32), ctx)?;
        let marks: Vec<String> = p3d_features::baked_landmarks(&m).into_keys().collect();
        let key = create_mesh_object(tx, Some(p.name.as_deref().unwrap_or(&p.feature)), &p.feature, p.parent.as_deref(), &p.position, m)?;
        if p3d_features::vegetation::is_vegetation(&p.feature)
            && let Some(o) = tx.doc_mut().object_mut(key)
        {
            // Card meshes: drawn two-sided, open by design.
            o.tags.push(p3d_features::vegetation::TAG.into());
        }
        let mut out = created_summary(tx, key, &format!("{} feature", p.feature));
        out.summary.push_str(&format!("; landmarks {}", marks.join(", ")));
        Ok(out)
    }
}
