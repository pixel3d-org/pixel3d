//! Triangle BVH for proximity, ray, containment and overlap queries.
//!
//! Built with obvhs (PLOC). The traversals are written here so that every
//! query is deterministic: ties between equally good triangles go to the
//! lower triangle index, never to whichever the tree visits first.

use std::time::Duration;

use glam::{Affine3A, DVec3, Vec3, Vec3A};
use obvhs::BvhBuildParams;
use obvhs::aabb::Aabb;
use obvhs::bvh2::Bvh2;
use obvhs::bvh2::builder::build_bvh2_from_tris;
use obvhs::triangle::Triangle;
use p3d_mesh::{Mesh, TriOptions};
use smallvec::SmallVec;

/// The closest surface point to a query point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closest {
    pub point: Vec3,
    pub dist: f32,
    /// Triangle index in the BVH.
    pub tri: u32,
    /// Source face of the triangle.
    pub face: u32,
}

/// The first surface hit along a ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    /// Distance along the (normalized) ray direction.
    pub t: f32,
    pub point: Vec3,
    pub tri: u32,
    pub face: u32,
}

/// Triangles of one surface in a bounding volume hierarchy.
#[derive(Clone)]
pub struct TriBvh {
    bvh: Option<Bvh2>,
    tris: Vec<[Vec3; 3]>,
    verts: Vec<[u32; 3]>,
    faces: Vec<u32>,
    lo: Vec3,
    hi: Vec3,
}

impl std::fmt::Debug for TriBvh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriBvh").field("tris", &self.tris.len()).field("lo", &self.lo).field("hi", &self.hi).finish()
    }
}

#[inline]
fn a(v: Vec3) -> Vec3A {
    Vec3A::from(v)
}

/// Squared distance from a point to a box (0 inside).
#[inline]
fn box_dist2(b: &Aabb, p: Vec3A) -> f32 {
    (b.min - p).max(p - b.max).max(Vec3A::ZERO).length_squared()
}

/// Entry distance of a ray into a box, `None` when it misses within `t_max`.
#[inline]
fn ray_box(b: &Aabb, o: Vec3A, inv: Vec3A, t_max: f32) -> Option<f32> {
    let t0 = (b.min - o) * inv;
    let t1 = (b.max - o) * inv;
    let near = t0.min(t1).max_element().max(0.0);
    let far = t0.max(t1).min_element().min(t_max);
    (near <= far).then_some(near)
}

/// Inverse direction with infinities for zero components.
#[inline]
fn inverse(d: Vec3) -> Vec3A {
    a(Vec3::new(1.0 / d.x, 1.0 / d.y, 1.0 / d.z))
}

/// Closest point on triangle `abc` to `p` (Ericson, Real-Time Collision
/// Detection, 5.1.5).
pub fn closest_on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Ray parameter of the hit of `o + t d` with triangle `abc` (both sides),
/// Moller-Trumbore in double precision. `None` when parallel or missed.
pub fn ray_triangle(o: Vec3, d: Vec3, tri: [Vec3; 3]) -> Option<f32> {
    ray_triangle_margin(o, d, tri).map(|(t, _)| t)
}

/// Like [`ray_triangle`], plus the smallest barycentric coordinate of the
/// hit: near 0 when the ray grazes an edge or a corner.
fn ray_triangle_margin(o: Vec3, d: Vec3, [a, b, c]: [Vec3; 3]) -> Option<(f32, f64)> {
    let (o, d) = (o.as_dvec3(), d.as_dvec3());
    let (a, b, c) = (a.as_dvec3(), b.as_dvec3(), c.as_dvec3());
    let e1 = b - a;
    let e2 = c - a;
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    // Parallel within a tolerance relative to the triangle's size.
    if det.abs() <= 1e-12 * e1.length() * e2.length() * d.length() {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - a;
    let u = tv.dot(pv) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(((e2.dot(qv) * inv) as f32, u.min(v).min(1.0 - u - v)))
}

/// Does the open segment `p q` cross the interior of triangle `abc`?
/// Touching at an end point, an edge or a corner does not count, nor does a
/// segment lying in the triangle's plane.
fn segment_crosses(p: DVec3, q: DVec3, [a, b, c]: [DVec3; 3]) -> bool {
    const EPS: f64 = 1e-9;
    let d = q - p;
    let e1 = b - a;
    let e2 = c - a;
    let pv = d.cross(e2);
    let det = e1.dot(pv);
    if det.abs() <= 1e-12 * e1.length() * e2.length() * d.length() {
        return false;
    }
    let inv = 1.0 / det;
    let tv = p - a;
    let u = tv.dot(pv) * inv;
    let qv = tv.cross(e1);
    let v = d.dot(qv) * inv;
    let t = e2.dot(qv) * inv;
    t > EPS && t < 1.0 - EPS && u > EPS && v > EPS && u + v < 1.0 - EPS
}

/// Do two triangles cross each other? An edge of one must pass through the
/// interior of the other. Coplanar overlaps and contacts along edges or at
/// corners are not crossings.
pub fn triangles_cross(t: [Vec3; 3], u: [Vec3; 3]) -> bool {
    let t = t.map(Vec3::as_dvec3);
    let u = u.map(Vec3::as_dvec3);
    (0..3).any(|i| segment_crosses(t[i], t[(i + 1) % 3], u)) || (0..3).any(|i| segment_crosses(u[i], u[(i + 1) % 3], t))
}

/// Closest points of segments `p1 q1` and `p2 q2` (Ericson 5.1.9).
fn segment_segment(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    const EPS: f32 = 1e-20;
    let (s, t) = if a <= EPS && e <= EPS {
        (0.0, 0.0)
    } else if a <= EPS {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > EPS { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p1 + d1 * s, p2 + d2 * t)
}

/// Distance between two triangles with the closest points on each (zero
/// with a point on the crossing when they cross).
pub fn triangle_distance(t: [Vec3; 3], u: [Vec3; 3]) -> (f32, Vec3, Vec3) {
    if triangles_cross(t, u) {
        // Any point of the crossing will do: where an edge pierces the other.
        for (x, y) in [(t, u), (u, t)] {
            for i in 0..3 {
                let (p, q) = (x[i], x[(i + 1) % 3]);
                if let Some(s) = ray_triangle(p, q - p, y)
                    && (0.0..=1.0).contains(&s)
                {
                    let hit = p + (q - p) * s;
                    return (0.0, hit, hit);
                }
            }
        }
    }
    let mut best = (f32::INFINITY, Vec3::ZERO, Vec3::ZERO);
    let mut take = |a: Vec3, b: Vec3| {
        let d = a.distance(b);
        if d < best.0 {
            best = (d, a, b);
        }
    };
    for p in t {
        take(p, closest_on_triangle(p, u));
    }
    for q in u {
        take(closest_on_triangle(q, t), q);
    }
    for i in 0..3 {
        for j in 0..3 {
            let (a, b) = segment_segment(t[i], t[(i + 1) % 3], u[j], u[(j + 1) % 3]);
            take(a, b);
        }
    }
    best
}

/// Squared distance between two boxes (0 when they overlap).
#[inline]
fn box_box_dist2(a: &Aabb, b: &Aabb) -> f32 {
    (a.min - b.max).max(b.min - a.max).max(Vec3A::ZERO).length_squared()
}

/// The closest pair of points between two surfaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nearest {
    pub dist: f32,
    /// Point on the first surface.
    pub a: Vec3,
    /// Point on the second surface.
    pub b: Vec3,
    pub tri_a: u32,
    pub tri_b: u32,
    /// The work budget ran out: `dist` is an upper bound, not the minimum.
    pub approximate: bool,
}

/// Leaf pairs [`TriBvh::nearest`] examines before it stops and reports an
/// upper bound. Only surfaces with large equidistant regions (a small part at
/// the exact center of a hollow sphere) come close.
pub const NEAREST_LEAF_PAIR_BUDGET: u32 = 200_000;

/// Ray directions for containment votes: generic, so that a ray rarely runs
/// exactly along an edge of axis-aligned or symmetric geometry.
/// Not normalized: only the sign of the ray parameter matters.
const VOTE_DIRS: [Vec3; 5] = [
    Vec3::new(0.381_034, 0.852_617, 0.357_628),
    Vec3::new(-0.771_249, 0.245_318, 0.587_441),
    Vec3::new(0.193_127, -0.662_903, -0.723_419),
    Vec3::new(-0.414_213, -0.302_775, 0.858_369),
    Vec3::new(0.913_545, 0.131_928, -0.384_017),
];

/// Hits closer than this barycentric margin to an edge make a ray's parity
/// unreliable.
const GRAZE: f64 = 1e-7;

impl TriBvh {
    /// Triangulates a mesh (as for the GPU) in the space given by `to_space`.
    ///
    /// Complexity: O(n log n) in triangles.
    pub fn from_mesh(mesh: &Mesh, to_space: Affine3A) -> TriBvh {
        let tb = mesh.to_tri_buffers(&TriOptions { normals: false, uvs: false, colors: false });
        let n = tb.triangle_count();
        let mut tris = Vec::with_capacity(n);
        let mut verts = Vec::with_capacity(n);
        let (corners, _) = tb.indices.as_chunks::<3>();
        for t in corners {
            let i = t.map(|i| i as usize);
            tris.push(i.map(|i| to_space.transform_point3(tb.positions[i])));
            verts.push(i.map(|i| tb.vert_source[i]));
        }
        TriBvh::from_triangles(tris, verts, tb.tri_face)
    }

    /// From triangles with their source vertex ids (used to skip neighbours
    /// in self-intersection tests) and source faces.
    ///
    /// Complexity: O(n log n) in triangles.
    pub fn from_triangles(tris: Vec<[Vec3; 3]>, verts: Vec<[u32; 3]>, faces: Vec<u32>) -> TriBvh {
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for t in &tris {
            for p in t {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        let bvh = if tris.is_empty() {
            None
        } else {
            let prims: Vec<Triangle> = tris.iter().map(|t| Triangle { v0: a(t[0]), v1: a(t[1]), v2: a(t[2]) }).collect();
            Some(build_bvh2_from_tris(&prims, BvhBuildParams::fast_build(), &mut Duration::default()))
        };
        TriBvh { bvh, tris, verts, faces, lo, hi }
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Bounds of all triangles; `None` when empty.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        (!self.tris.is_empty()).then_some((self.lo, self.hi))
    }

    pub fn triangle(&self, tri: u32) -> [Vec3; 3] {
        self.tris[tri as usize]
    }

    /// Source face of a triangle.
    pub fn face(&self, tri: u32) -> u32 {
        self.faces[tri as usize]
    }

    /// Unit normal of a triangle (zero when degenerate).
    pub fn normal(&self, tri: u32) -> Vec3 {
        let [p, q, r] = self.tris[tri as usize];
        (q - p).cross(r - p).normalize_or_zero()
    }

    /// The closest surface point within `max_dist`, ties to the lower
    /// triangle index. Depth-first, nearer child first, pruned by the best
    /// distance so far; the stack lives on the call stack (no allocation).
    ///
    /// Complexity: output-sensitive, O(log n + k) for the k leaves whose boxes
    /// come within the found distance: small for points near the surface,
    /// O(n) for a point equidistant from everything (a sphere's center).
    pub fn closest(&self, p: Vec3, max_dist: f32) -> Option<Closest> {
        let bvh = self.bvh.as_ref()?;
        let pa = a(p);
        let mut best_d2 = max_dist * max_dist;
        let mut best: Option<(u32, Vec3)> = None;
        let mut stack: SmallVec<[(f32, u32); 64]> = SmallVec::new();
        stack.push((box_dist2(bvh.nodes[0].aabb(), pa), 0));
        while let Some((d2, ni)) = stack.pop() {
            if d2 > best_d2 {
                continue;
            }
            let node = &bvh.nodes[ni as usize];
            if node.is_leaf() {
                for i in node.first_index..node.first_index + node.prim_count {
                    let tri = bvh.primitive_indices[i as usize];
                    let q = closest_on_triangle(p, self.tris[tri as usize]);
                    let d2 = q.distance_squared(p);
                    let better = match best {
                        None => d2 <= best_d2,
                        Some((bt, _)) => d2 < best_d2 || (d2 == best_d2 && tri < bt),
                    };
                    if better {
                        best_d2 = d2;
                        best = Some((tri, q));
                    }
                }
            } else {
                let (l, r) = (node.first_index, node.first_index + 1);
                let (dl, dr) = (box_dist2(bvh.nodes[l as usize].aabb(), pa), box_dist2(bvh.nodes[r as usize].aabb(), pa));
                // Push the farther child first so the nearer one is searched first.
                let (near, far) = if dl <= dr { ((dl, l), (dr, r)) } else { ((dr, r), (dl, l)) };
                if far.0 <= best_d2 {
                    stack.push(far);
                }
                if near.0 <= best_d2 {
                    stack.push(near);
                }
            }
        }
        best.map(|(tri, point)| Closest { point, dist: best_d2.sqrt(), tri, face: self.faces[tri as usize] })
    }

    /// Visits the leaves whose boxes pass `keep`; stops when `f` returns false.
    fn visit(&self, mut keep: impl FnMut(&Aabb) -> bool, mut f: impl FnMut(u32) -> bool) {
        let Some(bvh) = &self.bvh else { return };
        let mut stack: SmallVec<[u32; 64]> = SmallVec::new();
        stack.push(0);
        while let Some(ni) = stack.pop() {
            let node = &bvh.nodes[ni as usize];
            if !keep(node.aabb()) {
                continue;
            }
            if node.is_leaf() {
                for i in node.first_index..node.first_index + node.prim_count {
                    if !f(bvh.primitive_indices[i as usize]) {
                        return;
                    }
                }
            } else {
                stack.push(node.first_index + 1);
                stack.push(node.first_index);
            }
        }
    }

    /// The first hit along a ray within `t_max`, ties to the lower triangle
    /// index. `dir` need not be normalized; `t` is in units of its length.
    ///
    /// Complexity: O(log n) typical, O(n) worst case.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, t_max: f32) -> Option<RayHit> {
        let (o, inv) = (a(origin), inverse(dir));
        let mut best: Option<(f32, u32)> = None;
        // Shrinks as hits are found; read by the box test, written by the leaf test.
        let limit = std::cell::Cell::new(t_max);
        self.visit(
            |b| ray_box(b, o, inv, limit.get()).is_some(),
            |tri| {
                if let Some(t) = ray_triangle(origin, dir, self.tris[tri as usize])
                    && t >= 0.0
                    && t <= limit.get()
                    && best.is_none_or(|(bt, btri)| t < bt || (t == bt && tri < btri))
                {
                    best = Some((t, tri));
                    limit.set(t);
                }
                true
            },
        );
        best.map(|(t, tri)| RayHit { t, point: origin + dir * t, tri, face: self.faces[tri as usize] })
    }

    /// Number of triangles a ray from `origin` crosses (both sides counted).
    ///
    /// Complexity: O(log n + hits) typical.
    pub fn crossings(&self, origin: Vec3, dir: Vec3) -> u32 {
        self.crossings_checked(origin, dir).0
    }

    /// Crossings plus whether any hit grazed an edge or corner (then a
    /// shared edge may have been counted twice).
    fn crossings_checked(&self, origin: Vec3, dir: Vec3) -> (u32, bool) {
        let (o, inv) = (a(origin), inverse(dir));
        let (mut n, mut grazed) = (0, false);
        self.visit(
            |b| ray_box(b, o, inv, f32::INFINITY).is_some(),
            |tri| {
                if let Some((t, margin)) = ray_triangle_margin(origin, dir, self.tris[tri as usize])
                    && t > 0.0
                {
                    n += 1;
                    grazed |= margin < GRAZE;
                }
                true
            },
        );
        (n, grazed)
    }

    /// Is `p` inside the closed surface? Ray parity votes along generic
    /// directions; rays that graze an edge or corner do not vote, so shared
    /// edges cannot flip the answer. Meaningful only for closed surfaces.
    ///
    /// Complexity: O(log n + hits) typical.
    pub fn contains(&self, p: Vec3) -> bool {
        if self.tris.is_empty() || !(p.cmpge(self.lo).all() && p.cmple(self.hi).all()) {
            return false;
        }
        let (mut inside, mut outside, mut fallback) = (0usize, 0usize, 0usize);
        for d in VOTE_DIRS {
            let (n, grazed) = self.crossings_checked(p, d);
            if grazed {
                fallback += (n % 2) as usize;
                continue;
            }
            if n % 2 == 1 {
                inside += 1
            } else {
                outside += 1
            }
            if inside == 2 || outside == 2 {
                return inside == 2;
            }
        }
        // Every vote was unreliable or split: fall back to all rays' majority.
        2 * (inside + fallback) > VOTE_DIRS.len()
    }

    /// Triangles whose boxes overlap the box `lo..hi`; stops when `f`
    /// returns false.
    ///
    /// Complexity: O(log n + hits) typical.
    pub fn query_box(&self, lo: Vec3, hi: Vec3, f: impl FnMut(u32) -> bool) {
        let q = Aabb::new(a(lo), a(hi));
        self.visit(|b| b.intersect_aabb(&q), f);
    }

    /// Pairs of triangles that cross each other (see [`triangles_cross`]), one
    /// from `self` and one from `other`, in a deterministic order. Stops when
    /// `f` returns false.
    ///
    /// Complexity: O((n + m) log(n + m) + overlapping leaf pairs) typical.
    pub fn crossing_pairs(&self, other: &TriBvh, mut f: impl FnMut(u32, u32) -> bool) {
        let (Some(x), Some(y)) = (&self.bvh, &other.bvh) else { return };
        let mut stack = vec![(0u32, 0u32)];
        while let Some((i, j)) = stack.pop() {
            let (ni, nj) = (&x.nodes[i as usize], &y.nodes[j as usize]);
            if !ni.aabb().intersect_aabb(nj.aabb()) {
                continue;
            }
            match (ni.is_leaf(), nj.is_leaf()) {
                (true, true) => {
                    for pi in ni.first_index..ni.first_index + ni.prim_count {
                        let t = x.primitive_indices[pi as usize];
                        for pj in nj.first_index..nj.first_index + nj.prim_count {
                            let u = y.primitive_indices[pj as usize];
                            if triangles_cross(self.tris[t as usize], other.tris[u as usize]) && !f(t, u) {
                                return;
                            }
                        }
                    }
                }
                // Descend the larger inner node.
                (false, true) => stack.extend([(ni.first_index + 1, j), (ni.first_index, j)]),
                (true, false) => stack.extend([(i, nj.first_index + 1), (i, nj.first_index)]),
                (false, false) => {
                    if ni.aabb().half_area() >= nj.aabb().half_area() {
                        stack.extend([(ni.first_index + 1, j), (ni.first_index, j)]);
                    } else {
                        stack.extend([(i, nj.first_index + 1), (i, nj.first_index)]);
                    }
                }
            }
        }
    }

    /// The closest pair of points between this surface and `other` within
    /// `max_dist` (zero when they cross). Simultaneous depth-first descent of
    /// both trees, nearer node pairs first, pruned by box distance. Ties go
    /// to the lower triangle pair.
    ///
    /// Complexity: output-sensitive, O(log n + log m + k) for the k leaf
    /// pairs whose boxes come within the found distance, capped at
    /// [`NEAREST_LEAF_PAIR_BUDGET`] leaf pairs (then `approximate` is set).
    pub fn nearest(&self, other: &TriBvh, max_dist: f32) -> Option<Nearest> {
        let (Some(x), Some(y)) = (&self.bvh, &other.bvh) else { return None };
        let mut best_d = max_dist;
        let mut best: Option<Nearest> = None;
        let mut budget = NEAREST_LEAF_PAIR_BUDGET;
        let mut approximate = false;
        let mut stack: Vec<(f32, u32, u32)> = vec![(box_box_dist2(x.nodes[0].aabb(), y.nodes[0].aabb()), 0, 0)];
        while let Some((d2, i, j)) = stack.pop() {
            if d2 > best_d * best_d {
                continue;
            }
            let (ni, nj) = (&x.nodes[i as usize], &y.nodes[j as usize]);
            if ni.is_leaf() && nj.is_leaf() {
                if budget == 0 {
                    approximate = true;
                    break;
                }
                budget -= 1;
                for pi in ni.first_index..ni.first_index + ni.prim_count {
                    let t = x.primitive_indices[pi as usize];
                    for pj in nj.first_index..nj.first_index + nj.prim_count {
                        let u = y.primitive_indices[pj as usize];
                        let (d, a, b) = triangle_distance(self.tris[t as usize], other.tris[u as usize]);
                        let better = match &best {
                            None => d <= best_d,
                            Some(n) => d < best_d || (d == best_d && (t, u) < (n.tri_a, n.tri_b)),
                        };
                        if better {
                            best_d = d;
                            best = Some(Nearest { dist: d, a, b, tri_a: t, tri_b: u, approximate: false });
                        }
                    }
                }
                continue;
            }
            // Split the inner node with the larger box (or the only inner one).
            let split_i = !ni.is_leaf() && (nj.is_leaf() || ni.aabb().half_area() >= nj.aabb().half_area());
            let kids: [(u32, u32); 2] =
                if split_i { [(ni.first_index, j), (ni.first_index + 1, j)] } else { [(i, nj.first_index), (i, nj.first_index + 1)] };
            let d = kids.map(|(a, b)| box_box_dist2(x.nodes[a as usize].aabb(), y.nodes[b as usize].aabb()));
            let order = if d[0] <= d[1] { [1, 0] } else { [0, 1] };
            for k in order {
                if d[k] <= best_d * best_d {
                    stack.push((d[k], kids[k].0, kids[k].1));
                }
            }
        }
        best.map(|n| Nearest { approximate, ..n })
    }

    /// Pairs of triangles of this surface that cross each other, skipping
    /// pairs that share a vertex (neighbours). Each pair is reported once,
    /// lower triangle first. Stops when `f` returns false.
    ///
    /// Complexity: O(n log n + overlapping leaf pairs) typical.
    pub fn self_crossings(&self, mut f: impl FnMut(u32, u32) -> bool) {
        let Some(x) = &self.bvh else { return };
        self_pairs(x, |t, u| {
            let (vt, vu) = (self.verts[t as usize], self.verts[u as usize]);
            if vt.iter().any(|v| vu.contains(v)) || !triangles_cross(self.tris[t as usize], self.tris[u as usize]) {
                return true;
            }
            f(t.min(u), t.max(u))
        });
    }
}

/// Calls `f` once for every unordered pair of distinct primitives whose
/// leaves' boxes overlap; stops when `f` returns false.
///
/// Complexity: O(n log n + overlapping leaf pairs) typical.
fn self_pairs(x: &Bvh2, mut f: impl FnMut(u32, u32) -> bool) {
    let mut stack = vec![(0u32, 0u32)];
    while let Some((i, j)) = stack.pop() {
        let (ni, nj) = (&x.nodes[i as usize], &x.nodes[j as usize]);
        if i == j {
            if ni.is_leaf() {
                let r = ni.first_index..ni.first_index + ni.prim_count;
                for p in r.clone() {
                    for q in p + 1..r.end {
                        if !f(x.primitive_indices[p as usize], x.primitive_indices[q as usize]) {
                            return;
                        }
                    }
                }
            } else {
                let (l, r) = (ni.first_index, ni.first_index + 1);
                stack.extend([(l, r), (r, r), (l, l)]);
            }
            continue;
        }
        if !ni.aabb().intersect_aabb(nj.aabb()) {
            continue;
        }
        match (ni.is_leaf(), nj.is_leaf()) {
            (true, true) => {
                for p in ni.first_index..ni.first_index + ni.prim_count {
                    for q in nj.first_index..nj.first_index + nj.prim_count {
                        if !f(x.primitive_indices[p as usize], x.primitive_indices[q as usize]) {
                            return;
                        }
                    }
                }
            }
            (false, true) => stack.extend([(ni.first_index + 1, j), (ni.first_index, j)]),
            (true, false) => stack.extend([(i, nj.first_index + 1), (i, nj.first_index)]),
            (false, false) => {
                if ni.aabb().half_area() >= nj.aabb().half_area() {
                    stack.extend([(ni.first_index + 1, j), (ni.first_index, j)]);
                } else {
                    stack.extend([(i, nj.first_index + 1), (i, nj.first_index)]);
                }
            }
        }
    }
}

/// Pairs `(i, j)`, `i < j`, of boxes that overlap after growing each by
/// `pad`, sorted; at most `max` pairs (the flag says whether more existed).
/// A box tree finds them, so boxes that line up along an axis cost nothing
/// extra.
///
/// Complexity: O(k log k + overlapping pairs) for k boxes.
pub fn box_pairs(boxes: &[(Vec3, Vec3)], pad: f32, max: usize) -> (Vec<(usize, usize)>, bool) {
    if boxes.len() < 2 {
        return (Vec::new(), false);
    }
    let grown: Vec<Aabb> = boxes.iter().map(|(lo, hi)| Aabb::new(a(*lo - pad), a(*hi + pad))).collect();
    let tree = obvhs::bvh2::builder::build_bvh2(&grown, BvhBuildParams::fast_build(), &mut Duration::default());
    let mut pairs = Vec::new();
    let mut truncated = false;
    self_pairs(&tree, |p, q| {
        if grown[p as usize].intersect_aabb(&grown[q as usize]) {
            if pairs.len() >= max {
                truncated = true;
                return false;
            }
            pairs.push((p.min(q) as usize, p.max(q) as usize));
        }
        true
    });
    pairs.sort_unstable();
    (pairs, truncated)
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::prim::{BoxSpec, Origin, QuadSphereSpec, cuboid, quad_sphere};

    fn unit_box(at: Vec3) -> TriBvh {
        let m = cuboid(&BoxSpec { size: DVec3::ONE, segments: [2; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center).unwrap();
        TriBvh::from_mesh(&m, Affine3A::from_translation(at))
    }

    #[test]
    fn closest_points_on_a_box() {
        let b = unit_box(Vec3::ZERO);
        let c = b.closest(Vec3::new(2.0, 0.1, 0.2), 10.0).unwrap();
        assert!(c.point.distance(Vec3::new(0.5, 0.1, 0.2)) < 1e-6, "{c:?}");
        assert!((c.dist - 1.5).abs() < 1e-6);
        // Inside points find the nearest wall too.
        let c = b.closest(Vec3::new(0.0, 0.45, 0.0), 10.0).unwrap();
        assert!((c.point.y - 0.5).abs() < 1e-6 && (c.dist - 0.05).abs() < 1e-6);
        assert!(b.closest(Vec3::new(5.0, 0.0, 0.0), 1.0).is_none(), "beyond max_dist");
        assert!(b.normal(c.tri).y > 0.99);
    }

    #[test]
    fn rays_and_containment() {
        let b = unit_box(Vec3::new(0.0, 0.0, 2.0));
        let hit = b.raycast(Vec3::ZERO, Vec3::Z, 100.0).unwrap();
        assert!((hit.t - 1.5).abs() < 1e-6 && (hit.point.z - 1.5).abs() < 1e-6);
        assert!(b.raycast(Vec3::ZERO, Vec3::NEG_Z, 100.0).is_none());
        assert!(b.raycast(Vec3::ZERO, Vec3::Z, 1.0).is_none(), "beyond t_max");
        // Off-center: the face centers are grid vertices shared by many triangles.
        assert_eq!(b.crossings(Vec3::new(0.1, 0.13, 0.0), Vec3::Z), 2);
        assert!(b.contains(Vec3::new(0.0, 0.0, 2.0)));
        assert!(b.contains(Vec3::new(0.49, -0.49, 2.49)));
        assert!(!b.contains(Vec3::new(0.0, 0.0, 2.51)));
        assert!(!b.contains(Vec3::ZERO));

        let s = quad_sphere(&QuadSphereSpec { radius: 1.0, segments: 8 }, Origin::Center).unwrap();
        let s = TriBvh::from_mesh(&s, Affine3A::IDENTITY);
        // Points on symmetry axes and planes are the hard cases for parity.
        for p in [Vec3::ZERO, Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.9), Vec3::new(0.3, 0.3, 0.3)] {
            assert!(s.contains(p), "{p}");
        }
        for p in [Vec3::new(1.01, 0.0, 0.0), Vec3::new(0.7, 0.7, 0.7)] {
            assert!(!s.contains(p), "{p}");
        }
    }

    #[test]
    fn crossing_pairs_between_surfaces() {
        let a = unit_box(Vec3::ZERO);
        let overlapping = unit_box(Vec3::new(0.7, 0.3, 0.1));
        let mut n = 0;
        a.crossing_pairs(&overlapping, |_, _| {
            n += 1;
            true
        });
        assert!(n > 0);
        // Face-to-face contact is not a crossing; neither is a gap.
        for at in [Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.2, 0.0, 0.0)] {
            let other = unit_box(at);
            let mut n = 0;
            a.crossing_pairs(&other, |_, _| {
                n += 1;
                true
            });
            assert_eq!(n, 0, "{at}");
        }
        // Early stop.
        let mut n = 0;
        a.crossing_pairs(&overlapping, |_, _| {
            n += 1;
            false
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn self_crossings_skip_neighbours() {
        let s = quad_sphere(&QuadSphereSpec { radius: 1.0, segments: 6 }, Origin::Center).unwrap();
        let bvh = TriBvh::from_mesh(&s, Affine3A::IDENTITY);
        let mut n = 0;
        bvh.self_crossings(|_, _| {
            n += 1;
            true
        });
        assert_eq!(n, 0, "a clean sphere does not cross itself");

        // Two boxes merged into one surface cross each other.
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, segments: [1; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center).unwrap();
        let mut other = m.clone();
        other.transform(Affine3A::from_translation(Vec3::new(0.5, 0.2, 0.3)));
        m.append(&other);
        let bvh = TriBvh::from_mesh(&m, Affine3A::IDENTITY);
        let mut pairs = Vec::new();
        bvh.self_crossings(|t, u| {
            pairs.push((t, u));
            true
        });
        assert!(!pairs.is_empty());
        assert!(pairs.iter().all(|(t, u)| t < u));
        let faces: Vec<(u32, u32)> = pairs.iter().map(|&(t, u)| (bvh.face(t), bvh.face(u))).collect();
        assert!(faces.iter().all(|&(f, g)| (f < 6) != (g < 6)), "only pairs across the two boxes: {faces:?}");
    }

    #[test]
    fn query_box_and_empty() {
        let b = unit_box(Vec3::ZERO);
        let mut hits = Vec::new();
        b.query_box(Vec3::new(0.4, 0.4, 0.4), Vec3::splat(1.0), |t| {
            hits.push(t);
            true
        });
        assert!(!hits.is_empty() && hits.len() < b.len());
        let e = TriBvh::from_triangles(Vec::new(), Vec::new(), Vec::new());
        assert!(e.is_empty() && e.bounds().is_none());
        assert!(e.closest(Vec3::ZERO, 1.0).is_none() && e.raycast(Vec3::ZERO, Vec3::X, 1.0).is_none() && !e.contains(Vec3::ZERO));
    }

    /// Deterministic points in [-1.5, 1.5]^3.
    fn points(n: usize) -> Vec<Vec3> {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 3.0 - 1.5
        };
        (0..n).map(|_| Vec3::new(next(), next(), next())).collect()
    }

    #[test]
    fn queries_match_brute_force() {
        let s = quad_sphere(&QuadSphereSpec { radius: 1.0, segments: 7 }, Origin::Center).unwrap();
        let bvh = TriBvh::from_mesh(&s, Affine3A::from_scale(Vec3::new(1.0, 0.6, 0.8)));
        let all: Vec<u32> = (0..bvh.len() as u32).collect();
        for p in points(300) {
            let brute = all.iter().map(|&t| closest_on_triangle(p, bvh.triangle(t)).distance(p)).fold(f32::INFINITY, f32::min);
            let c = bvh.closest(p, 10.0).unwrap();
            assert!((c.dist - brute).abs() < 1e-6, "{p}: {} vs {brute}", c.dist);

            let dir = (Vec3::new(0.1, 0.2, 0.3) - p).normalize();
            let brute = all.iter().filter_map(|&t| ray_triangle(p, dir, bvh.triangle(t))).filter(|t| *t >= 0.0).fold(f32::INFINITY, f32::min);
            match bvh.raycast(p, dir, 100.0) {
                Some(h) => assert!((h.t - brute).abs() < 1e-6, "{p}"),
                None => assert!(brute.is_infinite(), "{p}"),
            }

            // Away from the surface, containment matches the ellipsoid.
            let e = (p / Vec3::new(1.0, 0.6, 0.8)).length();
            if (e - 1.0).abs() > 0.05 {
                assert_eq!(bvh.contains(p), e < 1.0, "{p}");
            }
        }
    }

    #[test]
    fn nearest_between_surfaces() {
        let a = unit_box(Vec3::ZERO);
        let b = unit_box(Vec3::new(1.25, 0.3, 0.0));
        let n = a.nearest(&b, 10.0).unwrap();
        assert!((n.dist - 0.25).abs() < 1e-6 && !n.approximate, "{n:?}");
        assert!((n.a.x - 0.5).abs() < 1e-6 && (n.b.x - 0.75).abs() < 1e-6);
        assert!(a.nearest(&b, 0.2).is_none(), "beyond max_dist");
        let touching = unit_box(Vec3::new(1.0, 0.0, 0.0));
        assert!(a.nearest(&touching, 1.0).unwrap().dist < 1e-6);
        let crossing = unit_box(Vec3::new(0.6, 0.2, 0.1));
        assert_eq!(a.nearest(&crossing, 1.0).unwrap().dist, 0.0);

        // Brute force over triangle pairs on curved surfaces.
        let s = quad_sphere(&QuadSphereSpec { radius: 0.5, segments: 4 }, Origin::Center).unwrap();
        let p = TriBvh::from_mesh(&s, Affine3A::IDENTITY);
        let q = TriBvh::from_mesh(&s, Affine3A::from_translation(Vec3::new(0.8, 0.7, -0.3)));
        let brute = (0..p.len() as u32)
            .flat_map(|t| (0..q.len() as u32).map(move |u| (t, u)))
            .map(|(t, u)| triangle_distance(p.triangle(t), q.triangle(u)).0)
            .fold(f32::INFINITY, f32::min);
        assert!((p.nearest(&q, 10.0).unwrap().dist - brute).abs() < 1e-6);
    }

    #[test]
    fn box_pairs_find_exactly_the_overlaps() {
        let b = |x: f32, w: f32| (Vec3::new(x, 0.0, 0.0), Vec3::new(x + w, 1.0, 1.0));
        let boxes = vec![b(0.0, 1.0), b(0.5, 1.0), b(3.0, 1.0), b(1.2, 0.5), b(10.0, 1.0)];
        assert_eq!(box_pairs(&boxes, 0.0, 100), (vec![(0, 1), (1, 3)], false));
        // Each box grows by the pad: boxes 2 and 3 are 1.3 apart, more than 2 x 0.5.
        assert_eq!(box_pairs(&boxes, 0.5, 100).0, vec![(0, 1), (0, 3), (1, 3)]);
        assert_eq!(box_pairs(&boxes, 0.7, 100).0, vec![(0, 1), (0, 3), (1, 3), (2, 3)]);
        let (pairs, truncated) = box_pairs(&boxes, 0.5, 2);
        assert_eq!(pairs.len(), 2);
        assert!(truncated);
        // A column of boxes: all overlap in x, none in y.
        let column: Vec<(Vec3, Vec3)> = (0..200).map(|i| (Vec3::new(0.0, i as f32 * 2.0, 0.0), Vec3::new(1.0, i as f32 * 2.0 + 1.0, 1.0))).collect();
        assert_eq!(box_pairs(&column, 0.0, 100), (Vec::new(), false));
        assert_eq!(box_pairs(&column, 0.6, 1000).0.len(), 199);
        assert_eq!(box_pairs(&column[..1], 1.0, 10), (Vec::new(), false));
    }

    #[test]
    fn closest_on_triangle_regions() {
        let t = [Vec3::ZERO, Vec3::X, Vec3::Y];
        assert_eq!(closest_on_triangle(Vec3::new(-1.0, -1.0, 0.0), t), Vec3::ZERO);
        assert!(closest_on_triangle(Vec3::new(0.2, 0.2, 5.0), t).distance(Vec3::new(0.2, 0.2, 0.0)) < 1e-6);
        assert!(closest_on_triangle(Vec3::new(1.0, 1.0, 0.0), t).distance(Vec3::new(0.5, 0.5, 0.0)) < 1e-6);
        assert_eq!(closest_on_triangle(Vec3::new(0.5, -1.0, 0.0), t), Vec3::new(0.5, 0.0, 0.0));
    }
}
