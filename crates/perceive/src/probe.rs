//! `probe`: what is at this spot? A debugger watch for a surface point:
//! part, position, normal, local topology, curvature, thickness, groups,
//! material and the nearest other part.

use glam::Vec3;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Report;
use crate::error::PerceiveError;
use crate::fmt::{deg, mm, point, unit};
use crate::view::{Part, View};

/// Generic landmarks every part has (project spec 17.8).
pub const LANDMARKS: [&str; 7] = ["top", "bottom", "front", "back", "left", "right", "center"];

/// A ray cast at a part from one side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RayFrom {
    /// Side the ray comes from: `+x`, `-x`, `+y`, `-y`, `+z` or `-z`.
    pub from: String,
    /// The other two world coordinates of the ray in meters, in x, y, z order
    /// (for `+x`: [y, z]); the part's center when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f32; 2]>,
}

/// A point on a part's surface, addressed by meaning (project spec 17.8).
/// Give exactly one of `landmark`, `nearest`, `ray` or `group`; the
/// `center` landmark when none is given.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfacePoint {
    /// Part (name, path or id).
    pub on: String,
    /// `top`, `bottom`, `front` (+Z), `back`, `left` (-X), `right`, or
    /// `center`: where a ray through the part's box center toward that side
    /// leaves the part (`center`: the surface point nearest the box center).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landmark: Option<String>,
    /// The surface point nearest to this world position (meters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest: Option<[f32; 3]>,
    /// The first hit of a ray from one side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ray: Option<RayFrom>,
    /// The surface point nearest the area center of a face group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A resolved surface point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Located {
    pub part: usize,
    pub point: Vec3,
    pub face: u32,
}

fn side(name: &str, path: &str) -> Result<(usize, f32), PerceiveError> {
    let s = match name {
        "+x" | "x" => (0, 1.0),
        "-x" => (0, -1.0),
        "+y" | "y" => (1, 1.0),
        "-y" => (1, -1.0),
        "+z" | "z" => (2, 1.0),
        "-z" => (2, -1.0),
        other => {
            return Err(PerceiveError::invalid(path, format!("`{other}` is not a side"))
                .with_suggestions(["+x", "-x", "+y", "-y", "+z", "-z"].map(String::from).to_vec()));
        }
    };
    Ok(s)
}

/// The first surface hit of a ray entering the part from side `(k, s)`
/// along the line through `through`.
fn ray_hit(part: &Part, k: usize, s: f32, through: Vec3) -> Option<(Vec3, u32)> {
    let mut origin = through;
    let reach = part.size().length() + 1.0;
    origin[k] = if s > 0.0 { part.hi[k] + reach } else { part.lo[k] - reach };
    let mut dir = Vec3::ZERO;
    dir[k] = -s;
    part.bvh().raycast(origin, dir, 4.0 * reach).map(|h| (h.point, h.face))
}

fn nearest_on(part: &Part, p: Vec3) -> Option<(Vec3, u32)> {
    part.bvh().closest(p, f32::INFINITY).map(|c| (c.point, c.face))
}

/// Resolves a surface point.
///
/// Complexity: O(log n) typical for landmarks, rays and nearest points;
/// O(group size) for groups.
pub fn locate(view: &View, at: &SurfacePoint) -> Result<Located, PerceiveError> {
    let pi = view.part(&at.on, "/at/on")?;
    let part = &view.parts[pi];
    let given = [at.landmark.is_some(), at.nearest.is_some(), at.ray.is_some(), at.group.is_some()].iter().filter(|&&b| b).count();
    if given > 1 {
        return Err(PerceiveError::invalid("/at", "give only one of landmark, nearest, ray or group"));
    }
    let center = part.center();
    let found = if let Some(n) = at.nearest {
        nearest_on(part, Vec3::from(n))
    } else if let Some(r) = &at.ray {
        let (k, s) = side(&r.from, "/at/ray/from")?;
        let mut through = center;
        if let Some([a, b]) = r.at {
            let others: Vec<usize> = (0..3).filter(|&i| i != k).collect();
            through[others[0]] = a;
            through[others[1]] = b;
        }
        ray_hit(part, k, s, through)
    } else if let Some(g) = &at.group {
        let group = part.mesh.group(g).filter(|gr| gr.kind() == p3d_mesh::ElementKind::Face).ok_or_else(|| {
            PerceiveError::new("NOT_FOUND", format!("{} has no face group `{g}`", part.label))
                .at("/at/group")
                .with_suggestions(p3d_scene::name::suggest(g, part.mesh.groups().map(|(n, _)| n), 3))
        })?;
        let (mut sum, mut total) = (Vec3::ZERO, 0.0f32);
        for f in group.iter() {
            let a = part.mesh.face_area(f);
            sum += part.world.transform_point3(part.mesh.face_centroid(f)) * a;
            total += a;
        }
        if total <= 0.0 {
            return Err(PerceiveError::new("EMPTY_SELECTION", format!("group `{g}` has no area")).at("/at/group"));
        }
        nearest_on(part, sum / total)
    } else {
        let name = at.landmark.as_deref().unwrap_or("center");
        match name {
            "center" => nearest_on(part, center),
            "top" => ray_hit(part, 1, 1.0, center).or_else(|| nearest_on(part, Vec3::new(center.x, part.hi.y, center.z))),
            "bottom" => ray_hit(part, 1, -1.0, center).or_else(|| nearest_on(part, Vec3::new(center.x, part.lo.y, center.z))),
            "front" => ray_hit(part, 2, 1.0, center).or_else(|| nearest_on(part, Vec3::new(center.x, center.y, part.hi.z))),
            "back" => ray_hit(part, 2, -1.0, center).or_else(|| nearest_on(part, Vec3::new(center.x, center.y, part.lo.z))),
            "right" => ray_hit(part, 0, 1.0, center).or_else(|| nearest_on(part, Vec3::new(part.hi.x, center.y, center.z))),
            "left" => ray_hit(part, 0, -1.0, center).or_else(|| nearest_on(part, Vec3::new(part.lo.x, center.y, center.z))),
            other => {
                return Err(PerceiveError::new("NOT_FOUND", format!("no landmark `{other}`"))
                    .at("/at/landmark")
                    .with_suggestions(p3d_scene::name::suggest(other, LANDMARKS, 3))
                    .with_hint(format!("generic landmarks: {}", LANDMARKS.join(", "))));
            }
        }
    };
    let (point, face) = found.ok_or_else(|| PerceiveError::new("EMPTY_SELECTION", "the probe missed the part").at("/at"))?;
    Ok(Located { part: pi, point, face })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Probe {
    pub part: String,
    pub id: u32,
    pub point: Vec3,
    pub normal: Vec3,
    /// Corners of the face there (3 triangle, 4 quad, more n-gon).
    pub face_corners: usize,
    pub face_groups: Vec<String>,
    pub corner_valences: Vec<usize>,
    pub mean_edge: f32,
    /// Largest angle between the face and its neighbours, degrees.
    pub max_bend: f32,
    pub sharp_edges: usize,
    pub on_boundary: bool,
    /// Distance through the part along the inward normal.
    pub thickness: Option<f32>,
    pub material: u16,
    pub uv_layers: usize,
    /// Nearest other part within a quarter of the view's size: name and gap.
    pub nearest_part: Option<(String, f32)>,
}

/// What is at a surface point.
///
/// Complexity: O(log n) per BVH query, O(face size x valence) locally,
/// O(k) over the k parts near the point.
pub fn probe(view: &View, at: &SurfacePoint) -> Result<Probe, PerceiveError> {
    let loc = locate(view, at)?;
    let part = &view.parts[loc.part];
    let m = part.mesh;
    let f = loc.face;
    let normal_m = part.world.matrix3.inverse().transpose();
    let normal = (normal_m * m.face_normal(f)).normalize_or_zero();
    let w = |v: u32| part.world.transform_point3(m.positions[v as usize]);
    let hs: Vec<u32> = m.face_halfedges(f).collect();
    let mean_edge = hs.iter().map(|&h| w(m.origin(h)).distance(w(m.dest(h)))).sum::<f32>() / hs.len().max(1) as f32;
    let mut max_bend = 0.0f32;
    let mut on_boundary = false;
    let mut sharp_edges = 0;
    for &h in &hs {
        if m.edge_flags(h >> 1).contains(p3d_mesh::EdgeFlags::SHARP) {
            sharp_edges += 1;
        }
        match m.face(h ^ 1) {
            Some(g) => max_bend = max_bend.max(m.face_normal(f).angle_between(m.face_normal(g)).to_degrees()),
            None => on_boundary = true,
        }
    }
    let eps = part.size().length() * 1e-5;
    let thickness = part.bvh().raycast(loc.point - normal * eps, -normal, f32::INFINITY).map(|h| h.t + eps);
    let reach = 0.25 * view.diagonal();
    let mut nearest_part: Option<(String, f32)> = None;
    for (j, other) in view.parts.iter().enumerate() {
        if j == loc.part {
            continue;
        }
        let gap = (other.lo - loc.point).max(loc.point - other.hi).max(Vec3::ZERO).length();
        if gap > reach || nearest_part.as_ref().is_some_and(|(_, d)| gap > *d) {
            continue;
        }
        if let Some(c) = other.bvh().closest(loc.point, reach)
            && nearest_part.as_ref().is_none_or(|(_, d)| c.dist < *d)
        {
            nearest_part = Some((other.label.clone(), c.dist));
        }
    }
    Ok(Probe {
        part: part.label.clone(),
        id: part.id,
        point: loc.point,
        normal,
        face_corners: hs.len(),
        face_groups: m.groups().filter(|(_, g)| g.kind() == p3d_mesh::ElementKind::Face && g.contains(f)).map(|(n, _)| n.to_owned()).collect(),
        corner_valences: hs.iter().map(|&h| m.valence(m.origin(h))).collect(),
        mean_edge,
        max_bend: if max_bend.is_finite() { max_bend } else { 0.0 },
        sharp_edges,
        on_boundary,
        thickness,
        material: m.face_material(f),
        uv_layers: m.uv_layer_count(),
        nearest_part,
    })
}

fn describe_point(at: &SurfacePoint) -> String {
    if let Some(n) = at.nearest {
        format!("{} nearest {}", at.on, point(Vec3::from(n)))
    } else if let Some(r) = &at.ray {
        format!("{} ray from {}", at.on, r.from)
    } else if let Some(g) = &at.group {
        format!("{} group {g}", at.on)
    } else {
        format!("{}.{}", at.on, at.landmark.as_deref().unwrap_or("center"))
    }
}

/// Compact text of a probe.
pub fn probe_text(at: &SurfacePoint, p: &Probe) -> String {
    let kind = match p.face_corners {
        3 => "triangle".to_owned(),
        4 => "quad".to_owned(),
        n => format!("{n}-gon"),
    };
    let mut t = format!("probe {} at {} mm\n", describe_point(at), point(p.point));
    t.push_str(&format!("  part {} (o:{}), normal {}\n", p.part, p.id, unit(p.normal)));
    let groups = if p.face_groups.is_empty() { String::new() } else { format!(" in {}", p.face_groups.join(", ")) };
    let vals: Vec<String> = p.corner_valences.iter().map(|v| v.to_string()).collect();
    t.push_str(&format!("  face {kind}{groups}; corner valences {}; edges about {} mm\n", vals.join(" "), mm(p.mean_edge)));
    let surface = if p.max_bend < 1.0 {
        "in a flat area".to_owned()
    } else {
        format!("bends up to {} deg into its neighbours ({})", deg(p.max_bend), if p.max_bend < 30.0 { "soft" } else { "hard edge" })
    };
    let mut line = format!("  face {surface}");
    if p.sharp_edges > 0 {
        line.push_str(&format!(", {} sharp-marked edge{}", p.sharp_edges, if p.sharp_edges == 1 { "" } else { "s" }));
    }
    if p.on_boundary {
        line.push_str(", on an open border");
    }
    t.push_str(&line);
    t.push('\n');
    let thick = p.thickness.map_or("thickness: no surface behind (open)".to_owned(), |d| format!("thickness {} mm", mm(d)));
    let uv = if p.uv_layers == 0 { "no UVs".to_owned() } else { format!("{} UV layer{}", p.uv_layers, if p.uv_layers == 1 { "" } else { "s" }) };
    t.push_str(&format!("  {thick}; material slot {}; {uv}\n", p.material));
    match &p.nearest_part {
        Some((n, d)) => t.push_str(&format!("  nearest other part: {n} at {} mm\n", mm(*d))),
        None => t.push_str("  no other part nearby\n"),
    }
    t
}

/// The `probe` report.
pub fn report(view: &View, at: &SurfacePoint) -> Result<Report, PerceiveError> {
    let p = probe(view, at)?;
    Ok(Report::new("probe", probe_text(at, &p), &p))
}
