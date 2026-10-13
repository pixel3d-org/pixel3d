//! Shading findings (project spec 17.10): smooth shading across folds
//! (`SMOOTHING_ARTIFACT`), n-gons on curved areas (`CURVED_NGON`) and hard
//! edges whose UVs run on across them (`HARD_EDGE_NO_SEAM`, which bakes a
//! visible line into normal maps). One pass over edges and faces per part.

use glam::Vec3;
use p3d_mesh::{EdgeFlags, Mesh};
use serde_json::{Value, json};

use super::source::{part_fix, spec_part};
use super::{Draft, Severity, decal, tagged};
use crate::fmt::thousands;
use crate::view::View;

/// Smooth shading across a fold sharper than this many degrees smears.
pub const SMOOTH_FOLD_DEG: f32 = 60.0;
/// Folds count once the faces along them cover this share of the part's
/// surface (a few pinched voxels of an SDF mesh do not show).
pub const SMOOTH_FOLD_AREA: f32 = 0.025;
/// The angle a `shade.smooth_by_angle` fix hardens edges at.
const FIX_ANGLE_DEG: f32 = 45.0;
/// N-gons whose corners leave their plane by more than this share of their
/// size follow a curved area.
pub const NGON_BEND: f32 = 0.02;
/// Element ids a fix names at most (beyond it the fix covers the part).
const MAX_IDS: usize = 256;

/// Whether an edge is shaded smoothly (not sharp, not on the boundary).
fn smooth(m: &Mesh, e: u32) -> bool {
    !m.edge_flags(e).contains(EdgeFlags::SHARP) && !m.is_boundary_edge(e)
}

/// Shading findings of part `i`.
///
/// Complexity: O(V + E + F + C) over the part's mesh.
pub fn check(view: &View, i: usize, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    if decal(view, part) || tagged(view, part, "strands") {
        return;
    }
    let m = part.mesh;
    let mut normals = m.face_normals();
    // Slivers have no trustworthy normal: leave them to DEGENERATE_FACES.
    for f in m.faces() {
        let longest = m.face_halfedges(f).map(|h| m.edge_length(h >> 1)).fold(0.0f32, f32::max);
        if m.face_area(f) < 1e-3 * longest * longest {
            normals[f as usize] = Vec3::ZERO;
        }
    }
    let sp = spec_part(view.doc, part.key);
    let fold = |e: u32| -> Option<f32> {
        let [Some(f), Some(g)] = m.edge_faces(e) else { return None };
        let (a, b) = (normals[f as usize], normals[g as usize]);
        (a != Vec3::ZERO && b != Vec3::ZERO).then(|| a.angle_between(b).to_degrees())
    };
    let mid = |e: u32| {
        let [a, b] = m.edge_verts(e);
        part.world.transform_point3((m.positions[a as usize] + m.positions[b as usize]) / 2.0)
    };

    // Smooth shading across folds (authored custom normals are left alone).
    if m.corner_normal.is_none() {
        let (mut n, mut worst) = (0usize, None::<(f32, u32)>);
        let mut along = vec![false; m.face_slots()];
        for e in m.edges().filter(|&e| smooth(m, e)) {
            if let Some(a) = fold(e)
                && a > SMOOTH_FOLD_DEG
            {
                n += 1;
                for f in m.edge_faces(e).into_iter().flatten() {
                    along[f as usize] = true;
                }
                if worst.is_none_or(|(w, _)| a > w) {
                    worst = Some((a, e));
                }
            }
        }
        let (mut hit, mut total) = (0.0f32, 0.0f32);
        if worst.is_some() {
            for f in m.faces() {
                let a = m.face_area(f);
                total += a;
                if along[f as usize] {
                    hit += a;
                }
            }
        }
        let share = if total > 0.0 { hit / total } else { 0.0 };
        if let Some((a, e)) = worst.filter(|_| share >= SMOOTH_FOLD_AREA) {
            let mut d = Draft::new(
                "SMOOTHING_ARTIFACT",
                Severity::Warn,
                i,
                mid(e),
                format!(
                    "{} smooth-shaded edge{} fold more than {SMOOTH_FOLD_DEG} degrees (worst {}, {}% of the surface): the shading smears dark across them",
                    thousands(n),
                    if n == 1 { "" } else { "s" },
                    a.round() as i32,
                    (share * 100.0).round() as i32
                ),
            )
            .measure(a, SMOOTH_FOLD_DEG, "deg");
            d.fixes.push(part_fix(
                view,
                i,
                sp.as_ref(),
                "shade.smooth_by_angle",
                json!({ "angle": FIX_ANGLE_DEG }),
                &format!("harden edges sharper than {FIX_ANGLE_DEG} degrees"),
            ));
            out.push(d);
        }
    }

    // N-gons whose corners follow a curve instead of a plane.
    let (mut ngons, mut worst, mut ids) = (0usize, None::<(f32, u32)>, Vec::new());
    for f in m.faces().filter(|&f| m.face_len(f) > 4) {
        let nf = normals[f as usize];
        if nf == Vec3::ZERO {
            continue;
        }
        let c = m.face_centroid(f);
        let (mut off, mut size) = (0.0f32, 0.0f32);
        for v in m.face_verts(f) {
            let p = m.positions[v as usize];
            off = off.max((p - c).dot(nf).abs());
            size = size.max(p.distance(c));
        }
        let bend = if size > 0.0 { off / size } else { 0.0 };
        let score = bend / NGON_BEND;
        if score > 1.0 {
            ngons += 1;
            if ids.len() < MAX_IDS {
                ids.push(f);
            }
            if worst.is_none_or(|(w, _)| score > w) {
                worst = Some((score, f));
            }
        }
    }
    if let Some((_, f)) = worst {
        let at = part.world.transform_point3(m.face_centroid(f));
        let mut d = Draft::new(
            "CURVED_NGON",
            Severity::Warn,
            i,
            at,
            format!(
                "{} n-gon{} on curved areas (corners up to {:.0}% of their size off their plane): they triangulate and shade unpredictably",
                thousands(ngons),
                if ngons == 1 { "" } else { "s" },
                worst.map_or(0.0, |(s, _)| s * NGON_BEND * 100.0)
            ),
        )
        .measure(worst.map_or(0.0, |(s, _)| s * NGON_BEND * 100.0), NGON_BEND * 100.0, "%");
        let select = if ngons <= MAX_IDS { json!({ "faces": { "ids": ids } }) } else { Value::Null };
        let args = if select.is_null() { json!({}) } else { json!({ "select": select }) };
        d.fixes.push(part_fix(view, i, sp.as_ref(), "edit.triangulate", args, "split the curved n-gons into triangles"));
        out.push(d);
    }

    // Hard edges whose UVs continue across them.
    if m.uv_layer_count() > 0 {
        let (mut n, mut gentle, mut first, mut ids) = (0usize, true, None, Vec::new());
        for e in m.edges() {
            let flags = m.edge_flags(e);
            if !flags.contains(EdgeFlags::SHARP) || flags.contains(EdgeFlags::SEAM) || m.is_boundary_edge(e) {
                continue;
            }
            let h = m.edge_he(e);
            let t = h ^ 1;
            let same = |x: u32, y: u32| match (m.corner_uv(0, x), m.corner_uv(0, y)) {
                (Some(a), Some(b)) => a.distance(b) < 1e-6,
                _ => false,
            };
            // Both ends of the edge: the corner of each face at that vertex.
            if same(h, m.next(t)) && same(m.next(h), t) {
                n += 1;
                first.get_or_insert(e);
                gentle &= fold(e).is_some_and(|a| a < FIX_ANGLE_DEG);
                if ids.len() < MAX_IDS {
                    ids.push(e);
                }
            }
        }
        if let Some(e) = first {
            let mut d = Draft::new(
                "HARD_EDGE_NO_SEAM",
                Severity::Warn,
                i,
                mid(e),
                format!("{} hard edge{} without a UV split: the normal map bakes a line along them", thousands(n), if n == 1 { "" } else { "s" }),
            )
            .count(n, 0);
            if gentle && n <= MAX_IDS {
                d.fixes.push(part_fix(
                    view,
                    i,
                    sp.as_ref(),
                    "shade.clear_sharp",
                    json!({ "select": { "edges": { "ids": ids } } }),
                    "soften them (they are gentle)",
                ));
            } else {
                d.hint = Some("cut UV seams along the hard edges (UV ops come with Phase 4), or soften the gentle ones".into());
            }
            out.push(d);
        }
    }
}
