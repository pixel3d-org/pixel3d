//! The interactive viewport path, rendered headless: background, surfaces,
//! grid, selection outline and picking.
//!
//! These tests need a GPU or a software adapter (CI installs lavapipe on
//! Linux and uses WARP on Windows).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, OnceLock};

use p3d_render::viewport::linear_hex;
use p3d_render::{Image, OrbitCamera, RenderScene, Renderer, Shading, ViewFrame, ViewKind, ViewStyle, ViewportRenderer, prepare};
use p3d_testdata::scenes;

fn gpu() -> &'static Renderer {
    static R: OnceLock<Renderer> = OnceLock::new();
    R.get_or_init(|| Renderer::new().unwrap_or_else(|e| panic!("{e}")))
}

/// One viewport renderer per test binary (pipelines are slow to build); tests take turns.
fn viewport() -> std::sync::MutexGuard<'static, ViewportRenderer> {
    static V: OnceLock<Mutex<ViewportRenderer>> = OnceLock::new();
    let r = gpu();
    V.get_or_init(|| Mutex::new(ViewportRenderer::new(r.device(), p3d_render::wgpu::TextureFormat::Rgba8UnormSrgb, 1)))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn frame(scene: &RenderScene, view: ViewKind) -> ViewFrame {
    let mut camera = OrbitCamera::default();
    camera.set_view(view);
    camera.frame(scene.lo, scene.hi, 4.0 / 3.0);
    ViewFrame { width: 160, height: 120, camera, shading: Shading::Solid, grid: false, style: ViewStyle::default(), ids: false, pick: None }
}

fn shot(v: &mut ViewportRenderer, scene: &RenderScene, key: u64, f: &ViewFrame) -> Image {
    let r = gpu();
    v.upload(r.device(), scene, key);
    v.snapshot(r.device(), r.queue(), f).unwrap()
}

fn srgb(c: [f32; 4]) -> [u8; 3] {
    let e = |x: f32| {
        let s = if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u8
    };
    [e(c[0]), e(c[1]), e(c[2])]
}

fn near(a: [u8; 4], b: [u8; 3], tol: u8) -> bool {
    a[..3].iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

#[test]
fn background_surfaces_and_determinism() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let mut v = viewport();
    let f = frame(&scene, ViewKind::ThreeQuarter);
    let img = shot(&mut v, &scene, 1, &f);
    let style = ViewStyle::default();
    // The gradient runs from the top color to the bottom color.
    assert!(near(img.get(1, 0), srgb(style.bg_top), 3), "top {:?}", img.get(1, 0));
    assert!(near(img.get(1, img.height - 1), srgb(style.bg_bottom), 3), "bottom {:?}", img.get(1, img.height - 1));
    // The framed crate covers the middle of the image.
    let mid = img.get(img.width / 2, img.height / 2);
    assert!(!near(mid, srgb(style.bg_top), 12) && !near(mid, srgb(style.bg_bottom), 12), "center {mid:?}");
    // Same input, same pixels.
    assert_eq!(shot(&mut v, &scene, 1, &f), img);
    // Wire mode draws dark edges on paper surfaces: the image changes.
    let wire = shot(&mut v, &scene, 1, &ViewFrame { shading: Shading::Wire, ..f });
    assert_ne!(wire, img);
}

#[test]
fn grid_draws_on_the_ground_only() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let mut v = viewport();
    let f = frame(&scene, ViewKind::ThreeQuarter);
    let without = shot(&mut v, &scene, 1, &f);
    let with = shot(&mut v, &scene, 1, &ViewFrame { grid: true, ..f });
    let changed = (0..with.height).flat_map(|y| (0..with.width).map(move |x| (x, y))).filter(|&(x, y)| with.get(x, y) != without.get(x, y)).count();
    assert!(changed > 50, "the grid shows ({changed} px changed)");
    // Looking level from the front, the sky above the horizon stays untouched.
    let level = frame(&scene, ViewKind::Front);
    let without = shot(&mut v, &scene, 1, &level);
    let with = shot(&mut v, &scene, 1, &ViewFrame { grid: true, ..level });
    let rows = |from: u32, to: u32| {
        (from..to).flat_map(|y| (0..with.width).map(move |x| (x, y))).filter(|&(x, y)| with.get(x, y) != without.get(x, y)).count()
    };
    assert_eq!(rows(0, with.height / 2 - 2), 0, "nothing above the horizon");
    assert!(rows(with.height / 2 + 2, with.height) > 0, "lines below it");
}

#[test]
fn selection_outline_and_tint() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let r = gpu();
    let mut v = viewport();
    let f = ViewFrame { ids: true, ..frame(&scene, ViewKind::Front) };
    v.set_selection(r.device(), r.queue(), &[0]);
    let plain = shot(&mut v, &scene, 1, &f);
    v.set_selection(r.device(), r.queue(), &[1]);
    let selected = shot(&mut v, &scene, 1, &f);
    v.set_selection(r.device(), r.queue(), &[0]);
    let blue = srgb(linear_hex(0x4a8fff, 1.0));
    let count =
        |img: &Image| (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| near(img.get(x, y), blue, 10)).count();
    assert_eq!(count(&plain), 0, "nothing selected: no selection color");
    assert!(count(&selected) > 40, "outline pixels: {}", count(&selected));
}

#[test]
fn picks_return_the_part_under_the_pixel() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let mut v = viewport();
    let base = frame(&scene, ViewKind::Front);
    let slot = v.picks();
    // The middle of the front view is the crate body (part 0).
    let (cx, cy) = (base.width / 2, base.height / 2 + 10);
    shot(&mut v, &scene, 1, &ViewFrame { pick: Some((cx, cy, 7)), ..base });
    let got = slot.lock().unwrap().take();
    let part = got.and_then(|p| p.part).map(|i| scene.parts[i as usize].name.clone());
    assert_eq!(got.map(|p| p.request), Some(7));
    assert_eq!(part.as_deref(), Some("crate"), "{got:?}");
    shot(&mut v, &scene, 1, &ViewFrame { pick: Some((0, 0, 8)), ..base });
    assert_eq!(slot.lock().unwrap().take().map(|p| (p.request, p.part)), Some((8, None)), "background");
}

#[test]
fn empty_scenes_draw_background_and_grid() {
    let scene = RenderScene::default();
    let mut v = viewport();
    let f = ViewFrame { grid: true, ..frame(&scene, ViewKind::ThreeQuarter) };
    let img = shot(&mut v, &scene, 2, &f);
    assert_eq!((img.width, img.height), (160, 120));
    assert!(v.key() == Some(2));
}
