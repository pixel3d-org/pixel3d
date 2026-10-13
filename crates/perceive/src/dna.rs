//! Visual DNA (ADR 0011): the form genes an agent writes, measured back
//! from geometry in the same units, and the art-direction critique.
//!
//! Genes measured here: form hierarchy (primary, secondary and tertiary
//! volume shares), lines (share of smoothly curved surface, creases), taper
//! and gesture (centerline bend) of elongated parts, silhouette solidity in
//! the front and side views, bevel width variation, rest areas free of
//! small detail, part ratios, and for humanoids the head count, shoulder
//! width and leg length read from the body's front silhouette. Culture and
//! style presets (`dwarven`, `elven`, `orcish`, `human_medieval`;
//! `realistic`, `stylized_painterly`) and archetypes ([`ARCHETYPES`]) give
//! default targets; written genes override them.
//!
//! Critique codes: `tangent` (forms that just touch), `twinning` (equal
//! forms or even spacing side by side), `uniform_bevel` (one bevel width on
//! forms of very different sizes), `weak_silhouette` (an outline that reads
//! as a blob), `proportion` (outside the archetype's range), `stiff` (a
//! straight centerline where gesture is expected), `floating_detail` (a
//! small form touching nothing) and `uniform_detail` (detail everywhere, no
//! rest area). Each carries a [`Remedy`] that the spec layer turns into a
//! spec patch with a predicted result. Detail below pixel size is a
//! diagnostic (`DETAIL_SCALE`), not a gene.

use std::collections::BTreeMap;

use glam::{Vec2, Vec3};
use p3d_mesh::EdgeFlags;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::sample::surface_samples;
use crate::silhouette::{ViewDir, rasterize};
use crate::{Report, View};

/// Dihedral above which an edge is a crease even when smooth shaded.
const CREASE_DEG: f32 = 50.0;
/// Dihedral below which neighbouring faces count as flat.
const FLAT_DEG: f32 = 2.0;
/// Longest over second extent from which a part counts as elongated.
const ELONGATED: f32 = 2.5;
/// Bins along an elongated part's axis for its taper and bend.
const TAPER_BINS: usize = 12;
/// A secondary form is at least this share of the primary's volume.
const SECONDARY: f32 = 0.15;
/// Raster resolution for silhouettes.
const RESOLUTION: usize = 128;
/// Samples of the smaller part when measuring a contact patch.
const CONTACT_SAMPLES: usize = 256;
/// Most parts the pairwise critiques look at (twinning, rhythm).
const MAX_PAIR_PARTS: usize = 64;
/// Bands along the model's length for rest areas.
const REST_BANDS: usize = 8;
/// Rows of the body silhouette for humanoid measures.
const BODY_ROWS: usize = 160;

/// A gene range in the spec: `[min, max]`.
pub type Range2 = [f32; 2];

/// The `lines` gene: a preset word or explicit ranges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LinesGene {
    /// `angular`, `flowing`, `heavy` or `balanced`.
    Preset(String),
    Ranges {
        #[serde(default)]
        curved: Option<Range2>,
        #[serde(default)]
        taper: Option<Range2>,
    },
}

/// The `edges` gene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EdgesGene {
    /// `varied`, `uniform`, `sharp` or `soft`.
    Preset(String),
    /// Coefficient of variation of bevel widths.
    Ranges { bevel_variation: Range2 },
}

/// The `silhouette` gene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SilhouetteGene {
    /// Filled share of the outline's convex hull, front and side.
    #[serde(default)]
    pub solidity: Option<Range2>,
}

/// The `gesture` gene: how much elongated parts' centerlines bend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GestureGene {
    /// Parts that should bend; every elongated part when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
    /// The bend: the centerline's largest distance from its chord over the
    /// part's length (0.02 a hint of a curve, 0.1 a sabre).
    pub bend: Range2,
}

/// The genes as written (a spec's or brief's `dna` block).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DnaSpec {
    /// `dwarven`, `elven`, `orcish` or `human_medieval`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub culture: Option<String>,
    /// `realistic` or `stylized_painterly`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// The archetype whose proportion ranges and gesture apply (`dwarf`,
    /// `sword`, ...); archetype instances in a spec set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<String>,
    /// Primary, secondary and tertiary volume shares, for example `[0.6, 0.3, 0.1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<LinesGene>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<EdgesGene>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silhouette: Option<SilhouetteGene>,
    /// `varied` (repeated parts must vary) or `regular`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhythm: Option<String>,
    /// Centerline bend of elongated parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gesture: Option<GestureGene>,
    /// Share of the model's length free of small detail (rest areas).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest: Option<Range2>,
    /// Ratios: `{"blade_to_haft": [0.3, 0.45]}` (part heights), or the
    /// humanoid measures `heads`, `shoulders` (head heights) and `legs`
    /// (crotch height over height).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub proportions: BTreeMap<String, Range2>,
    /// Surface genes (wear, damage, ornament), built in Phase 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<serde_json::Value>,
    /// Seed, asymmetry and variants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variation: Option<serde_json::Value>,
    /// The vegetation archetype: a species word (`broadleaf`, `conifer`,
    /// `palm`, `birch`, `stylized`) or ranges (crate::vegetation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vegetation: Option<crate::vegetation::VegetationGene>,
}

/// An archetype's default genes and critique thresholds (ADR 0011, ADR
/// 0016). Add a category by adding an entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArchetypeGenes {
    pub id: &'static str,
    /// Proportion ranges (see [`DnaSpec::proportions`]).
    pub proportions: &'static [(&'static str, Range2)],
    /// The parts whose centerline may carry gesture (a blade, a wing);
    /// empty: every elongated part.
    pub gesture_parts: &'static [&'static str],
    /// The archetype's own bend range for those parts; otherwise the
    /// culture's (flowing lines expect a bend).
    pub gesture: Option<Range2>,
    /// The share of curved surface, when the archetype's forms decide it
    /// rather than the culture (bodies are curved whatever the culture).
    pub curved: Option<Range2>,
}

const BODY: Option<Range2> = Some([0.6, 1.0]);

/// Every archetype with genes.
pub const ARCHETYPES: &[ArchetypeGenes] = &[
    ArchetypeGenes {
        id: "human",
        proportions: &[("heads", [7.0, 8.0]), ("shoulders", [1.75, 2.3]), ("legs", [0.43, 0.53])],
        gesture_parts: &["none"],
        gesture: None,
        curved: BODY,
    },
    ArchetypeGenes {
        id: "elf",
        proportions: &[("heads", [7.4, 8.6]), ("shoulders", [1.5, 2.0]), ("legs", [0.46, 0.56])],
        gesture_parts: &["none"],
        gesture: None,
        curved: BODY,
    },
    ArchetypeGenes {
        id: "dwarf",
        proportions: &[("heads", [3.9, 4.8]), ("shoulders", [1.8, 2.5]), ("legs", [0.3, 0.42])],
        gesture_parts: &["none"],
        gesture: None,
        curved: BODY,
    },
    ArchetypeGenes {
        id: "orc",
        proportions: &[("heads", [6.4, 7.6]), ("shoulders", [2.1, 2.9]), ("legs", [0.39, 0.5])],
        gesture_parts: &["none"],
        gesture: None,
        curved: BODY,
    },
    ArchetypeGenes {
        id: "halfling",
        proportions: &[("heads", [4.4, 5.6]), ("shoulders", [1.4, 1.95]), ("legs", [0.34, 0.47])],
        gesture_parts: &["none"],
        gesture: None,
        curved: BODY,
    },
    ArchetypeGenes { id: "sword", proportions: &[("blade_to_grip", [3.0, 6.5])], gesture_parts: &["blade"], gesture: None, curved: None },
    ArchetypeGenes { id: "dagger", proportions: &[("blade_to_grip", [1.5, 3.5])], gesture_parts: &["blade"], gesture: None, curved: None },
    ArchetypeGenes { id: "axe", proportions: &[("blade_to_haft", [0.18, 0.5])], gesture_parts: &["haft"], gesture: None, curved: None },
    ArchetypeGenes { id: "hammer", proportions: &[("hammer_head_to_haft", [0.15, 0.45])], gesture_parts: &["haft"], gesture: None, curved: None },
    ArchetypeGenes { id: "spear", proportions: &[("spearhead_to_shaft", [0.06, 0.3])], gesture_parts: &["spearhead"], gesture: None, curved: None },
    ArchetypeGenes { id: "feathered_wings", proportions: &[], gesture_parts: &["wing_l", "wing_r"], gesture: Some([0.03, 0.45]), curved: None },
    ArchetypeGenes { id: "membrane_wings", proportions: &[], gesture_parts: &["wing_l", "wing_r"], gesture: Some([0.03, 0.45]), curved: None },
];

/// The genes of an archetype.
///
/// Complexity: O(archetypes).
pub fn archetype_genes(id: &str) -> Option<&'static ArchetypeGenes> {
    ARCHETYPES.iter().find(|a| a.id == id)
}

/// Targets after presets: every gene that has a target.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Targets {
    pub hierarchy: Option<[f32; 3]>,
    pub curved: Option<Range2>,
    pub taper: Option<Range2>,
    pub bevel_variation: Option<Range2>,
    pub solidity: Option<Range2>,
    pub varied_rhythm: bool,
    /// Parts that should bend (empty: every elongated part) and the range.
    pub gesture: Option<(Vec<String>, Range2)>,
    pub rest: Option<Range2>,
    pub proportions: BTreeMap<String, Range2>,
    /// The archetype the proportions came from.
    pub archetype: Option<String>,
}

fn lines_preset(word: &str) -> Option<(Range2, Range2)> {
    // (curved share, taper)
    Some(match word {
        "angular" => ([0.0, 0.4], [0.0, 0.35]),
        "flowing" => ([0.55, 1.0], [0.35, 1.0]),
        "heavy" => ([0.15, 0.6], [0.0, 0.3]),
        "balanced" => ([0.25, 0.75], [0.1, 0.6]),
        _ => return None,
    })
}

fn edges_preset(word: &str) -> Option<Range2> {
    Some(match word {
        "varied" => [0.25, 2.0],
        "uniform" => [0.0, 0.15],
        "sharp" | "soft" => [0.0, 2.0],
        _ => return None,
    })
}

/// Resolves presets, the archetype and written genes into targets.
///
/// Complexity: O(genes + archetypes).
pub fn targets(d: &DnaSpec) -> Result<Targets, (String, String)> {
    let mut t = Targets::default();
    let culture = d.culture.as_deref();
    let (lines, edges, hierarchy, varied) = match culture {
        // Blocky, heavy bases, symmetric, chunky bevels.
        Some("dwarven") => ("angular", "varied", [0.65, 0.25, 0.1], false),
        // Tapering S-curves, lighter masses, slight asymmetry.
        Some("elven") => ("flowing", "varied", [0.55, 0.3, 0.15], true),
        // Crude and massive.
        Some("orcish") => ("heavy", "varied", [0.7, 0.2, 0.1], true),
        Some("human_medieval") | None => ("balanced", "varied", [0.6, 0.3, 0.1], false),
        Some(other) => return Err(("/culture".into(), format!("`{other}` is not a culture preset; use dwarven, elven, orcish or human_medieval"))),
    };
    if culture.is_some() {
        let (c, tp) = lines_preset(lines).unwrap_or(([0.0, 1.0], [0.0, 1.0]));
        t.curved = Some(c);
        t.taper = Some(tp);
        t.bevel_variation = edges_preset(edges);
        t.hierarchy = Some(hierarchy);
        t.varied_rhythm = varied;
    }
    // Flowing lines expect elongated forms to bend a little.
    if lines == "flowing" && culture.is_some() {
        t.gesture = Some((Vec::new(), [0.015, 1.0]));
    }
    match d.style.as_deref() {
        // Painterly stylization reads at a glance: a strong outline.
        Some("stylized_painterly") => {
            t.solidity = Some([0.45, 0.88]);
            t.varied_rhythm = true;
        }
        Some("realistic") => t.solidity = Some([0.4, 0.95]),
        None => {}
        Some(other) => return Err(("/style".into(), format!("`{other}` is not a style preset; use realistic or stylized_painterly"))),
    }
    if let Some(a) = d.archetype.as_deref() {
        let genes = archetype_genes(a).ok_or_else(|| {
            let ids: Vec<&str> = ARCHETYPES.iter().map(|g| g.id).collect();
            ("/archetype".to_owned(), format!("`{a}` has no genes; use one of {}", ids.join(", ")))
        })?;
        for (k, r) in genes.proportions {
            t.proportions.insert((*k).to_owned(), *r);
        }
        // Gesture belongs to the archetype's bending parts: its own range,
        // else the culture's.
        let parts: Vec<String> = genes.gesture_parts.iter().map(|p| (*p).to_owned()).collect();
        t.gesture = match (genes.gesture, t.gesture.take()) {
            (Some(r), _) => Some((parts, r)),
            (None, Some((_, r))) if !genes.gesture_parts.is_empty() => Some((parts, r)),
            (None, other) => other,
        };
        if genes.curved.is_some() {
            t.curved = genes.curved;
        }
        t.archetype = Some(a.to_owned());
    }
    if let Some(h) = d.hierarchy {
        let sum: f32 = h.iter().sum();
        if !(0.95..=1.05).contains(&sum) || h.iter().any(|x| *x < 0.0) {
            return Err(("/hierarchy".into(), "the three shares must be positive and add up to 1".into()));
        }
        t.hierarchy = Some(h);
    }
    match &d.lines {
        Some(LinesGene::Preset(w)) => {
            let (c, tp) = lines_preset(w)
                .ok_or_else(|| ("/lines".to_owned(), format!("`{w}` is not a lines preset; use angular, flowing, heavy or balanced")))?;
            t.curved = Some(c);
            t.taper = Some(tp);
        }
        Some(LinesGene::Ranges { curved, taper }) => {
            t.curved = curved.or(t.curved);
            t.taper = taper.or(t.taper);
        }
        None => {}
    }
    match &d.edges {
        Some(EdgesGene::Preset(w)) => {
            t.bevel_variation = Some(
                edges_preset(w).ok_or_else(|| ("/edges".to_owned(), format!("`{w}` is not an edges preset; use varied, uniform, sharp or soft")))?,
            );
        }
        Some(EdgesGene::Ranges { bevel_variation }) => t.bevel_variation = Some(*bevel_variation),
        None => {}
    }
    if let Some(s) = d.silhouette.as_ref().and_then(|s| s.solidity) {
        t.solidity = Some(s);
    }
    match d.rhythm.as_deref() {
        Some("varied") => t.varied_rhythm = true,
        Some("regular") => t.varied_rhythm = false,
        None => {}
        Some(other) => return Err(("/rhythm".into(), format!("`{other}` is not a rhythm; use varied or regular"))),
    }
    if let Some(g) = &d.gesture {
        if !(g.bend[0] >= 0.0 && g.bend[1] >= g.bend[0]) {
            return Err(("/gesture/bend".into(), "give [min, max] with 0 <= min <= max".into()));
        }
        t.gesture = Some((g.parts.clone(), g.bend));
    }
    if let Some(r) = d.rest {
        if !(r[0] >= 0.0 && r[1] >= r[0]) {
            return Err(("/rest".into(), "give [min, max] shares from 0 to 1".into()));
        }
        t.rest = Some(r);
    }
    for (k, r) in &d.proportions {
        t.proportions.insert(k.clone(), *r);
    }
    if let Some(v) = &d.vegetation {
        crate::vegetation::gene_targets(v)?;
    }
    Ok(t)
}

/// What the geometry shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Measured {
    /// Volume shares of primary, secondary and tertiary forms.
    pub hierarchy: [f32; 3],
    /// Forms counted (parts, or a single part's shape groups).
    pub forms: usize,
    /// Share of surface area that bends smoothly.
    pub curved: f32,
    /// Crease edges (hard or above 50 degrees).
    pub creases: usize,
    /// Taper of each elongated form: 1 - thinner end / widest section.
    pub tapers: Vec<(String, f32)>,
    /// Bend of each elongated form: the centerline's largest distance from
    /// its chord over the length.
    pub bends: Vec<(String, f32)>,
    /// Filled share of the convex hull, front and side.
    pub solidity: [f32; 2],
    /// Bevel faces found and the variation of their widths (CV) with the
    /// median width in meters.
    pub bevels: usize,
    pub bevel_variation: f32,
    pub bevel_width: f32,
    /// Smallest and largest form size, meters.
    pub form_sizes: [f32; 2],
    /// Small forms (tertiary parts) and the share of the length free of them.
    pub details: usize,
    pub rest: f32,
    pub ratios: BTreeMap<String, f32>,
}

/// A change the critique proposes, in world terms; the spec layer turns it
/// into a spec patch and predicts its result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Remedy {
    /// Scale a part uniformly.
    Scale { part: String, factor: f32 },
    /// Move a part by a world offset.
    Move { part: String, by: Vec3 },
    /// Push a part's middle sideways (world offset), its ends held.
    Bend { part: String, by: Vec3, along: Vec3 },
    /// Space parts unevenly between their ends.
    Space { parts: Vec<String>, from: Vec3, to: Vec3 },
    /// Bring a measured proportion to a target.
    Proportion { gene: String, measured: f32, target: f32 },
    /// Leave parts out (a rest area).
    Remove { parts: Vec<String> },
}

/// One critique finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Critique {
    pub code: &'static str,
    /// What it is about: a part, a gene (`shoulders`) or a word
    /// (`bevels`, `outline`, `detail`); stable across revisions.
    pub subject: String,
    pub parts: Vec<String>,
    pub message: String,
    pub fix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<Remedy>,
}

fn volume(p: &crate::Part) -> f32 {
    let m = p.mesh;
    let mut v = 0.0f64;
    for f in m.faces() {
        for t in m.face_triangles(f) {
            let [a, b, c] = t.map(|h| p.world.transform_point3(m.positions[m.origin(h) as usize]).as_dvec3());
            v += a.dot(b.cross(c)) / 6.0;
        }
    }
    let v = v.abs() as f32;
    if v > 0.0 { v } else { p.size().x * p.size().y * p.size().z * 0.5 }
}

/// Volume shares of the three tiers: the largest form is primary, forms of
/// at least 15% of its volume secondary, the rest tertiary.
fn tiers(vols: &[f32]) -> [f32; 3] {
    let total: f32 = vols.iter().sum::<f32>().max(1e-12);
    let big = vols.iter().copied().fold(0.0f32, f32::max);
    let mut out = [0.0f32; 3];
    let mut primary_taken = false;
    for &v in vols {
        if v == big && !primary_taken {
            out[0] += v;
            primary_taken = true;
        } else if v >= SECONDARY * big {
            out[1] += v;
        } else {
            out[2] += v;
        }
    }
    out.map(|x| x / total)
}

/// Convex hull area of 2D points (monotone chain).
///
/// Complexity: O(n log n).
fn hull_area(mut pts: Vec<Vec2>) -> f32 {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return 0.0;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in iter {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    let n = hull.len();
    (0..n).map(|i| hull[i].perp_dot(hull[(i + 1) % n])).sum::<f32>().abs() * 0.5
}

/// Filled share of the convex hull of a view's raster.
///
/// Complexity: O(cells log cells).
fn solidity(view: &View, dir: ViewDir) -> f32 {
    let r = rasterize(view, dir, RESOLUTION, None);
    let mut pts = Vec::new();
    let mut filled = 0usize;
    for j in 0..r.h {
        for i in 0..r.w {
            if r.owner(i, j).is_some() {
                filled += 1;
                // Cell corners, so the hull covers whole cells.
                for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    pts.push(Vec2::new((i + di) as f32, (j + dj) as f32));
                }
            }
        }
    }
    let hull = hull_area(pts);
    if hull > 0.0 { (filled as f32 / hull).min(1.0) } else { 1.0 }
}

/// Whether a part is strands (hair, fur, grass): left out of form genes
/// that read solid masses.
fn is_strands(view: &View, p: &crate::Part) -> bool {
    view.doc.object(p.key).is_some_and(|o| o.tags.iter().any(|t| t == "strands"))
}

/// Taper and bend of an elongated part along its longest world axis, or
/// none when it is not elongated.
///
/// Complexity: O(V).
fn centerline(p: &crate::Part) -> Option<(f32, f32, Vec3, Vec3)> {
    let s = p.size();
    let mut dims = [(s.x, 0usize), (s.y, 1), (s.z, 2)];
    dims.sort_by(|a, b| b.0.total_cmp(&a.0));
    if dims[1].0 <= 0.0 || dims[0].0 / dims[1].0 < ELONGATED {
        return None;
    }
    let axis = dims[0].1;
    let (lo, hi) = (p.lo[axis], p.hi[axis]);
    let mut sums = [(Vec3::ZERO, 0u32); TAPER_BINS];
    let bin = |x: f32| (((x - lo) / (hi - lo).max(1e-9)) * TAPER_BINS as f32).clamp(0.0, TAPER_BINS as f32 - 1.0) as usize;
    for v in p.mesh.verts() {
        let q = p.world.transform_point3(p.mesh.positions[v as usize]);
        let b = bin(q[axis]);
        sums[b].0 += q;
        sums[b].1 += 1;
    }
    let mut radius = [0.0f32; TAPER_BINS];
    for v in p.mesh.verts() {
        let q = p.world.transform_point3(p.mesh.positions[v as usize]);
        let b = bin(q[axis]);
        let c = sums[b].0 / sums[b].1.max(1) as f32;
        let mut d = q - c;
        d[axis] = 0.0;
        radius[b] = radius[b].max(d.length());
    }
    let used: Vec<(f32, Vec3)> = radius.iter().zip(&sums).filter(|(_, s)| s.1 > 0).map(|(r, s)| (*r, s.0 / s.1 as f32)).collect();
    let (Some(&(first, a)), Some(&(last, b))) = (used.first(), used.last()) else { return None };
    let widest = used.iter().map(|u| u.0).fold(0.0f32, f32::max).max(1e-9);
    // The centerline's largest distance from the chord between its ends.
    let chord = b - a;
    let len = chord.length().max(1e-9);
    let mut far = (0.0f32, Vec3::ZERO);
    for &(_, c) in &used {
        let t = ((c - a).dot(chord) / (len * len)).clamp(0.0, 1.0);
        let off = c - (a + chord * t);
        if off.length() > far.0 {
            far = (off.length(), off);
        }
    }
    Some((1.0 - first.min(last) / widest, far.0 / dims[0].0.max(1e-9), chord / len, far.1))
}

/// A front silhouette of some parts: per row the filled cells' x extent,
/// and per cell whether it is filled.
struct Front {
    cell: f32,
    lo: Vec2,
    w: usize,
    h: usize,
    filled: Vec<bool>,
}

impl Front {
    /// Rasterizes the triangles of the chosen parts seen from the front.
    ///
    /// Complexity: O(triangles + covered cells).
    fn of(view: &View, parts: &[usize]) -> Option<Front> {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for &i in parts {
            let p = &view.parts[i];
            lo = lo.min(Vec2::new(p.lo.x, p.lo.y));
            hi = hi.max(Vec2::new(p.hi.x, p.hi.y));
        }
        if hi.y <= lo.y || !hi.y.is_finite() || !lo.y.is_finite() {
            return None;
        }
        let cell = (hi.y - lo.y) / BODY_ROWS as f32;
        let w = ((hi.x - lo.x) / cell).ceil() as usize + 1;
        let h = BODY_ROWS + 1;
        let mut f = Front { cell, lo, w, h, filled: vec![false; w * h] };
        for &i in parts {
            let bvh = view.parts[i].bvh();
            for t in 0..bvh.len() as u32 {
                let tri = bvh.triangle(t).map(|q| (Vec2::new(q.x, q.y) - lo) / cell);
                f.fill(tri);
            }
        }
        Some(f)
    }

    fn fill(&mut self, p: [Vec2; 3]) {
        let mn = p[0].min(p[1]).min(p[2]);
        let mx = p[0].max(p[1]).max(p[2]);
        let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        let (i0, j0) = (mn.x.floor().max(0.0) as usize, mn.y.floor().max(0.0) as usize);
        let (i1, j1) = ((mx.x.floor() as usize).min(self.w - 1), (mx.y.floor() as usize).min(self.h - 1));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let c = Vec2::new(i as f32 + 0.5, j as f32 + 0.5);
                let inside = area.abs() > 1e-12 && {
                    let w0 = (p[2] - p[1]).perp_dot(c - p[1]) / area;
                    let w1 = (p[0] - p[2]).perp_dot(c - p[2]) / area;
                    w0 >= 0.0 && w1 >= 0.0 && w0 + w1 <= 1.0
                };
                if inside {
                    self.filled[j * self.w + i] = true;
                }
            }
        }
        // Corners always count, so thin parts still show.
        for q in p {
            let (i, j) = (q.x.floor() as isize, q.y.floor() as isize);
            if i >= 0 && j >= 0 && (i as usize) < self.w && (j as usize) < self.h {
                self.filled[j as usize * self.w + i as usize] = true;
            }
        }
    }

    /// Width of a row (outermost filled cells), meters.
    fn width(&self, j: usize) -> f32 {
        let row = &self.filled[j * self.w..(j + 1) * self.w];
        match (row.iter().position(|&x| x), row.iter().rposition(|&x| x)) {
            (Some(a), Some(b)) => (b - a + 1) as f32 * self.cell,
            _ => 0.0,
        }
    }

    fn row_of(&self, y: f32) -> usize {
        (((y - self.lo.y) / self.cell) as usize).min(self.h - 1)
    }
}

/// Humanoid measures from the front silhouette of the body (parts tagged
/// `anatomy`, else every solid part): height in head heights (chin from a
/// `head` part's `chin` group, else the neck's narrowest row), shoulder
/// width in head heights at the deltoids, and crotch height over height
/// when the legs stand apart.
///
/// Complexity: O(triangles of the body + raster cells).
fn humanoid(view: &View) -> Option<(f32, f32, Option<f32>)> {
    let tagged: Vec<usize> =
        (0..view.parts.len()).filter(|&i| view.doc.object(view.parts[i].key).is_some_and(|o| o.tags.iter().any(|t| t == "anatomy"))).collect();
    let body: Vec<usize> = if tagged.is_empty() { (0..view.parts.len()).filter(|&i| !is_strands(view, &view.parts[i])).collect() } else { tagged };
    let front = Front::of(view, &body)?;
    let (bottom, top) = (front.lo.y, front.lo.y + front.cell * BODY_ROWS as f32);
    let height = top - bottom;
    let head = body.iter().map(|&i| &view.parts[i]).find(|p| p.label == "head" || p.label.ends_with("/head"));
    let chin = head.and_then(|p| {
        let g = p.mesh.group("chin")?;
        g.iter()
            .flat_map(|f| p.mesh.face_verts(f).collect::<Vec<_>>())
            .map(|v| p.world.transform_point3(p.mesh.positions[v as usize]).y)
            .reduce(f32::min)
    });
    let chin = match chin {
        Some(c) => c,
        None => {
            // The neck: the narrowest row below the head's widest, above the shoulders.
            let top_row = front.h - 1;
            let head_rows = top_row.saturating_sub(BODY_ROWS / 4)..top_row;
            let widest = head_rows.clone().max_by(|&a, &b| front.width(a).total_cmp(&front.width(b)))?;
            let mut neck = widest;
            for j in (top_row.saturating_sub(BODY_ROWS / 2)..widest).rev() {
                if front.width(j) > front.width(widest) * 1.6 {
                    break;
                }
                if front.width(j) < front.width(neck) {
                    neck = j;
                }
            }
            bottom + (neck as f32 + 0.5) * front.cell + (top - bottom) * 0.02
        }
    };
    let head_h = (top - chin).max(1e-6);
    let shoulders = front.width(front.row_of(chin - 0.45 * head_h)) / head_h;
    // The crotch: the first filled cell up the middle column.
    let mid = ((front.w as f32 * 0.5) as usize).min(front.w - 1);
    let legs = (0..front.h).find(|&j| front.filled[j * front.w + mid]).filter(|&j| j > 1).map(|j| j as f32 * front.cell / height);
    Some((height / head_h, shoulders, legs))
}

/// Measures the genes of a view.
///
/// Complexity: O(faces + edges) per part, O(V) per elongated part for its
/// taper and bend, two rasters for silhouettes, one for humanoid measures
/// when `humanoid` is set.
pub fn measure(view: &View) -> Measured {
    measure_with(view, false)
}

fn measure_with(view: &View, humanoid_measures: bool) -> Measured {
    let mut m = Measured::default();
    // Forms: parts, or the shape groups of a single part.
    // A form's size for bevels is its cross-section: the middle extent.
    let middle = |s: Vec3| s.x + s.y + s.z - s.max_element() - s.min_element();
    let mut forms: Vec<(String, f32, f32)> = view.parts.iter().map(|p| (p.label.clone(), volume(p), middle(p.size()))).collect();
    if view.parts.len() == 1 {
        let p = &view.parts[0];
        let mesh = p.mesh;
        let groups: Vec<(String, f32, f32)> = mesh
            .groups()
            .filter(|(_, g)| g.kind() == p3d_mesh::ElementKind::Face)
            .map(|(n, g)| {
                let area: f32 = g.iter().map(|f| mesh.face_area(f)).sum();
                (n.to_owned(), area.powf(1.5), area.sqrt())
            })
            .collect();
        if groups.len() >= 2 {
            forms = groups;
        }
    }
    let vols: Vec<f32> = forms.iter().map(|f| f.1).collect();
    m.hierarchy = tiers(&vols);
    m.forms = forms.len();
    m.form_sizes = [forms.iter().map(|f| f.2).fold(f32::MAX, f32::min), forms.iter().map(|f| f.2).fold(0.0, f32::max)];
    // Lines and bevels, per part.
    let (mut curved, mut area) = (0.0f32, 0.0f32);
    let mut widths: Vec<f32> = Vec::new();
    for p in &view.parts {
        let mesh = p.mesh;
        let crease_cos = CREASE_DEG.to_radians().cos();
        let flat_cos = FLAT_DEG.to_radians().cos();
        let mut is_crease = vec![false; mesh.edge_slots()];
        for e in mesh.edges() {
            let [Some(f), Some(g)] = mesh.edge_faces(e) else { continue };
            let c = mesh.face_normal(f).dot(mesh.face_normal(g));
            if c < crease_cos || mesh.edge_flags(e).contains(EdgeFlags::SHARP) {
                is_crease[e as usize] = true;
                m.creases += 1;
            }
        }
        let size = p.size().max_element().max(1e-6);
        for f in mesh.faces() {
            let a = mesh.face_area(f);
            area += a;
            let n = mesh.face_normal(f);
            let mut bends = false;
            for h in mesh.face_halfedges(f) {
                let e = mesh.edge_of(h);
                if is_crease[e as usize] {
                    continue;
                }
                if let Some(g) = mesh.face(mesh.twin(h))
                    && n.dot(mesh.face_normal(g)) < flat_cos
                {
                    bends = true;
                }
            }
            if bends {
                curved += a;
            }
            // A bevel face: a narrow quad whose long sides both turn.
            if mesh.face_len(f) == 4 {
                let hs: Vec<u32> = mesh.face_halfedges(f).collect();
                let len = |h: u32| mesh.positions[mesh.origin(h) as usize].distance(mesh.positions[mesh.dest(h) as usize]);
                let (a0, a1) = ((len(hs[0]) + len(hs[2])) * 0.5, (len(hs[1]) + len(hs[3])) * 0.5);
                let (w, l, long) = if a0 < a1 { (a0, a1, [hs[1], hs[3]]) } else { (a1, a0, [hs[0], hs[2]]) };
                let turns = |h: u32| {
                    mesh.face(mesh.twin(h)).is_some_and(|g| {
                        let c = n.dot(mesh.face_normal(g));
                        (0.2..0.985).contains(&c)
                    })
                };
                if w < 0.08 * size && l > 3.0 * w && turns(long[0]) && turns(long[1]) {
                    let s = p.world.matrix3.x_axis.length();
                    widths.push(w * s);
                }
            }
        }
    }
    m.curved = if area > 0.0 { curved / area } else { 0.0 };
    m.bevels = widths.len();
    if !widths.is_empty() {
        widths.sort_by(f32::total_cmp);
        let mean = widths.iter().sum::<f32>() / widths.len() as f32;
        let var = widths.iter().map(|w| (w - mean) * (w - mean)).sum::<f32>() / widths.len() as f32;
        m.bevel_variation = if mean > 0.0 { var.sqrt() / mean } else { 0.0 };
        m.bevel_width = widths[widths.len() / 2];
    }
    // Taper and bend of elongated parts along their longest world axis.
    for p in &view.parts {
        if is_strands(view, p) {
            continue;
        }
        if let Some((taper, bend, _, _)) = centerline(p) {
            m.tapers.push((p.label.clone(), taper));
            m.bends.push((p.label.clone(), bend));
        }
    }
    // Rest areas: bands along the longest axis with no small form in them.
    let span = view.hi - view.lo;
    let axis = span.max_position();
    let big = view.parts.iter().map(volume).fold(0.0f32, f32::max);
    let mut busy = [false; REST_BANDS];
    for p in view.parts.iter().filter(|p| !is_strands(view, p) && volume(p) < SECONDARY * big) {
        m.details += 1;
        let t = ((p.center()[axis] - view.lo[axis]) / span[axis].max(1e-9)).clamp(0.0, 0.999);
        busy[(t * REST_BANDS as f32) as usize] = true;
    }
    m.rest = busy.iter().filter(|b| !**b).count() as f32 / REST_BANDS as f32;
    m.solidity = [solidity(view, ViewDir::Front), solidity(view, ViewDir::Right)];
    if humanoid_measures && let Some((heads, shoulders, legs)) = humanoid(view) {
        m.ratios.insert("heads".into(), heads);
        m.ratios.insert("shoulders".into(), shoulders);
        if let Some(l) = legs {
            m.ratios.insert("legs".into(), l);
        }
    }
    m
}

/// Height ratios between named parts (`<a>_to_<b>`).
fn ratios(view: &View, wanted: &BTreeMap<String, Range2>, out: &mut BTreeMap<String, f32>) {
    for name in wanted.keys() {
        let Some((a, b)) = name.split_once("_to_") else { continue };
        let h = |n: &str| view.parts.iter().find(|p| p.label == n || p.label.ends_with(&format!("/{n}"))).map(|p| p.size().max_element());
        if let (Some(x), Some(y)) = (h(a), h(b)) {
            out.insert(name.clone(), x / y.max(1e-9));
        }
    }
}

/// Art-direction critique of a view.
///
/// Complexity: O(p^2) part pairs for twinning and rhythm over at most 64
/// parts (more are skipped), a contact sample per touching pair, and the
/// contact graph for floating detail.
pub fn critique(view: &View, measured: &Measured, targets: &Targets) -> Vec<Critique> {
    let mut out = Vec::new();
    let n = view.parts.len().min(MAX_PAIR_PARTS);
    let center = (view.lo + view.hi) * 0.5;
    let solid = |i: usize| !is_strands(view, &view.parts[i]);
    // Twinning: equal forms side by side that are not mirror images.
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&view.parts[i], &view.parts[j]);
            let (sa, sb) = (a.size(), b.size());
            let same = (sa - sb).abs().max_element() <= 0.04 * sa.max(sb).max_element();
            let mirrored = (a.center().x - center.x + b.center().x - center.x).abs() < 0.02 * (view.hi.x - view.lo.x).max(1e-6)
                && (a.center().y - b.center().y).abs() < 0.02 * sa.y.max(1e-6)
                && (a.center().z - b.center().z).abs() < 0.02 * sa.z.max(1e-6);
            let near = a.center().distance(b.center()) < 3.0 * sa.max_element();
            if same && near && !mirrored && sa.max_element() > 0.0 && solid(i) && solid(j) {
                out.push(Critique {
                    code: "twinning",
                    subject: b.label.clone(),
                    parts: vec![a.label.clone(), b.label.clone()],
                    message: format!("{} and {} are the same size side by side", a.label, b.label),
                    fix: format!("vary them: scale {} to about 75% or move it off the line", b.label),
                    remedy: Some(Remedy::Scale { part: b.label.clone(), factor: 0.75 }),
                });
            }
        }
    }
    // Rhythm: three or more like forms in a row at even spacing.
    if targets.varied_rhythm {
        let mut used = vec![false; n];
        for i in 0..n {
            if used[i] || !solid(i) {
                continue;
            }
            let si = view.parts[i].size();
            let group: Vec<usize> =
                (i..n).filter(|&j| solid(j) && (view.parts[j].size() - si).abs().max_element() <= 0.06 * si.max_element().max(1e-9)).collect();
            if group.len() < 3 {
                continue;
            }
            for &j in &group {
                used[j] = true;
            }
            let pts: Vec<Vec3> = group.iter().map(|&j| view.parts[j].center()).collect();
            let (first, last) = (pts[0], pts[pts.len() - 1]);
            let dir = (last - first).normalize_or(Vec3::X);
            let mut along: Vec<(f32, usize)> = pts.iter().enumerate().map(|(k, p)| ((*p - first).dot(dir), k)).collect();
            along.sort_by(|a, b| a.0.total_cmp(&b.0));
            let gaps: Vec<f32> = along.windows(2).map(|w| w[1].0 - w[0].0).collect();
            let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
            let cv = (gaps.iter().map(|g| (g - mean) * (g - mean)).sum::<f32>() / gaps.len() as f32).sqrt() / mean.max(1e-9);
            if mean > 0.0 && cv < 0.05 {
                let names: Vec<String> = along.iter().map(|&(_, k)| view.parts[group[k]].label.clone()).collect();
                let (a, b) = (pts[along[0].1], pts[along[along.len() - 1].1]);
                out.push(Critique {
                    code: "twinning",
                    subject: names[0].clone(),
                    parts: names.clone(),
                    message: format!("{} like forms are evenly spaced (gaps vary {:.0}%): a mechanical rhythm", names.len(), cv * 100.0),
                    fix: "space them with a rhythm (long, short, ...): a distribute constraint with spacing varied".into(),
                    remedy: Some(Remedy::Space { parts: names, from: a, to: b }),
                });
            }
        }
    }
    // Tangents: touching forms whose contact is a point or a line.
    let rel = crate::relations::relations(view);
    for r in rel.pairs.iter().filter(|r| r.kind == crate::relations::RelationKind::Touching) {
        if !solid(r.a) || !solid(r.b) {
            continue;
        }
        let (a, b) = (&view.parts[r.a], &view.parts[r.b]);
        let (small, other) = if a.size().max_element() <= b.size().max_element() { (a, b) } else { (b, a) };
        let tol = rel.tolerance.max(1e-4) * 2.0;
        let samples = surface_samples(small, CONTACT_SAMPLES);
        let touching = samples.iter().filter(|s| other.bvh().closest(s.point, tol).is_some()).count();
        if !samples.is_empty() && touching * 100 < samples.len() && small.size().max_element() > 0.05 * (view.hi - view.lo).max_element() {
            let into = (other.center() - small.center()).normalize_or(Vec3::NEG_Y)
                * (0.15 * small.size().min_element().max(small.size().max_element() * 0.05));
            out.push(Critique {
                code: "tangent",
                subject: small.label.clone(),
                parts: vec![a.label.clone(), b.label.clone()],
                message: format!("{} and {} just touch (a point or line of contact reads as an accident)", a.label, b.label),
                fix: format!("sink {} into {} by a clear amount, or open a visible gap", small.label, other.label),
                remedy: Some(Remedy::Move { part: small.label.clone(), by: into }),
            });
        }
    }
    // Floating detail: a small form that touches nothing.
    let size = (view.hi - view.lo).max_element();
    for f in &rel.floating {
        let p = &view.parts[f.part];
        if !solid(f.part) || p.size().max_element() > 0.25 * size {
            continue;
        }
        let Some(near) = &f.nearest else { continue };
        let by = (near.to - near.from) * 1.05 + (near.to - near.from).normalize_or(Vec3::ZERO) * (0.1 * p.size().min_element());
        out.push(Critique {
            code: "floating_detail",
            subject: p.label.clone(),
            parts: vec![p.label.clone(), view.parts[near.part].label.clone()],
            message: format!(
                "{} floats {} mm off {}: detail reads as stuck on when it does not sit on a form",
                p.label,
                (near.gap * 1000.0).round(),
                view.parts[near.part].label
            ),
            fix: format!("seat {} on {} (anchor it there, or move it by the gap)", p.label, view.parts[near.part].label),
            remedy: Some(Remedy::Move { part: p.label.clone(), by }),
        });
    }
    // Stiff: a straight centerline where the genes expect gesture.
    if let Some((want, range)) = &targets.gesture {
        for p in view.parts.iter().filter(|p| !is_strands(view, p)) {
            if !want.is_empty() && !want.iter().any(|w| p.label == *w || p.label.ends_with(&format!("/{w}"))) {
                continue;
            }
            let Some((_, bend, along, off)) = centerline(p) else { continue };
            if bend >= range[0] {
                continue;
            }
            // Push the middle sideways in the part's flat direction.
            let side = if off.length() > 1e-6 {
                off.normalize()
            } else {
                let s = p.size();
                let thin = s.min_position();
                let mut d = Vec3::ZERO;
                d[thin] = 1.0;
                d.cross(along).normalize_or(along.any_orthonormal_vector())
            };
            let len = p.size().max_element();
            out.push(Critique {
                code: "stiff",
                subject: p.label.clone(),
                parts: vec![p.label.clone()],
                message: format!("{} runs dead straight (bend {:.3}, want {} to {}): it reads stiff", p.label, bend, range[0], range[1]),
                fix: format!("give {} a gesture: bend its middle by about {:.0} mm", p.label, (range[0] * 1.5 * len * 1000.0).max(1.0)),
                remedy: Some(Remedy::Bend { part: p.label.clone(), by: side * (range[0] * 1.5 * len), along }),
            });
        }
    }
    // Proportions outside the targets (the archetype's or written).
    for (gene, r) in &targets.proportions {
        let Some(&v) = measured.ratios.get(gene) else { continue };
        if (r[0]..=r[1]).contains(&v) {
            continue;
        }
        let target = v.clamp(r[0], r[1]) * 0.5 + (r[0] + r[1]) * 0.25;
        let whose = targets.archetype.as_deref().map_or(String::new(), |a| format!(" {a}"));
        out.push(Critique {
            code: "proportion",
            subject: gene.clone(),
            parts: gene.split_once("_to_").map(|(a, _)| vec![a.to_owned()]).unwrap_or_default(),
            message: format!("{gene} {v:.2} is outside the{whose} range {} to {}", r[0], r[1]),
            fix: format!("bring {gene} to about {target:.2}"),
            remedy: Some(Remedy::Proportion { gene: gene.clone(), measured: v, target }),
        });
    }
    // One bevel width on forms of very different sizes reads as CG.
    if measured.bevels >= 12 && measured.bevel_variation < 0.1 && measured.form_sizes[1] > 3.0 * measured.form_sizes[0] {
        let wanted = targets.bevel_variation.is_none_or(|r| r[0] > 0.1);
        if wanted {
            out.push(Critique {
                code: "uniform_bevel",
                subject: "bevels".into(),
                parts: Vec::new(),
                message: format!(
                    "every bevel is about {} mm on forms from {} to {} mm",
                    (measured.bevel_width * 1000.0).round(),
                    (measured.form_sizes[0] * 1000.0).round(),
                    (measured.form_sizes[1] * 1000.0).round()
                ),
                fix: "scale bevels with their form: about 1 to 2% of each form's size".into(),
                remedy: None,
            });
        }
    }
    // Detail everywhere: no band of the length is free of small forms.
    let rest_min = targets.rest.map_or(0.125, |r| r[0]);
    if measured.details >= 8 && measured.rest < rest_min {
        // Offer the middle band as the rest area.
        let span = view.hi - view.lo;
        let axis = span.max_position();
        let big = view.parts.iter().map(volume).fold(0.0f32, f32::max);
        let quiet: Vec<String> = view
            .parts
            .iter()
            .filter(|p| !is_strands(view, p) && volume(p) < SECONDARY * big)
            .filter(|p| {
                let t = (p.center()[axis] - view.lo[axis]) / span[axis].max(1e-9);
                (0.4..0.6).contains(&t)
            })
            .map(|p| p.label.clone())
            .collect();
        out.push(Critique {
            code: "uniform_detail",
            subject: "detail".into(),
            parts: quiet.clone(),
            message: format!(
                "{} small forms spread over every part of the length (rest areas {:.0}%, want {:.0}%): the eye has nowhere to rest",
                measured.details,
                measured.rest * 100.0,
                rest_min * 100.0
            ),
            fix: "cluster detail and leave a quiet area: drop or move the small forms in the middle band".into(),
            remedy: (!quiet.is_empty()).then_some(Remedy::Remove { parts: quiet }),
        });
    }
    // A blob or box outline in both views.
    let size = view.hi - view.lo;
    let aspect = size.y / size.x.max(1e-6);
    if measured.solidity.iter().all(|s| *s > 0.93) && (0.75..=1.33).contains(&aspect) {
        out.push(Critique {
            code: "weak_silhouette",
            subject: "outline".into(),
            parts: Vec::new(),
            message: format!(
                "the outline fills {}% of its hull from the front and the side: it reads as a blob",
                (measured.solidity[0] * 100.0).round()
            ),
            fix: "give it a distinct extreme (a flare, a spike, a lean) or cut negative space through it".into(),
            remedy: None,
        });
    }
    out
}

fn range_text(r: Range2) -> String {
    format!("{} to {}", r[0], r[1])
}

/// Verdict text for a measured value against an optional range.
fn verdict(v: f32, r: Option<Range2>) -> (&'static str, String) {
    match r {
        None => ("", String::new()),
        Some(r) if v < r[0] => ("LOW", format!(" (target {})", range_text(r))),
        Some(r) if v > r[1] => ("HIGH", format!(" (target {})", range_text(r))),
        Some(r) => ("ok", format!(" (target {})", range_text(r))),
    }
}

/// The measured genes and the critique of a view against targets.
///
/// Complexity: as [`measure`] and [`critique`].
pub fn evaluate(view: &View, targets: &Targets) -> (Measured, Vec<Critique>) {
    let humanoid = ["heads", "shoulders", "legs"].iter().any(|k| targets.proportions.contains_key(*k));
    let mut measured = measure_with(view, humanoid);
    let mut found = std::mem::take(&mut measured.ratios);
    ratios(view, &targets.proportions, &mut found);
    measured.ratios = found;
    let crit = critique(view, &measured, targets);
    (measured, crit)
}

/// The `dna` aspect: measured genes against targets, then the critique.
///
/// Complexity: as [`measure`] and [`critique`].
pub fn report(view: &View, spec: &DnaSpec) -> Result<Report, (String, String)> {
    let targets = targets(spec)?;
    let (measured, crit) = evaluate(view, &targets);
    let mut lines = Vec::new();
    let mut on = 0usize;
    let mut total = 0usize;
    let mut count = |s: &str| {
        if !s.is_empty() {
            total += 1;
            if s == "ok" {
                on += 1;
            }
        }
    };
    let h = measured.hierarchy;
    let hv = match targets.hierarchy {
        Some(t) => {
            let off = h.iter().zip(&t).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            let s = if off <= 0.12 { "ok" } else { "OFF" };
            count(s);
            format!(" (target {:.2} / {:.2} / {:.2}) {s}", t[0], t[1], t[2])
        }
        None => String::new(),
    };
    lines.push(format!("hierarchy {:.2} / {:.2} / {:.2} over {} forms{hv}", h[0], h[1], h[2], measured.forms));
    let (s, t) = verdict(measured.curved, targets.curved);
    count(s);
    lines.push(format!("lines: {}% curved, {} creases{t} {s}", (measured.curved * 100.0).round(), measured.creases));
    if !measured.tapers.is_empty() {
        let list: Vec<String> = measured.tapers.iter().map(|(p, v)| format!("{p} {v:.2}")).collect();
        let worst = measured.tapers.iter().map(|t| t.1).fold(0.0f32, f32::max);
        let (s, t) = verdict(worst, targets.taper);
        count(s);
        lines.push(format!("taper: {}{t} {s}", list.join(", ")));
    }
    if let Some((want, range)) = &targets.gesture {
        let bends: Vec<&(String, f32)> =
            measured.bends.iter().filter(|(p, _)| want.is_empty() || want.iter().any(|w| p == w || p.ends_with(&format!("/{w}")))).collect();
        if !bends.is_empty() {
            let least = bends.iter().map(|b| b.1).fold(f32::MAX, f32::min);
            let (s, t) = verdict(least, Some(*range));
            count(s);
            let list: Vec<String> = bends.iter().map(|(p, v)| format!("{p} {v:.3}")).collect();
            lines.push(format!("gesture: bend {}{t} {s}", list.join(", ")));
        }
    }
    let (s, t) = verdict(measured.solidity[0].max(measured.solidity[1]), targets.solidity);
    count(s);
    lines.push(format!("silhouette: front solidity {:.2}, side {:.2}{t} {s}", measured.solidity[0], measured.solidity[1]));
    if measured.bevels > 0 {
        let (s, t) = verdict(measured.bevel_variation, targets.bevel_variation);
        count(s);
        lines.push(format!(
            "edges: {} bevel faces, widths about {} mm varying {:.2}{t} {s}",
            measured.bevels,
            (measured.bevel_width * 1000.0).round(),
            measured.bevel_variation
        ));
    }
    if measured.details > 0 {
        let (s, t) = verdict(measured.rest, targets.rest);
        count(s);
        lines.push(format!("detail: {} small forms, {:.0}% of the length at rest{t} {s}", measured.details, measured.rest * 100.0));
    }
    for (name, r) in &targets.proportions {
        match measured.ratios.get(name) {
            Some(&v) => {
                let (s, t) = verdict(v, Some(*r));
                count(s);
                lines.push(format!("{name} {v:.2}{t} {s}"));
            }
            None => lines.push(format!("{name}: not measurable here (parts not found, or legs not apart)")),
        }
    }
    if let Some(v) = &spec.vegetation {
        for (s, l) in crate::vegetation::gene_lines(view, v)? {
            count(s);
            lines.push(l);
        }
    }
    let mut label: Vec<&str> = [spec.archetype.as_deref(), spec.culture.as_deref(), spec.style.as_deref()].into_iter().flatten().collect();
    label.dedup();
    let label = if label.is_empty() { String::new() } else { format!(" ({})", label.join(", ")) };
    let mut text = if total > 0 { format!("dna{label}: {on}/{total} genes on target\n") } else { format!("dna{label}: measured (no targets)\n") };
    for l in &lines {
        text.push_str(&format!("  {}\n", l.trim_end()));
    }
    if crit.is_empty() {
        text.push_str("critique: none\n");
    } else {
        text.push_str("critique:\n");
        for c in &crit {
            text.push_str(&format!("  {}: {}; {}\n", c.code, c.message, c.fix));
        }
    }
    #[derive(Serialize)]
    struct Data<'a> {
        targets: &'a Targets,
        measured: &'a Measured,
        critique: &'a [Critique],
        on_target: usize,
        genes: usize,
    }
    Ok(Report::new("dna", text, &Data { targets: &targets, measured: &measured, critique: &crit, on_target: on, genes: total }))
}

/// The one-line summary every spec answer with a `dna` block carries:
/// `dna 4/5 genes on target; critique: uniform_bevel`.
pub fn line(view: &View, spec: &DnaSpec) -> Option<String> {
    let r = report(view, spec).ok()?;
    let on = r.data.get("on_target")?.as_u64()?;
    let genes = r.data.get("genes")?.as_u64()?;
    let codes: Vec<String> = r.data.get("critique")?.as_array()?.iter().filter_map(|c| c.get("code")?.as_str().map(str::to_owned)).collect();
    let mut t = format!("dna {on}/{genes} genes on target");
    if !codes.is_empty() {
        t.push_str(&format!("; critique: {}", codes.join(", ")));
    }
    Some(t)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use glam::DVec3;
    use p3d_geo::prim::{BoxSpec, Origin, UvSphereSpec, cuboid, uv_sphere};
    use p3d_scene::Document;

    use super::*;

    fn boxed(doc: &mut Document, name: &str, size: [f64; 3], at: [f32; 3], bevel: f64) {
        let m = cuboid(
            &BoxSpec { size: DVec3::from(size), bevel_width: bevel, bevel_segments: u32::from(bevel > 0.0), ..BoxSpec::default() },
            Origin::Center,
        )
        .unwrap();
        let k = doc.create_mesh_object(name, None, m).unwrap();
        doc.object_mut(k).unwrap().transform.translation = Vec3::from(at);
    }

    fn codes(crit: &[Critique]) -> Vec<&'static str> {
        crit.iter().map(|c| c.code).collect()
    }

    #[test]
    fn boxes_are_angular_and_spheres_curved() {
        let mut doc = Document::new();
        boxed(&mut doc, "block", [1.0, 1.0, 1.0], [0.0; 3], 0.0);
        let view = View::new(&doc, None).unwrap();
        let m = measure(&view);
        assert!(m.curved < 0.05 && m.creases == 12, "{m:?}");
        let mut doc = Document::new();
        let s = uv_sphere(&UvSphereSpec { radius: 0.5, segments: 48, rings: 24 }, Origin::Center).unwrap();
        doc.create_mesh_object("ball", None, s).unwrap();
        let view = View::new(&doc, None).unwrap();
        let m = measure(&view);
        assert!(m.curved > 0.95, "{m:?}");
        let r = report(&view, &DnaSpec { culture: Some("dwarven".into()), ..DnaSpec::default() }).unwrap();
        assert!(r.text.contains("lines: 100% curved") && r.text.contains("HIGH"), "{}", r.text);
        assert!(r.text.contains("weak_silhouette"), "{}", r.text);
    }

    #[test]
    fn twins_and_uniform_bevels_are_critiqued() {
        let mut doc = Document::new();
        boxed(&mut doc, "base", [1.2, 0.3, 0.6], [0.0, 0.15, 0.0], 0.01);
        boxed(&mut doc, "post_a", [0.1, 0.8, 0.1], [0.3, 0.7, 0.0], 0.01);
        boxed(&mut doc, "post_b", [0.1, 0.8, 0.1], [0.45, 0.7, 0.0], 0.01);
        let view = View::new(&doc, None).unwrap();
        let m = measure(&view);
        assert!(m.bevels >= 12 && m.bevel_variation < 0.1, "{m:?}");
        let crit = critique(&view, &m, &targets(&DnaSpec::default()).unwrap());
        assert!(codes(&crit).contains(&"twinning") && codes(&crit).contains(&"uniform_bevel"), "{crit:?}");
        assert!(crit.iter().any(|c| c.remedy == Some(Remedy::Scale { part: "post_b".into(), factor: 0.75 })), "{crit:?}");
        // The two posts are elongated with no taper and no bend.
        assert!(m.tapers.iter().any(|(p, t)| p == "post_a" && *t < 0.1), "{m:?}");
        assert!(m.bends.iter().any(|(p, b)| p == "post_a" && *b < 0.01), "{m:?}");
    }

    #[test]
    fn stiff_floating_rhythm_and_proportion_are_critiqued() {
        // An elven blade dead straight, a rivet floating off a plate, five
        // studs at even spacing.
        let mut doc = Document::new();
        boxed(&mut doc, "plate", [1.0, 0.1, 0.4], [0.0, 0.05, 0.0], 0.0);
        boxed(&mut doc, "blade", [0.06, 0.9, 0.01], [-0.6, 0.45, 0.0], 0.0);
        boxed(&mut doc, "rivet", [0.03, 0.03, 0.03], [0.0, 0.2, 0.0], 0.0);
        for i in 0..5 {
            boxed(&mut doc, &format!("stud_{i}"), [0.04, 0.02, 0.04], [-0.4 + 0.2 * i as f32, 0.11, 0.15], 0.0);
        }
        let view = View::new(&doc, None).unwrap();
        let mut t = targets(&DnaSpec { culture: Some("elven".into()), ..DnaSpec::default() }).unwrap();
        t.proportions.insert("blade_to_plate".into(), [1.5, 2.0]);
        let (m, crit) = evaluate(&view, &t);
        let c = codes(&crit);
        assert!(c.contains(&"stiff") && c.contains(&"floating_detail") && c.contains(&"proportion"), "{crit:?}");
        assert!(crit.iter().any(|c| c.code == "twinning" && c.parts.len() == 5), "rhythm: {crit:?}");
        let float = crit.iter().find(|c| c.code == "floating_detail").unwrap();
        match &float.remedy {
            Some(Remedy::Move { part, by }) => assert!(part == "rivet" && by.y < -0.07, "{by}"),
            other => panic!("{other:?}"),
        }
        assert!((m.ratios["blade_to_plate"] - 0.9).abs() < 0.02, "{m:?}");
        let r = report(&view, &DnaSpec { culture: Some("elven".into()), ..DnaSpec::default() }).unwrap();
        assert!(r.text.contains("gesture: bend") && r.text.contains("stiff"), "{}", r.text);
    }

    #[test]
    fn presets_archetypes_and_bad_genes() {
        let t = targets(&DnaSpec { culture: Some("elven".into()), style: Some("stylized_painterly".into()), ..DnaSpec::default() }).unwrap();
        assert_eq!(t.curved, Some([0.55, 1.0]));
        assert!(t.varied_rhythm && t.solidity.is_some() && t.gesture.is_some());
        let t = targets(&DnaSpec { archetype: Some("dwarf".into()), ..DnaSpec::default() }).unwrap();
        assert_eq!(t.proportions.get("heads"), Some(&[3.9, 4.8]));
        let e = targets(&DnaSpec { archetype: Some("gnome".into()), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/archetype");
        let e = targets(&DnaSpec { culture: Some("gnomish".into()), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/culture");
        let e = targets(&DnaSpec { hierarchy: Some([0.5, 0.5, 0.5]), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/hierarchy");
        let e = targets(&DnaSpec { gesture: Some(GestureGene { parts: Vec::new(), bend: [0.2, 0.1] }), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/gesture/bend");
        assert!((hull_area(vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y, Vec2::splat(0.5)]) - 1.0).abs() < 1e-6);
        assert!(ARCHETYPES.iter().all(|a| archetype_genes(a.id).is_some()));
    }
}
