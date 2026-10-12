//! `describe` and `scene_overview`: one-screen summaries built from the
//! perception aspects and topology metrics (project spec 17.17).

use glam::Vec3;
use p3d_perceive::fmt::{deg, dims, point, thousands};
use p3d_perceive::relations::relations;
use p3d_perceive::shape::shape_of;
use p3d_perceive::structure::{structure, structure_text};
use p3d_perceive::{PerceiveError, View};
use p3d_scene::Document;
use serde_json::{Value, json};

/// Where an object's origin sits on its own box: `bottom center`, `center`, ...
fn origin_place(origin: Vec3, lo: Vec3, hi: Vec3) -> String {
    let size = (hi - lo).max(Vec3::splat(1e-9));
    let r = (origin - lo) / size;
    let tol = 0.01;
    let word = |x: f32, low: &'static str, high: &'static str| {
        if (x - 0.5).abs() <= tol {
            Some("center")
        } else if x.abs() <= tol {
            Some(low)
        } else if (x - 1.0).abs() <= tol {
            Some(high)
        } else {
            None
        }
    };
    match (word(r.y, "bottom", "top"), word(r.x, "left", "right"), word(r.z, "back", "front")) {
        (Some("center"), Some("center"), Some("center")) => "center".into(),
        (Some(y), Some("center"), Some("center")) => format!("{y} center"),
        (Some(y), Some(x), Some(z)) => format!("{y} {x} {z}"),
        _ => "off the box's landmarks".into(),
    }
}

/// The `describe` text and JSON of one object.
///
/// Complexity: O(objects + vertices) of the target's subtree, plus one
/// shape and one relations analysis.
pub fn describe(doc: &Document, target: &str) -> Result<(String, Value), PerceiveError> {
    let view = View::new(doc, Some(target))?;
    let key = view.root.ok_or_else(|| PerceiveError::invalid("/target", "no object"))?;
    let obj = doc.object(key).ok_or_else(|| PerceiveError::invalid("/target", "no object"))?;
    let world = doc.world_affine(key).unwrap_or_default();
    let children = view.parts.len() + view.others.len() - 1;
    let class = doc.settings.asset_class.as_str();
    let mut t = format!("object {} ({}) {}, {class}\n", obj.name(), obj.public_id(), obj.kind().as_str());
    let mut data = json!({ "name": obj.name(), "id": obj.public_id(), "kind": obj.kind().as_str(), "children": children });
    let origin: Vec3 = world.translation.into();
    if view.parts.is_empty() {
        t.push_str(&format!("at {} mm with {children} children, no geometry\n", point(origin)));
        return Ok((t, data));
    }
    let with = if children > 0 { format!(" with {children} child{}", if children == 1 { "" } else { "ren" }) } else { String::new() };
    t.push_str(&format!("size {} (W x H x D){with}\n", dims(view.size())));
    data["size"] = json!(view.size());
    let own = view.parts.iter().position(|p| p.key == key);
    if let Some(i) = own {
        let p = &view.parts[i];
        t.push_str(&format!("origin {} at {} mm\n", origin_place(origin, p.lo, p.hi), point(origin)));
    } else {
        t.push_str(&format!("origin at {} mm\n", point(origin)));
    }
    if children > 0 {
        let names: Vec<String> = view.parts.iter().map(|p| p.label.clone()).take(12).collect();
        t.push_str(&format!("parts: {}{}\n", names.join(", "), if view.parts.len() > 12 { ", ..." } else { "" }));
    }
    if let Some(i) = own {
        let p = &view.parts[i];
        let s = shape_of(&view, i);
        let fit = |f: &p3d_perceive::shape::Fit| match &f.axis {
            Some(a) => format!("{} {} {:.0}%", f.kind, a, f.percent),
            None => format!("{} {:.0}%", f.kind, f.percent),
        };
        let mirrors: Vec<String> = s.mirror.iter().map(|m| format!("{} {}", m.axis, if m.symmetric { "yes" } else { "no" })).collect();
        t.push_str(&format!(
            "shape: {} (then {}); mirror {}\n",
            s.fits.first().map(fit).unwrap_or_default(),
            s.fits.get(1).map(fit).unwrap_or_default(),
            mirrors.join(", ")
        ));
        let m = p.topology();
        let budget = doc.settings.tri_budget() as usize;
        let total: usize = View::new(doc, None).map(|v| v.parts.iter().map(|q| q.topology().tris).sum()).unwrap_or(m.tris);
        t.push_str(&format!(
            "topology: {} tris (scene {} of {} budget), quads {:.0}%, tris {}, ngons {}, poles 3:{} 5:{} 6+:{}, min angle {} deg, holes {}, pieces {}\n",
            thousands(m.tris),
            thousands(total),
            thousands(budget),
            m.quad_percent,
            m.tri_faces,
            m.ngon_faces,
            m.poles_3,
            m.poles_5,
            m.poles_6_plus,
            deg(m.min_angle_deg),
            m.boundary_loops,
            m.parts
        ));
        let groups: Vec<&str> = p.mesh.groups().map(|(n, _)| n).collect();
        if !groups.is_empty() {
            t.push_str(&format!(
                "groups: {}{}\n",
                groups.iter().take(16).copied().collect::<Vec<_>>().join(" "),
                if groups.len() > 16 { " ..." } else { "" }
            ));
        }
        let sharp = p.mesh.edges().filter(|&e| p.mesh.edge_flags(e).contains(p3d_mesh::EdgeFlags::SHARP)).count();
        t.push_str(&format!("shading: {} hard edges (30 deg or more), {} marked sharp\n", thousands(s.hard_edges), thousands(sharp)));
        let mut mats: Vec<u16> = p.mesh.faces().map(|f| p.mesh.face_material(f)).collect();
        mats.sort_unstable();
        mats.dedup();
        let uv = if p.mesh.uv_layer_count() == 0 {
            "none".to_owned()
        } else {
            format!("{} layer{}", p.mesh.uv_layer_count(), if p.mesh.uv_layer_count() == 1 { "" } else { "s" })
        };
        t.push_str(&format!("uv: {uv}; material slots: {}\n", mats.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")));
        data["shape"] = serde_json::to_value(&s).unwrap_or(Value::Null);
        data["topology"] = serde_json::to_value(m).unwrap_or(Value::Null);
        data["groups"] = json!(groups);
    }
    let rel = relations(&view);
    if let Some(i) = own {
        let st = structure(&view);
        let phrases = &st.parts[i].relations;
        if !phrases.is_empty() {
            t.push_str(&format!("contacts: {}\n", phrases.join(", ")));
        }
    }
    if !rel.floating.is_empty() {
        let names: Vec<String> = rel.floating.iter().map(|f| view.parts[f.part].label.clone()).collect();
        t.push_str(&format!("floating: {}\n", names.join(", ")));
    }
    Ok((t, data))
}

/// The `scene_overview` text and JSON.
///
/// Complexity: O(objects + vertices) plus relations of the whole scene.
pub fn overview(doc: &Document, revision: u64, full: bool) -> (String, Value) {
    let Ok(view) = View::new(doc, None) else { return ("empty scene\n".into(), json!({})) };
    let mut s = structure(&view);
    let tris: usize = view.parts.iter().map(|p| p.topology().tris).sum();
    let budget = doc.settings.tri_budget() as usize;
    let mut t = format!(
        "scene rev {revision}: {} object{} ({} mesh{}), {} tris (budget {}, {})\n",
        doc.object_count(),
        if doc.object_count() == 1 { "" } else { "s" },
        view.parts.len(),
        if view.parts.len() == 1 { "" } else { "es" },
        thousands(tris),
        thousands(budget),
        doc.settings.asset_class.as_str()
    );
    if doc.object_count() == 0 {
        t.push_str("empty: start with run (for example prim.box) or docs quickstart\n");
        return (t, json!({ "revision": revision, "objects": 0 }));
    }
    if !full {
        for p in &mut s.parts {
            p.groups.clear();
            p.tags.clear();
        }
    }
    // The structure text without its header line.
    let body = structure_text(&s);
    for line in body.lines().skip(1) {
        t.push_str(line);
        t.push('\n');
    }
    if full {
        let rel = relations(&view);
        for l in p3d_perceive::relations::relation_lines(&view, &rel, 20) {
            t.push_str(&l);
            t.push('\n');
        }
    }
    let data = json!({ "revision": revision, "objects": doc.object_count(), "tris": tris, "budget": budget, "structure": s });
    (t, data)
}
