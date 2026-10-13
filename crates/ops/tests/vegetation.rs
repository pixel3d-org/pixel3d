//! `gen.tree` and the vegetation feature templates through the registry:
//! species and crown outlines, the report kept as metadata, tags, groups
//! and errors at parameter paths.

#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use serde_json::json;

mod fixtures;
use fixtures::*;

#[test]
fn trees_grow_from_species_with_their_report_and_groups() {
    let mut tx = fixture();
    let s = ok(&mut tx, "gen.tree", json!({ "name": "pine", "species": "conifer", "height": "10m", "budget": 5000, "position": [3, 0, 0] }));
    assert!(s.contains("conifer 10") && s.contains("whorl branches") && s.contains("coverage"), "{s}");
    let k = tx.doc().resolve("pine").unwrap();
    let o = tx.doc().object(k).unwrap();
    assert!(o.tags.iter().any(|t| t == "vegetation"));
    let meta = &o.metadata["vegetation"];
    assert_eq!(meta["species"], "conifer");
    assert_eq!(meta["crown"]["shape"], "cone");
    assert!(meta["coverage"].as_f64().unwrap() > 0.6, "{meta}");
    let m = tx.doc().object_mesh(k).unwrap();
    assert!(m.triangle_count() <= 5_500, "{}", m.triangle_count());
    for g in ["trunk", "foliage", "level_1", "lm.fork", "lm.crown_top"] {
        assert!(m.group(g).is_some(), "{g}");
    }
    assert!(m.corner_color.is_some() && m.uv_layer_count() == 2, "wind data");
}

#[test]
fn crown_outlines_shape_the_hull() {
    let mut tx = fixture();
    let s = ok(&mut tx, "gen.tree", json!({ "crown": [[0.3, 0], [1, 0.2], [0.9, 0.6], [0.2, 1]], "crown_width": "8m", "budget": 4000 }));
    assert!(s.contains("broadleaf"), "{s}");
    let k = tx.doc().resolve("tree").unwrap();
    assert_eq!(tx.doc().object(k).unwrap().metadata["vegetation"]["crown"]["shape"], "outline");
}

#[test]
fn tree_errors_point_at_parameters() {
    let mut tx = fixture();
    let e = err(&mut tx, "gen.tree", json!({ "height": "300m" }));
    assert_eq!(e.path.as_deref(), Some("/height"), "{e:?}");
    let e = err(&mut tx, "gen.tree", json!({ "crown": [[1, 0.5]] }));
    assert_eq!(e.path.as_deref(), Some("/crown"), "{e:?}");
    let e = err(&mut tx, "gen.tree", json!({ "levels": 5 }));
    assert_eq!(e.path.as_deref(), Some("/levels"), "{e:?}");
    let e = err(&mut tx, "gen.tree", json!({ "species": "oak" }));
    assert_eq!(e.path.as_deref(), Some("/species"), "{e:?}");
}

#[test]
fn vegetation_features_are_tagged() {
    let mut tx = fixture();
    ok(&mut tx, "gen.feature", json!({ "feature": "flower", "name": "daisy", "params": { "petal_shape": "strap", "petals": 21 } }));
    ok(&mut tx, "gen.feature", json!({ "feature": "grass_cards", "name": "lawn" }));
    for n in ["daisy", "lawn"] {
        let k = tx.doc().resolve(n).unwrap();
        assert!(tx.doc().object(k).unwrap().tags.iter().any(|t| t == "vegetation"), "{n}");
    }
}

#[test]
fn vegetation_expectations_check_crown_coverage_and_overdraw() {
    let mut tx = fixture();
    ok(&mut tx, "gen.tree", json!({ "name": "oak", "budget": 6000 }));
    let s = ok(
        &mut tx,
        "expect.add",
        json!({ "expect": { "vegetation": { "target": "oak", "crown_width": { "min": "9m", "max": "15m" }, "max_overdraw": 4 } } }),
    );
    assert!(s.contains("vegetation(oak)"), "{s}");
    let e = err(&mut tx, "expect.add", json!({ "expect": { "vegetation": { "target": "oak", "coverage": { "min": 85 } } } }));
    assert_eq!(e.path.as_deref(), Some("/expect/vegetation/coverage"), "{e:?}");
    let e = err(&mut tx, "expect.add", json!({ "expect": { "vegetation": { "target": "oak" } } }));
    assert_eq!(e.path.as_deref(), Some("/expect/vegetation"), "{e:?}");
}
