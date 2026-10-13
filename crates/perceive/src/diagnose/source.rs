//! Where a part comes from: the Model Spec part it was compiled from, the
//! spec node that made a face (provenance), and fixes written as spec
//! patches (project spec 17.10: causes are spec paths, fixes spec patches).

use glam::Vec3;
use p3d_scene::{Document, ObjectKey, PartRecord, ProvEntry};
use serde_json::{Value, json};

use super::Fix;
use crate::view::View;

/// A part object compiled from a Model Spec.
#[derive(Debug, Clone, Copy)]
pub struct SpecPart<'a> {
    /// The spec's root object and its path (the `target` of `spec.patch`).
    pub root: ObjectKey,
    /// The part's name in the spec.
    pub name: &'a str,
    /// The part's JSON as written.
    pub json: &'a Value,
    pub record: &'a PartRecord,
}

impl SpecPart<'_> {
    /// A JSON pointer into the spec under this part: `/parts/<name>/<rest>`.
    pub fn path(&self, rest: &str) -> String {
        if rest.is_empty() { format!("/parts/{}", self.name) } else { format!("/parts/{}/{rest}", self.name) }
    }

    /// The value at a pointer below the part (`params/clumps`).
    pub fn get(&self, rest: &str) -> Option<&Value> {
        self.json.pointer(&format!("/{rest}"))
    }
}

/// The spec part an object was compiled from, if any.
///
/// Complexity: O(log parts).
pub fn spec_part(doc: &Document, key: ObjectKey) -> Option<SpecPart<'_>> {
    let o = doc.object(key)?;
    let root = o.parent()?;
    let record = doc.spec(root)?;
    let (name, part) = record.parts.get_key_value(o.name())?;
    if part.object != Some(o.id()) {
        return None;
    }
    let json = record.source.get("parts")?.get(name.as_str())?;
    Some(SpecPart { root, name: name.as_str(), json, record: part })
}

/// The part whose shape a part copies: the source of a `mirror` or
/// `instance` (followed through chains), else the part itself. Edit layers
/// and shape parameters of copies belong there.
///
/// Complexity: O(chain length x log parts).
pub fn shape_source<'a>(doc: &'a Document, sp: SpecPart<'a>) -> SpecPart<'a> {
    let mut cur = sp;
    for _ in 0..8 {
        let src = cur.get("mirror").or_else(|| cur.get("instance")).and_then(Value::as_str);
        match src.and_then(|s| part_object(doc, cur.root, s)).and_then(|k| spec_part(doc, k)) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// The object of a named part of the spec on `root`.
///
/// Complexity: O(log parts).
pub fn part_object(doc: &Document, root: ObjectKey, name: &str) -> Option<ObjectKey> {
    doc.object_by_id(doc.spec(root)?.parts.get(name)?.object?)
}

/// The spec root's path, for `spec.patch` targets.
pub fn root_path(doc: &Document, root: ObjectKey) -> String {
    doc.path(root).unwrap_or_default()
}

/// The provenance entry of the face of part `i` nearest a world point, with
/// the spec root's name as a prefix for the text (`elf `).
///
/// Complexity: one nearest-face query, plus O(faces) to number the face
/// when the mesh has unused slots, plus O(runs).
pub fn provenance_at(view: &View, i: usize, at: Vec3) -> Option<(String, ProvEntry)> {
    let part = &view.parts[i];
    let sp = spec_part(view.doc, part.key)?;
    let prov = &sp.record.provenance;
    let m = part.mesh;
    if prov.entries.is_empty() || prov.face_count() != m.face_count() {
        return None;
    }
    let c = part.bvh().closest(at, part.size().length().max(1e-3) * 2.0)?;
    let index = if m.has_holes_in_storage() { m.faces().position(|f| f == c.face)? } else { c.face as usize };
    let entry = prov.entry_of(index)?.clone();
    let root = view.doc.object(sp.root).map_or_else(String::new, |o| format!("{} ", o.name()));
    Some((root, entry))
}

/// JSON Patch operations that set `value` at `rest` below a spec part,
/// creating missing objects on the way (`add` replaces existing keys).
///
/// Complexity: O(depth of `rest`).
pub fn set_ops(sp: &SpecPart, rest: &str, value: Value) -> Vec<Value> {
    let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    // The deepest existing object on the way.
    let mut have = 0;
    let mut cur = sp.json;
    for s in &segs[..segs.len().saturating_sub(1)] {
        match cur.get(*s) {
            Some(v) if v.is_object() => {
                cur = v;
                have += 1;
            }
            _ => break,
        }
    }
    let mut v = value;
    for s in segs[have + 1..].iter().rev() {
        v = json!({ *s: v });
    }
    let path = sp.path(&segs[..=have.min(segs.len().saturating_sub(1))].join("/"));
    vec![json!({ "op": "add", "path": path, "value": v })]
}

/// A `spec.patch` fix with JSON Patch operations.
pub fn patch_fix(doc: &Document, root: ObjectKey, ops: Vec<Value>, summary: impl Into<String>) -> Fix {
    Fix::op("spec.patch", json!({ "target": root_path(doc, root), "ops": ops }), summary)
}

/// A fix running `op` on part `i` of a view: an edit layer of its spec
/// part when it has one (`sp`), else the op on the object.
pub fn part_fix(view: &View, i: usize, sp: Option<&SpecPart>, op: &str, args: Value, summary: &str) -> Fix {
    match sp {
        Some(sp) => edit_fix(view.doc, sp, op, args, format!("{summary} (an edit layer)")),
        None => {
            let mut args = args;
            if let Some(a) = args.as_object_mut() {
                a.insert("target".into(), json!(view.parts[i].path));
            }
            Fix::op(op, args, summary)
        }
    }
}

/// A fix appending an edit layer (an op run on the part's mesh after its
/// shape) to a spec part, or to the part it mirrors or instances.
pub fn edit_fix(doc: &Document, sp: &SpecPart, op: &str, args: Value, summary: impl Into<String>) -> Fix {
    let own = sp.name;
    let sp = &shape_source(doc, *sp);
    let mut summary = summary.into();
    if sp.name != own {
        summary.push_str(&format!(" on {}, which {own} copies", sp.name));
    }
    let edit = if args.as_object().is_some_and(|a| !a.is_empty()) { json!({ "op": op, "args": args }) } else { json!({ "op": op }) };
    let ops = if sp.json.get("edits").and_then(Value::as_array).is_some() {
        vec![json!({ "op": "add", "path": sp.path("edits/-"), "value": edit })]
    } else {
        vec![json!({ "op": "add", "path": sp.path("edits"), "value": [edit] })]
    };
    patch_fix(doc, sp.root, ops, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_ops_create_missing_objects() {
        let json = json!({ "primitive": { "cylinder": { "radius": 0.04 } } });
        let sp = SpecPart { root: ObjectKey::default(), name: "arm", json: &json, record: &PartRecord::default() };
        let ops = set_ops(&sp, "primitive/cylinder/advanced/height_segments", json!(8));
        assert_eq!(ops, vec![json!({ "op": "add", "path": "/parts/arm/primitive/cylinder/advanced", "value": { "height_segments": 8 } })]);
        let ops = set_ops(&sp, "primitive/cylinder/radius", json!(0.05));
        assert_eq!(ops, vec![json!({ "op": "add", "path": "/parts/arm/primitive/cylinder/radius", "value": 0.05 })]);
        let ops = set_ops(&sp, "transform/scale", json!([1.1, 1.0, 1.0]));
        assert_eq!(ops, vec![json!({ "op": "add", "path": "/parts/arm/transform", "value": { "scale": [1.1, 1.0, 1.0] } })]);
    }
}
