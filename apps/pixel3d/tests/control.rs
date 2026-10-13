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

struct App(Child);

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Polls `f` until it returns `Some`, for up to `secs`.
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

#[test]
#[ignore = "needs a display and a GPU"]
fn open_edit_with_the_gizmo_undo_and_save() {
    let port = free_port();
    let _app = App(Command::new(env!("CARGO_BIN_EXE_pixel3d")).args(["--fresh", "--sample", "--control", &port.to_string()]).spawn().unwrap());
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut c = wait_for(60, || Client::connect(addr).ok());
    // Wait for the first frames: the viewport has a size.
    wait_for(60, || c.call("ui.inspect", Value::Null).ok().filter(|s| s["viewport"][2].as_f64().unwrap_or(0.0) > 10.0));

    // Save the sample, start over, open it again.
    let dir = std::env::temp_dir().join(format!("pixel3d-control-{port}"));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("accept.p3d");
    let path = file.display().to_string();
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
    c.call("ui.command", json!({ "id": "file.save" })).unwrap();
    assert!(std::fs::metadata(&file).unwrap().len() > 0);
    let shot = dir.join("after.png");
    let r = c.call("ui.screenshot", json!({ "path": shot.display().to_string() })).unwrap();
    assert_eq!(r["path"], json!(shot.display().to_string()));
    assert!(std::fs::metadata(&shot).unwrap().len() > 1000);
    let _ = c.call("ui.command", json!({ "id": "app.quit" }));
}
