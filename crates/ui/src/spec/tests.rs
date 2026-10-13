use std::time::Duration;

use super::*;
use crate::Services;
use crate::inspector::Tab;

/// A new app with a starter spec added (and so selected).
fn with(starter: &str) -> PixelApp {
    let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
    a.run("spec.starter", json!({ "name": starter })).unwrap();
    a
}

/// Lets background runs finish and takes them in.
fn settle(a: &mut PixelApp) {
    let ctx = egui::Context::default();
    for _ in 0..4000 {
        a.spec_tick(&ctx);
        a.sdf_tick(&ctx);
        if !a.spec.busy() && !a.sdf.busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the spec runs did not finish");
}

fn select(a: &mut PixelApp, name: &str) {
    let k = a.doc().resolve(name).unwrap();
    a.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
    a.spec_follow();
    a.sdf_follow();
}

fn y_of(a: &PixelApp, path: &str) -> f32 {
    let k = a.doc().resolve(path).unwrap();
    a.doc().world_affine(k).unwrap().translation.y
}

#[test]
fn a_starter_opens_the_spec_tab_with_its_parts() {
    let a = with("lamp");
    let root = a.doc().resolve("lamp").unwrap();
    assert_eq!((a.spec.root, a.spec.part.as_deref(), a.ui.inspector_tab), (Some(root), None, Tab::Spec));
    let parts = parts_of(a.doc(), root);
    let seen: Vec<(&str, &str, &str, Option<&str>)> = parts.iter().map(|p| (p.name.as_str(), p.kind.as_str(), p.join, p.on.as_deref())).collect();
    assert_eq!(
        seen,
        [("base", "cylinder", "separate", None), ("pole", "cylinder", "separate", Some("base")), ("shade", "cone", "separate", Some("pole"))]
    );
    assert!(parts.iter().all(|p| p.object.is_some() && p.blend_into.is_none()));
    // The raw view shows the source.
    assert!(a.spec.json.contains("\"shade\""));
}

#[test]
fn the_elf_heads_blended_parts_merge_into_the_head() {
    let a = with("elf_head");
    let doc = a.doc();
    let root = doc.resolve("elf_head").unwrap();
    let head = doc.resolve("elf_head/head").unwrap();
    let parts = parts_of(doc, root);
    let get = |n: &str| parts.iter().find(|p| p.name == n).unwrap();
    assert_eq!((get("nose").join, get("nose").object, get("nose").blend_into), ("blend", None, Some(head)));
    // A mirror joins like its source.
    assert_eq!((get("ear_r").kind.as_str(), get("ear_r").join, get("ear_r").blend_into), ("mirror of ear_l", "blend", Some(head)));
    assert_eq!((get("eye_r").join, get("eye_r").object.is_some()), ("separate", true));
}

#[test]
fn part_objects_jump_to_their_part() {
    let mut a = with("lamp");
    select(&mut a, "lamp/pole");
    assert_eq!(a.spec.part.as_deref(), Some("pole"));
    // The root keeps the part chosen in the tab.
    select(&mut a, "lamp");
    assert_eq!(a.spec.part.as_deref(), Some("pole"));
    a.run("spec.select_part", json!({ "part": "shade" })).unwrap();
    assert_eq!(a.spec.part.as_deref(), Some("shade"));
    assert!(a.run("spec.select_part", json!({ "part": "lampshade" })).is_err());
    // Another object closes the editor.
    a.run("prim.box", json!({ "name": "crate" })).unwrap();
    a.spec_follow();
    assert_eq!((a.spec.root, a.spec.part.as_deref()), (None, None));
}

#[test]
fn a_patch_runs_in_the_background_as_one_undo_step() {
    let mut a = with("lamp");
    select(&mut a, "lamp/pole");
    let steps = a.session.history().len();
    let shade = y_of(&a, "lamp/shade");
    a.spec_patch_part(json!({ "primitive": { "cylinder": { "height": 0.6 } } }), "Shape").unwrap();
    assert!(a.spec.busy());
    settle(&mut a);
    assert_eq!(a.session.history().len(), steps + 1, "one undo step");
    // The shade sits on the pole's top, so it moved up with it.
    assert!((y_of(&a, "lamp/shade") - shade - 0.2).abs() < 1e-3, "{} -> {}", shade, y_of(&a, "lamp/shade"));
    a.run("edit.undo", Value::Null).unwrap();
    assert!((y_of(&a, "lamp/shade") - shade).abs() < 1e-4);
}

#[test]
fn errors_point_at_their_spec_path_and_line() {
    let mut a = with("lamp");
    select(&mut a, "lamp/pole");
    a.run("spec.set_landmark", json!({ "landmark": "topp" })).unwrap();
    settle(&mut a);
    let e = a.spec.error.clone().expect("the patch failed");
    assert_eq!(e.path, "/parts/pole/anchor/landmark", "{e:?}");
    let line = e.line.expect("the line of the path");
    assert!(a.spec.json.lines().nth(line).unwrap().contains("\"landmark\""), "{line}");
    assert!(e.message.contains("top"), "suggestions: {}", e.message);
}

#[test]
fn the_raw_source_applies_or_marks_the_error() {
    let mut a = with("lamp");
    let mut source = a.doc().spec(a.spec.root.unwrap()).unwrap().source.clone();
    source["parts"]["pole"]["join"] = json!("glue");
    assert!(a.run("spec.json", json!({ "spec": source })).is_err());
    let e = a.spec.json_error.clone().unwrap();
    assert_eq!(e.path, "/parts/pole/join");
    assert!(a.spec.json.lines().nth(e.line.unwrap()).unwrap().contains("glue"));
    // Broken JSON: the line of the syntax error.
    assert!(a.run("spec.json", json!({ "text": "{\n  \"parts\": {\n    oops\n  }\n}" })).is_err());
    assert_eq!(a.spec.json_error.as_ref().and_then(|e| e.line), Some(2));
    // A good source replaces the spec.
    source["parts"]["pole"]["join"] = json!("separate");
    source["parts"]["pole"]["tags"] = json!(["metal"]);
    a.run("spec.json", json!({ "spec": source })).unwrap();
    settle(&mut a);
    let root = a.spec.root.unwrap();
    assert_eq!(a.doc().spec(root).unwrap().source["parts"]["pole"]["tags"], json!(["metal"]));
    assert!(!a.spec.json_edited && a.spec.json_error.is_none());
}

#[test]
fn landmarks_list_and_picking_anchors_the_part() {
    let mut a = with("lamp");
    select(&mut a, "lamp/shade");
    let names: Vec<String> = a.spec_landmarks_of("pole").iter().map(|l| l.name.clone()).collect();
    assert!(names.iter().any(|n| n == "top") && names.iter().any(|n| n == "center"), "{names:?}");
    a.run("spec.pick_landmark", json!({ "on": true })).unwrap();
    assert!(a.spec.picking);
    // While picking, the overlay shows the parent's landmarks.
    assert_eq!(a.spec_shown_landmarks().map(|(p, _)| p).as_deref(), Some("pole"));
    let before = y_of(&a, "lamp/shade");
    a.run("spec.set_landmark", json!({ "landmark": "center" })).unwrap();
    assert!(!a.spec.picking);
    settle(&mut a);
    assert!(y_of(&a, "lamp/shade") < before - 0.1, "the shade moved down to the pole's middle");
    // The overlay toggle.
    assert_eq!(a.run("landmarks.toggle", Value::Null).unwrap(), json!({ "landmarks": true }));
    assert!(a.ui.view.landmarks);
}

#[test]
fn sdf_parts_open_the_shapes_editor_and_patch_the_spec() {
    let mut a = with("elf_head");
    select(&mut a, "elf_head/head");
    let root = a.doc().resolve("elf_head").unwrap();
    assert_eq!(a.sdf.part, Some((root, "head".to_owned())));
    assert_eq!(a.sdf.shapes.len(), 2, "cranium and jaw");
    assert_eq!(a.sdf_shown_object(), a.doc().resolve("elf_head/head").ok());
    let steps = a.session.history().len();
    a.run("shapes.delete", json!({ "path": [1] })).unwrap();
    settle(&mut a);
    assert_eq!(a.session.history().len(), steps + 1);
    let source = &a.doc().spec(root).unwrap().source;
    assert_eq!(source["parts"]["head"]["sdf"].as_array().map(Vec::len), Some(1), "the jaw left the spec");
    // A blended SDF part opens the editor too, without an object.
    a.run("spec.select_part", json!({ "part": "nose" })).unwrap();
    a.sdf_follow();
    assert_eq!((a.sdf.part.as_ref().map(|p| p.1.as_str()), a.sdf.object), (Some("nose"), None));
    // A primitive part closes it.
    a.run("spec.select_part", json!({ "part": "eye_l" })).unwrap();
    a.sdf_follow();
    assert!(!a.sdf.is_open());
}

#[test]
fn hovering_a_blended_part_lights_its_faces_in_the_head() {
    let mut a = with("elf_head");
    let doc = Arc::clone(a.session.doc());
    a.viewport.scene(&doc, a.scene_key(), &std::collections::BTreeSet::new());
    let root = doc.resolve("elf_head").unwrap();
    a.spec.hover_next = Some((root, "nose".to_owned()));
    a.spec_tick(&egui::Context::default());
    let (o, _) = a.spec_highlight().expect("the nose's faces light up");
    assert!(!o.faces.is_empty());
}

#[test]
fn the_face_under_the_pointer_names_its_spec_node() {
    let mut a = with("lamp");
    let doc = Arc::clone(a.session.doc());
    a.viewport.scene(&doc, a.scene_key(), &std::collections::BTreeSet::new());
    a.viewport.rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
    a.viewport.camera.set_view(p3d_render::ViewKind::Front);
    a.viewport.camera.frame(Vec3::new(-0.15, 0.0, -0.15), Vec3::new(0.15, 0.6, 0.15), 1.0);
    let pole = doc.resolve("lamp/pole").unwrap();
    let mid = doc.world_affine(pole).unwrap().transform_point3(Vec3::new(0.0, 0.2, 0.0));
    let vp = a.viewport.camera.view_proj(1.0);
    let at = project(vp, a.viewport.rect, mid).unwrap();
    let (k, part, entry) = a.spec_provenance_at(at).expect("the pole is under the pointer");
    assert_eq!((k, part.as_str()), (pole, "pole"));
    assert!(entry.path.starts_with("/parts/pole"), "{}", entry.path);
    assert_eq!(entry.op, "prim.cylinder");
    // Nothing there: no provenance.
    assert!(a.spec_provenance_at(egui::pos2(2.0, 2.0)).is_none());
}

#[test]
fn json_pointers_find_their_lines() {
    let text = serde_json::to_string_pretty(
        &json!({ "parts": { "a": { "anchor": { "on": "b", "landmark": "top" } }, "b": { "sdf": [{ "sphere": {} }, { "box": {} }] } } }),
    )
    .unwrap();
    let line_of = |p: &str| pointer_line(&text, p).map(|l| text.lines().nth(l).unwrap().trim().to_owned());
    assert_eq!(line_of("/parts/a/anchor/landmark").as_deref(), Some("\"landmark\": \"top\","));
    assert_eq!(line_of("/parts/b/sdf/1").as_deref(), Some("{"));
    assert!(text.lines().nth(pointer_line(&text, "/parts/b/sdf/1").unwrap() + 1).unwrap().contains("box"));
    // A missing key: its deepest existing ancestor.
    assert_eq!(line_of("/parts/a/anchor/offset").as_deref(), Some("\"anchor\": {"));
    assert_eq!(spec_path("/ops/0/args/spec/parts/a/join"), "/parts/a/join");
    assert_eq!(spec_path("/merge/parts/a"), "/parts/a");
}

#[test]
fn the_inspect_state_lists_parts_for_agents() {
    let mut a = with("lamp");
    select(&mut a, "lamp/pole");
    let s = a.spec_state();
    assert_eq!(s["part"], "pole");
    assert_eq!(s["parts"].as_array().map(Vec::len), Some(3));
    assert_eq!(s["parts"][1]["on"], "base");
}

#[test]
fn welded_parts_show_like_blended_ones() {
    let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
    a.run(
        "spec.set",
        json!({ "name": "beast", "spec": { "parts": {
            "body": { "primitive": { "box": { "width": 0.4, "height": 0.3, "depth": 0.3, "origin": "center" } } },
            "horn": { "primitive": { "cone": { "radius_bottom": 0.05, "radius_top": 0.005, "height": 0.2 } },
                "anchor": { "on": "body", "landmark": "top", "offset": [0, -0.03, 0] }, "join": "weld" }
        } } }),
    )
    .unwrap();
    let doc = Arc::clone(a.session.doc());
    let root = doc.resolve("beast").unwrap();
    let body = doc.resolve("beast/body").unwrap();
    let parts = parts_of(&doc, root);
    let horn = parts.iter().find(|p| p.name == "horn").unwrap();
    assert_eq!((horn.join, horn.object, horn.blend_into), ("weld", None, Some(body)));
    // Hovering its row lights the faces welded into the body.
    a.viewport.scene(&doc, a.scene_key(), &std::collections::BTreeSet::new());
    a.spec.hover_next = Some((root, "horn".to_owned()));
    a.spec_tick(&egui::Context::default());
    assert!(a.spec_highlight().is_some_and(|(o, _)| !o.faces.is_empty()));
}

/// The feature head of `docs/agents/examples/feature_head.json`, coarser.
fn feature_head() -> PixelApp {
    let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
    a.run(
        "spec.set",
        json!({ "name": "elf", "spec": { "category": "humanoid_head", "parts": {
            "head": { "feature": "head_base", "params": { "cheekbones": 0.8, "jaw": 0.7 }, "detail": "5mm" },
            "nose": { "feature": "nose", "params": { "length": "4.2cm", "width": "3cm" }, "anchor": { "on": "head", "landmark": "nose_root" },
                "join": "blend", "blend": "4mm" },
            "ear_l": { "feature": "ear", "params": { "variant": "elf", "length": "8cm" }, "anchor": { "on": "head", "landmark": "ear_root_l" } },
            "ear_r": { "mirror": "ear_l" }
        } } }),
    )
    .unwrap();
    a
}

fn size_of(doc: &p3d_scene::Document, path: &str) -> f32 {
    doc.object_mesh(doc.resolve(path).unwrap()).and_then(|m| m.bounds()).map_or(0.0, |(lo, hi)| (hi - lo).max_element())
}

#[test]
fn feature_parts_preview_param_drags_and_commit_one_step() {
    let mut a = feature_head();
    select(&mut a, "elf/ear_l");
    assert_eq!(a.spec.part.as_deref(), Some("ear_l"));
    let steps = a.session.history().len();
    let before = size_of(a.doc(), "elf/ear_l");
    // A drag: the patch compiles in the background and shows as a preview.
    let fields = json!({ "params": { "length": 0.11 } });
    a.spec.pending = Some((fields.clone(), "Feature parameters".into()));
    a.spec_preview(json!({ "parts": { "ear_l": fields.clone() } }));
    let ctx = egui::Context::default();
    for _ in 0..4000 {
        a.spec_tick(&ctx);
        if a.spec.preview_doc().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let (preview, _) = a.spec.preview_doc().expect("a preview");
    assert!(size_of(&preview, "elf/ear_l") > before + 0.02, "{before} -> {}", size_of(&preview, "elf/ear_l"));
    assert_eq!(a.session.history().len(), steps, "nothing committed while dragging");
    assert!(a.spec.preview_ms().is_some());
    // Letting go: one step.
    let (pending, label) = a.spec.pending.take().unwrap();
    a.spec_patch_part(pending, &label).unwrap();
    settle(&mut a);
    assert_eq!(a.session.history().len(), steps + 1);
    let root = a.doc().resolve("elf").unwrap();
    assert_eq!(a.doc().spec(root).unwrap().source["parts"]["ear_l"]["params"]["length"], json!(0.11));
    assert!(a.spec.preview_doc().is_none());
}

#[test]
fn feature_forms_read_spec_params_with_defaults() {
    let info = crate::features::info("ear").unwrap();
    let schema = crate::features::schema(&info);
    let root = p3d_ops::Registry::builtin().get("sdf.mesh").map(|o| o.params_schema()).unwrap_or(Value::Null);
    let v = crate::spec::panel::form_value_for_tests(&schema, &root, &json!({ "length": "8cm", "variant": "elf" }));
    assert_eq!((v["length"].clone(), v["variant"].clone()), (json!(0.08), json!("elf")));
    assert_eq!(v.as_object().map(|m| m.len()), Some(info.params.len()), "every parameter, defaults filled");
}

#[test]
fn sdf_feature_parts_show_their_landmarks() {
    let mut a = feature_head();
    select(&mut a, "elf");
    a.run("view.feature_landmarks", json!({ "on": true })).unwrap();
    let marks = a.feature_landmarks();
    let names: Vec<&str> = marks.iter().map(|(_, n, _)| n.as_str()).collect();
    // The head and nose are SDF features: from the spec; the ears' groups.
    assert!(names.contains(&"crown") && names.contains(&"chin"), "{names:?}");
    assert!(names.contains(&"ala_l") && names.contains(&"tip"), "{names:?}");
    assert!(names.iter().all(|n| !n.starts_with("lm.")));
}
