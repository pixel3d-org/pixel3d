//! Trees keep clean topology across species, styles, foliage modes and
//! sizes: valid meshes whose only open faces are foliage, within budget,
//! with every landmark, and the same tree for the same seed.

#![allow(clippy::unwrap_used)]

use p3d_vegetation::{Foliage, Species, Style, TreeSpec, groups, tree};
use proptest::prelude::*;

fn closed_but_foliage(m: &p3d_mesh::Mesh) -> bool {
    let cards: std::collections::BTreeSet<u32> = m.group(groups::FOLIAGE).map(|g| g.iter().collect()).unwrap_or_default();
    m.edges().filter(|&e| m.is_boundary_edge(e)).all(|e| m.edge_faces(e).iter().flatten().all(|f| cards.contains(f)))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if cfg!(debug_assertions) { 16 } else { 64 }))]

    #[test]
    fn random_trees_are_valid(species in 0usize..5, style in 0usize..3, foliage in 0usize..4, height in 1.0f32..30.0, lean in 0.0f32..0.5, seed in 0u64..1000, budget in 1500u32..12000) {
        let mut spec = TreeSpec::preset(Species::ALL[species]).with_style(Style::ALL[style]);
        let k = height / spec.height;
        spec.height = height;
        spec.trunk_radius *= k;
        spec.crown_width *= k;
        spec.card_size *= k.sqrt();
        spec.foliage = Foliage::ALL[foliage];
        spec.lean = lean;
        spec.seed = seed;
        spec.budget = budget;
        let t = tree(&spec).unwrap();
        prop_assert_eq!(t.mesh.validate(), Ok(()));
        prop_assert!(closed_but_foliage(&t.mesh));
        prop_assert!(t.mesh.triangle_count() <= budget as usize + budget as usize / 5 + 600, "{} tris for {}", t.mesh.triangle_count(), budget);
        for lm in ["base", "fork", "crown_top", "crown_center", "crown_base"] {
            let name = format!("lm.{lm}");
            prop_assert!(t.mesh.group(&name).is_some(), "{}", name);
        }
        let again = tree(&spec).unwrap();
        prop_assert_eq!(&t.mesh.positions, &again.mesh.positions);
    }
}
