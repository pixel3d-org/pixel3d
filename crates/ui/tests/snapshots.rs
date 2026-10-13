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

#[test]
fn booleans_for_two_meshes() {
    // The barrel first, the crate last (active): the tool settings row
    // offers Difference, Union and Intersect first.
    let select = [("edit.select", json!({ "target": "barrel" })), ("edit.select", json!({ "target": "crate", "add": true }))];
    let mut h = harness(app(ThemePref::Dark, &select));
    snapshot(&mut h, "booleans_dark");
}

#[test]
fn cleanup_report() {
    let mut h = harness(app(ThemePref::Dark, &[("selection.run", json!({ "op": "cleanup.report" }))]));
    snapshot(&mut h, "cleanup_report_dark");
}

#[test]
fn import_warning_with_a_fix() {
    // A box without its top, imported from the project folder: the card
    // shows the import's settings and the warning with its Clean up button.
    let dir = std::env::temp_dir().join(format!("p3d-snapshot-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
               f 1 4 3 2\nf 1 2 6 5\nf 3 4 8 7\nf 1 5 8 4\nf 2 3 7 6\n";
    std::fs::write(dir.join("bin.obj"), obj).unwrap();
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = ThemePref::Light;
    a.run("file.save", json!({ "path": dir.join("scene.p3d").display().to_string() })).unwrap();
    a.run("io.import", json!({ "path": "bin.obj" })).unwrap();
    let mut h = harness(a);
    snapshot(&mut h, "import_light");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_result_card_grows_to_show_new_warnings() {
    // The card first shows a boolean's settings (closed), then an import
    // with a warning: it must grow so the warning's Fix button is on screen.
    let dir = std::env::temp_dir().join(format!("p3d-snapshot-grow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj = "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
               f 1 4 3 2\nf 1 2 6 5\nf 3 4 8 7\nf 1 5 8 4\nf 2 3 7 6\n";
    std::fs::write(dir.join("bin.obj"), obj).unwrap();
    let select = [("edit.select", json!({ "target": "barrel" })), ("edit.select", json!({ "target": "crate", "add": true }))];
    let mut a = app(ThemePref::Dark, &select);
    a.run("file.save", json!({ "path": dir.join("scene.p3d").display().to_string() })).unwrap();
    a.run("selection.boolean", json!({ "operation": "union" })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("io.import", json!({ "path": "bin.obj" })).unwrap();
    h.run_steps(30);
    let fix = p3d_ui::control::widgets(&h.ctx).into_iter().find(|w| w.label == "Fix: Clean up").expect("the Fix button is drawn");
    let view = h.state().viewport.rect;
    assert!(view.contains_rect(fix.rect), "the button {:?} is inside the viewport {view:?}", fix.rect);
    let _ = std::fs::remove_dir_all(dir);
}

/// An SDF object from Add > SDF Shape with its head selected in the
/// Shapes tab, framed in the viewport.
fn sdf_app(theme: ThemePref) -> Harness<'static, PixelApp> {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = theme;
    a.run(
        "shapes.new",
        json!({ "name": "creature", "detail": 0.03, "shapes": [
            { "name": "body", "ellipsoid": { "at": [0, 0.35, 0], "radii": [0.3, 0.35, 0.25] } },
            { "name": "head", "smooth_union": 0.08, "sphere": { "at": [0, 0.82, 0], "radius": 0.18 } },
            { "name": "ear", "smooth_union": 0.03, "mirror": "x", "capsule": { "a": [0.1, 0.95, 0], "b": [0.2, 1.1, 0], "radius": 0.04, "radius_b": 0.01 } }
        ] }),
    )
    .unwrap();
    let mut h = harness(a);
    h.state_mut().run("shapes.select", json!({ "name": "head" })).unwrap();
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.run_steps(20);
    h
}

#[test]
fn sdf_shapes_panel() {
    let mut h = sdf_app(ThemePref::Dark);
    snapshot(&mut h, "sdf_panel_dark");
}

#[test]
fn sdf_proxies_and_gizmo() {
    let mut h = sdf_app(ThemePref::Light);
    h.state_mut().run("ui.tool", json!({ "tool": "move" })).unwrap();
    h.run_steps(10);
    snapshot(&mut h, "sdf_proxies_light");
}

#[test]
fn remesh_run_card() {
    let mut h = harness(app(ThemePref::Dark, &[("job.open", json!({ "op": "remesh.quad" }))]));
    snapshot(&mut h, "remesh_card_dark");
}

#[test]
fn pole_overlay() {
    let mut h = harness(app(ThemePref::Light, &[("view.poles", json!({ "on": true })), ("view.frame", Value::Null)]));
    snapshot(&mut h, "poles_light");
}

/// A spec starter from Add > Model Spec, framed, with `part` open in the
/// Spec tab.
fn spec_app(theme: ThemePref, starter: &str, part: &str) -> Harness<'static, PixelApp> {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = theme;
    a.run("spec.starter", json!({ "name": starter })).unwrap();
    // Room for the Spec tab: no Health panel, no History and Console.
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.bottom", json!({ "on": false })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.state_mut().run("spec.select_part", json!({ "part": part })).unwrap();
    h.run_steps(20);
    h
}

#[test]
fn spec_tab_with_an_anchored_part() {
    let mut h = spec_app(ThemePref::Dark, "elf_head", "eye_l");
    snapshot(&mut h, "spec_tab_dark");
}

#[test]
fn spec_tab_with_an_sdf_part() {
    let mut h = spec_app(ThemePref::Light, "elf_head", "head");
    snapshot(&mut h, "spec_sdf_light");
}

#[test]
fn landmark_overlay_while_picking() {
    let mut h = spec_app(ThemePref::Light, "lamp", "shade");
    h.state_mut().run("spec.pick_landmark", json!({ "on": true })).unwrap();
    h.run_steps(10);
    snapshot(&mut h, "spec_landmarks_light");
}

/// An outline starter from Add > Outline, framed, with `point` selected in
/// the Outline tab.
fn outline_app(theme: ThemePref, starter: &str, point: usize) -> Harness<'static, PixelApp> {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = theme;
    a.run("outline.new", json!({ "starter": starter })).unwrap();
    // Room for the canvas: no Health panel, no History and Console.
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.bottom", json!({ "on": false })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.state_mut().run("outline.select_point", json!({ "index": point })).unwrap();
    h.run_steps(20);
    h
}

#[test]
fn outline_tab_for_a_shell() {
    let mut h = outline_app(ThemePref::Dark, "leaf", 0);
    snapshot(&mut h, "outline_shell_dark");
}

#[test]
fn outline_tab_and_blueprint_for_silhouettes() {
    let mut h = outline_app(ThemePref::Light, "axe_head", 2);
    snapshot(&mut h, "outline_silhouettes_light");
}
