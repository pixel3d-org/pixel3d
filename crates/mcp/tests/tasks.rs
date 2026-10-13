//! Benchmark tasks T1, T2, T4 and T5 (project spec 23.2) modelled through
//! the agent tools: each reference session in `bench/agent-eval/reference/`
//! is replayed call by call, as an agent would send it. Every call must
//! succeed, every expectation must pass at the end, and the meshes must be
//! closed with no errors from `diagnose`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use p3d_engine::Session;
use p3d_mcp::Agent;
use serde_json::{Value, json};

/// Replays a reference session in its asset class (detail is judged at the
/// class's view distance) and returns the agent and the number of calls.
fn replay(source: &str) -> (Agent, usize) {
    let session: Value = serde_json::from_str(source).unwrap();
    let mut s = Session::new();
    if let Some(class) = session.get("asset_class") {
        let mut doc = p3d_scene::Document::new();
        doc.settings.asset_class = serde_json::from_value(class.clone()).unwrap();
        s.load(doc);
    }
    let mut agent = Agent::new(s);
    let calls = session["calls"].as_array().unwrap();
    for (i, call) in calls.iter().enumerate() {
        let tool = call["tool"].as_str().unwrap();
        let out = agent.call(tool, call["args"].clone());
        assert!(!out.is_error, "{} call {i} ({tool}) failed:\n{}", session["task"], out.text);
    }
    (agent, calls.len())
}

/// Every expectation passes and `diagnose` finds no errors or warnings.
fn verify(agent: &mut Agent, task: &str) {
    let tests = agent.call("expect", json!({ "action": "check" }));
    assert!(!tests.is_error, "{task}: {}", tests.text);
    assert!(tests.text.contains("pass") && !tests.text.contains("failing"), "{task}: {}", tests.text);
    let d = agent.call("diagnose", json!({}));
    assert!(!d.is_error, "{task}: {}", d.text);
    assert!(d.text.contains(" 0 errors, 0 warnings"), "{task} diagnose:\n{}", d.text);
    for (_, o) in agent.session().doc().objects() {
        if let Some(m) = o.mesh_key().and_then(|k| agent.session().doc().mesh(k)) {
            assert_eq!(m.validate(), Ok(()), "{task}: {}", o.name());
            assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "{task}: {} has open edges", o.name());
        }
    }
}

#[test]
fn t1_barrel_with_hoops() {
    let (mut agent, calls) = replay(include_str!("../../../bench/agent-eval/reference/t1_barrel.json"));
    verify(&mut agent, "T1");
    assert!(calls <= 4);
    let m = agent.session().doc().object_mesh(agent.session().doc().resolve("barrel").unwrap()).unwrap();
    for i in 1..=4 {
        assert!(m.group(&format!("hoop_{i}.cap")).is_some_and(|g| g.count() == 32), "hoop_{i}");
    }
}

#[test]
fn t2_scifi_crate_with_panels_and_handles() {
    let (mut agent, calls) = replay(include_str!("../../../bench/agent-eval/reference/t2_scifi_crate.json"));
    verify(&mut agent, "T2");
    assert!(calls <= 4);
    let m = agent.session().doc().object_mesh(agent.session().doc().resolve("crate").unwrap()).unwrap();
    for g in ["handles.cut", "vents.cut", "panel_front.inner", "plate_front.inner", "edges.faces"] {
        assert!(m.group(g).is_some_and(|g| g.count() > 0), "{g}");
    }
}

#[test]
fn t4_modular_wall_with_doorway() {
    let (mut agent, calls) = replay(include_str!("../../../bench/agent-eval/reference/t4_modular_wall.json"));
    verify(&mut agent, "T4");
    assert!(calls <= 4);
    let doc = agent.session().doc();
    let k = doc.resolve("wall").unwrap();
    let m = doc.object_mesh(k).unwrap();
    // The pivot is the wall's corner, and the doorway is open: nothing inside it.
    let (lo, _) = m.bounds().unwrap();
    assert!(lo.x.abs() < 1e-4 && lo.y.abs() < 1e-4, "{lo:?}");
    let inside = |p: glam::Vec3| p.x > 1.46 && p.x < 2.54 && p.y > 0.13 && p.y < 2.14 && p.z > -0.05 && p.z < 0.3;
    assert!(m.verts().all(|v| !inside(m.positions[v as usize])));
    assert!(m.faces().all(|f| !inside(m.face_verts(f).map(|v| m.positions[v as usize]).sum::<glam::Vec3>() / m.face_len(f) as f32)));
}

#[test]
fn t5_pistol_hard_surface() {
    let (mut agent, calls) = replay(include_str!("../../../bench/agent-eval/reference/t5_pistol.json"));
    verify(&mut agent, "T5");
    assert!(calls <= 4);
}
