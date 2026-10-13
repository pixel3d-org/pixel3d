//! The Model Spec compiler (ADR 0010). Each part is placed (transform or
//! anchor on its parent's surface), then built through ops in a scratch
//! document and cached by content hash; blended SDF children join their
//! parent's tree; mirrors reflect their source. Compiled parts become mesh
//! objects under the spec's root, and the record keeps per-face provenance.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

use glam::{Affine3A, EulerRot, Quat, Vec3};
use p3d_math::Trs;
use p3d_mesh::{ElementKind, Mesh};
use p3d_ops::sdf::{SdfPrimParam, SdfShapeParam, mesh_items, shape_items};
use p3d_ops::{ErrorCode, OpCtx, OpError, Registry, Tx};
use p3d_scene::{Document, ObjectKey, ObjectKind, PartRecord, ProvEntry, Provenance, SpecRecord};
use p3d_sdf::{Combine, Item, Modifiers, Prim, Sdf, Shape};
use serde::Serialize;
use serde_json::Value;

use crate::landmarks;
use crate::model::{Anchor, Axis, Join, Kind, Landmarks, ModelSpec, Orient, Part, parse};
use crate::surface::{Hit, Surface};

/// Bumped when the compiled output of an unchanged spec changes, so cached
/// parts rebuild.
const COMPILER_VERSION: u64 = 1;
/// Fillet of a blend join when the part gives none, meters.
const DEFAULT_BLEND: f32 = 0.01;
/// Depth limit of landmarks defined through other landmarks.
const MAX_LANDMARK_DEPTH: u32 = 8;

/// How a part fared in a compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartStatus {
    /// Its mesh was built.
    Built,
    /// Its mesh was kept (same content hash); it may have moved.
    Unchanged,
    /// Merged into the named part's mesh.
    Blended(String),
}

/// One part of a compile report.
#[derive(Debug, Clone)]
pub struct PartReport {
    pub name: String,
    pub status: PartStatus,
    /// `sdf`, `box`, `lathe`, `mirror`, `instance`, ...
    pub kind: String,
    pub faces: usize,
    /// Size and center of the part in the root's space.
    pub size: Vec3,
    pub center: Vec3,
}

/// What a compile did.
#[derive(Debug, Clone, Default)]
pub struct CompileReport {
    pub parts: Vec<PartReport>,
    /// Parts gone from the spec whose objects were deleted.
    pub removed: Vec<String>,
    pub warnings: Vec<String>,
    pub created: Vec<ObjectKey>,
    pub changed: Vec<ObjectKey>,
    pub deleted: Vec<u32>,
}

impl CompileReport {
    pub fn built(&self) -> usize {
        self.parts.iter().filter(|p| p.status == PartStatus::Built).count()
    }
}

/// FNV-1a over everything that shapes a part.
struct Hasher(u64);

impl Hasher {
    fn new() -> Hasher {
        let mut h = Hasher(0xCBF2_9CE4_8422_2325);
        h.u64(COMPILER_VERSION);
        h
    }

    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x0100_0000_01B3);
        }
        // A separator, so ("ab", "c") and ("a", "bc") differ.
        self.0 ^= 0xFF;
        self.0 = self.0.wrapping_mul(0x0100_0000_01B3);
    }

    fn u64(&mut self, x: u64) {
        self.bytes(&x.to_le_bytes());
    }

    fn json<T: Serialize + ?Sized>(&mut self, v: &T) {
        // Serializing plain data cannot fail; an empty string still hashes.
        self.bytes(serde_json::to_string(v).unwrap_or_default().as_bytes());
    }

    fn floats(&mut self, f: &[f32]) {
        for x in f {
            self.bytes(&x.to_bits().to_le_bytes());
        }
    }
}

/// A content fingerprint of a mesh: counts, positions and faces.
///
/// Complexity: O(V + C).
pub fn fingerprint(mesh: &Mesh) -> u64 {
    let mut h = Hasher::new();
    h.u64(mesh.vert_count() as u64);
    h.u64(mesh.face_count() as u64);
    let mut buf = Vec::with_capacity(mesh.vert_count() * 12);
    for v in mesh.verts() {
        for c in mesh.positions[v as usize].to_array() {
            buf.extend_from_slice(&c.to_bits().to_le_bytes());
        }
    }
    for f in mesh.faces() {
        for v in mesh.face_verts(f) {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.push(0xFE);
    }
    h.bytes(&buf);
    h.0
}

fn err(path: &str, message: impl Into<String>) -> OpError {
    OpError::invalid(&format!("/spec{path}"), message)
}

fn v3(p: &[p3d_ops::params::Length; 3]) -> Vec3 {
    Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32)
}

/// The reflection across the plane normal to `axis` through the root.
fn reflection(axis: Axis) -> Vec3 {
    let mut s = Vec3::ONE;
    s[axis.index()] = -1.0;
    s
}

/// M R M for the reflection M across the plane normal to `axis`: the
/// quaternion components of the other two axes flip.
fn mirror_quat(q: Quat, axis: Axis) -> Quat {
    let mut v = Vec3::new(q.x, q.y, q.z) * -reflection(axis);
    v[axis.index()] = q.to_array()[axis.index()];
    Quat::from_xyzw(v.x, v.y, v.z, q.w).normalize()
}

/// A placement reflected across a root plane: M t, M R M, same scale.
fn mirror_trs(t: Trs, axis: Axis) -> Trs {
    Trs { translation: t.translation * reflection(axis), rotation: mirror_quat(t.rotation, axis), scale: t.scale }
}

/// SDF items reflected across their space's plane normal to `axis`:
/// centers and rotations become M c and M R M, capsule ends reflect in the
/// item's frame, twist (and bend for x) turn the other way. Items that
/// already mirror across that plane are symmetric and stay. Noise is not
/// reflected.
///
/// Complexity: O(items).
fn mirror_items(items: &[Item], axis: Axis) -> Vec<Item> {
    let m = reflection(axis);
    items
        .iter()
        .map(|it| {
            if it.modifiers.mirror[axis.index()] {
                return it.clone();
            }
            let mut out = it.clone();
            out.center = it.center * m;
            out.rotation = mirror_quat(it.rotation, axis);
            out.shape = match &it.shape {
                Shape::Group(inner) => Shape::Group(mirror_items(inner, axis)),
                Shape::Prim(Prim::Capsule { a, b, ra, rb }) => Shape::Prim(Prim::Capsule { a: *a * m, b: *b * m, ra: *ra, rb: *rb }),
                other => other.clone(),
            };
            out.modifiers.twist = -it.modifiers.twist;
            if axis == Axis::X {
                out.modifiers.bend = -it.modifiers.bend;
            }
            out
        })
        .collect()
}

/// The direction of surface parameter (u, v): u around +Y from +Z toward
/// +X, v from the bottom to the top.
fn param_dir(u: f64, v: f64) -> Vec3 {
    let y = (2.0 * v - 1.0).clamp(-1.0, 1.0) as f32;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let a = (u as f32) * std::f32::consts::TAU;
    Vec3::new(r * a.sin(), y, r * a.cos())
}

/// Face group names of SDF prims with the spec paths of their shapes.
type LeafGroups = Vec<(String, String)>;

/// A built part mesh.
struct Built {
    mesh: Arc<Mesh>,
    provenance: Provenance,
    hash: u64,
    cached: bool,
}

/// The registry when the caller gave none (tests, direct calls).
fn fallback() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(crate::registry)
}

struct Compiler<'a> {
    spec: &'a ModelSpec,
    old: &'a BTreeMap<String, PartRecord>,
    doc: &'a Document,
    root: ObjectKey,
    registry: &'a Registry,
    cancel: Option<&'a AtomicBool>,
    force: bool,
    /// Blend children by parent, sorted.
    blends: BTreeMap<String, Vec<String>>,
    trs: BTreeMap<String, Trs>,
    items: BTreeMap<String, Arc<Vec<Item>>>,
    surfaces: BTreeMap<String, Arc<Surface>>,
    built: BTreeMap<String, Arc<Built>>,
    /// Nodes being resolved, for loop detection.
    active: Vec<String>,
    warnings: Vec<String>,
}

impl<'a> Compiler<'a> {
    fn part(&self, name: &str, path: &str) -> Result<&'a Part, OpError> {
        self.spec.parts.get(name).ok_or_else(|| {
            let mut e = err(path, format!("there is no part `{name}`"));
            e.suggestions = p3d_scene::name::suggest(name, self.spec.parts.keys().map(String::as_str), 3);
            e
        })
    }

    /// The SDF part a part's shapes come from (itself, or the end of a
    /// mirror chain), or none.
    fn sdf_source(&self, name: &str) -> Option<&'a str> {
        let mut n = self.spec.parts.get_key_value(name)?.0.as_str();
        for _ in 0..=self.spec.parts.len() {
            let part = self.spec.parts.get(n)?;
            match &part.mirror {
                Some(src) => n = self.spec.parts.get_key_value(src.as_str())?.0.as_str(),
                None => return part.sdf.as_ref().map(|_| n),
            }
        }
        None
    }

    fn seed(&self, name: &str) -> u64 {
        let mut h = Hasher::new();
        h.u64(self.spec.seed);
        h.bytes(name.as_bytes());
        h.0
    }

    fn enter(&mut self, node: String, path: &str) -> Result<(), OpError> {
        if let Some(i) = self.active.iter().position(|n| *n == node) {
            let chain: Vec<&str> = self.active[i..].iter().map(|n| n.split_once(' ').map_or(n.as_str(), |(_, p)| p)).collect();
            return Err(err(path, format!("parts depend on each other in a loop: {} -> {}", chain.join(" -> "), chain[0])));
        }
        self.active.push(node);
        Ok(())
    }

    fn leave(&mut self) {
        self.active.pop();
    }

    // ---- placement ---------------------------------------------------------

    /// The part's placement in the root's space.
    fn placement(&mut self, name: &str) -> Result<Trs, OpError> {
        if let Some(t) = self.trs.get(name) {
            return Ok(*t);
        }
        let path = format!("/parts/{name}");
        let part = self.part(name, &path)?;
        self.enter(format!("place {name}"), &path)?;
        let r = self.place(name, part, &path);
        self.leave();
        let t = r?;
        if !t.is_finite() {
            return Err(err(&path, "the placement is not finite"));
        }
        self.trs.insert(name.to_owned(), t);
        Ok(t)
    }

    fn place(&mut self, name: &str, part: &'a Part, path: &str) -> Result<Trs, OpError> {
        if let Kind::Mirror(src) = part.kind(path).map_err(spec_err)? {
            if part.anchor.is_some() || part.transform.is_some() {
                return Err(err(path, "a mirror sits opposite its source: leave out anchor and transform"));
            }
            self.part(src, &format!("{path}/mirror"))?;
            let s = self.placement(src)?;
            return Ok(mirror_trs(s, part.plane.unwrap_or_default()));
        }
        let t = part.transform.clone().unwrap_or_default();
        let rot = t.rotate.map_or(Quat::IDENTITY, |r| Quat::from_euler(EulerRot::XYZ, r[0].0 as f32, r[1].0 as f32, r[2].0 as f32));
        let scale = t.scale.map_or(Vec3::ONE, |s| Vec3::from(s.vec().map(|x| x as f32)));
        if !(scale.is_finite() && scale.min_element() > 0.0) {
            return Err(err(&format!("{path}/transform/scale"), "scale must be positive (use a mirror part to reflect)"));
        }
        let Some(a) = &part.anchor else {
            return Ok(Trs { translation: t.position.as_ref().map_or(Vec3::ZERO, v3), rotation: rot, scale });
        };
        if t.position.is_some() {
            return Err(err(&format!("{path}/transform/position"), "an anchored part takes its position from the anchor; use anchor.offset"));
        }
        let apath = format!("{path}/anchor");
        let on = a.on.as_deref().ok_or_else(|| err(&apath, "say which part it sits on, for example \"on\": \"head\""))?;
        if on == name {
            return Err(err(&format!("{apath}/on"), "a part cannot sit on itself"));
        }
        self.part(on, &format!("{apath}/on"))?;
        let hit = self.resolve(on, a, &apath, 0)?;
        let parent = self.placement(on)?;
        let frame = match a.orient.unwrap_or_default() {
            Orient::Parent => parent.rotation,
            Orient::Normal => Quat::from_rotation_arc(Vec3::Y, hit.normal),
        };
        let offset = a.offset.as_ref().map_or(Vec3::ZERO, v3);
        Ok(Trs { translation: hit.point + frame * offset, rotation: frame * rot, scale })
    }

    /// A surface coordinate on part `on`.
    fn resolve(&mut self, on: &str, a: &Anchor, path: &str, depth: u32) -> Result<Hit, OpError> {
        let given = [a.landmark.is_some(), a.param.is_some(), a.ray.is_some(), a.nearest.is_some()].iter().filter(|x| **x).count();
        if given != 1 {
            return Err(err(path, "give one of landmark, param, ray or nearest"));
        }
        let miss = |what: &str| err(path, format!("{what} does not reach the surface of `{on}`"));
        if let Some(l) = &a.landmark {
            return self.landmark(on, l, &format!("{path}/landmark"), depth);
        }
        let surface = self.surface(on)?;
        if let Some([u, v]) = a.param {
            return surface.toward(param_dir(u, v)).ok_or_else(|| miss("the param"));
        }
        if let Some(n) = &a.nearest {
            return surface.nearest(v3(n)).ok_or_else(|| miss("nearest"));
        }
        if let Some(r) = &a.ray {
            let (lo, hi) = surface.root_bounds();
            let center = (lo + hi) * 0.5;
            let far = (hi - lo).length() + 1.0;
            let (axis, sign) = match r.from.as_str() {
                "+x" => (0, 1.0),
                "-x" => (0, -1.0),
                "+y" => (1, 1.0),
                "-y" => (1, -1.0),
                "+z" => (2, 1.0),
                "-z" => (2, -1.0),
                other => return Err(err(&format!("{path}/ray/from"), format!("`{other}` is not a side; use +x, -x, +y, -y, +z or -z"))),
            };
            let mut through = r.through.as_ref().map_or(center, v3);
            if r.through.is_none()
                && let Some(h) = r.height
            {
                through.y = h.0 as f32;
            }
            let mut dir = Vec3::ZERO;
            dir[axis] = -sign;
            let mut origin = through;
            origin[axis] = center[axis] + sign * far;
            return surface.ray(origin, dir).ok_or_else(|| miss("the ray"));
        }
        Err(err(path, "give one of landmark, param, ray or nearest"))
    }

    /// A landmark of part `on`: its own named ones, `center`, the category's,
    /// the generic extremes, then its named SDF shapes.
    fn landmark(&mut self, on: &str, name: &str, path: &str, depth: u32) -> Result<Hit, OpError> {
        let part = self.part(on, path)?;
        if let Some(Landmarks::Named(map)) = &part.landmarks
            && let Some(a) = map.get(name)
        {
            if depth >= MAX_LANDMARK_DEPTH {
                return Err(err(path, format!("landmark `{name}` is defined through too many other landmarks")));
            }
            let mut a = a.clone();
            let target = a.on.take().unwrap_or_else(|| on.to_owned());
            let hit = self.resolve(&target, &a, &format!("/parts/{on}/landmarks/{name}"), depth + 1)?;
            let shift = a.offset.as_ref().map_or(Vec3::ZERO, v3);
            return Ok(Hit { point: hit.point + self.placement(on)?.rotation * shift, normal: hit.normal });
        }
        let surface = self.surface(on)?;
        if name == "center" {
            let (lo, hi) = surface.part_bounds();
            return Ok(Hit { point: surface.to_root().transform_point3((lo + hi) * 0.5), normal: self.placement(on)?.rotation * Vec3::Y });
        }
        let category = self.category_of(part);
        if let Some(d) = landmarks::direction(name, category.as_deref()) {
            return surface.toward(Vec3::from(d)).ok_or_else(|| err(path, format!("landmark `{name}` misses the surface of `{on}`")));
        }
        if let Some(center) = self.shape_center(on, name)? {
            let (lo, hi) = surface.part_bounds();
            let mid = (lo + hi) * 0.5;
            let out = (center - mid).try_normalize().unwrap_or(Vec3::Y);
            let far = center + out * (hi - lo).length();
            let to_root = surface.to_root();
            let (o, c) = (to_root.transform_point3(far), to_root.transform_point3(center));
            return surface.ray(o, c - o).ok_or_else(|| err(path, format!("landmark `{name}` misses the surface of `{on}`")));
        }
        let mut known: Vec<String> = vec!["center".into()];
        known.extend(landmarks::GENERIC.iter().map(|(n, _)| (*n).to_owned()));
        if let Some(t) = category.as_deref().and_then(landmarks::category) {
            known.extend(t.iter().map(|(n, _)| (*n).to_owned()));
        }
        if let Some(Landmarks::Named(map)) = &part.landmarks {
            known.extend(map.keys().cloned());
        }
        let mut e = err(path, format!("`{on}` has no landmark `{name}`"));
        e.suggestions = p3d_scene::name::suggest(name, known.iter().map(String::as_str), 3);
        if category.is_none() && landmarks::CATEGORIES.iter().any(|c| landmarks::category(c).is_some_and(|t| t.iter().any(|(n, _)| *n == name))) {
            e.hint = Some(format!("`{name}` is a category landmark: give the part \"landmarks\": \"{}\"", landmarks::CATEGORIES[0]));
        }
        Err(e)
    }

    fn category_of(&self, part: &Part) -> Option<String> {
        match &part.landmarks {
            Some(Landmarks::Category(c)) if c == "category" => self.spec.category.clone(),
            Some(Landmarks::Category(c)) => Some(c.clone()),
            _ => None,
        }
    }

    /// The center of a named prim of an SDF part (part space).
    fn shape_center(&mut self, on: &str, name: &str) -> Result<Option<Vec3>, OpError> {
        let part = self.part(on, "")?;
        if part.sdf.is_none() {
            return Ok(None);
        }
        let items = self.items(on)?;
        fn find(items: &[Item], name: &str, frame: Affine3A) -> Option<Vec3> {
            for it in items {
                let here = frame * Affine3A::from_rotation_translation(it.rotation, it.center);
                match &it.shape {
                    Shape::Group(inner) => {
                        if let Some(c) = find(inner, name, here) {
                            return Some(c);
                        }
                    }
                    _ if it.name.as_deref() == Some(name) => return Some(here.transform_point3(Vec3::ZERO)),
                    _ => {}
                }
            }
            None
        }
        Ok(find(&items, name, Affine3A::IDENTITY))
    }

    // ---- surfaces and items -----------------------------------------------

    fn surface(&mut self, name: &str) -> Result<Arc<Surface>, OpError> {
        if let Some(s) = self.surfaces.get(name) {
            return Ok(Arc::clone(s));
        }
        let path = format!("/parts/{name}");
        let part = self.part(name, &path)?;
        let place = self.placement(name)?.to_affine();
        part.kind(&path).map_err(spec_err)?;
        let s = match self.sdf_source(name) {
            // An SDF part (or its mirror) is queried on its own field
            // (blended children left out), so anchoring on it needs no mesh.
            Some(_) => {
                let items = self.items(name)?;
                let sdf = Sdf::new((*items).clone()).map_err(|e| err(&format!("{path}/sdf"), e.to_string()))?;
                Surface::of_sdf(Arc::new(sdf), place)
            }
            _ => {
                let b = self.mesh(name)?;
                Surface::of_mesh(&b.mesh, place).ok_or_else(|| err(&path, "the part has no faces to anchor on"))?
            }
        };
        let s = Arc::new(s);
        self.surfaces.insert(name.to_owned(), Arc::clone(&s));
        Ok(s)
    }

    fn items(&mut self, name: &str) -> Result<Arc<Vec<Item>>, OpError> {
        if let Some(i) = self.items.get(name) {
            return Ok(Arc::clone(i));
        }
        let part = self.part(name, &format!("/parts/{name}"))?;
        let items = match &part.mirror {
            Some(src) => Arc::new(mirror_items(&self.items(src)?, part.plane.unwrap_or_default())),
            None => {
                let shapes = part.sdf.as_deref().unwrap_or(&[]);
                Arc::new(shape_items(shapes, self.seed(name), &format!("/spec/parts/{name}/sdf"))?)
            }
        };
        self.items.insert(name.to_owned(), Arc::clone(&items));
        Ok(items)
    }

    // ---- meshes --------------------------------------------------------------

    fn mesh(&mut self, name: &str) -> Result<Arc<Built>, OpError> {
        if let Some(b) = self.built.get(name) {
            return Ok(Arc::clone(b));
        }
        let path = format!("/parts/{name}");
        let part = self.part(name, &path)?;
        self.enter(format!("mesh {name}"), &path)?;
        let r = self.build(name, part, &path);
        self.leave();
        let b = r?;
        self.built.insert(name.to_owned(), Arc::clone(&b));
        Ok(b)
    }

    fn build(&mut self, name: &str, part: &'a Part, path: &str) -> Result<Arc<Built>, OpError> {
        match part.kind(path).map_err(spec_err)? {
            Kind::Instance(src) => {
                self.part(src, &format!("{path}/instance"))?;
                if !part.edits.is_empty() {
                    return Err(err(&format!("{path}/edits"), "an instance shares its source's mesh; put the edits on the source"));
                }
                self.mesh(src)
            }
            Kind::Mirror(src) => {
                let s = self.mesh(src)?;
                let axis = part.plane.unwrap_or_default();
                let mut h = Hasher::new();
                h.bytes(b"mirror");
                h.u64(axis.index() as u64);
                h.u64(s.hash);
                h.json(&part.edits);
                let hash = h.0;
                if let Some(c) = self.cached(name, hash) {
                    return Ok(c);
                }
                let mut m = (*s.mesh).clone();
                m.transform(Affine3A::from_scale(reflection(axis)));
                let entries = s.provenance.entries.clone();
                let per_face = s.provenance.per_face();
                self.finish(name, path, m, entries, &per_face, hash)
            }
            Kind::Sdf(_) => self.build_sdf(name, part, path),
            Kind::Primitive(kind, args) => {
                self.build_op(name, part, path, &format!("prim.{kind}"), args, &format!("{path}/primitive/{kind}"), "primitive")
            }
            Kind::Op(key, op, args) => self.build_op(name, part, path, op, args, &format!("{path}/{key}"), key),
        }
    }

    /// The record's mesh when nothing that shapes the part changed.
    fn cached(&mut self, name: &str, hash: u64) -> Option<Arc<Built>> {
        if self.force {
            return None;
        }
        let rec = self.old.get(name)?;
        if rec.hash != hash {
            return None;
        }
        let key = self.doc.object_by_id(rec.object?)?;
        let o = self.doc.object(key)?;
        if o.parent() != Some(self.root) {
            return None;
        }
        let mesh = Arc::clone(self.doc.mesh_arc(o.mesh_key()?)?);
        if fingerprint(&mesh) != rec.fingerprint {
            self.warnings.push(format!("{name}: its mesh was edited outside the spec and is rebuilt; keep manual work in the part's edits"));
            return None;
        }
        Some(Arc::new(Built { mesh, provenance: rec.provenance.clone(), hash, cached: true }))
    }

    fn sub_ctx(&self, seed: u64, label: Option<&'a str>) -> OpCtx<'a> {
        OpCtx { seed, label, cancel: self.cancel, progress: None, files: None, registry: Some(self.registry) }
    }

    /// Runs an op in a scratch document; errors point into the spec.
    fn run(&self, tx: &mut Tx, op: &str, args: &Value, path: &str, seed: u64, label: Option<&'a str>) -> Result<p3d_ops::Applied, OpError> {
        if op.starts_with("spec.") {
            return Err(err(path, "a spec cannot run spec ops inside itself"));
        }
        let o = self.registry.get(op).map_err(|e| {
            let mut e2 = err(path, e.message.clone());
            e2.suggestions = e.suggestions;
            e2
        })?;
        o.run(tx, args, &self.sub_ctx(seed, label)).map_err(|mut e| {
            e.path = Some(format!("/spec{path}{}", e.path.as_deref().unwrap_or("")));
            e
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_op(
        &mut self,
        name: &str,
        part: &'a Part,
        path: &str,
        op: &str,
        args: &Value,
        args_path: &str,
        key: &str,
    ) -> Result<Arc<Built>, OpError> {
        let Value::Object(map) = args else {
            return Err(err(args_path, "give the parameters as an object"));
        };
        for k in ["name", "parent", "position", "target"] {
            if map.contains_key(k) {
                return Err(err(&format!("{args_path}/{k}"), format!("the spec places parts: leave out `{k}`")));
            }
        }
        let mut args = map.clone();
        args.insert("name".into(), Value::String("part".into()));
        let args = Value::Object(args);
        let mut h = Hasher::new();
        h.bytes(op.as_bytes());
        h.json(&args);
        h.json(&part.edits);
        h.u64(self.seed(name));
        let hash = h.0;
        if let Some(c) = self.cached(name, hash) {
            return Ok(c);
        }
        let mut tx = Tx::new(Document::new());
        self.run(&mut tx, op, &args, args_path, self.seed(name), None)?;
        let pk = tx.resolve("part", args_path)?;
        let entries = vec![ProvEntry { path: format!("/parts/{name}/{key}"), op: op.to_owned() }];
        self.edit_and_finish(name, part, path, tx, pk, entries, Vec::new(), hash)
    }

    /// The SDF tree of a part with its blended children (recursively) in
    /// the part's space, leaf names prefixed with `prefix`, and the face
    /// group of each leaf with its spec path.
    fn sdf_tree(&mut self, name: &str, prefix: &str, h: &mut Hasher) -> Result<(Vec<Item>, LeafGroups), OpError> {
        let path = format!("/parts/{name}");
        let part = self.part(name, &path)?;
        let source = self.sdf_source(name).ok_or_else(|| err(&format!("{path}/join"), "blend joins need SDF parts on both sides"))?;
        let shapes = self.spec.parts[source].sdf.as_deref().unwrap_or(&[]);
        let mut items = (*self.items(name)?).clone();
        if !prefix.is_empty() {
            rename(&mut items, prefix);
        }
        let mut groups = Vec::new();
        let mut leaf = 0usize;
        leaf_groups(shapes, &format!("/parts/{source}/sdf"), prefix, &mut leaf, &mut groups);
        h.bytes(name.as_bytes());
        h.json(shapes);
        h.json(&part.plane);
        h.u64(self.seed(source));
        let place = self.placement(name)?.to_affine();
        for child in self.blends.get(name).cloned().unwrap_or_default() {
            let c = self.part(&child, &format!("/parts/{child}"))?;
            let rel = Trs::from_affine(&(place.inverse() * self.placement(&child)?.to_affine()))
                .ok_or_else(|| err(&format!("/parts/{child}/anchor"), "the placement cannot be expressed in the parent's space"))?;
            if (rel.scale - Vec3::ONE).abs().max_element() > 1e-4 {
                return Err(err(
                    &format!("/parts/{child}/transform/scale"),
                    "a blended part cannot be scaled (distances would warp); scale its shapes",
                ));
            }
            let (inner, inner_groups) = self.sdf_tree(&child, &format!("{prefix}{child}."), h)?;
            let blend = self.sdf_source(&child).and_then(|s| self.spec.parts[s].blend).or(c.blend);
            let k = c.blend.or(blend).map_or(DEFAULT_BLEND, |b| b.0 as f32);
            h.floats(&[k, rel.translation.x, rel.translation.y, rel.translation.z, rel.rotation.x, rel.rotation.y, rel.rotation.z, rel.rotation.w]);
            items.push(Item {
                shape: Shape::Group(inner),
                center: rel.translation,
                rotation: rel.rotation,
                combine: Combine::SmoothUnion(k),
                modifiers: Modifiers::default(),
                name: None,
            });
            groups.extend(inner_groups);
        }
        Ok((items, groups))
    }

    fn build_sdf(&mut self, name: &str, part: &'a Part, path: &str) -> Result<Arc<Built>, OpError> {
        let mut h = Hasher::new();
        h.bytes(b"sdf");
        h.json(&part.detail);
        h.json(&part.edits);
        let (items, groups) = self.sdf_tree(name, "", &mut h)?;
        let hash = h.0;
        if let Some(c) = self.cached(name, hash) {
            return Ok(c);
        }
        let detail = part.detail.map(|d| d.0 as f32);
        let ctx = self.sub_ctx(self.seed(name), None);
        let meshed = mesh_items(items, detail, &format!("/spec{path}/sdf"), &ctx)?;
        for w in meshed.warnings {
            self.warnings.push(format!("{name}: {}", w.message));
        }
        let mut tx = Tx::new(Document::new());
        let pk = tx.doc_mut().create_mesh_object("part", None, meshed.mesh).map_err(|e| OpError::internal(e.to_string()))?;
        let mut entries = vec![ProvEntry { path: format!("/parts/{name}/sdf"), op: "sdf.mesh".into() }];
        let mut by_group = Vec::with_capacity(groups.len());
        for (group, spath) in groups {
            by_group.push((group, entries.len() as u32));
            entries.push(ProvEntry { path: spath, op: "sdf.mesh".into() });
        }
        self.edit_and_finish(name, part, path, tx, pk, entries, by_group, hash)
    }

    /// Runs the part's edits on its scratch object, then records provenance.
    #[allow(clippy::too_many_arguments)]
    fn edit_and_finish(
        &mut self,
        name: &str,
        part: &'a Part,
        path: &str,
        mut tx: Tx,
        pk: ObjectKey,
        mut entries: Vec<ProvEntry>,
        mut by_group: Vec<(String, u32)>,
        hash: u64,
    ) -> Result<Arc<Built>, OpError> {
        for (k, e) in part.edits.iter().enumerate() {
            let epath = format!("{path}/edits/{k}");
            let mut args = match &e.args {
                Value::Object(m) => m.clone(),
                Value::Null => serde_json::Map::new(),
                _ => return Err(err(&format!("{epath}/args"), "give the op's arguments as an object")),
            };
            if args.contains_key("target") {
                return Err(err(&format!("{epath}/args/target"), "edits act on their part: leave out `target`"));
            }
            args.insert("target".into(), Value::String("part".into()));
            let applied = self.run(
                &mut tx,
                &e.op,
                &Value::Object(args),
                &format!("{epath}/args"),
                self.seed(name).wrapping_add(k as u64 + 1),
                e.label.as_deref(),
            )?;
            let entry = entries.len() as u32;
            entries.push(ProvEntry { path: epath, op: e.op.clone() });
            for g in applied.output.groups.iter().filter(|g| g.kind == ElementKind::Face) {
                by_group.push((g.name.clone(), entry));
            }
        }
        let mesh = tx.doc().object_mesh(pk).cloned().ok_or_else(|| err(path, "the part's edits removed its mesh"))?;
        let mut slot = vec![u32::MAX; mesh.face_slots()];
        for (i, f) in mesh.faces().enumerate() {
            slot[f as usize] = i as u32;
        }
        let mut per_face = vec![0u32; mesh.face_count()];
        for (group, entry) in &by_group {
            if let Some(g) = mesh.group(group).filter(|g| g.kind() == ElementKind::Face) {
                for f in g.iter() {
                    if let Some(&i) = slot.get(f as usize)
                        && i != u32::MAX
                    {
                        per_face[i as usize] = *entry;
                    }
                }
            }
        }
        self.finish(name, path, mesh, entries, &per_face, hash)
    }

    fn finish(&self, name: &str, path: &str, mesh: Mesh, entries: Vec<ProvEntry>, per_face: &[u32], hash: u64) -> Result<Arc<Built>, OpError> {
        if mesh.face_count() == 0 {
            return Err(err(path, format!("part `{name}` has no faces")));
        }
        Ok(Arc::new(Built { mesh: Arc::new(mesh), provenance: Provenance::from_faces(entries, per_face), hash, cached: false }))
    }
}

fn spec_err(e: crate::SpecError) -> OpError {
    err(&e.path, e.message)
}

/// Prefixes the names of every prim (`item_<n>` becomes `shape_<n>`).
///
/// Complexity: O(items).
fn rename(items: &mut [Item], prefix: &str) {
    for it in items {
        match &mut it.shape {
            Shape::Group(inner) => rename(inner, prefix),
            _ => {
                let base = it.name.as_deref().map(|n| n.strip_prefix("item_").map_or_else(|| n.to_owned(), |k| format!("shape_{k}")));
                it.name = Some(format!("{prefix}{}", base.unwrap_or_default()));
            }
        }
    }
}

/// Face group names of every prim in leaf order, with their spec paths
/// (hidden prims count, as in `sdf.mesh`).
///
/// Complexity: O(shapes).
fn leaf_groups(shapes: &[SdfShapeParam], path: &str, prefix: &str, leaf: &mut usize, out: &mut Vec<(String, String)>) {
    for (i, s) in shapes.iter().enumerate() {
        let here = format!("{path}/{i}");
        match &s.prim {
            SdfPrimParam::Group(inner) => leaf_groups(inner, &format!("{here}/group"), prefix, leaf, out),
            _ => {
                let base = s.options.name.clone().unwrap_or_else(|| format!("shape_{leaf}"));
                out.push((format!("{prefix}{base}"), here));
                *leaf += 1;
            }
        }
    }
}

/// Blend children by the part they blend into, following mirror chains
/// (a mirror joins like its source); checks every join.
///
/// Complexity: O(parts x mirror chain length).
fn blend_targets(spec: &ModelSpec) -> Result<BTreeMap<String, Vec<String>>, OpError> {
    let mut blends: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, part) in &spec.parts {
        let path = format!("/parts/{name}");
        part.kind(&path).map_err(spec_err)?;
        if part.mirror.is_some() && part.join.is_some() {
            return Err(err(&format!("{path}/join"), "a mirror joins like its source: leave out join"));
        }
        // A mirror joins like the end of its mirror chain.
        let mut source = part;
        let mut target_of_mirror = None;
        for _ in 0..spec.parts.len() {
            let Some(src) = &source.mirror else { break };
            source = spec.parts.get(src).ok_or_else(|| err(&format!("{path}/mirror"), format!("there is no part `{src}`")))?;
            target_of_mirror = source.anchor.as_ref().and_then(|a| a.on.clone());
        }
        match source.join() {
            Join::Weld => return Err(err(&format!("{path}/join"), "weld joins are not built yet; use blend (SDF parts) or separate")),
            Join::Blend => {
                let on = match &part.mirror {
                    Some(_) => target_of_mirror,
                    None => part.anchor.as_ref().and_then(|a| a.on.clone()),
                }
                .ok_or_else(|| err(&format!("{path}/join"), "a blend joins the part it is anchored on: give an anchor"))?;
                // The parent must be an SDF part too (or a mirror of one).
                let mut target = spec.parts.get(&on);
                for _ in 0..spec.parts.len() {
                    match target.and_then(|t| t.mirror.as_ref()) {
                        Some(m) => target = spec.parts.get(m),
                        None => break,
                    }
                }
                if source.sdf.is_none() || target.is_some_and(|t| t.sdf.is_none()) {
                    return Err(err(&format!("{path}/join"), "blend joins need SDF parts on both sides"));
                }
                blends.entry(on).or_default().push(name.clone());
            }
            Join::Separate | Join::Decal => {}
        }
    }
    Ok(blends)
}

/// Compiles the spec on `root` into part objects under it, reusing parts
/// whose content hash is unchanged (`force` rebuilds all).
///
/// Complexity: the parts' own build costs for changed parts, plus O(V + C)
/// per part for fingerprints and O(parts log parts) bookkeeping.
pub fn compile(tx: &mut Tx, root: ObjectKey, ctx: &OpCtx, force: bool) -> Result<CompileReport, OpError> {
    let record: SpecRecord =
        tx.doc().spec(root).cloned().ok_or_else(|| OpError::new(ErrorCode::InvalidParam, "the object has no spec").at("/target"))?;
    let spec = parse(&record.source).map_err(spec_err)?;
    let registry: &Registry = match ctx.registry {
        Some(r) => r,
        None => fallback(),
    };
    let blends = blend_targets(&spec)?;
    let report = {
        let doc = tx.doc();
        let mut c = Compiler {
            spec: &spec,
            old: &record.parts,
            doc,
            root,
            registry,
            cancel: ctx.cancel,
            force,
            blends,
            trs: BTreeMap::new(),
            items: BTreeMap::new(),
            surfaces: BTreeMap::new(),
            built: BTreeMap::new(),
            active: Vec::new(),
            warnings: Vec::new(),
        };
        for name in spec.parts.keys() {
            c.placement(name)?;
        }
        // Blend targets first need their children's placements (done).
        let mut into: BTreeMap<String, String> = BTreeMap::new();
        for (parent, kids) in &c.blends {
            for k in kids {
                into.insert(k.clone(), parent.clone());
            }
        }
        for name in spec.parts.keys() {
            if !into.contains_key(name) {
                c.mesh(name)?;
                ctx.check_cancel()?;
            }
        }
        // Blended into the first ancestor that has an object.
        let owner = |mut n: String| {
            while let Some(p) = into.get(&n) {
                n = p.clone();
            }
            n
        };
        Plan {
            trs: c.trs,
            built: c.built,
            into: spec.parts.keys().filter(|n| into.contains_key(*n)).map(|n| (n.clone(), owner(n.clone()))).collect(),
            warnings: c.warnings,
        }
    };
    apply(tx, root, &spec, &record, report)
}

/// What the compiler decided, before it touches the document.
struct Plan {
    trs: BTreeMap<String, Trs>,
    built: BTreeMap<String, Arc<Built>>,
    into: BTreeMap<String, String>,
    warnings: Vec<String>,
}

fn kind_name(part: &Part) -> String {
    match part.kind("") {
        Ok(Kind::Sdf(_)) => "sdf".into(),
        Ok(Kind::Primitive(k, _)) => k.to_owned(),
        Ok(Kind::Op(k, _, _)) => k.to_owned(),
        Ok(Kind::Mirror(s)) => format!("mirror of {s}"),
        Ok(Kind::Instance(s)) => format!("instance of {s}"),
        Err(_) => "?".into(),
    }
}

/// Writes the plan into the document: part objects, meshes, transforms and
/// the new record.
fn apply(tx: &mut Tx, root: ObjectKey, spec: &ModelSpec, record: &SpecRecord, plan: Plan) -> Result<CompileReport, OpError> {
    let mut report = CompileReport { warnings: plan.warnings, ..CompileReport::default() };
    let mut parts: BTreeMap<String, PartRecord> = BTreeMap::new();
    let internal = |e: p3d_scene::SceneError| OpError::internal(e.to_string());
    // The object a part had, if it is still a child of the root.
    let existing = |doc: &Document, name: &str| -> Option<ObjectKey> {
        let by_record = record
            .parts
            .get(name)
            .and_then(|r| r.object)
            .and_then(|id| doc.object_by_id(id))
            .filter(|&k| doc.object(k).is_some_and(|o| o.parent() == Some(root)));
        by_record.or_else(|| {
            doc.child_named(Some(root), name).filter(|&k| record.parts.contains_key(name) || doc.object(k).is_some_and(|o| o.mesh_key().is_some()))
        })
    };
    // Sources before instances, so instances can share their mesh keys.
    let mut order: Vec<&String> = spec.parts.keys().collect();
    order.sort_by_key(|n| matches!(spec.parts[*n].kind(""), Ok(Kind::Instance(_))));
    let mut keys: BTreeMap<String, ObjectKey> = BTreeMap::new();
    for name in order {
        let part = &spec.parts[name];
        let trs = plan.trs.get(name).copied().unwrap_or(Trs::IDENTITY);
        if let Some(owner) = plan.into.get(name) {
            if let Some(k) = existing(tx.doc(), name)
                && record.parts.get(name).is_some_and(|r| r.object.is_some())
            {
                report.deleted.extend(tx.doc_mut().delete(k, false).map_err(internal)?);
            }
            parts.insert(name.clone(), PartRecord::default());
            report.parts.push(PartReport {
                name: name.clone(),
                status: PartStatus::Blended(owner.clone()),
                kind: kind_name(part),
                faces: 0,
                size: Vec3::ZERO,
                center: trs.translation,
            });
            continue;
        }
        let built = plan.built.get(name).ok_or_else(|| OpError::internal(format!("part {name} was not built")))?;
        let instance_of = match part.kind("") {
            Ok(Kind::Instance(src)) => keys.get(src).copied(),
            _ => None,
        };
        let key = match existing(tx.doc(), name) {
            Some(k) => k,
            None => {
                let k = tx.doc_mut().create_object(name, Some(root), ObjectKind::Empty).map_err(|e| err(&format!("/parts/{name}"), e.to_string()))?;
                report.created.push(k);
                k
            }
        };
        let current = tx.doc().object(key).and_then(|o| o.mesh_key());
        let same = current.and_then(|mk| tx.doc().mesh_arc(mk)).is_some_and(|m| Arc::ptr_eq(m, &built.mesh));
        let mesh_key = match instance_of.and_then(|src| tx.doc().object(src)?.mesh_key()) {
            Some(mk) => Some(mk),
            None if same => current,
            None => None,
        };
        let mk = match mesh_key {
            Some(mk) => mk,
            None => tx.doc_mut().add_shared_mesh(Arc::clone(&built.mesh)),
        };
        if current != Some(mk) {
            tx.doc_mut().set_kind(key, ObjectKind::Mesh { mesh: mk }).map_err(internal)?;
            tx.touch(key);
            if !report.created.contains(&key) {
                report.changed.push(key);
            }
        }
        let decal = part.join() == Join::Decal;
        let o = tx.doc_mut().object_mut(key).ok_or_else(|| OpError::internal("part object vanished"))?;
        if o.transform != trs && !report.changed.contains(&key) && !report.created.contains(&key) {
            report.changed.push(key);
        }
        o.transform = trs;
        o.visible = !decal;
        let mut tags: Vec<String> = part.tags.clone();
        if decal {
            tags.push("decal".into());
        }
        o.tags = tags.into_iter().collect();
        let id = o.id();
        keys.insert(name.clone(), key);
        let (lo, hi) = built.mesh.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
        let a = trs.to_affine();
        let corners = (0..8).map(|i| {
            a.transform_point3(Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            ))
        });
        let (wlo, whi) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(l, h), c| (l.min(c), h.max(c)));
        report.parts.push(PartReport {
            name: name.clone(),
            status: if built.cached { PartStatus::Unchanged } else { PartStatus::Built },
            kind: kind_name(part),
            faces: built.mesh.face_count(),
            size: whi - wlo,
            center: (wlo + whi) * 0.5,
        });
        parts.insert(
            name.clone(),
            PartRecord { hash: built.hash, object: Some(id), fingerprint: fingerprint(&built.mesh), provenance: built.provenance.clone() },
        );
    }
    // Parts gone from the spec take their objects with them.
    let gone: BTreeSet<&String> = record.parts.keys().filter(|n| !spec.parts.contains_key(*n)).collect();
    for name in gone {
        if let Some(id) = record.parts[name].object
            && let Some(k) = tx.doc().object_by_id(id)
            && tx.doc().object(k).is_some_and(|o| o.parent() == Some(root))
        {
            report.deleted.extend(tx.doc_mut().delete(k, false).map_err(internal)?);
        }
        report.removed.push(name.clone());
    }
    report.parts.sort_by(|a, b| a.name.cmp(&b.name));
    tx.doc_mut().set_spec(root, Some(SpecRecord { source: record.source.clone(), parts })).map_err(internal)?;
    Ok(report)
}

/// A read-only compiler over an already compiled spec: every part is a
/// cache hit, so queries need no rebuild.
fn query<'a>(doc: &'a Document, root: ObjectKey, spec: &'a ModelSpec, record: &'a SpecRecord) -> Result<Compiler<'a>, OpError> {
    Ok(Compiler {
        spec,
        old: &record.parts,
        doc,
        root,
        registry: fallback(),
        cancel: None,
        force: false,
        blends: blend_targets(spec)?,
        trs: BTreeMap::new(),
        items: BTreeMap::new(),
        surfaces: BTreeMap::new(),
        built: BTreeMap::new(),
        active: Vec::new(),
        warnings: Vec::new(),
    })
}

/// A landmark of a part: its name, and its point and normal in world space.
#[derive(Debug, Clone, PartialEq)]
pub struct LandmarkPoint {
    pub name: String,
    pub point: Vec3,
    pub normal: Vec3,
}

/// Every landmark of a part of the spec on `root`: `center`, the generic
/// extremes, its category's, its own named ones and its named SDF shapes,
/// in world space. Landmarks that miss the surface are left out.
///
/// Complexity: one surface query per landmark, plus the part's surface
/// (a BVH build for mesh parts).
pub fn part_landmarks(doc: &Document, root: ObjectKey, part: &str) -> Result<Vec<LandmarkPoint>, OpError> {
    let record = doc.spec(root).ok_or_else(|| OpError::new(ErrorCode::InvalidParam, "the object has no spec").at("/target"))?;
    let spec = parse(&record.source).map_err(spec_err)?;
    let mut c = query(doc, root, &spec, record)?;
    let p = c.part(part, &format!("/parts/{part}"))?;
    let mut names: Vec<String> = vec!["center".into()];
    names.extend(landmarks::GENERIC.iter().map(|(n, _)| (*n).to_owned()));
    if let Some(t) = c.category_of(p).as_deref().and_then(landmarks::category) {
        names.extend(t.iter().map(|(n, _)| (*n).to_owned()));
    }
    if let Some(Landmarks::Named(map)) = &p.landmarks {
        names.extend(map.keys().cloned());
    }
    if let Some(source) = c.sdf_source(part) {
        let mut groups = Vec::new();
        leaf_groups(spec.parts[source].sdf.as_deref().unwrap_or(&[]), "", "", &mut 0, &mut groups);
        names.extend(groups.into_iter().map(|(g, _)| g).filter(|g| !g.starts_with("shape_")));
    }
    let mut seen = BTreeSet::new();
    names.retain(|n| seen.insert(n.clone()));
    let world = doc.world_affine(root).unwrap_or(Affine3A::IDENTITY);
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if let Ok(h) = c.landmark(part, &name, "", 0) {
            out.push(LandmarkPoint {
                name,
                point: world.transform_point3(h.point),
                normal: world.transform_vector3(h.normal).normalize_or(h.normal),
            });
        }
    }
    Ok(out)
}

/// The spec node that made a face of a compiled part object: the part's
/// name and the provenance entry, or none for objects outside specs and
/// meshes edited since their compile.
///
/// Complexity: O(faces) to find the face's position, plus O(runs).
pub fn face_provenance(doc: &Document, object: ObjectKey, face: u32) -> Option<(String, ProvEntry)> {
    let o = doc.object(object)?;
    let record = doc.spec(o.parent()?)?;
    let part = record.parts.get(o.name())?;
    let mesh = doc.object_mesh(object)?;
    if part.provenance.face_count() != mesh.face_count() {
        return None;
    }
    let i = mesh.faces().position(|f| f == face)?;
    part.provenance.entry_of(i).map(|e| (o.name().to_owned(), e.clone()))
}
