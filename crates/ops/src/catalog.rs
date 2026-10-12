//! The op catalog as JSON: the single source for generated docs, TypeScript
//! types, WIT and agent discovery.

use serde_json::{Value, json};

use crate::registry::Registry;

/// Version of the catalog JSON layout.
pub const CATALOG_VERSION: u32 = 1;

/// Every op in registry order with its metadata, parameter schema, defaults
/// and examples.
///
/// Complexity: O(ops x schema size).
pub fn catalog_json(registry: &Registry) -> Value {
    let ops: Vec<Value> = registry
        .iter()
        .map(|op| {
            let m = op.meta();
            json!({
                "id": op.id(),
                "label": m.label,
                "category": m.category.as_str(),
                "summary": m.summary,
                "docs": m.docs,
                "aliases": m.aliases,
                "complexity": m.complexity,
                "flags": m.flags.names(),
                "version": m.version,
                "examples": m.examples.iter().map(|e| json!({ "args": serde_json::from_str::<Value>(e.args).unwrap_or(Value::Null), "result": e.result })).collect::<Vec<_>>(),
                "params": op.params_schema(),
                "defaults": op.default_params(),
            })
        })
        .collect();
    json!({ "catalog_version": CATALOG_VERSION, "app_version": env!("CARGO_PKG_VERSION"), "ops": ops })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_every_op_in_order() {
        let r = Registry::builtin();
        let c = catalog_json(&r);
        let ids: Vec<&str> = c["ops"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
        let expected: Vec<&str> = r.iter().map(|o| o.id()).collect();
        assert_eq!(ids, expected);
        let first = &c["ops"][0];
        assert!(first["params"]["type"] == "object" && first["examples"][0]["args"].is_object());
    }
}
