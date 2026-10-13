//! `.p3d` round trips, determinism, deduplication and corruption handling.

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use std::io::{Cursor, Read, Write};

use glam::{DVec3, Quat, Vec3};
use p3d_geo::prim::{BoxSpec, CylinderSpec, Origin, cuboid, cylinder};
use p3d_io::p3d::{self, P3dError};
use p3d_math::Trs;
use p3d_scene::{AssetClass, Check, Document, Expectation, ObjectKind, ObjectRef, OpNode, PartRecord, ProvEntry, Provenance, Range, SpecRecord};
use serde_json::json;

/// crate (box, bevel) > lid (box) > hinge (empty); post (cylinder);
/// post_instance shares post's mesh; post_copy shares its data under its own key.
fn sample() -> Document {
    let mut d = Document::new();
    d.settings.asset_class = AssetClass::HeroProp;
    d.extras.insert("future_field".into(), json!({ "keep": [1, 2, 3] }));
    let crate_mesh =
        cuboid(&BoxSpec { size: DVec3::splat(0.6), bevel_width: 0.01, bevel_segments: 2, ..BoxSpec::default() }, Origin::Bottom).unwrap();
    let c = d.create_mesh_object("crate", None, crate_mesh).unwrap();
    let lid = d.create_mesh_object("lid", Some(c), cuboid(&BoxSpec::default(), Origin::Center).unwrap()).unwrap();
    let hinge = d.create_object("hinge", Some(lid), ObjectKind::Empty).unwrap();
    let post = d.create_mesh_object("post", None, cylinder(&CylinderSpec::default(), Origin::Bottom).unwrap()).unwrap();
    let mk = d.object(post).unwrap().mesh_key().unwrap();
    d.create_object("post_instance", None, ObjectKind::Mesh { mesh: mk }).unwrap();
    let copy = d.add_shared_mesh(d.mesh_arc(mk).unwrap().clone());
    d.create_object("post_copy", None, ObjectKind::Mesh { mesh: copy }).unwrap();
    {
        let o = d.object_mut(lid).unwrap();
        o.transform = Trs { translation: Vec3::new(0.0, 0.6, 0.0), rotation: Quat::from_rotation_x(-0.3), scale: Vec3::new(1.0, 0.1, 1.0) };
        o.tags.push("low".into());
        o.metadata.insert("hint".into(), json!("oak"));
        o.locked = true;
    }
    d.object_mut(hinge).unwrap().visible = false;
    d.push_history(c, OpNode { op: "prim.box".into(), version: 1, params: json!({ "width": 0.6 }), label: "box_1".into() }).unwrap();
    d.expectations_mut().push(Expectation {
        name: "E1".into(),
        check: Check::Attached {
            part: ObjectRef { name: "lid".into(), id: 2 },
            to: ObjectRef { name: "crate".into(), id: 1 },
            max_gap: 0.002,
            max_penetration: Some(0.0),
        },
    });
    d.expectations_mut().push(Expectation {
        name: "size".into(),
        check: Check::Dims {
            target: ObjectRef { name: "crate".into(), id: 1 },
            width: Some(Range { min: Some(0.5), max: None }),
            height: None,
            depth: None,
        },
    });
    d.set_brief("elf", json!({ "title": "Elf head", "category": "humanoid_head" }));
    let root = d.create_object("elf_head", None, ObjectKind::Empty).unwrap();
    let entries = vec![ProvEntry { path: "/parts/head/sdf/0".into(), op: "sdf.mesh".into() }];
    let record = SpecRecord {
        source: json!({ "spec": "elf_head", "parts": { "head": { "sdf": [{ "sphere": { "at": [0, 0, 0], "radius": 0.1 } }] } } }),
        parts: [(
            "head".to_owned(),
            PartRecord { hash: 7, object: Some(3), fingerprint: 9, provenance: Provenance::from_faces(entries, &[0, 0, 0]) },
        )]
        .into(),
    };
    d.set_spec(root, Some(record)).unwrap();
    // Free an id so next_id must be stored, not inferred.
    let tmp = d.create_object("tmp", None, ObjectKind::Empty).unwrap();
    d.delete(tmp, true).unwrap();
    d
}

/// Everything observable about a document, in a comparable form.
fn dump(d: &Document) -> Vec<String> {
    let mut out = vec![format!(
        "settings {:?} extras {:?} next {} expect {:?} briefs {:?}",
        d.settings,
        d.extras,
        d.next_id(),
        d.expectations(),
        d.briefs().collect::<Vec<_>>()
    )];
    for (k, o) in d.objects() {
        let t = o.transform;
        out.push(format!(
            "{} {} {:?} t={:?} r={:?} s={:?} tags={:?} meta={:?} vis={} lock={} hist={:?} spec={:?}",
            o.public_id(),
            d.path(k).unwrap(),
            o.kind().as_str(),
            t.translation.to_array().map(f32::to_bits),
            t.rotation.to_array().map(f32::to_bits),
            t.scale.to_array().map(f32::to_bits),
            o.tags,
            o.metadata,
            o.visible,
            o.locked,
            d.history(k).map(|g| g.nodes.clone()),
            o.spec
        ));
        if let Some(m) = d.object_mesh(k) {
            out.push(format!("  mesh users={} raw={:?}", d.mesh_users(o.mesh_key().unwrap()), m.to_raw()));
        }
    }
    out
}

#[test]
fn round_trip_keeps_everything() {
    let d = sample();
    let (bytes, report) = p3d::to_bytes(&d).unwrap();
    assert_eq!(report.blobs.len(), 3, "crate, lid, post (instance and copy share post's blob)");
    let mut back = p3d::from_bytes(&bytes).unwrap();
    back.check().unwrap();
    assert_eq!(dump(&back), dump(&d));
    let post = back.resolve("post").unwrap();
    let inst = back.resolve("post_instance").unwrap();
    let copy = back.resolve("post_copy").unwrap();
    assert_eq!(back.object(post).unwrap().mesh_key(), back.object(inst).unwrap().mesh_key(), "instances stay instances");
    assert_ne!(back.object(post).unwrap().mesh_key(), back.object(copy).unwrap().mesh_key(), "copies stay independent");
    assert_eq!(back.create_object_with_id(8, "x", None, ObjectKind::Empty).map(|_| ()), Ok(()));
    assert_eq!(back.next_id(), 9);
}

#[test]
fn save_load_save_gives_identical_bytes_and_blob_hashes() {
    let d = sample();
    let (a, ra) = p3d::to_bytes(&d).unwrap();
    let (b, rb) = p3d::to_bytes(&p3d::from_bytes(&a).unwrap()).unwrap();
    assert_eq!(ra.blobs, rb.blobs);
    assert_eq!(a, b, "archives are reproducible");
    let (c, _) = p3d::to_bytes(&d).unwrap();
    assert_eq!(a, c);
}

#[test]
fn large_blobs_are_deflated_small_ones_stored() {
    let mut d = Document::new();
    d.create_mesh_object("big", None, cuboid(&BoxSpec { segments: [64; 3], ..BoxSpec::default() }, Origin::Center).unwrap()).unwrap();
    d.create_mesh_object("small", None, cuboid(&BoxSpec::default(), Origin::Center).unwrap()).unwrap();
    let (bytes, report) = p3d::to_bytes(&d).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut methods = Vec::new();
    for h in &report.blobs {
        let f = zip.by_name(&format!("blobs/{h}.bin")).unwrap();
        methods.push((f.size() >= p3d::DEFLATE_THRESHOLD as u64, f.compression()));
    }
    for (large, method) in methods {
        assert_eq!(method == zip::CompressionMethod::Deflated, large);
    }
}

#[test]
fn files_on_disk_are_written_atomically() {
    let dir = std::env::temp_dir().join(format!("p3d-io-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.p3d");
    p3d::save(&sample(), &path).unwrap();
    p3d::save(&sample(), &path).unwrap();
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["scene.p3d"], "no temporary files left");
    assert_eq!(dump(&p3d::load(&path).unwrap()), dump(&sample()));
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(matches!(p3d::load(&dir.join("missing.p3d")), Err(P3dError::Io { .. })));
}

/// Rewrites one entry of an archive.
fn patch(bytes: &[u8], name: &str, f: impl Fn(Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let mut src = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..src.len() {
        let mut e = src.by_index(i).unwrap();
        let entry = e.name().unwrap().into_owned();
        let mut data = Vec::new();
        e.read_to_end(&mut data).unwrap();
        if entry == name {
            data = f(data);
        }
        out.start_file(entry, zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(&data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

#[test]
fn corruption_is_detected() {
    let (bytes, report) = p3d::to_bytes(&sample()).unwrap();
    let blob = format!("blobs/{}.bin", report.blobs[0]);
    let flipped = patch(&bytes, &blob, |mut d| {
        d[10] ^= 0xff;
        d
    });
    assert!(matches!(p3d::from_bytes(&flipped), Err(P3dError::Blob { .. })));
    assert!(matches!(p3d::from_bytes(b"not a zip"), Err(P3dError::Zip(_))));

    let edit = |f: &dyn Fn(&mut serde_json::Value)| {
        patch(&bytes, "manifest.json", |d| {
            let mut v: serde_json::Value = serde_json::from_slice(&d).unwrap();
            f(&mut v);
            serde_json::to_vec(&v).unwrap()
        })
    };
    let newer = edit(&|v| v["format_version"] = json!(99));
    assert!(matches!(p3d::from_bytes(&newer), Err(P3dError::Version(_))));
    let foreign = edit(&|v| v["format"] = json!("other"));
    assert!(matches!(p3d::from_bytes(&foreign), Err(P3dError::Format(_))));
    // crate becomes its own grandchild: unreachable from the top level.
    let cycle = edit(&|v| v["objects"][0]["parent"] = json!(3));
    assert!(matches!(p3d::from_bytes(&cycle), Err(P3dError::Object { .. })), "{:?}", p3d::from_bytes(&cycle).err());
    let bad_parent = edit(&|v| v["objects"][1]["parent"] = json!(42));
    assert!(p3d::from_bytes(&bad_parent).is_err());
}

#[test]
fn manifest_is_readable_json_with_stable_order() {
    let (bytes, _) = p3d::to_bytes(&sample()).unwrap();
    let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut text = String::new();
    zip.by_name("manifest.json").unwrap().read_to_string(&mut text).unwrap();
    let keys: Vec<&str> = text.lines().filter(|l| l.starts_with("  \"")).map(|l| l.trim().split('"').nth(1).unwrap()).collect();
    assert_eq!(
        keys,
        vec!["format", "format_version", "app_version", "settings", "next_id", "meshes", "objects", "expectations", "briefs", "future_field"]
    );
    assert!(text.contains("\"hero_prop\""));
}

#[test]
fn json_dump_inlines_meshes_as_polygons() {
    let d = sample();
    let v = p3d::to_json(&d).unwrap();
    assert_eq!(v["format"], "pixel3d");
    assert_eq!(v["objects"][0]["name"], "crate");
    let lid = &v["meshes"][1]["data"];
    assert_eq!(lid["faces"].as_array().unwrap().len(), 6);
    assert_eq!(lid["faces"][0].as_array().unwrap().len(), 4);
    assert_eq!(lid["positions"].as_array().unwrap().len(), 8);
    assert!(lid["groups"].as_array().unwrap().iter().any(|g| g["name"] == "top" && g["kind"] == "faces"));
    assert_eq!(v["expectations"][0]["name"], "E1");
}

#[test]
fn curve_objects_round_trip() {
    use p3d_scene::{Curve, Spline};
    let mut d = Document::new();
    let bez = Spline {
        points: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
        handles: Some(vec![[[0.0, 0.0, 0.0], [0.3, 0.0, 0.0]], [[1.0, 0.7, 0.0], [1.0, 1.0, 0.0]]]),
        closed: false,
    };
    let curve = Curve { splines: vec![bez, Spline::polyline([Vec3::ZERO, Vec3::X, Vec3::Y], true)] };
    let k = d.create_curve_object("path", None, curve.clone()).unwrap();
    let dir = std::env::temp_dir().join(format!("p3d-io-curve-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("curve.p3d");
    p3d::save(&d, &path).unwrap();
    let back = p3d::load(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    back.check().unwrap();
    let bk = back.resolve("path").unwrap();
    assert_eq!(back.object_curve(bk), Some(&curve));
    assert_eq!(back.object(bk).unwrap().id(), d.object(k).unwrap().id());
}
