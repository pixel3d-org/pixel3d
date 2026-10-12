//! Shared parameter types. Lengths are meters or unit strings, angles are
//! degrees or unit strings; both are stored in internal units (meters,
//! radians) and serialize back as plain numbers (meters, degrees).

use std::borrow::Cow;
use std::fmt;

use p3d_math::{parse_angle, parse_length};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A length in meters. Accepts `0.3`, `"30cm"`, `"12mm"`, `"1in"`, `"3ft"`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Length(pub f64);

impl Length {
    pub fn meters(self) -> f64 {
        self.0
    }
}

/// An angle stored in radians. Accepts degrees (`45`) or `"45deg"`, `"0.8rad"`.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Angle(pub f64);

impl Angle {
    pub fn from_degrees(d: f64) -> Self {
        Angle(d.to_radians())
    }

    pub fn radians(self) -> f64 {
        self.0
    }

    pub fn degrees(self) -> f64 {
        self.0.to_degrees()
    }
}

struct QuantityVisitor {
    what: &'static str,
    parse: fn(&str) -> Result<f64, p3d_math::UnitError>,
    /// Converts a bare number to internal units.
    scale: f64,
}

impl Visitor<'_> for QuantityVisitor {
    type Value = f64;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a {} as a number or a string with a unit", self.what)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<f64, E> {
        if v.is_finite() { Ok(v * self.scale) } else { Err(E::custom(format!("{} must be finite", self.what))) }
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<f64, E> {
        self.visit_f64(v as f64)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<f64, E> {
        self.visit_f64(v as f64)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<f64, E> {
        (self.parse)(v).map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(QuantityVisitor { what: "length", parse: parse_length, scale: 1.0 }).map(Length)
    }
}

impl Serialize for Length {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Angle {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(QuantityVisitor { what: "angle", parse: parse_angle, scale: std::f64::consts::PI / 180.0 }).map(Angle)
    }
}

impl Serialize for Angle {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Round-trip through degrees without accumulating noise.
        let d = self.0.to_degrees();
        let r = (d * 1e9).round() / 1e9;
        s.serialize_f64(r)
    }
}

const NUMBER: &str = r"[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?";

impl JsonSchema for Length {
    fn schema_name() -> Cow<'static, str> {
        "Length".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Length: meters as a number, or a string with a unit (mm, cm, dm, m, km, in, ft), e.g. \"30cm\".",
            "anyOf": [
                { "type": "number" },
                { "type": "string", "pattern": format!(r"^\s*{NUMBER}\s*(mm|cm|dm|m|km|in|ft)?\s*$") }
            ]
        })
    }
}

impl JsonSchema for Angle {
    fn schema_name() -> Cow<'static, str> {
        "Angle".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "Angle: degrees as a number, or a string with a unit (deg, rad), e.g. \"45deg\".",
            "anyOf": [
                { "type": "number" },
                { "type": "string", "pattern": format!(r"^\s*{NUMBER}\s*(deg|rad)?\s*$") }
            ]
        })
    }
}

/// Where a primitive's origin sits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OriginParam {
    /// Bounding box center.
    Center,
    /// Center of the bottom: the object stands on the ground plane.
    #[default]
    Bottom,
}

impl From<OriginParam> for p3d_geo::prim::Origin {
    fn from(o: OriginParam) -> Self {
        match o {
            OriginParam::Center => p3d_geo::prim::Origin::Center,
            OriginParam::Bottom => p3d_geo::prim::Origin::Bottom,
        }
    }
}

/// Converts a position parameter to a vector.
pub fn vec3(p: &[Length; 3]) -> glam::Vec3 {
    glam::Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lengths() {
        let l: Length = serde_json::from_value(json!("30cm")).unwrap();
        assert!((l.0 - 0.3).abs() < 1e-12);
        let l: Length = serde_json::from_value(json!(2)).unwrap();
        assert_eq!(l.0, 2.0);
        assert_eq!(serde_json::to_value(Length(0.25)).unwrap(), json!(0.25));
        let err = serde_json::from_value::<Length>(json!("5yd")).unwrap_err().to_string();
        assert!(err.contains("unknown unit `yd`"), "{err}");
        assert!(serde_json::from_value::<Length>(json!(true)).is_err());
    }

    #[test]
    fn angles() {
        let a: Angle = serde_json::from_value(json!(90)).unwrap();
        assert!((a.radians() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let a: Angle = serde_json::from_value(json!("0.5rad")).unwrap();
        assert!((a.radians() - 0.5).abs() < 1e-12);
        assert_eq!(serde_json::to_value(Angle::from_degrees(30.0)).unwrap(), json!(30.0));
    }

    #[test]
    fn schemas_accept_numbers_and_unit_strings() {
        let s = serde_json::to_value(schemars::schema_for!(Length)).unwrap();
        let text = s.to_string();
        assert!(text.contains("anyOf") && text.contains("mm|cm"), "{text}");
    }
}
