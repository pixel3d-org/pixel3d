//! What gets drawn: world-space triangles with normals and part ids, the
//! polygon edges for wireframes (not the triangulation's diagonals), and
//! each part's name and bounds for labels.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use p3d_mesh::TriOptions;
use p3d_scene::{Document, ObjectKey};

/// A triangle-list vertex.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub part: u32,
}

/// A line-list vertex.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 3],
    pub part: u32,
}

/// A part to label.
#[derive(Debug, Clone, PartialEq)]
pub struct PartLabel {
    /// The object this part draws (the viewport maps picks back to it).
    pub object: ObjectKey,
    pub name: String,
    /// Area-weighted center of its surface (world).
    pub center: Vec3,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// Everything one render needs, in world space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderScene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub lines: Vec<LineVertex>,
    pub parts: Vec<PartLabel>,
    pub lo: Vec3,
    pub hi: Vec3,
}

impl RenderScene {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Visible mesh objects under `root` (the whole scene when `None`), parents
/// first, as one world-space scene. Leaf names label the parts.
///
/// Complexity: O(objects + total mesh size).
pub fn prepare(doc: &Document, root: Option<ObjectKey>) -> RenderScene {
    let keys: Vec<ObjectKey> = match root {
        Some(r) => doc.subtree(r),
        None => doc.roots().flat_map(|k| doc.subtree(k)).collect(),
    };
    let mut s = RenderScene { lo: Vec3::splat(f32::INFINITY), hi: Vec3::splat(f32::NEG_INFINITY), ..RenderScene::default() };
    for key in keys {
        let (Some(obj), Some(mesh)) = (doc.object(key), doc.object_mesh(key)) else { continue };
        if !obj.visible || mesh.face_count() == 0 {
            continue;
        }
        let world = doc.world_affine(key).unwrap_or_default();
        let normal_m = world.matrix3.inverse().transpose();
        let part = s.parts.len() as u32;
        let tb = mesh.to_tri_buffers(&TriOptions { normals: true, uvs: false, colors: false });
        let base = s.vertices.len() as u32;
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for (p, n) in tb.positions.iter().zip(&tb.normals) {
            let w = world.transform_point3(*p);
            lo = lo.min(w);
            hi = hi.max(w);
            let wn = (normal_m * *n).normalize_or_zero();
            s.vertices.push(Vertex { pos: w.to_array(), normal: wn.to_array(), part });
        }
        // A mirroring transform flips the winding; keep triangles facing out.
        let flip = world.matrix3.determinant() < 0.0;
        let (corners, _) = tb.indices.as_chunks::<3>();
        let (mut center, mut area) = (Vec3::ZERO, 0.0f32);
        for t in corners {
            let [a, b, c] = *t;
            let (pa, pb, pc) = (s.vertices[(base + a) as usize].pos, s.vertices[(base + b) as usize].pos, s.vertices[(base + c) as usize].pos);
            let (pa, pb, pc) = (Vec3::from(pa), Vec3::from(pb), Vec3::from(pc));
            let ar = 0.5 * (pb - pa).cross(pc - pa).length();
            center += (pa + pb + pc) / 3.0 * ar;
            area += ar;
            if flip {
                s.indices.extend([base + a, base + c, base + b]);
            } else {
                s.indices.extend([base + a, base + b, base + c]);
            }
        }
        for e in mesh.edges() {
            let [a, b] = mesh.edge_verts(e);
            for v in [a, b] {
                s.lines.push(LineVertex { pos: world.transform_point3(mesh.positions[v as usize]).to_array(), part });
            }
        }
        s.lo = s.lo.min(lo);
        s.hi = s.hi.max(hi);
        let center = if area > 0.0 { center / area } else { (lo + hi) / 2.0 };
        s.parts.push(PartLabel { object: key, name: obj.name().to_owned(), center, lo, hi });
    }
    if s.parts.is_empty() {
        s.lo = Vec3::ZERO;
        s.hi = Vec3::ZERO;
    }
    s
}
