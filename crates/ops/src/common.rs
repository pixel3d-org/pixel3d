//! Helpers shared by op implementations.

use p3d_math::format_length;
use p3d_mesh::Mesh;
use p3d_scene::name::check_name;
use p3d_scene::{Document, ObjectKey};

use crate::error::OpError;
use crate::op::GroupInfo;
use crate::params::{Length, vec3};
use crate::tx::Tx;

/// `1234567` as `1,234,567`.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1 x 0.6 x 0.8 m` from a mesh's bounds.
pub fn dims(mesh: &Mesh) -> String {
    match mesh.bounds() {
        None => "empty".into(),
        Some((lo, hi)) => {
            let d = hi - lo;
            let parts: Vec<String> = [d.x, d.y, d.z].iter().map(|&x| format_length(f64::from(x))).collect();
            // "1 m x 0.6 m x 0.8 m" reads worse than "1 x 0.6 x 0.8 m" when units agree.
            let units: Vec<&str> = parts.iter().map(|p| p.rsplit(' ').next().unwrap_or("")).collect();
            if units.iter().all(|u| *u == units[0]) {
                let nums: Vec<&str> = parts.iter().map(|p| p.split(' ').next().unwrap_or("")).collect();
                format!("{} {}", nums.join(" x "), units[0])
            } else {
                parts.join(" x ")
            }
        }
    }
}

/// Checks an optional parent target and an optional object name before
/// anything is created.
pub fn check_new_object(doc: &Document, name: Option<&str>, parent: Option<&str>) -> Result<Option<ObjectKey>, OpError> {
    let parent_key = match parent {
        Some(p) => Some(doc.resolve(p).map_err(|e| OpError::from_scene(e, "/parent"))?),
        None => None,
    };
    if let Some(n) = name {
        check_name(n).map_err(|e| OpError::invalid("/name", e.to_string()))?;
    }
    Ok(parent_key)
}

/// Creates a mesh object: the given name (must be free) or a free variant of
/// `default_name`, under `parent`, at `position`.
pub fn create_mesh_object(
    tx: &mut Tx,
    name: Option<&str>,
    default_name: &str,
    parent: Option<&str>,
    position: &[Length; 3],
    mesh: Mesh,
) -> Result<ObjectKey, OpError> {
    let parent_key = check_new_object(tx.doc(), name, parent)?;
    let name = match name {
        Some(n) => n.to_owned(),
        None => tx.doc().unique_name(default_name, parent_key),
    };
    let key = tx.doc_mut().create_mesh_object(&name, parent_key, mesh).map_err(|e| OpError::from_scene(e, "/name"))?;
    if let Some(o) = tx.doc_mut().object_mut(key) {
        o.transform.translation = vec3(position);
    }
    Ok(key)
}

/// Groups of an object's mesh, in creation order.
pub fn mesh_groups(doc: &Document, key: ObjectKey) -> Vec<GroupInfo> {
    doc.object_mesh(key)
        .map(|m| m.groups().map(|(name, g)| GroupInfo { object: key, name: name.to_owned(), kind: g.kind(), count: g.count() as u32 }).collect())
        .unwrap_or_default()
}

/// `crate (o:3)`.
pub fn label(doc: &Document, key: ObjectKey) -> String {
    match (doc.path(key), doc.object(key)) {
        (Some(p), Some(o)) => format!("{p} ({})", o.public_id()),
        _ => "?".into(),
    }
}

/// Checks a length parameter against a range, as `OUT_OF_RANGE` at `path`.
pub fn check_length(path: &str, l: Length, min: f64, max: f64) -> Result<(), OpError> {
    if l.0.is_finite() && (min..=max).contains(&l.0) {
        Ok(())
    } else {
        Err(OpError::out_of_range(path, format!("{} is outside {} to {}", format_length(l.0), format_length(min), format_length(max))))
    }
}

/// Checks a count parameter against a range, as `OUT_OF_RANGE` at `path`.
pub fn check_count(path: &str, n: u32, min: u32, max: u32) -> Result<(), OpError> {
    if (min..=max).contains(&n) { Ok(()) } else { Err(OpError::out_of_range(path, format!("{n} is outside {min} to {max}"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1188), "1,188");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
