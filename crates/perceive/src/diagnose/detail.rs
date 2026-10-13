//! Detail scale (project spec 17.10, game readiness): modelled detail that
//! ends up under about 2 pixels at the asset class's view distance on a
//! 1080p screen reads as noise and costs triangles; it belongs in a normal
//! map or a decal (`DETAIL_SCALE`).

use serde_json::json;

use super::source::{patch_fix, set_ops, spec_part};
use super::{Draft, Fix, Severity, decal, tagged};
use crate::fmt::{mm, thousands};
use crate::view::View;
use crate::visibility::pixels_per_meter;

/// Detail smaller than this many pixels is too small to model.
pub const MIN_DETAIL_PX: f32 = 2.0;

/// Parts whose cross-section (their middle extent: a speck or a wire seen
/// from any side) is under [`MIN_DETAIL_PX`] at the view distance.
///
/// Complexity: O(parts).
pub fn check(view: &View, out: &mut Vec<Draft>) {
    let class = view.doc.settings.asset_class;
    let distance = class.view_distance();
    let px = pixels_per_meter(distance);
    for (i, part) in view.parts.iter().enumerate() {
        if decal(view, part) || tagged(view, part, "strands") {
            continue;
        }
        let mut s = part.size().to_array();
        s.sort_unstable_by(f32::total_cmp);
        let across = s[1];
        let size_px = across * px;
        if size_px >= MIN_DETAIL_PX {
            continue;
        }
        let tris = part.mesh.triangle_count();
        let mut d = Draft::new(
            "DETAIL_SCALE",
            Severity::Warn,
            i,
            part.center(),
            format!(
                "{} mm across: {:.1} px at {} m on 1080p ({}), under {MIN_DETAIL_PX} px: {} tris of detail nobody sees",
                mm(across),
                size_px,
                (distance * 10.0).round() / 10.0,
                class.as_str(),
                thousands(tris)
            ),
        )
        .measure(size_px, MIN_DETAIL_PX, "px");
        match spec_part(view.doc, part.key) {
            Some(sp) => {
                d.cause_path = Some(sp.path(""));
                d.cause = Some(format!("/parts/{} is modelled at {} mm", sp.name, mm(across)));
                d.fixes.push(patch_fix(
                    view.doc,
                    sp.root,
                    set_ops(&sp, "join", json!("decal")),
                    format!("make it a decal join (baked into the textures): -{} tris", thousands(tris)),
                ));
            }
            None => d.fixes.push(Fix::op(
                "scene.tag",
                json!({ "target": part.path, "add": ["decal"] }),
                format!("tag it decal (bake it into a normal map): -{} tris", thousands(tris)),
            )),
        }
        out.push(d);
    }
}
