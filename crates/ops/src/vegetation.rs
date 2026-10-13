//! `gen.tree`: a tree grown from a species preset toward a crown hull, with
//! buttress roots, swept wood and leaf cards, clumps or a crown shell
//! (ADR 0009, ADR 0014). It is the `tree` feature template with typed
//! parameters, crown outlines, the report kept as metadata and the
//! `vegetation` tag.

use glam::Vec2;
use p3d_features::vegetation::{TAG, tree_spec};
use p3d_scene::Document;
use p3d_vegetation::{CrownShape, Tree, TreeSpec, WIND_LAYOUT};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::common::{check_new_object, create_mesh_object, thousands};
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::{Angle, Length};
use crate::prim::created_summary;
use crate::tx::Tx;

/// A growth preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TreeSpecies {
    /// Trunk to a fork at 37%, 3 to 5 scaffold limbs, a dome as wide as tall.
    Broadleaf,
    /// A leader with whorls of drooping branches in a cone.
    Conifer,
    /// A curved trunk and a crown of arching fronds.
    Palm,
    /// A slender leader, a narrow oval crown, hanging twigs.
    Birch,
    /// A chunky trunk, three limbs and faceted clumps.
    Stylized,
}

/// A look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TreeStyle {
    /// Cluster and single-leaf cards, normals mostly from the cards.
    Realistic,
    /// Large cards with crown normals (a soft painted mass).
    Painted,
    /// Faceted clumps, no alpha.
    LowPoly,
}

/// A crown hull shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TreeCrownShape {
    Dome,
    Sphere,
    Ellipsoid,
    Cone,
    Umbrella,
    Column,
}

/// The crown hull: a shape word, or an outline of `[radius share, height
/// share]` points from the crown base (0) to the top (1), turned around
/// the trunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TreeCrown {
    Shape(TreeCrownShape),
    Outline(Vec<[f64; 2]>),
}

/// What the leaves become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TreeFoliage {
    Cards,
    Clumps,
    Shell,
    None,
}

/// Clump shading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClumpShading {
    Smooth,
    Faceted,
}

/// Parameters of `gen.tree`. Values left out come from the species preset
/// and its style.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TreeParams {
    /// Object name; `tree` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Parent object (name, path or id); top level when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Base position in the parent's space.
    #[serde(default)]
    pub position: [Length; 3],
    /// Growth preset: `broadleaf` (default), `conifer`, `palm`, `birch` or `stylized`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub species: Option<TreeSpecies>,
    /// Look: `realistic`, `painted` or `low_poly`; the species' own when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<TreeStyle>,
    /// Total height (broadleaf 12 m, conifer 15, palm 9, birch 14, stylized 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<Length>,
    /// Trunk radius at breast height; the species' share of the height when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trunk_radius: Option<Length>,
    /// Lean of the trunk from vertical (0 to 45 degrees).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lean: Option<Angle>,
    /// Heading of the lean: 0 toward +X, 90 toward +Z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lean_toward: Option<Angle>,
    /// Height share of the first fork (crown base for conifer and birch, trunk top for palm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<f64>,
    /// Scaffold limbs from the fork, branches per whorl (conifer) or fronds (palm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limbs: Option<u32>,
    /// Branch levels built as wood after the trunk (1 to 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<u32>,
    /// Angle of branches from their parent; scaffold limbs from vertical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_angle: Option<Angle>,
    /// Growth step of the branches (conifer: whorl spacing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_spacing: Option<Length>,
    /// Radius ratio of a branch to its parent at an even split (0.3 to 0.98).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper: Option<f64>,
    /// Random bends and curves (0 straight to 1 gnarled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gnarl: Option<f64>,
    /// Twist of the trunk and scaffold limbs along their length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twist: Option<Angle>,
    /// Buttress roots blended into the trunk base (0 to 12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<u32>,
    /// Root flare width at the ground over the trunk diameter (1: no flare).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_spread: Option<f64>,
    /// Crown hull: `dome`, `sphere`, `ellipsoid`, `cone`, `umbrella`, `column`, or an outline `[[radius share, height share], ...]` from the crown base up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown: Option<TreeCrown>,
    /// Crown diameter (broadleaf: about the height).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown_width: Option<Length>,
    /// Height share of the crown's lowest point; derived from the fork when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown_base: Option<f64>,
    /// Lumps of the crown's outline as a share of its radius (0 smooth to 0.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crown_lumps: Option<f64>,
    /// Foliage gathered into clumps with gaps between them (0 even to 1 distinct clumps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clumping: Option<f64>,
    /// What the leaves become: `cards`, `clumps`, `shell` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foliage: Option<TreeFoliage>,
    /// Cluster card (or clump) size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_size: Option<Length>,
    /// Silhouette coverage target of the crown (0.05 to 1); cards are added until it is reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<f64>,
    /// Share of single-leaf cards at the silhouette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leaf_cards: Option<f64>,
    /// Turn between successive cards around a twig (137.5 degrees).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phyllotaxis: Option<Angle>,
    /// Angle of the cards from their twig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<Angle>,
    /// Gravity droop of the cards, growing toward the twig tips (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub droop: Option<f64>,
    /// Turn of the cards toward the sky (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sun_bias: Option<f64>,
    /// Crown normal transfer: 0 keeps the card normals, 1 shades like the hull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_blend: Option<f64>,
    /// Per-card hue variation in vertex color (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue_jitter: Option<f64>,
    /// Per-card brightness variation in vertex color (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_jitter: Option<f64>,
    /// Crown gradient: darker and cooler inside, warmer outside, ambient occlusion (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<f64>,
    /// Cards get smaller toward the silhouette (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_falloff: Option<f64>,
    /// Atlas variants the cards pick from (1 to 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<u32>,
    /// Clumps `faceted` (low poly) or `smooth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clump_shading: Option<ClumpShading>,
    /// Triangle budget at LOD0; the wood takes about a third.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
    /// Variation: another seed grows another tree of the same kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// A template or kernel error at the op's parameter path.
fn param_error(e: p3d_features::FeatureError) -> OpError {
    let path = e.path.strip_prefix("/params").unwrap_or(&e.path).to_owned();
    let mut o = OpError::invalid(if path.is_empty() { "/" } else { &path }, e.message);
    o.suggestions = e.suggestions;
    o
}

/// The tree spec of the parameters: the template's parameters resolved (so
/// ranges live in one table), then the crown outline.
///
/// Complexity: O(parameters + outline points).
pub fn spec_of(p: &TreeParams) -> Result<TreeSpec, OpError> {
    let mut v = serde_json::to_value(p).map_err(|e| OpError::internal(e.to_string()))?;
    let mut outline = None;
    if let Value::Object(m) = &mut v {
        for k in ["name", "parent", "position"] {
            m.remove(k);
        }
        if let Some(Value::Array(_)) = m.get("crown") {
            outline = m.remove("crown");
        }
    }
    let info = p3d_features::get("tree").map_err(|e| OpError::internal(e.message))?.info();
    let resolved = p3d_features::resolve(&info, &v).map_err(param_error)?;
    let mut spec = tree_spec(&resolved);
    if let Some(Value::Array(points)) = outline {
        let pts: Vec<Vec2> = points.iter().filter_map(|q| Some(Vec2::new(q.get(0)?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))).collect();
        spec.crown = CrownShape::Outline(pts);
    }
    p3d_vegetation::validate(&spec).map_err(|e| OpError::invalid(&format!("/{}", e.param), e.message))?;
    Ok(spec)
}

/// The report of a grown tree as JSON, kept on the object as metadata
/// `vegetation`: species, style, foliage, sizes (m), counts, coverage and
/// its target, overdraw, repeats, the crown hull, the wind layout, the
/// atlas layout and warnings.
pub fn report_json(spec: &TreeSpec, t: &Tree) -> Value {
    let r = &t.report;
    let q = |x: f32| (f64::from(x) * 1e4).round() / 1e4;
    json!({
        "species": spec.species.as_str(),
        "style": spec.style.as_str(),
        "foliage": spec.foliage.as_str(),
        "height": q(r.height),
        "crown_width": q(r.crown_width),
        "crown_depth": q(r.crown_depth),
        "crown_height": q(r.crown_height),
        "crown_base": q(r.crown_base),
        "fork_height": q(r.fork_height),
        "trunk_radius": q(r.trunk_radius),
        "limbs": r.limbs,
        "branches": r.branches,
        "cards": r.cards,
        "leaf_cards": r.leaf_cards,
        "wood_triangles": r.wood_triangles,
        "foliage_triangles": r.foliage_triangles,
        "coverage": q(r.coverage),
        "coverage_target": q(spec.coverage),
        "overdraw": q(r.overdraw),
        "repeats": r.repeats,
        "crown": {
            "shape": spec.crown.as_str(),
            "axis": [q(t.hull.axis.x), q(t.hull.axis.y)],
            "base": q(t.hull.base),
            "top": q(t.hull.top),
            "radius": q(t.hull.radius),
            "lumps": q(t.hull.lumps),
        },
        "materials": ["bark", "foliage"],
        "wind": WIND_LAYOUT,
        "atlas": "4 columns; cluster variants in the top half, single leaves, sprays and fronds in the bottom half",
        "warnings": r.warnings,
    })
}

/// One line about a grown tree.
pub fn tree_summary(spec: &TreeSpec, t: &Tree) -> String {
    let r = &t.report;
    let what = match spec.foliage {
        p3d_vegetation::Foliage::Cards if spec.species == p3d_vegetation::Species::Palm => format!("{} fronds", r.cards),
        p3d_vegetation::Foliage::Cards => format!("{} cards", thousands(r.cards + r.leaf_cards)),
        p3d_vegetation::Foliage::Clumps => format!("{} clumps", r.cards),
        p3d_vegetation::Foliage::Shell => "a crown shell".to_owned(),
        p3d_vegetation::Foliage::None => "no foliage".to_owned(),
    };
    let limbs = match spec.species {
        p3d_vegetation::Species::Conifer => format!("{} whorl branches", r.limbs),
        p3d_vegetation::Species::Palm | p3d_vegetation::Species::Birch => String::new(),
        _ => format!("{} limbs", r.limbs),
    };
    let mut s = format!(
        "{} {:.1} m: crown {:.1} x {:.1} m, {:.1} m tall from {:.1} m; fork at {:.2} m ({:.0}%){}{}; {what}",
        spec.species.as_str(),
        r.height,
        r.crown_width,
        r.crown_depth,
        r.crown_height,
        r.crown_base,
        r.fork_height,
        r.fork_height / r.height.max(1e-6) * 100.0,
        if limbs.is_empty() { "" } else { ", " },
        limbs,
    );
    if spec.foliage != p3d_vegetation::Foliage::None {
        s.push_str(&format!(", coverage {:.0}% (target {:.0}%), overdraw {:.1}", r.coverage * 100.0, spec.coverage * 100.0, r.overdraw));
    }
    s.push_str(&format!("; wood {} tris, foliage {} tris", thousands(r.wood_triangles), thousands(r.foliage_triangles)));
    for w in &r.warnings {
        s.push_str(&format!("; {w}"));
    }
    s
}

/// `gen.tree`.
pub struct GenTree;

impl Op for GenTree {
    const ID: &'static str = "gen.tree";
    type Params = TreeParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Tree",
            category: Category::Gen,
            summary: "Grows a tree from a species preset (broadleaf, conifer, palm, birch, stylized) toward a crown hull, with buttress roots, swept wood and leaf cards, clumps or a crown shell.",
            docs: "Trees are described, not drawn. Pick a `species` (its preset sets everything) and change what differs: `height`, \
                   `crown` (a shape or an outline), `crown_width`, `fork`, `limbs`, `branch_angle`, `roots`, `foliage`, `card_size`, \
                   `coverage`. The trunk and scaffold limbs are laid out from the species; finer branches grow toward the crown hull \
                   by seeded space colonization. Branches emerge from their parents on smooth curves, and buttress roots are part of \
                   the trunk's surface. Cards are added until the crown silhouette reaches the `coverage` target or the `budget` \
                   runs out, oriented by phyllotaxis, spread, droop and sun bias, with seeded atlas variants, flips and turns, per-card \
                   tints, a crown gradient, normals blended toward the hull, and wind data (level in vertex color alpha, phase and \
                   stiffness in UV layer 1). Material 0 is bark, 1 foliage. Groups: `trunk`, `roots`, `level_1` to `level_3`, \
                   `twig_tips`, `foliage`; landmarks `lm.base`, `lm.fork`, `lm.crown_top`, `lm.crown_center`, `lm.crown_base`. The \
                   report stays on the object as metadata `vegetation`; `perceive` with aspect `vegetation` measures it.\n\n\
                   **Use for** trees, bushes (a short tree with `fork` low) and stylized props. Flowers and grass patches are the \
                   `flower` and `grass_cards` features.",
            examples: &[OpExample {
                args: r#"{"species": "broadleaf", "height": "12m"}"#,
                result: "created tree (o:4) tree 12.6097 x 12.4058 x 13.0091 m: 4,538 faces (9,020 tris; budget 6,000); broadleaf 12.3 m: crown 12.6 x 13.0 m, 9.1 m tall from 3.2 m; fork at 4.44 m (36%), 4 limbs; 1,560 cards, coverage 90% (target 90%), overdraw 2.1; wood 2,998 tris, foliage 6,022 tris",
            }],
            aliases: &["tree", "oak", "pine", "fir", "conifer", "palm", "birch", "foliage", "vegetation", "speedtree", "leaf cards"],
            complexity: "O(attractors x rounds) for space colonization through a hash grid, plus O(wood faces + cards x pixels)",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &TreeParams) -> Result<(), OpError> {
        check_new_object(doc, p.name.as_deref(), p.parent.as_deref())?;
        spec_of(p).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &TreeParams, _ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let spec = spec_of(p)?;
        let tree = p3d_vegetation::tree(&spec).map_err(|e| OpError::invalid(&format!("/{}", e.param), e.message))?;
        let report = report_json(&spec, &tree);
        let summary = tree_summary(&spec, &tree);
        let key = create_mesh_object(tx, p.name.as_deref(), "tree", p.parent.as_deref(), &p.position, tree.mesh)?;
        if let Some(o) = tx.doc_mut().object_mut(key) {
            o.tags.push(TAG.into());
            o.metadata.insert("vegetation".into(), report);
        }
        let mut out = created_summary(tx, key, "tree");
        out.summary = format!("{}; {summary}", out.summary);
        Ok(out)
    }
}
