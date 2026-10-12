//! Edit workloads shared by the `edit` and `scale` benches.

use p3d_geo::edit::{
    ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, QuadSpec, extrude, inset, loop_cut, poke, split, subdivide, triangulate, tris_to_quads,
};
use p3d_mesh::Mesh;
use p3d_testdata::grid;

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
        _ => loop_cut(&mut m, faces, 1, 0.0).map_or(0, |r| r.faces.len()),
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
