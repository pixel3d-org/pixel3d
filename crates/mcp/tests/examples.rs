//! The example sessions in `docs/agents/examples/` (a fantasy sword, a
//! stylized tree) still build: every call succeeds and `diagnose` finds no
//! errors. Warnings are allowed (a blade's edge is thinner than 1 mm by
//! design, gems sit in their bezels).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use p3d_engine::Session;
use p3d_mcp::Agent;
use serde_json::{Value, json};

fn replay(source: &str) {
    let session: Value = serde_json::from_str(source).unwrap();
    let mut agent = Agent::new(Session::new());
    for (i, call) in session["calls"].as_array().unwrap().iter().enumerate() {
        let tool = call["tool"].as_str().unwrap();
        let out = agent.call(tool, call["args"].clone());
        assert!(!out.is_error, "call {i} ({tool}) failed:\n{}", out.text);
    }
    let d = agent.call("diagnose", json!({}));
    assert!(d.text.contains(" 0 errors"), "{}", d.text);
}

#[test]
fn fantasy_sword() {
    replay(include_str!("../../../docs/agents/examples/fantasy_sword.json"));
}

#[test]
fn stylized_tree() {
    replay(include_str!("../../../docs/agents/examples/stylized_tree.json"));
}
