//! Transform settings, as Blender's viewport header has them: the
//! orientation of the gizmo axes, the pivot point, snapping (what snaps,
//! to what, with which elements, affecting which transforms), proportional
//! editing, and the 3D cursor. They are UI state: the GUI computes final
//! values with them and then calls ops (`scene.set_transform`,
//! `edit.transform`), so agents never need them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::t;

macro_rules! choice {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $label:literal),* $(,)? } default $default:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            #[default]
            $default,
            $($variant,)*
        }

        impl $name {
            pub const ALL: &[$name] = &[$name::$default, $($name::$variant,)*];

            pub fn label(self) -> &'static str {
                #[allow(unreachable_patterns)]
                match self {
                    $($name::$variant => t($label),)*
                    _ => t(stringify!($default)),
                }
            }
        }
    };
}

choice! {
    /// The axes the gizmo and typed values use.
    Orientation { Local => "Local", Normal => "Normal", Gimbal => "Gimbal", View => "View", Cursor => "Cursor", Parent => "Parent" } default Global
}

choice! {
    /// The point rotation and scaling happen around.
    Pivot { BoundsCenter => "Bounding Box Center", Cursor => "3D Cursor", Individual => "Individual Origins", Active => "Active Element" } default Median
}

choice! {
    /// The part of the selection that snaps.
    SnapBase { Center => "Center", Median => "Median", Active => "Active" } default Closest
}

choice! {
    /// What the selection snaps to.
    SnapTarget {
        Grid => "Grid",
        Vertex => "Vertex",
        Edge => "Edge",
        Face => "Face",
        Volume => "Volume",
        EdgeCenter => "Edge Center",
        EdgePerpendicular => "Edge Perpendicular",
    } default Increment
}

choice! {
    /// How proportional editing fades with distance.
    Falloff {
        Sphere => "Sphere",
        Root => "Root",
        InverseSquare => "Inverse Square",
        Sharp => "Sharp",
        Linear => "Linear",
        Constant => "Constant",
        Random => "Random",
    } default Smooth
}

impl Pivot {
    /// Blender's name for the default.
    pub fn name(self) -> &'static str {
        match self {
            Pivot::Median => t("Median Point"),
            other => other.label(),
        }
    }
}

/// The settings of the transform header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TransformSettings {
    pub orientation: Orientation,
    pub pivot: Pivot,
    pub snap_base: SnapBase,
    /// Sorted, without repeats.
    pub snap_targets: Vec<SnapTarget>,
    /// Snap individual elements: project onto faces, or to the nearest face.
    pub face_project: bool,
    pub face_nearest: bool,
    /// Which geometry snapping considers.
    pub include_active: bool,
    pub include_edited: bool,
    pub include_nonedited: bool,
    pub exclude_nonselectable: bool,
    /// Which transforms snap.
    pub affect_move: bool,
    pub affect_rotate: bool,
    pub affect_scale: bool,
    pub proportional: bool,
    pub falloff: Falloff,
    pub connected_only: bool,
    pub projected: bool,
    /// Proportional editing radius (meters).
    pub radius: f32,
    /// The 3D cursor (world, meters).
    pub cursor: [f32; 3],
}

impl Default for TransformSettings {
    fn default() -> Self {
        TransformSettings {
            orientation: Orientation::Global,
            pivot: Pivot::Median,
            snap_base: SnapBase::Closest,
            snap_targets: vec![SnapTarget::Increment],
            face_project: false,
            face_nearest: false,
            include_active: true,
            include_edited: true,
            include_nonedited: false,
            exclude_nonselectable: true,
            affect_move: true,
            affect_rotate: false,
            affect_scale: false,
            proportional: false,
            falloff: Falloff::Smooth,
            connected_only: false,
            projected: false,
            radius: 1.0,
            cursor: [0.0; 3],
        }
    }
}

/// `true`, `false` or `"toggle"` applied to `flag`.
fn set_flag(flag: &mut bool, v: &Value) -> Result<(), String> {
    match v {
        Value::Bool(b) => *flag = *b,
        Value::String(s) if s == "toggle" => *flag = !*flag,
        _ => return Err(format!("{}: {v}", t("expected true, false or \"toggle\""))),
    }
    Ok(())
}

fn parse<T: serde::de::DeserializeOwned>(key: &str, v: &Value) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("{key}: {e}"))
}

impl TransformSettings {
    /// Applies the fields of a `ui.transform` call: choices by name
    /// (`orientation`, `pivot`, `snapBase`, `falloff`), flags as `true`,
    /// `false` or `"toggle"`, `snapTarget` and `affect` toggling one entry,
    /// `radius` in meters and `cursor` as `[x, y, z]`. Nothing changes when
    /// a field is refused.
    ///
    /// Complexity: O(fields + snap targets).
    pub fn apply(&mut self, params: &Value) -> Result<Value, String> {
        let Some(fields) = params.as_object() else {
            return Ok(json!(self));
        };
        let mut next = self.clone();
        for (k, v) in fields {
            match k.as_str() {
                "orientation" => next.orientation = parse(k, v)?,
                "pivot" => next.pivot = parse(k, v)?,
                "snapBase" => next.snap_base = parse(k, v)?,
                "falloff" => next.falloff = parse(k, v)?,
                "snapTarget" => {
                    let target: SnapTarget = parse(k, v)?;
                    match next.snap_targets.binary_search(&target) {
                        Ok(i) => {
                            next.snap_targets.remove(i);
                        }
                        Err(i) => next.snap_targets.insert(i, target),
                    }
                }
                "affect" => match v.as_str() {
                    Some("move") => next.affect_move = !next.affect_move,
                    Some("rotate") => next.affect_rotate = !next.affect_rotate,
                    Some("scale") => next.affect_scale = !next.affect_scale,
                    _ => return Err(t("affect is move, rotate or scale").to_owned()),
                },
                "faceProject" => set_flag(&mut next.face_project, v)?,
                "faceNearest" => set_flag(&mut next.face_nearest, v)?,
                "includeActive" => set_flag(&mut next.include_active, v)?,
                "includeEdited" => set_flag(&mut next.include_edited, v)?,
                "includeNonedited" => set_flag(&mut next.include_nonedited, v)?,
                "excludeNonselectable" => set_flag(&mut next.exclude_nonselectable, v)?,
                "proportional" => set_flag(&mut next.proportional, v)?,
                "connectedOnly" => set_flag(&mut next.connected_only, v)?,
                "projected" => set_flag(&mut next.projected, v)?,
                "radius" => {
                    let r = v.as_f64().filter(|r| *r > 0.0).ok_or(t("radius must be a positive length in meters"))?;
                    next.radius = r as f32;
                }
                "cursor" => next.cursor = parse(k, v)?,
                other => return Err(format!("{} `{other}`", t("unknown transform setting"))),
            }
        }
        *self = next;
        Ok(json!(self))
    }

    pub fn snaps_to(&self, target: SnapTarget) -> bool {
        self.snap_targets.binary_search(&target).is_ok()
    }

    pub fn cursor(&self) -> glam::Vec3 {
        glam::Vec3::from_array(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_apply_by_name_and_toggle() {
        let mut s = TransformSettings::default();
        s.apply(&json!({ "orientation": "local", "pivot": "cursor", "falloff": "inverse_square" })).unwrap();
        assert_eq!((s.orientation, s.pivot, s.falloff), (Orientation::Local, Pivot::Cursor, Falloff::InverseSquare));
        s.apply(&json!({ "snapTarget": "vertex" })).unwrap();
        assert_eq!(s.snap_targets, vec![SnapTarget::Increment, SnapTarget::Vertex]);
        s.apply(&json!({ "snapTarget": "increment", "affect": "rotate", "proportional": "toggle" })).unwrap();
        assert_eq!(s.snap_targets, vec![SnapTarget::Vertex]);
        assert!(s.affect_rotate && s.proportional);
        // A bad field changes nothing.
        let before = s.clone();
        assert!(s.apply(&json!({ "pivot": "median", "radius": -1 })).is_err());
        assert!(s.apply(&json!({ "colour": "red" })).is_err());
        assert_eq!(s, before);
        s.apply(&json!({ "cursor": [1, 2, 3] })).unwrap();
        assert_eq!(s.cursor(), glam::Vec3::new(1.0, 2.0, 3.0));
        // Labels for every choice.
        assert_eq!(Pivot::Median.name(), "Median Point");
        assert!(Orientation::ALL.iter().all(|o| !o.label().is_empty()));
        assert_eq!(Orientation::ALL.len(), 7);
        assert_eq!(SnapTarget::ALL.len(), 8);
        assert_eq!(Falloff::ALL.len(), 8);
    }
}
