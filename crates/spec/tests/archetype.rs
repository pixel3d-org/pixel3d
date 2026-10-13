//! Archetypes (ADR 0016): instances expand into a skeleton and generated
//! parts that compile, written parts layer over generated ones, joint
//! edits move limbs, and mistakes point into `/archetypes`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;

use glam::Vec3;
use p3d_ops::{OpCtx, OpError, Registry, Tx};
use p3d_scene::Document;
use serde_json::{Value, json};

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(p3d_spec::registry)
}

fn run(tx: &mut Tx, op: &str, args: Value) -> Result<String, OpError> {
    let reg = registry();
    let o = reg.get(op)?;
    let applied = o.run(tx, &args, &OpCtx { registry: Some(reg), ..OpCtx::default() })?;
    Ok(applied.output.summary)
}

/// World bounds of every part object under a root.
fn bounds(tx: &Tx, root: &str) -> (Vec3, Vec3) {
    let doc = tx.doc();
    let r = doc.resolve(root).unwrap();
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for &k in doc.object(r).unwrap().children() {
        let (Some(m), Some(w)) = (doc.object_mesh(k), doc.world_affine(k)) else { continue };
        if doc.object(k).is_some_and(|o| o.tags.iter().any(|t| t == "strands")) {
            continue;
        }
        for v in m.verts() {
            let p = w.transform_point3(m.positions[v as usize]);
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    (lo, hi)
}

fn joint(tx: &Tx, root: &str, name: &str) -> Vec3 {
    let r = tx.doc().resolve(root).unwrap();
    p3d_spec::skeleton_joints(tx.doc(), r).unwrap().into_iter().find(|j| j.name == name).unwrap().point
}

#[test]
fn every_race_compiles_to_its_height() {
    for (race, height) in [("human", 1.8f32), ("elf", 1.9), ("dwarf", 1.35), ("orc", 2.0), ("halfling", 1.1)] {
        let mut tx = Tx::new(Document::new());
        let s = run(
            &mut tx,
            "spec.set",
            json!({ "name": "body", "spec": { "archetypes": { "who": { "kind": race, "params": { "beard": "none", "hair": "none" } } } } }),
        )
        .unwrap_or_else(|e| panic!("{race}: {e:?}"));
        assert!(s.contains(&format!("archetype who ({race})")) && s.contains("torso"), "{s}");
        let (lo, hi) = bounds(&tx, "body");
        assert!((hi.y - lo.y - height).abs() < 0.06 * height, "{race}: {lo} {hi}");
        assert!(lo.y.abs() < 0.03 * height, "{race} stands on the ground: {lo}");
        // The head sits on its joint, the hands on the wrists.
        let head = tx.doc().resolve("body/head").unwrap();
        let at = tx.doc().object(head).unwrap().transform.translation;
        assert!((at - joint(&tx, "body", "head")).length() < 1e-4, "{race}");
    }
}

#[test]
fn written_parts_and_joint_edits_change_the_body() {
    let mut tx = Tx::new(Document::new());
    let spec = json!({
        "archetypes": { "dwarf": {
            "kind": "dwarf",
            "params": { "hold": "right", "beard": "none", "hair": "none" },
            "joints": { "elbow_l": { "by": [0, 0.1, 0] } },
            "omit": ["ear_l", "ear_r"]
        } },
        "parts": {
            "head": { "params": { "brow": 1.0 } },
            "helmet": { "primitive": { "uv_sphere": { "radius": 0.15 } }, "anchor": { "on": "dwarf", "landmark": "head_top", "offset": [0, -0.1, 0] } }
        }
    });
    run(&mut tx, "spec.set", json!({ "name": "d", "spec": spec.clone() })).unwrap();
    let parsed = p3d_spec::parse(&spec).unwrap();
    assert_eq!(parsed.parts["head"].params.as_ref().unwrap()["brow"], json!(1.0));
    assert!(parsed.parts["head"].params.as_ref().unwrap()["jaw"].is_number(), "generated params stay");
    assert!(!parsed.parts.contains_key("ear_l") && parsed.parts.contains_key("helmet"));
    // The edit moved the left elbow and the wrist below it, not the right.
    let base = p3d_spec::parse(&json!({ "archetypes": { "dwarf": { "kind": "dwarf", "params": { "hold": "right" } } } })).unwrap();
    let j = |s: &p3d_spec::ModelSpec, n: &str| s.skeletons["dwarf"].joints[n].frame().0;
    assert!((j(&parsed, "wrist_l") - j(&base, "wrist_l") - Vec3::new(0.0, 0.1, 0.0)).length() < 1e-4);
    assert!((j(&parsed, "elbow_r") - j(&base, "elbow_r")).length() < 1e-6);
    // The gripping right fist is in front of the body with its handle upright.
    let grip = &parsed.skeletons["dwarf"].joints["grip_r"];
    let (g, q) = grip.frame();
    assert!(g.z > 0.15 && (q * Vec3::Y).y > 0.9, "{g} {:?}", q * Vec3::Y);
    let helmet = tx.doc().resolve("d/helmet").unwrap();
    assert!((tx.doc().object(helmet).unwrap().transform.translation.y - (joint(&tx, "d", "head_top").y - 0.1)).abs() < 1e-4);
}

#[test]
fn archetype_mistakes_point_into_the_block() {
    let cases = [
        (json!({ "dwarf": { "kind": "dwarff" } }), "/archetypes/dwarf/kind", "dwarf"),
        (json!({ "dwarf": { "kind": "dwarf", "params": { "hieght": 1.4 } } }), "/archetypes/dwarf/params/hieght", "height"),
        (json!({ "dwarf": { "kind": "dwarf", "params": { "heads": 20 } } }), "/archetypes/dwarf/params/heads", ""),
        (json!({ "dwarf": { "kind": "dwarf", "joints": { "elbo_l": { "by": [0, 0, 0] } } } }), "/archetypes/dwarf/joints/elbo_l", "elbow_l"),
        (json!({ "dwarf": { "kind": "dwarf", "omit": ["torsoo"] } }), "/archetypes/dwarf/omit/0", "torso"),
        (json!({ "dwarf": { "kind": "dwarf", "at": "nobody.grip_r" } }), "/archetypes/dwarf/at", ""),
        (json!({ "a": { "kind": "dwarf" }, "b": { "kind": "elf" } }), "/archetypes/b", "prefix"),
        (json!({ "a": { "kind": "dwarf", "at": "b.grip_r" }, "b": { "kind": "elf", "at": "a.grip_l" } }), "/archetypes/a/at", ""),
    ];
    for (block, path, hint) in cases {
        let e = p3d_spec::parse(&json!({ "archetypes": block })).unwrap_err();
        assert_eq!(e.path, path, "{e:?}");
        assert!(e.message.contains(hint), "{e:?}");
    }
    // A prefix lets two instances share part names.
    let s = p3d_spec::parse(
        &json!({ "archetypes": { "a": { "kind": "dwarf" }, "b": { "kind": "elf", "prefix": "elf_", "transform": { "position": [1, 0, 0] } } } }),
    )
    .unwrap();
    assert!(s.parts.contains_key("torso") && s.parts.contains_key("elf_torso"));
    assert_eq!(s.parts["elf_nose"].anchor.as_ref().unwrap().on.as_deref(), Some("elf_head"));
    assert!((s.skeletons["b"].joints["root"].frame().0 - Vec3::X).length() < 1e-6);
}
