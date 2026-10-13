//! Feature templates keep clean topology across their parameter ranges:
//! random parameters give valid, closed meshes with every landmark (or a
//! clean error at a parameter path), and every default builds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use p3d_features::{FeatureInfo, ParamKind, Shape, library};
use p3d_sdf::{MeshSpec, Sdf, mesh};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

fn closed(m: &p3d_mesh::Mesh) -> bool {
    m.edges().all(|e| !m.is_boundary_edge(e))
}

/// Checks one build; returns whether it succeeded.
fn check(info: &FeatureInfo, params: &Value) -> bool {
    match p3d_features::build(info.id, params) {
        Err(e) => {
            assert!(e.path.starts_with("/params"), "{}: {e:?} for {params}", info.id);
            false
        }
        Ok(b) => {
            match &b.shape {
                Shape::Mesh(m) => {
                    assert_eq!(m.validate(), Ok(()), "{} {params}", info.id);
                    assert!(closed(m), "{} is open for {params}", info.id);
                    let baked = p3d_features::baked_landmarks(m);
                    for (name, _) in info.landmarks {
                        assert!(baked.contains_key(*name), "{} lost landmark {name} for {params}", info.id);
                    }
                }
                Shape::Sdf(items) => {
                    let sdf = Sdf::new(items.clone()).unwrap();
                    let (lo, hi) = sdf.bounds();
                    // Thin, long shapes need voxels from their smallest extent.
                    let r = mesh(&sdf, &MeshSpec { voxel: (hi - lo).min_element() / 12.0 }).unwrap();
                    assert!(closed(&r.mesh), "{} {params}", info.id);
                    for (name, _) in info.landmarks {
                        assert!(b.landmarks.contains_key(*name), "{} lost landmark {name}", info.id);
                    }
                }
            }
            true
        }
    }
}

#[test]
fn every_default_builds() {
    for f in library() {
        let info = f.info();
        assert!(check(&info, &json!({})), "{} defaults fail", info.id);
    }
}

/// Parameters drawn inside every range from `u` values in [0, 1).
fn params(info: &FeatureInfo, u: &[f64]) -> Value {
    let mut m = Map::new();
    for (p, &x) in info.params.iter().zip(u.iter().cycle()) {
        let v = match p.kind {
            ParamKind::Choice(words) => json!(words[((x * words.len() as f64) as usize).min(words.len() - 1)]),
            ParamKind::Count => json!((p.min + x * (p.max - p.min)).round()),
            _ => json!(p.min + x * (p.max - p.min)),
        };
        m.insert(p.name.to_owned(), v);
    }
    Value::Object(m)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 24 } else { 128 }))]

    #[test]
    fn random_parameters_keep_clean_topology(which in 0usize..64, u in prop::collection::vec(0.0f64..1.0, 12)) {
        let lib = library();
        let info = lib[which % lib.len()].info();
        // Stay away from the extreme ends, where some combinations are refused by design.
        let u: Vec<f64> = u.iter().map(|x| 0.05 + 0.9 * x).collect();
        check(&info, &params(&info, &u));
    }
}

#[test]
fn most_random_parameters_succeed() {
    // A coarse grid over the middle of the ranges: refusals are rare.
    for f in library() {
        let info = f.info();
        let mut ok = 0;
        let total = 9;
        for i in 0..total {
            let u: Vec<f64> = (0..12).map(|k| 0.2 + 0.6 * (((i * 7 + k * 3) % 10) as f64 / 9.0)).collect();
            if check(&info, &params(&info, &u)) {
                ok += 1;
            }
        }
        assert!(ok * 10 >= total * 7, "{}: only {ok} of {total} built", info.id);
    }
}
