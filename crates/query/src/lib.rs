//! Selector language and evaluation.
//!
//! Layer L1 (document). See `docs/architecture.md`.
//!
//! Selectors refer to geometry by meaning (groups, normals, flags, regions),
//! never by raw indices, so references survive edits:
//!
//! ```json
//! { "faces": { "group": "top" } }
//! { "faces": { "normal": { "dir": [0, 1, 0], "within_deg": 10 } } }
//! { "edges": { "sharper_than_deg": 30 } }
//! { "verts": { "in_box": { "min": [-1, 0, -1], "max": [0, 2, 1] } } }
//! { "and": [ { "faces": { "group": "side" } }, { "faces": { "normal": { "dir": [1, 0, 0], "within_deg": 45 } } } ] }
//! { "grow": { "steps": 1, "of": { "faces": { "group": "top" } } } }
//! ```

use glam::Vec3;
use p3d_mesh::{EdgeFlags, ElementKind, FixedBitSet, Mesh};
use p3d_scene::name::suggest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A set of mesh elements described by meaning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Selector {
    /// Faces matching every condition of the filter (all faces when empty).
    Faces(Filter),
    /// Edges matching every condition of the filter.
    Edges(Filter),
    /// Vertices matching every condition of the filter.
    Verts(Filter),
    /// Elements in every selector (all of the same kind).
    And(Vec<Selector>),
    /// Elements in any selector (all of the same kind).
    Or(Vec<Selector>),
    /// Elements of the same kind not in the selector.
    Not(Box<Selector>),
    /// Adds neighbours `steps` times: faces across edges, edges and vertices
    /// through shared vertices.
    Grow { steps: u32, of: Box<Selector> },
    /// Removes the outermost elements `steps` times (the inverse of grow).
    Shrink { steps: u32, of: Box<Selector> },
}

/// Conditions on elements; all given conditions must hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// Member of this named group (the group must hold this element kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Facing within an angle of a direction. Faces use the face normal,
    /// edges the mean of their faces' normals, vertices the mean of their
    /// faces' normals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<NormalFilter>,
    /// Faces with this material slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<u16>,
    /// Edges whose faces meet at more than this angle in degrees (0 is flat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharper_than_deg: Option<f32>,
    /// On the mesh boundary (true) or not (false). Faces: having a boundary edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<bool>,
    /// Edges marked sharp (true) or not (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharp: Option<bool>,
    /// Edges marked as UV seams (true) or not (false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seam: Option<bool>,
    /// Inside an axis-aligned box in object space: vertex positions, edge
    /// midpoints, face centroids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_box: Option<BoxFilter>,
    /// Element indices, valid only at the mesh revision they were read at
    /// (interactive selections in the app; prefer groups and filters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<u32>>,
}

/// Direction and tolerance for the `normal` filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalFilter {
    /// Direction in object space (need not be unit length).
    pub dir: [f32; 3],
    /// Largest angle from `dir` in degrees.
    pub within_deg: f32,
}

/// Axis-aligned box for the `in_box` filter, in meters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoxFilter {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Why a selector cannot be evaluated. The path is a JSON pointer into the
/// selector.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum QueryError {
    #[error("no group `{name}` (at {path})")]
    NoGroup { name: String, path: String, suggestions: Vec<String> },
    #[error("group `{name}` holds {has}, but the selector asks for {wanted} (at {path})")]
    GroupKind { name: String, has: &'static str, wanted: &'static str, path: String },
    #[error("`{field}` does not apply to {kind} (at {path})")]
    NotApplicable { field: &'static str, kind: &'static str, path: String },
    #[error("{what} (at {path})")]
    Invalid { what: String, path: String },
    #[error("cannot combine {a} with {b} (at {path})")]
    KindMismatch { a: &'static str, b: &'static str, path: String },
    #[error("`{op}` needs at least one selector (at {path})")]
    Empty { op: &'static str, path: String },
}

impl QueryError {
    /// JSON pointer into the selector.
    pub fn path(&self) -> &str {
        match self {
            QueryError::NoGroup { path, .. }
            | QueryError::GroupKind { path, .. }
            | QueryError::NotApplicable { path, .. }
            | QueryError::Invalid { path, .. }
            | QueryError::KindMismatch { path, .. }
            | QueryError::Empty { path, .. } => path,
        }
    }
}

/// Elements of one kind, as a bitset over element slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub kind: ElementKind,
    pub bits: FixedBitSet,
}

impl Selection {
    pub fn count(&self) -> usize {
        self.bits.count_ones(..)
    }

    pub fn is_empty(&self) -> bool {
        self.bits.is_clear()
    }

    /// Elements in index order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.ones().map(|i| i as u32)
    }
}

/// Evaluates a selector on a mesh.
///
/// Complexity: O(n) per filter in elements of its kind; grow and shrink add
/// O(n + H) whatever their step count.
pub fn select(mesh: &Mesh, selector: &Selector) -> Result<Selection, QueryError> {
    eval(mesh, selector, "")
}

fn eval(mesh: &Mesh, s: &Selector, path: &str) -> Result<Selection, QueryError> {
    match s {
        Selector::Faces(f) => filter(mesh, ElementKind::Face, f, &format!("{path}/faces")),
        Selector::Edges(f) => filter(mesh, ElementKind::Edge, f, &format!("{path}/edges")),
        Selector::Verts(f) => filter(mesh, ElementKind::Vert, f, &format!("{path}/verts")),
        Selector::And(list) | Selector::Or(list) => {
            let (name, is_and) = if matches!(s, Selector::And(_)) { ("and", true) } else { ("or", false) };
            let base = format!("{path}/{name}");
            let mut acc: Option<Selection> = None;
            for (i, sub) in list.iter().enumerate() {
                let p = format!("{base}/{i}");
                let next = eval(mesh, sub, &p)?;
                acc = Some(match acc {
                    None => next,
                    Some(mut a) => {
                        if a.kind != next.kind {
                            return Err(QueryError::KindMismatch { a: a.kind.as_str(), b: next.kind.as_str(), path: p });
                        }
                        if is_and {
                            a.bits.intersect_with(&next.bits);
                        } else {
                            a.bits.union_with(&next.bits);
                        }
                        a
                    }
                });
            }
            acc.ok_or(QueryError::Empty { op: name, path: base })
        }
        Selector::Not(inner) => {
            let mut sel = eval(mesh, inner, &format!("{path}/not"))?;
            sel.bits.toggle_range(..);
            clear_dead(mesh, &mut sel);
            Ok(sel)
        }
        Selector::Grow { steps, of } => {
            let mut sel = eval(mesh, of, &format!("{path}/grow/of"))?;
            grow(mesh, &mut sel, *steps);
            Ok(sel)
        }
        Selector::Shrink { steps, of } => {
            let mut sel = eval(mesh, of, &format!("{path}/shrink/of"))?;
            // shrink^s(S) = not(grow^s(not S)) within live elements: one pass.
            sel.bits.toggle_range(..);
            clear_dead(mesh, &mut sel);
            grow(mesh, &mut sel, *steps);
            sel.bits.toggle_range(..);
            clear_dead(mesh, &mut sel);
            Ok(sel)
        }
    }
}

/// Clears bits of dead elements.
///
/// Complexity: O(slots).
fn clear_dead(mesh: &Mesh, sel: &mut Selection) {
    for i in 0..sel.bits.len() {
        if sel.bits.contains(i) && !mesh.is_alive(sel.kind, i as u32) {
            sel.bits.set(i, false);
        }
    }
}

/// Grows a selection by up to `steps` rings of neighbours. Each step expands
/// only the elements added by the previous one (the frontier) and growing
/// stops once nothing new is added, so any step count costs at most one pass
/// over the reachable elements. Edges grow through vertices, and each vertex
/// fan is walked once.
///
/// Complexity: O(n + H) for any `steps`.
fn grow(mesh: &Mesh, sel: &mut Selection, steps: u32) {
    let mut frontier: Vec<u32> = sel.iter().collect();
    let mut next: Vec<u32> = Vec::new();
    let mut verts: Vec<u32> = Vec::new();
    for _ in 0..steps {
        if frontier.is_empty() {
            break;
        }
        next.clear();
        let add = |bits: &mut FixedBitSet, i: u32, next: &mut Vec<u32>| {
            if !bits.put(i as usize) {
                next.push(i);
            }
        };
        match sel.kind {
            ElementKind::Face => {
                for &f in &frontier {
                    for h in mesh.face_halfedges(f) {
                        if let Some(n) = mesh.face(h ^ 1) {
                            add(&mut sel.bits, n, &mut next);
                        }
                    }
                }
            }
            ElementKind::Vert => {
                for &v in &frontier {
                    for n in mesh.vert_neighbors(v) {
                        add(&mut sel.bits, n, &mut next);
                    }
                }
            }
            ElementKind::Edge => {
                // Every edge at a frontier edge's endpoints: walk each endpoint
                // fan once, however many frontier edges meet there.
                verts.clear();
                verts.extend(frontier.iter().flat_map(|&e| mesh.edge_verts(e)));
                verts.sort_unstable();
                verts.dedup();
                for &v in &verts {
                    for h in mesh.vert_outgoing(v) {
                        add(&mut sel.bits, h >> 1, &mut next);
                    }
                }
            }
        }
        std::mem::swap(&mut frontier, &mut next);
    }
}

fn kind_name(kind: ElementKind) -> &'static str {
    kind.as_str()
}

/// Mean of the face normals around a vertex or edge.
fn mean_normal(faces: impl Iterator<Item = u32>, face_n: &[Vec3]) -> Vec3 {
    faces.fold(Vec3::ZERO, |acc, f| acc + face_n[f as usize]).normalize_or_zero()
}

/// Complexity: O(slots of the kind + C) for normal tests on faces.
fn filter(mesh: &Mesh, kind: ElementKind, f: &Filter, path: &str) -> Result<Selection, QueryError> {
    let n = mesh.slots(kind);
    let mut bits = FixedBitSet::with_capacity(n);
    for i in 0..n as u32 {
        if mesh.is_alive(kind, i) {
            bits.insert(i as usize);
        }
    }
    let not_for = |field: &'static str| QueryError::NotApplicable { field, kind: kind_name(kind), path: format!("{path}/{field}") };

    if let Some(ids) = &f.ids {
        let mut keep = FixedBitSet::with_capacity(n);
        for (i, &x) in ids.iter().enumerate() {
            if !mesh.is_alive(kind, x) {
                return Err(QueryError::Invalid {
                    what: format!("{} {x} does not exist (ids are only valid at the revision they were read)", kind_name(kind)),
                    path: format!("{path}/ids/{i}"),
                });
            }
            keep.insert(x as usize);
        }
        bits.intersect_with(&keep);
    }
    if let Some(name) = &f.group {
        let g = mesh.group(name).ok_or_else(|| QueryError::NoGroup {
            name: name.clone(),
            path: format!("{path}/group"),
            suggestions: suggest(name, mesh.groups().map(|(n, _)| n), 3),
        })?;
        if g.kind() != kind {
            return Err(QueryError::GroupKind { name: name.clone(), has: g.kind().as_str(), wanted: kind_name(kind), path: format!("{path}/group") });
        }
        bits.intersect_with(g.bits());
    }
    if let Some(m) = f.material {
        if kind != ElementKind::Face {
            return Err(not_for("material"));
        }
        retain(&mut bits, |i| mesh.face_material(i) == m);
    }
    if let Some(on) = f.sharp {
        if kind != ElementKind::Edge {
            return Err(not_for("sharp"));
        }
        retain(&mut bits, |e| mesh.edge_flags(e).contains(EdgeFlags::SHARP) == on);
    }
    if let Some(on) = f.seam {
        if kind != ElementKind::Edge {
            return Err(not_for("seam"));
        }
        retain(&mut bits, |e| mesh.edge_flags(e).contains(EdgeFlags::SEAM) == on);
    }
    if let Some(on) = f.boundary {
        match kind {
            ElementKind::Vert => retain(&mut bits, |v| mesh.is_boundary_vert(v) == on),
            ElementKind::Edge => retain(&mut bits, |e| mesh.is_boundary_edge(e) == on),
            ElementKind::Face => retain(&mut bits, |fc| mesh.face_halfedges(fc).any(|h| mesh.is_boundary_he(h ^ 1)) == on),
        }
    }
    if let Some(b) = &f.in_box {
        let (lo, hi) = (Vec3::from(b.min), Vec3::from(b.max));
        if !(lo.is_finite() && hi.is_finite() && lo.cmple(hi).all()) {
            return Err(QueryError::Invalid { what: "in_box needs finite min <= max on every axis".into(), path: format!("{path}/in_box") });
        }
        let inside = |p: Vec3| p.cmpge(lo).all() && p.cmple(hi).all();
        match kind {
            ElementKind::Vert => retain(&mut bits, |v| inside(mesh.positions[v as usize])),
            ElementKind::Edge => retain(&mut bits, |e| {
                let [a, c] = mesh.edge_verts(e);
                inside((mesh.positions[a as usize] + mesh.positions[c as usize]) * 0.5)
            }),
            ElementKind::Face => retain(&mut bits, |fc| inside(mesh.face_centroid(fc))),
        }
    }
    if let Some(nf) = &f.normal {
        let dir = Vec3::from(nf.dir);
        if !dir.is_finite() || dir.length_squared() < 1e-12 {
            return Err(QueryError::Invalid { what: "normal.dir must be a non-zero vector".into(), path: format!("{path}/normal/dir") });
        }
        if !(0.0..=180.0).contains(&nf.within_deg) {
            return Err(QueryError::Invalid {
                what: "normal.within_deg must be between 0 and 180".into(),
                path: format!("{path}/normal/within_deg"),
            });
        }
        let dir = dir.normalize();
        let min_cos = nf.within_deg.to_radians().cos() - 1e-6;
        let face_n = mesh.face_normals();
        match kind {
            ElementKind::Face => retain(&mut bits, |fc| face_n[fc as usize].dot(dir) >= min_cos),
            ElementKind::Edge => retain(&mut bits, |e| mean_normal(mesh.edge_faces(e).into_iter().flatten(), &face_n).dot(dir) >= min_cos),
            ElementKind::Vert => retain(&mut bits, |v| mean_normal(mesh.vert_faces(v), &face_n).dot(dir) >= min_cos),
        }
    }
    if let Some(deg) = f.sharper_than_deg {
        if kind != ElementKind::Edge {
            return Err(not_for("sharper_than_deg"));
        }
        if !(0.0..=180.0).contains(&deg) {
            return Err(QueryError::Invalid { what: "sharper_than_deg must be between 0 and 180".into(), path: format!("{path}/sharper_than_deg") });
        }
        let face_n = mesh.face_normals();
        let max_cos = deg.to_radians().cos();
        retain(&mut bits, |e| match mesh.edge_faces(e) {
            [Some(a), Some(b)] => face_n[a as usize].dot(face_n[b as usize]) < max_cos,
            _ => false,
        });
    }
    Ok(Selection { kind, bits })
}

/// Keeps set bits for which `keep` holds.
///
/// Complexity: O(slots / 64 + set bits).
fn retain(bits: &mut FixedBitSet, keep: impl Fn(u32) -> bool) {
    let drop: Vec<usize> = bits.ones().filter(|&i| !keep(i as u32)).collect();
    for i in drop {
        bits.set(i, false);
    }
}

#[cfg(test)]
mod tests;
