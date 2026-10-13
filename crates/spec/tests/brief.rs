//! Briefs: expansion into checks, coverage before and after a spec answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;

use p3d_ops::{OpCtx, Registry, Tx};
use p3d_scene::Document;
use p3d_spec::brief::Status;
use serde_json::{Value, json};

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(p3d_spec::registry)
}

fn ok(tx: &mut Tx, op: &str, args: Value) -> String {
    let reg = registry();
    let applied = reg.get(op).unwrap().run(tx, &args, &OpCtx { registry: Some(reg), ..OpCtx::default() }).unwrap_or_else(|e| panic!("{op}: {e:?}"));
    applied.output.summary
}

fn brief() -> Value {
    json!({
        "title": "Elf ranger head",
        "category": "humanoid_head",
        "style": "stylized",
        "size": { "height": "0.23m" },
        "budget": { "tris": [2000, 40000] },
        "must_have": [{ "feature": "ear", "count": 2, "traits": { "length_ratio_to_head": [0.35, 0.6], "pointiness": "high" } }],
        "must_not": ["visible_seams_on_face", "eye_l_penetrating_eye_r"]
    })
}

fn head(ear_tip: [f64; 3]) -> Value {
    json!({
        "brief": "elf",
        "category": "humanoid_head",
        "parts": {
            "head": {
                "sdf": [
                    { "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" },
                    { "ellipsoid": { "at": [0, 0.06, 0.035], "radii": [0.055, 0.045, 0.055] }, "name": "jaw", "smooth_union": "2.5cm" }
                ],
                "detail": "5mm",
                "landmarks": "category"
            },
            "ear_l": {
                "sdf": [{ "capsule": { "a": [0, 0, 0], "b": ear_tip, "radius": "1cm", "radius_b": "2mm" } }],
                "anchor": { "on": "head", "landmark": "ear_root_l" },
                "join": "blend",
                "blend": "8mm"
            },
            "ear_r": { "mirror": "ear_l" },
            "eye_l": { "primitive": { "uv_sphere": { "radius": 0.012, "origin": "center" } }, "anchor": { "on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008] } },
            "eye_r": { "mirror": "eye_l" }
        }
    })
}

#[test]
fn a_brief_expands_into_checks_and_waits_for_its_spec() {
    let mut tx = Tx::new(Document::new());
    let s = ok(&mut tx, "brief.set", json!({ "name": "elf", "brief": brief() }));
    assert!(s.starts_with("brief elf: "), "{s}");
    let c = p3d_spec::coverage(tx.doc(), "elf").unwrap();
    assert!(c.spec.is_none());
    assert!(c.items.iter().all(|i| i.status == Status::Pending), "{c:?}");
    assert!(c.line().ends_with("no spec answers it yet"), "{}", c.line());
}

#[test]
fn coverage_measures_the_spec_that_answers_the_brief() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "brief.set", json!({ "name": "elf", "brief": brief() }));
    // Short ears first: the ear length check fails, as in the elf session.
    ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": head([0.03, 0.04, -0.01]) }));
    let c = p3d_spec::coverage(tx.doc(), "elf").unwrap();
    let get = |c: &p3d_spec::Coverage, label: &str| {
        c.items.iter().find(|i| i.label.starts_with(label)).cloned().unwrap_or_else(|| panic!("{label} in {c:?}"))
    };
    for label in ["height", "budget", "symmetric", "head closed", "parts attached", "width_to_height", "eye_line", "2 ear"] {
        assert_eq!(get(&c, label).status, Status::Pass, "{label}: {:?}", get(&c, label));
    }
    assert_eq!(get(&c, "ear length").status, Status::Fail, "{c:?}");
    assert_eq!(get(&c, "ear pointiness").status, Status::Pending);
    assert_eq!(get(&c, "no visible seams").status, Status::Pending);
    assert_eq!(get(&c, "no eye l penetrating eye r").status, Status::Pass);
    assert!(c.line().contains("failing: ear length"), "{}", c.line());
    // Longer ears fix it.
    ok(
        &mut tx,
        "spec.patch",
        json!({ "target": "elf_head", "merge": { "parts": { "ear_l": { "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0.05, 0.075, -0.02], "radius": "1cm", "radius_b": "2mm" } }] } } } }),
    );
    let c = p3d_spec::coverage(tx.doc(), "elf").unwrap();
    assert_eq!(get(&c, "ear length").status, Status::Pass, "{c:?}");
}
