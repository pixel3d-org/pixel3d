//! DNA critique on specs: archetype genes set targets, each critique comes
//! with a spec patch, and predictions say whether the patch resolves it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;

use p3d_ops::{OpCtx, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(p3d_spec::registry)
}

/// The dna report of a spec built from `spec`, as text and fixes.
fn critique(spec: Value) -> (String, Vec<Value>) {
    let reg = registry();
    let mut tx = Tx::new(Document::new());
    reg.get("spec.set").unwrap().run(&mut tx, &json!({ "name": "m", "spec": spec }), &OpCtx { registry: Some(reg), ..OpCtx::default() }).unwrap();
    let root = tx.doc().resolve("m").unwrap();
    let dna = p3d_spec::dna_of(tx.doc(), root).unwrap_or_default();
    let r = p3d_spec::dna::report(tx.doc(), root, &dna, reg, 0).unwrap();
    let fixes = r.data["fixes"].as_array().cloned().unwrap_or_default();
    (r.text, fixes)
}

#[test]
fn archetypes_bring_their_genes_and_proportion_fixes() {
    // The default dwarf is on target.
    let (text, fixes) = critique(json!({ "archetypes": { "d": { "kind": "dwarf", "params": { "beard": "none", "hair": "none" } } } }));
    assert!(text.contains("dna (dwarf, dwarven)") && text.contains("heads 4.") && text.contains("critique: none"), "{text}");
    assert!(fixes.is_empty());
    // Six heads tall is no dwarf: the fix changes the archetype's heads.
    let (text, fixes) = critique(json!({ "archetypes": { "d": { "kind": "dwarf", "params": { "heads": 6.0, "beard": "none", "hair": "none" } } } }));
    assert!(text.contains("proportion: heads"), "{text}");
    let f = fixes.iter().find(|f| f["subject"] == "heads").unwrap();
    assert!(f["patch"]["merge"]["archetypes"]["d"]["params"]["heads"].as_f64().unwrap() < 5.0, "{f}");
    assert!(f["predicted"].as_str().unwrap().starts_with("resolved"), "{f}");
}

#[test]
fn stiff_blades_bend_through_their_archetype() {
    let (text, fixes) = critique(json!({ "archetypes": { "s": { "kind": "sword", "params": { "style": "elven" } } } }));
    assert!(text.contains("stiff: blade"), "{text}");
    let f = fixes.iter().find(|f| f["code"] == "stiff").unwrap();
    assert!(f["patch"]["merge"]["archetypes"]["s"]["params"]["curve"].is_number(), "{f}");
    assert!(f["predicted"].as_str().unwrap().starts_with("resolved"), "{f}");
    // A curved elven blade passes.
    let (text, _) = critique(json!({ "archetypes": { "s": { "kind": "sword", "params": { "style": "elven", "curve": 12 } } } }));
    assert!(!text.contains("stiff"), "{text}");
}

#[test]
fn floating_detail_and_twins_get_part_patches() {
    let spec = json!({
        "dna": { "culture": "human_medieval" },
        "parts": {
            "plate": { "primitive": { "box": { "width": 1.0, "height": 0.1, "depth": 0.4 } } },
            "rivet": { "primitive": { "box": { "width": 0.03, "height": 0.03, "depth": 0.03 } }, "transform": { "position": [0, 0.2, 0] } },
            "post_a": { "primitive": { "box": { "width": 0.1, "height": 0.6, "depth": 0.1 } }, "anchor": { "on": "plate", "landmark": "top", "offset": [0.2, 0, 0] } },
            "post_b": { "primitive": { "box": { "width": 0.1, "height": 0.6, "depth": 0.1 } }, "anchor": { "on": "plate", "landmark": "top", "offset": [0.35, 0, 0] } }
        }
    });
    let (text, fixes) = critique(spec);
    assert!(text.contains("floating_detail: rivet") && text.contains("twinning"), "{text}");
    let f = fixes.iter().find(|f| f["code"] == "floating_detail").unwrap();
    let y = f["patch"]["merge"]["parts"]["rivet"]["transform"]["position"][1].as_f64().unwrap();
    assert!(y < 0.12, "the rivet moves down onto the plate: {f}");
    assert!(f["predicted"].as_str().unwrap().starts_with("resolved"), "{f}");
    let t = fixes.iter().find(|f| f["code"] == "twinning").unwrap();
    assert_eq!(t["patch"]["merge"]["parts"]["post_b"]["transform"]["scale"], json!(0.75), "{t}");
}
