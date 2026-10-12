//! Operation errors: stable codes, JSON pointers, hints and suggestions.

use std::fmt;

use p3d_query::QueryError;
use p3d_scene::SceneError;
use serde::{Deserialize, Serialize};

/// Stable error codes (UPPER_SNAKE in JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnknownOp,
    InvalidParam,
    OutOfRange,
    NotFound,
    AmbiguousName,
    EmptySelection,
    Topology,
    NonManifoldInput,
    BudgetExceeded,
    Cancelled,
    Timeout,
    Io,
    PathNotAllowed,
    Unsupported,
    AddonTrap,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::UnknownOp => "UNKNOWN_OP",
            ErrorCode::InvalidParam => "INVALID_PARAM",
            ErrorCode::OutOfRange => "OUT_OF_RANGE",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::AmbiguousName => "AMBIGUOUS_NAME",
            ErrorCode::EmptySelection => "EMPTY_SELECTION",
            ErrorCode::Topology => "TOPOLOGY",
            ErrorCode::NonManifoldInput => "NON_MANIFOLD_INPUT",
            ErrorCode::BudgetExceeded => "BUDGET_EXCEEDED",
            ErrorCode::Cancelled => "CANCELLED",
            ErrorCode::Timeout => "TIMEOUT",
            ErrorCode::Io => "IO",
            ErrorCode::PathNotAllowed => "PATH_NOT_ALLOWED",
            ErrorCode::Unsupported => "UNSUPPORTED",
            ErrorCode::AddonTrap => "ADDON_TRAP",
            ErrorCode::Internal => "INTERNAL",
        }
    }
}

/// An operation error, written for agents: what went wrong with numbers,
/// where (JSON pointer into the request), how to fix it, nearest valid values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    pub retryable: bool,
}

impl OpError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        OpError { code, message: message.into(), path: None, hint: None, suggestions: Vec::new(), retryable: false }
    }

    pub fn invalid(path: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParam, message).at(path)
    }

    pub fn out_of_range(path: &str, message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OutOfRange, message).at(path)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    /// Sets the JSON pointer (empty means the whole request).
    pub fn at(mut self, path: &str) -> Self {
        self.path = if path.is_empty() { None } else { Some(path.to_owned()) };
        self
    }

    /// Prefixes the JSON pointer, for example with `/ops/2/args`.
    pub fn prefixed(mut self, prefix: &str) -> Self {
        self.path = Some(format!("{prefix}{}", self.path.as_deref().unwrap_or("")));
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// Maps a scene error found while handling the parameter at `path`.
    pub fn from_scene(e: SceneError, path: &str) -> Self {
        match e {
            SceneError::NotFound { target, suggestions } => Self::new(ErrorCode::NotFound, format!("no object `{target}`"))
                .at(path)
                .with_suggestions(suggestions)
                .with_hint("refer to objects by name, path (parent/child) or id (o:12)"),
            SceneError::Ambiguous { name, candidates } => {
                Self::new(ErrorCode::AmbiguousName, format!("`{name}` matches {} objects", candidates.len()))
                    .at(path)
                    .with_suggestions(candidates)
                    .with_hint("use one of the paths or an id")
            }
            SceneError::NameTaken { path: taken, suggestion } => {
                Self::invalid(path, format!("an object `{taken}` already exists")).with_suggestions(vec![suggestion])
            }
            e @ (SceneError::InvalidName(_)
            | SceneError::Cycle { .. }
            | SceneError::SingularParent { .. }
            | SceneError::IdTaken(_)
            | SceneError::HasChildren(_)) => Self::invalid(path, e.to_string()),
            e @ (SceneError::StaleObject | SceneError::StaleMesh | SceneError::StaleCurve) => Self::internal(e.to_string()),
        }
    }

    /// Maps a selector error; `path` points at the selector in the request.
    pub fn from_query(e: QueryError, path: &str) -> Self {
        let at = format!("{path}{}", e.path());
        match e {
            QueryError::NoGroup { name, suggestions, .. } => Self::new(ErrorCode::NotFound, format!("no group `{name}`"))
                .at(&at)
                .with_suggestions(suggestions)
                .with_hint("op outputs list the groups they create; select.save makes new ones"),
            other => Self::invalid(&at, other.to_string()),
        }
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error {}", self.code.as_str())?;
        if let Some(p) = &self.path {
            write!(f, " at {p}")?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(h) = &self.hint {
            write!(f, "\nhint: {h}")?;
        }
        if !self.suggestions.is_empty() {
            write!(f, "\ndid you mean: {}", self.suggestions.join(", "))?;
        }
        Ok(())
    }
}

impl std::error::Error for OpError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_for_agents() {
        let e = OpError::invalid("/width", "0.08 m is wider than the shortest adjacent edge (0.05 m)")
            .prefixed("/ops/0/args")
            .with_hint("use width <= 0.024");
        assert_eq!(
            e.to_string(),
            "error INVALID_PARAM at /ops/0/args/width: 0.08 m is wider than the shortest adjacent edge (0.05 m)\nhint: use width <= 0.024"
        );
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["code"], "INVALID_PARAM");
        assert!(json.get("suggestions").is_none());
    }

    #[test]
    fn scene_errors_map_to_codes() {
        let e = OpError::from_scene(SceneError::NotFound { target: "lidd".into(), suggestions: vec!["lid".into()] }, "/target");
        assert_eq!((e.code, e.path.as_deref(), e.suggestions.clone()), (ErrorCode::NotFound, Some("/target"), vec!["lid".to_owned()]));
        let e = OpError::from_scene(SceneError::Ambiguous { name: "lid".into(), candidates: vec!["a/lid".into(), "b/lid".into()] }, "/target");
        assert_eq!(e.code, ErrorCode::AmbiguousName);
        assert_eq!(serde_json::to_value(ErrorCode::EmptySelection).unwrap(), "EMPTY_SELECTION");
    }
}
