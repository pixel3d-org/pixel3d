//! Whole trees: every species and foliage mode builds a valid mesh with its
//! groups, landmarks, budget and coverage.

#![allow(clippy::unwrap_used, clippy::print_stdout)]

use p3d_vegetation::{Foliage, Species, Style, TreeSpec, groups, tree};

#[test]
fn every_species_builds_within_its_budget() {
    for species in Species::ALL {
        let spec = TreeSpec::preset(species);
        let t = tree(&spec).unwrap();
        let r = &t.report;
        println!("{species:?}: {r:?}");
        t.mesh.validate().unwrap();
        assert!(t.mesh.triangle_count() <= spec.budget as usize + spec.budget as usize / 10, "{species:?} {} tris", t.mesh.triangle_count());
        assert!(t.mesh.group(groups::TRUNK).is_some(), "{species:?} trunk");
        assert!(t.mesh.group(groups::FOLIAGE).is_some(), "{species:?} foliage");
        for lm in ["base", "fork", "crown_top", "crown_center"] {
            assert!(t.mesh.group(&format!("lm.{lm}")).is_some(), "{species:?} {lm}");
        }
    }
}

#[test]
fn the_broadleaf_matches_its_reference_profile() {
    let spec = TreeSpec::preset(Species::Broadleaf);
    let t = tree(&spec).unwrap();
    let r = &t.report;
    let ratio = r.crown_width / r.height;
    assert!((0.85..=1.15).contains(&ratio), "crown width / height {ratio}");
    let fork = r.fork_height / r.height;
    assert!((0.33..=0.42).contains(&fork), "fork at {fork}");
    assert!((3..=5).contains(&r.limbs), "{} limbs", r.limbs);
    assert!(r.coverage >= 0.83, "coverage {}", r.coverage);
    let tris = t.mesh.triangle_count();
    assert!((7_000..=11_000).contains(&tris), "{tris} tris");
    assert_eq!(r.repeats, 0, "neighbouring repeats");
}

#[test]
fn foliage_modes_and_styles_build() {
    for foliage in Foliage::ALL {
        let spec = TreeSpec { foliage, ..TreeSpec::preset(Species::Broadleaf) };
        let t = tree(&spec).unwrap();
        t.mesh.validate().unwrap();
        println!("{foliage:?}: {} tris, coverage {:.2}", t.mesh.triangle_count(), t.report.coverage);
    }
    for style in Style::ALL {
        let spec = TreeSpec::preset(Species::Broadleaf).with_style(style);
        let t = tree(&spec).unwrap();
        println!("{style:?}: {} tris, {} cards, coverage {:.2}", t.mesh.triangle_count(), t.report.cards, t.report.coverage);
    }
}

#[test]
fn cards_run_inner_to_outer_and_darken_inside() {
    let t = tree(&TreeSpec::preset(Species::Broadleaf)).unwrap();
    let m = &t.mesh;
    let colors = m.corner_color.as_ref().unwrap();
    let foliage: Vec<u32> = m.group(groups::FOLIAGE).unwrap().iter().collect();
    let center = t.hull.center();
    let mean = |faces: &[u32]| {
        let (mut lum, mut dist, mut n) = (0.0f32, 0.0f32, 0.0f32);
        for &f in faces {
            for h in m.face_halfedges(f) {
                let c = colors[h as usize];
                lum += (c.x + c.y + c.z) / 3.0;
                dist += m.positions[m.origin(h) as usize].distance(center);
                n += 1.0;
            }
        }
        (lum / n, dist / n)
    };
    let k = foliage.len() / 5;
    let (inner_lum, inner_dist) = mean(&foliage[..k]);
    let (outer_lum, outer_dist) = mean(&foliage[foliage.len() - k..]);
    assert!(inner_dist < outer_dist, "emitted inner first: {inner_dist} vs {outer_dist}");
    assert!(inner_lum < outer_lum, "darker inside: {inner_lum} vs {outer_lum}");
    // Card normals lean toward the crown hull's.
    let normals = m.corner_normal.as_ref().unwrap();
    let outward = foliage.iter().filter(|&&f| {
        let h = m.face_he(f);
        normals[h as usize].dot(t.hull.normal(m.positions[m.origin(h) as usize])) > 0.0
    });
    assert!(outward.count() * 10 > foliage.len() * 7, "most card normals face out of the crown");
}
