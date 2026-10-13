//! The shape list of an SDF object as the op takes it: JSON shapes, each
//! one primitive key (`sphere`, `ellipsoid`, ..., or a `group` of shapes)
//! plus its join, modifiers, name and visibility. Shapes are addressed by
//! paths of indices (`[2, 0]` is the first member of the third shape's
//! group). Everything here edits that JSON and nothing else, so what the
//! editor commits is exactly what an agent would write.

use serde_json::{Map, Value, json};

/// The primitive keys, in the order the Add menu lists them.
pub const PRIMS: [&str; 8] = ["sphere", "ellipsoid", "box", "capsule", "cone", "torus", "cylinder", "group"];

/// How a shape joins the shapes before it: the parameter key (empty for a
/// plain union) and whether it takes a blend size.
pub const JOINS: [(&str, bool); 6] =
    [("union", false), ("smooth_union", true), ("subtract", false), ("smooth_subtract", true), ("intersect", false), ("smooth_intersect", true)];

/// Mirror choices offered in a row: none, or one axis.
pub const MIRRORS: [&str; 4] = ["", "x", "y", "z"];

/// A path to a shape: indices through groups.
pub type Path = Vec<usize>;

/// The primitive key of a shape.
pub fn prim_key(shape: &Value) -> Option<&'static str> {
    PRIMS.iter().copied().find(|k| shape.get(*k).is_some())
}

/// The join of a shape and its blend size (meters) for smooth joins.
pub fn join_of(shape: &Value) -> (&'static str, Option<f64>) {
    for (key, smooth) in JOINS.iter().skip(1) {
        match shape.get(*key) {
            Some(Value::Bool(true)) if !smooth => return (key, None),
            Some(v) if *smooth && v.is_number() => return (key, v.as_f64()),
            _ => {}
        }
    }
    ("union", None)
}

/// Sets the join of a shape; smooth joins take `blend` meters.
pub fn set_join(shape: &mut Value, join: &str, blend: f64) {
    let Some(m) = shape.as_object_mut() else { return };
    for (key, _) in JOINS.iter().skip(1) {
        m.remove(*key);
    }
    match JOINS.iter().find(|(k, _)| *k == join) {
        Some((k, true)) => {
            m.insert((*k).to_owned(), json!(blend.max(1e-4)));
        }
        Some((k, false)) if *k != "union" => {
            m.insert((*k).to_owned(), json!(true));
        }
        _ => {}
    }
}

/// The mirrored axes of a shape, as written (`""`, `"x"`, `"x, z"`).
pub fn mirror_of(shape: &Value) -> String {
    match shape.get("mirror") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

/// Mirrors a shape across one axis (`"x"`), or not (`""`).
pub fn set_mirror(shape: &mut Value, axis: &str) {
    let Some(m) = shape.as_object_mut() else { return };
    if axis.is_empty() {
        m.remove("mirror");
    } else {
        m.insert("mirror".into(), json!(axis));
    }
}

pub fn is_hidden(shape: &Value) -> bool {
    shape.get("hidden").and_then(Value::as_bool).unwrap_or(false)
}

pub fn set_hidden(shape: &mut Value, hidden: bool) {
    let Some(m) = shape.as_object_mut() else { return };
    if hidden {
        m.insert("hidden".into(), json!(true));
    } else {
        m.remove("hidden");
    }
}

pub fn name_of(shape: &Value) -> Option<&str> {
    shape.get("name").and_then(Value::as_str).filter(|n| !n.is_empty())
}

/// The members of a group shape.
pub fn members(shape: &Value) -> Option<&Vec<Value>> {
    shape.get("group").and_then(Value::as_array)
}

fn members_mut(shape: &mut Value) -> Option<&mut Vec<Value>> {
    shape.get_mut("group").and_then(Value::as_array_mut)
}

/// The shape at `path`.
///
/// Complexity: O(depth).
pub fn get<'a>(shapes: &'a [Value], path: &[usize]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    let s = shapes.get(*first)?;
    if rest.is_empty() { Some(s) } else { get(members(s)?, rest) }
}

/// The shape at `path`, to change.
///
/// Complexity: O(depth).
pub fn get_mut<'a>(shapes: &'a mut [Value], path: &[usize]) -> Option<&'a mut Value> {
    let (first, rest) = path.split_first()?;
    let s = shapes.get_mut(*first)?;
    if rest.is_empty() { Some(s) } else { get_mut(members_mut(s)?, rest) }
}

/// The list that holds the shape at `path` (the top list or a group's).
fn list_mut<'a>(shapes: &'a mut Vec<Value>, path: &[usize]) -> Option<&'a mut Vec<Value>> {
    match path.split_last() {
        Some((_, [])) => Some(shapes),
        Some((_, parent)) => members_mut(get_mut(shapes, parent)?),
        None => None,
    }
}

/// Every shape with its path, depth first, groups before their members.
///
/// Complexity: O(shapes).
pub fn flatten(shapes: &[Value]) -> Vec<(Path, &Value)> {
    fn walk<'a>(list: &'a [Value], prefix: &mut Path, out: &mut Vec<(Path, &'a Value)>) {
        for (i, s) in list.iter().enumerate() {
            prefix.push(i);
            out.push((prefix.clone(), s));
            if let Some(m) = members(s) {
                walk(m, prefix, out);
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(shapes, &mut Vec::new(), &mut out);
    out
}

/// The face group of every primitive: its name, or `shape_<n>` by its
/// place among all primitives (hidden ones counted), as the op names them.
///
/// Complexity: O(shapes).
pub fn leaf_groups(shapes: &[Value]) -> Vec<(Path, String)> {
    let mut n = 0;
    flatten(shapes)
        .into_iter()
        .filter(|(_, s)| prim_key(s) != Some("group"))
        .map(|(p, s)| {
            let g = name_of(s).map_or_else(|| format!("shape_{n}"), str::to_owned);
            n += 1;
            (p, g)
        })
        .collect()
}

/// The face groups a row stands for: its own, or its members' for a group.
///
/// Complexity: O(shapes).
pub fn groups_under(shapes: &[Value], path: &[usize]) -> Vec<String> {
    leaf_groups(shapes).into_iter().filter(|(p, _)| p.starts_with(path)).map(|(_, g)| g).collect()
}

/// The label of a row: the name, else the primitive and its group name.
pub fn label(shapes: &[Value], path: &[usize]) -> String {
    let Some(s) = get(shapes, path) else { return String::new() };
    if let Some(n) = name_of(s) {
        return n.to_owned();
    }
    match prim_key(s) {
        Some("group") => "group".to_owned(),
        _ => leaf_groups(shapes).into_iter().find(|(p, _)| p == path).map(|(_, g)| g).unwrap_or_default(),
    }
}

/// A name not used by any shape: `base`, else `base_2`, `base_3`, ...
///
/// Complexity: O(shapes x tries).
pub fn unique_name(shapes: &[Value], base: &str) -> String {
    let used: std::collections::BTreeSet<&str> = flatten(shapes).iter().filter_map(|(_, s)| name_of(s)).collect();
    if !used.contains(base) {
        return base.to_owned();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| !used.contains(n.as_str())).unwrap_or_else(|| base.to_owned())
}

/// Inserts `shape` right after `path` (in the same list), or at the end
/// of the top list without a path. Returns its path.
///
/// Complexity: O(shapes in that list).
pub fn insert_after(shapes: &mut Vec<Value>, path: Option<&[usize]>, shape: Value) -> Path {
    match path.filter(|p| get(shapes, p).is_some()) {
        Some(p) => {
            let mut at = p.to_vec();
            if let (Some(list), Some(last)) = (list_mut(shapes, p), at.last_mut()) {
                *last += 1;
                list.insert(*last, shape);
                return at;
            }
            at
        }
        None => {
            shapes.push(shape);
            vec![shapes.len() - 1]
        }
    }
}

/// Removes the shape at `path`. Refuses to leave the list or a group empty.
///
/// Complexity: O(shapes in that list).
pub fn remove(shapes: &mut Vec<Value>, path: &[usize]) -> Result<Value, &'static str> {
    let i = *path.last().ok_or("no shape")?;
    let list = list_mut(shapes, path).ok_or("no shape")?;
    if list.len() <= 1 {
        return Err(if path.len() == 1 { "an SDF object needs at least one shape" } else { "a group needs at least one shape" });
    }
    if i >= list.len() {
        return Err("no shape");
    }
    Ok(list.remove(i))
}

/// Copies the shape at `path` right after it, under a new name if it has
/// one. Returns the copy's path.
///
/// Complexity: O(shapes).
pub fn duplicate(shapes: &mut Vec<Value>, path: &[usize]) -> Option<Path> {
    let mut copy = get(shapes, path)?.clone();
    if let Some(n) = name_of(&copy).map(str::to_owned) {
        let fresh = unique_name(shapes, &n);
        copy["name"] = json!(fresh);
    }
    Some(insert_after(shapes, Some(path), copy))
}

/// Wraps the shape at `path` into a new group in its place: the group takes
/// over the shape's join, the shape joins plainly inside. Returns the
/// group's path (the shape is its first member).
///
/// Complexity: O(shapes).
pub fn wrap_in_group(shapes: &mut [Value], path: &[usize]) -> Option<Path> {
    let name = unique_name(shapes, "group");
    let s = get_mut(shapes, path)?;
    let mut inner = std::mem::take(s);
    let (join, blend) = join_of(&inner);
    set_join(&mut inner, "union", 0.0);
    let mut group = Value::Object(Map::new());
    group["name"] = json!(name);
    group["group"] = json!([inner]);
    set_join(&mut group, join, blend.unwrap_or(0.0));
    *s = group;
    Some(path.to_vec())
}

/// Moves the shape at `from` to just before `before` (a path in any list;
/// one past the last index puts it at the end). Returns its new path, or
/// `None` when the move is impossible (into itself, or a list left empty).
///
/// Complexity: O(shapes).
pub fn move_to(shapes: &mut Vec<Value>, from: &[usize], before: &[usize]) -> Option<Path> {
    if from.is_empty() || before.is_empty() || before.starts_with(from) {
        return None;
    }
    let (fi, fparent) = from.split_last()?;
    let (bi, bparent) = before.split_last()?;
    let mut target_parent = bparent.to_vec();
    let mut target_index = *bi;
    if fparent == bparent && fi < bi {
        target_index -= 1;
    }
    if fparent == bparent && *fi == target_index {
        return None;
    }
    // Removing `from` shifts later siblings on the target's path.
    if target_parent.len() > fparent.len() && target_parent.starts_with(fparent) && target_parent[fparent.len()] > *fi {
        target_parent[fparent.len()] -= 1;
    }
    let shape = remove(shapes, from).ok()?;
    if !target_parent.is_empty() && get(shapes, &target_parent).and_then(members).is_none() {
        // No such group: put the shape back where it was.
        if let Some(list) = list_mut(shapes, from) {
            list.insert((*fi).min(list.len()), shape);
        }
        return None;
    }
    let list = if target_parent.is_empty() { shapes } else { get_mut(shapes, &target_parent).and_then(members_mut)? };
    let at = target_index.min(list.len());
    list.insert(at, shape);
    let mut path = target_parent;
    path.push(at);
    Some(path)
}

/// A new shape of primitive `prim` around `at` (object space), about
/// `size` meters across, blended in with a fillet.
pub fn new_shape(prim: &str, at: [f64; 3], size: f64, name: &str) -> Value {
    let s = size.max(1e-3) * 0.5;
    let r = |x: f64| (x * 1e4).round() / 1e4;
    let p = |dy: f64| json!([r(at[0]), r(at[1] + dy), r(at[2])]);
    let mut v = match prim {
        "ellipsoid" => json!({ "ellipsoid": { "at": p(0.0), "radii": [r(s), r(s * 0.7), r(s)] } }),
        "box" => json!({ "box": { "at": p(0.0), "size": [r(s * 2.0), r(s * 2.0), r(s * 2.0)], "corner": r(s * 0.15) } }),
        "capsule" => json!({ "capsule": { "a": p(-s), "b": p(s), "radius": r(s * 0.4) } }),
        "cone" => json!({ "cone": { "at": p(-s), "height": r(s * 2.0), "radius_bottom": r(s), "radius_top": 0.0 } }),
        "torus" => json!({ "torus": { "at": p(0.0), "major": r(s), "minor": r(s * 0.3) } }),
        "cylinder" => json!({ "cylinder": { "at": p(0.0), "height": r(s * 2.0), "radius": r(s * 0.6) } }),
        _ => json!({ "sphere": { "at": p(0.0), "radius": r(s) } }),
    };
    v["name"] = json!(name);
    set_join(&mut v, "smooth_union", r(s * 0.3));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Value> {
        vec![
            json!({ "name": "body", "sphere": { "at": [0, 0, 0], "radius": 0.1 } }),
            json!({ "sphere": { "at": [0, 0.1, 0], "radius": 0.05 }, "smooth_union": 0.02 }),
            json!({ "name": "legs", "group": [
                { "capsule": { "a": [0.05, 0, 0], "b": [0.05, -0.1, 0], "radius": 0.02 } },
                { "name": "foot", "sphere": { "at": [0.05, -0.1, 0.02], "radius": 0.02 }, "hidden": true }
            ] }),
        ]
    }

    #[test]
    fn joins_mirrors_and_visibility_round_trip() {
        let mut s = json!({ "sphere": { "at": [0, 0, 0], "radius": 0.1 } });
        assert_eq!(join_of(&s), ("union", None));
        set_join(&mut s, "smooth_subtract", 0.01);
        assert_eq!(join_of(&s), ("smooth_subtract", Some(0.01)));
        set_join(&mut s, "intersect", 0.0);
        assert_eq!((join_of(&s), s.get("smooth_subtract")), (("intersect", None), None));
        set_mirror(&mut s, "x");
        assert_eq!(mirror_of(&s), "x");
        set_mirror(&mut s, "");
        assert!(s.get("mirror").is_none());
        set_hidden(&mut s, true);
        assert!(is_hidden(&s));
        set_hidden(&mut s, false);
        assert!(s.get("hidden").is_none());
    }

    #[test]
    fn group_names_follow_the_op() {
        let s = sample();
        let groups: Vec<String> = leaf_groups(&s).into_iter().map(|(_, g)| g).collect();
        assert_eq!(groups, ["body", "shape_1", "shape_2", "foot"]);
        assert_eq!(groups_under(&s, &[2]), ["shape_2", "foot"]);
        assert_eq!(label(&s, &[1]), "shape_1");
        assert_eq!(flatten(&s).iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), [vec![0], vec![1], vec![2], vec![2, 0], vec![2, 1]]);
    }

    #[test]
    fn edits_keep_the_tree_valid() {
        let mut s = sample();
        let copy = duplicate(&mut s, &[0]).unwrap();
        assert_eq!((copy.clone(), name_of(&s[1])), (vec![1], Some("body_2")));
        assert_eq!(remove(&mut s, &copy).ok().and_then(|v| name_of(&v).map(str::to_owned)).as_deref(), Some("body_2"));
        let g = wrap_in_group(&mut s, &[1]).unwrap();
        assert_eq!(prim_key(&s[1]), Some("group"));
        assert_eq!(join_of(&s[1]), ("smooth_union", Some(0.02)), "the group takes the join");
        assert_eq!(join_of(get(&s, &[1, 0]).unwrap()), ("union", None));
        assert_eq!(g, vec![1]);
        // Move the foot to the top, before the body.
        let moved = move_to(&mut s, &[2, 1], &[0]).unwrap();
        assert_eq!((moved, name_of(&s[0])), (vec![0], Some("foot")));
        assert!(remove(&mut s, &[3, 0]).is_err(), "a group keeps one shape");
        // Into its own group: refused.
        assert!(move_to(&mut s, &[2], &[2, 0]).is_none());
        // To the end of the top list.
        let n = s.len();
        let end = move_to(&mut s, &[0], &[n]).unwrap();
        assert_eq!(end, vec![s.len() - 1]);
        let added = insert_after(&mut s, Some(&[0]), new_shape("torus", [0.0, 0.0, 0.0], 0.2, "ring"));
        assert_eq!((added.clone(), prim_key(get(&s, &added).unwrap())), (vec![1], Some("torus")));
        assert_eq!(unique_name(&s, "ring"), "ring_2");
    }
}
