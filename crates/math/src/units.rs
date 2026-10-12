//! Lengths and angles at API boundaries.
//!
//! Internally pixel3d works in meters and radians. Every API accepts a length
//! as a number of meters or as a string with a unit (`"12mm"`, `"2.5 cm"`,
//! `"0.3m"`, `"1in"`, `"3ft"`), and an angle as a number of degrees or as a
//! string with a unit (`"45deg"`, `"0.5rad"`).

use std::fmt;

/// Length units accepted in strings, with their size in meters.
pub const LENGTH_UNITS: &[(&str, f64)] = &[("mm", 0.001), ("cm", 0.01), ("dm", 0.1), ("m", 1.0), ("km", 1000.0), ("in", 0.0254), ("ft", 0.3048)];

/// Angle units accepted in strings, with their size in radians.
pub const ANGLE_UNITS: &[(&str, f64)] = &[("deg", std::f64::consts::PI / 180.0), ("rad", 1.0)];

/// Why a quantity string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnitError {
    #[error("`{0}` is empty; expected a number with an optional unit such as \"12mm\"")]
    Empty(String),
    #[error("`{input}` has no valid number before the unit")]
    BadNumber { input: String },
    #[error("`{input}` uses the unknown unit `{unit}`; valid units: {valid}")]
    UnknownUnit { input: String, unit: String, valid: String },
    #[error("`{0}` is not a finite number")]
    NotFinite(String),
}

/// Splits `"2.5 cm"` into `(2.5, "cm")`. The unit is the trailing run of
/// ASCII letters; an exponent such as `1e-3` stays part of the number.
///
/// Complexity: O(n) in string length.
fn split_quantity(input: &str) -> Result<(f64, &str), UnitError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(UnitError::Empty(input.to_owned()));
    }
    let unit_start = s.char_indices().rev().take_while(|(_, c)| c.is_ascii_alphabetic()).last().map_or(s.len(), |(i, _)| i);
    let (number, unit) = s.split_at(unit_start);
    let value: f64 = number.trim_end().parse().map_err(|_| UnitError::BadNumber { input: input.to_owned() })?;
    if !value.is_finite() {
        return Err(UnitError::NotFinite(input.to_owned()));
    }
    Ok((value, unit))
}

fn convert(input: &str, units: &[(&str, f64)], default_scale: f64) -> Result<f64, UnitError> {
    let (value, unit) = split_quantity(input)?;
    if unit.is_empty() {
        return Ok(value * default_scale);
    }
    let lower = unit.to_ascii_lowercase();
    match units.iter().find(|(name, _)| *name == lower) {
        Some((_, scale)) => Ok(value * scale),
        None => Err(UnitError::UnknownUnit {
            input: input.to_owned(),
            unit: unit.to_owned(),
            valid: units.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
        }),
    }
}

/// Parses a length string to meters. A bare number is meters.
///
/// Complexity: O(n) in string length.
pub fn parse_length(input: &str) -> Result<f64, UnitError> {
    convert(input, LENGTH_UNITS, 1.0)
}

/// Parses an angle string to radians. A bare number is degrees.
///
/// Complexity: O(n) in string length.
pub fn parse_angle(input: &str) -> Result<f64, UnitError> {
    convert(input, ANGLE_UNITS, std::f64::consts::PI / 180.0)
}

/// Formats meters for people and agents: millimeters below 10 cm,
/// otherwise meters, trimmed to at most four significant decimals.
pub fn format_length(meters: f64) -> String {
    if meters != 0.0 && meters.abs() < 0.1 { format!("{} mm", trim(meters * 1000.0, 2)) } else { format!("{} m", trim(meters, 4)) }
}

/// Formats radians as degrees with at most two decimals.
pub fn format_angle(radians: f64) -> String {
    format!("{} deg", trim(radians.to_degrees(), 2))
}

fn trim(value: f64, decimals: usize) -> String {
    let s = format!("{value:.decimals$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { &s };
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

/// A length in meters, displayed with [`format_length`].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Meters(pub f64);

impl fmt::Display for Meters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_length(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn lengths_with_units() {
        for (s, m) in [
            ("12mm", 0.012),
            ("2.5cm", 0.025),
            ("2.5 cm", 0.025),
            ("0.3m", 0.3),
            ("1in", 0.0254),
            ("3ft", 0.9144),
            ("3FT", 0.9144),
            ("-4mm", -0.004),
            ("1e-3m", 0.001),
            ("1.5e2mm", 0.15),
            (" 7 ", 7.0),
            ("2", 2.0),
        ] {
            let got = parse_length(s).unwrap();
            assert!(close(got, m), "{s}: {got} != {m}");
        }
    }

    #[test]
    fn length_errors() {
        assert_eq!(parse_length(""), Err(UnitError::Empty(String::new())));
        assert!(matches!(parse_length("mm"), Err(UnitError::BadNumber { .. })));
        assert!(matches!(parse_length("1.2.3m"), Err(UnitError::BadNumber { .. })));
        assert!(matches!(parse_length("inf"), Err(UnitError::BadNumber { .. }) | Err(UnitError::UnknownUnit { .. })));
        assert!(matches!(parse_length("1e999m"), Err(UnitError::NotFinite(_))));
        let err = parse_length("5yd").unwrap_err();
        assert!(matches!(&err, UnitError::UnknownUnit { unit, .. } if unit == "yd"));
        assert!(err.to_string().contains("mm, cm"), "{err}");
    }

    #[test]
    fn angles() {
        assert!(close(parse_angle("90").unwrap(), std::f64::consts::FRAC_PI_2));
        assert!(close(parse_angle("45deg").unwrap(), std::f64::consts::FRAC_PI_4));
        assert!(close(parse_angle("0.5rad").unwrap(), 0.5));
        assert!(matches!(parse_angle("3grad"), Err(UnitError::UnknownUnit { .. })));
    }

    #[test]
    fn formatting() {
        assert_eq!(format_length(0.012), "12 mm");
        assert_eq!(format_length(0.0125), "12.5 mm");
        assert_eq!(format_length(0.45), "0.45 m");
        assert_eq!(format_length(2.0), "2 m");
        assert_eq!(format_length(0.0), "0 m");
        assert_eq!(format_angle(std::f64::consts::FRAC_PI_2), "90 deg");
        assert_eq!(Meters(1.25).to_string(), "1.25 m");
    }
}
