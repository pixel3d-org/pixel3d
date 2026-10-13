//! SDF trees: primitives placed in the part's space, combined in order with
//! hard or smooth operators, each with optional modifiers. This is the form
//! agents write (a list of named shapes and how each joins the shape so
//! far), evaluated as signed distance: negative inside, positive outside,
//! in meters.

use glam::{Quat, Vec3};
use p3d_geo::deform::fbm;

/// Deepest nesting of groups.
pub const MAX_DEPTH: usize = 16;
/// Most items in one tree, groups and their members included.
pub const MAX_ITEMS: usize = 1024;

/// A primitive in its own frame (centered at the origin unless noted).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prim {
    Sphere {
        radius: f32,
    },
    /// Semi-axes along x, y, z.
    Ellipsoid {
        radii: Vec3,
    },
    /// Half sizes and the corner rounding radius (inside the half sizes).
    Box {
        half: Vec3,
        round: f32,
    },
    /// A segment from `a` to `b` (in the item's frame) with radii at each end.
    Capsule {
        a: Vec3,
        b: Vec3,
        ra: f32,
        rb: f32,
    },
    /// A capped cone along +Y from y = 0 (bottom radius) to `height`.
    Cone {
        height: f32,
        bottom: f32,
        top: f32,
    },
    /// A ring around Y.
    Torus {
        major: f32,
        minor: f32,
    },
    /// A capped cylinder along Y, centered.
    Cylinder {
        height: f32,
        radius: f32,
    },
}

/// How an item joins the shape so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Combine {
    #[default]
    Union,
    /// Union with a fillet of about this size where the shapes meet.
    SmoothUnion(f32),
    Subtract,
    SmoothSubtract(f32),
    Intersect,
    SmoothIntersect(f32),
}

/// Seeded fractal noise added to the distance (a bumpy surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Noise {
    pub amplitude: f32,
    /// Size of the bumps in meters.
    pub scale: f32,
    pub seed: u64,
    pub octaves: u32,
}

/// Changes to an item's shape before it joins.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Modifiers {
    /// Grows the shape by this much with rounded edges.
    pub round: f32,
    /// Hollows the shape into a skin of this thickness (0: solid).
    pub shell: f32,
    /// Turn around the item's Y axis, radians per meter of height.
    pub twist: f32,
    /// Bend toward +X along Y, radians per meter.
    pub bend: f32,
    /// Mirror across the item frame's planes x = 0, y = 0, z = 0.
    pub mirror: [bool; 3],
    pub noise: Option<Noise>,
}

/// What an item is: a primitive or a group of items combined in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Prim(Prim),
    Group(Vec<Item>),
}

/// One entry of a tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub shape: Shape,
    /// The item's frame in the parent's space: shapes are built around the
    /// origin, turned by `rotation`, then moved to `center`.
    pub center: Vec3,
    pub rotation: Quat,
    pub combine: Combine,
    pub modifiers: Modifiers,
    pub name: Option<String>,
}

impl Item {
    pub fn prim(prim: Prim, center: Vec3) -> Item {
        Item { shape: Shape::Prim(prim), center, rotation: Quat::IDENTITY, combine: Combine::Union, modifiers: Modifiers::default(), name: None }
    }

    pub fn with(mut self, combine: Combine) -> Item {
        self.combine = combine;
        self
    }

    pub fn named(mut self, name: &str) -> Item {
        self.name = Some(name.to_owned());
        self
    }
}

/// Why a tree cannot be evaluated.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SdfError {
    #[error("the tree is empty")]
    Empty,
    #[error("item {path}: {message}")]
    Invalid { path: String, message: String },
    #[error("more than {MAX_ITEMS} items")]
    TooManyItems,
    #[error("groups nest deeper than {MAX_DEPTH}")]
    TooDeep,
    #[error("{0} voxels is more than one mesh takes; use a larger voxel size")]
    TooLarge(u64),
}

/// A checked tree, ready to evaluate. Items are numbered depth first; leaf
/// numbers name the item a surface point belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Sdf {
    items: Vec<Item>,
    /// Leaf item names (or `item_<n>`), by leaf number.
    leaves: Vec<String>,
    /// Upper bound of the gradient length (Lipschitz constant).
    lipschitz: f32,
}

/// The smallest and largest corners of a box.
pub type Aabb = (Vec3, Vec3);

fn prim_lipschitz(p: &Prim) -> f32 {
    match p {
        Prim::Ellipsoid { radii } => radii.max_element() / radii.min_element().max(1e-9),
        // The sphere-swept cone of a tapered capsule is exact, the capped cone nearly so.
        _ => 1.0,
    }
}

fn prim_bounds(p: &Prim) -> Aabb {
    match *p {
        Prim::Sphere { radius } => (Vec3::splat(-radius), Vec3::splat(radius)),
        Prim::Ellipsoid { radii } => (-radii, radii),
        Prim::Box { half, .. } => (-half, half),
        Prim::Capsule { a, b, ra, rb } => {
            let r = ra.max(rb);
            (a.min(b) - Vec3::splat(r), a.max(b) + Vec3::splat(r))
        }
        Prim::Cone { height, bottom, top } => {
            let r = bottom.max(top);
            (Vec3::new(-r, 0.0, -r), Vec3::new(r, height, r))
        }
        Prim::Torus { major, minor } => {
            let r = major + minor;
            (Vec3::new(-r, -minor, -r), Vec3::new(r, minor, r))
        }
        Prim::Cylinder { height, radius } => (Vec3::new(-radius, -height / 2.0, -radius), Vec3::new(radius, height / 2.0, radius)),
    }
}

fn check_prim(p: &Prim) -> Result<(), String> {
    let ok = |x: f32| x.is_finite() && x > 0.0;
    let fine = |x: f32| x.is_finite() && x >= 0.0;
    let good = match *p {
        Prim::Sphere { radius } => ok(radius),
        Prim::Ellipsoid { radii } => ok(radii.x) && ok(radii.y) && ok(radii.z),
        Prim::Box { half, round } => ok(half.x) && ok(half.y) && ok(half.z) && fine(round) && round <= half.min_element(),
        Prim::Capsule { a, b, ra, rb } => a.is_finite() && b.is_finite() && fine(ra) && fine(rb) && (ra > 0.0 || rb > 0.0),
        Prim::Cone { height, bottom, top } => ok(height) && fine(bottom) && fine(top) && (bottom > 0.0 || top > 0.0),
        Prim::Torus { major, minor } => ok(major) && ok(minor),
        Prim::Cylinder { height, radius } => ok(height) && ok(radius),
    };
    if good { Ok(()) } else { Err("sizes must be positive and finite (box rounding at most half the smallest side)".into()) }
}

impl Sdf {
    /// Checks a tree: sizes positive and finite, at most [`MAX_ITEMS`]
    /// items and [`MAX_DEPTH`] levels.
    ///
    /// Complexity: O(items).
    pub fn new(items: Vec<Item>) -> Result<Sdf, SdfError> {
        if items.is_empty() {
            return Err(SdfError::Empty);
        }
        let mut leaves = Vec::new();
        let mut count = 0usize;
        let lipschitz = Self::check(&items, "", 0, &mut leaves, &mut count)?;
        Ok(Sdf { items, leaves, lipschitz: lipschitz * 1.1 })
    }

    fn check(items: &[Item], prefix: &str, depth: usize, leaves: &mut Vec<String>, count: &mut usize) -> Result<f32, SdfError> {
        if depth > MAX_DEPTH {
            return Err(SdfError::TooDeep);
        }
        let mut l: f32 = 1.0;
        for (i, it) in items.iter().enumerate() {
            *count += 1;
            if *count > MAX_ITEMS {
                return Err(SdfError::TooManyItems);
            }
            let path = format!("{prefix}{i}");
            let bad = |message: String| SdfError::Invalid { path: path.clone(), message };
            if !(it.center.is_finite() && it.rotation.is_finite()) {
                return Err(bad("position and rotation must be finite".into()));
            }
            let m = &it.modifiers;
            if !(m.round.is_finite() && m.round >= 0.0 && m.shell.is_finite() && m.shell >= 0.0 && m.twist.is_finite() && m.bend.is_finite()) {
                return Err(bad("round and shell must be 0 or more, twist and bend finite".into()));
            }
            match it.combine {
                Combine::SmoothUnion(k) | Combine::SmoothSubtract(k) | Combine::SmoothIntersect(k) if !(k.is_finite() && k > 0.0) => {
                    return Err(bad("a smooth blend needs a positive size".into()));
                }
                _ => {}
            }
            let (inner, extent) = match &it.shape {
                Shape::Prim(p) => {
                    check_prim(p).map_err(bad)?;
                    leaves.push(it.name.clone().unwrap_or_else(|| format!("item_{}", leaves.len())));
                    let (lo, hi) = prim_bounds(p);
                    (prim_lipschitz(p), lo.abs().max(hi.abs()).length())
                }
                Shape::Group(g) => {
                    if g.is_empty() {
                        return Err(bad("a group needs at least one item".into()));
                    }
                    let inner = Self::check(g, &format!("{path}."), depth + 1, leaves, count)?;
                    let (lo, hi) = Self::bounds_of(g);
                    (inner, lo.abs().max(hi.abs()).length())
                }
            };
            // Twisting and bending stretch space by up to the turn across the extent.
            let warp = (1.0 + (m.twist.abs() * extent).powi(2)).sqrt() * (1.0 + m.bend.abs() * extent);
            let noise = m.noise.map_or(0.0, |n| 2.5 * n.amplitude / n.scale.max(1e-6));
            l = l.max(inner * warp + noise);
        }
        Ok(l)
    }

    /// Leaf item names by leaf number.
    pub fn leaves(&self) -> &[String] {
        &self.leaves
    }

    /// Upper bound of the gradient length: `|f(p)| / lipschitz` is a safe
    /// distance to the surface.
    pub fn lipschitz(&self) -> f32 {
        self.lipschitz
    }

    /// Box holding the surface.
    ///
    /// Complexity: O(items).
    pub fn bounds(&self) -> Aabb {
        Self::bounds_of(&self.items)
    }

    fn bounds_of(items: &[Item]) -> Aabb {
        let mut acc: Option<Aabb> = None;
        for it in items {
            let (lo, hi) = match &it.shape {
                Shape::Prim(p) => prim_bounds(p),
                Shape::Group(g) => Self::bounds_of(g),
            };
            let m = &it.modifiers;
            let grow = m.round + m.noise.map_or(0.0, |n| n.amplitude) + m.shell * 0.5;
            let (mut lo, mut hi) = (lo - Vec3::splat(grow), hi + Vec3::splat(grow));
            // Evaluation undoes the mirror, the twist and the bend in that
            // order, so the shape is bent first, then twisted, then mirrored.
            if m.bend != 0.0 {
                (lo, hi) = bent_bounds(lo, hi, m.bend);
            }
            if m.twist != 0.0 {
                // A twist turns around Y: the shape stays within its cylinder.
                let r = glam::Vec2::new(lo.x.abs().max(hi.x.abs()), lo.z.abs().max(hi.z.abs())).length();
                lo = Vec3::new(-r, lo.y, -r);
                hi = Vec3::new(r, hi.y, r);
            }
            for axis in 0..3 {
                if m.mirror[axis] {
                    let r = lo[axis].abs().max(hi[axis].abs());
                    lo[axis] = -r;
                    hi[axis] = r;
                }
            }
            // Distances in warped frames are estimates: keep a margin.
            if m.bend != 0.0 || m.twist != 0.0 {
                let pad = (hi - lo).max_element() * 0.05 + grow;
                lo -= Vec3::splat(pad);
                hi += Vec3::splat(pad);
            }
            // Turned box corners, then moved.
            let corners = (0..8).map(|c| Vec3::new([lo.x, hi.x][c & 1], [lo.y, hi.y][(c >> 1) & 1], [lo.z, hi.z][(c >> 2) & 1]));
            let pts: Vec<Vec3> = corners.map(|c| it.center + it.rotation * c).collect();
            let b = pts.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, z), &p| (a.min(p), z.max(p)));
            let b = match it.combine {
                Combine::SmoothUnion(k) => (b.0 - Vec3::splat(k * 0.25), b.1 + Vec3::splat(k * 0.25)),
                _ => b,
            };
            acc = Some(match (acc, it.combine) {
                (None, _) => b,
                // Cutting and intersecting never grow the shape so far.
                (Some(a), Combine::Subtract | Combine::SmoothSubtract(_) | Combine::Intersect | Combine::SmoothIntersect(_)) => a,
                (Some(a), _) => (a.0.min(b.0), a.1.max(b.1)),
            });
        }
        acc.unwrap_or((Vec3::ZERO, Vec3::ZERO))
    }

    /// Signed distance at `p`.
    ///
    /// Complexity: O(items).
    #[inline]
    pub fn eval(&self, p: Vec3) -> f32 {
        Self::eval_items(&self.items, p, &mut 0).0
    }

    /// Signed distance at `p` and the leaf item that decides it.
    ///
    /// Complexity: O(items).
    pub fn eval_item(&self, p: Vec3) -> (f32, u32) {
        Self::eval_items(&self.items, p, &mut 0)
    }

    /// Gradient by central differences of step `h`.
    ///
    /// Complexity: O(items).
    pub fn gradient(&self, p: Vec3, h: f32) -> Vec3 {
        let d = |o: Vec3| self.eval(p + o) - self.eval(p - o);
        Vec3::new(d(Vec3::X * h), d(Vec3::Y * h), d(Vec3::Z * h)) / (2.0 * h)
    }

    /// Distance and deciding leaf of a list combined in order. `leaf`
    /// counts leaves depth first.
    fn eval_items(items: &[Item], p: Vec3, leaf: &mut u32) -> (f32, u32) {
        let mut acc: Option<(f32, u32)> = None;
        for it in items {
            let item = Self::eval_item_at(it, p, leaf);
            acc = Some(match acc {
                None => match it.combine {
                    // A list starting with a cut starts from nothing.
                    Combine::Subtract | Combine::SmoothSubtract(_) => (f32::INFINITY, item.1),
                    _ => item,
                },
                Some(a) => combine(it.combine, a, item),
            });
        }
        acc.unwrap_or((f32::INFINITY, u32::MAX))
    }

    fn eval_item_at(it: &Item, p: Vec3, leaf: &mut u32) -> (f32, u32) {
        let m = &it.modifiers;
        // Into the item's frame.
        let mut q = it.rotation.inverse() * (p - it.center);
        for axis in 0..3 {
            if m.mirror[axis] {
                q[axis] = q[axis].abs();
            }
        }
        if m.twist != 0.0 {
            let a = -m.twist * q.y;
            let (s, c) = a.sin_cos();
            q = Vec3::new(c * q.x - s * q.z, q.y, s * q.x + c * q.z);
        }
        if m.bend != 0.0 {
            // Undo a bend of the Y axis toward +X (toward -X for a negative
            // bend, by mirroring): the Y axis becomes an arc of radius 1 / k
            // around (1 / k, 0), and arc length maps back to y.
            let s = m.bend.signum();
            let k = m.bend.abs();
            let r = 1.0 / k;
            let (dx, dy) = (q.x * s - r, q.y);
            let ang = dy.atan2(-dx);
            q = Vec3::new((r - (dx * dx + dy * dy).sqrt()) * s, ang / k, q.z);
        }
        let (mut d, id) = match &it.shape {
            Shape::Prim(prim) => {
                let id = *leaf;
                *leaf += 1;
                (prim_distance(prim, q), id)
            }
            Shape::Group(g) => Self::eval_items(g, q, leaf),
        };
        if m.shell > 0.0 {
            d = d.abs() - m.shell * 0.5;
        }
        d -= m.round;
        if let Some(n) = m.noise {
            d += n.amplitude * fbm(q / n.scale.max(1e-6), n.seed, n.octaves.max(1));
        }
        (d, id)
    }
}

/// Box of a box bent toward +X with curvature `k` (radians per meter): the
/// forward map of the bend, sampled along the arc and at the box corners.
///
/// Complexity: O(1) (a fixed number of samples).
fn bent_bounds(lo: Vec3, hi: Vec3, k: f32) -> (Vec3, Vec3) {
    let s = k.signum();
    let r = 1.0 / k.abs();
    let mut out = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    const STEPS: usize = 16;
    for i in 0..=STEPS {
        let y = lo.y + (hi.y - lo.y) * i as f32 / STEPS as f32;
        let (sin, cos) = (k.abs() * y).sin_cos();
        for x in [lo.x, hi.x] {
            // Mirrored into the positive bend, bent, mirrored back.
            let xs = x * s;
            let px = (r - (r - xs) * cos) * s;
            let py = (r - xs) * sin;
            for z in [lo.z, hi.z] {
                let p = Vec3::new(px, py, z);
                out = (out.0.min(p), out.1.max(p));
            }
        }
    }
    out
}

/// Joins `b` to the shape so far `a`; the deciding leaf is the one whose
/// surface the result follows.
#[inline]
fn combine(op: Combine, a: (f32, u32), b: (f32, u32)) -> (f32, u32) {
    // Polynomial smooth minimum (k is the blend width).
    let smin = |x: f32, y: f32, k: f32| {
        let h = (k - (x - y).abs()).max(0.0) / k;
        x.min(y) - h * h * k * 0.25
    };
    match op {
        Combine::Union => {
            if b.0 < a.0 {
                b
            } else {
                a
            }
        }
        Combine::SmoothUnion(k) => (smin(a.0, b.0, k), if b.0 < a.0 { b.1 } else { a.1 }),
        Combine::Subtract => {
            if -b.0 > a.0 {
                (-b.0, b.1)
            } else {
                a
            }
        }
        Combine::SmoothSubtract(k) => (-smin(-a.0, b.0, k), if -b.0 > a.0 { b.1 } else { a.1 }),
        Combine::Intersect => {
            if b.0 > a.0 {
                b
            } else {
                a
            }
        }
        Combine::SmoothIntersect(k) => (-smin(-a.0, -b.0, k), if b.0 > a.0 { b.1 } else { a.1 }),
    }
}

/// Distance to a primitive in its frame (exact or a close bound).
#[inline]
fn prim_distance(p: &Prim, q: Vec3) -> f32 {
    match *p {
        Prim::Sphere { radius } => q.length() - radius,
        Prim::Ellipsoid { radii } => {
            // Quilez's bound: exact on the axes, close elsewhere.
            let k0 = (q / radii).length();
            let k1 = (q / (radii * radii)).length();
            if k1 > 0.0 { k0 * (k0 - 1.0) / k1 } else { -radii.min_element() }
        }
        Prim::Box { half, round } => {
            let d = q.abs() - (half - Vec3::splat(round));
            d.max(Vec3::ZERO).length() + d.max_element().min(0.0) - round
        }
        Prim::Capsule { a, b, ra, rb } => round_cone(q, a, b, ra, rb),
        Prim::Cone { height, bottom, top } => {
            // Capped cone (Quilez), base at y = 0.
            let p = q - Vec3::new(0.0, height * 0.5, 0.0);
            let h = height * 0.5;
            let qx = (p.x * p.x + p.z * p.z).sqrt();
            let k1 = glam::Vec2::new(top, h);
            let k2 = glam::Vec2::new(top - bottom, 2.0 * h);
            let qq = glam::Vec2::new(qx, p.y);
            let ca = glam::Vec2::new(qq.x - qq.x.min(if qq.y < 0.0 { bottom } else { top }), qq.y.abs() - h);
            let t = ((k1 - qq).dot(k2) / k2.dot(k2)).clamp(0.0, 1.0);
            let cb = qq - k1 + k2 * t;
            let s = if cb.x < 0.0 && ca.y < 0.0 { -1.0 } else { 1.0 };
            s * ca.dot(ca).min(cb.dot(cb)).sqrt()
        }
        Prim::Torus { major, minor } => glam::Vec2::new((q.x * q.x + q.z * q.z).sqrt() - major, q.y).length() - minor,
        Prim::Cylinder { height, radius } => {
            let d = glam::Vec2::new((q.x * q.x + q.z * q.z).sqrt() - radius, q.y.abs() - height * 0.5);
            d.x.max(d.y).min(0.0) + d.max(glam::Vec2::ZERO).length()
        }
    }
}

/// Distance to a sphere-swept segment with radius `ra` at `a` and `rb` at
/// `b` (a tapered capsule; Quilez's round cone).
fn round_cone(p: Vec3, a: Vec3, b: Vec3, ra: f32, rb: f32) -> f32 {
    let ba = b - a;
    let l2 = ba.dot(ba);
    if l2 <= 1e-12 {
        return (p - a).length() - ra.max(rb);
    }
    let rr = ra - rb;
    let a2 = l2 - rr * rr;
    // One end sphere holds the other: the shape is the larger sphere.
    if a2 <= 0.0 {
        return if ra >= rb { (p - a).length() - ra } else { (p - b).length() - rb };
    }
    let il2 = 1.0 / l2;
    let pa = p - a;
    let y = pa.dot(ba);
    let z = y - l2;
    let xv = pa * l2 - ba * y;
    let x2 = xv.dot(xv);
    let y2 = y * y * l2;
    let z2 = z * z * l2;
    let k = rr.signum() * rr * rr * x2;
    if z.signum() * a2 * z2 > k {
        return (x2 + z2).sqrt() * il2 - rb;
    }
    if y.signum() * a2 * y2 < k {
        return (x2 + y2).sqrt() * il2 - ra;
    }
    ((x2 * a2 * il2).sqrt() + y * rr) * il2 - ra
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn primitives_measure_distance() {
        let s = Sdf::new(vec![Item::prim(Prim::Sphere { radius: 1.0 }, Vec3::new(1.0, 0.0, 0.0))]).unwrap();
        assert!(close(s.eval(Vec3::new(3.0, 0.0, 0.0)), 1.0));
        assert!(close(s.eval(Vec3::new(1.0, 0.0, 0.0)), -1.0));
        let b = Sdf::new(vec![Item::prim(Prim::Box { half: Vec3::ONE, round: 0.2 }, Vec3::ZERO)]).unwrap();
        assert!(close(b.eval(Vec3::new(2.0, 0.0, 0.0)), 1.0));
        // The rounded corner: 0.2 from the inner corner box.
        let c = Vec3::splat(0.8) + Vec3::splat(1.0).normalize() * 0.2;
        assert!(close(b.eval(c), 0.0));
        let cap = Sdf::new(vec![Item::prim(Prim::Capsule { a: Vec3::ZERO, b: Vec3::Y, ra: 0.5, rb: 0.5 }, Vec3::ZERO)]).unwrap();
        assert!(close(cap.eval(Vec3::new(1.0, 0.5, 0.0)), 0.5));
        assert!(close(cap.eval(Vec3::new(0.0, 2.0, 0.0)), 0.5));
        let t = Sdf::new(vec![Item::prim(Prim::Torus { major: 1.0, minor: 0.25 }, Vec3::ZERO)]).unwrap();
        assert!(close(t.eval(Vec3::new(1.0, 0.0, 0.0)), -0.25));
        let cy = Sdf::new(vec![Item::prim(Prim::Cylinder { height: 2.0, radius: 0.5 }, Vec3::ZERO)]).unwrap();
        assert!(close(cy.eval(Vec3::new(0.0, 1.5, 0.0)), 0.5));
        assert!(close(cy.eval(Vec3::new(1.0, 0.0, 0.0)), 0.5));
        let co = Sdf::new(vec![Item::prim(Prim::Cone { height: 1.0, bottom: 0.5, top: 0.0 }, Vec3::ZERO)]).unwrap();
        assert!(close(co.eval(Vec3::new(0.0, -0.5, 0.0)), 0.5));
        assert!(co.eval(Vec3::new(0.0, 0.5, 0.0)) < 0.0);
        let e = Sdf::new(vec![Item::prim(Prim::Ellipsoid { radii: Vec3::new(2.0, 1.0, 1.0) }, Vec3::ZERO)]).unwrap();
        assert!(close(e.eval(Vec3::new(3.0, 0.0, 0.0)), 1.0));
        assert!(e.lipschitz() >= 2.0);
    }

    #[test]
    fn combines_in_order_and_names_the_deciding_item() {
        let ball = |x: f32| Item::prim(Prim::Sphere { radius: 1.0 }, Vec3::new(x, 0.0, 0.0));
        let s = Sdf::new(vec![ball(0.0).named("a"), ball(1.5).named("b").with(Combine::SmoothUnion(0.5))]).unwrap();
        assert_eq!(s.leaves(), &["a".to_owned(), "b".to_owned()]);
        // The blend fills the waist between the balls.
        let hard = Sdf::new(vec![ball(0.0), ball(1.5)]).unwrap();
        let waist = Vec3::new(0.75, 0.68, 0.0);
        assert!(s.eval(waist) < hard.eval(waist));
        assert_eq!(s.eval_item(Vec3::new(-0.9, 0.0, 0.0)).1, 0);
        assert_eq!(s.eval_item(Vec3::new(2.4, 0.0, 0.0)).1, 1);
        // A cut is decided by the cutter where it cuts.
        let cut = Sdf::new(vec![ball(0.0), ball(1.0).with(Combine::Subtract)]).unwrap();
        assert!(cut.eval(Vec3::new(0.5, 0.0, 0.0)) > 0.0);
        assert_eq!(cut.eval_item(Vec3::new(0.0, 0.0, 0.0)).1, 1);
        let both = Sdf::new(vec![ball(0.0), ball(1.0).with(Combine::Intersect)]).unwrap();
        assert!(both.eval(Vec3::new(0.5, 0.0, 0.0)) < 0.0 && both.eval(Vec3::new(-0.5, 0.0, 0.0)) > 0.0);
    }

    #[test]
    fn modifiers_and_bounds() {
        let mut it = Item::prim(Prim::Box { half: Vec3::new(0.2, 1.0, 0.2), round: 0.0 }, Vec3::new(1.0, 0.0, 0.0));
        it.modifiers.mirror = [true, false, false];
        it.modifiers.round = 0.05;
        let s = Sdf::new(vec![it]).unwrap();
        // Mirrored across the frame's x = 0, which sits at x = 1 in the part.
        assert!(s.eval(Vec3::new(1.0, 0.0, 0.0)) < 0.0);
        let (lo, hi) = s.bounds();
        assert!(lo.x <= 0.75 && hi.x >= 1.25, "{lo:?} {hi:?}");
        assert!(close(s.eval(Vec3::new(1.3, 0.0, 0.0)), 0.05));
        let mut shell = Item::prim(Prim::Sphere { radius: 1.0 }, Vec3::ZERO);
        shell.modifiers.shell = 0.1;
        let s = Sdf::new(vec![shell]).unwrap();
        assert!(s.eval(Vec3::ZERO) > 0.0 && s.eval(Vec3::X) < 0.0);
        let bad = Sdf::new(vec![Item::prim(Prim::Sphere { radius: -1.0 }, Vec3::ZERO)]);
        assert!(matches!(bad, Err(SdfError::Invalid { .. })));
        assert_eq!(Sdf::new(vec![]), Err(SdfError::Empty));
        let group = Item { shape: Shape::Group(vec![]), ..Item::prim(Prim::Sphere { radius: 1.0 }, Vec3::ZERO) };
        assert!(matches!(Sdf::new(vec![group]), Err(SdfError::Invalid { .. })));
    }

    #[test]
    fn a_capsule_whose_end_sphere_holds_the_other_is_a_sphere() {
        let c = Sdf::new(vec![Item::prim(Prim::Capsule { a: Vec3::ZERO, b: Vec3::new(0.1, 0.0, 0.0), ra: 0.3, rb: 0.05 }, Vec3::ZERO)]).unwrap();
        assert!(close(c.eval(Vec3::new(0.0, 0.5, 0.0)), 0.2));
        assert!(c.eval(Vec3::new(0.2, 0.0, 0.0)).is_finite());
    }

    #[test]
    fn twist_and_bend_keep_the_surface_reachable() {
        let mut it = Item::prim(Prim::Box { half: Vec3::new(0.3, 1.0, 0.1), round: 0.0 }, Vec3::ZERO);
        it.modifiers.twist = 1.5;
        let s = Sdf::new(vec![it.clone()]).unwrap();
        // At y = 1 the slab has turned 1.5 rad: (0.3, 0, 0) turned lies inside.
        let a = 1.5f32;
        assert!(s.eval(Vec3::new(0.25 * a.cos(), 0.9, 0.25 * a.sin())) < 0.0);
        assert!(s.lipschitz() > 1.0);
        it.modifiers.twist = 0.0;
        it.modifiers.bend = 0.5;
        let s = Sdf::new(vec![it]).unwrap();
        assert!(s.eval(Vec3::ZERO) < 0.0);
    }
}
