//! Meshing an SDF on a sparse narrow band with surface nets.
//!
//! The grid is split into blocks of [`BLOCK`]^3 voxels. An octree over the
//! blocks skips every node whose center is farther from the surface than
//! the node's half diagonal allows (the tree's Lipschitz bound), so only
//! blocks near the surface are sampled: the cost follows the surface area,
//! not the grid volume. In each sampled voxel the surface crosses, one
//! vertex sits at the mean of the edge crossings, pulled onto the surface
//! by a Newton step; every grid edge the surface crosses gives one quad
//! between the four voxels around it. The quads become a 2-manifold mesh
//! through soup repair (rare ambiguous voxels split instead of pinching).

use glam::{IVec3, Vec3};
use p3d_geo::soup::{SoupReport, SoupSpec, repair};
use p3d_mesh::Mesh;
use rayon::prelude::*;
use std::collections::BTreeMap;

use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::tree::{Sdf, SdfError};

/// A signed distance field the mesher can sample: negative inside, positive
/// outside, with a bound on its gradient for skipping empty space.
pub trait Field: Sync {
    /// Signed distance (or a bound of it) at `p`.
    fn eval(&self, p: Vec3) -> f32;
    /// Upper bound of the gradient length.
    fn lipschitz(&self) -> f32;
    /// Box holding the surface.
    fn bounds(&self) -> (Vec3, Vec3);
    /// The item a surface point belongs to (0 when there are no items).
    fn leaf(&self, _p: Vec3) -> u32 {
        0
    }
    /// Gradient by central differences of step `h`.
    fn gradient(&self, p: Vec3, h: f32) -> Vec3 {
        let d = |o: Vec3| self.eval(p + o) - self.eval(p - o);
        Vec3::new(d(Vec3::X * h), d(Vec3::Y * h), d(Vec3::Z * h)) / (2.0 * h)
    }
}

impl Field for Sdf {
    fn eval(&self, p: Vec3) -> f32 {
        Sdf::eval(self, p)
    }

    fn lipschitz(&self) -> f32 {
        Sdf::lipschitz(self)
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        Sdf::bounds(self)
    }

    fn leaf(&self, p: Vec3) -> u32 {
        self.eval_item(p).1
    }

    fn gradient(&self, p: Vec3, h: f32) -> Vec3 {
        Sdf::gradient(self, p, h)
    }
}

/// Voxels per block side.
pub const BLOCK: i32 = 8;
/// Most voxels along one axis.
pub const MAX_AXIS_VOXELS: i64 = 2048;
/// Most voxels sampled in one mesh (blocks near the surface).
pub const MAX_SAMPLED_VOXELS: u64 = 64_000_000;

/// Options of [`mesh`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshSpec {
    /// Voxel size in meters: about the edge length of the result.
    pub voxel: f32,
}

/// A meshed SDF.
#[derive(Debug, Clone, Default)]
pub struct SdfMesh {
    pub mesh: Mesh,
    /// The leaf item each face belongs to, by face index (see
    /// [`Sdf::leaves`]).
    pub face_leaf: Vec<u32>,
    /// What soup repair had to fix (ambiguous voxels).
    pub repair: SoupReport,
    /// Voxels sampled (blocks near the surface).
    pub sampled: u64,
    /// The voxel size used (smaller than asked where walls were thinner).
    pub voxel: f32,
    /// Quads left out because a voxel around them could not be sampled.
    pub unresolved: usize,
    /// Pieces smaller than half a voxel, removed.
    pub specks: usize,
}

/// One sampled block: its origin (in voxels), corner values and the
/// vertices of the voxels the surface crosses.
/// One sampled block: its origin (in voxels), corner values and the
/// surface pieces of the voxels the surface crosses.
struct Block {
    origin: IVec3,
    corners: Vec<f32>,
    voxels: Vec<Voxel>,
}

/// The surface pieces in one voxel: which piece each crossed edge belongs
/// to (`NONE` for edges the surface does not cross) and a vertex per piece.
struct Voxel {
    index: u16,
    edge_piece: [u8; 12],
    verts: SmallVec<[Vec3; 2]>,
}

const NONE: u8 = u8::MAX;
const C: i32 = BLOCK + 1;

/// The 12 voxel edges as corner offset pairs: x edges at (y, z) = (0,0),
/// (1,0), (0,1), (1,1), then y edges at (x, z), then z edges at (x, y).
const EDGES: [([i32; 3], [i32; 3]); 12] = [
    ([0, 0, 0], [1, 0, 0]),
    ([0, 1, 0], [1, 1, 0]),
    ([0, 0, 1], [1, 0, 1]),
    ([0, 1, 1], [1, 1, 1]),
    ([0, 0, 0], [0, 1, 0]),
    ([1, 0, 0], [1, 1, 0]),
    ([0, 0, 1], [0, 1, 1]),
    ([1, 0, 1], [1, 1, 1]),
    ([0, 0, 0], [0, 0, 1]),
    ([1, 0, 0], [1, 0, 1]),
    ([0, 1, 0], [0, 1, 1]),
    ([1, 1, 0], [1, 1, 1]),
];

/// The six voxel faces: four corners in cyclic order and the edges between
/// consecutive corners (corner i to i + 1). Opposite faces list their
/// corners in the same order, so neighbouring voxels read a shared face
/// alike.
const FACES: [([[i32; 3]; 4], [usize; 4]); 6] = [
    ([[0, 0, 0], [0, 1, 0], [0, 1, 1], [0, 0, 1]], [4, 10, 6, 8]),
    ([[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]], [5, 11, 7, 9]),
    ([[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], [0, 9, 2, 8]),
    ([[0, 1, 0], [1, 1, 0], [1, 1, 1], [0, 1, 1]], [1, 11, 3, 10]),
    ([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], [0, 5, 1, 4]),
    ([[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]], [2, 7, 3, 6]),
];

/// Local edge of a voxel for the grid edge along `axis` whose low corner
/// sits at `offset` (0 or 1 on the other two axes) from the voxel's corner.
#[inline]
fn local_edge(axis: usize, offset: IVec3) -> usize {
    match axis {
        0 => (offset.y + 2 * offset.z) as usize,
        1 => 4 + (offset.x + 2 * offset.z) as usize,
        _ => 8 + (offset.x + 2 * offset.y) as usize,
    }
}

#[inline]
fn corner_index(x: i32, y: i32, z: i32) -> usize {
    ((z * C + y) * C + x) as usize
}

/// Groups a voxel's crossed edges into surface pieces: on each face, the
/// crossed edges pair up along the curve where the surface meets the face;
/// a face crossed four times (a checkerboard) pairs by the asymptotic
/// decider (the sign of the bilinear saddle). Returns the piece of each
/// edge and the number of pieces.
///
/// Complexity: O(1).
fn pieces(value: impl Fn([i32; 3]) -> f32) -> ([u8; 12], u8) {
    let crossed: [bool; 12] = std::array::from_fn(|e| (value(EDGES[e].0) < 0.0) != (value(EDGES[e].1) < 0.0));
    let mut parent: [usize; 12] = std::array::from_fn(|i| i);
    fn find(p: &mut [usize; 12], mut x: usize) -> usize {
        while p[x] != x {
            x = p[x];
        }
        x
    }
    let mut join = |a: usize, b: usize| {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    };
    for (corners, edges) in FACES {
        let v = corners.map(&value);
        let hit: Vec<usize> = (0..4).filter(|&i| crossed[edges[i]]).collect();
        match hit.as_slice() {
            [a, b] => join(edges[*a], edges[*b]),
            [_, _, _, _] => {
                // Corners 0 and 2 share a sign. If the saddle has it too,
                // they connect across the face and the curves cut off
                // corners 1 and 3; otherwise they cut off 0 and 2.
                let den = v[0] + v[2] - v[1] - v[3];
                let saddle = if den.abs() > 1e-20 { (v[0] * v[2] - v[1] * v[3]) / den } else { (v[0] + v[1] + v[2] + v[3]) * 0.25 };
                if (saddle < 0.0) == (v[0] < 0.0) {
                    join(edges[0], edges[1]);
                    join(edges[2], edges[3]);
                } else {
                    join(edges[3], edges[0]);
                    join(edges[1], edges[2]);
                }
            }
            _ => {}
        }
    }
    let mut out = [NONE; 12];
    let mut root_piece: [u8; 12] = [NONE; 12];
    let mut n = 0u8;
    for e in 0..12 {
        if crossed[e] {
            let r = find(&mut parent, e);
            if root_piece[r] == NONE {
                root_piece[r] = n;
                n += 1;
            }
            out[e] = root_piece[r];
        }
    }
    (out, n)
}

/// Blocks near the surface, by octree descent over the block grid.
///
/// Complexity: O(b log n) for b blocks near the surface in an n-block grid.
fn active_blocks<F: Field>(sdf: &F, origin: Vec3, h: f32, blocks: IVec3) -> Vec<IVec3> {
    let side = blocks.max_element().max(1) as u32;
    let top = side.next_power_of_two() as i32;
    let size = h * BLOCK as f32;
    let l = sdf.lipschitz();
    let mut out = Vec::new();
    let mut stack = vec![(IVec3::ZERO, top)];
    while let Some((at, n)) = stack.pop() {
        if at.x >= blocks.x || at.y >= blocks.y || at.z >= blocks.z {
            continue;
        }
        let lo = origin + at.as_vec3() * size;
        let extent = n as f32 * size;
        let center = lo + Vec3::splat(extent * 0.5);
        let r = extent * 0.5 * 3f32.sqrt();
        if sdf.eval(center).abs() > l * r + h {
            continue;
        }
        if n == 1 {
            out.push(at);
            continue;
        }
        let half = n / 2;
        for c in 0..8 {
            stack.push((at + IVec3::new(c & 1, (c >> 1) & 1, (c >> 2) & 1) * half, half));
        }
    }
    out.sort_unstable_by_key(|b| (b.z, b.y, b.x));
    out
}

/// Samples one block and places a vertex for each surface piece in each of
/// its voxels: the mean of the piece's edge crossings, pulled onto the
/// surface by a Newton step and kept near the voxel.
///
/// Complexity: O(BLOCK^3 x items).
fn sample<F: Field>(sdf: &F, origin: Vec3, h: f32, at: IVec3) -> Block {
    let base = at * BLOCK;
    let pos = |p: IVec3| origin + (base + p).as_vec3() * h;
    let mut corners = vec![0.0f32; (C * C * C) as usize];
    for z in 0..C {
        for y in 0..C {
            for x in 0..C {
                // A value that is not a number counts as outside.
                let f = sdf.eval(pos(IVec3::new(x, y, z)));
                corners[corner_index(x, y, z)] = if f.is_nan() { f32::MAX } else { f };
            }
        }
    }
    let mut voxels = Vec::new();
    for z in 0..BLOCK {
        for y in 0..BLOCK {
            for x in 0..BLOCK {
                let value = |c: [i32; 3]| corners[corner_index(x + c[0], y + c[1], z + c[2])];
                let (edge_piece, n) = pieces(value);
                if n == 0 {
                    continue;
                }
                let mut sums: SmallVec<[(Vec3, u32); 2]> = SmallVec::from_elem((Vec3::ZERO, 0), n as usize);
                for (e, &piece) in edge_piece.iter().enumerate() {
                    if piece == NONE {
                        continue;
                    }
                    let (a, b) = EDGES[e];
                    let (fa, fb) = (value(a), value(b));
                    let t = (fa / (fa - fb)).clamp(0.0, 1.0);
                    let (pa, pb) = (IVec3::from(a).as_vec3(), IVec3::from(b).as_vec3());
                    let s = &mut sums[piece as usize];
                    s.0 += pa + (pb - pa) * t;
                    s.1 += 1;
                }
                let cell = IVec3::new(x, y, z);
                let (lo, hi) = (pos(cell) - Vec3::splat(h * 0.5), pos(cell + IVec3::ONE) + Vec3::splat(h * 0.5));
                let verts = sums
                    .iter()
                    .map(|&(sum, k)| {
                        let p = origin + (base.as_vec3() + cell.as_vec3() + sum / k as f32) * h;
                        let g = sdf.gradient(p, h * 0.25);
                        let g2 = g.length_squared();
                        let q = if g2 > 1e-12 { (p - g * (sdf.eval(p) / g2)).clamp(lo, hi) } else { p };
                        if q.is_finite() { q } else { (lo + hi) * 0.5 }
                    })
                    .collect();
                voxels.push(Voxel { index: ((z * BLOCK + y) * BLOCK + x) as u16, edge_piece, verts });
            }
        }
    }
    Block { origin: base, corners, voxels }
}

/// Most rounds of sampling blocks the octree skipped but the surface needs.
pub const MAX_GROW: u32 = 8;

/// Vertices and quads of the sampled blocks (in block order), and the
/// voxels a quad needed that no sampled block holds.
///
/// Complexity: O(v) expected for the v sampled voxels.
fn assemble(blocks: &BTreeMap<(i32, i32, i32), Block>) -> (Vec<Vec3>, Vec<u32>, Vec<IVec3>) {
    // Vertex ids in block order: each voxel's first vertex and its pieces.
    let mut ids: FxHashMap<IVec3, (u32, [u8; 12])> = FxHashMap::default();
    let mut positions: Vec<Vec3> = Vec::new();
    for b in blocks.values() {
        for v in &b.voxels {
            let i = i32::from(v.index);
            let local = IVec3::new(i % BLOCK, (i / BLOCK) % BLOCK, i / (BLOCK * BLOCK));
            ids.insert(b.origin + local, (positions.len() as u32, v.edge_piece));
            positions.extend(v.verts.iter().copied());
        }
    }
    // A quad for every crossed grid edge whose low corner the block owns,
    // joining the pieces of the four voxels around it that hold the edge.
    let mut corners: Vec<u32> = Vec::new();
    let mut missing: Vec<IVec3> = Vec::new();
    let axes = [IVec3::X, IVec3::Y, IVec3::Z];
    for b in blocks.values() {
        for z in 0..BLOCK {
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let f0 = b.corners[corner_index(x, y, z)];
                    for (a, e) in axes.iter().enumerate() {
                        let f1 = b.corners[corner_index(x + e.x, y + e.y, z + e.z)];
                        if (f0 < 0.0) == (f1 < 0.0) {
                            continue;
                        }
                        let g = b.origin + IVec3::new(x, y, z);
                        // The two other axes, in the order that makes the quad face +axis.
                        let (u, v) = (axes[(a + 1) % 3], axes[(a + 2) % 3]);
                        let ring = [g - u - v, g - v, g, g - u];
                        let mut q = [0u32; 4];
                        let mut whole = true;
                        for (slot, &cell) in q.iter_mut().zip(&ring) {
                            match ids.get(&cell) {
                                Some((first, piece)) if piece[local_edge(a, g - cell)] != NONE => {
                                    *slot = first + u32::from(piece[local_edge(a, g - cell)]);
                                }
                                _ => {
                                    missing.push(cell);
                                    whole = false;
                                }
                            }
                        }
                        if !whole {
                            continue;
                        }
                        // Inside at the low corner: the surface faces +axis.
                        if f0 < 0.0 {
                            corners.extend(q);
                        } else {
                            corners.extend([q[3], q[2], q[1], q[0]]);
                        }
                    }
                }
            }
        }
    }
    (positions, corners, missing)
}

/// Most times the voxel size halves to close walls thinner than a voxel.
pub const MAX_REFINE: u32 = 2;

/// Meshes the zero set of `sdf` with voxels of `spec.voxel` meters. Where
/// walls are thinner than a voxel the surface cannot close; the voxel size
/// then halves, up to [`MAX_REFINE`] times ([`SdfMesh::voxel`] says which
/// size was used).
///
/// Complexity: O(v x items) for the v voxels in blocks near the surface
/// (about the surface area over the voxel area, times the block size), plus
/// O(b log n) for the block search; blocks are sampled in parallel. Each
/// refinement costs about four times the previous pass.
pub fn mesh<F: Field>(sdf: &F, spec: &MeshSpec) -> Result<SdfMesh, SdfError> {
    let h = spec.voxel;
    if !(h.is_finite() && h > 0.0) {
        return Err(SdfError::Invalid { path: "voxel".into(), message: "the voxel size must be positive".into() });
    }
    let mut out = mesh_at(sdf, h)?;
    for k in 1..=MAX_REFINE {
        if out.mesh.edges().all(|e| !out.mesh.is_boundary_edge(e)) {
            break;
        }
        match mesh_at(sdf, h / (1u32 << k) as f32) {
            Ok(finer) => out = finer,
            // Too fine to sample: keep the coarser result with its cracks.
            Err(SdfError::TooLarge(_)) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// One meshing pass at voxel size `h`.
///
/// Complexity: see [`mesh`].
fn mesh_at<F: Field>(sdf: &F, h: f32) -> Result<SdfMesh, SdfError> {
    let (lo, hi) = sdf.bounds();
    let start = ((lo / h).floor() - Vec3::splat(2.0)).as_ivec3();
    let end = ((hi / h).ceil() + Vec3::splat(2.0)).as_ivec3();
    let n = end - start;
    if i64::from(n.max_element()) > MAX_AXIS_VOXELS {
        let total = i64::from(n.x) * i64::from(n.y) * i64::from(n.z);
        return Err(SdfError::TooLarge(total.max(0) as u64));
    }
    let origin = start.as_vec3() * h;
    let blocks = (n + IVec3::splat(BLOCK - 1)) / BLOCK;
    let per_block = (BLOCK * BLOCK * BLOCK) as u64;
    // Blocks the octree skipped although the surface reaches into them
    // (distance bounds of warped or noisy items are estimates) are found
    // through the quads that need them, sampled, and the quads rebuilt.
    let mut done: BTreeMap<(i32, i32, i32), Block> = BTreeMap::new();
    let mut queue = active_blocks(sdf, origin, h, blocks);
    let mut rounds = 0;
    let (positions, corners, unresolved) = loop {
        queue.retain(|b| b.cmpge(IVec3::ZERO).all() && b.cmplt(blocks).all() && !done.contains_key(&(b.z, b.y, b.x)));
        queue.sort_unstable_by_key(|b| (b.z, b.y, b.x));
        queue.dedup();
        let sampled = (done.len() + queue.len()) as u64 * per_block;
        if sampled > MAX_SAMPLED_VOXELS {
            return Err(SdfError::TooLarge(sampled));
        }
        let fresh: Vec<Block> = queue.par_iter().map(|&b| sample(sdf, origin, h, b)).collect();
        for (k, b) in queue.iter().zip(fresh) {
            done.insert((k.z, k.y, k.x), b);
        }
        let (positions, corners, missing) = assemble(&done);
        if missing.is_empty() || rounds >= MAX_GROW {
            break (positions, corners, missing.len());
        }
        queue = missing.iter().map(|v| IVec3::new(v.x.div_euclid(BLOCK), v.y.div_euclid(BLOCK), v.z.div_euclid(BLOCK))).collect();
        rounds += 1;
    };
    let sampled = done.len() as u64 * per_block;
    if corners.is_empty() {
        return Err(SdfError::Empty);
    }
    let sizes = vec![4u32; corners.len() / 4];
    // The pieces make a 2-manifold by construction; repair stays as a
    // safety net (vertices are told apart by index, never welded by position).
    let (mesh, report) = match Mesh::from_polygons(positions.clone(), &sizes, &corners) {
        Ok(m) => (m, SoupReport::default()),
        Err(e) => {
            tracing::debug!("surface nets gave a non-manifold mesh ({e}); repairing");
            let r = repair_by_index(&positions, &sizes, &corners)?;
            let mut m = r.mesh;
            close_cracks(&mut m);
            (m, r.report)
        }
    };
    let mut mesh = mesh;
    // Pinched cells can also place two vertices on one point.
    separate_pinches(&mut mesh, h);
    let specks = drop_specks(&mut mesh, h);
    if mesh.face_count() == 0 {
        return Err(SdfError::Empty);
    }
    let face_leaf = mesh
        .faces()
        .map(|f| {
            let c = mesh.face_verts(f).map(|v| mesh.positions[v as usize]).sum::<Vec3>() / mesh.face_len(f) as f32;
            sdf.leaf(c)
        })
        .collect();
    Ok(SdfMesh { mesh, face_leaf, repair: report, sampled, voxel: h, unresolved, specks })
}

/// Removes closed pieces holding less than half a voxel of volume (or
/// facing inward): bits of surface smaller than the grid can show, whose
/// vertices may even turn over. Returns how many went.
///
/// Complexity: O(F + C).
fn drop_specks(mesh: &mut Mesh, h: f32) -> usize {
    let (label, count) = mesh.face_components();
    let mut vol = vec![0.0f64; count as usize];
    for f in mesh.faces() {
        for t in mesh.face_triangles(f) {
            let [a, b, c] = t.map(|x| mesh.positions[mesh.origin(x) as usize].as_dvec3());
            vol[label[f as usize] as usize] += a.dot(b.cross(c)) / 6.0;
        }
    }
    let limit = 0.5 * f64::from(h).powi(3);
    let small: Vec<u32> = mesh.faces().filter(|&f| vol[label[f as usize] as usize] < limit).collect();
    if small.is_empty() || mesh.delete_faces(&small).is_err() {
        return 0;
    }
    vol.iter().filter(|&&v| v < limit).count()
}

/// Pulls apart vertices that pinched cells (or the repair's copies of a
/// pinched vertex) put on one point: they sit an ulp apart, and the edges between them would read
/// as degenerate (they break normals, UVs and bakes). Each copy moves a
/// twentieth of a voxel toward the center of its own other neighbours.
/// Returns how many vertices moved.
///
/// Complexity: O(E) plus the neighbours of the moved vertices.
fn separate_pinches(mesh: &mut Mesh, h: f32) -> usize {
    let tiny = (h * 1e-3).max(1e-6);
    let short: Vec<u32> = mesh.edges().filter(|&e| mesh.edge_length(e) < tiny).collect();
    let mut moves: Vec<(u32, Vec3)> = Vec::with_capacity(short.len() * 2);
    for e in short {
        let [a, b] = mesh.edge_verts(e);
        for (v, other) in [(a, b), (b, a)] {
            let p = mesh.positions[v as usize];
            let (sum, n) =
                mesh.vert_neighbors(v).filter(|&u| u != other).fold((Vec3::ZERO, 0u32), |(s, n), u| (s + mesh.positions[u as usize], n + 1));
            if n > 0 {
                let to = (sum / n as f32 - p).normalize_or_zero();
                moves.push((v, p + to * h * 0.05));
            }
        }
    }
    for &(v, p) in &moves {
        mesh.positions[v as usize] = p;
    }
    moves.len()
}

/// Most edges of a crack the mesher closes with a polygon.
pub const MAX_CRACK_EDGES: usize = 16;

/// Closes the small boundary loops repair leaves where a sharp rim pinched
/// the surface (one polygon each, up to [`MAX_CRACK_EDGES`] edges).
///
/// Complexity: O(H) to find the loops plus their lengths.
fn close_cracks(mesh: &mut Mesh) {
    let mut seen = vec![false; mesh.he_slots()];
    let mut starts = Vec::new();
    for h in 0..mesh.he_slots() as u32 {
        if seen[h as usize] || !mesh.is_he_alive(h) || !mesh.is_boundary_he(h) {
            continue;
        }
        let Ok(lp) = mesh.boundary_loop(h) else { continue };
        for &x in &lp {
            seen[x as usize] = true;
        }
        if lp.len() <= MAX_CRACK_EDGES {
            starts.push(h);
        }
    }
    for h in starts {
        // A loop that cannot close (it would break the surface) stays open.
        let _ = mesh.fill_boundary_loop(h);
    }
}

/// Soup repair on vertex indices: positions are made unique first so no
/// two vertices weld.
///
/// Complexity: O(V + C) expected.
fn repair_by_index(positions: &[Vec3], sizes: &[u32], corners: &[u32]) -> Result<p3d_geo::soup::Repaired, SdfError> {
    // Nudge exact duplicates by an ulp so the exact-position weld leaves them apart.
    let mut seen: FxHashMap<[u32; 3], u32> = FxHashMap::default();
    let unique: Vec<Vec3> = positions
        .iter()
        .map(|p| {
            let mut q = *p;
            while seen.insert(q.to_array().map(f32::to_bits), 0).is_some() {
                q.x = f32::from_bits(q.x.to_bits() + 1);
            }
            q
        })
        .collect();
    repair(&unique, sizes, corners, &SoupSpec::default()).map_err(|e| SdfError::Invalid { path: "mesh".into(), message: e.to_string() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::tree::{Combine, Item, Prim};

    fn closed(m: &Mesh) -> bool {
        m.edges().all(|e| !m.is_boundary_edge(e))
    }

    fn volume(m: &Mesh) -> f64 {
        m.faces()
            .flat_map(|f| m.face_triangles(f))
            .map(|t| {
                let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn surfaces_touching_grid_planes_leave_no_tiny_edges() {
        // Poles exactly on grid planes, where several cells' vertices meet.
        for (r, h) in [(0.5f32, 0.05f32), (0.23, 0.0025), (0.3, 0.01)] {
            let s = Sdf::new(vec![Item::prim(Prim::Sphere { radius: r }, Vec3::ZERO)]).unwrap();
            let m = mesh(&s, &MeshSpec { voxel: h }).unwrap().mesh;
            let shortest = m.edges().map(|e| m.edge_length(e)).fold(f32::INFINITY, f32::min);
            assert!(shortest > (h * 1e-3).max(1e-6), "r {r} h {h}: shortest edge {shortest:e}");
        }
    }

    #[test]
    fn sphere_becomes_a_closed_quad_mesh() {
        let s = Sdf::new(vec![Item::prim(Prim::Sphere { radius: 0.5 }, Vec3::new(0.1, 0.2, 0.3))]).unwrap();
        let r = mesh(&s, &MeshSpec { voxel: 0.04 }).unwrap();
        let m = &r.mesh;
        assert_eq!(m.validate(), Ok(()));
        assert!(closed(m));
        assert!(m.faces().all(|f| m.face_len(f) == 4));
        let v = volume(m);
        let want = 4.0 / 3.0 * std::f64::consts::PI * 0.125;
        assert!((v - want).abs() / want < 0.02, "{v} vs {want}");
        // Vertices sit on the sphere.
        for v in m.verts() {
            let d = (m.positions[v as usize] - Vec3::new(0.1, 0.2, 0.3)).length() - 0.5;
            assert!(d.abs() < 0.004, "{d}");
        }
        assert!(r.repair.is_clean());
        // On a finer grid only blocks near the surface are sampled.
        let fine = mesh(&s, &MeshSpec { voxel: 0.005 }).unwrap();
        let grid = (1.0f64 / 0.005 + 4.0).powi(3) as u64;
        assert!(fine.sampled * 3 < grid, "{} of {grid}", fine.sampled);
    }

    #[test]
    fn faces_know_their_item() {
        let s = Sdf::new(vec![
            Item::prim(Prim::Sphere { radius: 0.3 }, Vec3::ZERO).named("head"),
            Item::prim(Prim::Sphere { radius: 0.15 }, Vec3::new(0.35, 0.0, 0.0)).named("nose").with(Combine::SmoothUnion(0.08)),
        ])
        .unwrap();
        let r = mesh(&s, &MeshSpec { voxel: 0.02 }).unwrap();
        assert!(closed(&r.mesh));
        let nose = r.face_leaf.iter().filter(|&&l| l == 1).count();
        let head = r.face_leaf.iter().filter(|&&l| l == 0).count();
        assert!(nose > 20 && head > nose, "{head} {nose}");
    }

    #[test]
    fn results_repeat_exactly() {
        let s = Sdf::new(vec![Item::prim(Prim::Torus { major: 0.4, minor: 0.12 }, Vec3::ZERO)]).unwrap();
        let a = mesh(&s, &MeshSpec { voxel: 0.03 }).unwrap();
        let b = mesh(&s, &MeshSpec { voxel: 0.03 }).unwrap();
        assert_eq!(a.mesh.positions, b.mesh.positions);
        assert!(closed(&a.mesh));
        assert_eq!(a.mesh.face_count(), b.mesh.face_count());
    }

    #[test]
    fn bad_sizes_are_refused() {
        let s = Sdf::new(vec![Item::prim(Prim::Sphere { radius: 1.0 }, Vec3::ZERO)]).unwrap();
        assert!(matches!(mesh(&s, &MeshSpec { voxel: 0.0 }), Err(SdfError::Invalid { .. })));
        assert!(matches!(mesh(&s, &MeshSpec { voxel: 1e-4 }), Err(SdfError::TooLarge(_))));
    }
}
