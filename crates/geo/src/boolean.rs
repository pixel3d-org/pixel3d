//! Mesh booleans: union, difference and intersection of closed meshes
//! through a replaceable [`Engine`] (manifold-rust's exact engine, ADR 0008),
//! followed by the cleanup game assets need.
//!
//! The engine works on triangles and returns triangles. Every input triangle
//! carries the id of the polygon it came from, so the result can be rebuilt
//! the way an artist expects: the triangles of one source polygon merge back
//! into one polygon (an n-gon where the cut trimmed it), pieces with holes
//! join into quads where they can, corner attributes (UVs, colors) are
//! interpolated in the source triangle, and materials, sharp and seam flags,
//! creases and the target's groups carry over. Vertices the cut created that
//! sit closer than the weld distance merge, so no sliver edges remain.

use glam::{DVec2, DVec3, Vec2, Vec3, Vec4};
use p3d_mesh::{CornerSrc, EdgeFlags, ElementKind, MAX_EAR_CLIP_CORNERS, Mesh, Patch};
use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::edit::{Region, merge_by_distance, regions, simple_polygons};
use crate::generate::shape::{convex_merge, fill_tris};

/// Largest number of input triangles (target plus tools) one boolean takes.
pub const MAX_BOOLEAN_TRIS: usize = 4_000_000;

/// Neighbouring source polygons bent more than this (degrees) across a
/// smooth edge make a curved surface (see [`BooleanResult::curved`]).
const CURVED_DEG: f64 = 5.0;

/// Internal name of the group that tracks the tools' faces through the
/// cleanup patches; never left on a result.
const CUT_GROUP: &str = "\u{0}boolean.cut";

/// What to do with the volumes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BoolOp {
    /// Everything inside either.
    #[default]
    Union,
    /// The target without what is inside the tools.
    Difference,
    /// Only what is inside both.
    Intersect,
}

/// Closed triangles as an engine sees them: positions in f64, three vertex
/// indices per triangle (counter-clockwise from outside) and a caller-chosen
/// id per triangle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tris {
    pub positions: Vec<DVec3>,
    pub tris: Vec<[u32; 3]>,
    /// One id per triangle. An engine gives every output triangle the id of
    /// the input triangle it lies in.
    pub source: Vec<u32>,
}

/// Why a boolean failed. The meshes are unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BooleanError {
    /// An operand has open edges, so it has no inside.
    #[error("{} is not closed ({open} open edge{})", operand_name(*.operand), if *.open == 1 { "" } else { "s" })]
    NotClosed {
        /// 0 for the target, 1 + i for tool i.
        operand: usize,
        open: usize,
    },
    /// Too many triangles for one boolean.
    #[error("{0} triangles is more than one boolean takes ({MAX_BOOLEAN_TRIS})")]
    TooLarge(usize),
    /// The engine refused or returned something that is not a closed mesh.
    #[error("the boolean engine failed: {0}")]
    Engine(String),
    /// Nothing is left (a difference that removes everything, or an
    /// intersection of volumes that do not overlap).
    #[error("the result is empty")]
    Empty,
}

fn operand_name(i: usize) -> String {
    match i {
        0 => "the target".to_owned(),
        i => format!("tool {i}"),
    }
}

/// A boolean engine: combines closed triangle meshes.
pub trait Engine {
    /// `((target op tools[0]) op tools[1]) ...`, keeping every triangle's
    /// source id.
    fn run(&self, target: &Tris, tools: &[Tris], op: BoolOp) -> Result<Tris, BooleanError>;
}

/// manifold-rust's exact engine (ADR 0008).
#[derive(Debug, Clone, Copy, Default)]
pub struct ManifoldEngine;

impl ManifoldEngine {
    fn import(t: &Tris) -> Result<manifold_rust::manifold::Manifold, BooleanError> {
        use manifold_rust::manifold::Manifold;
        use manifold_rust::types::{Error, MeshGL64};
        let mesh = MeshGL64 {
            num_prop: 3,
            vert_properties: t.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
            tri_verts: t.tris.iter().flat_map(|t| t.map(u64::from)).collect(),
            face_id: t.source.iter().map(|&s| u64::from(s)).collect(),
            ..MeshGL64::default()
        };
        let m = Manifold::from_mesh_gl64(&mesh);
        match m.status() {
            Error::NoError => Ok(m),
            e => Err(BooleanError::Engine(format!("an operand was refused: {e:?}"))),
        }
    }
}

impl Engine for ManifoldEngine {
    /// Complexity: O((n + k) log n) for n input triangles and k
    /// intersections (manifold-rust's collider and exact predicates).
    fn run(&self, target: &Tris, tools: &[Tris], op: BoolOp) -> Result<Tris, BooleanError> {
        use manifold_rust::types::{BooleanEngine, Error, OpType};
        let kind = match op {
            BoolOp::Union => OpType::Add,
            BoolOp::Difference => OpType::Subtract,
            BoolOp::Intersect => OpType::Intersect,
        };
        let mut acc = Self::import(target)?;
        for t in tools {
            acc = acc.boolean_with_engine(&Self::import(t)?, kind, BooleanEngine::Exact);
            if acc.status() != Error::NoError {
                return Err(BooleanError::Engine(format!("{:?}", acc.status())));
            }
        }
        let g = acc.get_mesh_gl64(-1);
        let n = (g.num_prop as usize).max(3);
        let positions = g.vert_properties.chunks_exact(n).map(|c| DVec3::new(c[0], c[1], c[2])).collect();
        let tris = g.tri_verts.as_chunks::<3>().0.iter().map(|c| c.map(|i| i as u32)).collect();
        let source = g.face_id.iter().map(|&s| s as u32).collect();
        Ok(Tris { positions, tris, source })
    }
}

/// Options of [`boolean`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BooleanSpec {
    pub op: BoolOp,
    /// Vertices the cut created merge with neighbours closer than this
    /// (removes sliver edges); 0 keeps them all.
    pub weld: f32,
}

impl Default for BooleanSpec {
    fn default() -> Self {
        BooleanSpec { op: BoolOp::Union, weld: 1e-4 }
    }
}

/// What [`boolean`] made.
#[derive(Debug, Clone, Default)]
pub struct BooleanResult {
    pub mesh: Mesh,
    /// Faces that came from the tools (the walls of a hole, the added
    /// volume of a union).
    pub cut: Vec<u32>,
    /// Edges where the tools' faces meet the target's.
    pub edges: Vec<u32>,
    /// Whether anything changed: false when the tools miss the target in a
    /// difference (the result is the target as it was).
    pub touched: bool,
    /// Target polygons the cut crossed that sit on a smooth curved surface
    /// (a neighbour across a smooth edge is bent more than 5 degrees). Cuts
    /// there show the facets of a low-poly mesh.
    pub curved: usize,
    /// Whether the target's custom split normals were dropped (they no
    /// longer fit the cut surface).
    pub normals_cleared: bool,
}

/// The source polygon of an input triangle and its corners there.
#[derive(Debug, Clone, Copy)]
struct Src {
    /// 0 for the target, 1 + i for tool i.
    operand: u32,
    face: u32,
    corners: [u32; 3],
}

impl Src {
    fn poly(&self) -> u64 {
        (u64::from(self.operand) << 32) | u64::from(self.face)
    }
}

/// The triangles of one operand for the engine, appending their sources.
///
/// Complexity: O(V + C) plus the triangulation of each face.
fn operand_tris(mesh: &Mesh, operand: u32, srcs: &mut Vec<Src>) -> Result<Tris, BooleanError> {
    let open = mesh.edges().filter(|&e| mesh.is_boundary_edge(e)).count();
    if open > 0 {
        return Err(BooleanError::NotClosed { operand: operand as usize, open });
    }
    let mut map = vec![u32::MAX; mesh.vert_slots()];
    let mut positions = Vec::with_capacity(mesh.vert_count());
    for v in mesh.verts() {
        map[v as usize] = positions.len() as u32;
        positions.push(mesh.positions[v as usize].as_dvec3());
    }
    let mut out = Tris { positions, ..Tris::default() };
    for f in mesh.faces() {
        for t in mesh.face_triangles(f) {
            if srcs.len() >= MAX_BOOLEAN_TRIS {
                return Err(BooleanError::TooLarge(srcs.len() + 1));
            }
            out.tris.push(t.map(|h| map[mesh.origin(h) as usize]));
            out.source.push(srcs.len() as u32);
            srcs.push(Src { operand, face: f, corners: t });
        }
    }
    Ok(out)
}

/// Barycentric weights of `p` in triangle `abc`, clamped to the triangle.
fn barycentric(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> [f64; 3] {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let (d20, d21) = (v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() <= f64::EPSILON * d00 * d11 {
        return [1.0, 0.0, 0.0];
    }
    let v = ((d11 * d20 - d01 * d21) / den).max(0.0);
    let w = ((d00 * d21 - d01 * d20) / den).max(0.0);
    let u = (1.0 - v - w).max(0.0);
    let s = u + v + w;
    if s > 0.0 { [u / s, v / s, w / s] } else { [1.0, 0.0, 0.0] }
}

/// Distance from `p` to segment `ab`.
fn segment_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn position_key(p: Vec3) -> [u32; 3] {
    p.to_array().map(f32::to_bits)
}

/// Combines `target` with each tool (all in the target's space) and cleans
/// the result up (see the module docs).
///
/// Complexity: the engine's O((n + k) log n) for n triangles and k
/// intersections, plus O(n log n) for the cleanup (region merges, quad
/// matching and welding are each O(n log n) or better).
pub fn boolean(target: &Mesh, tools: &[&Mesh], spec: &BooleanSpec, engine: &dyn Engine) -> Result<BooleanResult, BooleanError> {
    let operands: Vec<&Mesh> = std::iter::once(target).chain(tools.iter().copied()).collect();
    let mut srcs = Vec::new();
    let input = operands.iter().enumerate().map(|(i, m)| operand_tris(m, i as u32, &mut srcs)).collect::<Result<Vec<_>, _>>()?;
    let target_tris = input.first().map_or(0, |t| t.tris.len());
    let out = engine.run(&input[0], &input[1..], spec.op)?;
    if out.tris.is_empty() {
        return Err(BooleanError::Empty);
    }
    if out.source.len() != out.tris.len() || out.source.iter().any(|&s| s as usize >= srcs.len()) {
        return Err(BooleanError::Engine("the result lost its source ids".into()));
    }
    let from_tools = out.source.iter().filter(|&&s| srcs[s as usize].operand != 0).count();
    if from_tools == 0 && out.tris.len() == target_tris {
        return Ok(BooleanResult { mesh: target.clone(), touched: false, ..BooleanResult::default() });
    }
    let mut mesh = rebuild(&out)?;
    let fsrc: Vec<Src> = out.source.iter().map(|&s| srcs[s as usize]).collect();
    transfer_corners(&mut mesh, &operands, &out, &fsrc);
    let cut_edges = transfer_edges(&mut mesh, &operands, &fsrc);
    transfer_groups(&mut mesh, target, &fsrc)?;
    let curved = curved_polys(target, &mesh, &fsrc, &cut_edges);
    let normals_cleared = target.corner_normal.is_some();

    let new_verts: FxHashSet<u32> = new_vertices(&mesh, &operands);
    rebuild_polygons(&mut mesh, &fsrc, &new_verts)?;
    if spec.weld > 0.0 {
        let mut near: Vec<u32> = new_verts.iter().copied().filter(|&v| mesh.is_vert_alive(v)).collect();
        near.extend(near.clone().into_iter().flat_map(|v| mesh.vert_neighbors(v).collect::<SmallVec<[u32; 8]>>()));
        near.sort_unstable();
        near.dedup();
        // A weld that would break the surface is skipped; the slivers stay.
        let _ = merge_by_distance(&mut mesh, Some(&near), spec.weld);
    }
    drop_flat_pieces(&mut mesh, spec.weld)?;
    // A sliver thinner than the weld distance collapses completely.
    if mesh.face_count() == 0 {
        return Err(BooleanError::Empty);
    }
    // Sorted: a group lists its faces in index order.
    let cut: Vec<u32> = mesh.group(CUT_GROUP).map(|g| g.iter().filter(|&f| mesh.is_face_alive(f)).collect()).unwrap_or_default();
    mesh.remove_group(CUT_GROUP);
    let in_cut = |f: Option<u32>| f.is_some_and(|f| cut.binary_search(&f).is_ok());
    let edges = mesh.edges().filter(|&e| in_cut(mesh.face(2 * e)) != in_cut(mesh.face(2 * e + 1))).collect();
    Ok(BooleanResult { mesh, cut, edges, touched: true, curved, normals_cleared })
}

/// Removes pieces thinner on average than half the weld distance (volume
/// over area), such as the zero-volume pair of faces the engine can leave
/// where two operands share a face exactly.
///
/// Complexity: O(F + C).
fn drop_flat_pieces(mesh: &mut Mesh, weld: f32) -> Result<(), BooleanError> {
    let (label, count) = mesh.face_components();
    let mut volume = vec![0.0f64; count as usize];
    let mut area = vec![0.0f64; count as usize];
    for f in mesh.faces() {
        let c = label[f as usize] as usize;
        for t in mesh.face_triangles(f) {
            let [a, b, d] = t.map(|h| mesh.positions[mesh.origin(h) as usize].as_dvec3());
            volume[c] += a.dot(b.cross(d)) / 6.0;
            area[c] += (b - a).cross(d - a).length() / 2.0;
        }
    }
    let limit = f64::from(weld.max(1e-7)) * 0.5;
    let flat: Vec<u32> = mesh.faces().filter(|&f| volume[label[f as usize] as usize].abs() <= limit * area[label[f as usize] as usize]).collect();
    if !flat.is_empty() {
        mesh.delete_faces(&flat).map_err(|e| BooleanError::Engine(e.to_string()))?;
    }
    Ok(())
}

/// How one region (the connected faces of one source polygon) is rebuilt:
/// kept, or replaced by polygons given as (vertex, corner source) lists.
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    Keep,
    Polys(Vec<Vec<(u32, u32)>>),
}

/// Rebuilds a region without the `drop` vertices: one polygon over a
/// single border loop, or a fill of convex polygons over the plane region
/// inside several loops (a face the cut punched a hole through). `None` when that
/// is not possible with these drops.
///
/// Complexity: O(c log c) for the c corners of the region.
fn plan(mesh: &Mesh, (members, loops): &Region, drop: &FxHashSet<u32>) -> Option<Plan> {
    let kept: Vec<Vec<u32>> = loops.iter().map(|l| l.iter().copied().filter(|&h| !drop.contains(&mesh.origin(h))).collect()).collect();
    let dropped = kept.iter().zip(loops).any(|(k, l)| k.len() != l.len());
    if members.len() == 1 && !dropped {
        return Some(Plan::Keep);
    }
    if kept.iter().any(|l| l.len() < 3) {
        return None;
    }
    if let [l] = kept.as_slice()
        && l.len() <= MAX_EAR_CLIP_CORNERS
    {
        let pieces = simple_polygons(&l.iter().map(|&h| (mesh.origin(h), h)).collect::<Vec<_>>());
        return (!pieces.is_empty()).then_some(Plan::Polys(pieces));
    }
    let n = members.iter().map(|&f| mesh.face_normal_area2(f).as_dvec3()).sum::<DVec3>().normalize_or_zero();
    if n == DVec3::ZERO {
        return None;
    }
    let u = n.any_orthonormal_vector();
    let w = n.cross(u);
    let corners: Vec<u32> = kept.iter().flatten().copied().collect();
    let contours: Vec<Vec<DVec2>> = kept
        .iter()
        .map(|l| {
            l.iter()
                .map(|&h| {
                    let p = mesh.positions[mesh.origin(h) as usize].as_dvec3();
                    DVec2::new(p.dot(u), p.dot(w))
                })
                .collect()
        })
        .collect();
    let tris = fill_tris(&contours).ok()?;
    let polys = convex_merge(&contours.concat(), &tris);
    Some(Plan::Polys(polys.into_iter().map(|p| p.into_iter().map(|i| (mesh.origin(corners[i as usize]), corners[i as usize])).collect()).collect()))
}

/// Merges the result triangles of each source polygon back into polygons,
/// refills polygons with holes quad-dominantly, and drops vertices the cut
/// left on straight edges (where it crossed a source triangle's diagonal).
///
/// A vertex is only dropped when both regions around it can be rebuilt
/// without it; a region that cannot keeps its faces and its vertices.
///
/// Complexity: O(c log c) for the c corners of the mesh (each failed plan
/// releases at least one vertex, which happens a handful of times at most).
fn rebuild_polygons(mesh: &mut Mesh, fsrc: &[Src], new_verts: &FxHashSet<u32>) -> Result<(), BooleanError> {
    let poly: Vec<u64> = fsrc.iter().map(Src::poly).collect();
    let all: Vec<u32> = mesh.faces().collect();
    let regs = regions(mesh, &all, |e| match (mesh.face(2 * e), mesh.face(2 * e + 1)) {
        (Some(a), Some(b)) => poly[a as usize] == poly[b as usize],
        _ => false,
    });
    let scale = mesh.bounds().map_or(1.0, |(lo, hi)| f64::from((hi - lo).length()).max(1e-6));
    let mut drop = straight_cut_verts(mesh, &poly, new_verts, 1e-5 * scale);
    let mut at: FxHashMap<u32, SmallVec<[usize; 2]>> = FxHashMap::default();
    for (i, r) in regs.iter().enumerate() {
        for &h in r.1.iter().flatten() {
            if drop.contains(&mesh.origin(h)) {
                at.entry(mesh.origin(h)).or_default().push(i);
            }
        }
    }
    let mut plans = vec![Plan::Keep; regs.len()];
    let mut queued = vec![true; regs.len()];
    let mut queue: Vec<usize> = (0..regs.len()).rev().collect();
    while let Some(i) = queue.pop() {
        queued[i] = false;
        if let Some(p) = plan(mesh, &regs[i], &drop) {
            plans[i] = p;
            continue;
        }
        plans[i] = Plan::Keep;
        // Keep this region's vertices and plan its neighbours (and itself) again.
        for &h in regs[i].1.iter().flatten() {
            let v = mesh.origin(h);
            if drop.remove(&v) {
                for &j in at.get(&v).into_iter().flatten() {
                    if !queued[j] {
                        queued[j] = true;
                        queue.push(j);
                    }
                }
            }
        }
    }
    let mut p = Patch::new(mesh);
    for ((members, _), plan) in regs.iter().zip(&plans) {
        let Plan::Polys(polys) = plan else { continue };
        for &f in members {
            p.remove_face(f);
        }
        for poly in polys {
            let vs: Vec<u32> = poly.iter().map(|c| c.0).collect();
            let src: Vec<CornerSrc> = poly.iter().map(|c| CornerSrc::copy(c.1)).collect();
            p.add_face(&vs, &src, mesh.face_material(members[0]), Some(members[0]));
        }
    }
    if !p.is_empty() {
        mesh.apply_patch(&p).map_err(|e| BooleanError::Engine(format!("cannot rebuild the polygons: {e}")))?;
    }
    Ok(())
}

/// Cut vertices in the middle of a straight edge chain: exactly two of
/// their edges part different source polygons, and those two are in line.
///
/// Complexity: O(sum of their valences).
fn straight_cut_verts(mesh: &Mesh, poly: &[u64], new_verts: &FxHashSet<u32>, tol: f64) -> FxHashSet<u32> {
    let parts = |h: u32| match (mesh.face(h), mesh.face(h ^ 1)) {
        (Some(a), Some(b)) => poly[a as usize] != poly[b as usize],
        _ => true,
    };
    new_verts
        .iter()
        .copied()
        .filter(|&v| {
            let mut ends: SmallVec<[u32; 2]> = SmallVec::new();
            for h in mesh.vert_outgoing(v) {
                if parts(h) {
                    if ends.len() == 2 {
                        return false;
                    }
                    ends.push(mesh.dest(h));
                }
            }
            let [a, b] = match ends.as_slice() {
                [a, b] => [*a, *b].map(|x| mesh.positions[x as usize].as_dvec3()),
                _ => return false,
            };
            let p = mesh.positions[v as usize].as_dvec3();
            (a - p).dot(b - p) < 0.0 && segment_distance(p, a, b) <= tol
        })
        .collect()
}

/// The engine's triangles as a mesh (face i is triangle i), without
/// unused vertices.
///
/// Complexity: O(V + n).
fn rebuild(out: &Tris) -> Result<Mesh, BooleanError> {
    let mut map = vec![u32::MAX; out.positions.len()];
    let mut positions = Vec::new();
    let mut corners = Vec::with_capacity(out.tris.len() * 3);
    for t in &out.tris {
        for &v in t {
            let slot = map.get_mut(v as usize).ok_or_else(|| BooleanError::Engine("a triangle uses a missing vertex".into()))?;
            if *slot == u32::MAX {
                *slot = positions.len() as u32;
                positions.push(out.positions[v as usize].as_vec3());
            }
            corners.push(*slot);
        }
    }
    Mesh::from_polygons(positions, &vec![3; out.tris.len()], &corners)
        .map_err(|e| BooleanError::Engine(format!("the result is not a closed surface: {e}")))
}

/// Materials per face and UVs and colors per corner, interpolated in each
/// result triangle's source triangle. Custom normals are dropped.
///
/// Complexity: O(n) for n result triangles.
fn transfer_corners(mesh: &mut Mesh, operands: &[&Mesh], out: &Tris, fsrc: &[Src]) {
    let layers = operands[0].uv_layer_count();
    let colors = operands.iter().any(|m| m.corner_color.is_some());
    let mut uvs: Vec<Vec<Vec2>> = vec![Vec::with_capacity(3 * out.tris.len()); layers];
    let mut cols: Vec<Vec4> = Vec::with_capacity(if colors { 3 * out.tris.len() } else { 0 });
    for (i, (t, s)) in out.tris.iter().zip(fsrc).enumerate() {
        let m = operands[s.operand as usize];
        let at = |h: u32| m.positions[m.origin(h) as usize].as_dvec3();
        let [a, b, c] = s.corners.map(at);
        for &v in t {
            let w = barycentric(out.positions[v as usize], a, b, c).map(|x| x as f32);
            for (l, column) in uvs.iter_mut().enumerate() {
                let uv = |k: usize| m.corner_uv(l, s.corners[k]).unwrap_or(Vec2::ZERO);
                column.push(uv(0) * w[0] + uv(1) * w[1] + uv(2) * w[2]);
            }
            if colors {
                let col = |k: usize| m.corner_color(s.corners[k]).unwrap_or(Vec4::ONE);
                cols.push(col(0) * w[0] + col(1) * w[1] + col(2) * w[2]);
            }
        }
        mesh.face_material[i] = m.face_material(s.face);
    }
    for column in &uvs {
        mesh.add_uv_layer_from_polygon_order(column);
    }
    if colors {
        let mut column = vec![Vec4::ONE; mesh.he_slots()];
        mesh.corner_values_from_polygon_order(&mut column, &cols);
        mesh.corner_color = Some(column);
    }
    mesh.corner_normal = None;
}

/// Sharp and seam flags, creases and the target's edge groups on result
/// edges that lie on a source edge carrying any of them. Returns the edges
/// where faces of different operands meet.
///
/// Complexity: O(E) expected (one hash lookup per edge between two source
/// polygons), plus O(E x G) for the target's G edge groups.
fn transfer_edges(mesh: &mut Mesh, operands: &[&Mesh], fsrc: &[Src]) -> Vec<u32> {
    let edge_groups: Vec<(&str, &p3d_mesh::Group)> = operands[0].groups().filter(|(_, g)| g.kind() == ElementKind::Edge).collect();
    // Source edges that carry something, by the pair of polygons they part.
    let mut carried: FxHashMap<(u32, u32, u32), SmallVec<[u32; 2]>> = FxHashMap::default();
    for (k, m) in operands.iter().enumerate() {
        for e in m.edges() {
            let grouped = k == 0 && edge_groups.iter().any(|(_, g)| g.contains(e));
            if m.edge_flags(e) == EdgeFlags::NONE && m.edge_crease(e) == 0.0 && !grouped {
                continue;
            }
            if let (Some(a), Some(b)) = (m.face(2 * e), m.face(2 * e + 1)) {
                carried.entry((k as u32, a.min(b), a.max(b))).or_default().push(e);
            }
        }
    }
    let scale = mesh.bounds().map_or(1.0, |(lo, hi)| f64::from((hi - lo).length()).max(1e-6));
    let tol = 1e-5 * scale;
    let creases = operands.iter().any(|m| m.edge_crease.is_some());
    if creases {
        mesh.edge_crease = Some(vec![0.0; mesh.edge_slots()]);
    }
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); edge_groups.len()];
    let mut crossing = Vec::new();
    let result_edges: Vec<u32> = mesh.edges().collect();
    for e in result_edges {
        let (Some(fa), Some(fb)) = (mesh.face(2 * e), mesh.face(2 * e + 1)) else { continue };
        let (sa, sb) = (fsrc[fa as usize], fsrc[fb as usize]);
        if sa.operand != sb.operand {
            crossing.push(e);
            continue;
        }
        if sa.face == sb.face {
            continue;
        }
        let Some(list) = carried.get(&(sa.operand, sa.face.min(sb.face), sa.face.max(sb.face))) else { continue };
        let m = operands[sa.operand as usize];
        let [p, q] = mesh.edge_verts(e).map(|v| mesh.positions[v as usize].as_dvec3());
        for &se in list {
            let [a, b] = m.edge_verts(se).map(|v| m.positions[v as usize].as_dvec3());
            if segment_distance(p, a, b) > tol || segment_distance(q, a, b) > tol {
                continue;
            }
            let flags = EdgeFlags::from_bits(mesh.edge_flags(e).bits() | m.edge_flags(se).bits());
            mesh.edge_flags[e as usize] = flags;
            if let Some(c) = mesh.edge_crease.as_mut() {
                c[e as usize] = c[e as usize].max(m.edge_crease(se));
            }
            if sa.operand == 0 {
                for (i, (_, g)) in edge_groups.iter().enumerate() {
                    if g.contains(se) {
                        members[i].push(e);
                    }
                }
            }
        }
    }
    let names: Vec<String> = edge_groups.iter().map(|(n, _)| (*n).to_owned()).collect();
    for (name, list) in names.iter().zip(members) {
        // Edges listed here are live result edges, so this cannot fail.
        let _ = mesh.set_group(name, ElementKind::Edge, list);
    }
    crossing
}

/// The target's face and vertex groups, plus the internal group of the
/// tools' faces.
///
/// Complexity: O((F + V) x G) for the target's G face and vertex groups.
fn transfer_groups(mesh: &mut Mesh, target: &Mesh, fsrc: &[Src]) -> Result<(), BooleanError> {
    let fail = |e: p3d_mesh::GroupError| BooleanError::Engine(e.to_string());
    let faces: Vec<u32> = mesh.faces().collect();
    for (name, g) in target.groups() {
        match g.kind() {
            ElementKind::Face => {
                let list = faces.iter().copied().filter(|&f| fsrc[f as usize].operand == 0 && g.contains(fsrc[f as usize].face));
                mesh.set_group(name, ElementKind::Face, list).map_err(fail)?;
            }
            ElementKind::Vert => {
                let by_pos: FxHashMap<[u32; 3], u32> = g.iter().map(|v| (position_key(target.positions[v as usize]), v)).collect();
                let list: Vec<u32> = mesh.verts().filter(|&v| by_pos.contains_key(&position_key(mesh.positions[v as usize]))).collect();
                mesh.set_group(name, ElementKind::Vert, list).map_err(fail)?;
            }
            ElementKind::Edge => {}
        }
    }
    let cut = faces.iter().copied().filter(|&f| fsrc[f as usize].operand != 0);
    mesh.set_group(CUT_GROUP, ElementKind::Face, cut).map_err(fail)?;
    Ok(())
}

/// Result vertices that are not at a source vertex: the cut made them.
///
/// Complexity: O(V) expected.
fn new_vertices(mesh: &Mesh, operands: &[&Mesh]) -> FxHashSet<u32> {
    let old: FxHashSet<[u32; 3]> = operands.iter().flat_map(|m| m.verts().map(|v| position_key(m.positions[v as usize]))).collect();
    mesh.verts().filter(|&v| !old.contains(&position_key(mesh.positions[v as usize]))).collect()
}

/// Target polygons next to the cut whose smooth neighbours bend away.
///
/// Complexity: O(cut edges + corners of the touched polygons).
fn curved_polys(target: &Mesh, mesh: &Mesh, fsrc: &[Src], cut_edges: &[u32]) -> usize {
    let mut touched: Vec<u32> = cut_edges
        .iter()
        .flat_map(|&e| [mesh.face(2 * e), mesh.face(2 * e + 1)])
        .flatten()
        .map(|f| fsrc[f as usize])
        .filter(|s| s.operand == 0)
        .map(|s| s.face)
        .collect();
    touched.sort_unstable();
    touched.dedup();
    let limit = CURVED_DEG.to_radians().cos();
    touched
        .iter()
        .filter(|&&f| {
            let n = target.face_normal(f).as_dvec3();
            target.face_halfedges(f).any(|h| {
                !target.edge_flags(h >> 1).contains(EdgeFlags::SHARP)
                    && target.face(h ^ 1).is_some_and(|g| n.dot(target.face_normal(g).as_dvec3()) < limit)
            })
        })
        .count()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::prim::{BoxSpec, CylinderSpec, Origin, cuboid};
    use glam::Affine3A;

    fn cube(size: f64) -> Mesh {
        cuboid(&BoxSpec { size: DVec3::splat(size), segments: [1; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center).unwrap()
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

    fn closed(m: &Mesh) -> bool {
        m.edges().all(|e| !m.is_boundary_edge(e))
    }

    fn moved(mut m: Mesh, by: Vec3) -> Mesh {
        m.transform(Affine3A::from_translation(by));
        m
    }

    fn run(a: &Mesh, b: &Mesh, op: BoolOp) -> BooleanResult {
        boolean(a, &[b], &BooleanSpec { op, ..BooleanSpec::default() }, &ManifoldEngine).unwrap()
    }

    #[test]
    fn offset_cubes_keep_quads_and_volumes() {
        let a = cube(2.0);
        let b = moved(cube(2.0), Vec3::splat(1.0));
        for (op, vol) in [(BoolOp::Union, 15.0), (BoolOp::Difference, 7.0), (BoolOp::Intersect, 1.0)] {
            let r = run(&a, &b, op);
            assert!(closed(&r.mesh), "{op:?}");
            assert!((volume(&r.mesh) - vol).abs() < 1e-4, "{op:?}: {}", volume(&r.mesh));
            // Every face is planar source material: no triangles survive.
            assert!(r.mesh.faces().all(|f| r.mesh.face_len(f) >= 4), "{op:?}");
            assert!(!r.cut.is_empty() && !r.edges.is_empty(), "{op:?}");
            assert!(r.touched);
        }
        // The difference is a cube with a corner notch: 3 L-shaped faces, 3
        // untouched quads, 3 walls of the notch.
        let d = run(&a, &b, BoolOp::Difference);
        assert_eq!(d.mesh.face_count(), 9);
        assert_eq!(d.cut.len(), 3);
        // Each wall meets the target along 2 edges.
        assert_eq!(d.edges.len(), 6);
    }

    #[test]
    fn missing_tool_leaves_the_target_alone() {
        let a = cube(1.0);
        let b = moved(cube(1.0), Vec3::splat(5.0));
        let r = run(&a, &b, BoolOp::Difference);
        assert!(!r.touched);
        assert_eq!(r.mesh.face_count(), 6);
        assert!(matches!(
            boolean(&a, &[&b], &BooleanSpec { op: BoolOp::Intersect, ..BooleanSpec::default() }, &ManifoldEngine),
            Err(BooleanError::Empty)
        ));
    }

    #[test]
    fn open_operands_are_refused() {
        let mut a = cube(1.0);
        let f = a.faces().next().unwrap();
        let mut p = p3d_mesh::Patch::new(&a);
        p.remove_face(f);
        a.apply_patch(&p).unwrap();
        let err = boolean(&a, &[&cube(1.0)], &BooleanSpec::default(), &ManifoldEngine).unwrap_err();
        assert_eq!(err, BooleanError::NotClosed { operand: 0, open: 4 });
        assert_eq!(err.to_string(), "the target is not closed (4 open edges)");
    }

    #[test]
    fn hole_through_a_box_keeps_groups_uvs_and_flags() {
        let mut a = cube(2.0);
        a.add_uv_layer();
        for h in 0..a.he_slots() as u32 {
            if a.is_he_alive(h) && a.face(h).is_some() {
                let p = a.positions[a.origin(h) as usize];
                a.corner_uv[0][h as usize] = Vec2::new(p.x, p.z);
            }
        }
        let top: Vec<u32> = a.faces().filter(|&f| a.face_normal(f).y > 0.5).collect();
        a.set_group("top", ElementKind::Face, top).unwrap();
        let sharp: Vec<u32> = a.edges().collect();
        for &e in &sharp {
            a.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
        let rod = crate::prim::cylinder(&CylinderSpec { radius: 0.5, height: 4.0, segments: 16, ..CylinderSpec::default() }, Origin::Center).unwrap();
        let r = run(&a, &rod, BoolOp::Difference);
        assert!(closed(&r.mesh));
        // A 16-sided prism of circumradius 0.5 through 2 m of box.
        let ring = 8.0 * 0.25 * (std::f64::consts::PI / 8.0).sin();
        assert!((volume(&r.mesh) - (8.0 - 2.0 * ring)).abs() < 1e-3, "{}", volume(&r.mesh));
        // The rod's 16 sides line the hole.
        assert_eq!(r.cut.len(), 16);
        assert_eq!(r.edges.len(), 32);
        // The top face (with a hole now) stays in its group as quads.
        let top = r.mesh.group("top").unwrap();
        assert!(top.count() >= 4);
        assert!(top.iter().all(|f| r.mesh.face_normal(f).y > 0.99));
        // UVs follow the planar mapping on the cut faces.
        for f in top.iter() {
            for h in r.mesh.face_halfedges(f) {
                let p = r.mesh.positions[r.mesh.origin(h) as usize];
                assert!((r.mesh.corner_uv(0, h).unwrap() - Vec2::new(p.x, p.z)).length() < 1e-4);
            }
        }
        // The box's 12 edges stay sharp; the cut and the rod's smooth sides carry nothing.
        let sharp = r.mesh.edges().filter(|&e| r.mesh.edge_flags(e).contains(EdgeFlags::SHARP)).count();
        assert_eq!(sharp, 12);
        assert_eq!(r.curved, 0);
    }
}
