//! `gen.strands`: hair, fur, eyebrows, beards and grass grown on an object
//! as cards, clumps or shells (ADR 0013).

use glam::Vec3;
use p3d_scene::Document;
use p3d_strands::{Kind, Output, Region, StrandSpec, StrandsError, Style};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::common::{check_new_object, create_mesh_object, mesh_target, thousands};
use crate::deform::{PointParam, group_center};
use crate::error::OpError;
use crate::op::{Category, Op, OpCtx, OpExample, OpFlags, OpMeta, OpOutput};
use crate::params::Length;
use crate::tx::Tx;

/// A strand preset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KindParam {
    #[default]
    Hair,
    Fur,
    Eyebrows,
    Beard,
    Grass,
}

/// The word `all`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AllWord {
    #[default]
    All,
}

/// Where strands grow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RegionParam {
    /// `"all"`: the whole surface.
    All(AllWord),
    /// `{"groups": ["cranium"]}`: these face groups.
    Groups { groups: Vec<String> },
    /// A closed loop of points around the patch, in order: landmark or
    /// group names, or coordinates.
    Loop(Vec<PointParam>),
}

impl Default for RegionParam {
    fn default() -> Self {
        RegionParam::All(AllWord::All)
    }
}

impl RegionParam {
    fn is_all(&self) -> bool {
        matches!(self, RegionParam::All(_))
    }
}

/// Which way strands run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StyleParam {
    SweptBack,
    Parted,
    Spiky,
    Straight,
    Combed,
    Upright,
    WindBlown,
    Ponytail,
}

/// What strands become.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputParam {
    Cards,
    Clumps,
    Shells,
}

/// Parameters of `gen.strands`. Unset values come from the `kind` preset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrandsParams {
    /// Object name; the kind is used when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The object the strands grow on (name, path or id); they become its child.
    pub on: String,
    /// Preset: `hair` (cards, swept back), `fur` (shells), `eyebrows` (cards combed outward), `beard` (clumps hanging) or `grass` (upright cards).
    #[serde(default)]
    pub kind: KindParam,
    /// `"all"`, `{"groups": [...]}` face groups, or a loop of points around the patch such as `["hairline_front", "temple_l", "nape", "temple_r"]` (vertex groups, `lm.<name>` landmarks by name, or coordinates in the object's space).
    #[serde(default, skip_serializing_if = "RegionParam::is_all")]
    pub region: RegionParam,
    /// A point inside a loop's patch when it is not the smaller side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<PointParam>,
    /// Which way strands run: `swept_back`, `parted`, `spiky`, `straight`, `combed`, `upright`, `wind_blown` or `ponytail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StyleParam>,
    /// Where the part line is for `parted` (x; 0 is the center).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<Length>,
    /// Direction for `combed` (the flow) and `wind_blown` (the wind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<[f64; 3]>,
    /// For `combed`: mirror the flow's x away from x = 0 on both sides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outward: Option<bool>,
    /// Where a `ponytail` gathers (a landmark or coordinates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<PointParam>,
    /// What strands become: `cards`, `clumps` or `shells`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputParam>,
    /// Strand length along the curve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<Length>,
    /// Random shortening as a share of the length (0 to 0.95).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_variation: Option<f64>,
    /// Guides: one card or clump each per layer (1 to 4096).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clumps: Option<u32>,
    /// Steps along each strand (2 to 64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<u32>,
    /// Droop toward -Y along the length (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gravity: Option<f64>,
    /// Waviness (0 straight to 1 tight curls).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curl: Option<f64>,
    /// Curl turns along the length (0 to 20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curl_turns: Option<f64>,
    /// Random direction changes (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<f64>,
    /// Distance off the surface at the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lift: Option<Length>,
    /// Extra distance off the surface toward the tips (hair volume).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<Length>,
    /// Root width of a card or clump; sized from the spacing when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<Length>,
    /// Tip width as a share of the root width (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_width: Option<f64>,
    /// Card layers, inner to outer (1 to 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers: Option<u32>,
    /// Thin flyaway cards as a share of the guides (0 to 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flyaways: Option<f64>,
    /// A scalp cap under the cards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<bool>,
    /// Offset copies for `shells` (1 to 32).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shells: Option<u32>,
    /// Points around a clump (3 to 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<u32>,
}

/// A strands error as an op error at its parameter, keeping suggestions.
pub fn strands_error(e: StrandsError) -> OpError {
    let mut o = OpError::invalid(&format!("/{}", e.param), e.message);
    o.suggestions = e.suggestions;
    o
}

/// The kernel spec from parameters: the preset, then every given value.
/// `point` resolves a point parameter (at its JSON path) into the
/// surface's space.
///
/// Complexity: O(loop points) plus what `point` costs.
pub fn strand_spec(p: &StrandsParams, seed: u64, point: &mut dyn FnMut(&PointParam, &str) -> Result<Vec3, OpError>) -> Result<StrandSpec, OpError> {
    let kind = match p.kind {
        KindParam::Hair => Kind::Hair,
        KindParam::Fur => Kind::Fur,
        KindParam::Eyebrows => Kind::Eyebrows,
        KindParam::Beard => Kind::Beard,
        KindParam::Grass => Kind::Grass,
    };
    let mut s = StrandSpec::preset(kind);
    s.seed = seed;
    s.region = match &p.region {
        RegionParam::All(_) => Region::All,
        RegionParam::Groups { groups } => Region::Groups(groups.clone()),
        RegionParam::Loop(points) => {
            let mut pts = Vec::with_capacity(points.len());
            for (i, q) in points.iter().enumerate() {
                pts.push(point(q, &format!("/region/{i}"))?);
            }
            let inside = match &p.inside {
                Some(q) => Some(point(q, "/inside")?),
                None => None,
            };
            Region::Loop { points: pts, inside }
        }
    };
    if p.inside.is_some() && !matches!(p.region, RegionParam::Loop(_)) {
        return Err(OpError::invalid("/inside", "inside only picks the side of a region loop"));
    }
    let flow = p.flow.map(|f| Vec3::new(f[0] as f32, f[1] as f32, f[2] as f32));
    if let Some(style) = p.style {
        s.style = match style {
            StyleParam::SweptBack => Style::SweptBack,
            StyleParam::Parted => Style::Parted(p.part.map_or(0.0, |x| x.0 as f32)),
            StyleParam::Spiky => Style::Spiky,
            StyleParam::Straight => Style::Straight,
            StyleParam::Combed => Style::Combed { flow: flow.unwrap_or(Vec3::NEG_Y), outward: p.outward.unwrap_or(false) },
            StyleParam::Upright => Style::Upright,
            StyleParam::WindBlown => Style::WindBlown(flow.unwrap_or(Vec3::X)),
            StyleParam::Ponytail => {
                let a = p.anchor.as_ref().ok_or_else(|| OpError::invalid("/anchor", "a ponytail needs an anchor to gather at"))?;
                Style::Ponytail(point(a, "/anchor")?)
            }
        };
    } else if let (Style::Combed { flow: f0, outward: o0 }, true) = (s.style, flow.is_some() || p.outward.is_some()) {
        s.style = Style::Combed { flow: flow.unwrap_or(f0), outward: p.outward.unwrap_or(o0) };
    }
    if let Some(o) = p.output {
        s.output = match o {
            OutputParam::Cards => Output::Cards,
            OutputParam::Clumps => Output::Clumps,
            OutputParam::Shells => Output::Shells,
        };
    }
    let f = |v: Option<f64>, d: f32| v.map_or(d, |x| x as f32);
    let l = |v: Option<Length>, d: f32| v.map_or(d, |x| x.0 as f32);
    s.length = l(p.length, s.length);
    s.length_variation = f(p.length_variation, s.length_variation);
    s.clumps = p.clumps.unwrap_or(s.clumps);
    s.segments = p.segments.unwrap_or(s.segments);
    s.gravity = f(p.gravity, s.gravity);
    s.curl = f(p.curl, s.curl);
    s.curl_turns = f(p.curl_turns, s.curl_turns);
    s.noise = f(p.noise, s.noise);
    s.lift = l(p.lift, s.lift);
    s.volume = l(p.volume, s.volume);
    s.width = l(p.width, s.width);
    s.tip_width = f(p.tip_width, s.tip_width);
    s.layers = p.layers.unwrap_or(s.layers);
    s.flyaways = f(p.flyaways, s.flyaways);
    s.cap = p.cap.unwrap_or(s.cap);
    s.shells = p.shells.unwrap_or(s.shells);
    s.sides = p.sides.unwrap_or(s.sides);
    Ok(s)
}

/// The report of grown strands as JSON, kept on the strands object as
/// metadata `strands` (points in its space): kind, output, guides, faces,
/// triangles, region area (m2), width, coverage, bald spot, card area over
/// region area and penetrating points.
pub fn report_json(kind: &str, s: &p3d_strands::Strands, spec: &StrandSpec) -> serde_json::Value {
    let r = &s.report;
    let q = |x: f32| (f64::from(x) * 1e5).round() / 1e5;
    serde_json::json!({
        "kind": kind,
        "output": match spec.output {
            Output::Cards => "cards",
            Output::Clumps => "clumps",
            Output::Shells => "shells",
        },
        "guides": r.guides,
        "faces": r.faces,
        "triangles": r.triangles,
        "region_area": q(r.region_area),
        "width": q(r.width),
        "coverage": r.coverage.map(q),
        "bald_spot": r.bald_spot.map(|(p, a)| serde_json::json!({ "at": [q(p.x), q(p.y), q(p.z)], "area": q(a) })),
        "overdraw": r.overdraw.map(q),
        "penetrating": r.penetrating,
    })
}

/// One line about grown strands.
pub fn strands_summary(s: &p3d_strands::Strands, spec: &StrandSpec) -> String {
    let r = &s.report;
    let what = match spec.output {
        Output::Cards => format!("{} cards", thousands(r.guides)),
        Output::Clumps => format!("{} clumps", thousands(r.guides)),
        Output::Shells => format!("{} shells", spec.shells),
    };
    let mut t = format!("{what}, {} tris, region {} cm2", thousands(r.triangles), (r.region_area * 1e4).round());
    if let Some(c) = r.coverage {
        t.push_str(&format!(", coverage {:.0}%", c * 100.0));
        if let Some((at, area)) = r.bald_spot.filter(|_| c < 0.95) {
            t.push_str(&format!(" (largest bald spot {} cm2 at ({:.0}, {:.0}, {:.0}) mm)", (area * 1e4).round(), at.x * 1e3, at.y * 1e3, at.z * 1e3));
        }
    }
    if r.penetrating > 0 {
        t.push_str(&format!(", {} points inside the surface", r.penetrating));
    }
    t
}

/// `gen.strands`.
pub struct GenStrands;

impl Op for GenStrands {
    const ID: &'static str = "gen.strands";
    type Params = StrandsParams;

    fn meta() -> OpMeta {
        OpMeta {
            label: "Strands",
            category: Category::Gen,
            summary: "Grows hair, fur, eyebrows, beards or grass on an object's surface as alpha cards, stylized clumps or fur shells, from about ten parameters.",
            docs: "Strands are described, not drawn. Pick a `kind` (its preset sets everything), a `region` (the whole surface, face groups, or a loop of \
                   landmarks such as `[\"hairline_front\", \"temple_l\", \"nape\", \"temple_r\"]`), and change what differs: `style` (`swept_back`, `parted`, \
                   `spiky`, `straight`, `combed`, `upright`, `wind_blown`, `ponytail` with an `anchor`), `length`, `clumps`, `gravity`, `curl`, `volume`. Strands \
                   are pushed out of the surface at every step, so they never penetrate it. `cards` come with a scalp cap, inner to outer layers and \
                   flyaways, UVs into a 4-column hair atlas (v from root to tip); `clumps` are closed tapered tubes; `shells` are offset copies for fur. \
                   The summary reports coverage and the largest bald spot; the full report stays on the object as metadata `strands`. The object is a child of `on`, tagged `strands`.\n\n\
                   **Use for** hair, eyebrows, beards, fur, grass patches and other strand details.",
            examples: &[OpExample {
                args: r#"{"on": "crate", "kind": "grass", "region": {"groups": ["top"]}}"#,
                result: "created crate/grass (o:4) grass 0.6366 x 0.2489 x 0.6495 m: 480 faces (960 tris; budget 6,000); 120 cards, 960 tris, region 3600 cm2, coverage 98%",
            }],
            aliases: &["hair", "fur", "grass", "beard", "eyebrows", "hair cards", "particle hair", "hair curves", "groom"],
            complexity: "O(guides x segments x log F) to grow, plus O(F) for the region and coverage",
            flags: OpFlags::UNDOABLE.union(OpFlags::CREATES_OBJECTS).union(OpFlags::CREATES_GROUPS),
            version: 1,
        }
    }

    fn validate(doc: &Document, p: &StrandsParams) -> Result<(), OpError> {
        mesh_target(doc, &p.on).map_err(|e| e.at("/on"))?;
        check_new_object(doc, p.name.as_deref(), Some(&p.on)).map(|_| ())
    }

    fn apply(tx: &mut Tx, p: &StrandsParams, ctx: &OpCtx) -> Result<OpOutput, OpError> {
        let on = mesh_target(tx.doc(), &p.on).map_err(|e| e.at("/on"))?;
        let mesh = tx.mesh(on, "/on")?;
        let mut point = |q: &PointParam, path: &str| -> Result<Vec3, OpError> {
            match q {
                PointParam::At(c) => Ok(Vec3::new(c[0].0 as f32, c[1].0 as f32, c[2].0 as f32)),
                // A landmark name finds its `lm.` group.
                PointParam::Group(g) if mesh.group(g).is_some() => group_center(mesh, g, path),
                PointParam::Group(g) if mesh.group(&format!("lm.{g}")).is_some() => group_center(mesh, &format!("lm.{g}"), path),
                PointParam::Group(g) => {
                    let names = mesh.groups().map(|(n, _)| n.strip_prefix("lm.").unwrap_or(n));
                    let mut e = OpError::invalid(path, format!("no landmark or group `{g}` on {}", p.on));
                    e.suggestions = p3d_scene::name::suggest(g, names, 3);
                    Err(e)
                }
            }
        };
        let spec = strand_spec(p, ctx.seed, &mut point)?;
        let grown = p3d_strands::generate(mesh, &spec).map_err(strands_error)?;
        let summary = strands_summary(&grown, &spec);
        let kind = serde_json::to_value(p.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "strands".into());
        let report = report_json(&kind, &grown, &spec);
        let parent = tx.doc().path(on).unwrap_or_default();
        let key = create_mesh_object(tx, p.name.as_deref(), &kind, Some(&parent), &Default::default(), grown.mesh)?;
        if let Some(o) = tx.doc_mut().object_mut(key) {
            o.tags.push("strands".into());
            o.metadata.insert("strands".into(), report);
        }
        let mut out = crate::prim::created_summary(tx, key, &kind);
        out.summary = format!("{}; {summary}", out.summary);
        Ok(out)
    }
}
