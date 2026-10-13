//! Boolean engine evaluation (ADR 0008): manifold-rust against boolmesh on
//! correctness (valid manifold results, volumes, coplanar and degenerate
//! cases, random pairs) and speed. Run with
//! `cargo bench -p p3d-geo --bench boolean_eval`.

// A measurement program, not library code: it reports with println and stops on setup errors.
#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use glam::{Affine3A, DVec3, Quat, Vec3};
use p3d_geo::prim::{BoxSpec, IcoSphereSpec, Origin, QuadSphereSpec, cuboid, ico_sphere, quad_sphere};
use p3d_mesh::Mesh;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Union,
    Difference,
    Intersect,
}

/// Welded triangles: positions and three indices per triangle.
#[derive(Clone, Default)]
struct Tris {
    pos: Vec<DVec3>,
    idx: Vec<usize>,
}

fn tris(m: &Mesh) -> Tris {
    let mut m = m.clone();
    m.compact();
    let mut idx = Vec::new();
    for f in m.faces() {
        for t in m.face_triangles(f) {
            idx.extend(t.map(|h| m.origin(h) as usize));
        }
    }
    Tris { pos: m.positions.iter().map(|p| p.as_dvec3()).collect(), idx }
}

fn volume(t: &Tris) -> f64 {
    t.idx.chunks(3).map(|c| t.pos[c[0]].dot(t.pos[c[1]].cross(t.pos[c[2]]))).sum::<f64>() / 6.0
}

/// Whether the result is a closed, consistently wound 2-manifold (it builds
/// into a `Mesh` with no open edges).
fn manifold(t: &Tris) -> bool {
    if t.idx.is_empty() {
        return true;
    }
    let pos: Vec<Vec3> = t.pos.iter().map(|p| p.as_vec3()).collect();
    let idx: Vec<u32> = t.idx.iter().map(|&i| i as u32).collect();
    // Drop unused vertices so the builder accepts the soup.
    let mut used = vec![false; pos.len()];
    idx.iter().for_each(|&i| used[i as usize] = true);
    let mut map = vec![u32::MAX; pos.len()];
    let mut kept = Vec::new();
    for (i, p) in pos.iter().enumerate() {
        if used[i] {
            map[i] = kept.len() as u32;
            kept.push(*p);
        }
    }
    let idx: Vec<u32> = idx.iter().map(|&i| map[i as usize]).collect();
    match Mesh::from_polygons(kept, &vec![3; idx.len() / 3], &idx) {
        Ok(m) => m.edges().all(|e| !m.is_boundary_edge(e)),
        Err(_) => false,
    }
}

trait Engine {
    fn name(&self) -> &'static str;
    fn run(&self, a: &Tris, b: &Tris, op: Op) -> Result<Tris, String>;
}

struct ManifoldRust;

impl Engine for ManifoldRust {
    fn name(&self) -> &'static str {
        "manifold-rust"
    }

    fn run(&self, a: &Tris, b: &Tris, op: Op) -> Result<Tris, String> {
        use manifold_rust::manifold::Manifold;
        use manifold_rust::types::{BooleanEngine, MeshGL64, OpType};
        let to = |t: &Tris| {
            let mesh = MeshGL64 {
                num_prop: 3,
                vert_properties: t.pos.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
                tri_verts: t.idx.iter().map(|&i| i as u64).collect(),
                ..MeshGL64::default()
            };
            Manifold::from_mesh_gl64(&mesh)
        };
        let (ma, mb) = (to(a), to(b));
        let op = match op {
            Op::Union => OpType::Add,
            Op::Difference => OpType::Subtract,
            Op::Intersect => OpType::Intersect,
        };
        let r = ma.boolean_with_engine(&mb, op, BooleanEngine::Exact);
        if r.status() != manifold_rust::types::Error::NoError {
            return Err(format!("{:?}", r.status()));
        }
        let g = r.get_mesh_gl64(-1);
        let n = g.num_prop as usize;
        let pos = g.vert_properties.chunks(n).map(|c| DVec3::new(c[0], c[1], c[2])).collect();
        Ok(Tris { pos, idx: g.tri_verts.iter().map(|&i| i as usize).collect() })
    }
}

struct BoolMesh;

impl Engine for BoolMesh {
    fn name(&self) -> &'static str {
        "boolmesh"
    }

    fn run(&self, a: &Tris, b: &Tris, op: Op) -> Result<Tris, String> {
        use boolmesh::prelude::{Manifold, OpType, compute_boolean};
        let to = |t: &Tris| {
            let pos: Vec<f64> = t.pos.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
            Manifold::new(&pos, &t.idx)
        };
        let (ma, mb) = (to(a)?, to(b)?);
        let op = match op {
            Op::Union => OpType::Add,
            Op::Difference => OpType::Subtract,
            Op::Intersect => OpType::Intersect,
        };
        let r = compute_boolean(&ma, &mb, op)?;
        let pos = r.ps.iter().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
        Ok(Tris { pos, idx: r.hs.iter().map(|h| h.tail).collect() })
    }
}

fn cube(size: f64) -> Mesh {
    cuboid(&BoxSpec { size: DVec3::splat(size), segments: [1; 3], bevel_width: 0.0, bevel_segments: 0 }, Origin::Center).unwrap()
}

fn moved(m: &Mesh, t: Vec3, r: Quat) -> Mesh {
    let mut m = m.clone();
    m.transform(Affine3A::from_rotation_translation(r, t));
    m
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

struct Case {
    name: String,
    a: Tris,
    b: Tris,
    op: Op,
    expect: Option<f64>,
}

fn cases() -> Vec<Case> {
    let c = cube(1.0);
    let mut out = Vec::new();
    let mut add = |name: &str, a: &Mesh, b: &Mesh, op: Op, expect: Option<f64>| {
        out.push(Case { name: name.into(), a: tris(a), b: tris(b), op, expect });
    };
    let off = moved(&c, Vec3::splat(0.5), Quat::IDENTITY);
    add("cube | offset cube", &c, &off, Op::Union, Some(1.875));
    add("cube - offset cube", &c, &off, Op::Difference, Some(0.875));
    add("cube & offset cube", &c, &off, Op::Intersect, Some(0.125));
    add("cube | touching cube", &c, &moved(&c, Vec3::X, Quat::IDENTITY), Op::Union, Some(2.0));
    add("cube | coplanar half", &c, &moved(&c, Vec3::X * 0.5, Quat::IDENTITY), Op::Union, Some(1.5));
    add("cube - coplanar slab", &c, &moved(&cube(1.0), Vec3::new(0.0, 0.75, 0.0), Quat::IDENTITY), Op::Difference, Some(0.75));
    add("cube | same cube", &c, &c, Op::Union, Some(1.0));
    add("cube - same cube", &c, &c, Op::Difference, Some(0.0));
    add("cube | 1e-9 shift", &c, &moved(&c, Vec3::splat(1e-9), Quat::IDENTITY), Op::Union, None);
    add("cube - rotated cube", &c, &moved(&c, Vec3::ZERO, Quat::from_rotation_y(0.4)), Op::Difference, None);
    let s = quad_sphere(&QuadSphereSpec { radius: 0.6, segments: 8 }, Origin::Center).unwrap();
    add("cube - sphere", &c, &s, Op::Difference, None);
    add("sphere & cube", &s, &c, Op::Intersect, None);
    // Random pairs of boxes and spheres, rotated and offset.
    let mut rng = Rng(42);
    for i in 0..200 {
        let pick = |rng: &mut Rng| -> Mesh {
            if rng.next() < 0.5 {
                cube(0.3 + f64::from(rng.next()))
            } else {
                ico_sphere(&IcoSphereSpec { radius: 0.3 + f64::from(rng.next()) * 0.5, subdivisions: 1 + (rng.next() * 2.0) as u32 }, Origin::Center)
                    .unwrap()
            }
        };
        let a = pick(&mut rng);
        let b = pick(&mut rng);
        let t = Vec3::new(rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5);
        let r = Quat::from_euler(glam::EulerRot::XYZ, rng.next() * 3.0, rng.next() * 3.0, rng.next() * 3.0);
        let op = [Op::Union, Op::Difference, Op::Intersect][i % 3];
        add(&format!("random {i}"), &a, &moved(&b, t, r), op, None);
    }
    out
}

fn main() {
    let engines: [&dyn Engine; 2] = [&ManifoldRust, &BoolMesh];
    let cases = cases();
    println!("\n== correctness ({} cases) ==", cases.len());
    for e in engines {
        let (mut ok, mut failed, mut not_manifold, mut wrong_volume, mut inconsistent) = (0, 0, 0, 0, 0);
        let mut notes = Vec::new();
        let mut time = Duration::ZERO;
        for c in &cases {
            let start = Instant::now();
            let r = e.run(&c.a, &c.b, c.op);
            time += start.elapsed();
            match r {
                Err(msg) => {
                    failed += 1;
                    if notes.len() < 6 {
                        notes.push(format!("{}: error {msg}", c.name));
                    }
                }
                Ok(t) => {
                    let v = volume(&t);
                    let good_shape = manifold(&t);
                    if !good_shape {
                        not_manifold += 1;
                        if notes.len() < 6 {
                            notes.push(format!("{}: result is not a closed manifold", c.name));
                        }
                    }
                    if let Some(x) = c.expect
                        && (v - x).abs() > 1e-6
                    {
                        wrong_volume += 1;
                        notes.push(format!("{}: volume {v:.6}, expected {x}", c.name));
                    }
                    // Inclusion and exclusion: |A| + |B| = |A u B| + |A n B|.
                    if c.op == Op::Union
                        && let Ok(i) = e.run(&c.a, &c.b, Op::Intersect)
                    {
                        let err = (volume(&c.a) + volume(&c.b) - v - volume(&i)).abs();
                        if err > 1e-6 * (volume(&c.a) + volume(&c.b)) {
                            inconsistent += 1;
                            if notes.len() < 8 {
                                notes.push(format!("{}: union and intersection disagree by {err:.2e}", c.name));
                            }
                        }
                    }
                    if good_shape && !c.expect.is_some_and(|x| (v - x).abs() > 1e-6) {
                        ok += 1;
                    }
                }
            }
        }
        println!(
            "{:<14} ok {ok:>3}  errors {failed:>3}  not manifold {not_manifold:>3}  wrong volume {wrong_volume:>3}  inconsistent {inconsistent:>3}  total time {:.1} ms",
            e.name(),
            time.as_secs_f64() * 1e3
        );
        for n in notes {
            println!("    {n}");
        }
    }
    println!("\n== speed: union of two offset spheres of n triangles each ==");
    for n in [4_000usize, 16_000, 64_000, 256_000] {
        let a = p3d_testdata::sphere(n);
        let b = moved(&a, Vec3::new(0.3, 0.1, 0.05), Quat::from_rotation_y(0.2));
        let (ta, tb) = (tris(&a), tris(&b));
        let mut row = format!("{:>7} tris", ta.idx.len() / 3);
        for e in engines {
            let start = Instant::now();
            let r = e.run(&ta, &tb, Op::Union);
            let ms = start.elapsed().as_secs_f64() * 1e3;
            let status = match &r {
                Ok(t) if manifold(t) => "ok",
                Ok(_) => "not manifold",
                Err(_) => "error",
            };
            row.push_str(&format!("   {}: {ms:>8.1} ms ({status})", e.name()));
        }
        println!("{row}");
    }
}
