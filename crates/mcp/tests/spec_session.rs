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
            "title": "Elf ranger head", "category": "humanoid_head", "style": "stylized", "asset_class": "character",
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
    let t = call(&mut a, "perceive", json!({ "target": "elf_head", "aspect": "proportions" }));
    assert!(t.contains("width_to_height") && t.contains("ok") && !t.contains("OUT OF RANGE"), "{t}");
    let t = call(&mut a, "perceive", json!({ "target": "elf_head", "aspect": "brief" }));
    assert!(t.starts_with("brief elf 10/10"), "{t}");
    let t = call(&mut a, "perceive", json!({ "target": "elf_head", "aspect": "visibility" }));
    assert!(t.starts_with("visibility at 5 m on 1080p (character)"), "{t}");
    let t = call(&mut a, "spec", json!({ "target": "elf_head" }));
    assert!(t.contains("ear_l: blended into its parent") && t.contains("eye_l: o:"), "{t}");
    // Sculpt by numbers: lift the left ear inside the head mesh, crown pinned.
    let t = call(
        &mut a,
        "deform",
        json!({ "target": "elf_head/head", "handles": [{ "move": "ear_l.helix", "by": [0, 0.008, 0] }], "pins": ["crown"], "radius": "4cm" }),
    );
    assert!(t.contains("1 built"), "{t}");
    let t = call(&mut a, "spec", json!({ "target": "elf_head" }));
    assert!(t.contains("\"deform\":{\"handles\":[{\"by\":"), "kept in the spec: {t}");
    // A plain mesh takes world coordinates.
    call(
        &mut a,
        "run",
        json!({ "ops": [{ "op": "prim.cylinder", "args": { "name": "post", "radius": 0.05, "height": 1, "advanced": { "height_segments": 10 } } }] }),
    );
    let t = call(&mut a, "deform", json!({ "target": "post", "handles": [{ "move": [0, 1, 0], "by": [0.1, 0, 0] }], "pins": [[0, 0, 0]] }));
    assert!(t.contains("1 handle moved"), "{t}");
}

#[test]
fn a_spec_with_dna_reports_its_genes() {
    let mut a = Agent::new(Session::new());
    let axe = json!({
        "dna": { "culture": "dwarven", "style": "stylized_painterly", "proportions": { "head_to_haft": [0.3, 0.5] } },
        "parts": {
            "haft": { "primitive": { "cylinder": { "radius": 0.02, "height": 0.8 } } },
            "head": {
                "primitive": { "box": { "width": 0.3, "height": 0.16, "depth": 0.05, "bevel": { "width": "6mm", "segments": 1 }, "origin": "center" } },
                "anchor": { "on": "haft", "landmark": "top", "offset": [0.05, -0.08, 0] }
            },
            "pommel": {
                "primitive": { "box": { "width": 0.05, "height": 0.05, "depth": 0.05, "bevel": { "width": "6mm", "segments": 1 }, "origin": "center" } },
                "anchor": { "on": "haft", "landmark": "bottom" }
            }
        }
    });
    let t = call(&mut a, "spec", json!({ "action": "set", "target": "axe", "spec": axe }));
    assert!(t.contains("\ndna ") && t.contains("genes on target"), "{t}");
    let t = call(&mut a, "perceive", json!({ "target": "axe", "aspect": "dna" }));
    assert!(t.starts_with("dna (dwarven, stylized_painterly): "), "{t}");
    for gene in ["hierarchy", "lines:", "taper:", "silhouette:", "edges:", "head_to_haft"] {
        assert!(t.contains(gene), "{gene} in {t}");
    }
    // Bad genes point at their path.
    let out = a.call("spec", json!({ "action": "patch", "target": "axe", "merge": { "dna": { "culture": "gnomish" } } }));
    assert!(out.is_error && out.text.contains("/spec/dna/culture"), "{}", out.text);
}

#[test]
fn bisect_finds_the_patch_that_broke_the_brief() {
    let mut a = Agent::new(Session::new());
    call(
        &mut a,
        "brief",
        json!({ "action": "set", "name": "elf", "brief": {
            "category": "humanoid_head", "style": "stylized",
            "must_have": [{ "feature": "ear", "count": 2, "traits": { "length_ratio_to_head": [0.35, 0.6] } }]
        } }),
    );
    let spec = json!({
        "brief": "elf", "category": "humanoid_head",
        "parts": {
            "head": { "sdf": [{ "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" }], "detail": "6mm", "landmarks": "category" },
            "ear_l": ear([0.05, 0.075, -0.02]),
            "ear_r": { "mirror": "ear_l" },
            "eye_l": { "primitive": { "uv_sphere": { "radius": 0.012, "origin": "center" } }, "anchor": { "on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008] } }
        }
    });
    call(&mut a, "spec", json!({ "action": "set", "target": "elf_head", "spec": spec }));
    let eye = |r: f64| json!({ "action": "patch", "target": "elf_head", "merge": { "parts": { "eye_l": { "primitive": { "uv_sphere": { "radius": r, "origin": "center" } } } } } });
    call(&mut a, "spec", eye(0.013));
    call(&mut a, "spec", json!({ "action": "patch", "target": "elf_head", "merge": { "parts": { "ear_l": ear([0.02, 0.03, -0.01]) } } }));
    call(&mut a, "spec", eye(0.011));
    call(&mut a, "spec", eye(0.012));
    let t = call(&mut a, "history", json!({ "action": "bisect", "test": { "brief": { "name": "elf", "item": "ear length" } } }));
    assert!(t.contains("first fails at rev 4") && t.contains("passes at rev 3"), "{t}");
    assert!(t.contains("/parts/ear_l/sdf/0/capsule/b/0: 0.05 -> 0.02"), "{t}");
    let t = call(&mut a, "history", json!({ "action": "bisect", "test": { "finding": { "code": "FLOATING_PART" } } }));
    assert!(t.contains("passes now"), "{t}");
}
