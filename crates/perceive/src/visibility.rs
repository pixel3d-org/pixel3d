//! The `visibility` aspect (project spec 17.7): how the asset reads at its
//! asset class's view distance on a 1080p screen. Per part: its size in
//! pixels, parts and walls too small to model (better as a normal map or a
//! texture), the share of its surface never seen from outside (wasted
//! triangles), the share of edges under a pixel (over-tessellated for the
//! view), its share of the outline, and its triangles against its share of
//! the screen (budget allocation).

use glam::Vec3;
use serde::Serialize;

use crate::fmt::thousands;
use crate::sample::surface_samples;
use crate::silhouette::{ViewDir, rasterize};
use crate::{Report, View};

/// Screen height and vertical field of view the pixel sizes assume.
pub const SCREEN_HEIGHT: f32 = 1080.0;
pub const FOV_Y_DEG: f32 = 60.0;
/// A part smaller than this many pixels reads as a speck.
const MIN_PART_PX: f32 = 8.0;
/// A wall thinner than this many pixels is detail for a normal map.
const MIN_WALL_PX: f32 = 2.0;
/// Surface samples over all parts for the hidden-surface estimate.
const HIDDEN_SAMPLES: usize = 4096;
/// Raster resolution for screen and outline shares.
const RESOLUTION: usize = 256;
/// Parts listed in the text.
const MAX_LINES: usize = 10;

/// Directions a surface point can be seen from: the 6 axes and the 8
/// diagonals.
fn directions() -> [Vec3; 14] {
    let d = 1.0 / 3f32.sqrt();
    [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::new(d, d, d),
        Vec3::new(d, d, -d),
        Vec3::new(d, -d, d),
        Vec3::new(d, -d, -d),
        Vec3::new(-d, d, d),
        Vec3::new(-d, d, -d),
        Vec3::new(-d, -d, d),
        Vec3::new(-d, -d, -d),
    ]
}

/// One part at view distance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PartVisibility {
    pub part: String,
    /// Largest and smallest extent in pixels.
    pub size_px: f32,
    pub thin_px: f32,
    pub tris: usize,
    /// Shares of the asset's triangles and of its screen area (front, side
    /// and top views).
    pub tri_share: f32,
    pub screen_share: f32,
    /// Share of the outline cells it forms (front view).
    pub outline_share: f32,
    /// Share of the surface never seen from outside.
    pub hidden: f32,
    /// Share of edges shorter than a pixel.
    pub sub_pixel_edges: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Visibility {
    pub asset_class: &'static str,
    pub distance: f32,
    pub px_per_m: f32,
    pub parts: Vec<PartVisibility>,
}

/// Pixels per meter at `distance` on the reference screen.
pub fn pixels_per_meter(distance: f32) -> f32 {
    SCREEN_HEIGHT / (2.0 * distance.max(1e-3) * (FOV_Y_DEG.to_radians() / 2.0).tan())
}

/// Measures every part of the view.
///
/// Complexity: O(triangles + edges) for counts and rasters, plus about
/// 4,096 samples x 14 directions x parts BVH rays for the hidden surface.
pub fn visibility(view: &View) -> Visibility {
    let class = view.doc.settings.asset_class;
    let distance = class.view_distance();
    let px = pixels_per_meter(distance);
    let n = view.parts.len();
    let total_tris: usize = view.parts.iter().map(|p| p.mesh.triangle_count()).sum::<usize>().max(1);
    // Screen area by owner over three views, and outline cells in front.
    let mut cells = vec![0usize; n];
    let mut outline = vec![0usize; n];
    for (k, dir) in [ViewDir::Front, ViewDir::Right, ViewDir::Top].into_iter().enumerate() {
        let r = rasterize(view, dir, RESOLUTION, None);
        for j in 0..r.h {
            for i in 0..r.w {
                let Some(o) = r.owner(i, j) else { continue };
                cells[o] += 1;
                if k == 0 {
                    let (ii, jj) = (i as isize, j as isize);
                    if !(r.filled(ii - 1, jj) && r.filled(ii + 1, jj) && r.filled(ii, jj - 1) && r.filled(ii, jj + 1)) {
                        outline[o] += 1;
                    }
                }
            }
        }
    }
    let all_cells = cells.iter().sum::<usize>().max(1) as f32;
    let all_outline = outline.iter().sum::<usize>().max(1) as f32;
    let per = (HIDDEN_SAMPLES / n.max(1)).max(32);
    let diag = (view.hi - view.lo).length().max(1e-6);
    let dirs = directions();
    let parts = view
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let size = p.size();
            // Hidden: a sample no direction reaches without hitting a part.
            let samples = surface_samples(p, per);
            let (mut hidden, mut area) = (0.0f32, 0.0f32);
            for s in &samples {
                area += s.weight;
                let start = s.point + s.normal * diag * 1e-4;
                let seen = dirs
                    .iter()
                    .filter(|d| d.dot(s.normal) > 0.05)
                    .any(|&d| view.parts.iter().all(|q| q.bvh().raycast(start, d, diag * 2.0).is_none()));
                if !seen {
                    hidden += s.weight;
                }
            }
            // Edges under a pixel, in world lengths.
            let m = p.mesh;
            let (mut short, mut edges) = (0usize, 0usize);
            for e in m.edges() {
                let [a, b] = m.edge_verts(e);
                let l = p.world.transform_point3(m.positions[a as usize]).distance(p.world.transform_point3(m.positions[b as usize]));
                edges += 1;
                if l * px < 1.0 {
                    short += 1;
                }
            }
            let tris = m.triangle_count();
            PartVisibility {
                part: p.label.clone(),
                size_px: size.max_element() * px,
                thin_px: size.min_element() * px,
                tris,
                tri_share: tris as f32 / total_tris as f32,
                screen_share: cells[i] as f32 / all_cells,
                outline_share: outline[i] as f32 / all_outline,
                hidden: if area > 0.0 { hidden / area } else { 0.0 },
                sub_pixel_edges: if edges > 0 { short as f32 / edges as f32 } else { 0.0 },
            }
        })
        .collect();
    Visibility { asset_class: class.as_str(), distance, px_per_m: px, parts }
}

fn pct(x: f32) -> String {
    format!("{}%", (x * 100.0).round() as i32)
}

/// The aspect: one line per part (most triangles first), with advice.
///
/// Complexity: as [`visibility`].
pub fn report(view: &View) -> Report {
    let v = visibility(view);
    let mut t =
        format!("visibility at {} m on 1080p ({}): 1 m = {} px\n", (v.distance * 10.0).round() / 10.0, v.asset_class, v.px_per_m.round() as i32);
    let mut order: Vec<&PartVisibility> = v.parts.iter().collect();
    order.sort_by(|a, b| b.tris.cmp(&a.tris).then(a.part.cmp(&b.part)));
    for p in order.iter().take(MAX_LINES) {
        t.push_str(&format!(
            "  {} {} px, {} tris ({} of tris, {} of screen, {} of outline)",
            p.part,
            p.size_px.round() as i32,
            thousands(p.tris),
            pct(p.tri_share),
            pct(p.screen_share),
            pct(p.outline_share)
        ));
        let mut notes = Vec::new();
        if p.size_px < MIN_PART_PX {
            notes.push(format!("under {MIN_PART_PX} px: a texture or normal map would do"));
        } else if p.thin_px < MIN_WALL_PX {
            notes.push(format!("{} px thin: detail for a normal map", (p.thin_px * 10.0).round() / 10.0));
        }
        if p.hidden > 0.25 {
            notes.push(format!("{} never seen", pct(p.hidden)));
        }
        if p.sub_pixel_edges > 0.3 {
            notes.push(format!("{} of edges under 1 px: decimate", pct(p.sub_pixel_edges)));
        }
        if p.tri_share > 0.2 && p.tri_share > 3.0 * p.screen_share.max(0.01) {
            notes.push("dense for its screen share".into());
        }
        if !notes.is_empty() {
            t.push_str(&format!(": {}", notes.join("; ")));
        }
        t.push('\n');
    }
    if order.len() > MAX_LINES {
        t.push_str(&format!("  ... {} more parts\n", order.len() - MAX_LINES));
    }
    Report::new("visibility", t, &v)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use glam::DVec3;
    use p3d_geo::prim::{BoxSpec, Origin, cuboid};
    use p3d_scene::{AssetClass, Document};

    use super::*;

    #[test]
    fn small_and_buried_parts_are_flagged() {
        let mut doc = Document::new();
        doc.settings.asset_class = AssetClass::Character;
        let body = cuboid(&BoxSpec { size: DVec3::splat(1.0), segments: [8; 3], ..BoxSpec::default() }, Origin::Center).unwrap();
        doc.create_mesh_object("body", None, body).unwrap();
        // A bolt 5 mm across: about 1 px at 5 m.
        let bolt = cuboid(&BoxSpec { size: DVec3::splat(0.005), ..BoxSpec::default() }, Origin::Center).unwrap();
        let b = doc.create_mesh_object("bolt", None, bolt).unwrap();
        doc.object_mut(b).unwrap().transform.translation = Vec3::new(0.0, 0.0, 0.503);
        // A core fully inside the body.
        let core = cuboid(&BoxSpec { size: DVec3::splat(0.3), ..BoxSpec::default() }, Origin::Center).unwrap();
        doc.create_mesh_object("core", None, core).unwrap();
        let view = View::new(&doc, None).unwrap();
        let v = visibility(&view);
        assert!((v.px_per_m - 187.0).abs() < 1.0, "{}", v.px_per_m);
        let get = |n: &str| v.parts.iter().find(|p| p.part == n).unwrap();
        assert!(get("bolt").size_px < MIN_PART_PX);
        assert!(get("core").hidden > 0.99, "{:?}", get("core"));
        assert!(get("body").hidden < 0.05, "{:?}", get("body"));
        let r = report(&view);
        assert!(r.text.contains("bolt 1 px") && r.text.contains("a texture or normal map would do"), "{}", r.text);
        assert!(r.text.contains("core") && r.text.contains("100% never seen"), "{}", r.text);
    }
}
