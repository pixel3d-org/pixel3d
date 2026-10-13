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

/// Largest channel difference of two RGBA pixels.
fn pixel_diff(a: &[u8], b: &[u8]) -> u32 {
    (0..3).map(|k| (i32::from(a[k]) - i32::from(b[k])).unsigned_abs()).max().unwrap_or(0)
}

/// Mean difference and pixels off by more than 48 levels, forgiving
/// one-pixel shifts: each pixel matches its best neighbour within 3 x 3 in
/// the other image, both ways. GPU rasterizers (Metal, WARP, lavapipe)
/// place thin lines and edge coverage a pixel apart; a missing or extra
/// edge still differs everywhere along it.
///
/// Complexity: O(w h) (18 lookups per pixel).
fn compare(a: &[u8], b: &[u8], w: usize, h: usize) -> (f64, usize) {
    let at = |img: &[u8], x: usize, y: usize| -> [u8; 4] {
        let i = (y * w + x) * 4;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    };
    let best = |p: [u8; 4], other: &[u8], x: usize, y: usize| -> u32 {
        let mut d = u32::MAX;
        for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
            for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                d = d.min(pixel_diff(&p, &at(other, nx, ny)));
            }
        }
        d
    };
    let (mut sum, mut off) = (0u64, 0usize);
    for y in 0..h {
        for x in 0..w {
            let d = best(at(a, x, y), b, x, y).max(best(at(b, x, y), a, x, y));
            sum += u64::from(d);
            off += usize::from(d > 48);
        }
    }
    (sum as f64 / (w * h) as f64, off)
}

/// The goldens' tolerance: mean difference under 2 levels and under 0.5%
/// of pixels off.
fn close_enough((mean, off): (f64, usize), n: usize) -> bool {
    mean < 2.0 && (off as f64) < 0.005 * n as f64
}

fn read_png(path: &std::path::Path) -> (Vec<u8>, u32, u32) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e} (run with P3D_UPDATE_GOLDEN=1 once)", path.display()));
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    (buf, info.width, info.height)
}

/// Path of a golden (or of its `.new` render): `tests/golden/<os>/<name>.png`
/// when that baseline exists, otherwise `tests/golden/<name>.png`. `<os>`
/// is `std::env::consts::OS`; rasterizers differ in thin-line coverage (WARP
/// on Windows), so a platform may carry its own baseline, taken from a CI
/// artifact after checking it shows the same picture.
fn golden_path(name: &str, suffix: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let os = dir.join(std::env::consts::OS);
    let base = if os.join(format!("{name}.png")).exists() { os } else { dir };
    base.join(format!("{name}{suffix}.png"))
}

/// Compares with a golden PNG ([`compare`], [`close_enough`]). A failing
/// render is written next to it as `<name>.new.png` (CI keeps it as an
/// artifact). `P3D_UPDATE_GOLDEN=1` rewrites the goldens.
fn golden(name: &str, img: &Image) {
    let path = golden_path(name, "");
    if std::env::var("P3D_UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, img.to_png().unwrap()).unwrap();
        return;
    }
    let (buf, w, h) = read_png(&path);
    assert_eq!((w, h), (img.width, img.height), "{name}: size");
    let (mean, off) = compare(&buf, &img.pixels, w as usize, h as usize);
    if !close_enough((mean, off), (w * h) as usize) {
        let _ = std::fs::write(golden_path(name, ".new"), img.to_png().unwrap());
        panic!("{name}: mean difference {mean:.2}, {off} pixels off (see {name}.new.png)");
    }
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

/// The tolerance forgives a wireframe shifted by a pixel, not another
/// picture or a missing edge.
#[test]
fn golden_tolerance_forgives_shifts_only() {
    let (wire, w, h) = read_png(&golden_path("crate_three_quarter_wire", ""));
    let (w, h) = (w as usize, h as usize);
    let n = w * h;
    // Shift right by one pixel.
    let mut shifted = wire.clone();
    for y in 0..h {
        for x in (1..w).rev() {
            let (to, from) = ((y * w + x) * 4, (y * w + x - 1) * 4);
            shifted.copy_within(from..from + 4, to);
        }
    }
    assert!(close_enough(compare(&wire, &shifted, w, h), n));
    let (matcap, _, _) = read_png(&golden_path("crate_three_quarter_matcap", ""));
    assert!(!close_enough(compare(&wire, &matcap, w, h), n));
    // Edges missing from the left half.
    let mut erased = wire.clone();
    let paper = [wire[0], wire[1], wire[2], wire[3]];
    let mut dark = 0;
    for y in 0..h {
        for x in 0..w / 2 {
            let i = (y * w + x) * 4;
            if erased[i] < 100 {
                erased[i..i + 4].copy_from_slice(&paper);
                dark += 1;
            }
        }
    }
    assert!(dark > 100, "{dark}");
    let (mean, off) = compare(&wire, &erased, w, h);
    assert!(!close_enough((mean, off), n), "mean {mean:.2}, {off} off");
}

/// Every platform baseline (`tests/golden/<os>/`) shows the same picture as
/// the shared golden: rasterizer differences only, never a different render.
#[test]
fn platform_goldens_match_the_shared_ones() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for os in ["windows", "linux", "macos"] {
        let Ok(entries) = std::fs::read_dir(dir.join(os)) else { continue };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".png") || name.ends_with(".new.png") {
                continue;
            }
            let (a, w, h) = read_png(&e.path());
            let (b, w2, h2) = read_png(&dir.join(&name));
            assert_eq!((w, h), (w2, h2), "{os}/{name}: size");
            let (mean, off) = compare(&a, &b, w as usize, h as usize);
            assert!(mean < 2.0, "{os}/{name}: mean difference {mean:.2} ({off} pixels off) is another picture");
        }
    }
}
