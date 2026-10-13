//! Visual DNA v0 (ADR 0011): the form genes an agent writes, measured back
//! from geometry in the same units, and the first art-direction critique.
//!
//! Genes measured here: form hierarchy (primary, secondary and tertiary
//! volume shares), lines (share of smoothly curved surface, creases), taper
//! of elongated parts, silhouette solidity in the front and side views,
//! bevel width variation, and part ratios. Culture and style presets
//! (`dwarven`, `elven`, `orcish`, `human_medieval`; `realistic`,
//! `stylized_painterly`) give default targets; written genes override them.
//! Critique codes: `tangent` (forms that just touch), `twinning` (equal
//! forms side by side), `uniform_bevel` (one bevel width on forms of very
//! different sizes) and `weak_silhouette` (an outline that reads as a blob).

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
/// Bins along an elongated part's axis for its taper.
const TAPER_BINS: usize = 12;
/// A secondary form is at least this share of the primary's volume.
const SECONDARY: f32 = 0.15;
/// Raster resolution for silhouettes.
const RESOLUTION: usize = 128;
/// Samples of the smaller part when measuring a contact patch.
const CONTACT_SAMPLES: usize = 256;

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
    /// Height ratios between parts: `{"blade_to_haft": [0.3, 0.45]}`.
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

/// Targets after presets: every gene that has a target.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Targets {
    pub hierarchy: Option<[f32; 3]>,
    pub curved: Option<Range2>,
    pub taper: Option<Range2>,
    pub bevel_variation: Option<Range2>,
    pub solidity: Option<Range2>,
    pub varied_rhythm: bool,
    pub proportions: BTreeMap<String, Range2>,
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

/// Resolves presets and written genes into targets.
///
/// Complexity: O(genes).
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
    t.proportions = d.proportions.clone();
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
    /// Filled share of the convex hull, front and side.
    pub solidity: [f32; 2],
    /// Bevel faces found and the variation of their widths (CV) with the
    /// median width in meters.
    pub bevels: usize,
    pub bevel_variation: f32,
    pub bevel_width: f32,
    /// Smallest and largest form size, meters.
    pub form_sizes: [f32; 2],
    pub ratios: BTreeMap<String, f32>,
}

/// One critique finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Critique {
    pub code: &'static str,
    pub parts: Vec<String>,
    pub message: String,
    pub fix: String,
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

/// Measures the genes of a view.
///
/// Complexity: O(faces + edges) per part, O(V) per elongated part for its
/// taper, two rasters for silhouettes.
pub fn measure(view: &View) -> Measured {
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
    // Taper of elongated parts along their longest world axis.
    for p in &view.parts {
        let s = p.size();
        let mut dims = [(s.x, 0usize), (s.y, 1), (s.z, 2)];
        dims.sort_by(|a, b| b.0.total_cmp(&a.0));
        if dims[1].0 <= 0.0 || dims[0].0 / dims[1].0 < ELONGATED {
            continue;
        }
        let axis = dims[0].1;
        let (lo, hi) = (p.lo[axis], p.hi[axis]);
        let mut sums = [(Vec3::ZERO, 0u32); TAPER_BINS];
        let pts: Vec<Vec3> = p.mesh.verts().map(|v| p.world.transform_point3(p.mesh.positions[v as usize])).collect();
        let bin = |x: f32| (((x - lo) / (hi - lo).max(1e-9)) * TAPER_BINS as f32).clamp(0.0, TAPER_BINS as f32 - 1.0) as usize;
        for &q in &pts {
            let b = bin(q[axis]);
            sums[b].0 += q;
            sums[b].1 += 1;
        }
        let mut radius = [0.0f32; TAPER_BINS];
        for &q in &pts {
            let b = bin(q[axis]);
            let c = sums[b].0 / sums[b].1.max(1) as f32;
            let mut d = q - c;
            d[axis] = 0.0;
            radius[b] = radius[b].max(d.length());
        }
        let used: Vec<f32> = radius.iter().zip(&sums).filter(|(_, s)| s.1 > 0).map(|(r, _)| *r).collect();
        let (Some(&first), Some(&last)) = (used.first(), used.last()) else { continue };
        let widest = used.iter().copied().fold(0.0f32, f32::max).max(1e-9);
        m.tapers.push((p.label.clone(), 1.0 - first.min(last) / widest));
    }
    m.solidity = [solidity(view, ViewDir::Front), solidity(view, ViewDir::Right)];
    m
}

/// Height ratios between named parts.
fn ratios(view: &View, wanted: &BTreeMap<String, Range2>) -> BTreeMap<String, f32> {
    let mut out = BTreeMap::new();
    for name in wanted.keys() {
        let Some((a, b)) = name.split_once("_to_") else { continue };
        let h = |n: &str| view.parts.iter().find(|p| p.label == n || p.label.ends_with(&format!("/{n}"))).map(|p| p.size().max_element());
        if let (Some(x), Some(y)) = (h(a), h(b)) {
            out.insert(name.clone(), x / y.max(1e-9));
        }
    }
    out
}

/// Art-direction critique of a view.
///
/// Complexity: O(p^2) part pairs for twinning over at most 64 parts (more
/// are skipped), plus a contact sample per touching pair.
pub fn critique(view: &View, measured: &Measured, targets: &Targets) -> Vec<Critique> {
    let mut out = Vec::new();
    let n = view.parts.len().min(64);
    let center = (view.lo + view.hi) * 0.5;
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
            if same && near && !mirrored && sa.max_element() > 0.0 {
                out.push(Critique {
                    code: "twinning",
                    parts: vec![a.label.clone(), b.label.clone()],
                    message: format!("{} and {} are the same size side by side", a.label, b.label),
                    fix: format!("vary them: scale {} to about 70% or move it off the line", b.label),
                });
            }
        }
    }
    // Tangents: touching forms whose contact is a point or a line.
    let rel = crate::relations::relations(view);
    for r in rel.pairs.iter().filter(|r| r.kind == crate::relations::RelationKind::Touching) {
        let (a, b) = (&view.parts[r.a], &view.parts[r.b]);
        let (small, other) = if a.size().max_element() <= b.size().max_element() { (a, b) } else { (b, a) };
        let tol = rel.tolerance.max(1e-4) * 2.0;
        let samples = surface_samples(small, CONTACT_SAMPLES);
        let touching = samples.iter().filter(|s| other.bvh().closest(s.point, tol).is_some()).count();
        if !samples.is_empty() && touching * 100 < samples.len() && small.size().max_element() > 0.05 * (view.hi - view.lo).max_element() {
            out.push(Critique {
                code: "tangent",
                parts: vec![a.label.clone(), b.label.clone()],
                message: format!("{} and {} just touch (a point or line of contact reads as an accident)", a.label, b.label),
                fix: format!("sink {} into {} by a clear amount, or open a visible gap", small.label, other.label),
            });
        }
    }
    // One bevel width on forms of very different sizes reads as CG.
    if measured.bevels >= 12 && measured.bevel_variation < 0.1 && measured.form_sizes[1] > 3.0 * measured.form_sizes[0] {
        let wanted = targets.bevel_variation.is_none_or(|r| r[0] > 0.1);
        if wanted {
            out.push(Critique {
                code: "uniform_bevel",
                parts: Vec::new(),
                message: format!(
                    "every bevel is about {} mm on forms from {} to {} mm",
                    (measured.bevel_width * 1000.0).round(),
                    (measured.form_sizes[0] * 1000.0).round(),
                    (measured.form_sizes[1] * 1000.0).round()
                ),
                fix: "scale bevels with their form: about 1 to 2% of each form's size".into(),
            });
        }
    }
    // A blob or box outline in both views.
    let size = view.hi - view.lo;
    let aspect = size.y / size.x.max(1e-6);
    if measured.solidity.iter().all(|s| *s > 0.93) && (0.75..=1.33).contains(&aspect) {
        out.push(Critique {
            code: "weak_silhouette",
            parts: Vec::new(),
            message: format!(
                "the outline fills {}% of its hull from the front and the side: it reads as a blob",
                (measured.solidity[0] * 100.0).round()
            ),
            fix: "give it a distinct extreme (a flare, a spike, a lean) or cut negative space through it".into(),
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

/// The `dna` aspect: measured genes against targets, then the critique.
///
/// Complexity: as [`measure`] and [`critique`].
pub fn report(view: &View, spec: &DnaSpec) -> Result<Report, (String, String)> {
    let targets = targets(spec)?;
    let mut measured = measure(view);
    measured.ratios = ratios(view, &targets.proportions);
    let crit = critique(view, &measured, &targets);
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
    for (name, r) in &targets.proportions {
        match measured.ratios.get(name) {
            Some(&v) => {
                let (s, t) = verdict(v, Some(*r));
                count(s);
                lines.push(format!("{name} {v:.2}{t} {s}"));
            }
            None => lines.push(format!("{name}: parts not found")),
        }
    }
    if let Some(v) = &spec.vegetation {
        for (s, l) in crate::vegetation::gene_lines(view, v)? {
            count(s);
            lines.push(l);
        }
    }
    let label = match (spec.culture.as_deref(), spec.style.as_deref()) {
        (Some(c), Some(s)) => format!(" ({c}, {s})"),
        (Some(c), None) | (None, Some(c)) => format!(" ({c})"),
        _ => String::new(),
    };
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
        let codes: Vec<&str> = crit.iter().map(|c| c.code).collect();
        assert!(codes.contains(&"twinning") && codes.contains(&"uniform_bevel"), "{crit:?}");
        // The two posts are elongated with no taper.
        assert!(m.tapers.iter().any(|(p, t)| p == "post_a" && *t < 0.1), "{m:?}");
    }

    #[test]
    fn presets_and_bad_genes() {
        let t = targets(&DnaSpec { culture: Some("elven".into()), style: Some("stylized_painterly".into()), ..DnaSpec::default() }).unwrap();
        assert_eq!(t.curved, Some([0.55, 1.0]));
        assert!(t.varied_rhythm && t.solidity.is_some());
        let e = targets(&DnaSpec { culture: Some("gnomish".into()), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/culture");
        let e = targets(&DnaSpec { hierarchy: Some([0.5, 0.5, 0.5]), ..DnaSpec::default() }).unwrap_err();
        assert_eq!(e.0, "/hierarchy");
        assert!((hull_area(vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y, Vec2::splat(0.5)]) - 1.0).abs() < 1e-6);
    }
}
