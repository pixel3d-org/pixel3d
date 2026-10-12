//! Invariant checks for the half-edge mesh.

use crate::attrs::MAX_UV_LAYERS;
use crate::mesh::{INVALID, Mesh};

/// A broken mesh invariant: what is wrong and on which element.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid mesh: {0}")]
pub struct ValidationError(pub String);

macro_rules! ensure {
    ($cond:expr, $($fmt:tt)*) => {
        if !$cond {
            return Err(ValidationError(format!($($fmt)*)));
        }
    };
}

impl Mesh {
    /// Checks every structural invariant. Run after every op in debug builds
    /// and always in tests.
    ///
    /// - array lengths agree; half-edges come in pairs;
    /// - `prev(next(h)) == h`, `next(prev(h)) == h`, `origin(next(h)) == dest(h)`,
    ///   and both stay in the same face;
    /// - every face loop closes, has at least 3 corners and points back to its face;
    /// - every edge has two distinct live vertices and at least one face;
    /// - every live vertex has one outgoing half-edge as `vert_he` (the boundary one
    ///   on the boundary), a single fan, and a finite position;
    /// - free lists hold exactly the dead slots;
    /// - attribute columns match the element counts.
    ///
    /// Complexity: O(V + E + F) in element slots.
    pub fn validate(&self) -> Result<(), ValidationError> {
        let nh = self.he_slots();
        let nv = self.vert_slots();
        let nf = self.face_slots();
        ensure!(nh.is_multiple_of(2), "odd number of half-edges ({nh})");
        ensure!(self.he_next.len() == nh && self.he_prev.len() == nh && self.he_face.len() == nh, "half-edge arrays differ in length");
        ensure!(self.positions.len() == nv, "{} positions for {nv} vertex slots", self.positions.len());

        // Edges and half-edge links.
        let mut dead_edges = 0usize;
        for e in 0..self.edge_slots() as u32 {
            let (h0, h1) = (2 * e, 2 * e + 1);
            let alive0 = self.he_vert[h0 as usize] != INVALID;
            let alive1 = self.he_vert[h1 as usize] != INVALID;
            ensure!(alive0 == alive1, "edge {e} is half dead");
            if !alive0 {
                dead_edges += 1;
                continue;
            }
            let [a, b] = self.edge_verts(e);
            ensure!((a as usize) < nv && (b as usize) < nv, "edge {e} references vertex out of range");
            ensure!(a != b, "edge {e} is a loop on vertex {a}");
            ensure!(self.is_vert_alive(a) && self.is_vert_alive(b), "edge {e} references a dead vertex");
            ensure!(!(self.is_boundary_he(h0) && self.is_boundary_he(h1)), "edge {e} has no face (wire edge)");
            for h in [h0, h1] {
                let (n, p) = (self.next(h), self.prev(h));
                ensure!(self.is_he_alive(n) && self.is_he_alive(p), "half-edge {h} links to a dead half-edge");
                ensure!(self.prev(n) == h && self.next(p) == h, "half-edge {h}: next and prev disagree");
                ensure!(self.origin(n) == self.dest(h), "half-edge {h}: next does not start where it ends");
                ensure!(self.he_face[n as usize] == self.he_face[h as usize], "half-edge {h}: next lies in another face");
                if let Some(f) = self.face(h) {
                    ensure!(self.is_face_alive(f), "half-edge {h} belongs to dead face {f}");
                }
            }
        }

        // Faces.
        let mut dead_faces = 0usize;
        for f in 0..nf as u32 {
            let h = self.face_he[f as usize];
            if h == INVALID {
                dead_faces += 1;
                continue;
            }
            ensure!(self.is_he_alive(h), "face {f} starts at dead half-edge {h}");
            let mut len = 0usize;
            let mut cur = h;
            loop {
                ensure!(self.he_face[cur as usize] == f, "face {f}: loop half-edge {cur} belongs to another face");
                len += 1;
                ensure!(len <= nh, "face {f}: loop does not close");
                cur = self.next(cur);
                if cur == h {
                    break;
                }
            }
            ensure!(len >= 3, "face {f} has {len} corners");
        }

        // Vertices.
        let mut degree = vec![0u32; nv];
        let mut boundary_out = vec![0u32; nv];
        for h in 0..nh as u32 {
            let v = self.he_vert[h as usize];
            if v != INVALID {
                degree[v as usize] += 1;
                if self.is_boundary_he(h) {
                    boundary_out[v as usize] += 1;
                }
            }
        }
        let mut dead_verts = 0usize;
        for v in 0..nv as u32 {
            let h = self.vert_he[v as usize];
            if h == INVALID {
                ensure!(degree[v as usize] == 0, "dead vertex {v} still has edges");
                dead_verts += 1;
                continue;
            }
            ensure!(self.positions[v as usize].is_finite(), "vertex {v} has a non-finite position");
            ensure!(self.is_he_alive(h) && self.origin(h) == v, "vertex {v}: vert_he {h} does not start at it");
            ensure!(boundary_out[v as usize] <= 1, "vertex {v} has {} boundary fans", boundary_out[v as usize]);
            ensure!(boundary_out[v as usize] == 0 || self.is_boundary_he(h), "boundary vertex {v}: vert_he is not on the boundary");
            ensure!(self.vert_outgoing(v).count() == degree[v as usize] as usize, "vertex {v} has more than one fan (non-manifold)");
        }

        // Free lists hold exactly the dead slots, each once.
        ensure!(self.free_verts.len() == dead_verts, "free vertex list has {} entries for {dead_verts} dead vertices", self.free_verts.len());
        ensure!(self.free_edges.len() == dead_edges, "free edge list has {} entries for {dead_edges} dead edges", self.free_edges.len());
        ensure!(self.free_faces.len() == dead_faces, "free face list has {} entries for {dead_faces} dead faces", self.free_faces.len());
        ensure!(self.free_verts.iter().all(|&v| !self.is_vert_alive(v)), "free vertex list holds a live vertex");
        ensure!(self.free_edges.iter().all(|&e| (e as usize) < self.edge_slots() && !self.is_edge_alive(e)), "free edge list holds a live edge");
        ensure!(self.free_faces.iter().all(|&f| (f as usize) < nf && !self.is_face_alive(f)), "free face list holds a live face");

        self.validate_attributes()
    }

    /// Attribute column lengths and group contents.
    ///
    /// Complexity: O(slots) per group.
    fn validate_attributes(&self) -> Result<(), ValidationError> {
        let (nh, ne, nf) = (self.he_slots(), self.edge_slots(), self.face_slots());
        ensure!(self.corner_uv.len() <= MAX_UV_LAYERS, "{} UV layers", self.corner_uv.len());
        for (i, layer) in self.corner_uv.iter().enumerate() {
            ensure!(layer.len() == nh, "UV layer {i} has {} values for {nh} half-edges", layer.len());
        }
        if let Some(n) = &self.corner_normal {
            ensure!(n.len() == nh, "{} corner normals for {nh} half-edges", n.len());
        }
        if let Some(c) = &self.corner_color {
            ensure!(c.len() == nh, "{} corner colors for {nh} half-edges", c.len());
        }
        ensure!(self.face_material.len() == nf, "{} face materials for {nf} faces", self.face_material.len());
        ensure!(self.edge_flags.len() == ne, "{} edge flags for {ne} edges", self.edge_flags.len());
        if let Some(c) = &self.edge_crease {
            ensure!(c.len() == ne, "{} edge creases for {ne} edges", c.len());
        }
        for (name, g) in self.groups() {
            ensure!(!name.is_empty(), "group with an empty name");
            ensure!(g.bits().len() == self.slots(g.kind()), "group `{name}` has {} bits for {} slots", g.bits().len(), self.slots(g.kind()));
            if let Some(i) = g.iter().find(|&i| !self.is_alive(g.kind(), i)) {
                return Err(ValidationError(format!("group `{name}` holds dead {} element {i}", g.kind().as_str())));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{INVALID, Mesh, Vec3};

    fn two_quads() -> Mesh {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y, Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 0.0)];
        Mesh::from_polygons(p, &[4, 4], &[0, 1, 2, 3, 1, 4, 5, 2]).unwrap()
    }

    #[test]
    fn valid_mesh_passes() {
        two_quads().validate().unwrap();
    }

    #[test]
    fn broken_links_are_caught() {
        let mut m = two_quads();
        let h = m.face_he(0);
        let n = m.next(h);
        m.he_next[h as usize] = m.next(n);
        assert!(m.validate().is_err());
    }

    #[test]
    fn wrong_vert_he_is_caught() {
        let mut m = two_quads();
        // Vertex 1 is on the boundary; point it at an interior outgoing half-edge.
        let interior = m.vert_outgoing(1).find(|&h| !m.is_boundary_he(h)).unwrap();
        m.vert_he[1] = interior;
        let err = m.validate().unwrap_err();
        assert!(err.0.contains("vert_he is not on the boundary"), "{err}");
    }

    #[test]
    fn non_finite_position_is_caught() {
        let mut m = two_quads();
        m.positions[2] = Vec3::splat(f32::INFINITY);
        assert!(m.validate().is_err());
    }

    #[test]
    fn stale_free_list_is_caught() {
        let mut m = two_quads();
        m.free_faces.push(0);
        assert!(m.validate().is_err());
        let mut m = two_quads();
        m.face_he[1] = INVALID;
        assert!(m.validate().is_err());
    }
}
