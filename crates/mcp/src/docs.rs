//! Agent guides, embedded at build time from `docs/agents/*.md`, plus a
//! generated op list.

use p3d_ops::{OpError, Registry};
use serde_json::json;

use crate::agent::ToolOutput;
use crate::args::DocsArgs;

/// Guide topics with a one-line description and their markdown.
pub const GUIDES: &[(&str, &str, &str)] = &[
    ("quickstart", "conventions, the modelling loop, a crate in two calls, the tools", include_str!("../../../docs/agents/quickstart.md")),
    ("modeling", "ops, placing parts, sizes and budgets, selectors", include_str!("../../../docs/agents/modeling.md")),
    ("debugging", "perception aspects, diagnostics codes and fixes, expectations", include_str!("../../../docs/agents/debugging.md")),
];

/// Every op id with its summary, grouped by category.
///
/// Complexity: O(ops).
pub fn ops_guide(registry: &Registry) -> String {
    let mut t = String::from("# Ops\n\n`describe_op` shows parameters, defaults and examples of one op.\n");
    let mut last = "";
    for op in registry.iter() {
        let c = op.meta().category.as_str();
        if c != last {
            t.push_str(&format!("\n## {c}\n\n"));
            last = c;
        }
        t.push_str(&format!("- `{}`: {}\n", op.id(), op.meta().summary));
    }
    t
}

/// The text of a topic, if it exists.
pub fn topic(name: &str) -> Option<String> {
    if name == "ops" {
        return Some(ops_guide(&Registry::builtin()));
    }
    GUIDES.iter().find(|(n, _, _)| *n == name).map(|(_, _, text)| (*text).to_owned())
}

/// The `docs` tool.
pub fn tool(a: &DocsArgs) -> Result<ToolOutput, OpError> {
    match &a.topic {
        None => {
            let mut t = String::from("guides (docs with topic):\n");
            for (name, about, _) in GUIDES {
                t.push_str(&format!("- {name}: {about}\n"));
            }
            t.push_str("- ops: every op with a one-line summary\n");
            let topics: Vec<&str> = GUIDES.iter().map(|g| g.0).chain(std::iter::once("ops")).collect();
            Ok(ToolOutput::ok(t, json!({ "topics": topics })))
        }
        Some(name) => {
            let text = topic(name).ok_or_else(|| {
                OpError::new(p3d_ops::ErrorCode::NotFound, format!("no guide `{name}`")).at("/topic").with_suggestions(p3d_scene::name::suggest(
                    name,
                    GUIDES.iter().map(|g| g.0).chain(std::iter::once("ops")),
                    3,
                ))
            })?;
            Ok(ToolOutput::ok(text, json!({ "topic": name })))
        }
    }
}
