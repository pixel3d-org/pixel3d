//! Wire proxies of SDF shapes for the viewport (a sphere as three circles,
//! a box as its edges, ...), picking them, and changing a shape's numbers
//! by a gizmo transform: moving changes `at` (or a capsule's ends),
//! turning changes `rotate`, scaling changes the radius, radii or size.

use egui::{Pos2, Rect};
use glam::{EulerRot, Mat4, Quat, Vec3};
use p3d_ops::sdf::{MirrorParam, SdfPrimParam, SdfShapeParam};
use serde_json::{Value, json};

use super::shapes::{self, Path};

/// Points per circle.
const CIRCLE: usize = 40;
/// How near a click must be to a proxy line (points).
pub const PICK_RADIUS: f32 = 7.0;

/// A shape's wire proxy in object space.
#[derive(Debug, Clone)]
pub struct Proxy {
    pub path: Path,
    pub lines: Vec<Vec<Vec3>>,
    pub hidden: bool,
}

/// The typed form of a JSON shape (lengths in meters, angles in radians).
pub fn parse(shape: &Value) -> Option<SdfShapeParam> {
    serde_json::from_value(shape.clone()).ok()
}

fn v3(p: &[p3d_ops::params::Length; 3]) -> Vec3 {
    Vec3::new(p[0].0 as f32, p[1].0 as f32, p[2].0 as f32)
}

fn rotation(s: &SdfShapeParam) -> Quat {
    s.options.rotate.map_or(Quat::IDENTITY, |r| Quat::from_euler(EulerRot::XYZ, r[0].0 as f32, r[1].0 as f32, r[2].0 as f32))
}

/// A shape's center in its list's space: `at`, a capsule's middle, or a
/// group's `at` (the origin when unset).
fn center(s: &SdfShapeParam) -> Vec3 {
    match &s.prim {
        SdfPrimParam::Sphere(p) => v3(&p.at),
        SdfPrimParam::Ellipsoid(p) => v3(&p.at),
        SdfPrimParam::Box(p) => v3(&p.at),
        SdfPrimParam::Capsule(p) => (v3(&p.a) + v3(&p.b)) * 0.5,
        SdfPrimParam::Cone(p) => v3(&p.at),
        SdfPrimParam::Torus(p) => v3(&p.at),
        SdfPrimParam::Cylinder(p) => v3(&p.at),
        SdfPrimParam::Group(_) => s.options.at.as_ref().map_or(Vec3::ZERO, v3),
    }
}

/// The transform a group applies to its members: turned around its center.
fn group_frame(s: &SdfShapeParam) -> Mat4 {
    let c = center(s);
    Mat4::from_translation(c) * Mat4::from_quat(rotation(s)) * Mat4::from_translation(-c)
}

fn circle(c: Vec3, u: Vec3, v: Vec3, n: usize) -> Vec<Vec3> {
    (0..=n).map(|i| i as f32 / n as f32 * std::f32::consts::TAU).map(|t| c + u * t.cos() + v * t.sin()).collect()
}

fn arc(c: Vec3, u: Vec3, v: Vec3, n: usize) -> Vec<Vec3> {
    (0..=n).map(|i| i as f32 / n as f32 * std::f32::consts::PI).map(|t| c + u * t.cos() + v * t.sin()).collect()
}

/// Unit vectors perpendicular to `d`.
fn perpendicular(d: Vec3) -> (Vec3, Vec3) {
    let d = d.normalize_or(Vec3::Y);
    let helper = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let u = helper.cross(d).normalize();
    (u, d.cross(u))
}

/// The lines of a primitive around its own origin (before its rotation).
fn prim_lines(p: &SdfPrimParam, c: Vec3) -> Vec<Vec<Vec3>> {
    let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
    let f = |l: p3d_ops::params::Length| l.0 as f32;
    match p {
        SdfPrimParam::Sphere(s) => {
            let r = f(s.radius);
            vec![circle(Vec3::ZERO, x * r, y * r, CIRCLE), circle(Vec3::ZERO, y * r, z * r, CIRCLE), circle(Vec3::ZERO, x * r, z * r, CIRCLE)]
        }
        SdfPrimParam::Ellipsoid(e) => {
            let r = v3(&e.radii);
            vec![
                circle(Vec3::ZERO, x * r.x, y * r.y, CIRCLE),
                circle(Vec3::ZERO, y * r.y, z * r.z, CIRCLE),
                circle(Vec3::ZERO, x * r.x, z * r.z, CIRCLE),
            ]
        }
        SdfPrimParam::Box(b) => {
            let h = v3(&b.size) * 0.5;
            let corner =
                |i: usize| Vec3::new(if i & 1 == 0 { -h.x } else { h.x }, if i & 2 == 0 { -h.y } else { h.y }, if i & 4 == 0 { -h.z } else { h.z });
            let mut out = Vec::with_capacity(12);
            for i in 0..8 {
                for bit in [1, 2, 4] {
                    if i & bit == 0 {
                        out.push(vec![corner(i), corner(i | bit)]);
                    }
                }
            }
            out
        }
        SdfPrimParam::Capsule(k) => {
            let (a, b) = (v3(&k.a) - c, v3(&k.b) - c);
            let (ra, rb) = (f(k.radius), k.radius_b.map_or(f(k.radius), f));
            let d = (b - a).normalize_or(y);
            let (u, v) = perpendicular(d);
            let mut out = vec![circle(a, u * ra, v * ra, CIRCLE), circle(b, u * rb, v * rb, CIRCLE)];
            for (w, s) in [(u, 1.0), (u, -1.0), (v, 1.0), (v, -1.0)] {
                out.push(vec![a + w * ra * s, b + w * rb * s]);
            }
            out.push(arc(a, u * ra, -d * ra, CIRCLE / 2));
            out.push(arc(a, v * ra, -d * ra, CIRCLE / 2));
            out.push(arc(b, u * rb, d * rb, CIRCLE / 2));
            out.push(arc(b, v * rb, d * rb, CIRCLE / 2));
            out
        }
        SdfPrimParam::Cone(k) => {
            let (h, rb, rt) = (f(k.height), f(k.radius_bottom), f(k.radius_top));
            let top = y * h;
            let mut out = vec![circle(Vec3::ZERO, x * rb, z * rb, CIRCLE)];
            if rt > 0.0 {
                out.push(circle(top, x * rt, z * rt, CIRCLE));
            }
            for w in [x, -x, z, -z] {
                out.push(vec![w * rb, top + w * rt]);
            }
            out
        }
        SdfPrimParam::Torus(t) => {
            let (ma, mi) = (f(t.major), f(t.minor));
            let mut out = vec![circle(Vec3::ZERO, x * (ma + mi), z * (ma + mi), CIRCLE)];
            if ma > mi {
                out.push(circle(Vec3::ZERO, x * (ma - mi), z * (ma - mi), CIRCLE));
            }
            for w in [x, -x, z, -z] {
                out.push(circle(w * ma, w * mi, y * mi, CIRCLE / 2));
            }
            out
        }
        SdfPrimParam::Cylinder(k) => {
            let (h, r) = (f(k.height) * 0.5, f(k.radius));
            let mut out = vec![circle(y * h, x * r, z * r, CIRCLE), circle(-y * h, x * r, z * r, CIRCLE)];
            for w in [x, -x, z, -z] {
                out.push(vec![w * r - y * h, w * r + y * h]);
            }
            out
        }
        SdfPrimParam::Group(_) => Vec::new(),
    }
}

fn mirror_axes(s: &SdfShapeParam) -> [bool; 3] {
    let mut m = [false; 3];
    let axes = match &s.options.mirror {
        None => vec![],
        Some(MirrorParam::One(a)) => vec![*a],
        Some(MirrorParam::Many(v)) => v.clone(),
    };
    for a in axes {
        m[a as usize] = true;
    }
    m
}

/// The proxies of every primitive, in object space; mirrored shapes get
/// their reflected copies too. Shapes inside hidden groups are hidden.
///
/// Complexity: O(shapes x points per proxy).
pub fn proxies(list: &[Value]) -> Vec<Proxy> {
    fn walk(list: &[Value], parent: Mat4, hidden: bool, prefix: &mut Path, out: &mut Vec<Proxy>) {
        for (i, v) in list.iter().enumerate() {
            prefix.push(i);
            if let Some(s) = parse(v) {
                let hidden = hidden || s.options.hidden;
                match &s.prim {
                    SdfPrimParam::Group(_) => {
                        if let Some(m) = shapes::members(v) {
                            walk(m, parent * group_frame(&s), hidden, prefix, out);
                        }
                    }
                    p => {
                        let c = center(&s);
                        let frame = parent * Mat4::from_translation(c) * Mat4::from_quat(rotation(&s));
                        let mut lines: Vec<Vec<Vec3>> =
                            prim_lines(p, c).into_iter().map(|l| l.into_iter().map(|q| frame.transform_point3(q)).collect()).collect();
                        let m = mirror_axes(&s);
                        for (axis, on) in m.iter().enumerate() {
                            if *on {
                                let mut flip = Vec3::ONE;
                                flip[axis] = -1.0;
                                let copies: Vec<Vec<Vec3>> = lines.iter().map(|l| l.iter().map(|q| *q * flip).collect()).collect();
                                lines.extend(copies);
                            }
                        }
                        out.push(Proxy { path: prefix.clone(), lines, hidden });
                    }
                }
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(list, Mat4::IDENTITY, false, &mut Vec::new(), &mut out);
    out
}

/// The frame of the shape at `path` in object space: its center and
/// rotation (groups: their `at` and rotation), through its groups.
///
/// Complexity: O(depth).
pub fn frame_of(list: &[Value], path: &[usize]) -> Option<(Vec3, Quat)> {
    let parent = parent_frame(list, path)?;
    let s = parse(shapes::get(list, path)?)?;
    let f = parent * Mat4::from_translation(center(&s)) * Mat4::from_quat(rotation(&s));
    let (_, r, t) = f.to_scale_rotation_translation();
    Some((t, r))
}

/// The transform the groups around `path` apply to it.
fn parent_frame(list: &[Value], path: &[usize]) -> Option<Mat4> {
    let mut m = Mat4::IDENTITY;
    for depth in 1..path.len() {
        let g = parse(shapes::get(list, &path[..depth])?)?;
        m *= group_frame(&g);
    }
    Some(m)
}

/// Visible proxy bounds in object space.
///
/// Complexity: O(proxy points).
pub fn bounds(proxies: &[Proxy]) -> Option<(Vec3, Vec3)> {
    let mut it = proxies.iter().filter(|p| !p.hidden).flat_map(|p| p.lines.iter().flatten().copied());
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

fn project(vp: Mat4, rect: Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(egui::pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

/// The proxies on screen: per proxy, its path, whether hidden, and its
/// projected segments.
///
/// Complexity: O(proxy points).
pub fn screen(proxies: &[Proxy], world: Mat4, vp: Mat4, rect: Rect) -> Vec<(Path, bool, Vec<[Pos2; 2]>)> {
    let to_clip = vp * world;
    proxies
        .iter()
        .map(|p| {
            let mut segs = Vec::new();
            for l in &p.lines {
                let pts: Vec<Option<Pos2>> = l.iter().map(|q| project(to_clip, rect, *q)).collect();
                for w in pts.windows(2) {
                    if let [Some(a), Some(b)] = w {
                        segs.push([*a, *b]);
                    }
                }
            }
            (p.path.clone(), p.hidden, segs)
        })
        .collect()
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// The visible shape whose proxy passes nearest to `at`, within
/// [`PICK_RADIUS`].
///
/// Complexity: O(projected segments).
pub fn pick(on_screen: &[(Path, bool, Vec<[Pos2; 2]>)], at: Pos2) -> Option<Path> {
    on_screen
        .iter()
        .filter(|(_, hidden, _)| !hidden)
        .filter_map(|(path, _, segs)| segs.iter().map(|[a, b]| segment_distance(at, *a, *b)).reduce(f32::min).map(|d| (d, path)))
        .filter(|(d, _)| *d <= PICK_RADIUS)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p.clone())
}

fn len(x: f32) -> Value {
    json!((f64::from(x) * 1e6).round() / 1e6)
}

fn point(p: Vec3) -> Value {
    json!([len(p.x), len(p.y), len(p.z)])
}

fn degrees(q: Quat) -> Value {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    let d = |a: f32| json!((f64::from(a).to_degrees() * 1e4).round() / 1e4);
    json!([d(x), d(y), d(z)])
}

/// Multiplies the length at `key` of `body` by `k`.
fn scale_len(body: &mut Value, key: &str, k: f32) {
    if let Some(x) = body.get(key).and_then(Value::as_f64) {
        body[key] = len(x as f32 * k);
    }
}

/// Moves every position in a list (and its groups) by `t`.
fn translate_all(list: &mut [Value], t: Vec3) {
    for v in list.iter_mut() {
        let Some(key) = shapes::prim_key(v) else { continue };
        match key {
            "group" => {
                if let Some(at) = v.get("at").and_then(|a| serde_json::from_value::<[f32; 3]>(a.clone()).ok()) {
                    v["at"] = point(Vec3::from(at) + t);
                }
                if let Some(m) = v.get_mut("group").and_then(Value::as_array_mut) {
                    translate_all(m, t);
                }
            }
            "capsule" => {
                for end in ["a", "b"] {
                    if let Some(p) = v[key].get(end).and_then(|a| serde_json::from_value::<[f32; 3]>(a.clone()).ok()) {
                        v[key][end] = point(Vec3::from(p) + t);
                    }
                }
            }
            _ => {
                if let Some(p) = v[key].get("at").and_then(|a| serde_json::from_value::<[f32; 3]>(a.clone()).ok()) {
                    v[key]["at"] = point(Vec3::from(p) + t);
                }
            }
        }
    }
}

/// Applies `delta`, a transform in object space, to the shape at `path`:
/// its center moves, its rotation turns, and its sizes scale along its own
/// axes (radius, radii, size, height). Groups move and turn as a whole;
/// scaling a group is left to its members. Returns whether it changed.
///
/// Complexity: O(shapes in a moved group), else O(depth).
pub fn transform(list: &mut [Value], path: &[usize], delta: Mat4) -> bool {
    let Some(parent) = parent_frame(list, path) else { return false };
    let Some(s) = shapes::get(list, path).and_then(parse) else { return false };
    let Some(key) = shapes::get(list, path).and_then(shapes::prim_key) else { return false };
    // The delta in the shape's list space.
    let local = parent.inverse() * delta * parent;
    let r0 = rotation(&s);
    let c0 = center(&s);
    let (scale, rot, trans) = (local * Mat4::from_translation(c0) * Mat4::from_quat(r0)).to_scale_rotation_translation();
    let scale = scale.abs();
    let g = (scale.x * scale.y * scale.z).cbrt();
    let turned = rot.angle_between(r0) > 1e-5;
    let (_, rot_only, _) = local.to_scale_rotation_translation();
    let Some(v) = shapes::get_mut(list, path) else { return false };
    match key {
        "group" => {
            let c1 = local.transform_point3(c0);
            if let Some(m) = v.get_mut("group").and_then(Value::as_array_mut) {
                translate_all(m, c1 - c0);
            }
            if rot_only.angle_between(Quat::IDENTITY) > 1e-5 || v.get("at").is_some() {
                v["at"] = point(c1);
            }
            if turned {
                v["rotate"] = degrees(rot_only * r0);
            }
        }
        "capsule" => {
            let SdfPrimParam::Capsule(k) = &s.prim else { return false };
            let (a, b) = (v3(&k.a), v3(&k.b));
            let (pa, pb) = (c0 + r0 * (a - c0), c0 + r0 * (b - c0));
            let (qa, qb) = (local.transform_point3(pa), local.transform_point3(pb));
            let c1 = (qa + qb) * 0.5;
            let inv = r0.inverse();
            v[key]["a"] = point(c1 + inv * (qa - c1));
            v[key]["b"] = point(c1 + inv * (qb - c1));
            if (g - 1.0).abs() > 1e-6 {
                scale_len(&mut v[key], "radius", g);
                scale_len(&mut v[key], "radius_b", g);
            }
        }
        _ => {
            v[key]["at"] = point(trans);
            if turned {
                v["rotate"] = degrees(rot);
            }
            if (scale - Vec3::ONE).abs().max_element() > 1e-6 {
                let body = &mut v[key];
                let round = (scale.x * scale.z).sqrt();
                match key {
                    "sphere" => scale_len(body, "radius", g),
                    "ellipsoid" | "box" => {
                        let field = if key == "box" { "size" } else { "radii" };
                        if let Some(r) = body.get(field).and_then(|a| serde_json::from_value::<[f32; 3]>(a.clone()).ok()) {
                            body[field] = point(Vec3::from(r) * scale);
                        }
                        scale_len(body, "corner", scale.min_element());
                    }
                    "cone" => {
                        scale_len(body, "height", scale.y);
                        scale_len(body, "radius_bottom", round);
                        scale_len(body, "radius_top", round);
                    }
                    "torus" => {
                        scale_len(body, "major", round);
                        scale_len(body, "minor", g);
                    }
                    "cylinder" => {
                        scale_len(body, "height", scale.y);
                        scale_len(body, "radius", round);
                    }
                    _ => {}
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<Value> {
        vec![
            json!({ "name": "body", "sphere": { "at": [0, 0.1, 0], "radius": 0.1 } }),
            json!({ "name": "ear", "capsule": { "a": [0.05, 0.15, 0], "b": [0.1, 0.2, 0], "radius": 0.01 }, "mirror": "x" }),
            json!({ "name": "legs", "rotate": [0, 0, 90], "at": [0, 0, 0], "group": [
                { "name": "leg", "box": { "at": [0.1, 0, 0], "size": [0.02, 0.04, 0.02] } }
            ] }),
        ]
    }

    #[test]
    fn proxies_follow_shapes_groups_and_mirrors() {
        let p = proxies(&list());
        assert_eq!(p.iter().map(|x| x.path.clone()).collect::<Vec<_>>(), [vec![0], vec![1], vec![2, 0]]);
        let (lo, hi) = bounds(&p[..1]).unwrap();
        assert!((lo - Vec3::new(-0.1, 0.0, -0.1)).length() < 1e-4 && (hi - Vec3::new(0.1, 0.2, 0.1)).length() < 1e-4);
        // The ear is mirrored: lines on both sides of x = 0.
        assert!(p[1].lines.iter().flatten().any(|q| q.x < -0.04) && p[1].lines.iter().flatten().any(|q| q.x > 0.04));
        // The leg is turned 90 degrees about Z by its group: it sits on +Y.
        let (c, _) = frame_of(&list(), &[2, 0]).unwrap();
        assert!((c - Vec3::new(0.0, 0.1, 0.0)).length() < 1e-4, "{c}");
        let (lo, _) = bounds(&p[2..]).unwrap();
        assert!(lo.y > 0.08, "{lo}");
    }

    #[test]
    fn gizmo_moves_turn_and_scale_shapes() {
        let mut l = list();
        assert!(transform(&mut l, &[0], Mat4::from_translation(Vec3::new(0.1, 0.0, 0.0))));
        assert_eq!(l[0]["sphere"]["at"], json!([0.1, 0.1, 0.0]));
        // Scale about the sphere's center.
        let c = Vec3::new(0.1, 0.1, 0.0);
        let around = Mat4::from_translation(c) * Mat4::from_scale(Vec3::splat(2.0)) * Mat4::from_translation(-c);
        transform(&mut l, &[0], around);
        assert_eq!((l[0]["sphere"]["radius"].clone(), l[0]["sphere"]["at"].clone()), (json!(0.2), json!([0.1, 0.1, 0.0])));
        // A capsule moves by its ends.
        transform(&mut l, &[1], Mat4::from_translation(Vec3::Y * 0.1));
        assert_eq!(l[1]["capsule"]["a"], json!([0.05, 0.25, 0.0]));
        // A member of a turned group moves along the group's turned axes:
        // world +Y is the member's +X before the group's 90 degree turn.
        transform(&mut l, &[2, 0], Mat4::from_translation(Vec3::Y * 0.05));
        let at = l[2]["group"][0]["box"]["at"].clone();
        assert_eq!(at, json!([0.15, 0.0, 0.0]), "{at}");
        // Turning a shape writes its rotation in degrees.
        let turn = Mat4::from_translation(Vec3::new(0.1, 0.1, 0.0))
            * Mat4::from_rotation_y(45f32.to_radians())
            * Mat4::from_translation(-Vec3::new(0.1, 0.1, 0.0));
        transform(&mut l, &[0], turn);
        assert_eq!(l[0]["rotate"], json!([0.0, 45.0, 0.0]));
        // Moving a group moves its members.
        transform(&mut l, &[2], Mat4::from_translation(Vec3::Z * 0.1));
        assert_eq!(l[2]["group"][0]["box"]["at"], json!([0.15, 0.0, 0.1]));
    }

    #[test]
    fn clicks_pick_the_nearest_visible_proxy() {
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 200.0));
        let vp = Mat4::IDENTITY;
        let mut l = list();
        let on = screen(&proxies(&l), Mat4::IDENTITY, vp, rect);
        // The body's front circle passes through x = 0.1 at y = 0.1 (screen 110, 90).
        assert_eq!(pick(&on, egui::pos2(110.0, 90.0)), Some(vec![0]));
        assert_eq!(pick(&on, egui::pos2(190.0, 10.0)), None);
        shapes::set_hidden(&mut l[0], true);
        let on = screen(&proxies(&l), Mat4::IDENTITY, vp, rect);
        assert_ne!(pick(&on, egui::pos2(110.0, 90.0)), Some(vec![0]), "hidden shapes are not picked");
    }
}
