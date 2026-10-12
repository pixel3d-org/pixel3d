//! Procedural test meshes and fixtures.
//!
//! Support crate (dev-dependency only). See `docs/architecture.md`.
//!
//! - Sized meshes for benchmarks: [`grid`], [`sphere`], [`beveled_box`],
//!   [`terrain`], [`tube`], each sized by an approximate triangle count.
//! - Broken polygon soups that the kernel must reject: [`broken`].
//! - Small scenes for perception and agent tests: [`scenes`].
//! - proptest strategies for random valid meshes: [`strategy`].
//! - Problem sizes for the complexity scaling check: [`scale_sizes`].

use glam::{DVec3, Vec3};
use p3d_geo::prim::{BoxSpec, CapKind, CylinderSpec, Origin, PlaneSpec, QuadSphereSpec, cuboid, cylinder, plane, quad_sphere};
use p3d_mesh::Mesh;

pub mod broken;
pub mod scenes;
pub mod strategy;

/// Standard benchmark sizes in triangles.
pub const SIZES: [usize; 4] = [1_000, 10_000, 100_000, 1_000_000];

/// Square grid in the XZ plane with about `tris` triangles (1 m wide).
///
/// Complexity: O(tris).
pub fn grid(tris: usize) -> Mesh {
    let n = ((tris as f64 / 2.0).sqrt().round() as u32).max(1);
    plane(&PlaneSpec { size_x: 1.0, size_z: 1.0, segments_x: n, segments_z: n }).unwrap_or_default()
}

/// Quad sphere of radius 0.5 m with about `tris` triangles.
///
/// Complexity: O(tris).
pub fn sphere(tris: usize) -> Mesh {
    let n = ((tris as f64 / 12.0).sqrt().round() as u32).clamp(1, 1024);
    quad_sphere(&QuadSphereSpec { radius: 0.5, segments: n }, Origin::Center).unwrap_or_default()
}

/// Hard-surface box (1 x 0.6 x 0.8 m) with a 3-segment bevel and about
/// `tris` triangles.
///
/// Complexity: O(tris).
pub fn beveled_box(tris: usize) -> Mesh {
    // Flat faces dominate: about 4 (sx sy + sy sz + sz sx) triangles.
    let n = ((tris as f64 / 12.0).sqrt().round() as u32).clamp(1, 2048);
    let spec = BoxSpec { size: DVec3::new(1.0, 0.6, 0.8), segments: [n; 3], bevel_width: 0.05, bevel_segments: 3 };
    cuboid(&spec, Origin::Bottom).unwrap_or_default()
}

/// Grid with deterministic rolling hills (heights up to 0.1 m) and about
/// `tris` triangles; `seed` shifts the pattern.
///
/// Complexity: O(tris).
pub fn terrain(tris: usize, seed: u64) -> Mesh {
    let mut m = grid(tris);
    let phase = (seed % 1000) as f32 * 0.37;
    for p in &mut m.positions {
        let h = 0.05 * ((p.x * 9.0 + phase).sin() * (p.z * 7.0 - phase).cos()) + 0.05 * ((p.x + p.z) * 23.0 + phase).sin() * 0.3;
        p.y = h;
    }
    m
}

/// Open tube (cylinder without caps), 1 m tall, about `tris` triangles.
///
/// Complexity: O(tris).
pub fn tube(tris: usize) -> Mesh {
    let segments = ((tris as f64 / 2.0).sqrt().round() as u32).clamp(3, 4096);
    let rows = ((tris as f64 / 2.0 / f64::from(segments)).round() as u32).clamp(1, 4096);
    cylinder(&CylinderSpec { radius: 0.2, height: 1.0, segments, height_segments: rows, cap: CapKind::None }, Origin::Bottom).unwrap_or_default()
}

/// Problem sizes `n, 4n, 16n` for the scaling check. Small by default; set
/// `P3D_BENCH_SCALE=large` for the nightly sizes.
pub fn scale_sizes(small: usize, large: usize) -> [usize; 3] {
    let n = if std::env::var("P3D_BENCH_SCALE").is_ok_and(|v| v == "large") { large } else { small };
    [n, 4 * n, 16 * n]
}

/// Center of a mesh's bounding box (origin when empty).
pub fn center(m: &Mesh) -> Vec3 {
    m.bounds().map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_close_to_requested() {
        for &t in &SIZES[..3] {
            for (name, m) in [("grid", grid(t)), ("sphere", sphere(t)), ("box", beveled_box(t)), ("terrain", terrain(t, 1)), ("tube", tube(t))] {
                m.validate().unwrap();
                let got = m.triangle_count() as f64;
                assert!(got > 0.5 * t as f64 && got < 2.0 * t as f64, "{name}: {got} triangles for {t}");
            }
        }
    }

    #[test]
    fn terrain_is_deterministic() {
        assert_eq!(terrain(2000, 7).positions, terrain(2000, 7).positions);
        assert_ne!(terrain(2000, 7).positions, terrain(2000, 8).positions);
    }
}
