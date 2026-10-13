//! The interactive viewport path, rendered headless: background, surfaces,
//! grid, selection outline, picking, per-part transforms, x-ray and the
//! element overlay.
//!
//! These tests need a GPU or a software adapter (CI installs lavapipe on
//! Linux and uses WARP on Windows).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, OnceLock};

use glam::{Mat4, Vec3};
use p3d_render::viewport::linear_hex;
use p3d_render::{
    ElementSets, Image, OrbitCamera, Renderer, Shading, ViewFrame, ViewKind, ViewScene, ViewStyle, ViewportRenderer, edit_overlay, part_matrices,
    prepare_view, view_meshes, world_bounds,
};
use p3d_scene::Document;
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

/// A document prepared for the viewport: geometry, matrices, bounds and names.
struct Scene {
    view: ViewScene,
    models: Vec<Mat4>,
    lo: Vec3,
    hi: Vec3,
    names: Vec<String>,
}

fn scene(doc: &Document) -> Scene {
    let view = prepare_view(&view_meshes(doc), &Default::default());
    let models = part_matrices(doc, &view.parts);
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for (p, m) in view.parts.iter().zip(&models) {
        let (a, b) = world_bounds(p, *m);
        lo = lo.min(a);
        hi = hi.max(b);
    }
    if view.parts.is_empty() {
        (lo, hi) = (Vec3::ZERO, Vec3::ONE);
    }
    let names = view.parts.iter().map(|p| doc.object(p.object).map(|o| o.name().to_owned()).unwrap_or_default()).collect();
    Scene { view, models, lo, hi, names }
}

fn frame(s: &Scene, view: ViewKind) -> ViewFrame {
    let mut camera = OrbitCamera::default();
    camera.set_view(view);
    camera.frame(s.lo, s.hi, 4.0 / 3.0);
    ViewFrame {
        width: 160,
        height: 120,
        camera,
        shading: Shading::Solid,
        grid: false,
        style: ViewStyle::default(),
        ids: false,
        pick: None,
        xray: None,
    }
}

fn shot_with(v: &mut ViewportRenderer, s: &Scene, models: &[Mat4], key: u64, f: &ViewFrame) -> Image {
    let r = gpu();
    v.upload(r.device(), &s.view, key);
    v.set_transforms(r.device(), r.queue(), models);
    v.snapshot(r.device(), r.queue(), f).unwrap()
}

fn shot(v: &mut ViewportRenderer, s: &Scene, key: u64, f: &ViewFrame) -> Image {
    shot_with(v, s, &s.models, key, f)
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

fn differing(a: &Image, b: &Image) -> usize {
    (0..a.height).flat_map(|y| (0..a.width).map(move |x| (x, y))).filter(|&(x, y)| a.get(x, y) != b.get(x, y)).count()
}

#[test]
fn background_surfaces_and_determinism() {
    let s = scene(&scenes::crate_scene());
    let mut v = viewport();
    let f = frame(&s, ViewKind::ThreeQuarter);
    let img = shot(&mut v, &s, 1, &f);
    let style = ViewStyle::default();
    // The gradient runs from the top color to the bottom color.
    assert!(near(img.get(1, 0), srgb(style.bg_top), 3), "top {:?}", img.get(1, 0));
    assert!(near(img.get(1, img.height - 1), srgb(style.bg_bottom), 3), "bottom {:?}", img.get(1, img.height - 1));
    // The framed crate covers the middle of the image.
    let mid = img.get(img.width / 2, img.height / 2);
    assert!(!near(mid, srgb(style.bg_top), 12) && !near(mid, srgb(style.bg_bottom), 12), "center {mid:?}");
    // Same input, same pixels.
    assert_eq!(shot(&mut v, &s, 1, &f), img);
    // Wire mode draws dark edges on paper surfaces: the image changes.
    let wire = shot(&mut v, &s, 1, &ViewFrame { shading: Shading::Wire, ..f });
    assert_ne!(wire, img);
}

#[test]
fn grid_draws_on_the_ground_only() {
    let s = scene(&scenes::crate_scene());
    let mut v = viewport();
    let f = frame(&s, ViewKind::ThreeQuarter);
    let without = shot(&mut v, &s, 1, &f);
    let with = shot(&mut v, &s, 1, &ViewFrame { grid: true, ..f });
    assert!(differing(&with, &without) > 50, "the grid shows");
    // Looking level from the front, the sky above the horizon stays untouched.
    let level = frame(&s, ViewKind::Front);
    let without = shot(&mut v, &s, 1, &level);
    let with = shot(&mut v, &s, 1, &ViewFrame { grid: true, ..level });
    let rows = |from: u32, to: u32| {
        (from..to).flat_map(|y| (0..with.width).map(move |x| (x, y))).filter(|&(x, y)| with.get(x, y) != without.get(x, y)).count()
    };
    assert_eq!(rows(0, with.height / 2 - 2), 0, "nothing above the horizon");
    assert!(rows(with.height / 2 + 2, with.height) > 0, "lines below it");
}

#[test]
fn selection_outline_and_tint() {
    let s = scene(&scenes::crate_scene());
    let r = gpu();
    let mut v = viewport();
    let f = ViewFrame { ids: true, ..frame(&s, ViewKind::Front) };
    v.set_selection(r.device(), r.queue(), &[0]);
    let plain = shot(&mut v, &s, 1, &f);
    v.set_selection(r.device(), r.queue(), &[1]);
    let selected = shot(&mut v, &s, 1, &f);
    v.set_selection(r.device(), r.queue(), &[0]);
    let blue = srgb(linear_hex(0x4a8fff, 1.0));
    let count =
        |img: &Image| (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| near(img.get(x, y), blue, 10)).count();
    assert_eq!(count(&plain), 0, "nothing selected: no selection color");
    assert!(count(&selected) > 40, "outline pixels: {}", count(&selected));
}

#[test]
fn picks_return_the_part_under_the_pixel() {
    let s = scene(&scenes::crate_scene());
    let mut v = viewport();
    let base = frame(&s, ViewKind::Front);
    let slot = v.picks();
    // The middle of the front view is the crate body (part 0).
    let (cx, cy) = (base.width / 2, base.height / 2 + 10);
    shot(&mut v, &s, 1, &ViewFrame { pick: Some((cx, cy, 7)), ..base });
    let got = slot.lock().unwrap().take();
    let part = got.and_then(|p| p.part).map(|i| s.names[i as usize].clone());
    assert_eq!(got.map(|p| p.request), Some(7));
    assert_eq!(part.as_deref(), Some("crate"), "{got:?}");
    shot(&mut v, &s, 1, &ViewFrame { pick: Some((0, 0, 8)), ..base });
    assert_eq!(slot.lock().unwrap().take().map(|p| (p.request, p.part)), Some((8, None)), "background");
}

#[test]
fn transforms_move_parts_without_new_geometry() {
    let s = scene(&scenes::crate_scene());
    let mut v = viewport();
    let f = frame(&s, ViewKind::Front);
    let base = shot(&mut v, &s, 1, &f);
    // Mirroring the left-right symmetric crate across X keeps its picture:
    // normals and front faces follow the flipped winding.
    let mirror = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
    let mirrored: Vec<Mat4> = s.models.iter().map(|m| mirror * *m).collect();
    let img = shot_with(&mut v, &s, &mirrored, 1, &f);
    // Mirrored quads split along the other diagonal and edges rasterize a
    // little differently; a wrong normal or front face would change whole
    // faces by 30% (back faces shade darker).
    let far_off = (0..img.height)
        .flat_map(|y| (0..img.width).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get(x, y).iter().zip(base.get(x, y)).any(|(a, b)| a.abs_diff(b) > 24))
        .count();
    assert!(far_off < 120, "mirrored crate differs strongly in {far_off} px");
    // Moving every part right moves the picture; same geometry key.
    let moved: Vec<Mat4> = s.models.iter().map(|m| Mat4::from_translation(Vec3::X * 0.2) * *m).collect();
    let img = shot_with(&mut v, &s, &moved, 1, &f);
    assert!(differing(&img, &base) > 500);
    assert_eq!(v.key(), Some(1));
}

#[test]
fn xray_shows_what_is_behind() {
    let s = scene(&scenes::crate_scene());
    let mut v = viewport();
    let f = frame(&s, ViewKind::ThreeQuarter);
    let opaque = shot(&mut v, &s, 1, &f);
    let xray = shot(&mut v, &s, 1, &ViewFrame { xray: Some(0.5), ..f });
    assert!(differing(&xray, &opaque) > 500, "surfaces turn see-through");
    // The ground grid under the crate is hidden when opaque and shows
    // through in x-ray.
    let opaque_grid = shot(&mut v, &s, 1, &ViewFrame { grid: true, ..f });
    let xray_grid = shot(&mut v, &s, 1, &ViewFrame { grid: true, xray: Some(0.5), ..f });
    let through = (0..opaque.height)
        .flat_map(|y| (0..opaque.width).map(move |x| (x, y)))
        .filter(|&(x, y)| opaque_grid.get(x, y) == opaque.get(x, y) && xray_grid.get(x, y) != xray.get(x, y))
        .count();
    assert!(through > 20, "grid pixels seen through the crate: {through}");
    // Same input, same pixels.
    assert_eq!(shot(&mut v, &s, 1, &ViewFrame { xray: Some(0.5), ..f }), xray);
}

#[test]
fn element_overlay_draws_edges_dots_and_selected_faces() {
    let s = scene(&scenes::crate_scene());
    let r = gpu();
    let mut v = viewport();
    let f = frame(&s, ViewKind::Front);
    let plain = shot(&mut v, &s, 1, &f);
    let mesh = std::sync::Arc::clone(&s.view.meshes[0]);
    let none = std::collections::BTreeSet::new();
    let all_verts = mesh.verts().collect();
    let all_faces = mesh.faces().collect();
    let overlay = edit_overlay(&mesh, 0, ElementSets { verts: &all_verts, edges: &none, faces: &none }, true);
    assert_eq!(overlay.lines.len(), mesh.edge_count() * 2);
    assert_eq!(overlay.points.len(), mesh.vert_count());
    v.set_edit(r.device(), &overlay, 1);
    let dots = shot(&mut v, &s, 1, &f);
    let orange = srgb(linear_hex(0xfb923c, 1.0));
    let count =
        |img: &Image| (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| near(img.get(x, y), orange, 12)).count();
    assert!(count(&dots) > 10, "selected vertex dots: {}", count(&dots));
    assert_eq!(count(&plain), 0);
    // Selected faces are tinted, not solid: a different picture, few exact dots.
    let faces = edit_overlay(&mesh, 0, ElementSets { verts: &none, edges: &none, faces: &all_faces }, false);
    v.set_edit(r.device(), &faces, 2);
    let tinted = shot(&mut v, &s, 1, &f);
    assert!(differing(&tinted, &plain) > 500);
    // The same key does not upload again; clearing removes the overlay.
    v.set_edit(r.device(), &overlay, 2);
    assert_eq!(shot(&mut v, &s, 1, &f), tinted);
    v.clear_edit();
    assert_eq!(shot(&mut v, &s, 1, &f), plain);
}

#[test]
fn empty_scenes_draw_background_and_grid() {
    let s = scene(&Document::new());
    let mut v = viewport();
    let f = ViewFrame { grid: true, ..frame(&s, ViewKind::ThreeQuarter) };
    let img = shot(&mut v, &s, 2, &f);
    assert_eq!((img.width, img.height), (160, 120));
    assert!(v.key() == Some(2));
}
