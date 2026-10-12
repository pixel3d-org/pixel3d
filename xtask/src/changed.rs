//! Which workspace crates a working-tree change affects (for `ci-fast`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::Value;

/// Root files whose change affects every crate's build or tests.
const GLOBAL_FILES: &[&str] = &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml", "rustfmt.toml"];
/// Root directories whose change affects every crate's build or tests.
const GLOBAL_DIRS: &[&str] = &[".cargo/"];

/// A workspace crate: package name, directory relative to the workspace
/// root (with `/` separators and a trailing `/`), and the workspace crates
/// it depends on (any dependency kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub dir: String,
    pub deps: Vec<String>,
}

/// Crates to test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    All,
    Some(Vec<String>),
}

/// Paths from `git status --porcelain=v1 -z --untracked-files=all`,
/// including both sides of renames and copies.
///
/// Complexity: O(n) in bytes.
pub fn parse_porcelain(out: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry[0], entry[1]);
        paths.push(String::from_utf8_lossy(&entry[3..]).into_owned());
        if (matches!(x, b'R' | b'C') || matches!(y, b'R' | b'C'))
            && let Some(orig) = fields.next()
        {
            paths.push(String::from_utf8_lossy(orig).into_owned());
        }
    }
    paths
}

/// Changed crates plus every crate that depends on them, directly or not.
///
/// Complexity: O(f * m + m + e) for f changed files, m members and e
/// workspace dependency edges.
pub fn affected(paths: &[String], members: &[Member]) -> Selection {
    let mut changed: BTreeSet<&str> = BTreeSet::new();
    for path in paths {
        if GLOBAL_FILES.contains(&path.as_str()) || GLOBAL_DIRS.iter().any(|d| path.starts_with(d)) {
            return Selection::All;
        }
        // Longest directory prefix wins (nested crates).
        if let Some(m) = members.iter().filter(|m| path.starts_with(&m.dir)).max_by_key(|m| m.dir.len()) {
            changed.insert(&m.name);
        }
    }
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for m in members {
        for d in &m.deps {
            dependents.entry(d.as_str()).or_default().push(&m.name);
        }
    }
    let mut out: BTreeSet<&str> = changed.clone();
    let mut queue: VecDeque<&str> = changed.into_iter().collect();
    while let Some(n) = queue.pop_front() {
        for &d in dependents.get(n).into_iter().flatten() {
            if out.insert(d) {
                queue.push_back(d);
            }
        }
    }
    Selection::Some(out.into_iter().map(str::to_owned).collect())
}

/// Workspace members from `cargo metadata --format-version 1`.
///
/// Complexity: O(p + d) in packages and declared dependencies.
pub fn members(meta: &Value) -> Result<Vec<Member>, String> {
    let root = Path::new(meta["workspace_root"].as_str().ok_or("metadata: no workspace_root")?);
    let ids: BTreeSet<&str> =
        meta["workspace_members"].as_array().ok_or("metadata: no workspace_members")?.iter().filter_map(Value::as_str).collect();
    let pkgs: Vec<&Value> =
        meta["packages"].as_array().ok_or("metadata: no packages")?.iter().filter(|p| p["id"].as_str().is_some_and(|id| ids.contains(id))).collect();
    let names: BTreeSet<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("metadata: package without name")?;
        let manifest = Path::new(p["manifest_path"].as_str().ok_or("metadata: package without manifest_path")?);
        let dir = manifest.parent().and_then(|d| d.strip_prefix(root).ok()).ok_or_else(|| format!("{name}: manifest outside the workspace"))?;
        let mut rel: String = dir.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        rel.push('/');
        let mut deps: Vec<String> = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["name"].as_str())
            .filter(|d| names.contains(d) && *d != name)
            .map(str::to_owned)
            .collect();
        deps.sort();
        deps.dedup();
        out.push(Member { name: name.to_owned(), dir: rel, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, dir: &str, deps: &[&str]) -> Member {
        Member { name: name.into(), dir: dir.into(), deps: deps.iter().map(|d| (*d).into()).collect() }
    }

    fn ws() -> Vec<Member> {
        vec![
            m("p3d-math", "crates/math/", &[]),
            m("p3d-mesh", "crates/mesh/", &["p3d-math"]),
            m("p3d-scene", "crates/scene/", &["p3d-mesh"]),
            m("p3d-ui", "crates/ui/", &[]),
            m("pixel3d", "apps/pixel3d/", &["p3d-scene"]),
            m("pixel3d-cli", "apps/pixel3d-cli/", &[]),
        ]
    }

    fn some(names: &[&str]) -> Selection {
        Selection::Some(names.iter().map(|n| (*n).into()).collect())
    }

    #[test]
    fn porcelain_paths_and_renames() {
        let out = b" M crates/mesh/src/lib.rs\0?? docs/new.md\0R  crates/a.rs\0crates/b.rs\0";
        assert_eq!(parse_porcelain(out), vec!["crates/mesh/src/lib.rs", "docs/new.md", "crates/a.rs", "crates/b.rs"]);
    }

    #[test]
    fn change_selects_crate_and_dependents() {
        assert_eq!(affected(&["crates/math/src/lib.rs".into()], &ws()), some(&["p3d-math", "p3d-mesh", "p3d-scene", "pixel3d"]));
        assert_eq!(affected(&["crates/ui/Cargo.toml".into()], &ws()), some(&["p3d-ui"]));
    }

    #[test]
    fn app_directories_do_not_overlap() {
        assert_eq!(affected(&["apps/pixel3d-cli/src/main.rs".into()], &ws()), some(&["pixel3d-cli"]));
    }

    #[test]
    fn docs_only_change_selects_nothing() {
        assert_eq!(affected(&["docs/STATUS.md".into(), "README.md".into()], &ws()), some(&[]));
    }

    #[test]
    fn root_build_files_select_everything() {
        for f in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "clippy.toml", ".cargo/config.toml"] {
            assert_eq!(affected(&[f.into()], &ws()), Selection::All, "{f}");
        }
    }

    #[test]
    fn metadata_members() {
        let meta: Value = serde_json::from_str(
            r#"{
              "workspace_root": "/w",
              "workspace_members": ["a", "b"],
              "packages": [
                {"id": "a", "name": "p3d-mesh", "manifest_path": "/w/crates/mesh/Cargo.toml",
                 "dependencies": [{"name": "p3d-math"}, {"name": "glam"}, {"name": "p3d-math"}]},
                {"id": "b", "name": "p3d-math", "manifest_path": "/w/crates/math/Cargo.toml", "dependencies": []},
                {"id": "c", "name": "glam", "manifest_path": "/registry/glam/Cargo.toml", "dependencies": []}
              ]
            }"#,
        )
        .unwrap();
        let got = members(&meta).unwrap();
        assert_eq!(got, vec![m("p3d-math", "crates/math/", &[]), m("p3d-mesh", "crates/mesh/", &["p3d-math"])]);
    }
}
