//! Expectations: tests for models, stored in the document (project spec
//! 17.11). Values here are canonical (meters); the ops that add them accept
//! unit strings and resolve object references.

use serde::{Deserialize, Serialize};

/// A reference to an object: the name the author wrote and the id it
/// resolved to, so renaming the object does not break the expectation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub name: String,
    pub id: u32,
}

/// A closed range; a missing end is open.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Range {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Range {
    pub fn contains(&self, x: f64) -> bool {
        self.min.is_none_or(|m| x >= m) && self.max.is_none_or(|m| x <= m)
    }
}

/// What an expectation checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// World size of the target (with its children) within ranges, meters.
    Dims { target: ObjectRef, width: Option<Range>, height: Option<Range>, depth: Option<Range> },
    /// `a` mirrors `b` across the plane `axis = at` (world), or a single
    /// target mirrors itself across its own center plane when `b` is none.
    Symmetric { a: ObjectRef, b: Option<ObjectRef>, axis: char, at: Option<f64>, tolerance: f64 },
    /// `part` touches `to`: gap at most `max_gap`, sinking in at most
    /// `max_penetration`, meters.
    Attached { part: ObjectRef, to: ObjectRef, max_gap: f64, max_penetration: f64 },
    /// No two of the parts pass through each other or sit inside each other.
    NotIntersecting { parts: Vec<ObjectRef> },
    /// Triangles of the target (or the whole scene) within a range.
    Budget { target: Option<ObjectRef>, min_tris: Option<u64>, max_tris: u64 },
    /// The target's meshes are closed and face outward.
    Watertight { target: ObjectRef },
}

impl Check {
    /// Short name used in test lines: `attached(lid, crate)`.
    pub fn label(&self) -> String {
        match self {
            Check::Dims { target, .. } => format!("dims({})", target.name),
            Check::Symmetric { a, b: Some(b), .. } => format!("symmetric({}, {})", a.name, b.name),
            Check::Symmetric { a, b: None, axis, .. } => format!("symmetric({}, {axis})", a.name),
            Check::Attached { part, to, .. } => format!("attached({}, {})", part.name, to.name),
            Check::NotIntersecting { parts } => {
                format!("not_intersecting({})", parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
            }
            Check::Budget { target: Some(t), .. } => format!("budget({})", t.name),
            Check::Budget { target: None, .. } => "budget(scene)".into(),
            Check::Watertight { target } => format!("watertight({})", target.name),
        }
    }

    /// Ids of the objects the check refers to.
    pub fn objects(&self) -> Vec<u32> {
        match self {
            Check::Dims { target, .. } | Check::Watertight { target } => vec![target.id],
            Check::Symmetric { a, b, .. } => std::iter::once(a.id).chain(b.as_ref().map(|b| b.id)).collect(),
            Check::Attached { part, to, .. } => vec![part.id, to.id],
            Check::NotIntersecting { parts } => parts.iter().map(|p| p.id).collect(),
            Check::Budget { target, .. } => target.iter().map(|t| t.id).collect(),
        }
    }
}

/// A named expectation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    /// Unique within the document: `E1`, `E2`, ... or the author's name.
    pub name: String,
    pub check: Check,
}
