//! The example sessions in `docs/agents/examples/` (a fantasy sword, a
//! dwarven battle axe as a Model Spec, an elf head from feature templates,
//! a stylized tree, a broadleaf tree, a dwarf warrior and feathered wings
//! from archetypes) still build: every call succeeds and `diagnose` finds no
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

/// The AAA broadleaf of ADR 0009 from one spec part: it reads like the
/// reference profile in text, passes its crown and budget tests, and its
/// vegetation genes are on target.
#[test]
fn broadleaf_tree() {
    let source = include_str!("../../../docs/agents/examples/broadleaf_tree.json");
    replay(source);
    let session: Value = serde_json::from_str(source).unwrap();
    let mut agent = Agent::new(Session::new());
    let mut outs = Vec::new();
    for call in session["calls"].as_array().unwrap() {
        outs.push(agent.call(call["tool"].as_str().unwrap(), call["args"].clone()).text);
    }
    let veg = &outs[1];
    assert!(veg.contains("trunk: fork at") && veg.contains("coverage front") && veg.contains("wind data: level"), "{veg}");
    assert!(outs[2].contains("tests 2/2 pass"), "{}", outs[2]);
    assert!(outs[3].contains("crown width / height") && !outs[3].contains("LOW") && !outs[3].contains("HIGH"), "{}", outs[3]);
}

/// Replays a session and returns every answer.
fn answers(source: &str) -> Vec<String> {
    let session: Value = serde_json::from_str(source).unwrap();
    let mut agent = Agent::new(Session::new());
    let mut out = Vec::new();
    for (i, call) in session["calls"].as_array().unwrap().iter().enumerate() {
        let tool = call["tool"].as_str().unwrap();
        let o = agent.call(tool, call["args"].clone());
        assert!(!o.is_error, "call {i} ({tool}) failed:\n{}", o.text);
        out.push(o.text);
    }
    let d = agent.call("diagnose", json!({}));
    assert!(d.text.contains(" 0 errors"), "{}", d.text);
    out
}

/// A dwarf warrior from archetypes in five calls: the dwarf body with a
/// braided beard, an axe in its right fist through the weapon's grip
/// joint, gear on joints, rune and stud ornament; the brief passes, the
/// dwarf is measured against dwarf ranges, and the critique's fix applies.
#[test]
fn dwarf_warrior() {
    let a = answers(include_str!("../../../docs/agents/examples/dwarf_warrior.json"));
    assert!(a.len() <= 6 && a[0].contains("dwarf (Dwarf)"), "{}", a[0]);
    let spec = &a[2];
    assert!(spec.contains("archetype dwarf (dwarf)") && spec.contains("archetype axe (axe)") && spec.contains("brief dwarf 3/3"), "{spec}");
    let dna = &a[3];
    assert!(dna.contains("heads 4.") && dna.contains("shoulders") && dna.contains("tangent bracer_l"), "{dna}");
    let fixed = &a[4];
    assert!(fixed.contains("0 built") && !fixed.contains("critique:"), "{fixed}");
}

/// Feathered wings on an elf's back joint with an elven sword in hand.
#[test]
fn feathered_wings() {
    let a = answers(include_str!("../../../docs/agents/examples/feathered_wings.json"));
    assert!(a[0].contains("feathered_wings") && a[0].contains("tilt"), "{}", a[0]);
    assert!(a[1].contains("archetype wings (feathered_wings)") && a[1].contains("wing_l, wing_r"), "{}", a[1]);
    assert!(a[2].contains("dna (elf, elven)") && a[2].contains("critique: none"), "{}", a[2]);
}
