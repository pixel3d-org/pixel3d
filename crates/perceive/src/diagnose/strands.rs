//! Strands findings: `BALD_SPOT` from the report grown strands keep on
//! their object (metadata `strands`: coverage and the largest bald spot),
//! and `PENETRATING_STRANDS` when strand faces dip into the surface they
//! grow on (sampled against its BVH) or the report counts guide points
//! inside it.

use glam::Vec3;
use serde_json::{Value, json};

use super::source::{SpecPart, edit_fix, part_object, patch_fix, set_ops, shape_source, spec_part};
use super::{Draft, Severity};
use crate::fmt::mm;
use crate::view::View;

/// Strands covering less of their region than this leave bald spots (the
/// spec compiler warns below the same share).
pub const MIN_COVERAGE: f32 = 0.9;
/// Coverage a fix aims for.
const TARGET_COVERAGE: f32 = 0.96;
/// Most guides one strands part grows.
const MAX_CLUMPS: f64 = 4096.0;
/// Strand faces deeper than this inside their surface poke through it.
pub const DIP: f32 = 5e-4;
/// Strand faces sampled per part for the dip test.
const DIP_SAMPLES: usize = 2048;

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// How many times the cards' reach area must grow to cover `TARGET_COVERAGE`
/// of the region, for cards placed at random (coverage `1 - e^-density`).
fn growth(coverage: f32) -> f64 {
    let have = -(1.0 - f64::from(coverage).clamp(0.0, 0.99)).ln();
    let want = -(1.0 - f64::from(TARGET_COVERAGE)).ln();
    (want / have.max(1e-3)).clamp(1.5, 4.0)
}

/// The view index of the part strands grow on: the spec's `params.on`, else
/// the object's parent (where `gen.strands` puts them).
fn grows_on(view: &View, i: usize, sp: Option<&SpecPart>) -> Option<usize> {
    let doc = view.doc;
    let key = view.parts[i].key;
    let on = match sp {
        Some(sp) => sp.get("params/on").and_then(Value::as_str).and_then(|n| part_object(doc, sp.root, n)),
        None => doc.object(key).and_then(|o| o.parent()),
    }?;
    view.parts.iter().position(|p| p.key == on)
}

/// Strands findings of part `i`.
///
/// Complexity: O(1) for the report, plus one nearest-point query on the
/// surface the strands grow on when they penetrate it.
pub fn check(view: &View, i: usize, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    let Some(report) = view.doc.object(part.key).and_then(|o| o.metadata.get("strands")) else { return };
    let sp = spec_part(view.doc, part.key);
    if let Some(c) = num(report.get("coverage")).map(|c| c as f32)
        && c < MIN_COVERAGE
    {
        out.push(bald_spot(view, i, report, c, sp.as_ref()));
    }
    penetrating(view, i, report, sp.as_ref(), out);
}

fn bald_spot(view: &View, i: usize, report: &Value, c: f32, sp: Option<&SpecPart>) -> Draft {
    let part = &view.parts[i];
    let spot = report.get("bald_spot");
    let at = spot
        .and_then(|s| s.get("at"))
        .and_then(Value::as_array)
        .and_then(|a| Some(Vec3::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32)))
        .map_or(part.center(), |p| part.world.transform_point3(p));
    let area = num(spot.and_then(|s| s.get("area"))).unwrap_or(0.0);
    let pct = |x: f32| (x * 100.0).round() as i32;
    let mut message = format!("strands cover {}% of their region (want {}%)", pct(c), pct(MIN_COVERAGE));
    if area > 0.0 {
        message.push_str(&format!("; largest bald spot {} cm2", (area * 1e4).round() as i64));
    }
    let mut d = Draft::new("BALD_SPOT", Severity::Warn, i, at, message).measure(c * 100.0, MIN_COVERAGE * 100.0, "%");
    let width = num(report.get("width")).unwrap_or(0.0);
    let k = growth(c);
    match sp.map(|s| shape_source(view.doc, *s)) {
        Some(sp) => {
            let sp = &sp;
            let clumps = num(sp.get("params/clumps")).or_else(|| num(report.get("guides"))).unwrap_or(64.0).max(1.0);
            let more = (clumps * k).ceil().min(MAX_CLUMPS);
            let w = (width * 1e4).round() / 1e4;
            d.cause = Some(format!("{} clumps {} mm wide leave gaps", clumps.round() as i64, mm(width as f32)));
            d.cause_path = Some(sp.path("params/clumps"));
            let mut ops = set_ops(sp, "params/clumps", json!(more as u64));
            ops.extend(set_ops(sp, "params/width", json!(w)));
            d.fixes.push(patch_fix(
                view.doc,
                sp.root,
                ops,
                format!("raise clumps {} -> {}, keep cards {} mm wide", clumps.round() as i64, more as i64, mm(width as f32)),
            ));
            let wider = (width * k.sqrt() * 1e4).round() / 1e4;
            d.fixes.push(patch_fix(
                view.doc,
                sp.root,
                set_ops(sp, "params/width", json!(wider)),
                format!("widen the cards {} -> {} mm", mm(width as f32), mm(wider as f32)),
            ));
        }
        None => {
            d.hint = Some(format!(
                "grow them again with gen.strands and about {}x the clumps, or cards {} mm wide",
                (k * 10.0).round() / 10.0,
                mm((width * k.sqrt()) as f32)
            ));
        }
    }
    d
}

/// Strand faces inside the surface they grow on: how many of the sampled
/// face centers lie deeper than [`DIP`] inside it, the deepest, and the
/// middle of them. Faces lying on the surface (scalp caps) do not count.
///
/// Complexity: O(s x (log F + rays)) for s <= [`DIP_SAMPLES`] samples on
/// the parent's BVH.
fn dips(view: &View, i: usize, j: usize) -> (usize, f32, Vec3) {
    let (part, parent) = (&view.parts[i], &view.parts[j]);
    if !parent.topology().is_closed() {
        return (0, 0.0, Vec3::ZERO);
    }
    let m = part.mesh;
    let stride = m.face_count().div_ceil(DIP_SAMPLES).max(1);
    let (mut n, mut deepest, mut sum) = (0usize, 0.0f32, Vec3::ZERO);
    for f in m.faces().step_by(stride) {
        let c = part.world.transform_point3(m.face_centroid(f));
        let Some(near) = parent.bvh().closest(c, view.diagonal()) else { continue };
        if near.dist > DIP && parent.bvh().contains(c) {
            n += 1;
            deepest = deepest.max(near.dist);
            sum += c;
        }
    }
    (n * stride, deepest, if n > 0 { sum / n as f32 } else { Vec3::ZERO })
}

fn penetrating(view: &View, i: usize, report: &Value, sp: Option<&SpecPart>, out: &mut Vec<Draft>) {
    let part = &view.parts[i];
    let Some(j) = grows_on(view, i, sp) else { return };
    let guides = report.get("penetrating").and_then(Value::as_u64).unwrap_or(0) as usize;
    let (faces, deepest, middle) = dips(view, i, j);
    if faces == 0 && guides == 0 {
        return;
    }
    let parent = &view.parts[j];
    let on = parent.label.clone();
    let at = if faces > 0 { middle } else { parent.bvh().closest(part.center(), view.diagonal()).map_or(part.center(), |c| c.point) };
    let message = if faces > 0 {
        format!("about {faces} strand faces dip into {on}, up to {} mm deep: the scalp shows through", mm(deepest))
    } else {
        format!("{guides} guide point{} inside {on}: they show through it", if guides == 1 { "" } else { "s" })
    };
    let mut d = Draft::new("PENETRATING_STRANDS", Severity::Warn, i, at, message).count(faces.max(guides), 0);
    d.other = Some(j);
    let open = parent.topology().boundary_loops;
    match (sp.map(|s| shape_source(view.doc, *s)), spec_part(view.doc, parent.key)) {
        // The surface is broken: fix it and the strands regrow.
        (_, Some(psp)) if faces == 0 && open > 0 => {
            d.cause = Some(format!("{on} is open ({open} hole{}): the inside test sees through it", if open == 1 { "" } else { "s" }));
            d.cause_path = Some(psp.path(""));
            d.fixes.push(edit_fix(view.doc, &psp, "edit.fill", json!({}), format!("close the holes of {on} (the strands regrow)")));
        }
        (Some(sp), _) if faces > 0 => {
            let sp = &sp;
            let segments = num(sp.get("params/segments")).unwrap_or(8.0).max(2.0);
            let more = (segments * 3.0).min(64.0);
            d.cause = Some(format!("{} along each strand cut straight across the curve of {on}", segments as i64));
            d.cause_path = Some(sp.path("params/segments"));
            d.fixes.push(patch_fix(
                view.doc,
                sp.root,
                set_ops(sp, "params/segments", json!(more as u64)),
                format!("{} -> {} steps so the strands follow {on}", segments as i64, more as i64),
            ));
            let lift = sp.get("params/lift").and_then(Value::as_f64).unwrap_or(0.0);
            let up = ((lift + f64::from(deepest) + 1e-3) * 1e4).round() / 1e4;
            d.fixes.push(patch_fix(
                view.doc,
                sp.root,
                set_ops(sp, "params/lift", json!(up)),
                format!("lift the roots to {} mm off {on}", mm(up as f32)),
            ));
        }
        _ => {
            d.hint = Some(format!("grow the strands again with more segments or lift (gen.strands), or fix the surface of {on}"));
        }
    }
    out.push(d);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_raises_sparse_strands_more() {
        assert!(growth(0.5) > growth(0.8));
        assert!((1.5..=4.0).contains(&growth(0.89)));
        assert_eq!(growth(0.1), 4.0);
    }
}
