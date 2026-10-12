//! Moving, rotating and scaling mesh elements, with proportional editing
//! (unselected vertices nearby follow with a falloff), and moving vertices
//! along their normals (shrink and fatten).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::{Mat3, Quat, Vec3};
use p3d_mesh::Mesh;
use rustc_hash::FxHashMap;

use super::KernelError;
use crate::kdtree::PointTree;

/// What rotations and scales turn around.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Pivot {
    /// The mean of the selected vertices.
    #[default]
    Median,
    /// The center of their bounding box.
    BoundsCenter,
    /// Each connected piece of the selection around its own median.
    Individual,
    /// A given point (the 3D cursor, an active element), in object space.
    Point(Vec3),
}

/// How proportional editing fades out with distance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Falloff {
    #[default]
    Smooth,
    Sphere,
    Root,
    InverseSquare,
    Sharp,
    Linear,
    Constant,
    Random,
}

impl Falloff {
    /// Weight for `t` = 1 at the selection down to 0 at the radius.
    pub fn weight(self, t: f32, noise: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Falloff::Smooth => t * t * (3.0 - 2.0 * t),
            Falloff::Sphere => (t * (2.0 - t)).sqrt(),
            Falloff::Root => t.sqrt(),
            Falloff::InverseSquare => t * (2.0 - t),
            Falloff::Sharp => t * t,
            Falloff::Linear => t,
            Falloff::Constant => 1.0,
            Falloff::Random => t * noise,
        }
    }
}

/// Proportional editing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proportional {
    /// Unselected vertices within this distance of the selection follow.
    pub radius: f32,
    pub falloff: Falloff,
    /// Measure distance along edges instead of straight through space.
    pub connected_only: bool,
    /// Seed of the random falloff.
    pub seed: u64,
}

/// A transform of selected vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformSpec {
    /// Offset, in the `axes` frame.
    pub translate: Vec3,
    /// Rotation, in the `axes` frame.
    pub rotate: Quat,
    /// Scale along the `axes`.
    pub scale: Vec3,
    pub pivot: Pivot,
    /// Orientation frame (columns are its X, Y, Z axes in object space).
    pub axes: Mat3,
    pub proportional: Option<Proportional>,
}

impl Default for TransformSpec {
    fn default() -> Self {
        TransformSpec {
            translate: Vec3::ZERO,
            rotate: Quat::IDENTITY,
            scale: Vec3::ONE,
            pivot: Pivot::Median,
            axes: Mat3::IDENTITY,
            proportional: None,
        }
    }
}

/// What a transform did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransformResult {
    pub selected: usize,
    /// Unselected vertices that followed (proportional editing).
    pub influenced: usize,
}

/// Live, sorted, deduplicated vertices.
fn live(mesh: &Mesh, verts: &[u32]) -> Result<Vec<u32>, KernelError> {
    let mut vs: Vec<u32> = verts.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
    vs.sort_unstable();
    vs.dedup();
    if vs.is_empty() {
        return Err(KernelError::Empty("no vertices selected".into()));
    }
    Ok(vs)
}

/// A hash of a vertex id and a seed in [0, 1).
fn unit_noise(v: u32, seed: u64) -> f32 {
    let mut h = seed ^ u64::from(v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Moves, rotates and scales vertices around a pivot in an orientation
/// frame; with proportional editing, unselected vertices within the radius
/// follow by a falloff weight of their distance to the selection (straight
/// or along edges).
///
/// Complexity: O(k log k) for k selected vertices without proportional
/// editing; with it, O(n log k) (straight distances, n vertices) or
/// O(m log m) for the m vertices reached along edges.
pub fn transform_verts(mesh: &mut Mesh, verts: &[u32], spec: &TransformSpec) -> Result<TransformResult, KernelError> {
    let vs = live(mesh, verts)?;
    if !(spec.translate.is_finite() && spec.scale.is_finite() && spec.rotate.is_finite() && spec.axes.is_finite()) {
        return Err(KernelError::OutOfRange { param: "translate", message: "transform values must be finite".into(), hint: String::new() });
    }
    let pos = |v: u32| mesh.positions[v as usize];
    // Pivot of each selected vertex.
    let mut pivot_of: FxHashMap<u32, Vec3> = FxHashMap::default();
    match spec.pivot {
        Pivot::Median => {
            let c = vs.iter().map(|&v| pos(v)).sum::<Vec3>() / vs.len() as f32;
            pivot_of.extend(vs.iter().map(|&v| (v, c)));
        }
        Pivot::BoundsCenter => {
            let (lo, hi) = vs.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), &v| (a.min(pos(v)), b.max(pos(v))));
            pivot_of.extend(vs.iter().map(|&v| (v, (lo + hi) * 0.5)));
        }
        Pivot::Point(p) => pivot_of.extend(vs.iter().map(|&v| (v, p))),
        Pivot::Individual => {
            // Pieces: selected vertices joined by edges between selected vertices.
            let index: FxHashMap<u32, u32> = vs.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
            let mut parent: Vec<u32> = (0..vs.len() as u32).collect();
            fn root(p: &mut [u32], mut x: u32) -> u32 {
                while p[x as usize] != x {
                    p[x as usize] = p[p[x as usize] as usize];
                    x = p[x as usize];
                }
                x
            }
            for (i, &v) in vs.iter().enumerate() {
                for w in mesh.vert_neighbors(v) {
                    if let Some(&j) = index.get(&w) {
                        let (a, b) = (root(&mut parent, i as u32), root(&mut parent, j));
                        parent[a.max(b) as usize] = a.min(b);
                    }
                }
            }
            let mut sums: FxHashMap<u32, (Vec3, u32)> = FxHashMap::default();
            for (i, &v) in vs.iter().enumerate() {
                let r = root(&mut parent, i as u32);
                let e = sums.entry(r).or_insert((Vec3::ZERO, 0));
                e.0 += pos(v);
                e.1 += 1;
            }
            for (i, &v) in vs.iter().enumerate() {
                let r = root(&mut parent, i as u32);
                let (s, n) = sums[&r];
                pivot_of.insert(v, s / n as f32);
            }
        }
    }
    let inv = spec.axes.transpose();
    let apply = |p: Vec3, pivot: Vec3| -> Vec3 {
        let local = inv * (p - pivot);
        pivot + spec.axes * (spec.rotate * (local * spec.scale)) + spec.axes * spec.translate
    };
    let mut moves: Vec<(u32, Vec3)> = vs.iter().map(|&v| (v, apply(pos(v), pivot_of[&v]))).collect();
    let mut influenced = 0usize;
    if let Some(prop) = spec.proportional {
        if !(prop.radius.is_finite() && prop.radius > 0.0) {
            return Err(KernelError::OutOfRange {
                param: "proportional",
                message: "the proportional radius must be more than 0".into(),
                hint: "for example 0.2".into(),
            });
        }
        let selected: rustc_hash::FxHashSet<u32> = vs.iter().copied().collect();
        // Distance of each unselected vertex in reach to the selection, and
        // the selected vertex it is nearest to (its pivot).
        let mut reach: Vec<(u32, f32, u32)> = Vec::new();
        if prop.connected_only {
            let mut best: FxHashMap<u32, (f32, u32)> = FxHashMap::default();
            let mut heap: BinaryHeap<(Reverse<u32>, u32, u32)> = BinaryHeap::new();
            // Distances as f32 bits order correctly for non-negative values.
            for &v in &vs {
                best.insert(v, (0.0, v));
                heap.push((Reverse(0.0f32.to_bits()), v, v));
            }
            while let Some((Reverse(db), v, src)) = heap.pop() {
                let d = f32::from_bits(db);
                if best.get(&v).is_some_and(|&(b, _)| b < d) {
                    continue;
                }
                for w in mesh.vert_neighbors(v) {
                    let nd = d + pos(v).distance(pos(w));
                    if nd < prop.radius && best.get(&w).is_none_or(|&(b, _)| nd < b) {
                        best.insert(w, (nd, src));
                        heap.push((Reverse(nd.to_bits()), w, src));
                    }
                }
            }
            let mut list: Vec<(u32, f32, u32)> = best.into_iter().filter(|(v, _)| !selected.contains(v)).map(|(v, (d, s))| (v, d, s)).collect();
            list.sort_unstable_by_key(|x| x.0);
            reach = list;
        } else {
            let pts: Vec<Vec3> = vs.iter().map(|&v| pos(v)).collect();
            let tree = PointTree::new(&pts);
            for u in mesh.verts() {
                if selected.contains(&u) {
                    continue;
                }
                if let Some((i, d)) = tree.nearest(pos(u), prop.radius)
                    && d < prop.radius
                {
                    reach.push((u, d, vs[i as usize]));
                }
            }
        }
        for (u, d, src) in reach {
            let w = prop.falloff.weight(1.0 - d / prop.radius, unit_noise(u, prop.seed));
            if w > 0.0 {
                let p = pos(u);
                moves.push((u, p + (apply(p, pivot_of[&src]) - p) * w));
                influenced += 1;
            }
        }
    }
    for (v, p) in moves {
        mesh.positions[v as usize] = p;
    }
    mesh.revision += 1;
    Ok(TransformResult { selected: vs.len(), influenced })
}

/// Moves vertices along their normals (area-weighted over their faces) by
/// `distance`; with `even`, the offset stretches at creases so faces move
/// by the full distance. Returns the number of vertices moved.
///
/// Complexity: O(k log k + sum of valences) for k vertices.
pub fn shrink_fatten(mesh: &mut Mesh, verts: &[u32], distance: f32, even: bool) -> Result<usize, KernelError> {
    let vs = live(mesh, verts)?;
    if !distance.is_finite() {
        return Err(KernelError::OutOfRange { param: "distance", message: "distance must be finite".into(), hint: String::new() });
    }
    let moves: Vec<(u32, Vec3)> = vs
        .iter()
        .map(|&v| {
            let ns: Vec<Vec3> = mesh.vert_faces(v).map(|f| mesh.face_normal_area2(f)).collect();
            let n = ns.iter().sum::<Vec3>().normalize_or_zero();
            let stretch = if even { 1.0 / ns.iter().map(|x| x.normalize_or_zero().dot(n)).fold(1.0f32, f32::min).max(0.25) } else { 1.0 };
            (v, mesh.positions[v as usize] + n * distance * stretch)
        })
        .collect();
    for (v, p) in moves {
        mesh.positions[v as usize] = p;
    }
    mesh.revision += 1;
    Ok(vs.len())
}

/// Slides the vertices of selected edges along the edges that leave them
/// sideways. Each connected chain of selected edges is oriented from its
/// lowest-numbered edge on, so a whole loop has one "left" side: `factor` 1
/// moves every vertex all the way to its neighbour on that side, -1 to the
/// other side, 0 leaves it. Where a side has no face (an open border), the
/// vertex slides along the other side by the same amount. Returns the
/// number of vertices moved.
///
/// Complexity: O(e log e + sum of valences) for e selected edges.
pub fn edge_slide(mesh: &mut Mesh, edges: &[u32], factor: f32) -> Result<usize, KernelError> {
    if !(factor.is_finite() && (-1.0..=1.0).contains(&factor)) {
        return Err(KernelError::OutOfRange {
            param: "factor",
            message: format!("factor {factor} is outside -1 to 1"),
            hint: "0.5 slides halfway".into(),
        });
    }
    let mut edges: Vec<u32> = edges.iter().copied().filter(|&e| mesh.is_edge_alive(e)).collect();
    edges.sort_unstable();
    edges.dedup();
    if edges.is_empty() {
        return Err(KernelError::Empty("no edges selected".into()));
    }
    let selected: rustc_hash::FxHashSet<u32> = edges.iter().copied().collect();
    // Orient chains: walking from an oriented half-edge, the next selected
    // edge at its end starts there, the previous one at its start ends there.
    // Each vertex keeps the first oriented half-edge that reaches it.
    let mut oriented: FxHashMap<u32, u32> = FxHashMap::default(); // edge -> half-edge
    let mut first: FxHashMap<u32, (u32, bool)> = FxHashMap::default(); // vertex -> (half-edge, far end)
    for &e in &edges {
        if oriented.contains_key(&e) {
            continue;
        }
        let mut stack = vec![2 * e];
        oriented.insert(e, 2 * e);
        while let Some(h) = stack.pop() {
            first.entry(mesh.origin(h)).or_insert((h, false));
            first.entry(mesh.dest(h)).or_insert((h, true));
            for (x, starts_here) in [(mesh.dest(h), true), (mesh.origin(h), false)] {
                for g in mesh.vert_outgoing(x) {
                    let ge = g >> 1;
                    if selected.contains(&ge) && !oriented.contains_key(&ge) {
                        // g starts at x: keep it when the chain continues from x, flip it otherwise.
                        let o = if starts_here { g } else { g ^ 1 };
                        oriented.insert(ge, o);
                        stack.push(o);
                    }
                }
            }
        }
    }
    let mut verts: Vec<(u32, (u32, bool))> = first.into_iter().collect();
    verts.sort_unstable();
    let pos = |v: u32| mesh.positions[v as usize];
    let mut moves = Vec::with_capacity(verts.len());
    for (v, (h, far)) in verts {
        // The left face holds h (v -> w), the right face its twin. A vertex's
        // side neighbour in a face is along that face's other edge at it.
        let (left, right) = if far {
            (mesh.face(h).map(|_| mesh.dest(mesh.next(h))), mesh.face(h ^ 1).map(|_| mesh.origin(mesh.prev(h ^ 1))))
        } else {
            (mesh.face(h).map(|_| mesh.origin(mesh.prev(h))), mesh.face(h ^ 1).map(|_| mesh.dest(mesh.next(h ^ 1))))
        };
        let target = if factor >= 0.0 { left.or(right) } else { right.or(left) };
        if let Some(t) = target {
            moves.push((v, pos(v).lerp(pos(t), factor.abs())));
        }
    }
    let n = moves.len();
    for (v, p) in moves {
        mesh.positions[v as usize] = p;
    }
    mesh.revision += 1;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prim::{BoxSpec, Origin, PlaneSpec, cuboid, plane};
    use glam::DVec3;

    #[test]
    fn move_rotate_scale_around_pivots() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let top: Vec<u32> = m.verts().filter(|&v| m.positions[v as usize].y > 0.4).collect();
        // Raise the top 0.5 along +Y.
        transform_verts(&mut m, &top, &TransformSpec { translate: Vec3::Y * 0.5, ..TransformSpec::default() }).unwrap();
        assert!(top.iter().all(|&v| (m.positions[v as usize].y - 1.0).abs() < 1e-6));
        // Scale the top to a point at its median: a pyramid.
        transform_verts(&mut m, &top, &TransformSpec { scale: Vec3::ZERO, ..TransformSpec::default() }).unwrap();
        assert!(top.iter().all(|&v| m.positions[v as usize].abs_diff_eq(Vec3::Y, 1e-6)));
        m.validate().unwrap();
        // Rotate around a given point in a tilted frame.
        let mut g = plane(&PlaneSpec::default()).unwrap();
        let all: Vec<u32> = g.verts().collect();
        let spec = TransformSpec {
            rotate: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            pivot: Pivot::Point(Vec3::new(0.5, 0.0, 0.5)),
            ..TransformSpec::default()
        };
        transform_verts(&mut g, &all, &spec).unwrap();
        let (lo, hi) = g.bounds().unwrap();
        // Relative to (0.5, 0.5) the square spans x, z in [-1, 0]; a quarter turn
        // around +Y maps (x, z) to (z, -x).
        assert!(lo.abs_diff_eq(Vec3::new(-0.5, 0.0, 0.5), 1e-5) && hi.abs_diff_eq(Vec3::new(0.5, 0.0, 1.5), 1e-5), "{lo} {hi}");
    }

    #[test]
    fn translation_follows_the_orientation_frame() {
        let mut g = plane(&PlaneSpec::default()).unwrap();
        let all: Vec<u32> = g.verts().collect();
        // A frame whose Z axis is world Y: "move along local Z" moves up.
        let axes = Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::Y);
        transform_verts(&mut g, &all, &TransformSpec { translate: Vec3::Z * 0.3, axes, ..TransformSpec::default() }).unwrap();
        assert!(g.verts().all(|v| (g.positions[v as usize].y - 0.3).abs() < 1e-6));
    }

    #[test]
    fn individual_origins_scale_pieces_in_place() {
        let mut g = plane(&PlaneSpec { size_x: 5.0, size_z: 1.0, segments_x: 5, segments_z: 1 }).unwrap();
        // The end quads' vertices are two separate pieces (the middle vertices stay).
        let left: Vec<u32> = g.verts().filter(|&v| g.positions[v as usize].x < -1.4).collect();
        let right: Vec<u32> = g.verts().filter(|&v| g.positions[v as usize].x > 1.4).collect();
        let sel: Vec<u32> = left.iter().chain(&right).copied().collect();
        transform_verts(&mut g, &sel, &TransformSpec { scale: Vec3::splat(0.5), pivot: Pivot::Individual, ..TransformSpec::default() }).unwrap();
        g.validate().unwrap();
        let xs: Vec<f32> = left.iter().map(|&v| g.positions[v as usize].x).collect();
        assert!(xs.iter().all(|&x| (x + 2.25).abs() < 1e-5 || (x + 1.75).abs() < 1e-5), "{xs:?}");
    }

    #[test]
    fn proportional_editing_falls_off() {
        let mut g = plane(&PlaneSpec { size_x: 4.0, size_z: 4.0, segments_x: 8, segments_z: 8 }).unwrap();
        let center = g.verts().find(|&v| g.positions[v as usize].length() < 1e-6).unwrap();
        let spec = TransformSpec {
            translate: Vec3::Y,
            proportional: Some(Proportional { radius: 1.2, falloff: Falloff::Linear, connected_only: false, seed: 0 }),
            ..TransformSpec::default()
        };
        let r = transform_verts(&mut g, &[center], &spec).unwrap();
        assert_eq!(r.selected, 1);
        assert!(r.influenced >= 8);
        let h =
            |x: f32, z: f32| g.verts().find(|&v| g.positions[v as usize].x == x && g.positions[v as usize].z == z).map(|v| g.positions[v as usize].y);
        assert_eq!(h(0.0, 0.0), Some(1.0));
        let near = h(0.5, 0.0).unwrap();
        assert!((near - (1.0 - 0.5 / 1.2)).abs() < 1e-5, "{near}");
        assert_eq!(h(2.0, 0.0), Some(0.0), "outside the radius");
        // Along edges, diagonal neighbours are farther (two edges away).
        let mut c = plane(&PlaneSpec { size_x: 4.0, size_z: 4.0, segments_x: 8, segments_z: 8 }).unwrap();
        let spec = TransformSpec { proportional: Some(Proportional { connected_only: true, ..spec.proportional.unwrap() }), ..spec };
        transform_verts(&mut c, &[center], &spec).unwrap();
        let diag =
            c.verts().find(|&v| c.positions[v as usize].x == 0.5 && c.positions[v as usize].z == 0.5).map(|v| c.positions[v as usize].y).unwrap();
        assert!((diag - (1.0 - 1.0 / 1.2)).abs() < 1e-5, "{diag}");
    }

    #[test]
    fn falloff_shapes() {
        for f in [Falloff::Smooth, Falloff::Sphere, Falloff::Root, Falloff::InverseSquare, Falloff::Sharp, Falloff::Linear, Falloff::Constant] {
            assert!((f.weight(1.0, 0.5) - 1.0).abs() < 1e-6, "{f:?} at the selection");
            assert!(f.weight(0.0, 0.5) < 1e-6 || f == Falloff::Constant, "{f:?} at the radius");
        }
        assert!(Falloff::Sharp.weight(0.5, 0.0) < Falloff::Linear.weight(0.5, 0.0));
        assert!(Falloff::Root.weight(0.5, 0.0) > Falloff::Linear.weight(0.5, 0.0));
    }

    #[test]
    fn edge_slide_moves_a_loop_across_quads() {
        // A 1 x 2 grid; slide the middle row of edges (z = 0) toward one side.
        let mut g = plane(&PlaneSpec { size_x: 1.0, size_z: 2.0, segments_x: 1, segments_z: 2 }).unwrap();
        let mid: Vec<u32> = g.edges().filter(|&e| g.edge_verts(e).iter().all(|&v| g.positions[v as usize].z.abs() < 1e-6)).collect();
        assert_eq!(mid.len(), 1);
        let n = edge_slide(&mut g, &mid, 0.5).unwrap();
        g.validate().unwrap();
        assert_eq!(n, 2);
        let zs: Vec<f32> = g
            .verts()
            .filter(|&v| g.positions[v as usize].z.abs() > 1e-6 && g.positions[v as usize].z.abs() < 0.99)
            .map(|v| g.positions[v as usize].z)
            .collect();
        assert_eq!(zs.len(), 2);
        assert!((zs[0] - zs[1]).abs() < 1e-6 && (zs[0].abs() - 0.5).abs() < 1e-6, "both ends slid the same way: {zs:?}");
        assert!(edge_slide(&mut g, &mid, 2.0).is_err());
    }

    #[test]
    fn edge_slide_keeps_a_ring_together() {
        use crate::prim::{CapKind, CylinderSpec, cylinder};
        // The middle ring of a two-row cylinder: every vertex slides the same way.
        let mut m =
            cylinder(&CylinderSpec { radius: 0.5, height: 2.0, segments: 12, height_segments: 2, cap: CapKind::None }, Origin::Center).unwrap();
        let ring: Vec<u32> = m.edges().filter(|&e| m.edge_verts(e).iter().all(|&v| m.positions[v as usize].y.abs() < 1e-6)).collect();
        assert_eq!(ring.len(), 12);
        assert_eq!(edge_slide(&mut m, &ring, 0.5).unwrap(), 12);
        m.validate().unwrap();
        let ys: Vec<f32> = m
            .verts()
            .filter(|&v| m.positions[v as usize].y.abs() > 1e-6 && m.positions[v as usize].y.abs() < 0.99)
            .map(|v| m.positions[v as usize].y)
            .collect();
        assert_eq!(ys.len(), 12);
        assert!(ys.iter().all(|&y| (y - ys[0]).abs() < 1e-6 && (y.abs() - 0.5).abs() < 1e-6), "{ys:?}");
        // The open rim slides inward along the surface whatever the sign.
        let rim: Vec<u32> =
            m.edges().filter(|&e| m.is_boundary_edge(e) && m.edge_verts(e).iter().all(|&v| m.positions[v as usize].y > 0.99)).collect();
        edge_slide(&mut m, &rim, -0.5).unwrap();
        assert!(m.verts().all(|v| m.positions[v as usize].y < 0.99), "the rim moved down into the surface");
    }

    #[test]
    fn shrink_and_fatten_along_normals() {
        let mut m = cuboid(&BoxSpec { size: DVec3::ONE, ..BoxSpec::default() }, Origin::Center).unwrap();
        let all: Vec<u32> = m.verts().collect();
        shrink_fatten(&mut m, &all, 0.1, true).unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert!((hi - lo).abs_diff_eq(Vec3::splat(1.2), 1e-5), "even: every face moved 0.1 out: {}", hi - lo);
        assert!(shrink_fatten(&mut m, &[], 0.1, true).is_err());
    }
}
