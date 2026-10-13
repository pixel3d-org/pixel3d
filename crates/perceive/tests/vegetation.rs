//! The `vegetation` aspect on generated trees: crown proportions, fork and
//! trunk, card counts, coverage, gaps, overdraw and trunk visibility, all
//! measured from the mesh; the genes and the expectation helper.

#![allow(clippy::unwrap_used, clippy::print_stdout)]

use p3d_perceive::vegetation::{VegetationGene, expectation, gene_lines, measure, report};
use p3d_perceive::{TOKEN_BUDGET, View};
use p3d_scene::{Document, Range};
use p3d_vegetation::{Species, TreeSpec, tree};

fn doc_with(species: Species) -> (Document, p3d_vegetation::TreeReport) {
    let t = tree(&TreeSpec::preset(species)).unwrap();
    let mut doc = Document::new();
    let k = doc.create_mesh_object("tree", None, t.mesh).unwrap();
    doc.object_mut(k).unwrap().tags.push("vegetation".into());
    (doc, t.report)
}

#[test]
fn a_broadleaf_reads_like_its_reference_profile() {
    let (doc, r) = doc_with(Species::Broadleaf);
    let view = View::new(&doc, None).unwrap();
    let m = measure(&view, 0).unwrap();
    let rep = report(&view);
    println!("{}", rep.text);
    assert!(rep.tokens() <= TOKEN_BUDGET, "{} tokens", rep.tokens());
    assert!(!rep.text.contains('\u{2014}') && !rep.text.contains('\u{2013}'));
    assert_eq!(m.cards, r.cards + r.leaf_cards, "every card is one piece");
    assert_eq!(m.leaf_cards, r.leaf_cards);
    assert_eq!(m.limbs, r.limbs);
    let ratio = m.crown_width.max(m.crown_depth) / m.height;
    assert!((0.9..=1.15).contains(&ratio), "crown width / height {ratio}");
    let fork = m.fork.unwrap() / m.height;
    assert!((0.33..=0.42).contains(&fork), "fork {fork}");
    let d = m.trunk_diameter.unwrap();
    assert!((0.7..=1.0).contains(&d), "trunk diameter {d}");
    assert!(m.root_spread.unwrap() > 1.2, "root flare {:?}", m.root_spread);
    assert!(m.coverage_mean() > 0.8, "coverage {:?}", m.coverage);
    assert!((1.2..=4.0).contains(&m.overdraw_mean()), "overdraw {:?}", m.overdraw);
    let [front, side] = m.trunk_visible.unwrap();
    assert!(front > 0.3 && side > 0.3, "the trunk shows under the crown: {front} {side}");
    assert!(m.wind);
    let lines = gene_lines(&view, &VegetationGene::Preset("broadleaf".into())).unwrap();
    assert!(lines.iter().all(|(s, _)| *s == "ok"), "{lines:?}");
    let (pass, msg) = expectation(&m, Some(&Range { min: Some(10.0), max: Some(15.0) }), None, Some(&Range { min: Some(0.8), max: None }), Some(4.0));
    assert!(pass, "{msg}");
    let (pass, msg) = expectation(&m, None, None, None, Some(1.0));
    assert!(!pass && msg.contains("overdraw"), "{msg}");
}

#[test]
fn other_species_and_plain_meshes() {
    for s in [Species::Conifer, Species::Palm, Species::Birch, Species::Stylized] {
        let (doc, _) = doc_with(s);
        let view = View::new(&doc, None).unwrap();
        let rep = report(&view);
        assert!(rep.tokens() <= TOKEN_BUDGET, "{s:?}: {} tokens", rep.tokens());
        let lines = gene_lines(&view, &VegetationGene::Preset(s.as_str().into())).unwrap();
        assert!(lines.iter().all(|(v, _)| *v == "ok"), "{s:?}: {lines:?}\n{}", rep.text);
    }
    let doc = p3d_testdata::scenes::crate_scene();
    let rep = report(&View::new(&doc, None).unwrap());
    assert!(rep.text.contains("no part has foliage"), "{}", rep.text);
    assert!(gene_lines(&View::new(&doc, None).unwrap(), &VegetationGene::Preset("oak".into())).is_err());
}
