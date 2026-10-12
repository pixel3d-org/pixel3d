//! `cargo xtask emdash`: repository-wide scan for the em dash (U+2014) and
//! the en dash (U+2013).
//!
//! Scans every file git knows about (tracked plus untracked, not ignored),
//! so local-only files listed in `.git/info/exclude` are skipped.

use std::path::Path;
use std::process::Command;

/// Forbidden characters and how they are reported.
pub const FORBIDDEN: [(char, &str); 2] = [('\u{2014}', "em dash (U+2014)"), ('\u{2013}', "en dash (U+2013)")];

/// Files whose first bytes contain a NUL are treated as binary and skipped.
const BINARY_SNIFF: usize = 8192;

/// One forbidden character in a file. Line and column are 1-based; the
/// column counts characters, not bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub line: usize,
    pub column: usize,
    pub what: &'static str,
}

/// Finds every forbidden character in `bytes`. Binary content yields no
/// hits; invalid UTF-8 is decoded lossily.
///
/// Complexity: O(n) in bytes.
pub fn scan(bytes: &[u8]) -> Vec<Hit> {
    if bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(bytes);
    let mut hits = Vec::new();
    for (l, line) in text.lines().enumerate() {
        for (c, ch) in line.chars().enumerate() {
            if let Some((_, what)) = FORBIDDEN.iter().find(|(f, _)| *f == ch) {
                hits.push(Hit { line: l + 1, column: c + 1, what });
            }
        }
    }
    hits
}

/// Files to scan: tracked and untracked, minus ignored and excluded ones.
///
/// Complexity: O(f) in files.
fn files(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-files failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let mut paths: Vec<String> = out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Scans the repository and fails if any forbidden character is found.
///
/// Complexity: O(total bytes) over the scanned files.
pub fn run(root: &Path) -> Result<(), String> {
    let paths = files(root)?;
    let mut total = 0usize;
    let mut scanned = 0usize;
    for rel in &paths {
        let path = root.join(rel);
        // Deleted but still tracked files and directories (submodules) are skipped.
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
        scanned += 1;
        for hit in scan(&bytes) {
            println!("{rel}:{}:{}: {}", hit.line, hit.column, hit.what);
            total += 1;
        }
    }
    if total == 0 {
        println!("emdash: OK, {scanned} files scanned, no em or en dashes.");
        Ok(())
    } else {
        Err(format!("{total} em or en dash(es) found; use a colon, comma, parentheses or a hyphen"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_has_no_hits() {
        assert!(scan(b"plain - text: with (punctuation), only\n").is_empty());
    }

    #[test]
    fn em_dash_is_located() {
        let text = "first line\nab\u{2014}cd\n";
        assert_eq!(scan(text.as_bytes()), vec![Hit { line: 2, column: 3, what: "em dash (U+2014)" }]);
    }

    #[test]
    fn en_dash_is_reported() {
        let text = "1\u{2013}2";
        assert_eq!(scan(text.as_bytes()), vec![Hit { line: 1, column: 2, what: "en dash (U+2013)" }]);
    }

    #[test]
    fn column_counts_characters_not_bytes() {
        let text = "\u{e9}\u{e9}\u{2014}";
        assert_eq!(scan(text.as_bytes())[0].column, 3);
    }

    #[test]
    fn every_occurrence_is_reported() {
        let text = "\u{2014}\u{2014}\n\u{2013}";
        assert_eq!(scan(text.as_bytes()).len(), 3);
    }

    #[test]
    fn binary_files_are_skipped() {
        let mut bytes = vec![0u8, 1, 2];
        bytes.extend_from_slice("\u{2014}".as_bytes());
        assert!(scan(&bytes).is_empty());
    }

    #[test]
    fn invalid_utf8_still_scanned() {
        let mut bytes = vec![0xff, b'\n'];
        bytes.extend_from_slice("\u{2014}".as_bytes());
        assert_eq!(scan(&bytes), vec![Hit { line: 2, column: 1, what: "em dash (U+2014)" }]);
    }
}
