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

#[test]
fn handles_tool_on_a_spec_part() {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = ThemePref::Dark;
    a.run("spec.starter", json!({ "name": "lamp" })).unwrap();
    a.run("edit.select", json!({ "target": "lamp/shade" })).unwrap();
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.bottom", json!({ "on": false })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.state_mut().run("ui.tool", json!({ "tool": "handles" })).unwrap();
    // A pulled handle kept on the part, and a placed one ready to pull.
    h.state_mut().run("handles.add", json!({ "at": [0.05, 0.57, 0.0], "by": [0, 0.03, 0] })).unwrap();
    for _ in 0..200 {
        h.run_steps(2);
        if !h.state().handles.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.state_mut().run("handles.add", json!({ "at": [-0.05, 0.57, 0.0] })).unwrap();
    let c = h.state().viewport.rect.center();
    h.input_mut().events.push(egui::Event::PointerMoved(c));
    h.run_steps(10);
    snapshot(&mut h, "handles_dark");
}

#[test]
fn history_bisect_card() {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = ThemePref::Light;
    a.run("spec.starter", json!({ "name": "lamp" })).unwrap();
    a.run("expect.add", json!({ "name": "E1", "expect": { "dims": { "target": "lamp", "height": { "max": 0.7 } } } })).unwrap();
    a.run("spec.patch", json!({ "target": "lamp", "merge": { "parts": { "pole": { "primitive": { "cylinder": { "height": 0.9 } } } } } })).unwrap();
    a.run("prim.box", json!({ "name": "crate", "position": [1, 0, 0] })).unwrap();
    a.run("history.bisect_card", json!({ "on": true })).unwrap();
    a.run("history.bisect", json!({ "test": { "expect": "E1" } })).unwrap();
    let mut h = harness(a);
    h.run_steps(10);
    snapshot(&mut h, "bisect_light");
}

/// The Features panel open below the Outliner with the ear template
/// selected and inserted; its landmarks show in the viewport.
fn features_app(theme: ThemePref) -> Harness<'static, PixelApp> {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = theme;
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.dock", json!({ "operation": "close", "panel": "properties" })).unwrap();
    a.run("features.insert", json!({ "feature": "ear", "params": { "variant": "elf" }, "open": true })).unwrap();
    a.run("view.feature_landmarks", json!({ "on": true })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.run_steps(20);
    h
}

#[test]
fn features_panel_dark() {
    let mut h = features_app(ThemePref::Dark);
    snapshot(&mut h, "features_dark");
}

#[test]
fn features_panel_light() {
    let mut h = features_app(ThemePref::Light);
    snapshot(&mut h, "features_light");
}

#[test]
fn spec_tab_with_a_feature_part() {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = ThemePref::Dark;
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
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.bottom", json!({ "on": false })).unwrap();
    a.run("view.feature_landmarks", json!({ "on": true })).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.state_mut().run("spec.select_part", json!({ "part": "ear_l" })).unwrap();
    h.run_steps(20);
    snapshot(&mut h, "spec_feature_dark");
}

/// A head with hair grown in a landmark loop, the Strands panel open on it.
fn strands_app(theme: ThemePref) -> Harness<'static, PixelApp> {
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.stable = true;
    a.ui.theme = theme;
    a.run("ui.dock", json!({ "operation": "close", "panel": "health" })).unwrap();
    a.run("ui.dock", json!({ "operation": "close", "panel": "properties" })).unwrap();
    a.run("gen.feature", json!({ "name": "head", "feature": "head_base", "detail": "5mm" })).unwrap();
    a.run("strands.start", json!({ "kind": "hair" })).unwrap();
    a.run("strands.region", json!({ "landmarks": ["hairline_front", "temple_l", "nape", "temple_r"] })).unwrap();
    a.run("strands.set", json!({ "length": 0.12 })).unwrap();
    a.run("strands.grow", Value::Null).unwrap();
    let mut h = harness(a);
    h.state_mut().run("view.frame", Value::Null).unwrap();
    h.run_steps(20);
    h
}

#[test]
fn strands_panel_dark() {
    let mut h = strands_app(ThemePref::Dark);
    snapshot(&mut h, "strands_dark");
}

#[test]
fn strands_panel_light() {
    let mut h = strands_app(ThemePref::Light);
    snapshot(&mut h, "strands_light");
}

/// Hair cards on a head, drawn by the viewport renderer itself (the GPU
/// picture the other snapshots leave out): two-sided, darker at the roots.
#[test]
fn hair_cards_on_a_head_in_the_viewport() {
    use p3d_render::{OrbitCamera, Renderer, Shading, ViewFrame, ViewKind, ViewStyle, ViewportRenderer};
    let mut a = PixelApp::new(Session::new(), Services::default());
    a.run("gen.feature", json!({ "name": "head", "feature": "head_base", "detail": "4mm" })).unwrap();
    a.run("gen.strands", json!({ "on": "head", "region": ["hairline_front", "temple_l", "nape", "temple_r"], "length": "12cm" })).unwrap();
    let doc = a.session.doc();
    let list = p3d_render::view_meshes(doc);
    let strands: std::collections::BTreeSet<_> =
        list.iter().filter(|(k, _)| doc.object(*k).is_some_and(|o| o.tags.iter().any(|t| t == "strands"))).map(|(k, _)| *k).collect();
    assert_eq!(strands.len(), 1);
    let view = p3d_render::prepare_view(&list, &strands);
    assert!(view.vertices.iter().any(|v| v.strand >= 0.0) && view.vertices.iter().any(|v| v.strand < 0.0));
    let models = p3d_render::part_matrices(doc, &view.parts);
    let (mut lo, mut hi) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
    for (p, m) in view.parts.iter().zip(&models) {
        let (a, b) = p3d_render::world_bounds(p, *m);
        lo = lo.min(a);
        hi = hi.max(b);
    }
    let r = Renderer::new().unwrap();
    let mut v = ViewportRenderer::new(r.device(), p3d_render::wgpu::TextureFormat::Rgba8UnormSrgb, 1);
    v.upload(r.device(), &view, 1);
    v.set_transforms(r.device(), r.queue(), &models);
    let mut camera = OrbitCamera::default();
    camera.set_view(ViewKind::ThreeQuarter);
    camera.frame(lo, hi, 4.0 / 3.0);
    let frame = ViewFrame {
        width: 480,
        height: 360,
        camera,
        shading: Shading::Solid,
        grid: false,
        style: ViewStyle::default(),
        ids: false,
        pick: None,
        xray: None,
    };
    let img = v.snapshot(r.device(), r.queue(), &frame).unwrap();
    let rgba = image::RgbaImage::from_raw(img.width, img.height, img.pixels).unwrap();
    egui_kittest::image_snapshot_options(&rgba, "hair_viewport", &options("hair_viewport"));
}
