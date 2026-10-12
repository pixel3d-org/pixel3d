//! Face and corner normals.

use glam::Vec3;

use crate::attrs::EdgeFlags;
use crate::mesh::Mesh;

impl Mesh {
    /// Unit normal of every face slot (zero for dead or degenerate faces).
    ///
    /// Complexity: O(F + C).
    pub fn face_normals(&self) -> Vec<Vec3> {
        let mut out = vec![Vec3::ZERO; self.face_slots()];
        for f in self.faces() {
            out[f as usize] = self.face_normal(f);
        }
        out
    }

    /// Interior angle of the corner at the origin of `h` (radians).
    #[inline]
    pub fn corner_angle(&self, h: u32) -> f32 {
        let o = self.positions[self.origin(h) as usize];
        let a = self.positions[self.dest(h) as usize] - o;
        let b = self.positions[self.origin(self.prev(h)) as usize] - o;
        a.angle_between(b)
    }

    /// Shading normal of every corner (indexed by half-edge slot; boundary
    /// half-edges get zero). Custom split normals are returned as they are.
    /// Otherwise faces around a vertex are smoothed together, weighted by
    /// corner angle, and split at sharp edges and boundaries: every corner of
    /// one smooth fan gets exactly the same normal.
    ///
    /// Complexity: O(V + F + C).
    pub fn compute_corner_normals(&self) -> Vec<Vec3> {
        if let Some(custom) = &self.corner_normal {
            return custom.clone();
        }
        let face_n = self.face_normals();
        let mut out = vec![Vec3::ZERO; self.he_slots()];
        let mut fan: Vec<u32> = Vec::new();
        for v in self.verts() {
            fan.clear();
            fan.extend(self.vert_outgoing(v));
            let k = fan.len();
            // Edge of fan[j] separates sectors j - 1 and j (sector j = face(fan[j])).
            let breaks = |j: usize| -> bool {
                let h = fan[j % k];
                self.is_boundary_he(h) || self.is_boundary_he(h ^ 1) || self.edge_flags(h >> 1).contains(EdgeFlags::SHARP)
            };
            let start = (0..k).find(|&j| breaks(j)).unwrap_or(0);
            let mut j = start;
            while j < start + k {
                // One smooth run: sectors j.. until the next break.
                let run_start = j;
                let mut sum = Vec3::ZERO;
                loop {
                    let h = fan[j % k];
                    if let Some(f) = self.face(h) {
                        sum += face_n[f as usize] * self.corner_angle(h);
                    }
                    j += 1;
                    if j >= start + k || breaks(j) {
                        break;
                    }
                }
                let n = sum.normalize_or_zero();
                for i in run_start..j {
                    let h = fan[i % k];
                    if !self.is_boundary_he(h) {
                        out[h as usize] = n;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EdgeFlags;

    fn cube() -> Mesh {
        let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32) - Vec3::splat(0.5)).collect();
        let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
        Mesh::from_polygons(p, &[4; 6], &faces).unwrap()
    }

    #[test]
    fn smooth_cube_normals_point_to_corners() {
        let m = cube();
        let n = m.compute_corner_normals();
        for f in m.faces() {
            for h in m.face_halfedges(f) {
                let expect = m.positions[m.origin(h) as usize].normalize();
                assert!(n[h as usize].abs_diff_eq(expect, 1e-5), "{} vs {expect}", n[h as usize]);
            }
        }
    }

    #[test]
    fn sharp_cube_normals_are_face_normals() {
        let mut m = cube();
        for e in m.edges().collect::<Vec<_>>() {
            m.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
        let n = m.compute_corner_normals();
        for f in m.faces() {
            for h in m.face_halfedges(f) {
                assert!(n[h as usize].abs_diff_eq(m.face_normal(f), 1e-6));
            }
        }
    }

    #[test]
    fn sharp_edges_split_fans_only_where_they_separate_faces() {
        let mut m = cube();
        for b in [1, 2] {
            let e = m.find_halfedge(0, b).map(|h| h >> 1).unwrap();
            m.set_edge_flag(e, EdgeFlags::SHARP, true);
        }
        let n = m.compute_corner_normals();
        let distinct = |v: u32| {
            let mut ns: Vec<[u32; 3]> =
                m.vert_outgoing(v).filter(|&h| !m.is_boundary_he(h)).map(|h| n[h as usize].to_array().map(f32::to_bits)).collect();
            ns.sort_unstable();
            ns.dedup();
            ns.len()
        };
        // Two sharp edges at vertex 0 cut its fan in two; vertices 1 and 2 have one
        // sharp edge each, which cannot separate a closed fan.
        assert_eq!(distinct(0), 2);
        assert_eq!(distinct(1), 1);
        assert_eq!(distinct(2), 1);
        assert_eq!(distinct(7), 1);
    }

    #[test]
    fn custom_normals_win() {
        let mut m = cube();
        m.corner_normal = Some(vec![Vec3::Y; m.he_slots()]);
        assert!(m.compute_corner_normals().iter().all(|&n| n == Vec3::Y));
    }
}
