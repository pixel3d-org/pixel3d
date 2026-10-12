//! Triangle buffers for the GPU and for export.

use glam::{Vec2, Vec3, Vec4};

use crate::mesh::Mesh;

/// Largest concave polygon triangulated by ear clipping (O(k^2) in its
/// corners). Larger concave polygons fall back to a fan and are counted in
/// [`TriBuffers::fallback_faces`]; constrained Delaunay replaces the fallback
/// later.
pub const MAX_EAR_CLIP_CORNERS: usize = 256;

/// What [`Mesh::to_tri_buffers`] includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriOptions {
    pub normals: bool,
    pub uvs: bool,
    pub colors: bool,
}

impl Default for TriOptions {
    fn default() -> Self {
        TriOptions { normals: true, uvs: true, colors: true }
    }
}

/// A contiguous range of indices drawn with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submesh {
    pub material: u16,
    /// First index (a multiple of 3) and number of indices.
    pub start: u32,
    pub count: u32,
}

/// Indexed triangles with vertices split only where corner attributes
/// differ (UV seams, hard edges, color borders).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriBuffers {
    pub positions: Vec<Vec3>,
    /// Empty unless normals were requested.
    pub normals: Vec<Vec3>,
    /// One list per UV layer; empty unless UVs were requested.
    pub uvs: Vec<Vec<Vec2>>,
    /// Empty unless colors were requested and the mesh has them.
    pub colors: Vec<Vec4>,
    /// Three indices per triangle, sorted by material.
    pub indices: Vec<u32>,
    /// Source face of each triangle (for picking and provenance).
    pub tri_face: Vec<u32>,
    /// Source mesh vertex of each output vertex (for skinning and picking).
    pub vert_source: Vec<u32>,
    /// Index ranges per material, in material order.
    pub submeshes: Vec<Submesh>,
    /// Concave faces above [`MAX_EAR_CLIP_CORNERS`] corners triangulated as a fan.
    pub fallback_faces: u32,
}

impl TriBuffers {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Bit pattern of a float with -0.0 folded into 0.0, for exact keys.
#[inline]
fn key_bits(x: f32) -> u32 {
    if x == 0.0 { 0 } else { x.to_bits() }
}

impl Mesh {
    /// Triangulates every face and builds indexed vertex buffers.
    ///
    /// - Convex faces become fans; quads use the shorter valid diagonal;
    ///   concave faces use ear clipping.
    /// - Vertices are split per corner attribute tuple, deduplicated within
    ///   each vertex's fan (no global hash map).
    /// - Triangles are grouped by material with a counting sort.
    ///
    /// Complexity: O(V + F + C log(max valence)), plus O(k^2) for each
    /// concave face of k <= [`MAX_EAR_CLIP_CORNERS`] corners.
    pub fn to_tri_buffers(&self, opts: &TriOptions) -> TriBuffers {
        let normals = if opts.normals { Some(self.compute_corner_normals()) } else { None };
        let layers = if opts.uvs { self.corner_uv.len() } else { 0 };
        let colors = if opts.colors { self.corner_color.as_ref() } else { None };

        let mut out = TriBuffers { uvs: vec![Vec::new(); layers], ..TriBuffers::default() };
        // Output vertex of each corner (half-edge slot).
        let mut corner_vert = vec![u32::MAX; self.he_slots()];
        // Corner keys of one vertex fan: normal (3), UV layers (2 x 2), color (4).
        let mut keyed: Vec<([u32; 11], u32)> = Vec::new();
        for v in self.verts() {
            keyed.clear();
            for h in self.vert_outgoing(v) {
                if self.is_boundary_he(h) {
                    continue;
                }
                let mut key = [0u32; 11];
                if let Some(n) = &normals {
                    key[0..3].copy_from_slice(&n[h as usize].to_array().map(key_bits));
                }
                for l in 0..layers {
                    key[3 + 2 * l..5 + 2 * l].copy_from_slice(&self.corner_uv[l][h as usize].to_array().map(key_bits));
                }
                if let Some(c) = colors {
                    key[7..11].copy_from_slice(&c[h as usize].to_array().map(key_bits));
                }
                keyed.push((key, h));
            }
            keyed.sort_unstable();
            for i in 0..keyed.len() {
                let h = keyed[i].1;
                if i == 0 || keyed[i - 1].0 != keyed[i].0 {
                    out.positions.push(self.positions[v as usize]);
                    out.vert_source.push(v);
                    if let Some(n) = &normals {
                        out.normals.push(n[h as usize]);
                    }
                    for (l, uv) in out.uvs.iter_mut().enumerate() {
                        uv.push(self.corner_uv[l][h as usize]);
                    }
                    if let Some(c) = colors {
                        out.colors.push(c[h as usize]);
                    }
                }
                corner_vert[h as usize] = (out.positions.len() - 1) as u32;
            }
        }

        // Triangulate into per-face triangles, then counting-sort by material.
        let mut tris: Vec<[u32; 3]> = Vec::new();
        let mut tri_face: Vec<u32> = Vec::new();
        let mut corners: Vec<u32> = Vec::new();
        let mut ear = EarScratch::default();
        for f in self.faces() {
            corners.clear();
            corners.extend(self.face_halfedges(f));
            let before = tris.len();
            out.fallback_faces += u32::from(!self.triangulate_face(f, &corners, &mut ear, &mut tris));
            tri_face.extend(std::iter::repeat_n(f, tris.len() - before));
        }
        let mut counts: Vec<u32> = Vec::new();
        for &f in &tri_face {
            let m = self.face_material[f as usize] as usize;
            if counts.len() <= m {
                counts.resize(m + 1, 0);
            }
            counts[m] += 1;
        }
        let mut offset = vec![0u32; counts.len()];
        let mut acc = 0u32;
        for (m, &c) in counts.iter().enumerate() {
            offset[m] = acc;
            if c > 0 {
                out.submeshes.push(Submesh { material: m as u16, start: 3 * acc, count: 3 * c });
            }
            acc += c;
        }
        out.indices = vec![0; 3 * tris.len()];
        out.tri_face = vec![0; tris.len()];
        for (t, tri) in tris.iter().enumerate() {
            let f = tri_face[t];
            let m = self.face_material[f as usize] as usize;
            let slot = offset[m] as usize;
            offset[m] += 1;
            for (i, &h) in tri.iter().enumerate() {
                out.indices[3 * slot + i] = corner_vert[h as usize];
            }
            out.tri_face[slot] = f;
        }
        out
    }

    /// Appends the triangles of one face as corner half-edges. Returns false
    /// when a large concave face fell back to a fan.
    fn triangulate_face(&self, f: u32, corners: &[u32], ear: &mut EarScratch, tris: &mut Vec<[u32; 3]>) -> bool {
        let k = corners.len();
        if k == 3 {
            tris.push([corners[0], corners[1], corners[2]]);
            return true;
        }
        let normal = self.face_normal_area2(f);
        project(self, corners, normal, &mut ear.pts);
        let convex = (0..k).all(|i| cross(ear.pts[(i + k - 1) % k], ear.pts[i], ear.pts[(i + 1) % k]) >= 0.0);
        if k == 4 {
            // Diagonal 0-2 is valid when corners 0 and 2 are convex enough for
            // it: choose it unless 1 or 3 is reflex, or 1-3 is shorter on a convex quad.
            let reflex = |i: usize| cross(ear.pts[(i + 3) % 4], ear.pts[i], ear.pts[(i + 1) % 4]) < 0.0;
            let d02 = ear.pts[0].distance_squared(ear.pts[2]);
            let d13 = ear.pts[1].distance_squared(ear.pts[3]);
            let use13 = if reflex(1) || reflex(3) {
                true
            } else if reflex(0) || reflex(2) {
                false
            } else {
                d13 < d02
            };
            let c = corners;
            if use13 {
                tris.push([c[1], c[2], c[3]]);
                tris.push([c[1], c[3], c[0]]);
            } else {
                tris.push([c[0], c[1], c[2]]);
                tris.push([c[0], c[2], c[3]]);
            }
            return true;
        }
        if convex || k > MAX_EAR_CLIP_CORNERS {
            for i in 1..k - 1 {
                tris.push([corners[0], corners[i], corners[i + 1]]);
            }
            return convex;
        }
        ear_clip(corners, ear, tris);
        true
    }
}

#[derive(Default)]
struct EarScratch {
    pts: Vec<Vec2>,
    next: Vec<usize>,
    prev: Vec<usize>,
    /// Cached ear status per corner.
    ear: Vec<bool>,
}

/// z of the 2D cross product (b - a) x (c - b): positive for a left turn.
#[inline]
fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - b)
}

/// Projects face corners to 2D along the dominant axis of the normal, so
/// that the face winds counter-clockwise.
fn project(mesh: &Mesh, corners: &[u32], normal: Vec3, pts: &mut Vec<Vec2>) {
    let a = normal.abs();
    let (axis, sign) = if a.x >= a.y && a.x >= a.z {
        (0, normal.x)
    } else if a.y >= a.z {
        (1, normal.y)
    } else {
        (2, normal.z)
    };
    pts.clear();
    pts.extend(corners.iter().map(|&h| {
        let p = mesh.positions[mesh.origin(h) as usize];
        // Dropping one axis of a right-handed frame keeps CCW order when the
        // normal points along +axis; mirror one coordinate otherwise.
        let q = match axis {
            0 => Vec2::new(p.y, p.z),
            1 => Vec2::new(p.z, p.x),
            _ => Vec2::new(p.x, p.y),
        };
        if sign < 0.0 { Vec2::new(q.y, q.x) } else { q }
    }));
}

/// Whether corner `i` is an ear of the remaining polygon: convex, and no
/// other remaining corner lies inside or on the triangle (prev, i, next).
///
/// Complexity: O(remaining corners).
fn is_ear(ear: &EarScratch, i: usize) -> bool {
    let (pts, p, n) = (&ear.pts, ear.prev[i], ear.next[i]);
    if cross(pts[p], pts[i], pts[n]) <= 0.0 {
        return false;
    }
    let mut j = ear.next[n];
    while j != p {
        let q = pts[j];
        if cross(pts[p], pts[i], q) >= 0.0 && cross(pts[i], pts[n], q) >= 0.0 && cross(pts[n], pts[p], q) >= 0.0 {
            return false;
        }
        j = ear.next[j];
    }
    true
}

/// Ear clipping of a simple CCW polygon (already projected into `ear.pts`).
///
/// Ear status is cached: clipping a corner changes only its two neighbours'
/// status, so each clip costs one O(k) scan for the next ear plus two O(k)
/// ear tests.
///
/// Complexity: O(k^2) for k corners, k <= MAX_EAR_CLIP_CORNERS.
fn ear_clip(corners: &[u32], ear: &mut EarScratch, tris: &mut Vec<[u32; 3]>) {
    let k = corners.len();
    debug_assert!(k <= MAX_EAR_CLIP_CORNERS);
    ear.next.clear();
    ear.prev.clear();
    ear.next.extend((0..k).map(|i| (i + 1) % k));
    ear.prev.extend((0..k).map(|i| (i + k - 1) % k));
    ear.ear.clear();
    for i in 0..k {
        let e = is_ear(ear, i);
        ear.ear.push(e);
    }
    let mut remaining = k;
    let mut i = 0usize;
    while remaining > 3 {
        // Next cached ear, at most one lap; degenerate input without any ear
        // clips the current corner anyway so the loop always terminates.
        let mut steps = 0;
        while !ear.ear[i] && steps < remaining {
            i = ear.next[i];
            steps += 1;
        }
        let (p, n) = (ear.prev[i], ear.next[i]);
        tris.push([corners[p], corners[i], corners[n]]);
        ear.next[p] = n;
        ear.prev[n] = p;
        remaining -= 1;
        if remaining > 3 {
            ear.ear[p] = is_ear(ear, p);
            ear.ear[n] = is_ear(ear, n);
        }
        i = n;
    }
    let n = ear.next[i];
    tris.push([corners[ear.prev[i]], corners[i], corners[n]]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EdgeFlags;

    fn cube(sharp: bool) -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        let mut m = Mesh::from_polygons(p, &[4; 6], &faces).unwrap();
        if sharp {
            for e in m.edges().collect::<Vec<_>>() {
                m.set_edge_flag(e, EdgeFlags::SHARP, true);
            }
        }
        m
    }

    fn tri_area_sum(b: &TriBuffers) -> f32 {
        b.indices
            .chunks(3)
            .map(|t| {
                0.5 * (b.positions[t[1] as usize] - b.positions[t[0] as usize])
                    .cross(b.positions[t[2] as usize] - b.positions[t[0] as usize])
                    .length()
            })
            .sum()
    }

    #[test]
    fn smooth_cube_shares_vertices() {
        let b = cube(false).to_tri_buffers(&TriOptions::default());
        assert_eq!(b.triangle_count(), 12);
        assert_eq!(b.positions.len(), 8);
        assert_eq!(b.submeshes, vec![Submesh { material: 0, start: 0, count: 36 }]);
        assert!((tri_area_sum(&b) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn hard_cube_splits_per_face() {
        let b = cube(true).to_tri_buffers(&TriOptions::default());
        assert_eq!(b.positions.len(), 24);
        assert_eq!(b.normals.len(), 24);
        assert!(b.normals.iter().all(|n| n.abs().max_element() == 1.0));
        // Without normals nothing needs splitting.
        let plain = cube(true).to_tri_buffers(&TriOptions { normals: false, ..TriOptions::default() });
        assert_eq!(plain.positions.len(), 8);
    }

    #[test]
    fn triangles_face_outward() {
        let m = cube(true);
        let b = m.to_tri_buffers(&TriOptions::default());
        for (t, tri) in b.indices.chunks(3).enumerate() {
            let [a, bb, c] = [0, 1, 2].map(|i| b.positions[tri[i] as usize]);
            let n = (bb - a).cross(c - a).normalize();
            assert!(n.abs_diff_eq(m.face_normal(b.tri_face[t]), 1e-5));
        }
    }

    #[test]
    fn uv_seams_split_vertices() {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        let mut m = Mesh::from_polygons(p, &[3, 3], &[0, 1, 2, 2, 1, 3]).unwrap();
        // Face 1 uses different UVs on the shared vertices 1 and 2.
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::splat(0.5), Vec2::splat(0.6), Vec2::ONE];
        m.add_uv_layer_from_polygon_order(&uvs).unwrap();
        let b = m.to_tri_buffers(&TriOptions::default());
        assert_eq!(b.positions.len(), 6);
        assert_eq!(b.vert_source.iter().filter(|&&v| v == 1).count(), 2);
    }

    #[test]
    fn materials_become_sorted_submeshes() {
        let mut m = cube(false);
        m.face_material[0] = 2;
        m.face_material[3] = 2;
        m.face_material[5] = 1;
        let b = m.to_tri_buffers(&TriOptions::default());
        let s: Vec<(u16, u32, u32)> = b.submeshes.iter().map(|s| (s.material, s.start, s.count)).collect();
        assert_eq!(s, vec![(0, 0, 18), (1, 18, 6), (2, 24, 12)]);
        for sm in &b.submeshes {
            for t in (sm.start / 3)..(sm.start + sm.count) / 3 {
                assert_eq!(m.face_material[b.tri_face[t as usize] as usize], sm.material);
            }
        }
    }

    fn polygon(pts: &[[f32; 2]]) -> Mesh {
        let p = pts.iter().map(|&[x, y]| Vec3::new(x, y, 0.0)).collect();
        let idx: Vec<u32> = (0..pts.len() as u32).collect();
        Mesh::from_polygons(p, &[pts.len() as u32], &idx).unwrap()
    }

    #[test]
    fn concave_polygon_ear_clipping() {
        // L shape, area 3.
        let m = polygon(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 2.0], [0.0, 2.0]]);
        let b = m.to_tri_buffers(&TriOptions::default());
        assert_eq!(b.triangle_count(), 4);
        assert_eq!(b.fallback_faces, 0);
        assert!((tri_area_sum(&b) - 3.0).abs() < 1e-5, "{}", tri_area_sum(&b));
        for tri in b.indices.chunks(3) {
            let [a, bb, c] = [0, 1, 2].map(|i| b.positions[tri[i] as usize]);
            assert!((bb - a).cross(c - a).z > 0.0, "triangle wound backwards");
        }
    }

    #[test]
    fn comb_polygon_near_the_ear_clip_bound() {
        // 63 teeth: 2 + 4 * 63 = 254 corners, 126 of them reflex; area 4 per tooth.
        let teeth = 63u32;
        let mut pts = vec![[0.0, 0.0], [2.0 * teeth as f32, 0.0]];
        for t in (0..teeth).rev() {
            let x = 2.0 * t as f32;
            pts.extend([[x + 2.0, 1.0], [x + 1.0, 1.0], [x + 1.0, 3.0], [x, 3.0]]);
        }
        let b = polygon(&pts).to_tri_buffers(&TriOptions::default());
        assert_eq!(b.fallback_faces, 0);
        assert_eq!(b.triangle_count(), pts.len() - 2);
        assert!((tri_area_sum(&b) - 4.0 * teeth as f32).abs() < 1e-3, "{}", tri_area_sum(&b));
        for tri in b.indices.chunks(3) {
            let [a, bb, c] = [0, 1, 2].map(|i| b.positions[tri[i] as usize]);
            assert!((bb - a).cross(c - a).z > 0.0, "triangle wound backwards or degenerate");
        }
    }

    #[test]
    fn concave_quad_uses_valid_diagonal() {
        // Dart: corner 3 is reflex, so only diagonal 1-3 is inside.
        for start in 0..4 {
            let pts = [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0], [0.5, 0.5]];
            let rotated: Vec<[f32; 2]> = (0..4).map(|i| pts[(i + start) % 4]).collect();
            let b = polygon(&rotated).to_tri_buffers(&TriOptions::default());
            let expected = polygon(&rotated).face_area(0);
            assert!((tri_area_sum(&b) - expected).abs() < 1e-5, "start {start}");
        }
    }

    #[test]
    fn convex_ngon_and_flipped_plane() {
        let octagon: Vec<[f32; 2]> = (0..8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                [a.cos(), a.sin()]
            })
            .collect();
        let b = polygon(&octagon).to_tri_buffers(&TriOptions::default());
        assert_eq!(b.triangle_count(), 6);
        // The L shape wound clockwise (normal -Z) still clips correctly.
        let cw = polygon(&[[0.0, 2.0], [1.0, 2.0], [1.0, 1.0], [2.0, 1.0], [2.0, 0.0], [0.0, 0.0]]);
        let b = cw.to_tri_buffers(&TriOptions::default());
        assert!((tri_area_sum(&b) - 3.0).abs() < 1e-5);
    }
}
