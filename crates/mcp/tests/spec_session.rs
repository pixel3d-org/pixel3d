//! The elf head session (project spec 17.20, steps 1 and 2, then a fix)
//! through the `brief` and `spec` tools: every answer carries the brief
//! coverage, and the coverage follows the spec.

#![allow(clippy::unwrap_used, clippy::panic)]

use p3d_engine::Session;
use p3d_mcp::Agent;
use serde_json::{Value, json};

fn call(agent: &mut Agent, tool: &str, args: Value) -> String {
    let out = agent.call(tool, args);
    assert!(!out.is_error, "{tool}: {}", out.text);
    out.text
}

fn ear(tip: [f64; 3]) -> Value {
    json!({
        "sdf": [{ "capsule": { "a": [0, 0, 0], "b": tip, "radius": "1cm", "radius_b": "2mm" }, "name": "helix" }],
        "anchor": { "on": "head", "landmark": "ear_root_l" },
        "join": "blend",
        "blend": "8mm"
    })
}

#[test]
fn elf_head_from_brief_to_passing_spec() {
    let mut a = Agent::new(Session::new());
    let t = call(
        &mut a,
        "brief",
        json!({ "action": "set", "name": "elf", "brief": {
            "title": "Elf ranger head", "category": "humanoid_head", "style": "stylized",
            "size": { "height": "0.23m" }, "budget": { "tris": [2000, 40000] },
            "must_have": [{ "feature": "ear", "count": 2, "traits": { "length_ratio_to_head": [0.35, 0.6] } }]
        } }),
    );
    assert!(t.contains("brief elf: 10 checks") && t.contains("no spec answers it yet"), "{t}");

    let t = call(
        &mut a,
        "spec",
        json!({ "action": "set", "target": "elf_head", "spec": {
            "brief": "elf", "category": "humanoid_head",
            "parts": {
                "head": {
                    "sdf": [
                        { "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" },
                        { "ellipsoid": { "at": [0, 0.06, 0.035], "radii": [0.055, 0.045, 0.055] }, "name": "jaw", "smooth_union": "2.5cm" }
                    ],
                    "detail": "5mm", "landmarks": "category"
                },
                "ear_l": ear([0.03, 0.04, -0.01]),
                "ear_r": { "mirror": "ear_l" },
                "eye_l": { "primitive": { "uv_sphere": { "radius": 0.012, "origin": "center" } }, "anchor": { "on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008] } },
                "eye_r": { "mirror": "eye_l" }
            }
        } }),
    );
    assert!(t.starts_with("ok rev 2"), "{t}");
    assert!(t.contains("brief elf 9/10; failing: ear length"), "{t}");

    let t = call(&mut a, "spec", json!({ "action": "patch", "target": "elf_head", "merge": { "parts": { "ear_l": ear([0.05, 0.075, -0.02]) } } }));
    assert!(t.contains("1 built, 2 unchanged"), "only the head rebuilds: {t}");
    assert!(t.contains("brief elf 10/10"), "{t}");

    let t = call(&mut a, "brief", json!({ "name": "elf" }));
    assert!(t.lines().skip(1).all(|l| l.trim_start().starts_with("ok ")), "{t}");
    let t = call(&mut a, "spec", json!({ "target": "elf_head" }));
    assert!(t.contains("ear_l: blended into its parent") && t.contains("eye_l: o:"), "{t}");
}
