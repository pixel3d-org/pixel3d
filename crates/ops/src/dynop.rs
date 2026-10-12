//! Object-safe, JSON-in JSON-out view of an op. Built-in ops are adapted
//! with [`Typed`]; addon and script ops implement [`DynOp`] directly.

use std::marker::PhantomData;

use p3d_scene::Document;
use p3d_scene::name::suggest;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::OpError;
use crate::op::{Op, OpCtx, OpMeta, OpOutput};
use crate::tx::Tx;

/// The result of running an op: its output and the canonical parameters
/// (defaults filled in, units converted) that the history records.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub output: OpOutput,
    pub params: Value,
}

/// An op as the registry stores it.
pub trait DynOp: Send + Sync {
    fn id(&self) -> &'static str;
    fn meta(&self) -> OpMeta;
    /// JSON Schema of the parameters (strict: unknown fields rejected).
    fn params_schema(&self) -> Value;
    /// Parameters with every default filled in.
    fn default_params(&self) -> Value;
    /// Parses and validates without changing anything; returns canonical parameters.
    fn check(&self, doc: &Document, args: &Value) -> Result<Value, OpError>;
    /// Parses, validates and applies.
    fn run(&self, tx: &mut Tx, args: &Value, ctx: &OpCtx) -> Result<Applied, OpError>;
}

impl std::fmt::Debug for dyn DynOp + '_ {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DynOp({})", self.id())
    }
}

/// Adapts a typed [`Op`] to [`DynOp`].
pub struct Typed<O: Op>(PhantomData<fn() -> O>);

impl<O: Op> Typed<O> {
    pub fn new() -> Self {
        Typed(PhantomData)
    }
}

impl<O: Op> Default for Typed<O> {
    fn default() -> Self {
        Self::new()
    }
}

fn canonical<T: serde::Serialize>(p: &T) -> Result<Value, OpError> {
    serde_json::to_value(p).map_err(|e| OpError::internal(format!("cannot serialize parameters: {e}")))
}

impl<O: Op> DynOp for Typed<O> {
    fn id(&self) -> &'static str {
        O::ID
    }

    fn meta(&self) -> OpMeta {
        O::meta()
    }

    fn params_schema(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(O::Params)).unwrap_or(Value::Null)
    }

    fn default_params(&self) -> Value {
        serde_json::to_value(O::Params::default()).unwrap_or(Value::Null)
    }

    fn check(&self, doc: &Document, args: &Value) -> Result<Value, OpError> {
        let p: O::Params = parse_params(args)?;
        O::validate(doc, &p)?;
        canonical(&p)
    }

    fn run(&self, tx: &mut Tx, args: &Value, ctx: &OpCtx) -> Result<Applied, OpError> {
        let p: O::Params = parse_params(args)?;
        O::validate(tx.doc(), &p)?;
        let output = O::apply(tx, &p, ctx)?;
        Ok(Applied { output, params: canonical(&p)? })
    }
}

/// JSON pointer of a deserialization path.
fn pointer(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let mut out = String::new();
    for seg in path.iter() {
        match seg {
            Segment::Seq { index } => out.push_str(&format!("/{index}")),
            Segment::Map { key } => out.push_str(&format!("/{}", key.replace('~', "~0").replace('/', "~1"))),
            Segment::Enum { variant } => out.push_str(&format!("/{variant}")),
            Segment::Unknown => {}
        }
    }
    out
}

/// Names between backticks in a serde message, in order.
fn backticked(s: &str) -> Vec<&str> {
    s.split('`').skip(1).step_by(2).collect()
}

/// Parses op parameters strictly. Errors are `INVALID_PARAM` with a JSON
/// pointer; unknown fields and variants come with "did you mean" suggestions.
/// `null` means "all defaults".
pub fn parse_params<T: DeserializeOwned>(args: &Value) -> Result<T, OpError> {
    let empty = Value::Object(serde_json::Map::new());
    let args = if args.is_null() { &empty } else { args };
    serde_path_to_error::deserialize::<_, T>(args).map_err(|e| {
        let mut path = pointer(e.path());
        let msg = e.inner().to_string();
        let names = backticked(&msg);
        let (message, suggestions) = if msg.starts_with("unknown field") || msg.starts_with("unknown variant") {
            let bad = names.first().copied().unwrap_or_default();
            let expected = names.iter().skip(1).copied();
            (msg.split(',').next().unwrap_or(&msg).to_owned(), suggest(bad, expected, 3))
        } else if msg.starts_with("missing field") {
            path = format!("{path}/{}", names.first().copied().unwrap_or_default());
            (msg.clone(), Vec::new())
        } else {
            (msg.clone(), Vec::new())
        };
        OpError::invalid(&path, message).with_suggestions(suggestions)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Length;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        width: Option<Length>,
        #[serde(default)]
        bevel: Option<Bevel>,
        #[serde(default)]
        mode: Mode,
    }

    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Bevel {
        width: Length,
    }

    #[derive(Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
    #[serde(rename_all = "snake_case")]
    enum Mode {
        #[default]
        Copy,
        Instance,
    }

    #[test]
    fn null_means_defaults() {
        assert_eq!(parse_params::<P>(&Value::Null).unwrap(), P::default());
    }

    #[test]
    fn unknown_field_points_at_it_with_suggestions() {
        let e = parse_params::<P>(&json!({ "widht": 1 })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some("/widht"));
        assert_eq!(e.message, "unknown field `widht`");
        assert_eq!(e.suggestions, vec!["width"]);
        let e = parse_params::<P>(&json!({ "bevel": { "wdth": 1 } })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some("/bevel/wdth"));
    }

    #[test]
    fn bad_values_point_at_the_value() {
        let e = parse_params::<P>(&json!({ "bevel": { "width": "3yd" } })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some("/bevel/width"));
        assert!(e.message.contains("unknown unit"), "{}", e.message);
        let e = parse_params::<P>(&json!({ "mode": "instanse" })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some("/mode"));
        assert_eq!(e.suggestions, vec!["instance"]);
        let e = parse_params::<P>(&json!({ "bevel": {} })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some("/bevel/width"));
        assert!(e.message.contains("missing field"));
    }
}
