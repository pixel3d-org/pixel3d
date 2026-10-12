//! Object names and "did you mean" suggestions.

/// Longest object name in bytes.
pub const MAX_NAME_LEN: usize = 128;

/// Why a name is not acceptable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("name is empty")]
    Empty,
    #[error("name `{0}` is longer than {MAX_NAME_LEN} bytes")]
    TooLong(String),
    #[error("name `{0}` contains `/`, which separates path segments")]
    Slash(String),
    #[error("name `{0}` has leading or trailing whitespace or control characters")]
    Whitespace(String),
    #[error("name `{0}` looks like an id (`o:` prefix is reserved)")]
    LooksLikeId(String),
}

/// Checks that a leaf name is usable in paths and lookups.
///
/// Complexity: O(length).
pub fn check_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong(name.to_owned()));
    }
    if name.contains('/') {
        return Err(NameError::Slash(name.to_owned()));
    }
    if name.trim() != name || name.chars().any(char::is_control) {
        return Err(NameError::Whitespace(name.to_owned()));
    }
    if name.starts_with("o:") {
        return Err(NameError::LooksLikeId(name.to_owned()));
    }
    Ok(())
}

/// Levenshtein distance between two strings (by chars).
///
/// Complexity: O(|a| x |b|) time, O(|b|) memory.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// Up to `max` candidates closest to `name` (case-insensitive edit distance
/// at most a third of the name's length, at least 2), best first, ties in
/// candidate order. Candidates containing `name` as a substring also count.
///
/// Complexity: O(c x L^2) for c candidates of length L (error paths only).
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>, max: usize) -> Vec<String> {
    let lower = name.to_lowercase();
    let limit = (lower.chars().count() / 3).max(2);
    let mut scored: Vec<(usize, usize, &str)> = candidates
        .into_iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let cl = c.to_lowercase();
            let d = if cl.contains(&lower) || lower.contains(&cl) { 0 } else { edit_distance(&lower, &cl) };
            (d <= limit).then_some((d, i, c))
        })
        .collect();
    scored.sort_unstable();
    scored.dedup_by(|a, b| a.2 == b.2);
    scored.into_iter().take(max).map(|(_, _, c)| c.to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(check_name("crate"), Ok(()));
        assert_eq!(check_name("lid 2.v1"), Ok(()));
        assert_eq!(check_name(""), Err(NameError::Empty));
        assert!(matches!(check_name("a/b"), Err(NameError::Slash(_))));
        assert!(matches!(check_name(" a"), Err(NameError::Whitespace(_))));
        assert!(matches!(check_name("a\tb"), Err(NameError::Whitespace(_))));
        assert!(matches!(check_name("o:3"), Err(NameError::LooksLikeId(_))));
        assert!(matches!(check_name(&"x".repeat(129)), Err(NameError::TooLong(_))));
    }

    #[test]
    fn distance() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
    }

    #[test]
    fn suggestions() {
        let c = ["width", "height", "depth", "segments", "bevel_width"];
        assert_eq!(suggest("widht", c, 3), vec!["width"]);
        assert_eq!(suggest("width", c, 3), vec!["width", "bevel_width"]);
        assert_eq!(suggest("HEIGHT", c, 3), vec!["height"]);
        assert!(suggest("zzzzzz", c, 3).is_empty());
        assert_eq!(suggest("seg", c, 1), vec!["segments"]);
    }
}
