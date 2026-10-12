//! Units, angles, axis conventions, transforms and robust predicates.
//!
//! Layer L0 (foundation). See `docs/architecture.md`.
//!
//! Internal units are meters and radians; positions are `f32`, numeric
//! solves use `f64`. The internal axis convention is glTF's: right-handed,
//! +Y up, +Z front.

pub mod axes;
pub mod transform;
pub mod units;

pub use axes::{Axis, AxisSystem, Handedness};
pub use glam;
pub use transform::Trs;
pub use units::{Meters, UnitError, format_angle, format_length, parse_angle, parse_length};
