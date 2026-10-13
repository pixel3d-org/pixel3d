//! Spec constraints (project spec 17.3): relations between parts that the
//! compiler satisfies by moving (or, for `proportion`, scaling) parts after
//! their anchors resolve. Variables are part placements only, never
//! vertices; the solver runs Gauss-Seidel rounds in the written order.

use p3d_ops::params::Length;
use serde::{Deserialize, Serialize};

use crate::model::Axis;

/// A point: root coordinates or `part.landmark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PointRef {
    At([Length; 3]),
    Landmark(String),
}

/// A coordinate: a number (meters) or `part.landmark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CoordRef {
    Value(Length),
    Landmark(String),
}

/// Even spacing, or varied (a deterministic 15% rhythm, ends fixed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spacing {
    #[default]
    Even,
    Varied,
}

fn ground() -> String {
    "ground".into()
}

/// One constraint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Constraint {
    /// Moves `a`'s part so the landmark's coordinate along `axis` equals
    /// `b` (a landmark or a number) plus `offset`.
    Align {
        a: String,
        b: CoordRef,
        axis: Axis,
        #[serde(default)]
        offset: Length,
    },
    /// Drops `part` straight down (or lifts it) until its lowest point sits
    /// on `on`: `ground` (y = 0, the default) or another part's top surface.
    RestOn {
        part: String,
        #[serde(default = "ground")]
        on: String,
    },
    /// Moves `b` to the mirror position of `a` across the root plane normal
    /// to `axis` (x by default).
    Symmetric {
        a: String,
        b: String,
        #[serde(default)]
        axis: Axis,
    },
    /// Places the parts' origins along a line from `from` to `to`.
    Distribute {
        parts: Vec<String>,
        from: PointRef,
        to: PointRef,
        #[serde(default)]
        spacing: Spacing,
    },
    /// Pushes `a` away from `b` until they are at least `min` apart.
    Clearance { a: String, b: String, min: Length },
    /// Moves `part` toward (or away from) `to` until the gap is `gap`
    /// (0: touching).
    Attach {
        part: String,
        to: String,
        #[serde(default)]
        gap: Length,
    },
    /// Shifts `part` so its bounds lie inside `in`'s bounds.
    Inside {
        part: String,
        #[serde(rename = "in")]
        container: String,
    },
    /// Puts `part`'s origin at the landmark `from` plus `by`.
    Offset { part: String, from: String, by: [Length; 3] },
    /// Scales `a`'s part uniformly so `a` (`part.height`, `.width`, `.depth`
    /// or `.length`) is `ratio` times `b`.
    Proportion { a: String, b: String, ratio: f64 },
}

impl Constraint {
    /// Short label for reports: `align(ear_l.top, head.brow_l)`.
    pub fn label(&self) -> String {
        let c = |r: &CoordRef| match r {
            CoordRef::Value(v) => format!("{}", v.0),
            CoordRef::Landmark(l) => l.clone(),
        };
        match self {
            Constraint::Align { a, b, .. } => format!("align({a}, {})", c(b)),
            Constraint::RestOn { part, on } => format!("rest_on({part}, {on})"),
            Constraint::Symmetric { a, b, .. } => format!("symmetric({a}, {b})"),
            Constraint::Distribute { parts, .. } => format!("distribute({} parts)", parts.len()),
            Constraint::Clearance { a, b, .. } => format!("clearance({a}, {b})"),
            Constraint::Attach { part, to, .. } => format!("attach({part}, {to})"),
            Constraint::Inside { part, container } => format!("inside({part}, {container})"),
            Constraint::Offset { part, from, .. } => format!("offset({part}, {from})"),
            Constraint::Proportion { a, b, .. } => format!("proportion({a}, {b})"),
        }
    }
}

/// `part.landmark` split at the first dot.
pub fn split_landmark(s: &str) -> Option<(&str, &str)> {
    s.split_once('.')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn constraints_parse_from_their_short_forms() {
        let c: Vec<Constraint> = serde_json::from_value(json!([
            { "align": { "a": "ear_l.top", "b": "head.brow_l", "axis": "y", "offset": "1cm" } },
            { "rest_on": { "part": "chest" } },
            { "distribute": { "parts": ["r1", "r2", "r3"], "from": [0, 0, 0], "to": "rim.front", "spacing": "varied" } },
            { "inside": { "part": "gem", "in": "socket" } }
        ]))
        .unwrap();
        assert_eq!(c[0].label(), "align(ear_l.top, head.brow_l)");
        assert!(matches!(&c[1], Constraint::RestOn { on, .. } if on == "ground"));
        assert!(matches!(&c[2], Constraint::Distribute { to: PointRef::Landmark(l), spacing: Spacing::Varied, .. } if l == "rim.front"));
        assert!(serde_json::from_value::<Constraint>(json!({ "align": { "a": "x.top", "b": 0, "axis": "w" } })).is_err());
    }
}
