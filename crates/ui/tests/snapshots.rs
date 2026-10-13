//! Snapshot tests of the main layouts and panels in the dark and light
//! themes (project spec 20.8): the window is drawn headless and compared
//! with the PNGs in `tests/snapshots` with a small tolerance (see
//! [`options`] for per-platform baselines). After an intended change,
//! `UPDATE_SNAPSHOTS=1 cargo test -p p3d-ui --test snapshots` rewrites
//! them. CI uploads the `.new.png` and `.diff.png` of failures.
//!
//! The viewport's GPU picture is not part of these (it has its own tests in
//! `p3d-render`); here it shows its placeholder.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::{Harness, OsThreshold, SnapshotOptions};
use p3d_engine::Session;
use p3d_ui::theme::ThemePref;
use p3d_ui::{PixelApp, Services};
use serde_json::{Value, json};

/// The sample scene with the crate selected, ready to draw.
fn app(theme: ThemePref, setup: &[(&str, Value)]) -> PixelApp {
    let mut app = PixelApp::new(Session::new(), Services::default());
    app.stable = true;
    app.run("file.sample", Value::Null).unwrap();
    app.run("edit.select", json!({ "target": "crate" })).unwrap();
    app.ui.theme = theme;
    for (cmd, params) in setup {
        app.run(cmd, params.clone()).unwrap();
    }
    app
}

fn harness(app: PixelApp) -> Harness<'static, PixelApp> {
    let mut h = Harness::builder().with_size((1280.0, 800.0)).wgpu().build_ui_state(
        |ui, app: &mut PixelApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    // Let the health check finish on its thread and the layout settle.
    for _ in 0..400 {
        h.run_steps(2);
        let a = h.state();
        if a.health.key == Some(a.scene_key()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run_steps(10);
    h
}

/// Compares with `tests/snapshots/<name>.png`, or with the per-platform
/// baseline `tests/snapshots/<os>/<name>.png` when one exists (`<os>` is
/// `macos`, `linux` or `windows`). GPU rasterizers differ slightly in
/// antialiasing: Windows CI draws with WARP, which gets a wider margin.
fn options(name: &str) -> SnapshotOptions {
    let os_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(std::env::consts::OS);
    let options = SnapshotOptions::new().threshold(OsThreshold::new(1.5).windows(2.0)).max_failed_pixels(OsThreshold::new(400).windows(2000));
    if os_dir.join(format!("{name}.png")).exists() { options.output_path(os_dir) } else { options }
}

fn snapshot(h: &mut Harness<'static, PixelApp>, name: &str) {
    h.snapshot_options(name, &options(name));
}

#[test]
fn main_window_dark() {
    let mut h = harness(app(ThemePref::Dark, &[("ui.tool", json!({ "tool": "move" }))]));
    snapshot(&mut h, "main_dark");
}

#[test]
fn main_window_light() {
    let mut h = harness(app(ThemePref::Light, &[("ui.tool", json!({ "tool": "move" }))]));
    snapshot(&mut h, "main_light");
}

#[test]
fn edit_mode_with_faces_selected() {
    let mut h = harness(app(ThemePref::Dark, &[("ui.mode", json!({ "mode": "face" })), ("edit.select_all", Value::Null)]));
    snapshot(&mut h, "edit_mode_dark");
}

#[test]
fn command_palette() {
    let mut h = harness(app(ThemePref::Dark, &[("ui.palette", Value::Null)]));
    snapshot(&mut h, "palette_dark");
}

#[test]
fn uv_workspace() {
    let mut h = harness(app(ThemePref::Dark, &[("ui.workspace", json!({ "workspace": "uv" }))]));
    snapshot(&mut h, "workspace_uv_dark");
}
