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
    /// Strands (hair cards, fur): how far along the strand, 0 at the root to
    /// 1 at the tip (the UV v); negative for other surfaces.
    pub strand: f32,
}

/// [`Vertex::strand`] of surfaces that are not strands.
pub const NOT_STRAND: f32 = -1.0;

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
        // Cards (hair, foliage) are drawn two-sided.
        let strand = if obj.tags.iter().any(|t| t == "strands" || t == "vegetation") { 0.0 } else { NOT_STRAND };
        for (p, n) in tb.positions.iter().zip(&tb.normals) {
            let w = world.transform_point3(*p);
            lo = lo.min(w);
            hi = hi.max(w);
            let wn = (normal_m * *n).normalize_or_zero();
            s.vertices.push(Vertex { pos: w.to_array(), normal: wn.to_array(), part, strand });
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

/// One drawable part of a [`ViewScene`]: an object and its mesh bounds in
/// object space.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPart {
    pub object: ObjectKey,
    pub lo: Vec3,
    pub hi: Vec3,
}

/// The viewport's geometry: every visible mesh object in object space, with
/// its part index in the vertices. World transforms are separate (one
/// matrix per part), so moving an object never rebuilds geometry. The
/// meshes are kept alive so their pointers identify the geometry exactly
/// (a mesh edit replaces the shared `Arc`).
#[derive(Debug, Clone, Default)]
pub struct ViewScene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub lines: Vec<LineVertex>,
    pub parts: Vec<ViewPart>,
    pub meshes: Vec<std::sync::Arc<p3d_mesh::Mesh>>,
}

impl ViewScene {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether this scene was built from exactly these meshes.
    ///
    /// Complexity: O(parts).
    pub fn same_meshes(&self, list: &[(ObjectKey, std::sync::Arc<p3d_mesh::Mesh>)]) -> bool {
        self.parts.len() == list.len()
            && self.parts.iter().zip(&self.meshes).zip(list).all(|((p, m), (k, n))| p.object == *k && std::sync::Arc::ptr_eq(m, n))
    }
}

/// Visible mesh objects with faces, parents first (the viewport's parts).
///
/// Complexity: O(objects).
pub fn view_meshes(doc: &Document) -> Vec<(ObjectKey, std::sync::Arc<p3d_mesh::Mesh>)> {
    let mut out = Vec::new();
    for root in doc.roots() {
        for key in doc.subtree(root) {
            let Some(obj) = doc.object(key) else { continue };
            if !obj.visible {
                continue;
            }
            if let Some(m) = obj.mesh_key().and_then(|mk| doc.mesh_arc(mk))
                && m.face_count() > 0
            {
                out.push((key, std::sync::Arc::clone(m)));
            }
        }
    }
    out
}

/// Builds the object-space geometry of `list` (from [`view_meshes`]);
/// objects in `strands` (hair cards, fur) carry how far along the strand
/// each vertex is, from their first UV layer's v.
///
/// Complexity: O(total mesh size + parts x log strands).
pub fn prepare_view(list: &[(ObjectKey, std::sync::Arc<p3d_mesh::Mesh>)], strands: &std::collections::BTreeSet<ObjectKey>) -> ViewScene {
    let mut s = ViewScene::default();
    for (part, (key, mesh)) in list.iter().enumerate() {
        let part = part as u32;
        let strand = strands.contains(key);
        let tb = mesh.to_tri_buffers(&TriOptions { normals: true, uvs: strand, colors: false });
        let base = s.vertices.len() as u32;
        let along = tb.uvs.first().filter(|uv| strand && uv.len() == tb.positions.len());
        s.vertices.extend(tb.positions.iter().zip(&tb.normals).enumerate().map(|(i, (p, n))| Vertex {
            pos: p.to_array(),
            normal: n.to_array(),
            part,
            strand: along.map_or(if strand { 1.0 } else { NOT_STRAND }, |uv| uv[i].y.clamp(0.0, 1.0)),
        }));
        s.indices.extend(tb.indices.iter().map(|i| base + i));
        for e in mesh.edges() {
            let [a, b] = mesh.edge_verts(e);
            s.lines.push(LineVertex { pos: mesh.positions[a as usize].to_array(), part });
            s.lines.push(LineVertex { pos: mesh.positions[b as usize].to_array(), part });
        }
        let (lo, hi) = mesh.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
        s.parts.push(ViewPart { object: *key, lo, hi });
        s.meshes.push(std::sync::Arc::clone(mesh));
    }
    s
}

/// World matrix of every part of `parts`.
///
/// Complexity: O(parts x depth).
pub fn part_matrices(doc: &Document, parts: &[ViewPart]) -> Vec<glam::Mat4> {
    parts.iter().map(|p| glam::Mat4::from(doc.world_affine(p.object).unwrap_or_default())).collect()
}

/// World bounds of a part under `model`.
pub fn world_bounds(part: &ViewPart, model: glam::Mat4) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { part.lo.x } else { part.hi.x },
            if i & 2 == 0 { part.lo.y } else { part.hi.y },
            if i & 4 == 0 { part.lo.z } else { part.hi.z },
        );
        let w = model.transform_point3(c);
        lo = lo.min(w);
        hi = hi.max(w);
    }
    (lo, hi)
}

/// Bit of [`LineVertex::part`] that marks a selected element in an
/// [`EditOverlay`].
pub const EDIT_SELECTED: u32 = 1 << 31;

/// The mesh being edited, drawn over the scene in the element selection
/// modes: its edges (two vertices each), its vertices (one each, drawn as
/// dots) and its selected faces as triangles (three vertices each). Object
/// space; `part` is the part index (for its transform), with
/// [`EDIT_SELECTED`] set on selected elements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditOverlay {
    pub lines: Vec<LineVertex>,
    pub points: Vec<LineVertex>,
    pub faces: Vec<LineVertex>,
}

/// The selected elements an [`EditOverlay`] highlights, by mesh id.
#[derive(Debug, Clone, Copy)]
pub struct ElementSets<'a> {
    pub verts: &'a std::collections::BTreeSet<u32>,
    pub edges: &'a std::collections::BTreeSet<u32>,
    pub faces: &'a std::collections::BTreeSet<u32>,
}

/// Builds the overlay of `mesh`, drawn as part `part`; `points` adds the
/// vertex dots (vertex mode).
///
/// Complexity: O((V + E) log s + corners of the selected faces), s the
/// selection size.
pub fn edit_overlay(mesh: &p3d_mesh::Mesh, part: u32, sel: ElementSets<'_>, points: bool) -> EditOverlay {
    let at = |v: u32| mesh.positions.get(v as usize).map_or([0.0; 3], |p| p.to_array());
    let flag = |on: bool| if on { part | EDIT_SELECTED } else { part };
    let mut out = EditOverlay { lines: Vec::with_capacity(mesh.edge_count() * 2), ..Default::default() };
    for e in mesh.edges() {
        let [a, b] = mesh.edge_verts(e);
        let p = flag(sel.edges.contains(&e));
        out.lines.push(LineVertex { pos: at(a), part: p });
        out.lines.push(LineVertex { pos: at(b), part: p });
    }
    if points {
        out.points = mesh.verts().map(|v| LineVertex { pos: at(v), part: flag(sel.verts.contains(&v)) }).collect();
    }
    let mut corners = Vec::new();
    for &f in sel.faces {
        if !mesh.is_alive(p3d_mesh::ElementKind::Face, f) {
            continue;
        }
        corners.clear();
        corners.extend(mesh.face_verts(f));
        // A fan is enough for a tint over convex and mildly concave faces.
        for i in 1..corners.len().saturating_sub(1) {
            for v in [corners[0], corners[i], corners[i + 1]] {
                out.faces.push(LineVertex { pos: at(v), part: part | EDIT_SELECTED });
            }
        }
    }
    out
}
