//! Property tests on random valid meshes (primitives with random parameters
//! and random face deletions).

use p3d_mesh::{CornerSrc, Mesh, Patch, TriOptions, Vec3};
use p3d_testdata::strategy;
use proptest::prelude::*;

fn euler(m: &Mesh) -> i64 {
    m.vert_count() as i64 - m.edge_count() as i64 + m.face_count() as i64
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn primitives_are_valid(m in strategy::primitive()) {
        prop_assert!(m.face_count() > 0, "generator failed on valid parameters");
        prop_assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn deletions_keep_the_mesh_valid(m in strategy::mesh()) {
        prop_assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn compact_preserves_everything_live(mut m in strategy::mesh()) {
        let counts = (m.vert_count(), m.edge_count(), m.face_count(), m.triangle_count(), euler(&m));
        let positions: Vec<Vec3> = m.verts().map(|v| m.positions[v as usize]).collect();
        let groups: Vec<(String, usize)> = m.groups().map(|(n, g)| (n.to_owned(), g.count())).collect();
        m.compact();
        prop_assert_eq!(m.validate(), Ok(()));
        prop_assert!(!m.has_holes_in_storage());
        prop_assert_eq!((m.vert_count(), m.edge_count(), m.face_count(), m.triangle_count(), euler(&m)), counts);
        prop_assert_eq!(m.positions.clone(), positions);
        prop_assert_eq!(m.groups().map(|(n, g)| (n.to_owned(), g.count())).collect::<Vec<_>>(), groups);
    }

    #[test]
    fn triangle_buffers_are_consistent(m in strategy::mesh()) {
        let b = m.to_tri_buffers(&TriOptions::default());
        prop_assert_eq!(b.triangle_count(), m.triangle_count());
        prop_assert!(b.indices.iter().all(|&i| (i as usize) < b.positions.len()));
        prop_assert_eq!(b.normals.len(), b.positions.len());
        prop_assert!(b.vert_source.iter().all(|&v| m.is_vert_alive(v)));
        prop_assert!(b.tri_face.iter().all(|&f| m.is_face_alive(f)));
        // No more output vertices than corners, no fewer than used vertices.
        let corners: usize = m.faces().map(|f| m.face_len(f)).sum();
        prop_assert!(b.positions.len() <= corners && b.positions.len() >= m.vert_count());
        // Submeshes tile the index buffer.
        let covered: u32 = b.submeshes.iter().map(|s| s.count).sum();
        prop_assert_eq!(covered as usize, b.indices.len());
        // Triangulated area matches face area (quads of spheres are slightly non-planar).
        let face_area: f32 = m.faces().map(|f| m.face_area(f)).sum();
        let tri_area: f32 = b.indices.chunks(3).map(|t| {
            let [a, c, d] = [0, 1, 2].map(|i| b.positions[t[i] as usize]);
            0.5 * (c - a).cross(d - a).length()
        }).sum();
        prop_assert!((tri_area - face_area).abs() <= 0.02 * face_area + 1e-6, "{} vs {}", tri_area, face_area);
        // Deterministic.
        prop_assert_eq!(m.to_tri_buffers(&TriOptions::default()), b);
    }

    #[test]
    fn corner_normals_are_unit(m in strategy::mesh()) {
        let n = m.compute_corner_normals();
        for f in m.faces() {
            for h in m.face_halfedges(f) {
                prop_assert!((n[h as usize].length() - 1.0).abs() < 1e-4, "corner {} has normal {}", h, n[h as usize]);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn flip_reverses_every_face(m in strategy::mesh()) {
        let mut f = m.clone();
        f.flip_all();
        prop_assert_eq!(f.validate(), Ok(()));
        for face in m.faces() {
            let (a, b) = (m.face_normal(face), f.face_normal(face));
            prop_assert!(a.length() < 1e-6 || a.dot(b) < -0.99, "face {} not reversed", face);
        }
        f.flip_all();
        prop_assert_eq!(f.validate(), Ok(()));
        prop_assert_eq!(f.triangle_count(), m.triangle_count());
    }

    #[test]
    fn mirror_transform_is_valid(m in strategy::mesh(), axis in 0usize..3) {
        let mut s = Vec3::ONE;
        s[axis] = -1.0;
        let mut t = m.clone();
        t.transform(glam::Affine3A::from_scale(s));
        prop_assert_eq!(t.validate(), Ok(()));
        for face in m.faces() {
            let mirrored = m.face_normal(face) * s;
            prop_assert!(mirrored.length() < 1e-6 || t.face_normal(face).dot(mirrored) > 0.99);
        }
    }

    #[test]
    fn components_split_and_append_back(m in strategy::mesh()) {
        let (labels, n) = m.face_components();
        let parts = m.split_by_face_labels(&labels);
        prop_assert_eq!(parts.len(), n as usize);
        let mut joined = Mesh::new();
        for p in &parts {
            prop_assert_eq!(p.validate(), Ok(()));
            joined.append(p);
        }
        prop_assert_eq!(joined.validate(), Ok(()));
        prop_assert_eq!((joined.vert_count(), joined.edge_count(), joined.face_count()), (m.vert_count(), m.edge_count(), m.face_count()));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn filling_every_hole_closes_the_mesh(m in strategy::mesh()) {
        let mut f = m.clone();
        let mut filled = 0usize;
        // Fill loops of 3 or more edges until no boundary half-edge is left.
        while let Some(h) = (0..f.he_slots() as u32).find(|&h| f.is_he_alive(h) && f.is_boundary_he(h)) {
            if f.fill_boundary_loop(h).is_err() {
                break;
            }
            filled += 1;
            prop_assert_eq!(f.validate(), Ok(()));
        }
        prop_assert_eq!(f.face_count(), m.face_count() + filled);
        prop_assert_eq!((f.vert_count(), f.edge_count()), (m.vert_count(), m.edge_count()));
    }

    #[test]
    fn flipping_a_part_twice_is_identity(m in strategy::mesh(), pick in any::<u32>()) {
        let live: Vec<u32> = m.faces().collect();
        prop_assume!(!live.is_empty());
        let f0 = live[pick as usize % live.len()];
        let mut f = m.clone();
        let n = f.flip_part(f0).unwrap();
        prop_assert_eq!(f.validate(), Ok(()));
        let part = m.part_faces(f0).unwrap();
        prop_assert_eq!(n, part.len());
        for face in m.faces() {
            let (a, b) = (m.face_normal(face), f.face_normal(face));
            let flipped = part.binary_search(&face).is_ok();
            prop_assert!(a.length() < 1e-6 || a.dot(b) * if flipped { -1.0 } else { 1.0 } > 0.99, "face {}", face);
        }
        f.flip_part(f0).unwrap();
        prop_assert_eq!(f.validate(), Ok(()));
        prop_assert_eq!(f.to_tri_buffers(&TriOptions::default()).indices, m.to_tri_buffers(&TriOptions::default()).indices);
    }
}

/// Faces picked by random numbers, sorted and deduplicated.
fn pick_faces(m: &Mesh, picks: &[u32]) -> Vec<u32> {
    let live: Vec<u32> = m.faces().collect();
    let mut out: Vec<u32> = picks.iter().filter(|_| !live.is_empty()).map(|&p| live[p as usize % live.len()]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn patch_reinserting_faces_is_identity(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..10)) {
        let faces = pick_faces(&m, &picks);
        let mut p = Patch::new(&m);
        for &f in &faces {
            p.remove_face(f);
            let vs: Vec<u32> = m.face_verts(f).collect();
            let src: Vec<CornerSrc> = m.face_halfedges(f).map(CornerSrc::copy).collect();
            p.add_face(&vs, &src, m.face_material(f), Some(f));
        }
        let mut n = m.clone();
        let r = n.apply_patch(&p).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!((r.removed_edges, r.removed_verts, r.split_verts), (0, 0, 0));
        prop_assert_eq!((n.vert_count(), n.edge_count(), n.face_count(), euler(&n)), (m.vert_count(), m.edge_count(), m.face_count(), euler(&m)));
        prop_assert_eq!(n.triangle_count(), m.triangle_count());
    }

    #[test]
    fn patch_poking_faces(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..10)) {
        let faces = pick_faces(&m, &picks);
        let mut p = Patch::new(&m);
        let mut corners = 0usize;
        for &f in &faces {
            p.remove_face(f);
            let vs: Vec<u32> = m.face_verts(f).collect();
            let c = p.new_vert(m.face_centroid(f), None);
            for i in 0..vs.len() {
                p.add_face(&[vs[i], vs[(i + 1) % vs.len()], c], &[], 0, Some(f));
            }
            corners += vs.len();
        }
        let mut n = m.clone();
        n.apply_patch(&p).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!(n.face_count(), m.face_count() - faces.len() + corners);
        prop_assert_eq!(n.vert_count(), m.vert_count() + faces.len());
        prop_assert_eq!(euler(&n), euler(&m));
    }

    #[test]
    fn patch_extruding_a_random_region(m in strategy::mesh(), picks in prop::collection::vec(any::<u32>(), 1..12)) {
        let faces = pick_faces(&m, &picks);
        let in_region = |f: Option<u32>| f.is_some_and(|f| faces.binary_search(&f).is_ok());
        let mut p = Patch::new(&m);
        // One copy per fan of region faces around a vertex (a region can touch
        // itself at a vertex): every region corner of a fan maps to its copy.
        let mut copy: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        for &f in &faces {
            for h in m.face_halfedges(f) {
                if copy.contains_key(&h) {
                    continue;
                }
                // Back up to the first corner of the fan, then walk it forward.
                let mut first = h;
                loop {
                    let t = first ^ 1;
                    if !in_region(m.face(t)) || m.next(t) == h {
                        break;
                    }
                    first = m.next(t);
                }
                let v = m.origin(h);
                let c = p.new_vert(m.positions[v as usize] + Vec3::Y * 0.1, Some(v));
                let mut g = first;
                loop {
                    copy.insert(g, c);
                    let t = m.prev(g) ^ 1;
                    if !in_region(m.face(t)) || t == first {
                        break;
                    }
                    g = t;
                }
            }
        }
        let mut sides = 0usize;
        for &f in &faces {
            p.remove_face(f);
            let vs: Vec<u32> = m.face_halfedges(f).map(|h| copy[&h]).collect();
            let src: Vec<CornerSrc> = m.face_halfedges(f).map(CornerSrc::copy).collect();
            p.add_face(&vs, &src, 0, Some(f));
            for h in m.face_halfedges(f) {
                if !in_region(m.face(h ^ 1)) {
                    let (u, w) = (m.origin(h), m.dest(h));
                    p.add_face(&[u, w, copy[&m.next(h)], copy[&h]], &[], 0, None);
                    sides += 1;
                }
            }
        }
        let mut n = m.clone();
        n.apply_patch(&p).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(n.validate(), Ok(()));
        prop_assert_eq!(n.face_count(), m.face_count() + sides);
    }
}
