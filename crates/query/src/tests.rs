use p3d_mesh::{EdgeFlags, ElementKind, Mesh, Vec3};
use serde_json::json;

use crate::{QueryError, Selector, select};

/// Unit cube centered at (0.5, 0.5, 0.5) with box groups and sharp edges.
fn cube() -> Mesh {
    let p = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
    let faces = [0, 2, 3, 1, 4, 5, 7, 6, 0, 1, 5, 4, 2, 6, 7, 3, 0, 4, 6, 2, 1, 3, 7, 5];
    let mut m = Mesh::from_polygons(p, &[4; 6], &faces).unwrap();
    for (name, f) in [("back", 0), ("front", 1), ("bottom", 2), ("top", 3), ("left", 4), ("right", 5)] {
        m.set_group(name, ElementKind::Face, [f]).unwrap();
    }
    for e in m.edges().collect::<Vec<_>>() {
        m.set_edge_flag(e, EdgeFlags::SHARP, true);
    }
    m
}

fn sel(m: &Mesh, j: serde_json::Value) -> Result<Vec<u32>, QueryError> {
    let s: Selector = serde_json::from_value(j).unwrap();
    select(m, &s).map(|s| s.iter().collect())
}

#[test]
fn spec_examples_parse_and_run() {
    let m = cube();
    assert_eq!(sel(&m, json!({ "faces": { "group": "top" } })).unwrap(), vec![3]);
    assert_eq!(sel(&m, json!({ "faces": { "normal": { "dir": [0, 1, 0], "within_deg": 10 } } })).unwrap(), vec![3]);
    assert_eq!(sel(&m, json!({ "edges": { "sharper_than_deg": 30 } })).unwrap().len(), 12);
    assert_eq!(sel(&m, json!({ "verts": { "in_box": { "min": [-1, 0, -1], "max": [0.5, 2, 1] } } })).unwrap(), vec![0, 2, 4, 6]);
    let and = json!({ "and": [ { "faces": {} }, { "faces": { "normal": { "dir": [1, 0, 0], "within_deg": 45 } } } ] });
    assert_eq!(sel(&m, and).unwrap(), vec![5]);
    let grown = sel(&m, json!({ "grow": { "steps": 1, "of": { "faces": { "group": "top" } } } })).unwrap();
    assert_eq!(grown.len(), 5, "top plus its four neighbours");
}

#[test]
fn empty_filter_selects_everything_live() {
    let mut m = cube();
    assert_eq!(sel(&m, json!({ "faces": {} })).unwrap().len(), 6);
    m.delete_faces(&[3]).unwrap();
    assert_eq!(sel(&m, json!({ "faces": {} })).unwrap(), vec![0, 1, 2, 4, 5]);
    assert_eq!(sel(&m, json!({ "not": { "faces": { "group": "front" } } })).unwrap(), vec![0, 2, 4, 5]);
}

#[test]
fn boundary_and_flags() {
    let mut m = cube();
    m.delete_faces(&[3]).unwrap();
    assert_eq!(sel(&m, json!({ "edges": { "boundary": true } })).unwrap().len(), 4);
    assert_eq!(sel(&m, json!({ "verts": { "boundary": true } })).unwrap(), vec![2, 3, 6, 7]);
    assert_eq!(sel(&m, json!({ "faces": { "boundary": true } })).unwrap(), vec![0, 1, 4, 5]);
    let e = m.edges().next().unwrap();
    m.set_edge_flag(e, EdgeFlags::SHARP, false);
    m.set_edge_flag(e, EdgeFlags::SEAM, true);
    assert_eq!(sel(&m, json!({ "edges": { "sharp": false } })).unwrap(), vec![e]);
    assert_eq!(sel(&m, json!({ "edges": { "seam": true, "sharp": false } })).unwrap(), vec![e]);
    m.face_material[1] = 2;
    assert_eq!(sel(&m, json!({ "faces": { "material": 2 } })).unwrap(), vec![1]);
}

#[test]
fn grow_and_shrink() {
    let m = cube();
    let all = sel(&m, json!({ "grow": { "steps": 2, "of": { "faces": { "group": "top" } } } })).unwrap();
    assert_eq!(all.len(), 6);
    let back = sel(&m, json!({ "shrink": { "steps": 1, "of": { "not": { "faces": { "group": "top" } } } } })).unwrap();
    assert_eq!(back, vec![2], "only the bottom has no neighbour in the top");
    let v = sel(&m, json!({ "grow": { "steps": 1, "of": { "verts": { "in_box": { "min": [0, 0, 0], "max": [0, 0, 0] } } } } })).unwrap();
    assert_eq!(v, vec![0, 1, 2, 4]);
    let e =
        sel(&m, json!({ "grow": { "steps": 1, "of": { "and": [ { "edges": { "in_box": { "min": [0, 0, 0], "max": [0.6, 0, 0] } } } ] } } })).unwrap();
    assert_eq!(e.len(), 5, "edge 0-1 plus the four other edges at its ends");
}

#[test]
fn errors_point_into_the_selector() {
    let m = cube();
    match sel(&m, json!({ "faces": { "group": "tpo" } })) {
        Err(QueryError::NoGroup { path, suggestions, .. }) => {
            assert_eq!(path, "/faces/group");
            assert_eq!(suggestions, vec!["top"]);
        }
        other => panic!("{other:?}"),
    }
    let mut m2 = cube();
    m2.set_group("rim", ElementKind::Edge, [0]).unwrap();
    assert!(matches!(sel(&m2, json!({ "faces": { "group": "rim" } })), Err(QueryError::GroupKind { has: "edges", .. })));
    let err = sel(&m, json!({ "or": [ { "faces": {} }, { "edges": {} } ] })).unwrap_err();
    assert_eq!(err.path(), "/or/1");
    assert!(matches!(sel(&m, json!({ "faces": { "sharper_than_deg": 10 } })), Err(QueryError::NotApplicable { .. })));
    assert!(matches!(sel(&m, json!({ "and": [] })), Err(QueryError::Empty { .. })));
    let bad_box = sel(&m, json!({ "verts": { "in_box": { "min": [1, 1, 1], "max": [0, 0, 0] } } })).unwrap_err();
    assert_eq!(bad_box.path(), "/verts/in_box");
    assert!(sel(&m, json!({ "faces": { "normal": { "dir": [0, 0, 0], "within_deg": 5 } } })).is_err());
}

#[test]
fn unknown_fields_are_rejected() {
    let r: Result<Selector, _> = serde_json::from_value(json!({ "faces": { "gruop": "top" } }));
    let msg = r.unwrap_err().to_string();
    assert!(msg.contains("unknown field `gruop`") && msg.contains("group"), "{msg}");
    assert!(serde_json::from_value::<Selector>(json!({ "polygons": {} })).is_err());
}

#[test]
fn schema_is_strict() {
    let schema = serde_json::to_value(schemars::schema_for!(Selector)).unwrap();
    let text = schema.to_string();
    assert!(text.contains("\"additionalProperties\":false"), "{text}");
    assert!(text.contains("sharper_than_deg"));
}

#[test]
fn huge_step_counts_stop_when_nothing_changes() {
    let m = cube();
    let all = sel(&m, json!({ "grow": { "steps": u32::MAX, "of": { "faces": { "group": "top" } } } })).unwrap();
    assert_eq!(all.len(), 6);
    let none = sel(&m, json!({ "shrink": { "steps": u32::MAX, "of": { "not": { "faces": { "group": "top" } } } } })).unwrap();
    assert!(none.is_empty());
}

#[test]
fn edge_grow_from_a_fan_center_is_one_ring() {
    // Triangle fan of 64 around vertex 0: growing all spokes reaches the rim.
    let n = 64u32;
    let mut p = vec![Vec3::ZERO];
    p.extend((0..n).map(|i| {
        let a = i as f32 * std::f32::consts::TAU / n as f32;
        Vec3::new(a.cos(), 0.0, a.sin())
    }));
    let corners: Vec<u32> = (0..n).flat_map(|i| [0, 1 + (i + 1) % n, 1 + i]).collect();
    let m = Mesh::from_polygons(p, &vec![3; n as usize], &corners).unwrap();
    let spokes = sel(&m, json!({ "edges": { "boundary": false } })).unwrap();
    assert_eq!(spokes.len(), n as usize);
    let grown = sel(&m, json!({ "grow": { "steps": 1, "of": { "edges": { "boundary": false } } } })).unwrap();
    assert_eq!(grown.len(), 2 * n as usize);
}
