//! Skeletons (ADR 0016): joints as landmarks for anchors and constraints,
//! joint frames with `orient: "normal"`, and `into` for parts placed on
//! joints that blend into another part.

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

fn object_at(tx: &Tx, path: &str) -> (Vec3, glam::Quat) {
    let k = tx.doc().resolve(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let t = tx.doc().object(k).unwrap().transform;
    (t.translation, t.rotation)
}

fn rig() -> Value {
    json!({
        "skeletons": { "rig": {
            "joints": {
                "hip": { "at": [0, 1, 0] },
                "knee": { "at": [0.1, 0.5, 0], "axis": [0, -1, 0], "side": [1, 0, 0], "parent": "hip" },
                "hand": { "at": [0.4, 1.2, 0.2], "axis": [1, 0, 0], "side": [0, 0, -1] }
            },
            "curves": { "leg": ["hip", "knee"] }
        } },
        "parts": {
            "body": { "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": 0.2 } }], "anchor": { "on": "rig", "landmark": "hip" }, "detail": "2cm" },
            "shin": {
                "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0, -0.4, 0], "radius": 0.05 } }],
                "anchor": { "on": "rig", "landmark": "knee" },
                "join": "blend",
                "into": "body"
            },
            "stick": {
                "primitive": { "cylinder": { "radius": 0.01, "height": 0.3 } },
                "anchor": { "on": "rig", "landmark": "hand", "orient": "normal", "offset": [0, 0.1, 0] }
            }
        }
    })
}

#[test]
fn parts_sit_on_joints_and_take_their_frames() {
    let mut tx = Tx::new(Document::new());
    let s = run(&mut tx, "spec.set", json!({ "name": "fig", "spec": rig() })).unwrap();
    assert!(s.contains("1 blended"), "{s}");
    let (body, _) = object_at(&tx, "fig/body");
    assert!((body - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5, "{body}");
    // The stick's +Y follows the joint's axis (+X) and the offset runs along it.
    let (stick, rot) = object_at(&tx, "fig/stick");
    assert!((stick - Vec3::new(0.5, 1.2, 0.2)).length() < 1e-5, "{stick}");
    assert!((rot * Vec3::Y - Vec3::X).length() < 1e-5 && (rot * Vec3::X - Vec3::NEG_Z).length() < 1e-5, "{rot}");
    // The blended shin reaches down to y = 0.1 in the body's mesh.
    let k = tx.doc().resolve("fig/body").unwrap();
    let (lo, _) = tx.doc().object_mesh(k).unwrap().bounds().unwrap();
    assert!((lo.y + 1.0 - 0.05).abs() < 0.03, "{lo}");
    let joints = p3d_spec::skeleton_joints(tx.doc(), tx.doc().resolve("fig").unwrap()).unwrap();
    assert_eq!(joints.len(), 3);
    assert!(joints.iter().any(|j| j.name == "knee" && j.parent.as_deref() == Some("hip") && (j.axis - Vec3::NEG_Y).length() < 1e-5));
}

#[test]
fn constraints_and_named_landmarks_reach_joints() {
    let mut spec = rig();
    spec["constraints"] = json!([{ "offset": { "part": "stick", "from": "rig.knee", "by": [0, 0, 0.1] } }]);
    let mut tx = Tx::new(Document::new());
    run(&mut tx, "spec.set", json!({ "name": "fig", "spec": spec })).unwrap();
    let (stick, _) = object_at(&tx, "fig/stick");
    assert!((stick - Vec3::new(0.1, 0.5, 0.1)).length() < 1e-4, "{stick}");
}

#[test]
fn skeleton_mistakes_point_at_the_spec() {
    let cases = [
        (
            json!({ "parts": { "a": { "primitive": { "box": {} }, "anchor": { "on": "rig", "landmark": "kne" } } } }),
            "/spec/parts/a/anchor/landmark",
            "knee",
        ),
        (json!({ "parts": { "a": { "primitive": { "box": {} }, "anchor": { "on": "rig", "param": [0, 0.5] } } } }), "/spec/parts/a/anchor", ""),
        (
            json!({ "parts": { "a": { "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 } }], "anchor": { "on": "rig", "landmark": "hip" }, "join": "blend" } } }),
            "/spec/parts/a/anchor/on",
            "",
        ),
    ];
    for (parts, path, suggestion) in cases {
        let mut spec = rig();
        spec["parts"] = parts["parts"].clone();
        let mut tx = Tx::new(Document::new());
        let e = run(&mut tx, "spec.set", json!({ "name": "fig", "spec": spec })).unwrap_err();
        assert_eq!(e.path.as_deref(), Some(path), "{e:?}");
        if !suggestion.is_empty() {
            assert!(e.suggestions.iter().any(|s| s == suggestion), "{e:?}");
        }
    }
    // Names are shared between skeletons and parts; joint parents must exist.
    let e = p3d_spec::parse(&json!({ "skeletons": { "a": { "joints": {} } }, "parts": { "a": { "primitive": { "box": {} } } } })).unwrap_err();
    assert_eq!(e.path, "/skeletons/a");
    let e = p3d_spec::parse(
        &json!({ "skeletons": { "s": { "joints": { "j": { "at": [0, 0, 0], "parent": "x" } } } }, "parts": { "a": { "primitive": { "box": {} } } } }),
    )
    .unwrap_err();
    assert_eq!(e.path, "/skeletons/s/joints/j/parent");
}
