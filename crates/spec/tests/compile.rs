//! Model Spec compiles: placement by anchors and landmarks, blends, mirrors,
//! instances, edit layers, provenance, incremental rebuilds and errors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;
use std::time::Instant;

use glam::Vec3;
use p3d_ops::{OpCtx, OpError, Registry, Tx};
use p3d_scene::{Document, ObjectKey};
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

fn ok(tx: &mut Tx, op: &str, args: Value) -> String {
    run(tx, op, args).unwrap_or_else(|e| panic!("{op}: {e:?}"))
}

fn key(tx: &Tx, path: &str) -> ObjectKey {
    tx.doc().resolve(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// World position of an object's origin.
fn at(tx: &Tx, path: &str) -> Vec3 {
    tx.doc().object(key(tx, path)).unwrap().transform.translation
}

fn elf_head() -> Value {
    json!({
        "spec": "elf_head",
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
            "nose": {
                "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0, -0.025, 0.012], "radius": "6mm", "radius_b": "8mm" } }],
                "anchor": { "on": "head", "landmark": "nose_root" },
                "join": "blend",
                "blend": "6mm"
            },
            "ear_l": {
                "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0.04, 0.06, -0.02], "radius": "1cm", "radius_b": "2mm" }, "name": "lobe" }],
                "anchor": { "on": "head", "landmark": "ear_root_l" },
                "join": "blend",
                "blend": "8mm"
            },
            "ear_r": { "mirror": "ear_l" },
            "eye_l": {
                "primitive": { "uv_sphere": { "radius": 0.012, "origin": "center" } },
                "anchor": { "on": "head", "landmark": "eye_l", "offset": [0, 0, -0.008] }
            },
            "eye_r": { "mirror": "eye_l" }
        }
    })
}

#[test]
fn the_elf_head_blockout_compiles_from_a_spec() {
    let mut tx = Tx::new(Document::new());
    let s = ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": elf_head() }));
    assert!(s.starts_with("spec elf_head (o:1): 6 parts, 3 built, 0 unchanged, 3 blended"), "{s}");
    // Blended parts live in the head's mesh; separate ones are objects.
    for name in ["head", "eye_l", "eye_r"] {
        key(&tx, &format!("elf_head/{name}"));
    }
    for name in ["nose", "ear_l", "ear_r"] {
        assert!(tx.doc().resolve(&format!("elf_head/{name}")).is_err(), "{name} is blended");
    }
    let head = tx.doc().object_mesh(key(&tx, "elf_head/head")).unwrap().clone();
    assert!(head.edges().all(|e| !head.is_boundary_edge(e)), "closed");
    for g in ["cranium", "jaw", "nose.shape_0", "ear_l.lobe", "ear_r.lobe"] {
        assert!(head.group(g).is_some_and(|g| g.count() > 0), "group {g}");
    }
    // The ears stick out on both sides, past the cranium (75 mm half width).
    let (lo, hi) = head.bounds().unwrap();
    assert!(hi.x > 0.09 && lo.x < -0.09, "{lo} {hi}");
    assert!((hi.x + lo.x).abs() < 0.004, "symmetric: {lo} {hi}");
    // Eyes: left on +x, mirrored, on the face.
    let (l, r) = (at(&tx, "elf_head/eye_l"), at(&tx, "elf_head/eye_r"));
    assert!(l.x > 0.015 && (l.x + r.x).abs() < 1e-5 && (l.y - r.y).abs() < 1e-5, "{l} {r}");
    assert!(l.z > 0.05, "eyes sit on the front: {l}");
}

#[test]
fn every_face_has_provenance() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": elf_head() }));
    let root = key(&tx, "elf_head");
    let record = tx.doc().spec(root).unwrap().clone();
    for (name, rec) in &record.parts {
        let Some(id) = rec.object else { continue };
        let mesh = tx.doc().object_mesh(tx.doc().object_by_id(id).unwrap()).unwrap();
        assert_eq!(rec.provenance.face_count(), mesh.face_count(), "{name}");
        let per_face = rec.provenance.per_face();
        assert!(per_face.iter().all(|&e| (e as usize) < rec.provenance.entries.len()), "{name}");
    }
    // Head faces trace back to the blended parts' shapes.
    let head = &record.parts["head"].provenance;
    let paths: Vec<&str> = head.entries.iter().map(|e| e.path.as_str()).collect();
    for p in ["/parts/head/sdf/0", "/parts/head/sdf/1", "/parts/nose/sdf/0", "/parts/ear_l/sdf/0"] {
        assert!(paths.contains(&p), "{p} in {paths:?}");
    }
    let counts = |path: &str| {
        let i = head.entries.iter().position(|e| e.path == path).unwrap() as u32;
        head.per_face().iter().filter(|&&e| e == i).count()
    };
    assert!(counts("/parts/nose/sdf/0") > 10 && counts("/parts/ear_l/sdf/0") > 20, "blended parts own faces");
}

#[test]
fn a_one_part_patch_recompiles_only_that_part() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": elf_head() }));
    let head_mesh = tx.doc().object(key(&tx, "elf_head/head")).unwrap().mesh_key();
    let start = Instant::now();
    let s = ok(
        &mut tx,
        "spec.patch",
        json!({ "target": "elf_head", "merge": { "parts": { "eye_l": { "primitive": { "uv_sphere": { "radius": 0.014, "origin": "center" } } } } } }),
    );
    let took = start.elapsed();
    assert!(s.starts_with("spec elf_head (o:1): 6 parts, 2 built, 1 unchanged, 3 blended"), "{s}");
    assert_eq!(tx.doc().object(key(&tx, "elf_head/head")).unwrap().mesh_key(), head_mesh, "the head mesh is reused");
    #[cfg(not(debug_assertions))]
    assert!(took.as_millis() < 100, "{took:?}");
    let _ = took;
    // Changing a blended part rebuilds its parent too.
    let s = ok(&mut tx, "spec.patch", json!({ "target": "elf_head", "merge": { "parts": { "ear_l": { "blend": "4mm" } } } }));
    assert!(s.contains("1 built, 2 unchanged"), "{s}");
}

#[test]
fn moving_an_anchor_parent_moves_children_without_rebuilding_them() {
    let mut tx = Tx::new(Document::new());
    ok(
        &mut tx,
        "spec.set",
        json!({ "name": "lamp", "spec": { "parts": {
            "base": { "primitive": { "cylinder": { "radius": 0.1, "height": 0.02 } } },
            "pole": { "primitive": { "cylinder": { "radius": 0.01, "height": 0.4 } }, "anchor": { "on": "base", "landmark": "top" } },
            "shade": { "primitive": { "cone": { "radius_bottom": 0.12, "radius_top": 0.05, "height": 0.15 } }, "anchor": { "on": "pole", "landmark": "top" } }
        } } }),
    );
    assert!((at(&tx, "lamp/pole").y - 0.02).abs() < 1e-4, "{}", at(&tx, "lamp/pole"));
    assert!((at(&tx, "lamp/shade").y - 0.42).abs() < 1e-4, "{}", at(&tx, "lamp/shade"));
    let s = ok(
        &mut tx,
        "spec.patch",
        json!({ "target": "lamp", "merge": { "parts": { "base": { "primitive": { "cylinder": { "radius": 0.1, "height": 0.05 } } } } } }),
    );
    assert!(s.contains("1 built, 2 unchanged"), "{s}");
    assert!((at(&tx, "lamp/shade").y - 0.45).abs() < 1e-4, "{}", at(&tx, "lamp/shade"));
}

#[test]
fn removing_a_part_deletes_its_object_and_instances_share_meshes() {
    let mut tx = Tx::new(Document::new());
    ok(
        &mut tx,
        "spec.set",
        json!({ "name": "rack", "spec": { "parts": {
            "post": { "primitive": { "box": { "width": 0.05, "height": 1, "depth": 0.05 } } },
            "post_2": { "instance": "post", "transform": { "position": [0.5, 0, 0] } },
            "rail": { "primitive": { "box": { "width": 0.6, "height": 0.04, "depth": 0.04 } }, "transform": { "position": [0.25, 0.9, 0] } }
        } } }),
    );
    let a = tx.doc().object(key(&tx, "rack/post")).unwrap().mesh_key();
    let b = tx.doc().object(key(&tx, "rack/post_2")).unwrap().mesh_key();
    assert_eq!(a, b, "instances share the mesh");
    ok(&mut tx, "spec.patch", json!({ "target": "rack", "merge": { "parts": { "rail": null } } }));
    assert!(tx.doc().resolve("rack/rail").is_err());
}

#[test]
fn edit_layers_run_on_the_part_and_keep_provenance() {
    let mut tx = Tx::new(Document::new());
    ok(
        &mut tx,
        "spec.set",
        json!({ "name": "crate", "spec": { "parts": {
            "body": {
                "primitive": { "box": { "width": 0.6, "height": 0.6, "depth": 0.6 } },
                "edits": [{ "op": "edit.inset", "args": { "select": { "faces": { "normal": { "dir": [0, 0, 1], "within_deg": 10 } } }, "thickness": "4cm" }, "label": "panel" }]
            }
        } } }),
    );
    let root = key(&tx, "crate");
    let rec = tx.doc().spec(root).unwrap().parts["body"].clone();
    let mesh = tx.doc().object_mesh(key(&tx, "crate/body")).unwrap();
    assert!(mesh.group("panel.inner").is_some(), "the edit's groups exist");
    let edit = rec.provenance.entries.iter().position(|e| e.path == "/parts/body/edits/0" && e.op == "edit.inset").unwrap() as u32;
    assert!(rec.provenance.per_face().contains(&edit));
}

#[test]
fn errors_point_into_the_spec() {
    let mut tx = Tx::new(Document::new());
    let mut spec = elf_head();
    spec["parts"]["eye_l"]["anchor"]["landmark"] = json!("eye_ll");
    let e = run(&mut tx, "spec.set", json!({ "name": "h", "spec": spec })).unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/spec/parts/eye_l/anchor/landmark"), "{e:?}");
    assert!(e.suggestions.contains(&"eye_l".to_owned()), "{e:?}");

    let e = run(
        &mut tx,
        "spec.set",
        json!({ "name": "h", "spec": { "parts": {
            "a": { "primitive": { "box": {} } , "anchor": { "on": "b", "landmark": "top" } },
            "b": { "primitive": { "box": {} } , "anchor": { "on": "a", "landmark": "top" } }
        } } }),
    )
    .unwrap_err();
    assert!(e.message.contains("loop"), "{e:?}");

    let e = run(&mut tx, "spec.set", json!({ "name": "h", "spec": { "parts": { "a": { "primitive": { "box": { "width": -1 } } } } } })).unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/spec/parts/a/primitive/box/width"), "{e:?}");

    let e = run(
        &mut tx,
        "spec.set",
        json!({ "name": "h", "spec": { "parts": {
            "a": { "primitive": { "box": {} } },
            "b": { "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 } }], "anchor": { "on": "a", "landmark": "top" }, "join": "blend" }
        } } }),
    )
    .unwrap_err();
    assert!(e.message.contains("SDF parts on both sides"), "{e:?}");
}

#[test]
fn compiles_are_deterministic() {
    let build = || {
        let mut tx = Tx::new(Document::new());
        ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": elf_head() }));
        let m = tx.doc().object_mesh(key(&tx, "elf_head/head")).unwrap().clone();
        m.positions
    };
    assert_eq!(build(), build());
}

#[test]
fn every_spec_example_produces_its_documented_result() {
    let reg = registry();
    let mut setup: Option<Value> = None;
    for id in ["spec.set", "spec.patch", "spec.compile", "brief.set"] {
        let op = reg.get(id).unwrap();
        for ex in op.meta().examples {
            let mut tx = Tx::new(Document::new());
            // Patches and compiles act on the spec.set example's spec.
            if let Some(args) = setup.as_ref().filter(|_| id != "brief.set") {
                ok(&mut tx, "spec.set", args.clone());
            }
            let args: Value = serde_json::from_str(ex.args).unwrap();
            let got = ok(&mut tx, id, args.clone());
            assert_eq!(got, ex.result, "{id}: got `{got}`");
            if id == "spec.set" {
                setup = Some(args);
            }
        }
    }
}

#[test]
fn landmarks_and_provenance_can_be_queried() {
    let mut tx = Tx::new(Document::new());
    ok(&mut tx, "spec.set", json!({ "name": "elf_head", "spec": elf_head(), "position": [1, 0, 0] }));
    let root = key(&tx, "elf_head");
    let marks = p3d_spec::part_landmarks(tx.doc(), root, "head").unwrap();
    let names: Vec<&str> = marks.iter().map(|m| m.name.as_str()).collect();
    for n in ["center", "top", "crown", "ear_root_l", "cranium", "jaw"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let crown = marks.iter().find(|m| m.name == "crown").unwrap();
    assert!(crown.point.x > 0.99 && crown.point.y > 0.2 && crown.normal.y > 0.9, "world space: {crown:?}");
    let head = key(&tx, "elf_head/head");
    let mesh = tx.doc().object_mesh(head).unwrap();
    let ear_face = mesh.group("ear_l.lobe").unwrap().iter().next().unwrap();
    let (part, entry) = p3d_spec::face_provenance(tx.doc(), head, ear_face).unwrap();
    assert_eq!((part.as_str(), entry.path.as_str()), ("head", "/parts/ear_l/sdf/0"));
}

#[test]
fn shells_and_silhouettes_are_shape_kinds() {
    let mut tx = Tx::new(Document::new());
    ok(
        &mut tx,
        "spec.set",
        json!({ "name": "axe", "spec": { "parts": {
            "head": {
                "silhouettes": {
                    "front": "M -0.02 0 L 0.12 -0.06 C 0.16 0 0.16 0.08 0.12 0.14 L -0.02 0.08 Z",
                    "side": [[-0.015, -0.06], [0.015, -0.06], [0.015, 0.14], [-0.015, 0.14]],
                    "round": "3mm", "detail": "4mm"
                }
            },
            "feather": {
                "shell": { "outline": "M 0 0 C 0.02 0.03 0.015 0.09 0 0.12 C -0.015 0.09 -0.02 0.03 0 0 Z", "bend": 0.2, "detail": "6mm" },
                "anchor": { "on": "head", "landmark": "top" }
            }
        } } }),
    );
    let rec = tx.doc().spec(key(&tx, "axe")).unwrap().clone();
    assert_eq!(rec.parts["head"].provenance.entries[0].op, "sdf.silhouettes");
    assert_eq!(rec.parts["feather"].provenance.entries[0].path, "/parts/feather/shell");
    // `top` is the surface above the part's center: the sloped edge there.
    assert!((at(&tx, "axe/feather").y - 0.1164).abs() < 0.004, "on the head's top: {}", at(&tx, "axe/feather"));
}

#[test]
fn deform_handles_reshape_a_part_and_survive_recompiles() {
    let mut tx = Tx::new(Document::new());
    let ear = |deform: Option<Value>| {
        let mut p = json!({ "sdf": [{ "capsule": { "a": [0, 0, 0], "b": [0, 0.08, 0], "radius": "1.5cm", "radius_b": "5mm" }, "name": "lobe" }], "detail": "3mm" });
        if let Some(d) = deform {
            p["deform"] = d;
        }
        json!({ "name": "s", "spec": { "parts": { "ear": p } } })
    };
    ok(&mut tx, "spec.set", ear(None));
    let before = tx.doc().object_mesh(key(&tx, "s/ear")).unwrap().bounds().unwrap();
    let s = ok(&mut tx, "spec.set", ear(Some(json!({ "handles": [{ "move": "top", "by": [0, 0.01, 0] }], "pins": ["bottom"], "radius": "7cm" }))));
    assert!(s.contains("1 built"), "{s}");
    let after = tx.doc().object_mesh(key(&tx, "s/ear")).unwrap().bounds().unwrap();
    assert!((after.1.y - before.1.y - 0.01).abs() < 1.5e-3, "the tip rose 1 cm: {before:?} -> {after:?}");
    assert!((after.0.y - before.0.y).abs() < 1e-3, "the root stayed: {before:?} -> {after:?}");
    // A recompile keeps the deformed mesh (same hash) and names the error path.
    let s = ok(&mut tx, "spec.compile", json!({ "target": "s" }));
    assert!(s.contains("0 built, 1 unchanged"), "{s}");
    let e = run(&mut tx, "spec.set", ear(Some(json!({ "handles": [{ "move": "tipp", "by": [0, 0.01, 0] }] })))).unwrap_err();
    assert_eq!(e.path.as_deref(), Some("/spec/parts/ear/deform/handles/0/move"), "{e:?}");
    assert!(e.suggestions.iter().any(|s| s == "top" || s == "lobe"), "{e:?}");
}

#[test]
fn weld_joins_stitch_a_part_into_its_parent() {
    let mut tx = Tx::new(Document::new());
    let s = ok(
        &mut tx,
        "spec.set",
        json!({ "name": "beast", "spec": { "parts": {
            "body": { "primitive": { "box": { "width": 0.4, "height": 0.3, "depth": 0.3, "origin": "center" } } },
            "horn": {
                "primitive": { "cone": { "radius_bottom": 0.05, "radius_top": 0.005, "height": 0.2 } },
                "anchor": { "on": "body", "landmark": "top", "offset": [0, -0.03, 0] },
                "join": "weld"
            }
        } } }),
    );
    assert!(s.contains("1 welded") && s.contains("horn (cone) welded into body"), "{s}");
    assert!(tx.doc().resolve("beast/horn").is_err(), "no object of its own");
    let m = tx.doc().object_mesh(key(&tx, "beast/body")).unwrap();
    assert!(m.edges().all(|e| !m.is_boundary_edge(e)), "one closed mesh");
    assert!(m.group("horn").is_some_and(|g| g.count() > 0), "the horn's faces are named");
    assert!(m.groups().all(|(n, _)| !n.starts_with("@prov")), "no temporary groups left");
    let (lo, hi) = m.bounds().unwrap();
    assert!(hi.y > 0.3 && lo.y < -0.14, "{lo} {hi}");
    let rec = tx.doc().spec(key(&tx, "beast")).unwrap().parts["body"].clone();
    assert!(rec.provenance.entries.iter().any(|e| e.path == "/parts/horn/primitive"), "{:?}", rec.provenance.entries);
    assert_eq!(rec.provenance.face_count(), m.face_count());
}
