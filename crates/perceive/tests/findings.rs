//! Mesh-level findings of full diagnostics (project spec 17.10): edge flow
//! on deforming parts, shading, detail scale and strands reports, plus the
//! kernels the spec layer runs at landmarks (loops across a joint, flow
//! circling an eye).

#![allow(clippy::unwrap_used, clippy::panic)]

use glam::{DVec3, Vec3};
use p3d_geo::prim::{BoxSpec, CylinderSpec, IcoSphereSpec, Origin, UvSphereSpec, cuboid, cylinder, ico_sphere, uv_sphere};
use p3d_mesh::{EdgeFlags, Mesh};
use p3d_perceive::View;
use p3d_perceive::diagnose::flow::{joint_loops, ring_flow};
use p3d_perceive::diagnose::{Finding, diagnose};
use p3d_scene::{AssetClass, Document};
use p3d_testdata::scenes::add;

fn one(name: &str, mesh: Mesh, tags: &[&str]) -> Document {
    let mut doc = Document::new();
    let k = add(&mut doc, name, None, mesh, [0.0; 3]);
    doc.object_mut(k).unwrap().tags.extend(tags.iter().map(|t| (*t).to_owned()));
    doc
}

fn find<'a>(f: &'a [Finding], code: &str) -> Option<&'a Finding> {
    f.iter().find(|x| x.code == code)
}

fn codes(f: &[Finding]) -> Vec<&'static str> {
    f.iter().map(|x| x.code).collect()
}

#[test]
fn ring_flow_tells_concentric_loops_from_a_grid() {
    let s = uv_sphere(&UvSphereSpec { radius: 0.1, segments: 32, rings: 32 }, Origin::Center).unwrap();
    let doc = one("ball", s, &[]);
    let view = View::new(&doc, None).unwrap();
    let part = &view.parts[0];
    // Around the pole the parallels circle it and the meridians run out of it.
    let pole = ring_flow(part, Vec3::new(0.0, 0.1, 0.0), 0.05).unwrap();
    assert!(pole < 0.1, "{pole}");
    // On the equator the same lines make a grid that ignores the point.
    let side = ring_flow(part, Vec3::new(0.0, 0.0, 0.1), 0.05).unwrap();
    assert!(side > 0.5, "{side}");
}

#[test]
fn joint_loops_count_rings_across_a_limb() {
    let arm = |rings: u32| {
        cylinder(&CylinderSpec { radius: 0.04, height: 0.6, segments: 16, height_segments: rings, ..CylinderSpec::default() }, Origin::Bottom)
            .unwrap()
    };
    let elbow = Vec3::new(0.0, 0.3, 0.04);
    let doc = one("arm", arm(1), &[]);
    let view = View::new(&doc, None).unwrap();
    let j = joint_loops(&view.parts[0], elbow).unwrap();
    assert!(j.loops < 1.0 && (j.radius - 0.04).abs() < 0.004 && j.axis.y.abs() > 0.99, "{j:?}");
    // 30 rings over 600 mm: 20 mm apart, 4 across the 80 mm limb.
    let doc = one("arm", arm(30), &[]);
    let view = View::new(&doc, None).unwrap();
    let j = joint_loops(&view.parts[0], elbow).unwrap();
    assert!((j.loops - 4.0).abs() < 0.3, "{j:?}");
}

#[test]
fn deforming_parts_get_edge_flow_findings() {
    // Triangles: an ico sphere.
    let doc = one("blob", ico_sphere(&IcoSphereSpec { radius: 0.1, subdivisions: 3 }, Origin::Center).unwrap(), &["deforming"]);
    let f = diagnose(&View::new(&doc, None).unwrap());
    let t = find(&f, "DEFORM_TRIANGLES").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((t.measured, t.threshold, t.unit), (Some(100.0), Some(5.0), Some("%")));
    assert_eq!(t.fixes[0].op, "remesh.quad");
    // Without the tag nothing about edge flow is said.
    let doc = one("blob", ico_sphere(&IcoSphereSpec { radius: 0.1, subdivisions: 3 }, Origin::Center).unwrap(), &[]);
    let f = diagnose(&View::new(&doc, None).unwrap());
    assert!(find(&f, "DEFORM_TRIANGLES").is_none(), "{:?}", codes(&f));

    // Poles: a UV sphere's poles have 32 edges; its triangles are a small share.
    let s = uv_sphere(&UvSphereSpec { radius: 0.1, segments: 32, rings: 48 }, Origin::Center).unwrap();
    let doc = one("ball", s, &["deforming"]);
    let f = diagnose(&View::new(&doc, None).unwrap());
    let p = find(&f, "DEFORM_POLES").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!(p.measured, Some(2.0));
    assert!(p.message.contains("worst 32") && (p.at.y.abs() - 0.1).abs() < 1e-3, "{p:?}");

    // Density: one face of a box cut into 64 next to whole faces.
    let mut m = cuboid(&BoxSpec { size: DVec3::splat(0.4), segments: [1; 3], ..BoxSpec::default() }, Origin::Center).unwrap();
    let top: Vec<u32> = m.group("top").unwrap().iter().collect();
    p3d_geo::edit::subdivide(&mut m, &top, 3).unwrap();
    let doc = one("block", m, &["deforming"]);
    let f = diagnose(&View::new(&doc, None).unwrap());
    let d = find(&f, "DENSITY_JUMP").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((d.measured, d.unit), (Some(8.0), Some("ratio")));
}

#[test]
fn shading_findings() {
    // A cube shaded smooth all over: 90 degree folds.
    let mut m = cuboid(&BoxSpec { size: DVec3::splat(0.4), segments: [1; 3], ..BoxSpec::default() }, Origin::Center).unwrap();
    for e in m.edges().collect::<Vec<_>>() {
        m.set_edge_flag(e, EdgeFlags::SHARP, false);
    }
    let f = diagnose(&View::new(&one("cube", m, &[]), None).unwrap());
    let s = find(&f, "SMOOTHING_ARTIFACT").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((s.measured.map(f32::round), s.unit), (Some(90.0), Some("deg")));
    assert_eq!(s.fixes[0].op, "shade.smooth_by_angle");

    // The primitive cube is hard-edged: clean.
    let m = cuboid(&BoxSpec { size: DVec3::splat(0.4), segments: [1; 3], ..BoxSpec::default() }, Origin::Center).unwrap();
    let f = diagnose(&View::new(&one("cube", m, &[]), None).unwrap());
    assert!(f.is_empty(), "{:?}", codes(&f));

    // A hexagon bent out of its plane.
    let p: Vec<Vec3> = (0..6)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            Vec3::new(0.2 * a.cos(), if k % 2 == 0 { 0.02 } else { 0.0 }, 0.2 * a.sin())
        })
        .collect();
    let m = Mesh::from_polygons(p, &[6], &[0, 1, 2, 3, 4, 5]).unwrap();
    let f = diagnose(&View::new(&one("lid", m, &[]), None).unwrap());
    let n = find(&f, "CURVED_NGON").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((n.measured.map(f32::round), n.threshold, n.unit), (Some(5.0), Some(2.0), Some("%")));
    assert_eq!(n.fixes[0].op, "edit.triangulate");

    // Hard edges inside a box face, where its UVs run on.
    let mut m = cuboid(&BoxSpec { size: DVec3::splat(0.4), segments: [2; 3], ..BoxSpec::default() }, Origin::Center).unwrap();
    for e in m.edges().collect::<Vec<_>>() {
        m.set_edge_flag(e, EdgeFlags::SHARP, true);
    }
    // Box mapping: each side its own island, so only the cube's edges split UVs.
    let layer = m.add_uv_layer().unwrap();
    for f in m.faces().collect::<Vec<_>>() {
        let k = m.face_normal(f).abs().max_position();
        let hs: Vec<u32> = m.face_halfedges(f).collect();
        for h in hs {
            let p = m.positions[m.origin(h) as usize];
            let uv = match k {
                0 => glam::Vec2::new(p.y, p.z),
                1 => glam::Vec2::new(p.x + 1.0, p.z),
                _ => glam::Vec2::new(p.x + 2.0, p.y),
            };
            m.corner_uv[layer][h as usize] = uv;
        }
    }
    let f = diagnose(&View::new(&one("crate", m, &[]), None).unwrap());
    let h = find(&f, "HARD_EDGE_NO_SEAM").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((h.measured, h.unit), (Some(24.0), Some("count")));
    assert_eq!(h.fixes[0].op, "shade.clear_sharp");
}

#[test]
fn detail_under_two_pixels_is_flagged_at_the_class_distance() {
    let mut doc = Document::new();
    doc.settings.asset_class = AssetClass::PropMedium;
    add(
        &mut doc,
        "crate",
        None,
        cuboid(&BoxSpec { size: DVec3::splat(0.5), segments: [1; 3], ..BoxSpec::default() }, Origin::Bottom).unwrap(),
        [0.0; 3],
    );
    let rivet = uv_sphere(&UvSphereSpec { radius: 0.003, segments: 8, rings: 4 }, Origin::Center).unwrap();
    add(&mut doc, "rivet", None, rivet, [0.0, 0.25, 0.252]);
    let f = diagnose(&View::new(&doc, None).unwrap());
    let d = find(&f, "DETAIL_SCALE").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((d.part.as_str(), d.threshold, d.unit), ("rivet", Some(2.0), Some("px")));
    assert!((d.measured.unwrap() - 1.12).abs() < 0.02, "{d:?}");
    assert_eq!(d.fixes[0].op, "scene.tag");
    // Seen from 0.7 m (first-person weapon) it is 8 px: fine.
    doc.settings.asset_class = AssetClass::WeaponFp;
    let f = diagnose(&View::new(&doc, None).unwrap());
    assert!(find(&f, "DETAIL_SCALE").is_none(), "{:?}", codes(&f));
}

#[test]
fn strands_reports_become_findings() {
    let mut doc = Document::new();
    let head = add(&mut doc, "head", None, p3d_testdata::scenes::sphere_mesh(0.1, 8), [0.0, 0.3, 0.0]);
    let cards = p3d_testdata::scenes::sheet_mesh(0.1);
    let hair = add(&mut doc, "hair", Some(head), cards, [0.0, 0.1, 0.0]);
    let o = doc.object_mut(hair).unwrap();
    o.tags.push("strands".into());
    o.metadata.insert(
        "strands".into(),
        serde_json::json!({ "kind": "hair", "output": "cards", "guides": 20, "width": 0.009, "coverage": 0.62,
            "bald_spot": { "at": [0.0, 0.0, 0.05], "area": 0.0012 }, "penetrating": 3 }),
    );
    let f = diagnose(&View::new(&doc, None).unwrap());
    let b = find(&f, "BALD_SPOT").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((b.measured.map(f32::round), b.threshold, b.unit), (Some(62.0), Some(90.0), Some("%")));
    assert!(b.at.abs_diff_eq(Vec3::new(0.0, 0.4, 0.05), 1e-5) && b.message.contains("12 cm2"), "{b:?}");
    assert!(b.fixes.is_empty() && b.hint.as_deref().is_some_and(|h| h.contains("gen.strands")), "{b:?}");
    let p = find(&f, "PENETRATING_STRANDS").unwrap_or_else(|| panic!("{:?}", codes(&f)));
    assert_eq!((p.measured, p.other.as_deref()), (Some(3.0), Some("head")));
    // Strands are never floating, open or thin.
    assert!(f.iter().all(|x| x.part != "hair" || matches!(x.code, "BALD_SPOT" | "PENETRATING_STRANDS")), "{:?}", codes(&f));
}

#[test]
fn every_finding_says_its_unit() {
    for (name, doc) in p3d_testdata::scenes::broken_corpus() {
        for f in diagnose(&View::new(&doc, None).unwrap()) {
            if f.measured.is_some() {
                assert!(f.unit.is_some(), "{name} {}: measured without a unit", f.code);
            }
        }
    }
    let doc = p3d_testdata::scenes::broken_corpus().into_iter().find(|(n, _)| *n == "floating_part").unwrap().1;
    let f = diagnose(&View::new(&doc, None).unwrap());
    assert_eq!((f[0].measured.map(f32::round), f[0].unit), (Some(12.0), Some("mm")));
}
