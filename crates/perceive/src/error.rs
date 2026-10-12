//! Errors of perception requests, shaped like op errors so frontends can
//! show both the same way.

use std::fmt;

/// A request that cannot be answered: a stable code (the op error codes),
/// what went wrong, where in the request, how to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerceiveError {
    /// `NOT_FOUND`, `AMBIGUOUS_NAME`, `INVALID_PARAM`, `OUT_OF_RANGE` or `EMPTY_SELECTION`.
    pub code: &'static str,
    pub message: String,
    /// JSON pointer into the request.
    pub path: Option<String>,
    pub hint: Option<String>,
    pub suggestions: Vec<String>,
}

impl PerceiveError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        PerceiveError { code, message: message.into(), path: None, hint: None, suggestions: Vec::new() }
    }

    pub fn invalid(path: &str, message: impl Into<String>) -> Self {
        Self::new("INVALID_PARAM", message).at(path)
    }

    pub fn at(mut self, path: &str) -> Self {
        self.path = (!path.is_empty()).then(|| path.to_owned());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_suggestions(mut self, s: Vec<String>) -> Self {
        self.suggestions = s;
        self
    }
}

impl fmt::Display for PerceiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error {}", self.code)?;
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

impl std::error::Error for PerceiveError {}
