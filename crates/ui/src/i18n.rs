//! Interface strings. Every user-facing string goes through [`t`], keyed by
//! its English text (cadcraft's TSV catalog pattern). English is the source
//! language; other catalogs are tab-separated `english<TAB>translation`
//! lines loaded with [`load_catalog`].

use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    static CATALOG: RefCell<Option<BTreeMap<&'static str, &'static str>>> = const { RefCell::new(None) };
}

/// The translation of `english` in the active catalog, or `english` itself.
///
/// Complexity: O(log entries).
pub fn t(english: &'static str) -> &'static str {
    CATALOG.with(|c| c.borrow().as_ref().and_then(|m| m.get(english).copied()).unwrap_or(english))
}

/// Parses a TSV catalog (`english<TAB>translation` per line; `#` comments).
///
/// Complexity: O(n log n) in lines.
pub fn parse_catalog(tsv: &'static str) -> BTreeMap<&'static str, &'static str> {
    tsv.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// Makes a catalog active on this thread; `None` returns to English.
pub fn load_catalog(catalog: Option<BTreeMap<&'static str, &'static str>>) {
    CATALOG.with(|c| *c.borrow_mut() = catalog);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_the_default_and_catalogs_translate() {
        assert_eq!(t("Open"), "Open");
        load_catalog(Some(parse_catalog("# test\nOpen\tAbrir\nbroken line\n")));
        assert_eq!(t("Open"), "Abrir");
        assert_eq!(t("Save"), "Save", "missing entries stay English");
        load_catalog(None);
        assert_eq!(t("Open"), "Open");
    }
}
