//! Vegetation templates (ADR 0009, ADR 0014): `tree`, `flower` and
//! `grass_cards`. The kernels live in `p3d-vegetation` and `p3d-strands`;
//! these templates describe their parameters for agents and the GUI. Tree
//! parameters the caller leaves out come from the species preset (and the
//! style), so the listed defaults are the broadleaf ones.

use std::collections::BTreeMap;

use glam::Vec3;
use p3d_mesh::ElementKind;
use p3d_vegetation::{CrownShape, Foliage, PetalShape, Species, Style, TreeSpec, VegError};

use crate::{Built, Feature, FeatureError, FeatureInfo, ParamInfo, ParamKind, Params, Shape};

/// Object tag of vegetation: card meshes drawn two-sided and open by
/// design, which diagnostics treat like strands.
pub const TAG: &str = "vegetation";

/// Whether a feature id is a vegetation template.
pub fn is_vegetation(id: &str) -> bool {
    matches!(id, "tree" | "flower" | "grass_cards")
}

const SPECIES: &[&str] = &["broadleaf", "conifer", "palm", "birch", "stylized"];
const STYLES: &[&str] = &["realistic", "painted", "low_poly"];
const CROWNS: &[&str] = &["dome", "sphere", "ellipsoid", "cone", "umbrella", "column"];
const FOLIAGE: &[&str] = &["cards", "clumps", "shell", "none"];
const SHADING: &[&str] = &["smooth", "faceted"];

const fn p(name: &'static str, doc: &'static str, kind: ParamKind, default: f64, min: f64, max: f64) -> ParamInfo {
    ParamInfo { name, doc, kind, default, min, max }
}

/// Parameters of `tree`.
pub const TREE_PARAMS: &[ParamInfo] = &[
    p("species", "Growth preset; it sets every value left out.", ParamKind::Choice(SPECIES), 0.0, 0.0, 4.0),
    p(
        "style",
        "Look: realistic (cluster and leaf cards), painted (large cards, soft crown normals), low_poly (faceted clumps). Default: the species' own.",
        ParamKind::Choice(STYLES),
        0.0,
        0.0,
        2.0,
    ),
    p("height", "Total height (species: broadleaf 12 m, conifer 15, palm 9, birch 14, stylized 5).", ParamKind::Length, 12.0, 0.2, 120.0),
    p("trunk_radius", "Trunk radius at breast height (broadleaf: 3.5% of the height, a diameter of 7%).", ParamKind::Length, 0.42, 0.002, 5.0),
    p("lean", "Lean of the trunk from vertical.", ParamKind::Angle, 0.0, 0.0, 45.0),
    p("lean_toward", "Heading of the lean: 0 toward +X, 90 toward +Z.", ParamKind::Angle, 0.0, -360.0, 360.0),
    p(
        "fork",
        "Height share of the first fork (broadleaf, stylized), of the crown base (conifer, birch) or of the trunk top (palm).",
        ParamKind::Ratio,
        0.37,
        0.02,
        0.97,
    ),
    p(
        "limbs",
        "Scaffold limbs from the fork (3 to 5 for broadleaf), branches per whorl (conifer) or fronds (palm).",
        ParamKind::Count,
        4.0,
        0.0,
        32.0,
    ),
    p("levels", "Branch levels built as wood after the trunk (finer twigs are covered by cards).", ParamKind::Count, 3.0, 1.0, 3.0),
    p(
        "branch_angle",
        "Angle of branches from their parent; scaffold limbs from vertical (30 to 60 for broadleaf).",
        ParamKind::Angle,
        45.0,
        0.0,
        135.0,
    ),
    p("branch_spacing", "Growth step of the branches (conifer: whorl spacing); 0 sizes it from the height.", ParamKind::Length, 0.0, 0.0, 20.0),
    p("taper", "Radius ratio of a branch to its parent at an even split.", ParamKind::Ratio, 0.7, 0.3, 0.98),
    p("gnarl", "Random bends and curves of trunk and limbs (0 straight to 1 gnarled).", ParamKind::Ratio, 0.35, 0.0, 1.0),
    p("twist", "Twist of the trunk and scaffold limbs along their length.", ParamKind::Angle, 15.0, -360.0, 360.0),
    p("roots", "Buttress roots blended into the trunk base.", ParamKind::Count, 7.0, 0.0, 12.0),
    p("root_spread", "Spread of the root flare at the ground over the trunk diameter (1: no flare).", ParamKind::Ratio, 1.5, 1.0, 4.0),
    p("crown", "Crown hull the branches grow toward (gen.tree also takes an outline).", ParamKind::Choice(CROWNS), 0.0, 0.0, 5.0),
    p("crown_width", "Crown diameter (broadleaf: about the height).", ParamKind::Length, 12.0, 0.05, 100.0),
    p("crown_base", "Height share of the crown's lowest point; 0 derives it from the fork.", ParamKind::Ratio, 0.0, 0.0, 0.95),
    p("crown_lumps", "Lumps of the crown's outline as a share of its radius (0 smooth).", ParamKind::Ratio, 0.16, 0.0, 0.5),
    p("foliage", "What the leaves become: cards, clumps, shell or none.", ParamKind::Choice(FOLIAGE), 0.0, 0.0, 3.0),
    p("card_size", "Cluster card (or clump) size (broadleaf 0.4 to 0.8 m).", ParamKind::Length, 0.6, 0.005, 5.0),
    p("coverage", "Silhouette coverage target of the crown; cards are added until it is reached.", ParamKind::Ratio, 0.9, 0.05, 1.0),
    p("leaf_cards", "Share of single-leaf cards at the silhouette.", ParamKind::Ratio, 0.15, 0.0, 1.0),
    p("phyllotaxis", "Turn between successive cards around a twig.", ParamKind::Angle, 137.5, 0.0, 360.0),
    p("spread", "Angle of the cards from their twig.", ParamKind::Angle, 55.0, 0.0, 90.0),
    p("droop", "Gravity droop of the cards, growing toward the twig tips (birch 0.75).", ParamKind::Ratio, 0.35, 0.0, 1.0),
    p("sun_bias", "Turn of the cards toward the sky.", ParamKind::Ratio, 0.35, 0.0, 1.0),
    p("normal_blend", "Crown normal transfer: 0 keeps the card normals, 1 shades like the hull.", ParamKind::Ratio, 0.4, 0.0, 1.0),
    p("hue_jitter", "Per-card hue variation in vertex color.", ParamKind::Ratio, 0.25, 0.0, 1.0),
    p("value_jitter", "Per-card brightness variation in vertex color.", ParamKind::Ratio, 0.2, 0.0, 1.0),
    p("gradient", "Crown gradient: darker and cooler inside, warmer on the sunlit outside, ambient occlusion.", ParamKind::Ratio, 0.6, 0.0, 1.0),
    p("size_falloff", "Cards get smaller toward the silhouette.", ParamKind::Ratio, 0.3, 0.0, 1.0),
    p("variants", "Atlas variants the cards pick from.", ParamKind::Count, 4.0, 1.0, 4.0),
    p("clump_shading", "Clumps faceted (low poly) or smooth.", ParamKind::Choice(SHADING), 1.0, 0.0, 1.0),
    p("budget", "Triangle budget at LOD0; the wood takes about a third.", ParamKind::Count, 10_000.0, 200.0, 1_000_000.0),
    p("seed", "Variation: another seed grows another tree of the same kind.", ParamKind::Count, 1.0, 0.0, 1_000_000.0),
];

const TREE_LANDMARKS: &[(&str, &str)] = &[
    ("base", "the trunk's center at the ground"),
    ("fork", "the first fork (crown base on the leader, palm: the trunk top)"),
    ("crown_top", "the top of the crown"),
    ("crown_center", "the crown's center"),
    ("crown_base", "the crown's lowest point on its axis"),
];

/// A vegetation kernel error at its parameter.
fn veg_error(e: VegError) -> FeatureError {
    FeatureError::new(&format!("/params/{}", e.param), e.message)
}

/// The tree spec from resolved parameters: the species preset, its style,
/// then every value the caller gave.
///
/// Complexity: O(parameters).
pub fn tree_spec(p: &Params) -> TreeSpec {
    let species = Species::parse(p.choice("species")).unwrap_or(Species::Broadleaf);
    let mut s = TreeSpec::preset(species);
    if p.given("style")
        && let Some(st) = Style::parse(p.choice("style"))
    {
        s = s.with_style(st);
    }
    let f = |name: &str, field: &mut f32| {
        if p.given(name) {
            *field = p.f(name);
        }
    };
    f("height", &mut s.height);
    if !p.given("trunk_radius") {
        // Keep the species' trunk share when only the height changes.
        s.trunk_radius = TreeSpec::preset(species).trunk_radius / TreeSpec::preset(species).height * s.height;
    }
    if !p.given("crown_width") {
        s.crown_width = TreeSpec::preset(species).crown_width / TreeSpec::preset(species).height * s.height;
    }
    if !p.given("card_size") && p.given("height") {
        let k = (s.height / TreeSpec::preset(species).height).sqrt();
        s.card_size *= k;
    }
    f("trunk_radius", &mut s.trunk_radius);
    f("lean", &mut s.lean);
    f("lean_toward", &mut s.lean_toward);
    f("fork", &mut s.fork);
    f("branch_angle", &mut s.branch_angle);
    f("branch_spacing", &mut s.branch_spacing);
    f("taper", &mut s.taper);
    f("gnarl", &mut s.gnarl);
    f("twist", &mut s.twist);
    f("root_spread", &mut s.root_spread);
    f("crown_width", &mut s.crown_width);
    f("crown_base", &mut s.crown_base);
    f("crown_lumps", &mut s.crown_lumps);
    f("card_size", &mut s.card_size);
    f("coverage", &mut s.coverage);
    f("leaf_cards", &mut s.leaf_cards);
    f("phyllotaxis", &mut s.phyllotaxis);
    f("spread", &mut s.spread);
    f("droop", &mut s.droop);
    f("sun_bias", &mut s.sun_bias);
    f("normal_blend", &mut s.normal_blend);
    f("hue_jitter", &mut s.hue_jitter);
    f("value_jitter", &mut s.value_jitter);
    f("gradient", &mut s.gradient);
    f("size_falloff", &mut s.size_falloff);
    let c = |name: &str, field: &mut u32| {
        if p.given(name) {
            *field = p.count(name) as u32;
        }
    };
    c("limbs", &mut s.limbs);
    c("levels", &mut s.levels);
    c("roots", &mut s.roots);
    c("variants", &mut s.variants);
    c("budget", &mut s.budget);
    if p.given("seed") {
        s.seed = p.count("seed") as u64;
    }
    if p.given("crown")
        && let Some(shape) = CrownShape::parse(p.choice("crown"))
    {
        s.crown = shape;
    }
    if p.given("foliage")
        && let Some(fo) = Foliage::parse(p.choice("foliage"))
    {
        s.foliage = fo;
    }
    if p.given("clump_shading") {
        s.facets = p.choice("clump_shading") == "faceted";
    }
    s
}

/// Grows a tree from a spec, errors at its parameter.
///
/// Complexity: the kernel's.
pub fn grow(spec: &TreeSpec) -> Result<p3d_vegetation::Tree, FeatureError> {
    p3d_vegetation::tree(spec).map_err(veg_error)
}

/// `tree`.
pub struct Tree;

impl Feature for Tree {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "tree",
            label: "Tree",
            summary: "A tree grown from a species preset toward a crown hull, with buttress roots, swept wood and leaf cards, clumps or a crown shell.",
            params: TREE_PARAMS,
            landmarks: TREE_LANDMARKS,
            budget: 10_000,
            aliases: &["tree", "oak", "broadleaf", "conifer", "pine", "fir", "spruce", "palm", "birch", "foliage", "vegetation", "speedtree"],
            sdf: false,
        }
    }

    /// Complexity: the tree kernel's (`p3d_vegetation::tree`).
    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let t = grow(&tree_spec(p))?;
        Ok(Built { shape: Shape::Mesh(t.mesh), landmarks: t.landmarks })
    }
}

const SHAPES: &[&str] = &["round", "pointed", "heart", "strap"];

const FLOWER_PARAMS: &[ParamInfo] = &[
    p("petals", "Petals per ring.", ParamKind::Count, 8.0, 3.0, 60.0),
    p("layers", "Petal rings, outer first.", ParamKind::Count, 1.0, 1.0, 4.0),
    p("petal_length", "Petal length.", ParamKind::Length, 0.035, 0.001, 2.0),
    p("petal_width", "Petal width.", ParamKind::Length, 0.018, 0.0005, 2.0),
    p(
        "petal_shape",
        "Petal outline: round (rose, poppy), pointed (lily, tulip), heart (primrose), strap (daisy).",
        ParamKind::Choice(SHAPES),
        0.0,
        0.0,
        3.0,
    ),
    p("cup", "Cupping across each petal.", ParamKind::Ratio, 0.3, 0.0, 2.0),
    p("open", "How open the bloom is (0 a closed bud, 1 flat).", ParamKind::Ratio, 0.7, 0.0, 1.0),
    p("thickness", "Petal thickness; 0 makes one-sided cards.", ParamKind::Length, 0.0, 0.0, 0.2),
    p("center_radius", "Radius of the center disc.", ParamKind::Length, 0.008, 0.0005, 1.0),
    p("center_height", "Dome height of the center.", ParamKind::Length, 0.005, 0.0, 1.0),
    p("stem_height", "Stem height (0: a head only).", ParamKind::Length, 0.35, 0.0, 5.0),
    p("stem_radius", "Stem radius.", ParamKind::Length, 0.0025, 0.0002, 0.2),
    p("stem_bend", "Curve of the stem toward +X.", ParamKind::Ratio, 0.25, 0.0, 1.0),
    p("head_tilt", "Tilt of the head from the stem's top direction.", ParamKind::Angle, 20.0, 0.0, 90.0),
    p("leaves", "Leaves along the stem.", ParamKind::Count, 2.0, 0.0, 12.0),
    p("leaf_length", "Leaf length.", ParamKind::Length, 0.07, 0.001, 2.0),
    p("seed", "Variation of petal turns and leaf positions.", ParamKind::Count, 1.0, 0.0, 1_000_000.0),
];

/// `flower`.
pub struct Flower;

impl Feature for Flower {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "flower",
            label: "Flower",
            summary: "A flower: rings of petal shells around a domed center on a curved stem with leaves.",
            params: FLOWER_PARAMS,
            landmarks: &[("base", "the stem's foot"), ("stem_top", "the top of the stem"), ("head", "the top of the center")],
            budget: 1_500,
            aliases: &["flower", "blossom", "bloom", "daisy", "rose", "tulip", "petals", "plant"],
            sdf: false,
        }
    }

    /// Complexity: O(petals x petal faces + leaves x leaf faces).
    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let spec = p3d_vegetation::FlowerSpec {
            petals: p.count("petals") as u32,
            layers: p.count("layers") as u32,
            petal_length: p.f("petal_length"),
            petal_width: p.f("petal_width"),
            shape: PetalShape::parse(p.choice("petal_shape")).unwrap_or(PetalShape::Round),
            cup: p.f("cup"),
            open: p.f("open"),
            thickness: p.f("thickness"),
            center_radius: p.f("center_radius"),
            center_height: p.f("center_height"),
            stem_height: p.f("stem_height"),
            stem_radius: p.f("stem_radius"),
            stem_bend: p.f("stem_bend"),
            head_tilt: p.f("head_tilt"),
            leaves: p.count("leaves") as u32,
            leaf_length: p.f("leaf_length"),
            seed: p.count("seed") as u64,
        };
        let f = p3d_vegetation::flower(&spec).map_err(veg_error)?;
        Ok(Built { shape: Shape::Mesh(f.mesh), landmarks: f.landmarks })
    }
}

const GRASS_PARAMS: &[ParamInfo] = &[
    p("width", "Patch size along X.", ParamKind::Length, 1.0, 0.01, 100.0),
    p("depth", "Patch size along Z.", ParamKind::Length, 1.0, 0.01, 100.0),
    p("blade_height", "Blade card height.", ParamKind::Length, 0.3, 0.005, 5.0),
    p("blade_width", "Blade card width at the root.", ParamKind::Length, 0.04, 0.001, 1.0),
    p("density", "Cards per square meter.", ParamKind::Ratio, 120.0, 1.0, 4096.0),
    p("bend", "Droop of the blades (0 upright to 1 flat).", ParamKind::Ratio, 0.25, 0.0, 1.0),
    p("height_variation", "Random shortening as a share of the height.", ParamKind::Ratio, 0.4, 0.0, 0.95),
    p("seed", "Variation of the blade positions.", ParamKind::Count, 1.0, 0.0, 1_000_000.0),
];

/// `grass_cards`.
pub struct GrassCards;

impl Feature for GrassCards {
    fn info(&self) -> FeatureInfo {
        FeatureInfo {
            id: "grass_cards",
            label: "Grass cards",
            summary: "A patch of upright grass blade cards on the ground plane (strands' grass preset without the ground).",
            params: GRASS_PARAMS,
            landmarks: &[("center", "the patch's center on the ground")],
            budget: 1_000,
            aliases: &["grass", "lawn", "turf", "meadow", "grass patch", "tuft"],
            sdf: false,
        }
    }

    /// Complexity: O(cards x segments x log F) to grow on the ground
    /// patch's F faces (`p3d_strands::generate`).
    fn build(&self, p: &Params) -> Result<Built, FeatureError> {
        let (w, d) = (p.get("width"), p.get("depth"));
        let seg = |s: f64| ((s / 0.1).ceil() as u32).clamp(1, 200);
        let ground = p3d_geo::prim::plane(&p3d_geo::prim::PlaneSpec { size_x: w, size_z: d, segments_x: seg(w), segments_z: seg(d) })
            .map_err(|e| FeatureError::new("/params/width", e.to_string()))?;
        let mut spec = p3d_strands::StrandSpec::preset(p3d_strands::Kind::Grass);
        spec.length = p.f("blade_height");
        spec.width = p.f("blade_width");
        spec.gravity = p.f("bend");
        spec.length_variation = p.f("height_variation");
        spec.clumps = ((w * d * p.get("density")).round() as u32).clamp(1, p3d_strands::MAX_GUIDES);
        spec.seed = p.count("seed") as u64;
        let grown = p3d_strands::generate(&ground, &spec).map_err(|e| FeatureError::new(&format!("/params/{}", e.param), e.message))?;
        let mut mesh = grown.mesh;
        let all: Vec<u32> = mesh.faces().collect();
        mesh.set_group(p3d_vegetation::groups::FOLIAGE, ElementKind::Face, all).map_err(|e| FeatureError::new("/params", e.to_string()))?;
        let mut landmarks = BTreeMap::new();
        landmarks.insert("center".to_owned(), Vec3::ZERO);
        crate::bake_landmarks(&mut mesh, &landmarks);
        Ok(Built { shape: Shape::Mesh(mesh), landmarks })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn species_fill_what_the_caller_leaves_out() {
        let info = Tree.info();
        let p = crate::resolve(&info, &json!({ "species": "conifer" })).unwrap();
        let s = tree_spec(&p);
        assert_eq!(s.species, Species::Conifer);
        assert_eq!(s.crown, CrownShape::Cone);
        assert!((s.height - 15.0).abs() < 1e-6);
        let p = crate::resolve(&info, &json!({ "species": "conifer", "height": "30m", "crown": "column" })).unwrap();
        let s = tree_spec(&p);
        assert!((s.height - 30.0).abs() < 1e-6);
        assert!((s.crown_width - 5.8 * 2.0).abs() < 1e-3, "the width follows the height");
        assert_eq!(s.crown, CrownShape::Column);
        let p = crate::resolve(&info, &json!({ "style": "low_poly" })).unwrap();
        assert_eq!(tree_spec(&p).foliage, Foliage::Clumps);
    }

    #[test]
    fn vegetation_templates_build() {
        for id in ["flower", "grass_cards"] {
            let b = crate::build(id, &serde_json::Value::Null).unwrap();
            let Shape::Mesh(m) = b.shape else { unreachable!() };
            m.validate().unwrap();
            assert!(m.face_count() > 10, "{id}");
        }
        let e = crate::build("tree", &json!({ "height": 10, "trunk_radius": 4 })).unwrap_err();
        assert_eq!(e.path, "/params/trunk_radius", "{e}");
    }
}
