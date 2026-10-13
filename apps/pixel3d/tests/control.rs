//! The desktop app driven through its control server, as an agent or a UI
//! test would: open a `.p3d`, move an object with the gizmo, undo, save.
//!
//! Needs a display and a GPU, so it is ignored by default:
//! `cargo test -p pixel3d --test control -- --ignored`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Child, Command};
use std::time::{Duration, Instant};

use p3d_control::Client;
use serde_json::{Value, json};

/// The app process and its control port; killed (and its access token
/// file removed, which a killed app cannot do itself) when dropped.
struct App(Child, u16);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        if let Some(token) = p3d_control::token_path(self.1) {
            let _ = std::fs::remove_file(token);
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Polls `f` until it returns `Some`, for up to `secs` (a timeout names the caller's line).
#[track_caller]
fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(start.elapsed() < Duration::from_secs(secs), "timed out");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn pos_x(c: &mut Client) -> f64 {
    let s = c.call("ui.inspect", Value::Null).unwrap();
    s["activeTransform"]["position"][0].as_f64().unwrap_or(f64::NAN)
}

/// Starts the app with the control server on a free port and `args`
/// (agents name files only inside the folders the user allowed, such as
/// an `--allow-dir`). Returns the app and a connected client, once the
/// first frames are drawn.
fn start(args: &[String]) -> (App, Client) {
    let port = free_port();
    let mut all: Vec<String> = args.to_vec();
    all.extend(["--fresh".to_owned(), "--control".to_owned(), port.to_string()]);
    let app = App(Command::new(env!("CARGO_BIN_EXE_pixel3d")).args(&all).spawn().unwrap(), port);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut c = wait_for(60, || Client::connect_published(addr).ok());
    // Wait for the first frames: the viewport has a size.
    wait_for(60, || c.call("ui.inspect", Value::Null).ok().filter(|s| s["viewport"][2].as_f64().unwrap_or(0.0) > 10.0));
    (app, c)
}

/// A fresh folder for one test run.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pixel3d-control-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

#[test]
#[ignore = "needs a display and a GPU"]
fn open_edit_with_the_gizmo_undo_and_save() {
    let dir = temp_dir("accept");
    let file = dir.join("accept.p3d");
    let path = file.display().to_string();
    let (_app, mut c) = start(&["--sample".to_owned(), "--allow-dir".to_owned(), dir.display().to_string()]);

    // Save the sample, start over, open it again.
    c.call("ui.command", json!({ "id": "file.save_as", "params": { "path": path } })).unwrap();
    c.call("ui.command", json!({ "id": "file.new" })).unwrap();
    assert_eq!(c.call("ui.inspect", Value::Null).unwrap()["scene"]["objects"], json!(0));
    c.call("ui.command", json!({ "id": "file.open", "params": { "path": path } })).unwrap();
    assert_eq!(c.call("ui.inspect", Value::Null).unwrap()["scene"]["objects"], json!(4));

    // Select the crate, take the Move tool, drag the X arrow.
    c.call("ui.command", json!({ "id": "edit.select", "params": { "target": "crate" } })).unwrap();
    c.call("ui.command", json!({ "id": "ui.tool", "params": { "tool": "move" } })).unwrap();
    let gizmo = wait_for(30, || c.call("ui.inspect", Value::Null).ok().map(|s| s["gizmo"].clone()).filter(Value::is_object));
    let (cx, cy) = (gizmo["center"][0].as_f64().unwrap(), gizmo["center"][1].as_f64().unwrap());
    let (tx, ty) = (gizmo["x"][0].as_f64().unwrap(), gizmo["x"][1].as_f64().unwrap());
    let len = ((tx - cx).powi(2) + (ty - cy).powi(2)).sqrt();
    let (dx, dy) = ((tx - cx) / len * 120.0, (ty - cy) / len * 120.0);
    // Grab the shaft just inside the tip.
    let from = [cx + (tx - cx) * 0.8, cy + (ty - cy) * 0.8];
    c.call("ui.drag", json!({ "from": from, "to": [from[0] + dx, from[1] + dy], "steps": 12 })).unwrap();
    let moved = wait_for(30, || Some(pos_x(&mut c)).filter(|x| *x > 0.05));
    assert!(moved > 0.05, "the crate moved along +X: {moved}");

    // Undo with the keyboard, then save and check the file.
    c.call("ui.key", json!({ "key": "Z", "modifiers": { "cmd": true } })).unwrap();
    wait_for(30, || Some(pos_x(&mut c)).filter(|x| x.abs() < 1e-4));

    // Modal G Z 0.5 Enter from the keyboard (Dynamic Input), then undo.
    for (key, text) in [("G", None), ("Z", None), ("", Some("0.5")), ("Enter", None)] {
        match text {
            Some(t) => c.call("ui.text", json!({ "text": t })).unwrap(),
            None => c.call("ui.key", json!({ "key": key })).unwrap(),
        };
    }
    let pos_z = |c: &mut Client| c.call("ui.inspect", Value::Null).unwrap()["activeTransform"]["position"][2].as_f64().unwrap_or(f64::NAN);
    wait_for(30, || Some(pos_z(&mut c)).filter(|z| (z - 0.5).abs() < 1e-4));
    c.call("ui.command", json!({ "id": "edit.undo" })).unwrap();
    c.call("ui.command", json!({ "id": "file.save" })).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() > 0);
    let shot = dir.join("after.png");
    let r = c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert_eq!(r["path"], json!(shot.display().to_string()));
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}

fn inspect(c: &mut Client) -> Value {
    c.call("ui.inspect", Value::Null).unwrap()
}

/// Waits until the camera rests (framing eases it in over a few frames).
fn settle_camera(c: &mut Client) {
    let mut last = Value::Null;
    wait_for(30, || {
        let cam = inspect(c)["camera"].clone();
        let resting = cam == last;
        last = cam;
        resting.then_some(())
    });
}

#[test]
#[ignore = "needs a display and a GPU"]
fn booleans_imports_and_cleanup_in_the_app() {
    let dir = temp_dir("bool");
    let scene = dir.join("scene.p3d").display().to_string();
    // A box without its top: cleanup.auto would fill the hole.
    let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
               f 1 4 3 2\nf 1 2 6 5\nf 3 4 8 7\nf 1 5 8 4\nf 2 3 7 6\n";
    std::fs::write(dir.join("bin.obj"), obj).unwrap();
    // A start-up command is the user's: saving there makes the folder the project's.
    let (_app, mut c) = start(&["--command".to_owned(), format!("file.save path={scene}"), "--command".to_owned(), "ui.start on=false".to_owned()]);
    assert_eq!(inspect(&mut c)["allowedFolders"], json!([dir.display().to_string()]));
    let elsewhere = temp_dir("bool-elsewhere").join("x.p3d").display().to_string();
    let refused = c.call("ui.command", json!({ "id": "file.save_as", "params": { "path": elsewhere } })).unwrap_err().to_string();
    assert!(refused.contains("only the folders the user allowed"), "{refused}");

    // A crate and a drill through it; the crate selected last (active).
    c.call("ui.command", json!({ "id": "prim.box", "params": { "name": "crate" } })).unwrap();
    c.call("ui.command", json!({ "id": "prim.cylinder", "params": { "name": "drill", "radius": 0.2, "height": 2.0 } })).unwrap();
    c.call("ui.command", json!({ "id": "edit.select", "params": { "target": "drill" } })).unwrap();
    c.call("ui.command", json!({ "id": "edit.select", "params": { "target": "crate", "add": true } })).unwrap();
    // The Object menu's Difference is enabled now; Cmd+Minus runs it.
    c.call("ui.key", json!({ "key": "Minus", "modifiers": { "cmd": true } })).unwrap();
    wait_for(30, || Some(inspect(&mut c)).filter(|s| s["scene"]["objects"] == json!(1)));
    // Tab: Edit Mode with the cut faces selected.
    c.call("ui.key", json!({ "key": "Tab" })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["mode"] == json!("face")));
    assert!(s["edit"]["picked"].as_array().is_some_and(|p| !p.is_empty()), "the cut faces are picked");
    c.call("ui.key", json!({ "key": "Tab" })).unwrap();

    // Import from the project folder in the background, then run the fix
    // the import's warning offers by clicking its button.
    let bin = dir.join("bin.obj").display().to_string();
    c.call("ui.command", json!({ "id": "file.import", "params": { "path": bin } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["scene"]["objects"] == json!(2)));
    let revision = s["scene"]["revision"].clone();
    let fix = wait_for(30, || {
        let found = c.call("ui.elements", json!({ "label": "Fix: Clean up" })).ok()?;
        found["elements"].as_array().and_then(|e| e.first()).cloned()
    });
    c.call("ui.click", json!({ "target": { "id": fix["id"] } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["scene"]["revision"] != revision));
    assert!(s["status"].as_str().is_some_and(|m| m.contains("filled 1 hole")), "{}", s["status"]);

    // Mesh > Clean Up > Report changes nothing.
    let revision = s["scene"]["revision"].clone();
    c.call("ui.command", json!({ "id": "selection.run", "params": { "op": "cleanup.report" } })).unwrap();
    assert_eq!(inspect(&mut c)["scene"]["revision"], revision);
    let shot = dir.join("bool.png");
    c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}

#[test]
#[ignore = "needs a display and a GPU"]
fn sculpt_an_sdf_shape_with_the_gizmo() {
    let dir = temp_dir("sdf");
    let (_app, mut c) = start(&["--command".to_owned(), "ui.start on=false".to_owned()]);
    // An SDF object: selecting it opens the editor.
    c.call(
        "ui.command",
        json!({ "id": "shapes.new", "params": { "name": "blob", "detail": 0.02, "shapes": [
        { "name": "body", "sphere": { "at": [0, 0.2, 0], "radius": 0.2 } },
        { "name": "head", "smooth_union": 0.05, "sphere": { "at": [0, 0.45, 0], "radius": 0.12 } }
    ] } }),
    )
    .unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["shapes"]["object"].is_string()));
    assert_eq!(s["shapes"]["shapes"].as_array().map(Vec::len), Some(2));
    let revision = |c: &mut Client| inspect(c)["scene"]["revision"].clone();
    let head_x = |c: &mut Client| inspect(c)["shapes"]["selectedShape"]["sphere"]["at"][0].as_f64().unwrap_or(f64::NAN);
    // Select the head, frame it, take the Move tool: the gizmo sits on the shape.
    c.call("ui.command", json!({ "id": "shapes.select", "params": { "name": "head" } })).unwrap();
    c.call("ui.command", json!({ "id": "view.frame" })).unwrap();
    c.call("ui.command", json!({ "id": "ui.tool", "params": { "tool": "move" } })).unwrap();
    let gizmo = wait_for(30, || inspect(&mut c)["gizmo"].as_object().cloned());
    let (cx, cy) = (gizmo["center"][0].as_f64().unwrap(), gizmo["center"][1].as_f64().unwrap());
    let (tx, ty) = (gizmo["x"][0].as_f64().unwrap(), gizmo["x"][1].as_f64().unwrap());
    let len = ((tx - cx).powi(2) + (ty - cy).powi(2)).sqrt();
    let before = revision(&mut c);
    // Drag the X arrow and hold: the mesh previews coarsely, nothing commits.
    let from = [cx + (tx - cx) * 0.8, cy + (ty - cy) * 0.8];
    let to = [from[0] + (tx - cx) / len * 80.0, from[1] + (ty - cy) / len * 80.0];
    c.call("ui.drag", json!({ "from": from, "to": to, "steps": 10, "hold": true })).unwrap();
    wait_for(30, || Some(inspect(&mut c)).filter(|s| s["shapes"]["previewing"] == json!(true)));
    assert_eq!(revision(&mut c), before, "nothing is committed while dragging");
    // Enter confirms: one undo step at full detail, the head moved along +X.
    c.call("ui.key", json!({ "key": "Enter" })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["scene"]["revision"] != before && s["shapes"]["busy"] == json!(false)));
    assert_eq!(s["shapes"]["previewing"], json!(false));
    let moved = head_x(&mut c);
    assert!(moved > 0.01, "the head moved along +X: {moved}");
    let shot = dir.join("sdf.png");
    c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    // Undo puts the shapes back in the editor.
    c.call("ui.command", json!({ "id": "edit.undo" })).unwrap();
    wait_for(30, || Some(head_x(&mut c)).filter(|x| x.abs() < 1e-6));
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}

#[test]
#[ignore = "needs a display and a GPU"]
fn remesh_in_the_background_and_review_poles() {
    let dir = temp_dir("remesh");
    let (_app, mut c) = start(&["--command".to_owned(), "ui.start on=false".to_owned()]);
    c.call("ui.command", json!({ "id": "prim.torus", "params": { "name": "ring", "major_radius": 0.3, "minor_radius": 0.1 } })).unwrap();
    // Mesh > Remesh > Quad remesh: the card estimates the result first.
    c.call("ui.command", json!({ "id": "job.open", "params": { "op": "remesh.quad" } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["jobCard"]["estimate"]["faces"].as_u64().is_some_and(|f| f > 0)));
    assert_eq!(s["jobCard"]["estimate"]["openEdges"], json!(0));
    let before = s["scene"]["revision"].clone();
    // Click Run in the card: the remesh runs in the background, one step.
    let run = wait_for(30, || {
        let found = c.call("ui.elements", json!({ "label": "Run" })).ok()?;
        found["elements"].as_array().and_then(|e| e.iter().find(|w| w["label"] == json!("Run")).cloned())
    });
    c.call("ui.click", json!({ "target": { "id": run["id"] } })).unwrap();
    let s = wait_for(60, || Some(inspect(&mut c)).filter(|s| s["scene"]["revision"] != before && s["jobCard"]["running"] == json!(false)));
    assert!(s["jobCard"]["summary"].as_str().is_some_and(|m| m.contains("remeshed into")), "{}", s["jobCard"]);
    // The pole overlay and its counts for agents.
    c.call("ui.command", json!({ "id": "view.poles", "params": { "on": true } })).unwrap();
    let poles = c.call("ui.command", json!({ "id": "mesh.poles", "params": { "target": "ring" } })).unwrap();
    let total = ["valence3", "valence5", "valence6plus"].iter().map(|k| poles["objects"][0][k].as_u64().unwrap_or(0)).sum::<u64>();
    assert!(total > 0, "a remeshed torus has some poles: {poles}");
    let shot = dir.join("poles.png");
    c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}

#[test]
#[ignore = "needs a display and a GPU"]
fn edit_a_model_spec_in_the_spec_tab() {
    let dir = temp_dir("spec");
    let (_app, mut c) = start(&["--command".to_owned(), "ui.start on=false".to_owned()]);
    // Add > Model Spec > Lamp spec: the root is selected, the Spec tab open.
    c.call("ui.command", json!({ "id": "spec.starter", "params": { "name": "lamp" } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["spec"]["root"].is_string()));
    assert_eq!(s["spec"]["parts"].as_array().map(Vec::len), Some(3));
    // Room for the Spec tab under the Outliner.
    c.call("ui.command", json!({ "id": "ui.dock", "params": { "operation": "close", "panel": "health" } })).unwrap();
    c.call("ui.command", json!({ "id": "view.frame" })).unwrap();
    settle_camera(&mut c);
    let revision = |c: &mut Client| inspect(c)["scene"]["revision"].clone();
    let shade_y = |c: &mut Client| inspect(c)["activeTransform"]["position"][1].as_f64().unwrap_or(f64::NAN);
    // Click the shade in the Outliner: its part opens in the Spec tab.
    let row = wait_for(30, || {
        let found = c.call("ui.elements", json!({ "label": "shade" })).ok()?;
        found["elements"].as_array().and_then(|e| e.iter().find(|w| w["role"] == json!("treeitem")).cloned())
    });
    c.call("ui.click", json!({ "target": { "id": row["id"] } })).unwrap();
    wait_for(30, || Some(inspect(&mut c)).filter(|s| s["spec"]["part"] == json!("shade")));
    let before = (revision(&mut c), shade_y(&mut c));
    // Pick a landmark: the pole's landmarks show in the viewport; a click
    // on its center anchors the shade there (one undo step).
    let pick = wait_for(30, || {
        let found = c.call("ui.elements", json!({ "label": "Pick landmark" })).ok()?;
        found["elements"].as_array().and_then(|e| e.first().cloned())
    });
    c.call("ui.click", json!({ "target": { "id": pick["id"] } })).unwrap();
    let s = wait_for(30, || {
        Some(inspect(&mut c))
            .filter(|s| s["spec"]["picking"] == json!(true) && s["spec"]["onScreen"]["landmarks"].as_array().is_some_and(|l| !l.is_empty()))
    });
    assert_eq!(s["spec"]["onScreen"]["part"], json!("pole"));
    let center = s["spec"]["onScreen"]["landmarks"].as_array().unwrap().iter().find(|l| l["name"] == json!("center")).cloned().unwrap();
    c.call("ui.click", json!({ "target": { "at": center["at"] } })).unwrap();
    let s = wait_for(60, || Some(inspect(&mut c)).filter(|s| s["scene"]["revision"] != before.0 && s["spec"]["busy"] == json!(false)));
    assert_eq!((s["spec"]["picking"].clone(), s["spec"]["part"].clone()), (json!(false), json!("shade")), "the click picked, not selected");
    assert!(shade_y(&mut c) < before.1 - 0.1, "the shade moved down to the pole's middle");
    // The provenance of the face under a resting pointer.
    let s = inspect(&mut c);
    let base = s["spec"]["onScreen"].clone();
    assert!(base.is_null(), "the overlay is off after picking: {base}");
    c.call("ui.command", json!({ "id": "landmarks.toggle", "params": { "on": true } })).unwrap();
    c.call("ui.command", json!({ "id": "spec.select_part", "params": { "part": "base" } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["spec"]["onScreen"]["landmarks"].as_array().is_some_and(|l| !l.is_empty())));
    let front = s["spec"]["onScreen"]["landmarks"].as_array().unwrap().iter().find(|l| l["name"] == json!("front")).cloned().unwrap();
    c.call("ui.hover", json!({ "target": { "at": front["at"] } })).unwrap();
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["spec"]["provenance"].is_object()));
    assert!(s["spec"]["provenance"]["path"].as_str().is_some_and(|p| p.starts_with("/parts/")), "{}", s["spec"]["provenance"]);
    let shot = dir.join("spec.png");
    c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    // Undo puts the shade back on the pole's top.
    c.call("ui.command", json!({ "id": "edit.undo" })).unwrap();
    let raw = c.call("ui.command", json!({ "id": "spec.json", "params": { "text": "{\"parts\": {\"pole\": {\"join\": \"glue\"}}}" } }));
    assert!(raw.is_err());
    let s = inspect(&mut c);
    assert_eq!(s["spec"]["jsonError"]["path"], json!("/parts/pole/join"));
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}

#[test]
#[ignore = "needs a display and a GPU"]
fn drag_an_outline_point_and_rebuild() {
    let dir = temp_dir("outline");
    let (_app, mut c) = start(&["--command".to_owned(), "ui.start on=false".to_owned()]);
    // Add > Outline > Leaf shell: the Outline tab opens on its canvas.
    c.call("ui.command", json!({ "id": "outline.new", "params": { "starter": "leaf" } })).unwrap();
    c.call("ui.command", json!({ "id": "ui.dock", "params": { "operation": "close", "panel": "health" } })).unwrap();
    c.call("ui.command", json!({ "id": "view.frame" })).unwrap();
    settle_camera(&mut c);
    let s = wait_for(30, || Some(inspect(&mut c)).filter(|s| s["outline"]["points"].as_array().is_some_and(|p| p.len() == 2)));
    let height = |s: &Value| {
        let b = &s["outline"]["mesh"]["bounds"];
        b[1][1].as_f64().unwrap_or(0.0) - b[0][1].as_f64().unwrap_or(0.0)
    };
    let (before, revision, path) = (height(&s), s["scene"]["revision"].clone(), s["outline"]["layers"][0]["value"].clone());
    // Drag the tip up: a preview while dragging, one rebuild on release.
    let tip = s["outline"]["points"].as_array().unwrap().iter().find(|p| p["index"] == json!(1)).cloned().unwrap();
    let (x, y) = (tip["at"][0].as_f64().unwrap(), tip["at"][1].as_f64().unwrap());
    c.call("ui.drag", json!({ "from": [x, y], "to": [x, y - 40.0], "steps": 12 })).unwrap();
    let s = wait_for(60, || {
        Some(inspect(&mut c))
            .filter(|s| s["scene"]["revision"] != revision && s["outline"]["busy"] == json!(false) && s["outline"]["dragging"] == json!(false))
    });
    assert_ne!(s["outline"]["layers"][0]["value"], path, "the outline changed");
    assert!(height(&s) > before + 0.005, "the rebuilt leaf is longer: {before} -> {}", height(&s));
    assert_eq!(s["scene"]["objects"], json!(1), "rebuilt in place");
    let shot = dir.join("outline.png");
    c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    // Undo brings the old outline back.
    c.call("ui.command", json!({ "id": "edit.undo" })).unwrap();
    wait_for(30, || Some(inspect(&mut c)).filter(|s| s["outline"]["layers"][0]["value"] == path));
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}
