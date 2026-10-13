//! Archetypes (ADR 0011, ADR 0016): genome templates that expand into a
//! named skeleton, generated parts and default genes when a spec is parsed.

use std::borrow::Cow;

use serde_json::Value;

use crate::SpecError;

/// The spec with every archetype instance expanded; the source itself when
/// it has none.
///
/// Complexity: O(size of the expanded spec).
pub fn expand(source: &Value) -> Result<Cow<'_, Value>, SpecError> {
    match source.get("archetypes") {
        None => Ok(Cow::Borrowed(source)),
        Some(Value::Object(m)) if m.is_empty() => Ok(Cow::Borrowed(source)),
        Some(_) => Err(SpecError::new("/archetypes", "archetypes are not available yet")),
    }
}
