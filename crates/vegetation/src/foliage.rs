//! Foliage from the same skeleton: leaf cards along the twigs (cluster
//! cards plus single leaves at the silhouette, fronds for palms), solid
//! clumps or one crown shell. Cards are placed until the crown silhouette
//! reaches the coverage target, oriented by phyllotaxis, spread, droop and
//! sun bias, varied by atlas variant, flip and turn, tinted per card and by
//! the crown gradient, given normals blended toward the hull's, and emitted
//! inner to outer for alpha sorting.

use std::f32::consts::{PI, TAU};

use glam::{IVec3, Quat, Vec2, Vec3, Vec4};
use p3d_geo::deform::fbm;
use p3d_geo::generate::Rng;
use rustc_hash::FxHashMap;

use crate::build::{Builder, Corner};
use crate::hull::Hull;
use crate::raster::Views;
use crate::skeleton::Skeleton;
use crate::wood::WoodOut;
use crate::{ATLAS_COLUMNS, Foliage, MAX_CARDS, Species, TreeSpec, VegError, groups};

/// What the foliage pass built.
#[derive(Debug, Clone, Default)]
pub(crate) struct FoliageOut {
    pub cards: usize,
    pub leaf_cards: usize,
    pub coverage: f32,
    pub overdraw: f32,
    pub repeats: usize,
    pub warnings: Vec<String>,
    pub bounds: Option<(Vec3, Vec3)>,
}

/// Views used for the coverage target.
const VIEWS: usize = 4;
/// Discrete in-plane turns of a card (degrees).
const TURNS: [f32; 5] = [-24.0, -12.0, 0.0, 12.0, 24.0];

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Builds the foliage into `b`.
///
/// Complexity: O(N log N) to rank N twig nodes, plus O(C x P) for C cards
/// rasterized over P pixels each (C bounded by the budget and
/// [`MAX_CARDS`]).
pub(crate) fn build(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, wood: &WoodOut, b: &mut Builder) -> Result<FoliageOut, VegError> {
    let budget = (spec.budget as usize).saturating_sub(b.triangles());
    let mut out = match spec.foliage {
        Foliage::None => FoliageOut::default(),
        Foliage::Shell => shell(spec, hull, b),
        Foliage::Clumps => clumps(spec, sk, hull, wood, b, budget),
        Foliage::Cards if spec.species == Species::Palm => fronds(spec, sk, hull, b, budget),
        Foliage::Cards => cards(spec, sk, hull, wood, b, budget),
    };
    if spec.foliage != Foliage::None && out.coverage + 0.02 < spec.coverage && spec.species != Species::Palm {
        out.warnings.push(format!(
            "the foliage covers {:.0}% of the crown silhouette, under the {:.0}% target: raise budget or card_size",
            out.coverage * 100.0,
            spec.coverage * 100.0
        ));
    }
    Ok(out)
}

/// A twig node cards can grow on.
#[derive(Debug, Clone, Copy)]
struct Twig {
    node: u32,
    share: f32,
    /// 1 at a tip, smaller further in.
    tip: f32,
    key: f32,
}

/// Twig nodes: fine branches (low pipe weight) in the crown, ranked by a
/// seeded weighted draw that favours the outer shell.
///
/// Complexity: O(N log N).
fn twigs(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, rng: &mut Rng, max_weight: f32) -> Vec<Twig> {
    let mut out = Vec::new();
    for n in 0..sk.pos.len() {
        let chain = sk.chain_of[n] as usize;
        let leader = chain == 0;
        if leader && !matches!(spec.species, Species::Conifer | Species::Birch) {
            continue;
        }
        let p = sk.pos[n];
        if p.y < hull.base - hull.height() * 0.1 {
            continue;
        }
        let w = sk.weight[n];
        let fine = match spec.species {
            Species::Conifer => !leader || p.y > hull.top - hull.height() * 0.12,
            _ => w <= max_weight,
        };
        if !fine {
            continue;
        }
        let share = hull.share(p).min(1.3);
        // Clumps: a low-frequency density field over the crown leaves gaps
        // between masses of leaves (hard gaps where it is lowest).
        let dense = clump_density(spec, hull, p);
        if dense < spec.clumping * 0.4 {
            continue;
        }
        let prio = (0.2 + share * share) * (1.0 - spec.clumping + spec.clumping * dense);
        let u = rng.unit().max(1e-6);
        out.push(Twig { node: n as u32, share, tip: (1.0 / w).clamp(0.0, 1.0), key: u.powf(1.0 / prio) });
    }
    out.sort_by(|a, b| b.key.total_cmp(&a.key).then(a.node.cmp(&b.node)));
    out
}

/// Foliage density of the clump field at `p` (0 in a gap to 1 in a clump):
/// two octaves of seeded noise at about a third of the crown's radius.
///
/// Complexity: O(1).
fn clump_density(spec: &TreeSpec, hull: &Hull, p: Vec3) -> f32 {
    if spec.clumping <= 0.0 {
        return 1.0;
    }
    let scale = (hull.radius * 0.32).max(1e-3);
    let n = fbm(p / scale, spec.seed ^ 0xC1A5_0005, 2);
    smoothstep(-0.3, 0.3, n)
}

/// A card ready to emit.
#[derive(Debug, Clone)]
struct Card {
    pts: [Vec3; 6],
    uvs: [Vec2; 6],
    /// 6 for a folded cluster card, 4 for a single leaf.
    n: usize,
    /// Share along the card (0 at the base, 1 at the tip) per corner.
    along: [f32; 6],
    normal: Vec3,
    tint: Vec3,
    phase: f32,
    share: f32,
}

/// Atlas cell of a variant: clusters in the top half, single leaves (and
/// fronds, sprays) in the bottom half.
fn cell(variant: u32, single: bool) -> (Vec2, Vec2) {
    let w = 1.0 / ATLAS_COLUMNS as f32;
    let u0 = variant as f32 * w;
    if single { (Vec2::new(u0, 0.0), Vec2::new(u0 + w, 0.5)) } else { (Vec2::new(u0, 0.5), Vec2::new(u0 + w, 1.0)) }
}

/// Per-card tint: crown gradient (darker and cooler inside, lighter and
/// warmer on the sunlit outside, darker low down) times hue and brightness
/// jitter.
fn tint(spec: &TreeSpec, hull: &Hull, p: Vec3, rng: &mut Rng) -> Vec3 {
    let share = hull.share(p).min(1.0);
    let t = hull.t(p.y).clamp(0.0, 1.0);
    let g = spec.gradient;
    let depth = (1.0 - share).clamp(0.0, 1.0);
    let k = g * depth;
    let mut c = Vec3::new(1.0 - 0.5 * k, 1.0 - 0.42 * k, 1.0 - 0.28 * k);
    let sun = g * share * smoothstep(0.2, 1.0, t) * 0.14;
    c *= Vec3::new(1.0 + sun, 1.0 + 0.8 * sun, 1.0 - 0.4 * sun);
    c *= 1.0 - g * 0.22 * (1.0 - t);
    let hj = spec.hue_jitter;
    let j = Vec3::new(rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5) * 2.0;
    c *= Vec3::new(1.0 + hj * 0.16 * j.x, 1.0 + hj * 0.07 * j.y, 1.0 + hj * 0.16 * j.z);
    c *= 1.0 + spec.value_jitter * 0.3 * (rng.unit() - 0.5) * 2.0;
    c.clamp(Vec3::splat(0.05), Vec3::splat(1.5))
}

/// Unit vector `angle` from `axis`, turned `azimuth` around it.
fn cone_dir(axis: Vec3, angle: f32, azimuth: f32) -> Vec3 {
    let u = axis.any_orthonormal_vector();
    let w = axis.cross(u);
    (axis * angle.cos() + (u * azimuth.cos() + w * azimuth.sin()) * angle.sin()).normalize_or(axis)
}

/// Emits cards sorted inner to outer.
///
/// Complexity: O(C log C) for C cards.
fn emit_cards(spec: &TreeSpec, hull: &Hull, b: &mut Builder, cards: &mut [Card], group: usize, leaf_group: usize, all: usize) {
    cards.sort_by(|a, c| a.share.total_cmp(&c.share));
    for card in cards.iter() {
        let base = b.pos.len() as u32;
        for p in &card.pts[..card.n] {
            b.vert(*p);
        }
        let corner = |i: usize| {
            let p = card.pts[i];
            let hn = hull.normal(p);
            let n = card.normal.lerp(hn, spec.normal_blend).normalize_or(card.normal);
            let v = card.along[i];
            let shade = 0.8 + 0.2 * v;
            let tint = card.tint * shade;
            Corner {
                v: base + i as u32,
                uv: card.uvs[i],
                wind: Vec2::new(card.phase, 0.16 * (1.0 - v)),
                color: Vec4::new(tint.x, tint.y, tint.z, 1.0),
                normal: n,
            }
        };
        if card.n == 6 {
            // Spine base 0, tip 3; right wing 1, 2; left wing 4, 5.
            b.face(&[corner(0), corner(1), corner(2), corner(3)], &[all, group], groups::LEAF_MATERIAL);
            b.face(&[corner(0), corner(3), corner(4), corner(5)], &[all, group], groups::LEAF_MATERIAL);
        } else {
            b.face(&[corner(0), corner(1), corner(2), corner(3)], &[all, leaf_group], groups::LEAF_MATERIAL);
        }
    }
}

/// Cluster cards along the twigs until the coverage target or the budget.
///
/// Complexity: O(N log N + C x P).
fn cards(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, wood: &WoodOut, b: &mut Builder, budget: usize) -> FoliageOut {
    let _ = wood;
    let mut rng = Rng::new(spec.seed ^ 0xCA4D_0003);
    let list = twigs(spec, sk, hull, &mut rng, 6.0);
    let mut views = Views::around(hull, VIEWS);
    let mut placed: Vec<Card> = Vec::new();
    let mut per_node: FxHashMap<u32, u32> = FxHashMap::default();
    let cell_size = spec.card_size.max(1e-3);
    let mut near: FxHashMap<IVec3, Vec<(Vec3, u32)>> = FxHashMap::default();
    let mut tris = 0usize;
    let mut out = FoliageOut::default();
    let mut repeats = 0usize;
    let fold = 18f32.to_radians().tan();
    let phi0: u64 = spec.seed.wrapping_mul(0x9E37_79B9);
    'passes: for pass in 0..8 {
        if list.is_empty() {
            break;
        }
        for tw in &list {
            if placed.len() >= MAX_CARDS || tris + 4 > budget {
                break 'passes;
            }
            if placed.len().is_multiple_of(8) && views.coverage() >= spec.coverage {
                break 'passes;
            }
            // Later passes only fill what is still open.
            let n = tw.node as usize;
            let parent = sk.parent[n];
            let along = rng.unit() * 0.8;
            let at = if parent == crate::skeleton::NONE { sk.pos[n] } else { sk.pos[n].lerp(sk.pos[parent as usize], along) };
            let twig = sk.dir(tw.node);
            let count = per_node.entry(tw.node).or_insert(0);
            let idx = *count;
            *count += 1;
            let phi = (idx as f32) * spec.phyllotaxis + ((phi0 ^ u64::from(tw.node)) % 6283) as f32 / 1000.0;
            let spread = spec.spread * (0.85 + 0.3 * rng.unit());
            let d0 = cone_dir(twig, spread, phi);
            let droop = spec.droop * (0.35 + 0.65 * tw.tip);
            let mut d = (d0 - Vec3::Y * droop * 1.1).normalize_or(d0);
            let radial = (d0 - twig * twig.dot(d0)).normalize_or(twig.any_orthonormal_vector());
            let tangential = twig.cross(radial);
            let mut nrm = d.cross(tangential).normalize_or(Vec3::Y);
            let out_dir = hull.normal(at);
            if nrm.dot(out_dir + Vec3::Y * 0.5) < 0.0 {
                nrm = -nrm;
            }
            let up = (Vec3::Y - d * d.y).normalize_or(nrm);
            nrm = nrm.lerp(up, spec.sun_bias).normalize_or(nrm);
            nrm = (nrm - d * d.dot(nrm)).normalize_or(nrm);
            // Variant, flip and turn, re-rolled when a neighbour has the same.
            let single = tw.share > 0.82 && rng.unit() < spec.leaf_cards;
            let mut combo = (0u32, false, 0usize);
            let mut clashed = false;
            let key = (at / cell_size).floor().as_ivec3();
            for attempt in 0..6 {
                combo = (
                    (rng.unit() * spec.variants as f32) as u32 % spec.variants.max(1),
                    rng.unit() < 0.5,
                    (rng.unit() * TURNS.len() as f32) as usize % TURNS.len(),
                );
                let code = combo.0 * 16 + u32::from(combo.1) * 8 + combo.2 as u32 + if single { 1000 } else { 0 };
                let clash = (-1..=1).any(|dx| {
                    (-1..=1).any(|dy| {
                        (-1..=1).any(|dz| {
                            near.get(&(key + IVec3::new(dx, dy, dz))).is_some_and(|l| l.iter().any(|(q, c)| *c == code && q.distance(at) < cell_size))
                        })
                    })
                });
                if !clash {
                    break;
                }
                if attempt == 5 {
                    clashed = true;
                }
            }
            let (variant, flip, turn) = combo;
            d = Quat::from_axis_angle(nrm, TURNS[turn].to_radians()) * d;
            let right = d.cross(nrm).normalize_or(Vec3::X);
            let falloff = 1.0 - spec.size_falloff * 0.5 * smoothstep(0.55, 1.1, tw.share);
            let size = spec.card_size * falloff * (0.85 + 0.3 * rng.unit()) * if single { 0.34 } else { 1.0 };
            let (len, half) = (size, size * 0.46);
            let (uv0, uv1) = cell(variant, single);
            let uv = |x: f32, y: f32| {
                let x = if flip { -x } else { x };
                Vec2::new(uv0.x + (x * 0.5 + 0.5) * (uv1.x - uv0.x), uv0.y + y.clamp(0.0, 1.0) * (uv1.y - uv0.y))
            };
            let base = at - d * len * 0.06;
            let local = |x: f32, y: f32| base + right * x * half + d * y * len + nrm * x.abs() * half * fold;
            let card = if single {
                let flat = |x: f32, y: f32| base + right * x * half + d * y * len;
                Card {
                    pts: [flat(-1.0, 0.0), flat(1.0, 0.0), flat(1.0, 1.0), flat(-1.0, 1.0), Vec3::ZERO, Vec3::ZERO],
                    uvs: [uv(-1.0, 0.0), uv(1.0, 0.0), uv(1.0, 1.0), uv(-1.0, 1.0), Vec2::ZERO, Vec2::ZERO],
                    along: [0.0, 0.0, 1.0, 1.0, 0.0, 0.0],
                    n: 4,
                    normal: nrm,
                    tint: Vec3::ONE,
                    phase: 0.0,
                    share: tw.share,
                }
            } else {
                let shape = [(0.0, 0.0), (1.0, 0.22), (0.86, 0.82), (0.0, 1.0), (-0.86, 0.82), (-1.0, 0.22)];
                Card {
                    pts: shape.map(|(x, y)| local(x, y)),
                    uvs: shape.map(|(x, y)| uv(x, y)),
                    along: shape.map(|(_, y)| y),
                    n: 6,
                    normal: nrm,
                    tint: Vec3::ONE,
                    phase: 0.0,
                    share: tw.share,
                }
            };
            let mut card = card;
            let p = &card.pts;
            let tri_list: &[[Vec3; 3]] = if card.n == 6 {
                &[[p[0], p[1], p[2]], [p[0], p[2], p[3]], [p[0], p[3], p[4]], [p[0], p[4], p[5]]]
            } else {
                &[[p[0], p[1], p[2]], [p[0], p[2], p[3]], [p[0], p[0], p[0]], [p[0], p[0], p[0]]]
            };
            // Keep cards that open new silhouette; a few more give the crown depth.
            let need = [0.3, 0.15, 0.06, 0.01][pass.min(3)];
            let depth = pass == 0 && rng.unit() < 0.1;
            if views.gain(tri_list) < need && !depth {
                continue;
            }
            for t in tri_list {
                views.add(*t);
            }
            let code = combo.0 * 16 + u32::from(combo.1) * 8 + combo.2 as u32 + if single { 1000 } else { 0 };
            near.entry(key).or_default().push((at, code));
            if clashed {
                repeats += 1;
            }
            card.tint = tint(spec, hull, at, &mut rng);
            card.phase = (sk.chains[sk.chain_of[n] as usize].phase + rng.unit() * 0.2).fract();
            if card.n == 6 {
                tris += 4;
            } else {
                tris += 2;
                out.leaf_cards += 1;
            }
            placed.push(card);
        }
    }
    out.cards = placed.len() - out.leaf_cards;
    out.coverage = views.coverage();
    out.overdraw = views.overdraw();
    out.repeats = repeats;
    out.bounds = bounds(placed.iter().flat_map(|c| c.pts[..c.n].iter().copied()));
    let all = b.group(groups::FOLIAGE);
    let g = b.group(groups::CLUSTER_CARDS);
    let lg = b.group(groups::LEAF_CARDS);
    emit_cards(spec, hull, b, &mut placed, g, lg, all);
    out
}

/// Bounding box of points.
///
/// Complexity: O(points).
fn bounds(points: impl Iterator<Item = Vec3>) -> Option<(Vec3, Vec3)> {
    points.fold(None, |acc, p| match acc {
        None => Some((p, p)),
        Some((lo, hi)) => Some((lo.min(p), hi.max(p))),
    })
}

/// Palm fronds: arching, drooping strips from the top of the trunk, folded
/// along the rachis, younger ones up and older ones hanging.
///
/// Complexity: O(fronds x segments).
fn fronds(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, b: &mut Builder, budget: usize) -> FoliageOut {
    let mut rng = Rng::new(spec.seed ^ 0xF20D_0004);
    let top = sk.pos[sk.top as usize];
    let count = spec.limbs.max(3) as usize;
    let segs = 10usize;
    let per = segs * 4;
    let count = count.min(budget / per.max(1)).max(1);
    let mut views = Views::around(hull, VIEWS);
    let all = b.group(groups::FOLIAGE);
    let g = b.group(groups::FRONDS);
    let fold = 0.35f32;
    let mut pts_all: Vec<Vec3> = Vec::new();
    for i in 0..count {
        let az = i as f32 * 2.39996 + rng.unit() * 0.3;
        let age = i as f32 / count as f32;
        // Young fronds rise, old ones fall.
        let elev = (50.0 - 85.0 * age + (rng.unit() - 0.5) * 12.0).to_radians();
        let len = hull.radius * (0.95 + 0.25 * rng.unit()) * (1.0 - 0.2 * (1.0 - age));
        let radial = Vec3::new(az.cos(), 0.0, az.sin());
        let dir0 = (radial * elev.cos() + Vec3::Y * elev.sin()).normalize_or(radial);
        let droop = spec.droop * (0.8 + 0.6 * age);
        let width = len * 0.2 * (spec.card_size / 0.9).clamp(0.3, 3.0);
        let variant = (rng.unit() * spec.variants as f32) as u32 % spec.variants.max(1);
        let (uv0, uv1) = cell(variant, true);
        let tnt = tint(spec, hull, top + dir0 * len * 0.6, &mut rng);
        let phase = rng.unit();
        let base = b.pos.len() as u32;
        let mut p = top;
        let mut d = dir0;
        let step = len / segs as f32;
        let mut rows: Vec<(Vec3, Vec3, Vec3)> = Vec::with_capacity(segs + 1);
        for s in 0..=segs {
            let f = s as f32 / segs as f32;
            let side = d.cross(Vec3::Y).normalize_or(Vec3::X);
            let up = side.cross(d).normalize_or(Vec3::Y);
            let w = width * (PI * f.powf(0.7)).sin().max(0.04) * 0.5;
            rows.push((p, side * w, up * w * fold));
            p += d * step;
            d = (d - Vec3::Y * droop * 0.22).normalize_or(d);
        }
        for (s, (c, side, up)) in rows.iter().enumerate() {
            let _ = s;
            b.vert(*c - *side + *up);
            b.vert(*c);
            b.vert(*c + *side + *up);
            pts_all.extend([*c - *side, *c + *side]);
        }
        for s in 0..segs {
            let f0 = s as f32 / segs as f32;
            let f1 = (s + 1) as f32 / segs as f32;
            let r0 = base + (s * 3) as u32;
            let r1 = r0 + 3;
            let uv = |x: f32, f: f32| Vec2::new(uv0.x + x * (uv1.x - uv0.x), uv0.y + f * (uv1.y - uv0.y));
            let nrm = |k: u32| {
                let (c, side, _) = rows[(k - base) as usize / 3];
                let dn = side.cross(rows[((k - base) as usize / 3).min(segs - 1) + 1].0 - c).normalize_or(Vec3::Y);
                let dn = if dn.y < 0.0 { -dn } else { dn };
                dn.lerp(hull.normal(c), spec.normal_blend).normalize_or(dn)
            };
            let corner = |k: u32, x: f32, f: f32| Corner {
                v: k,
                uv: uv(x, f),
                wind: Vec2::new(phase, 0.3 * (1.0 - f)),
                color: Vec4::new(tnt.x, tnt.y, tnt.z, 1.0),
                normal: nrm(k),
            };
            for (l, x0, x1) in [(0u32, 0.0f32, 0.5f32), (1, 0.5, 1.0)] {
                let face = [corner(r0 + l, x0, f0), corner(r0 + l + 1, x1, f0), corner(r1 + l + 1, x1, f1), corner(r1 + l, x0, f1)];
                b.face(&face, &[all, g], groups::LEAF_MATERIAL);
                let q = face.map(|c| b.pos[c.v as usize]);
                views.add([q[0], q[1], q[2]]);
                views.add([q[0], q[2], q[3]]);
            }
        }
    }
    FoliageOut { cards: count, coverage: views.coverage(), overdraw: views.overdraw(), bounds: bounds(pts_all.into_iter()), ..FoliageOut::default() }
}

/// Clumps: noisy blobs (faceted or smooth) on branch ends, until the
/// coverage target or the budget.
///
/// Complexity: O(N log N + clumps x blob faces x P).
fn clumps(spec: &TreeSpec, sk: &Skeleton, hull: &Hull, wood: &WoodOut, b: &mut Builder, budget: usize) -> FoliageOut {
    let _ = wood;
    let mut rng = Rng::new(spec.seed ^ 0xC1A4_0005);
    let level = if spec.facets { 1 } else { 2 };
    let Ok(blob) = p3d_geo::prim::ico_sphere(&p3d_geo::prim::IcoSphereSpec { radius: 1.0, subdivisions: level }, p3d_geo::prim::Origin::Center)
    else {
        return FoliageOut::default();
    };
    let tri_list: Vec<[u32; 3]> = blob
        .faces()
        .map(|f| {
            let v: Vec<u32> = blob.face_verts(f).collect();
            [v[0], v.get(1).copied().unwrap_or(v[0]), v.get(2).copied().unwrap_or(v[0])]
        })
        .collect();
    let per = tri_list.len();
    let mut list = twigs(spec, sk, hull, &mut rng, 40.0);
    // Clumps sit where several twigs meet: thin the list to one per card size.
    let cellsz = spec.card_size * 0.45;
    let mut taken: FxHashMap<IVec3, ()> = FxHashMap::default();
    list.retain(|t| taken.insert((sk.pos[t.node as usize] / cellsz).floor().as_ivec3(), ()).is_none());
    let mut views = Views::around(hull, VIEWS);
    let all = b.group(groups::FOLIAGE);
    let g = b.group(groups::CLUMPS);
    let mut out = FoliageOut::default();
    let mut placed: Vec<(Vec3, f32, f32, Vec3, f32, u64)> = Vec::new();
    for pass in 0..4 {
        for tw in &list {
            if (placed.len() + 1) * per > budget || placed.len() >= MAX_CARDS {
                break;
            }
            if views.coverage() >= spec.coverage {
                break;
            }
            let p = sk.pos[tw.node as usize];
            let falloff = 1.0 - spec.size_falloff * 0.5 * smoothstep(0.55, 1.1, tw.share);
            let r = spec.card_size * 0.6 * falloff * (0.8 + 0.4 * rng.unit());
            // Push the clump outward a little so it rounds the crown.
            let mut c = p + hull.normal(p) * r * 0.25;
            // Clumps rest above the crown base instead of hanging under it.
            c.y = c.y.max(hull.base + r * 0.7);
            let turn = rng.unit() * TAU;
            let seed = spec.seed.wrapping_add(placed.len() as u64 * 131);
            let q = Quat::from_rotation_y(turn);
            let tris: Vec<[Vec3; 3]> = tri_list
                .iter()
                .map(|t| {
                    t.map(|v| {
                        c + q
                            * (blob.positions[v as usize] * Vec3::new(r, r * 0.82, r) * (1.0 + 0.22 * fbm(blob.positions[v as usize] * 1.7, seed, 2)))
                    })
                })
                .collect();
            // Keep clumps that open new silhouette.
            let need = [0.25, 0.12, 0.05, 0.01][pass.min(3)];
            if views.gain(&tris) < need {
                continue;
            }
            for t in tris {
                views.add(t);
            }
            let tnt = tint(spec, hull, c, &mut rng);
            placed.push((c, r, turn, tnt, tw.share, seed));
        }
    }
    placed.sort_by(|a, c| a.4.total_cmp(&c.4));
    let mut pts_all = Vec::new();
    for (c, r, turn, tnt, _, seed) in &placed {
        let q = Quat::from_rotation_y(*turn);
        let base = b.pos.len() as u32;
        let mut map = vec![u32::MAX; blob.vert_slots()];
        for v in blob.verts() {
            let local = blob.positions[v as usize];
            let p = *c + q * (local * Vec3::new(*r, r * 0.82, *r) * (1.0 + 0.22 * fbm(local * 1.7, *seed, 2)));
            map[v as usize] = b.vert(p);
            pts_all.push(p);
        }
        let _ = base;
        let phase = rng.unit();
        for t in &tri_list {
            let vs = t.map(|v| map[v as usize]);
            let ps = vs.map(|v| b.pos[v as usize]);
            let flat = (ps[1] - ps[0]).cross(ps[2] - ps[0]).normalize_or(Vec3::Y);
            let corner = |k: usize| {
                let p = ps[k];
                let smooth = (p - *c).normalize_or(flat);
                let own = if spec.facets { flat } else { smooth };
                let n = own.lerp(hull.normal(p), spec.normal_blend).normalize_or(own);
                let uv = Vec2::new(0.5 + (p.x - c.x) / (4.0 * r), 0.5 + (p.y - c.y) / (4.0 * r));
                Corner { v: vs[k], uv, wind: Vec2::new(phase, 0.25), color: Vec4::new(tnt.x, tnt.y, tnt.z, 1.0), normal: n }
            };
            b.face(&[corner(0), corner(1), corner(2)], &[all, g], groups::LEAF_MATERIAL);
        }
    }
    out.cards = placed.len();
    out.coverage = views.coverage();
    out.overdraw = views.overdraw();
    out.bounds = bounds(pts_all.into_iter());
    out
}

/// One closed crown skin: the hull turned with lumpy noise.
///
/// Complexity: O(rings x sides).
fn shell(spec: &TreeSpec, hull: &Hull, b: &mut Builder) -> FoliageOut {
    let (rings, sides) = if spec.style == crate::Style::LowPoly { (8usize, 12usize) } else { (14, 28) };
    let all = b.group(groups::FOLIAGE);
    let g = b.group(groups::SHELL);
    let mut views = Views::around(hull, VIEWS);
    let base = b.pos.len() as u32;
    let lump = |p: Vec3| 1.0 + 0.16 * fbm(p / spec.card_size.max(0.05), spec.seed, 3);
    let bottom = b.vert(hull.axis_point(hull.base));
    let mut pts = Vec::new();
    for i in 1..rings {
        let t = i as f32 / rings as f32;
        let y = hull.base + t * hull.height();
        let r = hull.radius_at(y).max(hull.radius * 0.02);
        for k in 0..sides {
            // Counter-clockwise seen from above, so faces point out.
            let a = k as f32 / sides as f32 * TAU;
            let dir = Vec3::new(a.cos(), 0.0, -a.sin());
            let p0 = hull.axis_point(y) + dir * r;
            let p = hull.axis_point(y) + dir * r * lump(p0);
            b.vert(p);
            pts.push(p);
        }
    }
    let top = b.vert(hull.axis_point(hull.top));
    let tnt = |p: Vec3| {
        let t = hull.t(p.y).clamp(0.0, 1.0);
        let s = 1.0 - spec.gradient * 0.3 * (1.0 - t);
        Vec4::new(s, s, s * 0.97, 1.0)
    };
    let ring = |i: usize, k: usize| base + 1 + (i * sides + k % sides) as u32;
    let corner = |b: &Builder, v: u32, u: f32, t: f32| {
        let p = b.pos[v as usize];
        Corner { v, uv: Vec2::new(u, t), wind: Vec2::new(0.0, 0.5), color: tnt(p), normal: Vec3::ZERO }
    };
    for k in 0..sides {
        let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
        let f = [
            corner(b, bottom, (u0 + u1) / 2.0, 0.0),
            corner(b, ring(0, k + 1), u1, 1.0 / rings as f32),
            corner(b, ring(0, k), u0, 1.0 / rings as f32),
        ];
        b.face(&f, &[all, g], groups::LEAF_MATERIAL);
    }
    for i in 0..rings - 2 {
        for k in 0..sides {
            let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
            let (t0, t1) = ((i + 1) as f32 / rings as f32, (i + 2) as f32 / rings as f32);
            let f = [
                corner(b, ring(i, k), u0, t0),
                corner(b, ring(i, k + 1), u1, t0),
                corner(b, ring(i + 1, k + 1), u1, t1),
                corner(b, ring(i + 1, k), u0, t1),
            ];
            b.face(&f, &[all, g], groups::LEAF_MATERIAL);
            let q = f.map(|c| b.pos[c.v as usize]);
            views.add([q[0], q[1], q[2]]);
            views.add([q[0], q[2], q[3]]);
        }
    }
    let last = rings - 2;
    for k in 0..sides {
        let (u0, u1) = (k as f32 / sides as f32, (k + 1) as f32 / sides as f32);
        let f = [
            corner(b, ring(last, k), u0, 1.0 - 1.0 / rings as f32),
            corner(b, ring(last, k + 1), u1, 1.0 - 1.0 / rings as f32),
            corner(b, top, (u0 + u1) / 2.0, 1.0),
        ];
        b.face(&f, &[all, g], groups::LEAF_MATERIAL);
    }
    let _ = b.pos.get(top as usize);
    FoliageOut { cards: 1, coverage: views.coverage().max(0.0), overdraw: views.overdraw(), bounds: bounds(pts.into_iter()), ..FoliageOut::default() }
}
