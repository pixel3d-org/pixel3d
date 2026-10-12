//! Edit workloads shared by the `edit` and `scale` benches.

use p3d_geo::edit::{
    Axis, ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, Keep, PlaneCutSpec, QuadSpec, SmoothSpec, ThickenSpec, dissolve_limited, dissolve_verts,
    extrude, inset, loop_cut, merge_by_distance, plane_cut, poke, smooth, split, subdivide, symmetrize, thicken, triangulate, tris_to_quads,
};
use p3d_mesh::Mesh;
use p3d_testdata::{grid, sphere, terrain};

/// Workload names with their mesh and selected faces (edges for loop cuts), for a grid of about `tris` triangles.
pub fn workloads(tris: usize) -> Vec<(&'static str, Mesh, Vec<u32>)> {
    let g = grid(tris);
    let n = g.face_count();
    let half: Vec<u32> = g.faces().take(n / 2).collect();
    let quarter: Vec<u32> = g.faces().step_by(4).collect();
    let all: Vec<u32> = g.faces().collect();
    let mut tri_grid = g.clone();
    let _ = triangulate(&mut tri_grid, &all);
    let all_tris: Vec<u32> = tri_grid.faces().collect();
    vec![
        ("extrude_region_half", g.clone(), half),
        ("extrude_individual_quarter", g.clone(), quarter.clone()),
        ("inset_region_all", g.clone(), all.clone()),
        ("inset_individual_quarter", g.clone(), quarter.clone()),
        ("poke_quarter", g.clone(), quarter),
        ("triangulate_all", g, all),
        ("tris_to_quads_all", tri_grid, all_tris),
        ("split_half", grid(tris), half_again(tris)),
        ("subdivide_quarter", grid(tris), quarter_again(tris)),
        ("loop_cut_rows", grid(tris), left_column_edges(tris)),
        ("weld_checkerboard_split", split_checkerboard(tris), Vec::new()),
        ("dissolve_limited_all", grid(tris), grid(tris).faces().collect()),
        ("dissolve_inner_verts", grid(tris), inner_verts(tris)),
        ("plane_cut_sphere", sphere(tris), Vec::new()),
        ("symmetrize_sphere", sphere(tris), Vec::new()),
        ("thicken_grid", grid(tris), grid(tris).faces().collect()),
        ("smooth_terrain", terrain(tris, 3), terrain(tris, 3).verts().collect()),
    ]
}

/// Runs one workload; returns the number of faces it made.
pub fn run(name: &str, mut m: Mesh, faces: &[u32]) -> usize {
    match name {
        "extrude_region_half" => extrude(&mut m, faces, &ExtrudeSpec { distance: 0.01, ..ExtrudeSpec::default() }).map_or(0, |r| r.side.len()),
        "extrude_individual_quarter" => {
            extrude(&mut m, faces, &ExtrudeSpec { mode: ExtrudeMode::Individual, distance: 0.01, direction: None }).map_or(0, |r| r.side.len())
        }
        "inset_region_all" => inset(&mut m, faces, &InsetSpec { thickness: 1e-4, ..InsetSpec::default() }).map_or(0, |r| r.rim.len()),
        "inset_individual_quarter" => {
            inset(&mut m, faces, &InsetSpec { mode: InsetMode::Individual, thickness: 1e-4, depth: 0.0 }).map_or(0, |r| r.rim.len())
        }
        "poke_quarter" => poke(&mut m, faces, 0.01).map_or(0, |t| t.len()),
        "triangulate_all" => triangulate(&mut m, faces).map_or(0, |t| t.0.len()),
        "tris_to_quads_all" => tris_to_quads(&mut m, faces, &QuadSpec::default()).map_or(0, |q| q.len()),
        "split_half" => split(&mut m, faces).map_or(0, |r| r.duplicated),
        "subdivide_quarter" => subdivide(&mut m, faces, 1).map_or(0, |r| r.faces.len()),
        "loop_cut_rows" => loop_cut(&mut m, faces, 1, 0.0).map_or(0, |r| r.faces.len()),
        "weld_checkerboard_split" => merge_by_distance(&mut m, None, 1e-5).map_or(0, |r| r.merged),
        "dissolve_limited_all" => dissolve_limited(&mut m, faces, 5.0).map_or(0, |r| r.removed_verts),
        "dissolve_inner_verts" => dissolve_verts(&mut m, faces).map_or(0, |r| r.removed_verts),
        "plane_cut_sphere" => {
            let spec = PlaneCutSpec { point: glam::Vec3::new(0.0, 0.1, 0.0), normal: glam::Vec3::new(0.3, 1.0, 0.2), keep: Keep::Above, fill: true };
            plane_cut(&mut m, &spec).map_or(0, |r| r.faces_cut)
        }
        "symmetrize_sphere" => symmetrize(&mut m, Axis::X, true, 1e-5).map_or(0, |r| r.mirrored.len()),
        "thicken_grid" => thicken(&mut m, faces, &ThickenSpec::default()).map_or(0, |r| r.inner.len()),
        _ => smooth(&mut m, faces, &SmoothSpec { iterations: 4, ..SmoothSpec::default() }).unwrap_or(0),
    }
}

/// The first half of a grid's faces.
fn half_again(tris: usize) -> Vec<u32> {
    let g = grid(tris);
    g.faces().take(g.face_count() / 2).collect()
}

/// Every fourth face of a grid.
fn quarter_again(tris: usize) -> Vec<u32> {
    grid(tris).faces().step_by(4).collect()
}

/// The edges along the left side of a grid: one per row of quads, each
/// starting its own ring.
fn left_column_edges(tris: usize) -> Vec<u32> {
    let g = grid(tris);
    g.edges().filter(|&e| g.edge_verts(e).iter().all(|&v| g.positions[v as usize].x < -0.4999)).collect()
}

/// A grid with every other face split off: duplicated vertices to weld back.
fn split_checkerboard(tris: usize) -> Mesh {
    let mut g = grid(tris);
    let every_other: Vec<u32> = g.faces().step_by(2).collect();
    let _ = split(&mut g, &every_other);
    g
}

/// Every other inner vertex of a grid (in id order).
fn inner_verts(tris: usize) -> Vec<u32> {
    let g = grid(tris);
    g.verts().filter(|&v| !g.is_boundary_vert(v)).step_by(2).collect()
}
