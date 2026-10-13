//! Trees and flowers (ADR 0009, ADR 0014): a tree is grown from a species
//! preset and a handful of meaningful numbers. The trunk and scaffold limbs
//! are laid out explicitly, finer branches grow toward a crown hull by
//! seeded space colonization, the wood is swept as tapered tubes that emerge
//! smoothly from their parents, and the foliage is leaf cards, clumps or a
//! crown shell placed to reach a silhouette coverage target.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.

mod build;
mod flower;
mod foliage;
mod hull;
mod raster;
mod skeleton;
mod wood;

use std::collections::BTreeMap;

use glam::Vec3;
use p3d_mesh::Mesh;

pub use flower::{Flower, FlowerSpec, PetalShape, flower};
pub use hull::{CrownShape, Hull};

/// Most attractors one tree uses.
pub const MAX_ATTRACTORS: u32 = 40_000;
/// Most skeleton nodes one tree grows.
pub const MAX_NODES: usize = 200_000;
/// Most foliage cards (or clumps) one tree places.
pub const MAX_CARDS: usize = 40_000;

/// Face groups and material slots of a tree mesh.
pub mod groups {
    pub const TRUNK: &str = "trunk";
    pub const ROOTS: &str = "roots";
    /// `level_1` to `level_3`.
    pub const LEVEL_PREFIX: &str = "level_";
    /// Vertex group of the wood's branch tips.
    pub const TWIG_TIPS: &str = "twig_tips";
    /// Every foliage face.
    pub const FOLIAGE: &str = "foliage";
    pub const CLUSTER_CARDS: &str = "foliage.cluster_cards";
    pub const LEAF_CARDS: &str = "foliage.leaf_cards";
    pub const CLUMPS: &str = "foliage.clumps";
    pub const SHELL: &str = "foliage.shell";
    pub const FRONDS: &str = "foliage.fronds";
    /// Material slot of bark faces.
    pub const BARK_MATERIAL: u16 = 0;
    /// Material slot of foliage faces.
    pub const LEAF_MATERIAL: u16 = 1;
}

/// How the wind data is stored per vertex (ADR 0014): the branch level as
/// vertex color alpha (0 trunk, 1/3 level 1, 2/3 level 2, 1 level 3 and
/// foliage), phase and stiffness as UV layer 1 (x phase 0 to 1, y
/// stiffness 1 rigid to 0 free). Engine profiles convert it on export.
pub const WIND_LAYOUT: &str = "level: color.a (0 trunk to 1 leaves), phase: uv1.x (0 to 1), stiffness: uv1.y (1 rigid to 0 free)";

/// Atlas layout of the foliage UVs (ADR 0014): the top half holds up to
/// four cluster variants side by side, the bottom half single leaves (and
/// palm fronds, conifer sprays) in the same columns.
pub const ATLAS_COLUMNS: u32 = 4;

/// A growth preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Species {
    /// A mature deciduous tree: a trunk to a fork at about 37% of the
    /// height, 3 to 5 scaffold limbs, a dome as wide as the tree is tall.
    Broadleaf,
    /// A single leader with whorls of drooping branches and a cone crown.
    Conifer,
    /// A curved trunk without branches and a crown of arching fronds.
    Palm,
    /// A slender leader with side limbs, a narrow crown and hanging twigs.
    Birch,
    /// A chunky trunk, three limbs and a round crown of solid clumps.
    Stylized,
}

impl Species {
    pub const ALL: [Species; 5] = [Species::Broadleaf, Species::Conifer, Species::Palm, Species::Birch, Species::Stylized];

    pub fn as_str(self) -> &'static str {
        match self {
            Species::Broadleaf => "broadleaf",
            Species::Conifer => "conifer",
            Species::Palm => "palm",
            Species::Birch => "birch",
            Species::Stylized => "stylized",
        }
    }

    pub fn parse(s: &str) -> Option<Species> {
        Species::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// A look: card layout and shading defaults (never a different technique
/// for the wood).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Cluster cards plus single leaves at the silhouette, normals mostly
    /// from the cards (AAA, about 10k triangles).
    Realistic,
    /// Fewer, larger cluster cards with strong crown normal transfer.
    Painted,
    /// Faceted solid clumps, no alpha.
    LowPoly,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Realistic, Style::Painted, Style::LowPoly];

    pub fn as_str(self) -> &'static str {
        match self {
            Style::Realistic => "realistic",
            Style::Painted => "painted",
            Style::LowPoly => "low_poly",
        }
    }

    pub fn parse(s: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// What the leaves become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foliage {
    /// Alpha cards along the twigs (cluster cards, single leaves, fronds).
    Cards,
    /// Solid blobs, faceted or smooth.
    Clumps,
    /// One closed crown skin.
    Shell,
    /// Bare wood.
    None,
}

impl Foliage {
    pub const ALL: [Foliage; 4] = [Foliage::Cards, Foliage::Clumps, Foliage::Shell, Foliage::None];

    pub fn as_str(self) -> &'static str {
        match self {
            Foliage::Cards => "cards",
            Foliage::Clumps => "clumps",
            Foliage::Shell => "shell",
            Foliage::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Foliage> {
        Foliage::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Everything that shapes a tree. Lengths in meters, angles in radians.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeSpec {
    pub species: Species,
    pub style: Style,
    pub height: f32,
    /// Trunk radius at breast height (1.3 m, or a ninth of the height on
    /// small trees).
    pub trunk_radius: f32,
    /// Lean of the trunk from vertical, and its heading (0 toward +X, a
    /// quarter turn toward +Z).
    pub lean: f32,
    pub lean_toward: f32,
    /// Height share of the first fork (broadleaf, stylized), the crown base
    /// (birch, conifer) or the top of the trunk (palm).
    pub fork: f32,
    /// Scaffold limbs from the fork (broadleaf, stylized), branches per
    /// whorl (conifer) or fronds (palm).
    pub limbs: u32,
    /// Branch levels built as wood after the trunk (1 to 3).
    pub levels: u32,
    /// Angle of branches from their parent (scaffold limbs from vertical).
    pub branch_angle: f32,
    /// Growth step of the branches (conifer: whorl spacing); 0 sizes it
    /// from the height.
    pub branch_spacing: f32,
    /// Radius ratio of a branch to its parent at an even split.
    pub taper: f32,
    /// Random bends and curves (0 straight to 1 gnarled).
    pub gnarl: f32,
    /// Twist of the trunk and the scaffold limbs along their length.
    pub twist: f32,
    /// Buttress roots blended into the trunk base (0 to 12).
    pub roots: u32,
    /// Spread of the root flare at the ground over the trunk diameter (1:
    /// no flare).
    pub root_spread: f32,
    pub crown: CrownShape,
    pub crown_width: f32,
    /// Lumps of the crown's outline as a share of its radius (0 smooth).
    pub crown_lumps: f32,
    /// Height share of the crown's lowest point; 0 derives it from the
    /// fork.
    pub crown_base: f32,
    pub foliage: Foliage,
    /// Cluster card (or clump) size.
    pub card_size: f32,
    /// Silhouette coverage target of the crown (0.1 to 1).
    pub coverage: f32,
    /// Share of single-leaf cards at the silhouette (0 to 1).
    pub leaf_cards: f32,
    /// Turn between successive cards around a twig.
    pub phyllotaxis: f32,
    /// Angle of the cards from their twig.
    pub spread: f32,
    /// Gravity droop of the cards, growing toward the twig tips (0 to 1).
    pub droop: f32,
    /// Turn of the cards toward the sky (0 to 1).
    pub sun_bias: f32,
    /// Crown normal transfer: 0 keeps the card normals, 1 uses the hull's.
    pub normal_blend: f32,
    /// Per-card hue and brightness variation (0 to 1).
    pub hue_jitter: f32,
    pub value_jitter: f32,
    /// Crown gradient: darker and cooler inside, lighter and warmer on the
    /// sunlit outside, plus ambient occlusion (0 to 1).
    pub gradient: f32,
    /// Size falloff from the crown center to the silhouette (0 to 1).
    pub size_falloff: f32,
    /// Atlas variants the cards pick from (1 to 4).
    pub variants: u32,
    /// Clumps faceted (flat) instead of smooth.
    pub facets: bool,
    /// Triangle budget at LOD0; the wood takes about a third.
    pub budget: u32,
    /// Attractors for space colonization; 0 sizes them from the crown.
    pub attractors: u32,
    pub seed: u64,
}

impl TreeSpec {
    /// The defaults of a species in its own style.
    pub fn preset(species: Species) -> TreeSpec {
        let deg = f32::to_radians;
        let base = TreeSpec {
            species,
            style: Style::Realistic,
            height: 12.0,
            trunk_radius: 0.42,
            lean: 0.0,
            lean_toward: 0.0,
            fork: 0.37,
            limbs: 4,
            levels: 3,
            branch_angle: deg(45.0),
            branch_spacing: 0.0,
            taper: 0.7,
            gnarl: 0.35,
            twist: deg(15.0),
            roots: 7,
            root_spread: 1.5,
            crown: CrownShape::Dome,
            crown_width: 12.0,
            crown_lumps: 0.16,
            crown_base: 0.0,
            foliage: Foliage::Cards,
            card_size: 0.6,
            coverage: 0.87,
            leaf_cards: 0.15,
            phyllotaxis: deg(137.5),
            spread: deg(55.0),
            droop: 0.35,
            sun_bias: 0.35,
            normal_blend: 0.4,
            hue_jitter: 0.25,
            value_jitter: 0.2,
            gradient: 0.6,
            size_falloff: 0.3,
            variants: 4,
            facets: false,
            budget: 10_000,
            attractors: 0,
            seed: 1,
        };
        match species {
            Species::Broadleaf => base,
            Species::Conifer => TreeSpec {
                height: 15.0,
                trunk_radius: 0.36,
                fork: 0.12,
                limbs: 5,
                levels: 2,
                branch_angle: deg(78.0),
                branch_spacing: 0.55,
                gnarl: 0.15,
                twist: 0.0,
                roots: 5,
                root_spread: 1.3,
                crown: CrownShape::Cone,
                crown_width: 5.8,
                crown_lumps: 0.06,
                card_size: 0.5,
                coverage: 0.85,
                leaf_cards: 0.0,
                spread: deg(62.0),
                droop: 0.45,
                sun_bias: 0.35,
                normal_blend: 0.45,
                ..base
            },
            Species::Palm => TreeSpec {
                height: 9.0,
                trunk_radius: 0.17,
                lean: deg(8.0),
                fork: 0.9,
                limbs: 16,
                levels: 1,
                branch_angle: deg(60.0),
                gnarl: 0.3,
                twist: 0.0,
                roots: 12,
                root_spread: 1.35,
                crown: CrownShape::Umbrella,
                crown_width: 7.5,
                crown_lumps: 0.0,
                card_size: 0.9,
                coverage: 0.7,
                leaf_cards: 0.0,
                droop: 0.55,
                sun_bias: 0.2,
                normal_blend: 0.3,
                budget: 4_000,
                ..base
            },
            Species::Birch => TreeSpec {
                height: 14.0,
                trunk_radius: 0.24,
                fork: 0.3,
                limbs: 0,
                branch_angle: deg(38.0),
                gnarl: 0.25,
                twist: deg(5.0),
                roots: 5,
                root_spread: 1.2,
                crown: CrownShape::Ellipsoid,
                crown_width: 6.3,
                crown_lumps: 0.12,
                card_size: 0.42,
                coverage: 0.76,
                droop: 0.75,
                sun_bias: 0.2,
                ..base
            },
            Species::Stylized => TreeSpec {
                style: Style::LowPoly,
                height: 5.0,
                trunk_radius: 0.3,
                fork: 0.38,
                limbs: 3,
                levels: 2,
                branch_angle: deg(40.0),
                gnarl: 0.5,
                twist: deg(25.0),
                roots: 5,
                root_spread: 1.7,
                crown: CrownShape::Sphere,
                crown_width: 4.8,
                crown_lumps: 0.1,
                foliage: Foliage::Clumps,
                card_size: 1.6,
                coverage: 0.9,
                leaf_cards: 0.0,
                normal_blend: 0.0,
                facets: true,
                budget: 2_500,
                ..base
            },
        }
    }

    /// Switches to another style: its foliage mode, card size and shading
    /// defaults.
    pub fn with_style(mut self, style: Style) -> TreeSpec {
        if style == self.style {
            return self;
        }
        let card = TreeSpec::preset(self.species).card_size;
        self.style = style;
        match style {
            Style::Realistic => {
                self.foliage = Foliage::Cards;
                self.card_size = if self.species == Species::Stylized { 0.6 } else { card };
                self.leaf_cards = if matches!(self.species, Species::Broadleaf | Species::Birch | Species::Stylized) { 0.15 } else { 0.0 };
                self.normal_blend = 0.4;
                self.facets = false;
                self.budget = self.budget.max(8_000);
            }
            Style::Painted => {
                self.foliage = Foliage::Cards;
                self.card_size = if self.species == Species::Stylized { 1.0 } else { card * 1.6 };
                self.leaf_cards = 0.0;
                self.normal_blend = 0.85;
                self.hue_jitter = 0.15;
                self.gradient = 0.8;
                self.facets = false;
                self.budget = 6_000;
            }
            Style::LowPoly => {
                self.foliage = Foliage::Clumps;
                self.card_size = if self.species == Species::Stylized { card } else { self.crown_width * 0.24 };
                self.leaf_cards = 0.0;
                self.normal_blend = 0.0;
                self.facets = true;
                self.budget = 3_000;
            }
        }
        self
    }

    /// Growth step: the given spacing, or a fortieth of the height.
    pub fn step(&self) -> f32 {
        if self.branch_spacing > 0.0 { self.branch_spacing } else { (self.height / 40.0).max(0.01) }
    }

    /// The crown hull for a crown axis at (x, z).
    pub fn hull(&self, axis: glam::Vec2) -> Hull {
        let base = if self.crown_base > 0.0 {
            self.crown_base
        } else {
            match self.species {
                Species::Conifer => self.fork,
                Species::Palm => self.fork * 0.72,
                _ => self.fork * 0.8,
            }
        };
        Hull {
            axis,
            base: base * self.height,
            top: self.height,
            radius: self.crown_width / 2.0,
            shape: self.crown.clone(),
            lumps: self.crown_lumps,
            seed: self.seed ^ 0x1u64.rotate_left(40),
        }
    }
}

/// Why a tree could not be built: the parameter and the problem.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{param}: {message}")]
pub struct VegError {
    pub param: &'static str,
    pub message: String,
}

impl VegError {
    pub fn new(param: &'static str, message: impl Into<String>) -> VegError {
        VegError { param, message: message.into() }
    }
}

fn check(name: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), VegError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(()) } else { Err(VegError::new(name, format!("{v} is outside {lo} to {hi}"))) }
}

fn check_count(name: &'static str, v: u32, lo: u32, hi: u32) -> Result<(), VegError> {
    if (lo..=hi).contains(&v) { Ok(()) } else { Err(VegError::new(name, format!("{v} is outside {lo} to {hi}"))) }
}

/// Checks every value of a spec.
///
/// Complexity: O(outline points).
pub fn validate(s: &TreeSpec) -> Result<(), VegError> {
    let pi = std::f32::consts::PI;
    check("height", s.height, 0.2, 120.0)?;
    check("trunk_radius", s.trunk_radius, 0.002, s.height * 0.25)?;
    check("lean", s.lean, 0.0, pi / 4.0)?;
    check("lean_toward", s.lean_toward, -2.0 * pi, 2.0 * pi)?;
    check("fork", s.fork, 0.02, 0.97)?;
    check_count("limbs", s.limbs, 0, 32)?;
    check_count("levels", s.levels, 1, 3)?;
    check("branch_angle", s.branch_angle, 0.0, pi * 0.75)?;
    check("branch_spacing", s.branch_spacing, 0.0, s.height)?;
    check("taper", s.taper, 0.3, 0.98)?;
    check("gnarl", s.gnarl, 0.0, 1.0)?;
    check("twist", s.twist, -2.0 * pi, 2.0 * pi)?;
    check_count("roots", s.roots, 0, 12)?;
    check("root_spread", s.root_spread, 1.0, 4.0)?;
    check("crown_width", s.crown_width, 0.05, s.height * 5.0)?;
    check("crown_base", s.crown_base, 0.0, 0.95)?;
    check("crown_lumps", s.crown_lumps, 0.0, 0.5)?;
    check("card_size", s.card_size, 0.005, s.height.max(s.crown_width))?;
    check("coverage", s.coverage, 0.05, 1.0)?;
    check("leaf_cards", s.leaf_cards, 0.0, 1.0)?;
    check("phyllotaxis", s.phyllotaxis, 0.0, 2.0 * pi)?;
    check("spread", s.spread, 0.0, pi / 2.0)?;
    for (n, v) in [
        ("droop", s.droop),
        ("sun_bias", s.sun_bias),
        ("normal_blend", s.normal_blend),
        ("hue_jitter", s.hue_jitter),
        ("value_jitter", s.value_jitter),
        ("gradient", s.gradient),
        ("size_falloff", s.size_falloff),
    ] {
        check(n, v, 0.0, 1.0)?;
    }
    check_count("variants", s.variants, 1, ATLAS_COLUMNS)?;
    check_count("budget", s.budget, 200, 1_000_000)?;
    check_count("attractors", s.attractors, 0, MAX_ATTRACTORS)?;
    if let CrownShape::Outline(points) = &s.crown {
        if points.len() < 2 || points.len() > 64 {
            return Err(VegError::new("crown", "an outline needs 2 to 64 points"));
        }
        if points.iter().any(|p| !p.is_finite() || p.x < 0.0 || !(0.0..=1.0).contains(&p.y)) {
            return Err(VegError::new("crown", "outline points are (radius share >= 0, height share 0 to 1)"));
        }
        if points.windows(2).any(|w| w[1].y < w[0].y) {
            return Err(VegError::new("crown", "outline points go from the crown base up"));
        }
        if points.iter().all(|p| p.x <= 0.0) {
            return Err(VegError::new("crown", "the outline has no width"));
        }
    }
    let base = s.hull(glam::Vec2::ZERO).base;
    if base >= s.height * 0.98 {
        return Err(VegError::new("crown_base", "the crown starts at the top of the tree"));
    }
    Ok(())
}

/// What the tree looks like as numbers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TreeReport {
    pub height: f32,
    pub crown_width: f32,
    pub crown_depth: f32,
    pub crown_height: f32,
    /// Lowest foliage point.
    pub crown_base: f32,
    pub fork_height: f32,
    pub trunk_radius: f32,
    /// Scaffold limbs, whorl branches or fronds.
    pub limbs: usize,
    pub nodes: usize,
    /// Branch chains built as wood.
    pub branches: usize,
    pub wood_triangles: usize,
    pub foliage_triangles: usize,
    /// Cluster cards, single-leaf cards, clumps or fronds.
    pub cards: usize,
    pub leaf_cards: usize,
    /// Silhouette coverage of the crown hull (mean of four side views).
    pub coverage: f32,
    /// Card layers drawn over the covered silhouette, on average.
    pub overdraw: f32,
    /// Neighbouring cards with the same atlas variant, flip and turn.
    pub repeats: usize,
    pub warnings: Vec<String>,
}

/// A grown tree: its mesh (bark material 0, foliage material 1), landmarks
/// and report.
#[derive(Debug, Clone)]
pub struct Tree {
    pub mesh: Mesh,
    pub landmarks: BTreeMap<String, Vec3>,
    pub report: TreeReport,
    pub hull: Hull,
}

/// Grows a tree.
///
/// Complexity: O(A x I) for space colonization over A attractors and I
/// growth rounds (each attractor checks only the nodes added near it in the
/// round, through a hash grid), plus O(N) for the skeleton's N nodes,
/// O(W) for the wood's W faces and O(C x P) for C cards rasterized over P
/// pixels each.
pub fn tree(spec: &TreeSpec) -> Result<Tree, VegError> {
    validate(spec)?;
    let sk = skeleton::grow(spec)?;
    let hull = spec.hull(sk.crown_axis);
    let mut b = build::Builder::default();
    let wood = wood::build(spec, &sk, &hull, &mut b)?;
    let fol = foliage::build(spec, &sk, &hull, &wood, &mut b)?;
    let wood_tris = wood.triangles;
    let mesh = b.finish()?;
    let mut landmarks = BTreeMap::new();
    let fork = sk.pos[sk.fork as usize];
    landmarks.insert("base".to_owned(), Vec3::ZERO);
    landmarks.insert("fork".to_owned(), fork);
    landmarks.insert("crown_top".to_owned(), hull.axis_point(hull.top));
    landmarks.insert("crown_center".to_owned(), hull.center());
    landmarks.insert("crown_base".to_owned(), hull.axis_point(hull.base));
    let mut mesh = mesh;
    bake(&mut mesh, &landmarks);
    let (lo, hi) = fol.bounds.unwrap_or((fork, fork));
    let tris_wood: usize = wood_tris;
    let report = TreeReport {
        height: mesh.bounds().map_or(0.0, |(_, h)| h.y),
        crown_width: hi.x - lo.x,
        crown_depth: hi.z - lo.z,
        crown_height: hi.y - lo.y,
        crown_base: lo.y,
        fork_height: fork.y,
        trunk_radius: spec.trunk_radius,
        limbs: sk.limbs,
        nodes: sk.pos.len(),
        branches: wood.chains,
        wood_triangles: tris_wood,
        foliage_triangles: mesh.triangle_count().saturating_sub(tris_wood),
        cards: fol.cards,
        leaf_cards: fol.leaf_cards,
        coverage: fol.coverage,
        overdraw: fol.overdraw,
        repeats: fol.repeats,
        warnings: fol.warnings,
    };
    Ok(Tree { mesh, landmarks, report, hull })
}

/// Puts each landmark on its nearest vertex as a one-vertex group
/// `lm.<name>`.
///
/// Complexity: O(V) per landmark.
fn bake(mesh: &mut Mesh, landmarks: &BTreeMap<String, Vec3>) {
    for (name, p) in landmarks {
        let v =
            mesh.verts().min_by(|&a, &b| mesh.positions[a as usize].distance_squared(*p).total_cmp(&mesh.positions[b as usize].distance_squared(*p)));
        if let Some(v) = v {
            // A fresh one-vertex group on a live mesh cannot fail.
            let _ = mesh.set_group(&format!("lm.{name}"), p3d_mesh::ElementKind::Vert, vec![v]);
        }
    }
}
