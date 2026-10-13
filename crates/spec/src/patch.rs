//! Patching a spec's JSON: JSON Merge Patch (RFC 7386) for the common case
//! and JSON Patch (RFC 6902) for precise edits such as one array element.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::SpecError;

/// Merges `patch` into `target`: objects merge key by key, `null` removes
/// a key, anything else replaces.
///
/// Complexity: O(size of the patch) plus the replaced values.
pub fn merge(target: &mut Value, patch: &Value) {
    let Value::Object(p) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(serde_json::Map::new());
    }
    if let Value::Object(t) = target {
        for (k, v) in p {
            if v.is_null() {
                t.remove(k);
            } else {
                merge(t.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
    }
}

/// One JSON Patch operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchOp {
    /// `add`, `remove`, `replace`, `move`, `copy` or `test`.
    pub op: String,
    /// JSON pointer into the spec, for example `/parts/ear_l/blend`.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Source pointer of `move` and `copy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Pointer segments with `~1` and `~0` unescaped.
fn segments(pointer: &str) -> Result<Vec<String>, String> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let rest = pointer.strip_prefix('/').ok_or_else(|| format!("`{pointer}` is not a JSON pointer (it must start with /)"))?;
    Ok(rest.split('/').map(|s| s.replace("~1", "/").replace("~0", "~")).collect())
}

/// The parent container of a pointer and the last segment.
fn parent<'v>(doc: &'v mut Value, pointer: &str) -> Result<(&'v mut Value, String), String> {
    let mut segs = segments(pointer)?;
    let last = segs.pop().ok_or("the whole spec cannot be the target; use a merge patch")?;
    let mut cur = doc;
    for s in &segs {
        cur = match cur {
            Value::Object(m) => m.get_mut(s).ok_or_else(|| format!("`{s}` does not exist"))?,
            Value::Array(a) => {
                let i: usize = s.parse().map_err(|_| format!("`{s}` is not an array index"))?;
                a.get_mut(i).ok_or_else(|| format!("index {i} is past the end"))?
            }
            _ => return Err(format!("`{s}` is inside a value that is not an object or array")),
        };
    }
    Ok((cur, last))
}

fn add(doc: &mut Value, pointer: &str, value: Value) -> Result<(), String> {
    let (p, last) = parent(doc, pointer)?;
    match p {
        Value::Object(m) => {
            m.insert(last, value);
            Ok(())
        }
        Value::Array(a) => {
            let i = if last == "-" { a.len() } else { last.parse().map_err(|_| format!("`{last}` is not an array index"))? };
            if i > a.len() {
                return Err(format!("index {i} is past the end"));
            }
            a.insert(i, value);
            Ok(())
        }
        _ => Err("the parent is not an object or array".into()),
    }
}

fn remove(doc: &mut Value, pointer: &str) -> Result<Value, String> {
    let (p, last) = parent(doc, pointer)?;
    match p {
        Value::Object(m) => m.remove(&last).ok_or_else(|| format!("`{last}` does not exist")),
        Value::Array(a) => {
            let i: usize = last.parse().map_err(|_| format!("`{last}` is not an array index"))?;
            if i >= a.len() {
                return Err(format!("index {i} is past the end"));
            }
            Ok(a.remove(i))
        }
        _ => Err("the parent is not an object or array".into()),
    }
}

/// Applies JSON Patch operations in order; the first failing one stops and
/// leaves `doc` unchanged. Errors point at `/ops/<i>`.
///
/// Complexity: O(ops x pointer depth) plus the values moved.
pub fn apply(doc: &mut Value, ops: &[PatchOp]) -> Result<(), SpecError> {
    let mut work = doc.clone();
    for (i, o) in ops.iter().enumerate() {
        let at = |m: String| SpecError::new(&format!("/ops/{i}"), m);
        let value = || o.value.clone().ok_or_else(|| at(format!("`{}` needs a value", o.op)));
        match o.op.as_str() {
            "add" => add(&mut work, &o.path, value()?).map_err(at)?,
            "remove" => {
                remove(&mut work, &o.path).map_err(at)?;
            }
            "replace" => {
                remove(&mut work, &o.path).map_err(at)?;
                add(&mut work, &o.path, value()?).map_err(at)?;
            }
            "move" | "copy" => {
                let from = o.from.as_deref().ok_or_else(|| at(format!("`{}` needs from", o.op)))?;
                let v = if o.op == "move" {
                    remove(&mut work, from).map_err(at)?
                } else {
                    work.pointer(from).cloned().ok_or_else(|| at(format!("`{from}` does not exist")))?
                };
                add(&mut work, &o.path, v).map_err(at)?;
            }
            "test" => {
                if work.pointer(&o.path) != Some(&value()?) {
                    return Err(at(format!("test failed: `{}` is not the given value", o.path)));
                }
            }
            other => return Err(at(format!("`{other}` is not a patch op; use add, remove, replace, move, copy or test"))),
        }
    }
    *doc = work;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn merge_patch_follows_rfc_7386() {
        let mut doc = json!({ "a": "b", "c": { "d": "e", "f": "g" } });
        merge(&mut doc, &json!({ "a": "z", "c": { "f": null } }));
        assert_eq!(doc, json!({ "a": "z", "c": { "d": "e" } }));
        merge(&mut doc, &json!({ "list": [1, 2] }));
        merge(&mut doc, &json!({ "list": [3] }));
        assert_eq!(doc["list"], json!([3]), "arrays replace");
    }

    #[test]
    fn json_patch_edits_and_stays_atomic() {
        let mut doc = json!({ "parts": { "a": { "sdf": [1, 2] } } });
        let ops: Vec<PatchOp> = serde_json::from_value(json!([
            { "op": "replace", "path": "/parts/a/sdf/1", "value": 5 },
            { "op": "add", "path": "/parts/a/sdf/-", "value": 6 },
            { "op": "copy", "from": "/parts/a", "path": "/parts/b" },
            { "op": "test", "path": "/parts/b/sdf/2", "value": 6 }
        ]))
        .unwrap();
        apply(&mut doc, &ops).unwrap();
        assert_eq!(doc, json!({ "parts": { "a": { "sdf": [1, 5, 6] }, "b": { "sdf": [1, 5, 6] } } }));
        let bad: Vec<PatchOp> = serde_json::from_value(json!([
            { "op": "remove", "path": "/parts/a" },
            { "op": "remove", "path": "/parts/zzz" }
        ]))
        .unwrap();
        let e = apply(&mut doc, &bad).unwrap_err();
        assert_eq!(e.path, "/ops/1");
        assert!(doc["parts"]["a"].is_object(), "a failed patch changes nothing");
    }
}
