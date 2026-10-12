//! Scripted agent sessions through every tool; each answer is snapshotted
//! (text and structured JSON).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic)]

use p3d_engine::Session;
use p3d_mcp::{Agent, ToolOutput, tools};
use serde_json::{Value, json};

/// Calls a tool and records the exchange.
fn call(agent: &mut Agent, log: &mut String, tool: &str, args: Value) -> ToolOutput {
    let out = agent.call(tool, args.clone());
    // Default answers stay small (project spec 17.13); reference texts are on demand.
    if !matches!(tool, "describe_op" | "docs") {
        let tokens = p3d_perceive::fmt::approx_tokens(&out.text);
        assert!(tokens <= 800, "{tool}: {tokens} tokens\n{}", out.text);
    }
    log.push_str(&format!(">>> {tool} {args}\n{}{}\n", if out.is_error { "[error] " } else { "" }, scrub(&out.text)));
    out
}

#[test]
fn tools_have_strict_schemas_and_descriptions() {
    for t in tools() {
        assert!(t.description.ends_with('.'), "{}", t.name);
        assert_eq!(t.schema["additionalProperties"], json!(false), "{}: schema must be strict", t.name);
        assert_eq!(t.schema["type"], json!("object"), "{}", t.name);
    }
}

#[test]
fn beveled_crate_in_three_calls() {
    let mut a = Agent::new(Session::new());
    let mut log = String::new();
    let r = call(
        &mut a,
        &mut log,
        "run",
        json!({ "ops": [
            { "op": "prim.box", "args": { "name": "crate", "width": "600mm", "height": "600mm", "depth": "600mm", "bevel": { "width": "8mm", "segments": 2 } } },
            { "op": "expect.add", "args": { "expect": { "dims": { "target": "crate", "height": { "min": "590mm", "max": "610mm" } } } } },
            { "op": "expect.add", "args": { "expect": { "watertight": { "target": "crate" } } } }
        ] }),
    );
    assert!(!r.is_error, "{}", r.text);
    assert!(r.text.contains("tests 2/2 pass"), "{}", r.text);
    let d = call(&mut a, &mut log, "describe", json!({ "target": "crate" }));
    assert!(d.text.contains("box 100%") && d.text.contains("600 x 600 x 600 mm"), "{}", d.text);
    let p = call(&mut a, &mut log, "diagnose", json!({}));
    assert!(p.text.contains("no problems found"), "{}", p.text);
    insta::assert_snapshot!("beveled_crate", log);
}

#[test]
fn every_tool_answers() {
    let mut a = Agent::new(Session::new());
    let mut log = String::new();
    call(&mut a, &mut log, "scene_overview", json!({}));
    call(&mut a, &mut log, "list_ops", json!({ "search": "cube" }));
    call(&mut a, &mut log, "describe_op", json!({ "op": "cube" }));
    call(
        &mut a,
        &mut log,
        "run",
        json!({ "ops": [
            { "op": "prim.box", "args": { "name": "crate", "width": 0.6, "height": 0.6, "depth": 0.6 } },
            { "op": "prim.box", "args": { "name": "lid", "parent": "crate", "width": 0.62, "height": 0.04, "depth": 0.62, "position": [0, 0.612, 0] } },
            { "op": "prim.box", "args": { "name": "handle_l", "parent": "crate", "width": 0.03, "height": 0.04, "depth": 0.16, "position": [-0.315, 0.4, 0] } }
        ], "checkpoint": "blockout" }),
    );
    call(&mut a, &mut log, "scene_overview", json!({ "detail": "full" }));
    call(&mut a, &mut log, "expect", json!({ "action": "add", "expect": { "attached": { "part": "lid", "to": "crate" } } }));
    call(&mut a, &mut log, "perceive", json!({ "aspect": "relations" }));
    call(&mut a, &mut log, "perceive", json!({ "aspect": "silhouette", "view": "front", "points": 16 }));
    call(&mut a, &mut log, "perceive", json!({ "aspect": "section", "axis": "y", "at": "400mm" }));
    call(&mut a, &mut log, "perceive", json!({ "aspect": "probe", "point": { "on": "crate", "landmark": "top" } }));
    call(&mut a, &mut log, "perceive", json!({ "target": "lid", "aspect": "shape" }));
    let d = call(&mut a, &mut log, "diagnose", json!({}));
    assert!(d.text.contains("FLOATING_PART lid"), "{}", d.text);
    call(&mut a, &mut log, "diagnose", json!({ "explain": "F1" }));
    let f = call(&mut a, &mut log, "diagnose", json!({ "fix": "F1" }));
    assert!(f.text.contains("tests 1/1 pass"), "{}", f.text);
    call(&mut a, &mut log, "perceive", json!({ "aspect": "diff" }));
    call(&mut a, &mut log, "history", json!({ "action": "diff", "from": "blockout" }));
    call(
        &mut a,
        &mut log,
        "query",
        json!({ "target": "crate", "select": { "faces": { "normal": { "dir": [0, 1, 0], "within_deg": 5 } } }, "save_as": "up" }),
    );
    call(&mut a, &mut log, "analyze", json!({}));
    call(&mut a, &mut log, "expect", json!({ "action": "check" }));
    call(&mut a, &mut log, "history", json!({}));
    call(&mut a, &mut log, "history", json!({ "action": "undo", "steps": 2 }));
    call(&mut a, &mut log, "history", json!({ "action": "revert", "name": "blockout" }));
    call(&mut a, &mut log, "docs", json!({}));
    insta::assert_snapshot!("every_tool", log);
}

#[test]
fn errors_point_at_the_bad_argument() {
    let mut a = Agent::new(Session::new());
    let mut log = String::new();
    for (tool, args) in [
        ("run", json!({ "ops": [{ "op": "prim.bxo" }] })),
        ("run", json!({ "ops": [{ "op": "prim.box", "args": { "widht": 1 } }] })),
        ("run", json!({ "ops": [{ "op": "prim.box", "args": { "width": 1, "bevel": { "width": 0.6 } } }] })),
        ("run", json!({ "opps": [] })),
        ("describe", json!({ "target": "nothing" })),
        ("perceive", json!({ "aspect": "probe" })),
        ("perceive", json!({ "aspect": "silhouete" })),
        ("diagnose", json!({ "fix": "F7" })),
        ("history", json!({ "action": "undo" })),
        ("history", json!({ "action": "diff", "from": "nowhere" })),
        ("expect", json!({ "action": "remove", "name": "E1" })),
        ("docs", json!({ "topic": "quikstart" })),
        ("no_such_tool", json!({})),
    ] {
        let out = call(&mut a, &mut log, tool, args);
        assert!(out.is_error, "{tool}: {}", out.text);
        assert!(out.structured["error"]["code"].is_string(), "{tool}");
    }
    insta::assert_snapshot!("errors", log);
}

/// Masks timings (`(3 ops, 0.9 ms)` becomes `(3 ops, _ ms)`) so snapshots are stable.
fn scrub(text: &str) -> String {
    text.lines()
        .map(|l| match (l.find(", "), l.find(" ms)")) {
            (Some(a), Some(b)) if (l.starts_with("ok rev") || l.starts_with("dry run")) && a < b => format!("{}, _{}", &l[..a], &l[b..]),
            _ => l.to_owned(),
        })
        .map(|l| l + "\n")
        .collect()
}
