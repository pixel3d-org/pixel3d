//! The wing feature: feathered wings are closed, outward-facing feather
//! slabs on an arm (or open cards), membrane wings closed panels between
//! finger bones; the bones follow fold, raise and tilt; right wings mirror
//! left ones.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use glam::Vec3;
use p3d_features::{Shape, build, wing_bones};
use p3d_mesh::Mesh;
use serde_json::json;

fn mesh(params: serde_json::Value) -> Mesh {
    match build("wing", &params).unwrap().shape {
        Shape::Mesh(m) => m,
        Shape::Sdf(_) => panic!("a wing is a mesh"),
    }
}

fn volume(m: &Mesh) -> f32 {
    let mut v = 0.0f32;
    for f in m.faces() {
        for t in m.face_triangles(f) {
            let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize]);
            v += a.dot(b.cross(c)) / 6.0;
        }
    }
    v
}

#[test]
fn feathered_and_membrane_wings_are_closed_and_face_out() {
    for params in
        [json!({}), json!({ "type": "membrane" }), json!({ "fold": 0.8, "tilt": 70 }), json!({ "type": "membrane", "fold": 0.9, "fingers": 5 })]
    {
        let m = mesh(params.clone());
        assert_eq!(m.validate(), Ok(()), "{params}");
        assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "closed: {params}");
        assert!(volume(&m) > 0.0, "outward: {params}");
    }
    let f = mesh(json!({ "coverts": 3 }));
    for g in ["bones", "primaries", "secondaries", "coverts"] {
        assert!(f.group(g).is_some(), "{g}");
    }
    assert!(mesh(json!({ "type": "membrane" })).group("membrane").is_some());
    // Cards are open sheets.
    let cards = mesh(json!({ "feathers": "cards", "coverts": 0 }));
    assert!(cards.edges().any(|e| cards.is_boundary_edge(e)));
}

#[test]
fn bones_follow_the_pose_and_sides_mirror() {
    let spread = wing_bones(&json!({ "span": 1.0, "raise": 0, "sweep": 0 })).unwrap();
    assert!((spread.tip.x - 1.0).abs() < 0.12 && spread.tip.y.abs() < 1e-5, "{:?}", spread.tip);
    let raised = wing_bones(&json!({ "span": 1.0, "raise": 30 })).unwrap();
    assert!(raised.tip.y > 0.3, "{:?}", raised.tip);
    let folded = wing_bones(&json!({ "span": 1.0, "fold": 1.0, "raise": 0 })).unwrap();
    assert!(folded.tip.x < 0.4 && folded.tip.z < -0.2, "folded back: {:?}", folded.tip);
    let tilted = wing_bones(&json!({ "tilt": 70, "raise": 0 })).unwrap();
    assert!(tilted.back.y < -0.9, "the trailing edge hangs down: {:?}", tilted.back);
    let membrane = wing_bones(&json!({ "type": "membrane", "fingers": 5 })).unwrap();
    assert_eq!(membrane.fingers.len(), 5);
    let l = build("wing", &json!({ "side": "left" })).unwrap().landmarks;
    let r = build("wing", &json!({ "side": "right" })).unwrap().landmarks;
    for (name, p) in &l {
        assert!((r[name] - *p * Vec3::new(-1.0, 1.0, 1.0)).length() < 1e-5, "{name}");
    }
}
