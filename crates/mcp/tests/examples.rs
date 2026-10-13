//! The example sessions in `docs/agents/examples/` (a fantasy sword, a
//! dwarven battle axe as a Model Spec, an elf head from feature templates,
//! a stylized tree) still build: every call succeeds and `diagnose` finds no
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
fn sdf_head() {
    replay(include_str!("../../../docs/agents/examples/sdf_head.json"));
}

#[test]
fn stylized_tree() {
    replay(include_str!("../../../docs/agents/examples/stylized_tree.json"));
}

#[test]
fn feature_head() {
    replay(include_str!("../../../docs/agents/examples/feature_head.json"));
}

#[test]
fn battle_axe() {
    replay(include_str!("../../../docs/agents/examples/battle_axe.json"));
}

/// Acceptance T11: an elf head with ears, hair cards and eyebrows in at
/// most 6 calls and no image calls, answering its brief in full.
#[test]
fn t11_elf_head_with_hair_answers_its_brief() {
    let session: Value = serde_json::from_str(include_str!("../../../docs/agents/examples/feature_head.json")).unwrap();
    let calls = session["calls"].as_array().unwrap();
    assert!(calls.len() <= 6 && calls.iter().all(|c| c["tool"] != "render"));
    let mut agent = Agent::new(Session::new());
    let mut last = String::new();
    for call in calls {
        let out = agent.call(call["tool"].as_str().unwrap(), call["args"].clone());
        assert!(!out.is_error, "{}", out.text);
        last = out.text;
    }
    assert!(last.contains("brief elf 8/8"), "{last}");
    let d = agent.call("diagnose", json!({})).text;
    for code in ["OPEN_BOUNDARY", "FLOATING_PART", "SELF_INTERSECTION"] {
        assert!(!d.contains(code), "{code} on strands: {d}");
    }
}
