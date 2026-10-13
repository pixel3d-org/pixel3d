//! Field-aligned quad remeshing of a closed surface: sample it densely,
//! solve the orientation and position fields ([`crate::field`]), collapse
//! vertices that share a lattice point, connect lattice neighbours, walk
//! the faces, and relax the result onto the original surface.

use glam::Vec3;
use p3d_geo::bvh::TriBvh;
use p3d_geo::soup::{SoupSpec, repair};
use p3d_geo::subd::{SubdSpec, catmull_clark};
use p3d_mesh::Mesh;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::field::{Level, compat_orientation, compat_position, hierarchy, orientation_field, position_field};

/// Most vertices of the dense sampling.
pub const MAX_SAMPLES: usize = 4_000_000;
/// Longest face the walk keeps (longer ones are left as holes and filled).
const MAX_FACE: usize = 8;

/// Options of [`quad_remesh`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadSpec {
    /// Target edge length in meters.
    pub edge: f32,
    /// Smoothing sweeps per hierarchy level for each field.
    pub iterations: u32,
    /// Relaxation sweeps of the result onto the original surface.
    pub relax: u32,
    pub seed: u64,
    /// Sample spacing of the fields, as a fraction of the extraction lattice step.
    pub density: f32,
}

impl Default for QuadSpec {
    fn default() -> Self {
        QuadSpec { edge: 0.01, iterations: 12, relax: 4, seed: 0, density: 0.4 }
    }
}

/// What a remesh made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuadReport {
    pub quads: usize,
    pub triangles: usize,
    /// Faces with five or more corners.
    pub other: usize,
    /// Vertices whose valence is not 4 (poles): valence 3, 5, 6 and 7 or more.
    pub poles: [usize; 4],
    /// Vertices of the dense sampling the fields were solved on.
    pub samples: usize,
}

/// Why a remesh failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RemeshError {
    #[error("the edge length must be positive")]
    BadEdge,
    #[error("the mesh is empty")]
    Empty,
    #[error("{0} samples would be needed; use a longer edge")]
    TooLarge(usize),
    #[error("the remeshed surface could not be built: {0}")]
    Build(String),
}

/// The input as triangles over compact vertices.
fn triangles(mesh: &Mesh) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut map = vec![u32::MAX; mesh.vert_slots()];
    let mut pos = Vec::with_capacity(mesh.vert_count());
    for v in mesh.verts() {
        map[v as usize] = pos.len() as u32;
        pos.push(mesh.positions[v as usize]);
    }
    let tris = mesh.faces().flat_map(|f| mesh.face_triangles(f)).map(|t| t.map(|h| map[mesh.origin(h) as usize])).collect();
    (pos, tris)
}

/// Splits every triangle into four until the longest edge is at most
/// `max`: the fields compare neighbours, so no graph edge may span a
/// lattice step (long thin input triangles would).
///
/// Complexity: O(n) per split round for n triangles (rounds grow n by 4).
fn densify(mut pos: Vec<Vec3>, mut tris: Vec<[u32; 3]>, max: f32) -> Result<(Vec<Vec3>, Vec<[u32; 3]>), RemeshError> {
    loop {
        let longest = tris
            .iter()
            .map(|t| (0..3).map(|k| pos[t[k] as usize].distance(pos[t[(k + 1) % 3] as usize])).fold(0.0f32, f32::max))
            .fold(0.0f32, f32::max);
        if longest <= max {
            return Ok((pos, tris));
        }
        if pos.len() * 4 > MAX_SAMPLES {
            return Err(RemeshError::TooLarge(pos.len() * 4));
        }
        let mut mid: FxHashMap<(u32, u32), u32> = FxHashMap::default();
        let mut next = Vec::with_capacity(tris.len() * 4);
        for t in &tris {
            let mut m = [0u32; 3];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let key = (a.min(b), a.max(b));
                m[k] = *mid.entry(key).or_insert_with(|| {
                    pos.push((pos[a as usize] + pos[b as usize]) * 0.5);
                    pos.len() as u32 - 1
                });
            }
            next.extend([[t[0], m[0], m[2]], [m[0], t[1], m[1]], [m[2], m[1], t[2]], [m[0], m[1], m[2]]]);
        }
        tris = next;
    }
}

/// The finest graph level: normals and areas from the triangles.
///
/// Complexity: O(V + n log n).
fn graph(pos: Vec<Vec3>, tris: &[[u32; 3]]) -> Level {
    let mut n = vec![Vec3::ZERO; pos.len()];
    let mut area = vec![0.0f32; pos.len()];
    let mut edges = Vec::with_capacity(tris.len() * 3);
    for t in tris {
        let [a, b, c] = t.map(|i| pos[i as usize]);
        let cross = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += cross;
            area[i as usize] += cross.length() / 6.0;
        }
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            edges.push((i.min(j), i.max(j)));
        }
    }
    let n = n.into_iter().map(|v| v.normalize_or(Vec3::Y)).collect();
    Level::with_edges(pos, n, area, edges)
}

/// Disjoint sets over `u32` ids; the smaller root wins.
fn find(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        let up = p[p[x as usize] as usize];
        p[x as usize] = up;
        x = up;
    }
    x
}

fn union(p: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(p, a), find(p, b));
    if ra != rb {
        p[ra.max(rb) as usize] = ra.min(rb);
    }
}

/// Lattice offset of `b` from `a` (in steps along `q` and `n x q`).
#[inline]
fn steps(a: Vec3, b: Vec3, q: Vec3, n: Vec3, scale: f32) -> (f32, f32) {
    let q = (q - n * n.dot(q)).normalize_or(n.any_orthonormal_vector());
    let t = n.cross(q);
    let d = b - a;
    (q.dot(d) / scale, t.dot(d) / scale)
}

/// Sums over a cluster of samples: lattice positions, normals, the first
/// sample's orientation and the sample count.
type ClusterSum = (Vec3, Vec3, Vec3, u32);

/// Output vertices (lattice points) and the faces walked through the
/// lattice graph.
///
/// Samples whose lattice points coincide merge into one vertex; vertices
/// joined by a sample edge one lattice step long are connected, and so are
/// vertices whose averaged lattice points lie one step apart (the sample
/// edges' own rounding misses some near seams of the field). Vertices left
/// on the same lattice point merge too.
///
/// Complexity: O(E log E) for the E graph edges.
fn extract(l: &Level, q: &[Vec3], o: &[Vec3], scale: f32) -> (Vec<Vec3>, Vec<Vec<u32>>) {
    let nv = l.len();
    let mut parent: Vec<u32> = (0..nv as u32).collect();
    let mut links: Vec<(u32, u32, bool)> = Vec::new();
    let mut same: Vec<(f32, u32, u32)> = Vec::new();
    for i in 0..nv {
        for &j in l.neighbors(i) {
            if (j as usize) <= i {
                continue;
            }
            let j = j as usize;
            let qj = compat_orientation(q[i], l.n[i], q[j], l.n[j]).1;
            let (a, b, k0, k1) = compat_position((l.x[i], l.n[i], q[i], o[i]), (l.x[j], l.n[j], qj, o[j]), scale);
            match (k0[0] - k1[0]).abs() + (k0[1] - k1[1]).abs() {
                0 => same.push(((a - b).length_squared(), i as u32, j as u32)),
                d => links.push((i as u32, j as u32, d == 1)),
            }
        }
    }
    // Samples on one lattice point merge, surest pairs first; a merge that
    // would join two clusters whose lattice points lie half a step apart or
    // more is refused (errors would otherwise chain neighbours together).
    same.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut acc: Vec<(Vec3, u32)> = o.iter().map(|&p| (p, 1)).collect();
    for &(_, i, j) in &same {
        let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
        if ri == rj {
            continue;
        }
        let (pi, pj) = (acc[ri as usize].0 / acc[ri as usize].1 as f32, acc[rj as usize].0 / acc[rj as usize].1 as f32);
        if pi.distance(pj) > 0.5 * scale {
            links.push((i, j, false));
            continue;
        }
        let (lo, hi) = (ri.min(rj), ri.max(rj));
        parent[hi as usize] = lo;
        let moved = acc[hi as usize];
        acc[lo as usize].0 += moved.0;
        acc[lo as usize].1 += moved.1;
    }
    // Clusters, then a second round merging clusters on one lattice point.
    let clusters = |parent: &mut Vec<u32>| -> (Vec<u32>, Vec<ClusterSum>) {
        let mut id = vec![u32::MAX; nv];
        let mut sums: Vec<ClusterSum> = Vec::new();
        for i in 0..nv {
            let r = find(parent, i as u32) as usize;
            if id[r] == u32::MAX {
                id[r] = sums.len() as u32;
                sums.push((Vec3::ZERO, Vec3::ZERO, q[i], 0));
            }
            let s = &mut sums[id[r] as usize];
            s.0 += o[i];
            s.1 += l.n[i];
            s.3 += 1;
        }
        let of: Vec<u32> = (0..nv).map(|i| id[find(parent, i as u32) as usize]).collect();
        (of, sums)
    };
    let (of, sums) = clusters(&mut parent);
    let at = |s: &ClusterSum| s.0 / s.3 as f32;
    for &(i, j, _) in &links {
        let (a, b) = (of[i as usize] as usize, of[j as usize] as usize);
        if a == b {
            continue;
        }
        let n = (sums[a].1 + sums[b].1).normalize_or(Vec3::Y);
        let (u, v) = steps(at(&sums[a]), at(&sums[b]), sums[a].2, n, scale);
        if u.round() == 0.0 && v.round() == 0.0 {
            union(&mut parent, i, j);
        }
    }
    let (of, sums) = clusters(&mut parent);
    let pos: Vec<Vec3> = sums.iter().map(at).collect();
    let nrm: Vec<Vec3> = sums.iter().map(|s| s.1.normalize_or(Vec3::Y)).collect();
    let mut edges: Vec<(u32, u32)> = links
        .iter()
        .filter_map(|&(i, j, unit)| {
            let (a, b) = (of[i as usize], of[j as usize]);
            if a == b {
                return None;
            }
            let n = (nrm[a as usize] + nrm[b as usize]).normalize_or(Vec3::Y);
            let (u, v) = steps(pos[a as usize], pos[b as usize], sums[a as usize].2, n, scale);
            let one = u.round().abs() + v.round().abs() == 1.0;
            (unit || one).then_some((a.min(b), a.max(b)))
        })
        .collect();
    edges.sort_unstable();
    edges.dedup();
    // Neighbours of each output vertex, counter-clockwise around its normal.
    let mut nbrs: Vec<SmallVec<[u32; 6]>> = vec![SmallVec::new(); pos.len()];
    for &(a, b) in &edges {
        nbrs[a as usize].push(b);
        nbrs[b as usize].push(a);
    }
    for (v, list) in nbrs.iter_mut().enumerate() {
        let n = nrm[v];
        let qv = sums[v].2;
        let a = (qv - n * n.dot(qv)).normalize_or(n.any_orthonormal_vector());
        let b = n.cross(a);
        let angle = |w: u32| {
            let d = pos[w as usize] - pos[v];
            b.dot(d).atan2(a.dot(d))
        };
        list.sort_by(|&x, &y| angle(x).total_cmp(&angle(y)).then(x.cmp(&y)));
    }
    // Walk the face to the left of every directed edge: at each vertex turn
    // to the neighbour just before the one we came from.
    let slot = |list: &SmallVec<[u32; 6]>, w: u32| list.iter().position(|&x| x == w);
    let mut used: FxHashMap<(u32, u32), ()> = FxHashMap::default();
    let mut faces = Vec::new();
    for u in 0..pos.len() as u32 {
        for k0 in 0..nbrs[u as usize].len() {
            let v = nbrs[u as usize][k0];
            if used.contains_key(&(u, v)) {
                continue;
            }
            let mut face = vec![u];
            let (mut a, mut b) = (u, v);
            let mut ok = false;
            for _ in 0..MAX_FACE {
                used.insert((a, b), ());
                let list = &nbrs[b as usize];
                let Some(k) = slot(list, a) else { break };
                let w = list[(k + list.len() - 1) % list.len()];
                if b == u {
                    ok = true;
                    break;
                }
                face.push(b);
                a = b;
                b = w;
            }
            if ok && face.len() >= 3 {
                faces.push(face);
            }
        }
    }
    (pos, faces)
}

/// Remeshes a closed surface into quads aligned to a smooth direction
/// field, with edges about `spec.edge` long.
///
/// A quad-dominant mesh is extracted at twice the edge length and
/// subdivided once (Catmull-Clark), so every face is a quad: triangles and
/// pentagons of the extraction become poles of valence 3 and 5. Small holes
/// left by the extraction are closed first. Vertices are relaxed onto the
/// original surface.
///
/// Complexity: O(S x iterations) for the S samples of the densified input
/// (about the area over (edge / 1.25)^2), plus O(S log S) for the hierarchy
/// and O(V log n) per relaxation sweep for the BVH projection.
pub fn quad_remesh(mesh: &Mesh, spec: &QuadSpec) -> Result<(Mesh, QuadReport), RemeshError> {
    if !(spec.edge.is_finite() && spec.edge > 0.0) {
        return Err(RemeshError::BadEdge);
    }
    let (pos, tris) = triangles(mesh);
    if tris.is_empty() {
        return Err(RemeshError::Empty);
    }
    let bvh = TriBvh::from_triangles(tris.iter().map(|t| t.map(|i| pos[i as usize])).collect(), tris.clone(), (0..tris.len() as u32).collect());
    let scale = spec.edge * 2.0;
    let (pos, tris) = densify(pos, tris, scale * spec.density)?;
    let samples = pos.len();
    let levels = hierarchy(graph(pos, &tris), 64);
    let q = orientation_field(&levels, spec.iterations, spec.seed);
    let o = position_field(&levels, &q, scale, spec.iterations);
    let (vpos, faces) = extract(&levels[0], &q[0], &o[0], scale);
    if faces.is_empty() {
        return Err(RemeshError::Build("no faces came out; the edge may be too long for the shape".into()));
    }
    let sizes: Vec<u32> = faces.iter().map(|f| f.len() as u32).collect();
    let corners: Vec<u32> = faces.concat();
    let r = repair(&vpos, &sizes, &corners, &SoupSpec::default()).map_err(|e| RemeshError::Build(e.to_string()))?;
    let mut coarse = r.mesh;
    close_holes(&mut coarse);
    drop_flat_pieces(&mut coarse, scale * 0.1);
    // Degenerate and repeated faces of the walk go before subdividing.
    let tidy = p3d_geo::cleanup::CleanupSpec { weld: scale * 0.05, ..p3d_geo::cleanup::CleanupSpec::default() };
    p3d_geo::cleanup::cleanup(&mut coarse, &tidy);
    relax(&mut coarse, &bvh, spec.relax);
    let subd = SubdSpec { levels: 1, keep_corners: true, sharp_as_crease: false };
    let mut out = catmull_clark(&coarse, &subd).map_err(|e| RemeshError::Build(e.to_string()))?;
    relax(&mut out, &bvh, spec.relax);
    let report = report(&out, samples);
    Ok((out, report))
}

/// Removes pieces thinner on average (volume over area) than `thin`: pairs
/// of faces back to back that the face walk can leave on the surface.
///
/// Complexity: O(F + C).
fn drop_flat_pieces(mesh: &mut Mesh, thin: f32) {
    let (label, count) = mesh.face_components();
    if count < 2 {
        return;
    }
    let mut vol = vec![0.0f64; count as usize];
    let mut area = vec![0.0f64; count as usize];
    for f in mesh.faces() {
        let c = label[f as usize] as usize;
        for t in mesh.face_triangles(f) {
            let [a, b, d] = t.map(|h| mesh.positions[mesh.origin(h) as usize].as_dvec3());
            vol[c] += a.dot(b.cross(d)) / 6.0;
            area[c] += (b - a).cross(d - a).length() / 2.0;
        }
    }
    let flat: Vec<u32> = mesh
        .faces()
        .filter(|&f| {
            let c = label[f as usize] as usize;
            vol[c].abs() < f64::from(thin) * area[c]
        })
        .collect();
    if !flat.is_empty() {
        // Whole pieces go, so the rest stays closed.
        let _ = mesh.delete_faces(&flat);
    }
}

/// Closes every boundary loop the extraction left (where the walk failed,
/// mostly along seams of the position field): one polygon each,
/// triangulated when it has more than six corners.
///
/// Complexity: O(H) plus the loops.
fn close_holes(mesh: &mut Mesh) {
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
        starts.push(h);
    }
    let mut big = Vec::new();
    for h in starts {
        // A loop that cannot close stays open.
        if let Ok(f) = mesh.fill_boundary_loop(h)
            && mesh.face_len(f) > 6
        {
            big.push(f);
        }
    }
    // A long patch becomes triangles, so subdivision makes ordinary quads
    // instead of one many-sided pole.
    if !big.is_empty() {
        let _ = p3d_geo::edit::triangulate(mesh, &big);
    }
}

/// Moves every vertex toward the mean of its neighbours within its tangent
/// plane, then onto the original surface, `sweeps` times. A projection onto
/// a triangle facing away (the other side of a thin tip) is not taken.
///
/// Complexity: O(sweeps x (C + V log n)).
fn relax(mesh: &mut Mesh, bvh: &TriBvh, sweeps: u32) {
    let verts: Vec<u32> = mesh.verts().collect();
    let reach = bvh.bounds().map_or(1.0, |(lo, hi)| (hi - lo).length());
    for _ in 0..sweeps {
        let normals = vertex_normals(mesh);
        let moved: Vec<Vec3> = verts
            .iter()
            .map(|&v| {
                let at = mesh.positions[v as usize];
                let n = normals[v as usize];
                let (sum, k) = mesh.vert_neighbors(v).fold((Vec3::ZERO, 0u32), |(s, k), w| (s + mesh.positions[w as usize], k + 1));
                if k == 0 {
                    return at;
                }
                let d = sum / k as f32 - at;
                let p = at + d - n * n.dot(d);
                match bvh.closest(p, reach) {
                    Some(c) if bvh.normal(c.tri).dot(n) > 0.0 => c.point,
                    _ => p,
                }
            })
            .collect();
        for (&v, p) in verts.iter().zip(moved) {
            mesh.positions[v as usize] = p;
        }
    }
    mesh.revision += 1;
}

/// Area-weighted vertex normals by vertex slot.
///
/// Complexity: O(C).
fn vertex_normals(mesh: &Mesh) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; mesh.vert_slots()];
    for f in mesh.faces() {
        let fn_ = mesh.face_normal_area2(f);
        for v in mesh.face_verts(f) {
            n[v as usize] += fn_;
        }
    }
    n.into_iter().map(|v| v.normalize_or_zero()).collect()
}

fn report(mesh: &Mesh, samples: usize) -> QuadReport {
    let mut r = QuadReport { samples, ..QuadReport::default() };
    for f in mesh.faces() {
        match mesh.face_len(f) {
            3 => r.triangles += 1,
            4 => r.quads += 1,
            _ => r.other += 1,
        }
    }
    for v in mesh.verts() {
        let open = mesh.vert_outgoing(v).any(|h| mesh.face(h).is_none() || mesh.face(h ^ 1).is_none());
        match mesh.valence(v) {
            _ if open => {}
            3 => r.poles[0] += 1,
            4 => {}
            5 => r.poles[1] += 1,
            6 => r.poles[2] += 1,
            _ => r.poles[3] += 1,
        }
    }
    r
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stderr)]
    use super::*;
    use glam::DVec3;
    use p3d_geo::prim::{IcoSphereSpec, Origin, TorusSpec, ico_sphere, torus};

    fn closed(m: &Mesh) -> bool {
        m.edges().all(|e| !m.is_boundary_edge(e))
    }

    #[test]
    fn sphere_becomes_quads() {
        let s = ico_sphere(&IcoSphereSpec { radius: 0.5, subdivisions: 4 }, Origin::Center).unwrap();
        let (m, r) = quad_remesh(&s, &QuadSpec { edge: 0.08, ..QuadSpec::default() }).unwrap();
        eprintln!("{r:?}");
        assert_eq!(m.validate(), Ok(()));
        assert!(closed(&m), "{r:?}");
        let faces = r.quads + r.triangles + r.other;
        assert!(r.quads as f32 >= 0.9 * faces as f32, "{r:?}");
        // Edges come out near the target.
        let mean = m
            .edges()
            .map(|e| {
                let [a, b] = m.edge_verts(e);
                m.positions[a as usize].distance(m.positions[b as usize])
            })
            .sum::<f32>()
            / m.edges().count() as f32;
        assert!((mean - 0.08).abs() < 0.03, "{mean}");
        for v in m.verts() {
            assert!((m.positions[v as usize].length() - 0.5).abs() < 0.01);
        }
    }

    #[test]
    fn torus_is_almost_all_regular() {
        let t = torus(&TorusSpec { major_radius: 0.4, minor_radius: 0.15, segments: 48, sides: 24 }, Origin::Center).unwrap();
        let (m, r) = quad_remesh(&t, &QuadSpec { edge: 0.05, ..QuadSpec::default() }).unwrap();
        eprintln!("{r:?}");
        assert!(closed(&m));
        let faces = r.quads + r.triangles + r.other;
        assert!(r.quads as f32 >= 0.9 * faces as f32, "{r:?}");
        let _ = DVec3::ZERO;
    }

    #[test]
    fn the_same_input_gives_the_same_mesh() {
        let s = ico_sphere(&IcoSphereSpec { radius: 0.5, subdivisions: 3 }, Origin::Center).unwrap();
        let a = quad_remesh(&s, &QuadSpec { edge: 0.1, ..QuadSpec::default() }).unwrap().0;
        let b = quad_remesh(&s, &QuadSpec { edge: 0.1, ..QuadSpec::default() }).unwrap().0;
        assert_eq!(a.positions, b.positions);
    }
}
