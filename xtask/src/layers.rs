//! `cargo xtask layers`: machine check of the crate layer table
//! (`docs/architecture.md`, ADR 0001).
//!
//! The rules work on a small graph model so they can be unit-tested;
//! `from_metadata` builds that model from `cargo metadata`.

use std::collections::{BTreeSet, VecDeque};
use std::fmt;

use serde_json::Value;

/// Where a workspace package sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A crate may depend on every crate of a lower layer and on crates of
    /// its own layer with a lower rank.
    Layer { layer: u8, rank: u8 },
    /// A crate that may depend only on lower layers and that no crate of its
    /// own layer may depend on (the renderer).
    Isolated { layer: u8 },
    /// Test support: depends on layers up to `max_layer`; other crates may
    /// use it only as a dev-dependency.
    Support { max_layer: u8 },
    /// Apps and tooling: exempt from the rules, never a dependency.
    Exempt,
}

impl Class {
    /// Layer used by the external-crate rules (support counts as its top layer).
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer { layer, .. } | Class::Isolated { layer } => Some(layer),
            Class::Support { max_layer } => Some(max_layer),
            Class::Exempt => None,
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Class::Layer { layer, rank } => write!(f, "L{layer}.{rank}"),
            Class::Isolated { layer } => write!(f, "L{layer} isolated"),
            Class::Support { max_layer } => write!(f, "support (<= L{max_layer})"),
            Class::Exempt => write!(f, "exempt"),
        }
    }
}

const fn l(layer: u8, rank: u8) -> Class {
    Class::Layer { layer, rank }
}

/// The layer table. Names are package names without the `p3d-` prefix.
pub const TABLE: &[(&str, Class)] = &[
    // L0 foundation
    ("math", l(0, 0)),
    ("mesh", l(0, 1)),
    // L1 document
    ("scene", l(1, 0)),
    ("query", l(1, 1)),
    // L2 algorithms
    ("geo", l(2, 0)),
    ("tex", l(2, 0)),
    ("uv", l(2, 1)),
    ("opt", l(2, 1)),
    ("rig", l(2, 1)),
    ("bake", l(2, 1)),
    ("anim", l(2, 2)),
    ("sdf", l(2, 2)),
    ("remesh", l(2, 2)),
    ("features", l(2, 3)),
    ("strands", l(2, 3)),
    ("render", Class::Isolated { layer: 2 }),
    // L3 IO and analysis
    ("profiles", l(3, 0)),
    ("io", l(3, 1)),
    ("analyze", l(3, 1)),
    ("perceive", l(3, 2)),
    // L4 engine
    ("ops", l(4, 0)),
    ("graph", l(4, 1)),
    ("spec", l(4, 2)),
    ("engine", l(4, 3)),
    // L5 frontends
    ("plugin", l(5, 0)),
    ("control", l(5, 0)),
    ("mcp", l(5, 1)),
    ("ai", l(5, 2)),
    ("ui", l(5, 3)),
    // support, apps and tooling
    ("testdata", Class::Support { max_layer: 3 }),
    ("pixel3d", Class::Exempt),
    ("pixel3d-cli", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// A group of external crates that only some workspace crates may use.
/// Patterns ending in `*` are prefixes.
pub struct Restricted {
    pub group: &'static str,
    pub patterns: &'static [&'static str],
    /// Crates at or above this layer may depend on the group directly.
    pub min_layer: Option<u8>,
    /// Short names of further crates that may depend on the group directly.
    pub crates: &'static [&'static str],
    /// Also forbid reaching the group through other crates (below L5).
    pub transitive: bool,
}

pub const RESTRICTED: &[Restricted] = &[
    Restricted {
        group: "UI toolkit",
        patterns: &["egui*", "eframe", "epaint*", "emath", "winit", "rfd", "muda", "arboard", "accesskit*", "craft-ui", "transform-gizmo*"],
        min_layer: Some(5),
        crates: &[],
        transitive: true,
    },
    Restricted { group: "GPU", patterns: &["wgpu*"], min_layer: Some(5), crates: &["render"], transitive: false },
    Restricted {
        group: "async runtime or HTTP",
        patterns: &["tokio*", "axum*", "reqwest*"],
        min_layer: None,
        crates: &["mcp", "ai"],
        transitive: true,
    },
    Restricted {
        group: "WASM or script engine",
        patterns: &["wasmtime*", "rquickjs*", "boa_*", "oxc*"],
        min_layer: None,
        crates: &["plugin"],
        transitive: true,
    },
];

/// Layers below this one must not reach a transitive group at all.
const FRONTEND_LAYER: u8 = 5;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("p3d-").unwrap_or(pkg)
}

/// Complexity: O(t) in table entries.
pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Complexity: O(g * p) in groups and patterns.
fn group_of(name: &str) -> Option<&'static Restricted> {
    RESTRICTED.iter().find(|r| r.patterns.iter().any(|p| matches(p, name)))
}

fn may_use_directly(r: &Restricted, krate: &str, class: Class) -> bool {
    class == Class::Exempt || r.crates.contains(&short_name(krate)) || matches!((r.min_layer, class.layer()), (Some(min), Some(l)) if l >= min)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub workspace: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub kind: DepKind,
}

/// Resolved dependency graph: packages and their dependency edges.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub packages: Vec<Package>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, from: String, dep: String, to: String, kind: DepKind },
    SupportNotDev { krate: String, dep: String },
    ExemptAsDependency { krate: String, dep: String },
    Direct { krate: String, dep: String, group: &'static str },
    Transitive { krate: String, path: Vec<String>, group: &'static str },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unregistered { krate } => {
                write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE and docs/architecture.md")
            }
            Violation::Upward { krate, from, dep, to, kind } => {
                write!(f, "{krate} ({from}) -> {dep} ({to}) [{kind:?}]: only lower layers, or lower ranks of the same layer, are allowed")
            }
            Violation::SupportNotDev { krate, dep } => write!(f, "{krate} -> {dep}: support crates may only be dev-dependencies"),
            Violation::ExemptAsDependency { krate, dep } => write!(f, "{krate} -> {dep}: apps and tooling are never dependencies"),
            Violation::Direct { krate, dep, group } => write!(f, "{krate} -> {dep}: {group} crates are not allowed in this crate"),
            Violation::Transitive { krate, path, group } => {
                write!(f, "{krate} reaches {group} crate {} via {}", path.last().map_or("?", String::as_str), path.join(" -> "))
            }
        }
    }
}

/// Why a workspace dependency breaks the layer table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    Upward,
    SupportNotDev,
    Exempt,
}

/// Whether a workspace dependency `from -> to` follows the layer table.
fn allowed(from: Class, to: Class, kind: DepKind) -> Result<(), Refusal> {
    let ok = match (from, to) {
        (_, Class::Exempt) => return Err(Refusal::Exempt),
        (_, Class::Support { .. }) => return if kind == DepKind::Dev { Ok(()) } else { Err(Refusal::SupportNotDev) },
        (Class::Exempt, _) => true,
        (Class::Layer { layer: la, rank: ra }, Class::Layer { layer: lb, rank: rb }) => lb < la || (lb == la && rb < ra),
        (Class::Support { max_layer }, _) => to.layer().is_some_and(|lb| lb <= max_layer),
        (Class::Layer { layer: la, .. } | Class::Isolated { layer: la }, Class::Layer { layer: lb, .. } | Class::Isolated { layer: lb }) => lb < la,
    };
    if ok { Ok(()) } else { Err(Refusal::Upward) }
}

/// Checks every rule. Returns violations sorted and deduplicated.
///
/// Complexity: O(W * (P + E)) for W workspace crates, P packages and E edges
/// (one breadth-first search per workspace crate below L5).
pub fn check(g: &Graph) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); g.packages.len()];
    for e in &g.edges {
        if e.kind != DepKind::Dev {
            adjacency[e.from].push(e.to);
        }
    }
    for list in &mut adjacency {
        list.sort_unstable();
        list.dedup();
    }

    for (i, p) in g.packages.iter().enumerate() {
        if !p.workspace {
            continue;
        }
        let Some(class) = classify(&p.name) else {
            out.push(Violation::Unregistered { krate: p.name.clone() });
            continue;
        };
        for e in g.edges.iter().filter(|e| e.from == i) {
            let dep = &g.packages[e.to];
            if e.to == i {
                continue;
            }
            if dep.workspace {
                let Some(dep_class) = classify(&dep.name) else { continue };
                match allowed(class, dep_class, e.kind) {
                    Ok(()) => {}
                    Err(Refusal::Exempt) => out.push(Violation::ExemptAsDependency { krate: p.name.clone(), dep: dep.name.clone() }),
                    Err(Refusal::SupportNotDev) => out.push(Violation::SupportNotDev { krate: p.name.clone(), dep: dep.name.clone() }),
                    Err(Refusal::Upward) => out.push(Violation::Upward {
                        krate: p.name.clone(),
                        from: class.to_string(),
                        dep: dep.name.clone(),
                        to: dep_class.to_string(),
                        kind: e.kind,
                    }),
                }
            } else if let Some(r) = group_of(&dep.name)
                && !may_use_directly(r, &p.name, class)
            {
                out.push(Violation::Direct { krate: p.name.clone(), dep: dep.name.clone(), group: r.group });
            }
        }
        if class.layer().is_some_and(|l| l < FRONTEND_LAYER) {
            transitive(g, &adjacency, i, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Breadth-first search from `start` over normal and build edges; reports
/// every reachable package of a transitive group that is not a direct
/// dependency (direct ones are reported by `check`).
///
/// Complexity: O(P + E).
fn transitive(g: &Graph, adjacency: &[Vec<usize>], start: usize, out: &mut Vec<Violation>) {
    let mut parent: Vec<Option<usize>> = vec![None; g.packages.len()];
    let mut seen = vec![false; g.packages.len()];
    let mut queue = VecDeque::from([start]);
    seen[start] = true;
    while let Some(n) = queue.pop_front() {
        for &m in &adjacency[n] {
            if seen[m] {
                continue;
            }
            seen[m] = true;
            parent[m] = Some(n);
            let pkg = &g.packages[m];
            let forbidden = if pkg.workspace { None } else { group_of(&pkg.name).filter(|r| r.transitive) };
            let Some(r) = forbidden else {
                queue.push_back(m);
                continue;
            };
            // Never walk inside a forbidden crate: one report per entry point.
            if n != start {
                let mut path = vec![pkg.name.clone()];
                let mut cur = n;
                while cur != start {
                    path.push(g.packages[cur].name.clone());
                    cur = parent[cur].unwrap_or(start);
                }
                path.push(g.packages[start].name.clone());
                path.reverse();
                out.push(Violation::Transitive { krate: g.packages[start].name.clone(), path, group: r.group });
            }
        }
    }
}

/// Whether a target cfg applies only to wasm32.
fn wasm_only(cfg: &str) -> bool {
    cfg.contains("target_arch = \"wasm32\"") && !cfg.contains("not(target_arch = \"wasm32\")") && !cfg.contains("any(")
}

/// Builds the graph from `cargo metadata --format-version 1` (with `resolve`).
///
/// Complexity: O(P + E) plus an O(P log P) id index.
pub fn from_metadata(meta: &Value) -> Result<Graph, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: BTreeSet<&str> =
        meta["workspace_members"].as_array().ok_or("metadata: no workspace_members")?.iter().filter_map(Value::as_str).collect();
    let mut ids: Vec<(&str, usize)> = Vec::with_capacity(pkgs.len());
    let mut graph = Graph::default();
    for (i, p) in pkgs.iter().enumerate() {
        let id = p["id"].as_str().ok_or("metadata: package without id")?;
        let name = p["name"].as_str().ok_or("metadata: package without name")?;
        ids.push((id, i));
        graph.packages.push(Package { name: name.to_owned(), workspace: members.contains(id) });
    }
    ids.sort_unstable();
    let index = |id: &str| ids.binary_search_by(|(k, _)| (*k).cmp(id)).ok().map(|j| ids[j].1);
    let nodes = meta["resolve"]["nodes"].as_array().ok_or("metadata: no resolve graph (do not pass --no-deps)")?;
    for node in nodes {
        let from = index(node["id"].as_str().unwrap_or_default()).ok_or("metadata: resolve node without package")?;
        for d in node["deps"].as_array().into_iter().flatten() {
            let to = index(d["pkg"].as_str().unwrap_or_default()).ok_or("metadata: dependency without package")?;
            let mut kinds: BTreeSet<DepKind> = BTreeSet::new();
            for k in d["dep_kinds"].as_array().into_iter().flatten() {
                // pixel3d builds native targets only (addons are separate WASM guests):
                // edges that exist only on wasm32 never reach a pixel3d binary.
                if k["target"].as_str().is_some_and(wasm_only) {
                    continue;
                }
                kinds.insert(match k["kind"].as_str() {
                    Some("dev") => DepKind::Dev,
                    Some("build") => DepKind::Build,
                    _ => DepKind::Normal,
                });
            }
            for kind in kinds {
                graph.edges.push(Edge { from, to, kind });
            }
        }
    }
    Ok(graph)
}

/// Prints the violations, preceded by the table of workspace crates and
/// their workspace dependencies when `table` is set.
///
/// Complexity: O(W * E) for W workspace crates and E edges.
pub fn report(g: &Graph, table: bool) -> Result<(), String> {
    let mut members: Vec<(usize, &Package)> = g.packages.iter().enumerate().filter(|(_, p)| p.workspace).collect();
    members.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    if table {
        print_table(g, &members);
    }
    let violations = check(g);
    if violations.is_empty() {
        println!("layers: OK, {} crates, no violations.", members.len());
        Ok(())
    } else {
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Complexity: O(W * E) for W workspace crates and E edges.
fn print_table(g: &Graph, members: &[(usize, &Package)]) {
    println!("{:<16} {:<20} workspace dependencies", "crate", "layer");
    for (i, p) in members {
        let mut deps: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.from == *i && g.packages[e.to].workspace && e.to != *i)
            .map(|e| {
                let name = short_name(&g.packages[e.to].name).to_owned();
                if e.kind == DepKind::Dev { format!("{name} (dev)") } else { name }
            })
            .collect();
        deps.sort();
        deps.dedup();
        let class = classify(&p.name).map_or_else(|| "UNREGISTERED".to_owned(), |c| c.to_string());
        println!("{:<16} {:<20} {}", short_name(&p.name), class, deps.join(", "));
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use DepKind::*;

    #[test]
    fn wasm_only_edges_are_recognized() {
        assert!(wasm_only(r#"cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))"#));
        assert!(wasm_only(r#"cfg(target_arch = "wasm32")"#));
        assert!(!wasm_only(r#"cfg(not(target_arch = "wasm32"))"#));
        assert!(!wasm_only(r#"cfg(any(target_arch = "wasm32", unix))"#));
        assert!(!wasm_only("cfg(windows)"));
    }

    /// Builds a graph from `(crate, [(dep, kind)])`; names starting with
    /// `p3d-` or naming an app are workspace packages.
    fn graph(spec: &[(&str, &[(&str, DepKind)])]) -> Graph {
        fn idx(g: &mut Graph, name: &str) -> usize {
            if let Some(i) = g.packages.iter().position(|p| p.name == name) {
                return i;
            }
            let workspace = name.starts_with("p3d-") || classify(name).is_some();
            g.packages.push(Package { name: name.to_owned(), workspace });
            g.packages.len() - 1
        }
        let mut g = Graph::default();
        for (krate, deps) in spec {
            let from = idx(&mut g, krate);
            for (dep, kind) in *deps {
                let to = idx(&mut g, dep);
                g.edges.push(Edge { from, to, kind: *kind });
            }
        }
        g
    }

    #[test]
    fn every_workspace_crate_is_registered_once() {
        let mut names: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate entries in TABLE");
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = graph(&[
            ("p3d-mesh", &[("p3d-math", Normal), ("glam", Normal), ("p3d-testdata", Dev)]),
            ("p3d-testdata", &[("p3d-mesh", Normal), ("p3d-io", Normal)]),
            ("p3d-engine", &[("p3d-ops", Normal), ("p3d-spec", Normal), ("p3d-render", Normal)]),
            ("p3d-render", &[("p3d-scene", Normal), ("wgpu", Normal)]),
            ("p3d-ui", &[("p3d-engine", Normal), ("p3d-ai", Normal), ("egui", Normal), ("wgpu", Normal)]),
            ("p3d-mcp", &[("p3d-control", Normal), ("tokio", Normal)]),
            ("p3d-plugin", &[("wasmtime", Normal), ("rquickjs", Normal)]),
            ("pixel3d", &[("p3d-ui", Normal), ("eframe", Normal)]),
        ]);
        assert_eq!(check(&g), vec![]);
    }

    #[test]
    fn upward_dependency_flagged() {
        let v = check(&graph(&[("p3d-scene", &[("p3d-engine", Normal)])]));
        assert!(matches!(&v[..], [Violation::Upward { from, to, .. }] if from == "L1.0" && to == "L4.3"), "{v:?}");
    }

    #[test]
    fn upward_dev_dependency_flagged() {
        let v = check(&graph(&[("p3d-mesh", &[("p3d-engine", Dev)])]));
        assert!(matches!(&v[..], [Violation::Upward { kind: Dev, .. }]), "{v:?}");
    }

    #[test]
    fn same_layer_follows_rank() {
        assert!(check(&graph(&[("p3d-mesh", &[("p3d-math", Normal)])])).is_empty());
        assert!(check(&graph(&[("p3d-engine", &[("p3d-spec", Normal), ("p3d-graph", Normal), ("p3d-ops", Normal)])])).is_empty());
        assert!(check(&graph(&[("p3d-features", &[("p3d-sdf", Normal), ("p3d-remesh", Normal), ("p3d-geo", Normal)])])).is_empty());
        for (a, b) in [("p3d-math", "p3d-mesh"), ("p3d-uv", "p3d-opt"), ("p3d-ops", "p3d-engine"), ("p3d-mcp", "p3d-ui")] {
            let v = check(&graph(&[(a, &[(b, Normal)])]));
            assert!(matches!(&v[..], [Violation::Upward { .. }]), "{a} -> {b}: {v:?}");
        }
    }

    #[test]
    fn render_is_isolated_within_its_layer() {
        assert!(check(&graph(&[("p3d-render", &[("p3d-mesh", Normal), ("p3d-scene", Normal)])])).is_empty());
        assert!(check(&graph(&[("p3d-perceive", &[("p3d-render", Normal)])])).is_empty());
        let v = check(&graph(&[("p3d-render", &[("p3d-geo", Normal)])]));
        assert!(matches!(&v[..], [Violation::Upward { .. }]), "{v:?}");
        let v = check(&graph(&[("p3d-strands", &[("p3d-render", Normal)])]));
        assert!(matches!(&v[..], [Violation::Upward { .. }]), "{v:?}");
    }

    #[test]
    fn support_only_as_dev_dependency() {
        let v = check(&graph(&[("p3d-geo", &[("p3d-testdata", Normal)])]));
        assert!(matches!(&v[..], [Violation::SupportNotDev { .. }]), "{v:?}");
        let v = check(&graph(&[("p3d-testdata", &[("p3d-engine", Normal)])]));
        assert!(matches!(&v[..], [Violation::Upward { .. }]), "{v:?}");
    }

    #[test]
    fn apps_are_never_dependencies() {
        let v = check(&graph(&[("p3d-ui", &[("pixel3d-cli", Normal)])]));
        assert!(matches!(&v[..], [Violation::ExemptAsDependency { .. }]), "{v:?}");
    }

    #[test]
    fn unregistered_crate_is_error() {
        let v = check(&graph(&[("p3d-mystery", &[])]));
        assert!(matches!(&v[..], [Violation::Unregistered { krate }] if krate == "p3d-mystery"), "{v:?}");
    }

    #[test]
    fn restricted_crates_flagged_directly() {
        for (krate, dep) in [
            ("p3d-engine", "egui"),
            ("p3d-scene", "winit"),
            ("p3d-ops", "rfd"),
            ("p3d-io", "wgpu"),
            ("p3d-geo", "wgpu-core"),
            ("p3d-engine", "tokio"),
            ("p3d-ui", "reqwest"),
            ("p3d-control", "axum"),
            ("p3d-mcp", "wasmtime"),
            ("p3d-engine", "oxc_parser"),
            ("p3d-spec", "boa_engine"),
        ] {
            let v = check(&graph(&[(krate, &[(dep, Normal)])]));
            assert!(matches!(&v[..], [Violation::Direct { .. }]), "{krate} -> {dep}: {v:?}");
        }
    }

    #[test]
    fn restricted_crates_flagged_transitively_below_l5() {
        let v = check(&graph(&[("p3d-io", &[("some-crate", Normal)]), ("some-crate", &[("tokio", Normal)])]));
        assert!(matches!(&v[..], [Violation::Transitive { path, .. }] if path == &["p3d-io", "some-crate", "tokio"]), "{v:?}");
        // Through a workspace crate that is itself allowed to use it.
        let v = check(&graph(&[("p3d-engine", &[("p3d-render", Normal)]), ("p3d-render", &[("wgpu", Normal)])]));
        assert!(v.is_empty(), "GPU is checked directly only: {v:?}");
        // L5 crates may reach other groups through allowed L5 crates.
        let v = check(&graph(&[("p3d-ui", &[("p3d-ai", Normal)]), ("p3d-ai", &[("reqwest", Normal)])]));
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn one_report_per_forbidden_entry_point() {
        let v = check(&graph(&[("p3d-io", &[("some-crate", Normal)]), ("some-crate", &[("tokio", Normal)]), ("tokio", &[("tokio-macros", Normal)])]));
        assert_eq!(v.len(), 1, "{v:?}");
        let v = check(&graph(&[("p3d-io", &[("tokio", Normal)]), ("tokio", &[("tokio-macros", Normal)])]));
        assert!(matches!(&v[..], [Violation::Direct { .. }]), "{v:?}");
    }

    #[test]
    fn dev_edges_are_not_followed_transitively() {
        let v = check(&graph(&[("p3d-io", &[("helper", Dev)]), ("helper", &[("tokio", Normal)])]));
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn metadata_parsing() {
        let meta: Value = serde_json::from_str(
            r#"{
              "packages": [
                {"id": "a", "name": "p3d-mesh"},
                {"id": "b", "name": "p3d-math"},
                {"id": "c", "name": "glam"}
              ],
              "workspace_members": ["a", "b"],
              "resolve": {"nodes": [
                {"id": "a", "deps": [
                  {"pkg": "b", "dep_kinds": [{"kind": null}, {"kind": "dev"}]},
                  {"pkg": "c", "dep_kinds": [{"kind": null}]}
                ]},
                {"id": "b", "deps": []},
                {"id": "c", "deps": []}
              ]}
            }"#,
        )
        .unwrap();
        let g = from_metadata(&meta).unwrap();
        assert_eq!(g.packages.len(), 3);
        assert!(g.packages[0].workspace && g.packages[1].workspace && !g.packages[2].workspace);
        let kinds: Vec<DepKind> = g.edges.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Normal, Dev, Normal]);
        assert!(check(&g).is_empty());
    }
}
