//! Spec constraints: each kind on simple parts, conflicts and errors.

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

fn run(tx: &mut Tx, args: Value) -> Result<String, OpError> {
    let reg = registry();
    reg.get("spec.set")?.run(tx, &args, &OpCtx { registry: Some(reg), ..OpCtx::default() }).map(|a| a.output.summary)
}

fn build(parts: Value, constraints: Value) -> Tx {
    let mut tx = Tx::new(Document::new());
    run(&mut tx, json!({ "name": "s", "spec": { "parts": parts, "constraints": constraints } })).unwrap_or_else(|e| panic!("{e:?}"));
    tx
}

/// World bounds of a part.
fn bounds(tx: &Tx, part: &str) -> (Vec3, Vec3) {
    let k = tx.doc().resolve(&format!("s/{part}")).unwrap();
    let m = tx.doc().object_mesh(k).unwrap();
    let w = tx.doc().world_affine(k).unwrap();
    m.verts()
        .map(|v| w.transform_point3(m.positions[v as usize]))
        .fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(l, h), p| (l.min(p), h.max(p)))
}

fn cube(size: f64, at: [f64; 3]) -> Value {
    json!({ "primitive": { "box": { "width": size, "height": size, "depth": size, "origin": "center" } }, "transform": { "position": at } })
}

#[test]
fn rest_on_drops_parts_onto_the_ground_and_onto_each_other() {
    let tx = build(
        json!({ "crate": cube(0.5, [0.0, 2.0, 0.0]), "lid": cube(0.3, [0.0, 5.0, 0.0]) }),
        json!([{ "rest_on": { "part": "crate" } }, { "rest_on": { "part": "lid", "on": "crate" } }]),
    );
    assert!(bounds(&tx, "crate").0.y.abs() < 1e-4, "{:?}", bounds(&tx, "crate"));
    assert!((bounds(&tx, "lid").0.y - 0.5).abs() < 1e-4, "{:?}", bounds(&tx, "lid"));
}

#[test]
fn align_and_offset_use_landmarks() {
    let tx = build(
        json!({ "a": cube(0.2, [0.0, 0.0, 0.0]), "b": cube(0.4, [1.0, 0.3, 0.0]), "c": cube(0.1, [5.0, 5.0, 5.0]) }),
        json!([
            { "align": { "a": "a.top", "b": "b.top", "axis": "y", "offset": "1cm" } },
            { "offset": { "part": "c", "from": "b.front", "by": [0, 0, 0.05] } }
        ]),
    );
    assert!((bounds(&tx, "a").1.y - (bounds(&tx, "b").1.y + 0.01)).abs() < 1e-4);
    let (lo, hi) = bounds(&tx, "c");
    let c = (lo + hi) * 0.5;
    assert!(c.distance(Vec3::new(1.0, 0.3, 0.25)) < 1e-4, "{c}");
}

#[test]
fn distribute_spaces_parts_evenly_or_with_rhythm() {
    let parts = json!({ "r1": cube(0.05, [0.0, 0.0, 0.0]), "r2": cube(0.05, [0.0, 0.0, 0.0]), "r3": cube(0.05, [0.0, 0.0, 0.0]), "r4": cube(0.05, [0.0, 0.0, 0.0]) });
    let tx = build(parts.clone(), json!([{ "distribute": { "parts": ["r1", "r2", "r3", "r4"], "from": [0, 0, 0], "to": [0.9, 0, 0] } }]));
    let xs: Vec<f32> = ["r1", "r2", "r3", "r4"].iter().map(|p| (bounds(&tx, p).0.x + bounds(&tx, p).1.x) * 0.5).collect();
    for (x, want) in xs.iter().zip([0.0, 0.3, 0.6, 0.9]) {
        assert!((x - want).abs() < 1e-4, "{xs:?}");
    }
    let tx =
        build(parts, json!([{ "distribute": { "parts": ["r1", "r2", "r3", "r4"], "from": [0, 0, 0], "to": [0.9, 0, 0], "spacing": "varied" } }]));
    let xs: Vec<f32> = ["r1", "r2", "r3", "r4"].iter().map(|p| (bounds(&tx, p).0.x + bounds(&tx, p).1.x) * 0.5).collect();
    let gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).collect();
    assert!((xs[0]).abs() < 1e-4 && (xs[3] - 0.9).abs() < 1e-4, "ends stay: {xs:?}");
    assert!((gaps[0] - gaps[1]).abs() > 0.05, "a rhythm, not even: {gaps:?}");
}

#[test]
fn symmetric_attach_clearance_inside_and_proportion() {
    let tx = build(
        json!({
            "l": cube(0.1, [0.4, 0.2, 0.1]),
            "r": cube(0.1, [-1.0, 0.0, 0.0]),
            "base": cube(1.0, [0.0, 0.0, 3.0]),
            "knob": cube(0.1, [2.0, 0.0, 3.0]),
            "gem": cube(0.1, [0.0, 0.0, 6.0]),
            "box": cube(0.5, [0.0, 0.0, 9.0]),
            "small": cube(0.2, [3.0, 0.0, 9.0])
        }),
        json!([
            { "symmetric": { "a": "l", "b": "r" } },
            { "attach": { "part": "knob", "to": "base" } },
            { "clearance": { "a": "gem", "b": "base", "min": "20cm" } },
            { "inside": { "part": "small", "in": "box" } },
            { "proportion": { "a": "gem.height", "b": "l.height", "ratio": 2 } }
        ]),
    );
    let c = |p: &str| (bounds(&tx, p).0 + bounds(&tx, p).1) * 0.5;
    assert!(c("r").distance(Vec3::new(-0.4, 0.2, 0.1)) < 1e-4, "{}", c("r"));
    assert!((bounds(&tx, "knob").0.x - 0.5).abs() < 2e-3, "touching the base's +x face: {:?}", bounds(&tx, "knob"));
    assert!(bounds(&tx, "gem").0.z - 3.5 >= 0.2 - 2e-3, "{:?}", bounds(&tx, "gem"));
    let (lo, hi) = bounds(&tx, "small");
    assert!(lo.x >= -0.25 - 1e-4 && hi.x <= 0.25 + 1e-4, "{lo} {hi}");
    let h = bounds(&tx, "gem").1.y - bounds(&tx, "gem").0.y;
    assert!((h - 0.2).abs() < 1e-3, "{h}");
}

#[test]
fn conflicts_warn_and_mirrors_refuse_to_move() {
    let mut tx = Tx::new(Document::new());
    let reg = registry();
    let a = reg
        .get("spec.set")
        .unwrap()
        .run(
            &mut tx,
            &json!({ "name": "s", "spec": {
                "parts": { "a": cube(0.2, [0.0, 1.0, 0.0]) },
                "constraints": [{ "align": { "a": "a.bottom", "b": 0, "axis": "y" } }, { "align": { "a": "a.bottom", "b": 0.5, "axis": "y" } }]
            } }),
            &OpCtx { registry: Some(reg), ..OpCtx::default() },
        )
        .unwrap();
    assert!(a.output.warnings.iter().any(|w| w.message.contains("align(a.bottom, 0)") && w.message.contains("off")), "{:?}", a.output.warnings);
    let e = run(
        &mut tx,
        json!({ "name": "t", "spec": { "parts": { "a": cube(0.2, [0.3, 0.0, 0.0]), "b": { "mirror": "a" } }, "constraints": [{ "rest_on": { "part": "b" } }] } }),
    )
    .unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/spec/constraints/0/part"), "{e:?}");
    assert!(e.message.contains("constrain `a` instead"), "{e:?}");
}
