//! Text perception, diagnostics and expectation checks.
//!
//! Layer L3 (IO and analysis). See `docs/architecture.md`.
//!
//! Every question an agent would answer with a screenshot gets a compact,
//! unit-annotated text answer here (project spec 17.7): what is where
//! (`structure`), what each part looks like (`shape`), the outline per view
//! (`silhouette`), slices (`section`), how parts fit together
//! (`relations`), what is at a point (`probe`) and what an edit changed
//! (`diff`). Each aspect returns a [`Report`]: text for the model plus the
//! same data as JSON. Default reports stay under about 600 tokens.

pub mod error;
pub mod fmt;
pub mod relations;
pub mod sample;
pub mod section;
pub mod shape;
pub mod silhouette;
pub mod structure;
pub mod view;

use serde::Serialize;

pub use error::PerceiveError;
pub use view::{Part, View};

/// Token budget of a default perception report (project spec 17.7).
pub const TOKEN_BUDGET: usize = 600;

/// A perception answer: compact text for the model and the same data as
/// JSON for programs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub aspect: &'static str,
    pub text: String,
    pub data: serde_json::Value,
}

impl Report {
    /// Numbers in `data` are rounded to 5 decimals (10 micrometers for
    /// lengths): stable across platforms and free of float noise.
    pub fn new(aspect: &'static str, text: String, data: &impl Serialize) -> Report {
        let mut data = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
        round_floats(&mut data);
        Report { aspect, text, data }
    }

    /// Estimated model tokens of the text.
    pub fn tokens(&self) -> usize {
        fmt::approx_tokens(&self.text)
    }
}

/// Rounds every non-integer number in a JSON value to 5 decimals.
///
/// Complexity: O(size of the value).
pub fn round_floats(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            if let Some(x) = n.as_f64() {
                let r = (x * 1e5).round() / 1e5;
                let r = if r == 0.0 { 0.0 } else { r };
                if let Some(num) = serde_json::Number::from_f64(r) {
                    *n = num;
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(round_floats),
        serde_json::Value::Object(o) => o.values_mut().for_each(round_floats),
        _ => {}
    }
}
