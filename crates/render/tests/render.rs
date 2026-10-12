//! Headless renders: coverage matches the geometry, modes differ as
//! designed, results are deterministic, and golden images hold.
//!
//! These tests need a GPU or a software adapter (CI installs lavapipe on
//! Linux and uses WARP on Windows).

// Test helpers fail the test by panicking; clippy only exempts `#[test]` functions themselves.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::OnceLock;

use glam::Vec3;
use p3d_render::image::PAPER;
use p3d_render::{Image, Mode, RenderRequest, Renderer, ViewKind, camera, prepare};
use p3d_testdata::scenes;

fn renderer() -> &'static Renderer {
    static R: OnceLock<Renderer> = OnceLock::new();
    R.get_or_init(|| Renderer::new().unwrap_or_else(|e| panic!("{e}")))
}

fn one(view: ViewKind, mode: Mode, size: u32, labels: bool) -> RenderRequest {
    RenderRequest { views: vec![view], mode, size, sheet: false, labels }
}

/// Pixels that differ from the background.
fn covered(img: &Image) -> usize {
    let bg = img.get(0, 0);
    (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| img.get(x, y) != bg).count()
}

#[test]
fn coverage_matches_the_projected_area() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let size = 512;
    let img = &renderer().render(&scene, &one(ViewKind::Front, Mode::Shaded, size, false)).unwrap()[0].1;
    // Front outline: crate 600 x 600, lid 620 x 40 on top, handles 30 x 40 beside it.
    let area = 0.6 * 0.6 + 0.62 * 0.04 + 2.0 * 0.03 * 0.04;
    let cam = camera::frame(ViewKind::Front, scene.lo, scene.hi, 1.0, p3d_render::MARGIN);
    let (x0, _, _) = cam.to_pixel(Vec3::ZERO, size, size);
    let (x1, _, _) = cam.to_pixel(Vec3::X, size, size);
    let px_per_m = (x1 - x0) as f64;
    let expected = area * px_per_m * px_per_m;
    let got = covered(img) as f64;
    assert!((got - expected).abs() / expected < 0.02, "covered {got} px, expected {expected:.0}");
}

#[test]
fn renders_are_deterministic_and_modes_differ() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let r = renderer();
    let a = &r.render(&scene, &one(ViewKind::ThreeQuarter, Mode::Shaded, 128, true)).unwrap()[0].1;
    let b = &r.render(&scene, &one(ViewKind::ThreeQuarter, Mode::Shaded, 128, true)).unwrap()[0].1;
    assert_eq!(a, b, "same input, same pixels");
    let wire = &r.render(&scene, &one(ViewKind::ThreeQuarter, Mode::Wire, 128, false)).unwrap()[0].1;
    let matcap = &r.render(&scene, &one(ViewKind::ThreeQuarter, Mode::Matcap, 128, false)).unwrap()[0].1;
    let shaded = &r.render(&scene, &one(ViewKind::ThreeQuarter, Mode::Shaded, 128, false)).unwrap()[0].1;
    assert_ne!(wire, matcap);
    assert_ne!(matcap, shaded);
    // Wireframes are mostly paper with dark edges.
    let dark = (0..wire.height).flat_map(|y| (0..wire.width).map(move |x| (x, y))).filter(|&(x, y)| wire.get(x, y)[0] < 100).count();
    assert!(dark > 200, "{dark} dark edge pixels");
}

#[test]
fn inside_out_parts_show_red() {
    let (_, doc) = scenes::broken_corpus().into_iter().find(|(n, _)| *n == "inverted_normals").unwrap();
    let img = &renderer().render(&prepare(&doc, None), &one(ViewKind::ThreeQuarter, Mode::Shaded, 128, false)).unwrap()[0].1;
    let red = (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| {
        let p = img.get(x, y);
        p[0] > 150 && i32::from(p[0]) > i32::from(p[1]) + 80 && i32::from(p[0]) > i32::from(p[2]) + 70
    });
    assert!(red.count() > 2000, "inverted faces must read as red");
}

#[test]
fn sheets_and_labels() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    let out = renderer().render(&scene, &RenderRequest { size: 128, ..RenderRequest::default() }).unwrap();
    assert_eq!(out.len(), 1, "one contact sheet");
    let (title, sheet) = &out[0];
    assert_eq!(title, "sheet");
    assert_eq!((sheet.width, sheet.height), (2 * 128 + 2, 2 * 128 + 2));
    // Labels draw paper boxes with ink text: the labelled image has more paper.
    let plain = &renderer().render(&scene, &one(ViewKind::Front, Mode::Shaded, 256, false)).unwrap()[0].1;
    let labelled = &renderer().render(&scene, &one(ViewKind::Front, Mode::Shaded, 256, true)).unwrap()[0].1;
    let paper = |img: &Image| (0..img.height).flat_map(|y| (0..img.width).map(move |x| (x, y))).filter(|&(x, y)| img.get(x, y) == PAPER).count();
    assert!(paper(labelled) > paper(plain) + 100);
}

/// Compares with a golden PNG: mean absolute difference under 2 levels and
/// under 0.5% of pixels off by more than 48 levels (GPU drivers differ in
/// edge coverage). `P3D_UPDATE_GOLDEN=1` rewrites the goldens.
fn golden(name: &str, img: &Image) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"));
    if std::env::var("P3D_UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, img.to_png().unwrap()).unwrap();
        return;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run with P3D_UPDATE_GOLDEN=1 once)", path.display()));
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (img.width, img.height), "{name}: size");
    let (mut sum, mut off) = (0u64, 0usize);
    for (a, b) in buf.chunks(4).zip(img.pixels.chunks(4)) {
        let d = (0..3).map(|k| (i32::from(a[k]) - i32::from(b[k])).unsigned_abs()).max().unwrap_or(0);
        sum += u64::from(d);
        if d > 48 {
            off += 1;
        }
    }
    let n = (img.width * img.height) as usize;
    let mean = sum as f64 / n as f64;
    assert!(mean < 2.0 && (off as f64) < 0.005 * n as f64, "{name}: mean difference {mean:.2}, {off} pixels off");
}

#[test]
fn golden_images() {
    let doc = scenes::crate_scene();
    let scene = prepare(&doc, None);
    for mode in Mode::ALL {
        let img = &renderer().render(&scene, &one(ViewKind::ThreeQuarter, mode, 192, true)).unwrap()[0].1;
        golden(&format!("crate_three_quarter_{}", mode.name()), img);
    }
}
