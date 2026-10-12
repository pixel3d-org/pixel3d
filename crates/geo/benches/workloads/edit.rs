//! Edit workloads shared by the `edit` and `scale` benches.

use p3d_geo::edit::{ExtrudeMode, ExtrudeSpec, InsetMode, InsetSpec, extrude, inset};
use p3d_mesh::Mesh;
use p3d_testdata::grid;

/// Workload names with their mesh and selected faces, for a grid of about `tris` triangles.
pub fn workloads(tris: usize) -> Vec<(&'static str, Mesh, Vec<u32>)> {
    let g = grid(tris);
    let n = g.face_count();
    let half: Vec<u32> = g.faces().take(n / 2).collect();
    let quarter: Vec<u32> = g.faces().step_by(4).collect();
    let all: Vec<u32> = g.faces().collect();
    vec![
        ("extrude_region_half", g.clone(), half),
        ("extrude_individual_quarter", g.clone(), quarter.clone()),
        ("inset_region_all", g.clone(), all),
        ("inset_individual_quarter", g, quarter),
    ]
}

/// Runs one workload; returns the number of new walls or rim faces.
pub fn run(name: &str, mut m: Mesh, faces: &[u32]) -> usize {
    match name {
        "extrude_region_half" => extrude(&mut m, faces, &ExtrudeSpec { distance: 0.01, ..ExtrudeSpec::default() }).map_or(0, |r| r.side.len()),
        "extrude_individual_quarter" => {
            extrude(&mut m, faces, &ExtrudeSpec { mode: ExtrudeMode::Individual, distance: 0.01, direction: None }).map_or(0, |r| r.side.len())
        }
        "inset_region_all" => inset(&mut m, faces, &InsetSpec { thickness: 1e-4, ..InsetSpec::default() }).map_or(0, |r| r.rim.len()),
        _ => inset(&mut m, faces, &InsetSpec { mode: InsetMode::Individual, thickness: 1e-4, depth: 0.0 }).map_or(0, |r| r.rim.len()),
    }
}
