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
