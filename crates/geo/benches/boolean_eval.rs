//! Boolean correctness and speed check for engine upgrades (ADR 0008):
//! `p3d_geo::boolean` on manifold-rust over named cases (offset, touching,
//! coplanar, identical and nearly identical cubes, spheres) and 200 random
//! pairs, checking closed results, volumes and inclusion-exclusion, then
//! timing unions of growing spheres. Run with
//! `cargo bench -p p3d-geo --bench boolean_eval`.

// A measurement program, not library code: it reports with println and stops on setup errors.
#![allow(clippy::print_stdout, clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use glam::{Affine3A, DVec3, Quat, Vec3};
use p3d_geo::boolean::{BoolOp, BooleanError, BooleanSpec, ManifoldEngine, boolean};
use p3d_geo::prim::{BoxSpec, IcoSphereSpec, Origin, QuadSphereSpec, cuboid, ico_sphere, quad_sphere};
use p3d_mesh::Mesh;

fn volume(m: &Mesh) -> f64 {
    m.faces()
        .flat_map(|f| m.face_triangles(f))
        .map(|t| {
            let [a, b, c] = t.map(|h| m.positions[m.origin(h) as usize].as_dvec3());
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn closed(m: &Mesh) -> bool {
    m.edges().all(|e| !m.is_boundary_edge(e))
}

/// The result's volume, 0 for an empty result.
fn run(a: &Mesh, b: &Mesh, op: BoolOp) -> Result<Option<Mesh>, BooleanError> {
    match boolean(a, &[b], &BooleanSpec { op, ..BooleanSpec::default() }, &ManifoldEngine) {
        Ok(r) => Ok(Some(r.mesh)),
        Err(BooleanError::Empty) => Ok(None),
        Err(e) => Err(e),
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
    a: Mesh,
    b: Mesh,
    op: BoolOp,
    expect: Option<f64>,
}

fn cases() -> Vec<Case> {
    let c = cube(1.0);
    let mut out = Vec::new();
    let mut add = |name: &str, a: &Mesh, b: &Mesh, op: BoolOp, expect: Option<f64>| {
        out.push(Case { name: name.into(), a: a.clone(), b: b.clone(), op, expect });
    };
    let off = moved(&c, Vec3::splat(0.5), Quat::IDENTITY);
    add("cube | offset cube", &c, &off, BoolOp::Union, Some(1.875));
    add("cube - offset cube", &c, &off, BoolOp::Difference, Some(0.875));
    add("cube & offset cube", &c, &off, BoolOp::Intersect, Some(0.125));
    add("cube | touching cube", &c, &moved(&c, Vec3::X, Quat::IDENTITY), BoolOp::Union, Some(2.0));
    add("cube | coplanar half", &c, &moved(&c, Vec3::X * 0.5, Quat::IDENTITY), BoolOp::Union, Some(1.5));
    add("cube - coplanar slab", &c, &moved(&cube(1.0), Vec3::new(0.0, 0.75, 0.0), Quat::IDENTITY), BoolOp::Difference, Some(0.75));
    add("cube | same cube", &c, &c, BoolOp::Union, Some(1.0));
    add("cube - same cube", &c, &c, BoolOp::Difference, Some(0.0));
    add("cube | 1e-9 shift", &c, &moved(&c, Vec3::splat(1e-9), Quat::IDENTITY), BoolOp::Union, None);
    add("cube - rotated cube", &c, &moved(&c, Vec3::ZERO, Quat::from_rotation_y(0.4)), BoolOp::Difference, None);
    let s = quad_sphere(&QuadSphereSpec { radius: 0.6, segments: 8 }, Origin::Center).unwrap();
    add("cube - sphere", &c, &s, BoolOp::Difference, None);
    add("sphere & cube", &s, &c, BoolOp::Intersect, None);
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
        let op = [BoolOp::Union, BoolOp::Difference, BoolOp::Intersect][i % 3];
        add(&format!("random {i}"), &a, &moved(&b, t, r), op, None);
    }
    out
}

fn main() {
    let cases = cases();
    println!("\n== correctness ({} cases) ==", cases.len());
    let (mut ok, mut failed, mut open, mut wrong_volume, mut inconsistent) = (0, 0, 0, 0, 0);
    let mut notes = Vec::new();
    let mut time = Duration::ZERO;
    for c in &cases {
        let start = Instant::now();
        let r = run(&c.a, &c.b, c.op);
        time += start.elapsed();
        let m = match r {
            Err(e) => {
                failed += 1;
                notes.push(format!("{}: error {e}", c.name));
                continue;
            }
            Ok(m) => m,
        };
        let v = m.as_ref().map_or(0.0, volume);
        let good = m.as_ref().is_none_or(closed);
        if !good {
            open += 1;
            notes.push(format!("{}: the result is not closed", c.name));
        }
        let wrong = c.expect.is_some_and(|x| (v - x).abs() > 1e-5);
        if wrong {
            wrong_volume += 1;
            notes.push(format!("{}: volume {v:.6}, expected {:?}", c.name, c.expect));
        }
        // Inclusion and exclusion: |A| + |B| = |A u B| + |A n B|.
        if c.op == BoolOp::Union
            && let Ok(i) = run(&c.a, &c.b, BoolOp::Intersect)
        {
            let total = volume(&c.a) + volume(&c.b);
            let err = (total - v - i.as_ref().map_or(0.0, volume)).abs();
            if err > 1e-5 * total {
                inconsistent += 1;
                notes.push(format!("{}: union and intersection disagree by {err:.2e}", c.name));
            }
        }
        if good && !wrong {
            ok += 1;
        }
    }
    println!(
        "ok {ok:>3}  errors {failed:>3}  not closed {open:>3}  wrong volume {wrong_volume:>3}  inconsistent {inconsistent:>3}  total time {:.1} ms",
        time.as_secs_f64() * 1e3
    );
    for n in notes.iter().take(12) {
        println!("    {n}");
    }
    println!("\n== speed: union of two offset spheres of n triangles each, with cleanup ==");
    for n in [4_000usize, 16_000, 64_000, 256_000] {
        let a = p3d_testdata::sphere(n);
        let b = moved(&a, Vec3::new(0.3, 0.1, 0.05), Quat::from_rotation_y(0.2));
        let start = Instant::now();
        let r = run(&a, &b, BoolOp::Union);
        let ms = start.elapsed().as_secs_f64() * 1e3;
        let status = match &r {
            Ok(Some(m)) if closed(m) => format!("ok, {} faces", m.face_count()),
            Ok(_) => "not closed".to_owned(),
            Err(e) => format!("error {e}"),
        };
        println!("{:>7} faces: {ms:>8.1} ms ({status})", a.face_count());
    }
}
