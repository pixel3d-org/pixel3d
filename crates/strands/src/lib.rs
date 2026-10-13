//! Hair, fur and other strand details as cards, clumps and shells.
//!
//! Strands grow from a region of a parent surface as guide curves shaped
//! by a style, gravity, curl and noise, pushed out of the parent so they
//! never penetrate, and are output as alpha cards, stylized clumps or fur
//! shells (ADR 0013). About ten parameters describe a hairstyle.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.

mod braid;
mod grow;
mod output;
mod region;

use glam::{Affine3A, Vec3};
use p3d_geo::bvh::TriBvh;
use p3d_mesh::Mesh;

pub use grow::Guide;
pub use region::{MAX_LOOP_POINTS, region_faces};

/// Most guides one call grows.
pub const MAX_GUIDES: u32 = 4096;
/// Most faces one call outputs.
pub const MAX_FACES: usize = 2_000_000;
/// Most braids one call makes.
pub const MAX_BRAIDS: u32 = 256;

/// Where strands grow on the parent.
#[derive(Debug, Clone, PartialEq)]
pub enum Region {
    /// The whole surface.
    All,
    /// The faces of these face groups.
    Groups(Vec<String>),
    /// The patch a closed loop of surface points cuts off: the side holding
    /// `inside`, or the smaller side.
    Loop { points: Vec<Vec3>, inside: Option<Vec3> },
    /// The faces whose centers lie within an ellipsoid around a point
    /// (semi-axes along x, y and z): an eyebrow, a sideburn, a tuft.
    Around { center: Vec3, radii: Vec3 },
}

/// How a braid ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tie {
    /// The plies just end.
    None,
    /// A wrapped band, then a short tuft.
    #[default]
    Band,
    /// A large metal bead, then a short tuft.
    Bead,
}

/// Braids among the strands: three plies interleaved along a hanging guide,
/// with beads threaded on them and a tie at the end.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Braids {
    /// How many (0 for none).
    pub count: u32,
    /// Length along the braid; 0 uses 1.6 times the strand length.
    pub length: f32,
    /// Width across the plies; 0 uses 1.2 times the clump width.
    pub width: f32,
    pub tie: Tie,
    /// Beads threaded on each braid.
    pub beads: u32,
    /// Bead diameter; 0 uses 1.25 times the braid width.
    pub bead_size: f32,
    /// Points near which braids start (the nearest roots); empty spreads
    /// them over the lowest roots.
    pub at: Vec<Vec3>,
}

/// Which way strands run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    /// Back over the head and down.
    SweptBack,
    /// Away from a part line at this x, then down.
    Parted(f32),
    /// Straight out along the normal.
    Spiky,
    /// Down along the surface.
    Straight,
    /// Along a flow direction; `outward` mirrors its x away from x = 0
    /// (eyebrows, fur combed to both sides).
    Combed { flow: Vec3, outward: bool },
    /// Up, leaning out with the normal (grass).
    Upright,
    /// Along the wind.
    WindBlown(Vec3),
    /// Along the surface toward the anchor, then hanging.
    Ponytail(Vec3),
}

/// What the strands become.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Tapered alpha strips (realistic hair, eyebrows, grass).
    Cards,
    /// Closed tapered tubes with a lens section (stylized hair, beards).
    Clumps,
    /// Offset copies of the region (short fur).
    Shells,
}

/// A preset that sets the defaults of a [`StrandSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hair,
    Fur,
    Eyebrows,
    Beard,
    Grass,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Hair, Kind::Fur, Kind::Eyebrows, Kind::Beard, Kind::Grass];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Hair => "hair",
            Kind::Fur => "fur",
            Kind::Eyebrows => "eyebrows",
            Kind::Beard => "beard",
            Kind::Grass => "grass",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// Everything that shapes strands. Lengths in meters.
#[derive(Debug, Clone, PartialEq)]
pub struct StrandSpec {
    pub region: Region,
    pub style: Style,
    pub output: Output,
    /// Strand length (along the curve).
    pub length: f32,
    /// Random shortening, as a share of the length (0 to 0.95).
    pub length_variation: f32,
    /// Guides (one card or clump each per layer).
    pub clumps: u32,
    /// Steps along each guide (2 to 64).
    pub segments: u32,
    /// Pull toward -Y, growing along the length (0 to 1).
    pub gravity: f32,
    /// Waviness (0 straight to 1 tight curls).
    pub curl: f32,
    /// Curl turns along the length.
    pub curl_turns: f32,
    /// Random direction changes (0 to 1).
    pub noise: f32,
    /// Distance from the surface at the root.
    pub lift: f32,
    /// Extra distance from the surface toward the tip (hair volume).
    pub volume: f32,
    /// Root width of a card or clump; 0 sizes it from the guide spacing.
    pub width: f32,
    /// Tip width as a share of the root width (0 to 1).
    pub tip_width: f32,
    /// Card layers, inner to outer (1 to 3).
    pub layers: u32,
    /// Extra thin flyaway cards, as a share of the guides (0 to 1).
    pub flyaways: f32,
    /// A scalp cap under the cards (the region, slightly lifted).
    pub cap: bool,
    /// Offset copies for shells (1 to 32).
    pub shells: u32,
    /// Points around a clump (3 to 16).
    pub sides: u32,
    /// Clump section thickness over width (0.1 flat ribbon to 1 round).
    pub flatness: f32,
    /// Turns of a clump's section along its length.
    pub twist: f32,
    /// Width at the root as a share of the widest (0.1 to 1).
    pub root_width: f32,
    /// Where along the strand it is widest (0 at the root to 0.9): a leaf
    /// shaped beard clump bulges near 0.35.
    pub bulge: f32,
    /// Clump layers, inner to outer (1 to 3); outer ones fill the gaps.
    pub clump_layers: u32,
    pub braids: Braids,
    pub seed: u64,
}

impl StrandSpec {
    /// The defaults of a preset, growing on the whole surface.
    pub fn preset(kind: Kind) -> StrandSpec {
        let base = StrandSpec {
            region: Region::All,
            style: Style::SweptBack,
            output: Output::Cards,
            length: 0.15,
            length_variation: 0.2,
            clumps: 48,
            segments: 10,
            gravity: 0.6,
            curl: 0.0,
            curl_turns: 2.0,
            noise: 0.15,
            lift: 0.003,
            volume: 0.012,
            width: 0.0,
            tip_width: 0.3,
            layers: 2,
            flyaways: 0.15,
            cap: true,
            shells: 8,
            sides: 6,
            flatness: 0.45,
            twist: 0.0,
            root_width: 1.0,
            bulge: 0.0,
            clump_layers: 1,
            braids: Braids::default(),
            seed: 0,
        };
        match kind {
            Kind::Hair => base,
            Kind::Fur => StrandSpec {
                style: Style::Combed { flow: Vec3::NEG_Y, outward: false },
                output: Output::Shells,
                length: 0.015,
                segments: 4,
                gravity: 0.3,
                lift: 0.0,
                volume: 0.0,
                cap: false,
                flyaways: 0.0,
                ..base
            },
            Kind::Eyebrows => StrandSpec {
                style: Style::Combed { flow: Vec3::new(1.0, 0.25, 0.0), outward: true },
                length: 0.012,
                length_variation: 0.3,
                clumps: 16,
                segments: 4,
                gravity: 0.0,
                noise: 0.1,
                lift: 0.0008,
                volume: 0.001,
                tip_width: 0.2,
                flyaways: 0.0,
                cap: false,
                ..base
            },
            Kind::Beard => StrandSpec {
                style: Style::Straight,
                output: Output::Clumps,
                length: 0.07,
                length_variation: 0.25,
                clumps: 40,
                segments: 8,
                gravity: 0.8,
                curl: 0.15,
                curl_turns: 1.5,
                lift: 0.002,
                volume: 0.006,
                tip_width: 0.15,
                layers: 1,
                flyaways: 0.0,
                cap: false,
                ..base
            },
            Kind::Grass => StrandSpec {
                style: Style::Upright,
                length: 0.25,
                length_variation: 0.4,
                clumps: 120,
                segments: 4,
                gravity: 0.25,
                noise: 0.25,
                lift: 0.0,
                volume: 0.0,
                width: 0.04,
                tip_width: 0.1,
                layers: 1,
                flyaways: 0.0,
                cap: false,
                ..base
            },
        }
    }
}

/// Why strands could not be grown: the parameter, the problem and close
/// valid names.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{param}: {message}")]
pub struct StrandsError {
    pub param: &'static str,
    pub message: String,
    pub suggestions: Vec<String>,
}

impl StrandsError {
    pub fn new(param: &'static str, message: impl Into<String>) -> StrandsError {
        StrandsError { param, message: message.into(), suggestions: Vec::new() }
    }
}

/// What the strands look like as numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub guides: usize,
    pub faces: usize,
    pub triangles: usize,
    pub region_area: f32,
    /// Card or clump root width.
    pub width: f32,
    /// Share of the region within reach of a card or clump (none for
    /// shells, which cover it all).
    pub coverage: Option<f32>,
    /// Center and area of the largest uncovered patch.
    pub bald_spot: Option<(Vec3, f32)>,
    /// Card area over region area (alpha layers drawn on average).
    pub overdraw: Option<f32>,
    /// Guide points inside the parent (0 unless the parent is open or
    /// self-intersecting).
    pub penetrating: usize,
    pub braids: usize,
    pub beads: usize,
}

/// Grown strands: the output mesh (in the parent's space), the guide
/// curves and the report.
#[derive(Debug, Clone)]
pub struct Strands {
    pub mesh: Mesh,
    pub guides: Vec<Guide>,
    pub report: Report,
}

fn check(name: &'static str, v: f32, lo: f32, hi: f32) -> Result<(), StrandsError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(()) } else { Err(StrandsError::new(name, format!("{v} is outside {lo} to {hi}"))) }
}

fn check_count(name: &'static str, v: u32, lo: u32, hi: u32) -> Result<(), StrandsError> {
    if (lo..=hi).contains(&v) { Ok(()) } else { Err(StrandsError::new(name, format!("{v} is outside {lo} to {hi}"))) }
}

fn validate(s: &StrandSpec) -> Result<(), StrandsError> {
    check("length", s.length, 1e-4, 10.0)?;
    check("length_variation", s.length_variation, 0.0, 0.95)?;
    check_count("clumps", s.clumps, 1, MAX_GUIDES)?;
    check_count("segments", s.segments, 2, 64)?;
    check("gravity", s.gravity, 0.0, 1.0)?;
    check("curl", s.curl, 0.0, 1.0)?;
    check("curl_turns", s.curl_turns, 0.0, 20.0)?;
    check("noise", s.noise, 0.0, 1.0)?;
    check("lift", s.lift, 0.0, 1.0)?;
    check("volume", s.volume, 0.0, 1.0)?;
    check("width", s.width, 0.0, 1.0)?;
    check("tip_width", s.tip_width, 0.0, 1.0)?;
    check_count("layers", s.layers, 1, 3)?;
    check("flyaways", s.flyaways, 0.0, 1.0)?;
    check_count("shells", s.shells, 1, 32)?;
    check_count("sides", s.sides, 3, 16)?;
    check("flatness", s.flatness, 0.1, 1.0)?;
    check("twist", s.twist, -20.0, 20.0)?;
    check("root_width", s.root_width, 0.1, 1.0)?;
    check("bulge", s.bulge, 0.0, 0.9)?;
    check_count("clump_layers", s.clump_layers, 1, 3)?;
    check_count("braids", s.braids.count, 0, MAX_BRAIDS)?;
    check("braid_length", s.braids.length, 0.0, 10.0)?;
    check("braid_width", s.braids.width, 0.0, 1.0)?;
    check_count("beads", s.braids.beads, 0, 32)?;
    check("bead_size", s.braids.bead_size, 0.0, 1.0)?;
    if s.braids.at.iter().any(|p| !p.is_finite()) {
        return Err(StrandsError::new("braid_at", "the braid points must be finite"));
    }
    if let Region::Around { center, radii } = s.region
        && (!center.is_finite() || !radii.is_finite() || radii.min_element() <= 0.0)
    {
        return Err(StrandsError::new("region", "an around region needs a finite point and positive radii"));
    }
    let vectors = match s.style {
        Style::Combed { flow, .. } => Some(flow),
        Style::WindBlown(w) => Some(w),
        Style::Ponytail(a) => Some(a),
        _ => None,
    };
    if vectors.is_some_and(|v| !v.is_finite()) || matches!(s.style, Style::Parted(x) if !x.is_finite()) {
        return Err(StrandsError::new("style", "the style's vector must be finite"));
    }
    Ok(())
}

/// Grows strands on `parent` (its own space) and builds the output mesh.
///
/// Complexity: O(region) for the region and shells, plus
/// O(guides x layers x segments x log F) for growing (BVH queries) and
/// O(F + guide points) for the coverage check.
pub fn generate(parent: &Mesh, spec: &StrandSpec) -> Result<Strands, StrandsError> {
    generate_around(parent, None, spec)
}

/// [`generate`], with strands also kept outside `obstacles` (other meshes
/// in the parent's space, such as the chest a beard hangs over), and
/// draping styles resting on them.
///
/// Complexity: as [`generate`], with F counting the obstacles' faces for
/// the queries, plus O(braids x points x sides) for braids.
pub fn generate_around(parent: &Mesh, obstacles: Option<&Mesh>, spec: &StrandSpec) -> Result<Strands, StrandsError> {
    validate(spec)?;
    let faces = region_faces(parent, &spec.region)?;
    let area = p3d_geo::generate::surface_area(parent, &faces);
    if faces.is_empty() || area.is_nan() || area <= 0.0 {
        return Err(StrandsError::new("region", "the region has no area"));
    }
    let spacing = 0.7 * (area / spec.clumps as f32).sqrt();
    let width = if spec.width > 0.0 { spec.width } else { spacing * 1.6 };
    let estimate = match spec.output {
        Output::Cards => spec.clumps as usize * (spec.layers as usize + 1) * spec.segments as usize + if spec.cap { faces.len() } else { 0 },
        Output::Clumps => spec.clumps as usize * (spec.segments as usize + 1) * spec.sides as usize,
        Output::Shells => faces.len() * spec.shells as usize,
    };
    if estimate > MAX_FACES {
        return Err(StrandsError::new(
            "clumps",
            format!("about {estimate} faces would exceed {MAX_FACES}; use fewer clumps, segments, layers or shells"),
        ));
    }
    let surface = TriBvh::from_mesh(parent, Affine3A::IDENTITY);
    let both = obstacles.filter(|o| o.face_count() > 0).map(|o| {
        let mut all = parent.clone();
        all.append(o);
        TriBvh::from_mesh(&all, Affine3A::IDENTITY)
    });
    let bvh = both.as_ref().unwrap_or(&surface);
    let guides = match spec.output {
        Output::Shells => Vec::new(),
        _ => grow::grow(parent, &faces, &surface, bvh, spec, spacing)?,
    };
    let mut mesh = match spec.output {
        Output::Cards => output::cards(parent, &faces, &guides, width, spec)?,
        Output::Clumps => output::clumps(&guides, width, spec)?,
        Output::Shells => output::shells(parent, &faces, spec)?,
    };
    let (lo, hi) = parent.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
    let (braided, braid_guides, beads) = if spec.braids.count > 0 || !spec.braids.at.is_empty() {
        braid::braids(bvh, (lo + hi) * 0.5, &guides, width, spec)?
    } else {
        (Mesh::default(), Vec::new(), 0)
    };
    if braided.face_count() > 0 {
        mesh.append(&braided);
    }
    let (coverage, bald_spot) = match spec.output {
        Output::Shells => (None, None),
        _ => {
            let (c, b) = output::coverage(parent, &faces, &guides, width, spec);
            (Some(c), b)
        }
    };
    let overdraw = (spec.output == Output::Cards).then(|| output::card_area(&guides, width, spec) / area);
    let penetrating = grow::penetrating(bvh, &guides) + grow::penetrating(bvh, &braid_guides);
    let report = Report {
        guides: guides.len(),
        faces: mesh.face_count(),
        triangles: mesh.triangle_count(),
        region_area: area,
        width,
        coverage,
        bald_spot,
        overdraw,
        penetrating,
        braids: braid_guides.len(),
        beads,
    };
    let mut guides = guides;
    guides.extend(braid_guides);
    Ok(Strands { mesh, guides, report })
}
