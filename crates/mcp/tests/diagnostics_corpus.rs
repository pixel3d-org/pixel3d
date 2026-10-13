//! The diagnostics corpus (project spec 17.10 and the testing section): a
//! deliberately broken spec or mesh per case, driven through the agent
//! tools as an agent would. Each case asserts the exact finding code on the
//! exact part, its place (a named landmark, or a position within a
//! tolerance), its cause path into the spec, that the answer stays within
//! the token budget, that `explain` predicts the top fix resolves it, and
//! that applying that fix (`diagnose fix`) makes the finding go away.

#![allow(clippy::unwrap_used, clippy::panic)]

use glam::{DVec3, Vec2, Vec3};
use p3d_engine::Session;
use p3d_geo::prim::{BoxSpec, Origin, cuboid};
use p3d_mcp::Agent;
use p3d_mesh::{EdgeFlags, Mesh};
use p3d_perceive::TOKEN_BUDGET;
use p3d_perceive::fmt::approx_tokens;
use p3d_scene::{AssetClass, Document};
use p3d_testdata::scenes;
use serde_json::{Value, json};

/// Where a finding must be.
enum Place {
    Landmark(&'static str),
    /// World position (meters) within a tolerance (meters).
    Near([f32; 3], f32),
}

struct Case {
    name: &'static str,
    build: fn() -> Agent,
    code: &'static str,
    part: &'static str,
    place: Place,
    cause_path: Option<&'static str>,
}

fn call(a: &mut Agent, tool: &str, args: Value) -> (String, Value) {
    let out = a.call(tool, args.clone());
    assert!(!out.is_error, "{tool} {args}: {}", out.text);
    (out.text, out.structured)
}

fn spec_agent(target: &str, spec: Value) -> Agent {
    let mut a = Agent::new(Session::new());
    call(&mut a, "spec", json!({ "action": "set", "target": target, "spec": spec }));
    a
}

fn doc_agent(doc: Document) -> Agent {
    let mut s = Session::new();
    s.load(doc);
    Agent::new(s)
}

fn broken(name: &str) -> Agent {
    doc_agent(scenes::broken_corpus().into_iter().find(|(n, _)| *n == name).unwrap().1)
}

fn head() -> Value {
    json!({ "sdf": [{ "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.075, 0.105, 0.09] }, "name": "cranium" }], "detail": "5mm", "landmarks": "humanoid_head" })
}

fn hair(extra: Value) -> Value {
    let mut p = json!({ "on": "head", "region": ["hairline_front", "temple_l", "nape", "temple_r"], "length": "20cm", "clumps": 60 });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    json!({ "strands": "hair", "params": p })
}

fn ear(length: &str, side: &str) -> Value {
    json!({ "feature": "ear", "params": { "variant": "elf", "length": length }, "anchor": { "on": "head", "landmark": format!("ear_root_{side}") } })
}

fn cases() -> Vec<Case> {
    vec![
        // ---- geometry (meshes) -------------------------------------------------
        Case {
            name: "inverted_normals",
            build: || broken("inverted_normals"),
            code: "INVERTED_NORMALS",
            part: "box",
            place: Place::Near([0.0, 0.2, 0.0], 1e-3),
            cause_path: None,
        },
        Case {
            name: "hole",
            build: || broken("open_boundary"),
            code: "OPEN_BOUNDARY",
            part: "box",
            place: Place::Near([0.0, 0.4, 0.0], 1e-3),
            cause_path: None,
        },
        Case {
            name: "thin_wall",
            build: || broken("thin_wall"),
            code: "THIN_WALL",
            part: "shell",
            place: Place::Near([-0.134, 0.0, 0.1735], 0.005),
            cause_path: None,
        },
        Case {
            name: "self_intersection",
            build: || broken("self_intersection"),
            code: "SELF_INTERSECTION",
            part: "pair",
            place: Place::Near([0.1, 0.25, 0.05], 0.06),
            cause_path: None,
        },
        Case {
            name: "z_fighting",
            build: || broken("z_fighting"),
            code: "Z_FIGHTING",
            part: "label",
            place: Place::Near([0.0, 0.4, 0.0], 0.01),
            cause_path: None,
        },
        // ---- relations (specs) -------------------------------------------------
        Case {
            name: "floating_ear",
            build: || {
                let mut e = ear("6cm", "l");
                e["anchor"]["offset"] = json!([0.03, 0, 0]);
                spec_agent("elf", json!({ "parts": { "head": head(), "ear_l": e } }))
            },
            code: "FLOATING_PART",
            part: "ear_l",
            place: Place::Landmark("head.ear_root_l"),
            cause_path: Some("/parts/ear_l/anchor"),
        },
        // ---- strands -------------------------------------------------------------
        Case {
            name: "penetrating_hair",
            build: || spec_agent("elf", json!({ "parts": { "head": head(), "hair": hair(json!({ "segments": 3, "lift": 0, "volume": 0 })) } })),
            code: "PENETRATING_STRANDS",
            part: "hair",
            place: Place::Near([0.0, 0.16, -0.05], 0.05),
            cause_path: Some("/parts/hair/params/segments"),
        },
        Case {
            name: "bald_spot",
            build: || {
                spec_agent("elf", json!({ "parts": { "head": head(), "hair": hair(json!({ "clumps": 16, "width": "6mm", "length": "12cm" })) } }))
            },
            code: "BALD_SPOT",
            part: "hair",
            place: Place::Near([0.0, 0.17, -0.02], 0.12),
            cause_path: Some("/parts/hair/params/clumps"),
        },
        // ---- intent ----------------------------------------------------------------
        Case {
            name: "asymmetric_mirror",
            build: || spec_agent("elf", json!({ "parts": { "head": head(), "ear_l": ear("6cm", "l"), "ear_r": ear("7.5cm", "r") } })),
            code: "ASYMMETRY",
            part: "ear_r",
            place: Place::Landmark("head.ear_root_r"),
            cause_path: Some("/parts/ear_r/params/length"),
        },
        Case {
            name: "wrong_proportions",
            build: || {
                spec_agent(
                    "elf",
                    json!({ "category": "humanoid_head", "parts": { "head": { "sdf": [{ "ellipsoid": { "at": [0, 0.13, 0], "radii": [0.11, 0.105, 0.09] } }], "detail": "5mm", "landmarks": "category" } } }),
                )
            },
            code: "PROPORTIONS",
            part: "head",
            place: Place::Landmark("head.center"),
            cause_path: Some("/parts/head"),
        },
        Case {
            name: "brief_unmet",
            build: || {
                let mut a = Agent::new(Session::new());
                call(
                    &mut a,
                    "brief",
                    json!({ "action": "set", "name": "barrel", "brief": { "title": "Barrel", "size": { "height": ["0.8m", "1m"] } } }),
                );
                call(
                    &mut a,
                    "spec",
                    json!({ "action": "set", "target": "barrel", "spec": { "brief": "barrel", "parts": { "body": { "primitive": { "cylinder": { "radius": 0.25, "height": 0.6 } } } } } }),
                );
                a
            },
            code: "BRIEF_UNMET",
            part: "body",
            place: Place::Landmark("body.center"),
            cause_path: None,
        },
        Case {
            name: "failing_expectation",
            build: || {
                let mut a = spec_agent(
                    "crate",
                    json!({ "parts": {
                        "box": { "primitive": { "box": { "width": 0.4, "height": 0.4, "depth": 0.4 } } },
                        "lid": { "primitive": { "box": { "width": 0.42, "height": 0.03, "depth": 0.42 } }, "transform": { "position": [0, 0.43, 0] } }
                    } }),
                );
                call(&mut a, "expect", json!({ "action": "add", "expect": { "attached": { "part": "lid", "to": "box" } } }));
                a
            },
            code: "EXPECTATION_FAILED",
            part: "lid",
            place: Place::Landmark("lid.center"),
            cause_path: None,
        },
        // ---- edge flow and deformation ---------------------------------------------
        Case {
            name: "missing_joint_loops",
            build: || {
                spec_agent(
                    "arm",
                    json!({ "parts": { "arm": { "primitive": { "cylinder": { "radius": 0.04, "height": 0.6 } }, "tags": ["deforming"], "landmarks": { "elbow": { "param": [0, 0.5] } } } } }),
                )
            },
            code: "JOINT_LOOPS",
            part: "arm",
            place: Place::Landmark("arm.elbow"),
            cause_path: Some("/parts/arm/primitive/cylinder/advanced/height_segments"),
        },
        Case {
            name: "missing_face_loops",
            build: || {
                let mut h = head();
                h["tags"] = json!(["deforming"]);
                spec_agent("elf", json!({ "parts": { "head": h } }))
            },
            code: "FACE_LOOPS",
            part: "head",
            place: Place::Landmark("head.eye_l"),
            cause_path: Some("/parts/head/sdf/0"),
        },
        Case {
            name: "triangles_in_deforming_part",
            build: || {
                spec_agent(
                    "blob",
                    json!({ "parts": { "body": { "primitive": { "ico_sphere": { "radius": 0.1, "subdivisions": 3 } }, "tags": ["deforming"] } } }),
                )
            },
            code: "DEFORM_TRIANGLES",
            part: "body",
            place: Place::Landmark("body.center"),
            cause_path: Some("/parts/body/primitive"),
        },
        Case {
            name: "poles_in_deforming_part",
            build: || {
                spec_agent(
                    "ball",
                    json!({ "parts": { "body": { "primitive": { "uv_sphere": { "radius": 0.1, "segments": 32, "rings": 48 } }, "tags": ["deforming"] } } }),
                )
            },
            code: "DEFORM_POLES",
            part: "body",
            place: Place::Landmark("body.top"),
            cause_path: Some("/parts/body/primitive"),
        },
        Case {
            name: "density_jump",
            build: || {
                spec_agent(
                    "block",
                    json!({ "parts": { "body": {
                        "primitive": { "box": { "width": 0.4, "height": 0.4, "depth": 0.4 } }, "tags": ["deforming"],
                        "edits": [{ "op": "edit.subdivide", "args": { "levels": 3, "select": { "faces": { "group": "top" } } } }]
                    } } }),
                )
            },
            code: "DENSITY_JUMP",
            part: "body",
            place: Place::Near([0.0, 0.4, 0.0], 0.3),
            cause_path: Some("/parts/body/edits/0"),
        },
        // ---- shading -------------------------------------------------------------------
        Case {
            name: "smoothing_artifact",
            build: || {
                spec_agent(
                    "block",
                    json!({ "parts": { "body": { "primitive": { "box": { "width": 0.4, "height": 0.4, "depth": 0.4 } }, "edits": [{ "op": "shade.clear_sharp" }] } } }),
                )
            },
            code: "SMOOTHING_ARTIFACT",
            part: "body",
            place: Place::Near([0.0, 0.2, 0.0], 0.35),
            cause_path: Some("/parts/body/primitive"),
        },
        Case {
            name: "curved_ngon",
            build: || doc_agent(bent_lid()),
            code: "CURVED_NGON",
            part: "lid",
            place: Place::Near([0.0, 0.31, 0.0], 0.01),
            cause_path: None,
        },
        Case {
            name: "hard_edge_without_seam",
            build: || doc_agent(hard_edged_crate()),
            code: "HARD_EDGE_NO_SEAM",
            part: "crate",
            place: Place::Near([0.0, 0.2, 0.0], 0.25),
            cause_path: None,
        },
        // ---- detail scale ---------------------------------------------------------------
        Case {
            name: "micro_detail",
            build: || {
                spec_agent(
                    "crate",
                    json!({ "parts": {
                        "box": { "primitive": { "box": { "width": 0.5, "height": 0.5, "depth": 0.5 } } },
                        "rivet": { "primitive": { "uv_sphere": { "radius": 0.003 } }, "anchor": { "on": "box", "landmark": "front" } }
                    } }),
                )
            },
            code: "DETAIL_SCALE",
            part: "rivet",
            place: Place::Landmark("box.front"),
            cause_path: Some("/parts/rivet"),
        },
    ]
}

/// A crate lid whose hexagonal top is bent out of its plane.
fn bent_lid() -> Document {
    let mut doc = Document::new();
    scenes::add(&mut doc, "crate", None, scenes::box_mesh([0.4, 0.3, 0.4], 0.0), [0.0; 3]);
    let p: Vec<Vec3> = (0..6)
        .map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 6.0;
            Vec3::new(0.2 * a.cos(), if k % 2 == 0 { 0.02 } else { 0.0 }, 0.2 * a.sin())
        })
        .collect();
    let lid = Mesh::from_polygons(p, &[6], &[0, 1, 2, 3, 4, 5]).unwrap();
    scenes::add(&mut doc, "lid", None, lid, [0.0, 0.3, 0.0]);
    doc
}

/// A crate with UVs box-mapped per side and every edge marked hard: the
/// edges inside each side are hard with UVs running on across them.
fn hard_edged_crate() -> Document {
    let mut m = cuboid(&BoxSpec { size: DVec3::splat(0.4), segments: [2; 3], ..BoxSpec::default() }, Origin::Bottom).unwrap();
    for e in m.edges().collect::<Vec<_>>() {
        m.set_edge_flag(e, EdgeFlags::SHARP, true);
    }
    let layer = m.add_uv_layer().unwrap();
    for f in m.faces().collect::<Vec<_>>() {
        let k = m.face_normal(f).abs().max_position();
        for h in m.face_halfedges(f).collect::<Vec<_>>() {
            let p = m.positions[m.origin(h) as usize];
            m.corner_uv[layer][h as usize] = match k {
                0 => Vec2::new(p.y, p.z),
                1 => Vec2::new(p.x + 1.0, p.z),
                _ => Vec2::new(p.x + 2.0, p.y),
            };
        }
    }
    let mut doc = Document::new();
    scenes::add(&mut doc, "crate", None, m, [0.0; 3]);
    doc
}

fn findings(v: &Value) -> Vec<Value> {
    v["findings"].as_array().cloned().unwrap_or_default()
}

fn find<'a>(all: &'a [Value], code: &str, part: &str) -> Option<&'a Value> {
    all.iter().find(|f| f["code"] == code && f["part"] == part)
}

/// Runs one case; returns what went wrong.
fn run(c: &Case) -> Vec<String> {
    let mut bad = Vec::new();
    let mut a = (c.build)();
    let (text, data) = call(&mut a, "diagnose", json!({}));
    let tokens = approx_tokens(&text);
    if tokens > TOKEN_BUDGET {
        bad.push(format!("diagnose answer is {tokens} tokens"));
    }
    let all = findings(&data);
    let Some(f) = find(&all, c.code, c.part) else {
        let got: Vec<String> = all.iter().map(|f| format!("{} {}", f["code"], f["part"])).collect();
        return vec![format!("no {} on {}; got {got:?}", c.code, c.part)];
    };
    match c.place {
        Place::Landmark(l) => {
            if f["landmark"] != l {
                bad.push(format!("landmark {} (want {l})", f["landmark"]));
            }
        }
        Place::Near(p, tol) => {
            let at = Vec3::new(f["at"][0].as_f64().unwrap() as f32, f["at"][1].as_f64().unwrap() as f32, f["at"][2].as_f64().unwrap() as f32);
            if at.distance(Vec3::from(p)) > tol {
                bad.push(format!("at {at} (want within {tol} m of {p:?})"));
            }
        }
    }
    let cause = f["cause_path"].as_str();
    if cause != c.cause_path {
        bad.push(format!("cause path {cause:?} (want {:?}; cause {})", c.cause_path, f["cause"]));
    }
    if f["measured"].is_number() && !f["unit"].is_string() {
        bad.push("measured without a unit".into());
    }
    // The top fix, predicted.
    let id = f["id"].as_str().unwrap().to_owned();
    let (_, x) = call(&mut a, "diagnose", json!({ "explain": id }));
    let top = &x["finding"]["fixes"][0];
    let predicted = top["predicted"].as_str().unwrap_or("none");
    if !predicted.starts_with("resolved") {
        bad.push(format!("top fix {} {} predicted: {predicted}", top["op"], top["args"]));
    }
    // Applied, and gone.
    call(&mut a, "diagnose", json!({ "fix": id }));
    let (_, after) = call(&mut a, "diagnose", json!({}));
    if let Some(g) = find(&findings(&after), c.code, c.part) {
        bad.push(format!("still there after the fix: {}", g["message"]));
    }
    bad
}

#[test]
fn diagnostics_corpus_passes() {
    let all = cases();
    let mut failures = Vec::new();
    for c in &all {
        let bad = run(c);
        if !bad.is_empty() {
            failures.push(format!("{}:\n    {}", c.name, bad.join("\n    ")));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases fail:\n{}", failures.len(), all.len(), failures.join("\n"));
    // One case per new category, plus the corpus of project spec 17.10.
    for code in [
        "BALD_SPOT",
        "PENETRATING_STRANDS",
        "ASYMMETRY",
        "PROPORTIONS",
        "BRIEF_UNMET",
        "EXPECTATION_FAILED",
        "JOINT_LOOPS",
        "FACE_LOOPS",
        "DEFORM_TRIANGLES",
        "DEFORM_POLES",
        "DENSITY_JUMP",
        "SMOOTHING_ARTIFACT",
        "CURVED_NGON",
        "HARD_EDGE_NO_SEAM",
        "DETAIL_SCALE",
        "FLOATING_PART",
        "INVERTED_NORMALS",
        "OPEN_BOUNDARY",
        "THIN_WALL",
        "SELF_INTERSECTION",
        "Z_FIGHTING",
    ] {
        assert!(all.iter().any(|c| c.code == code), "no case for {code}");
    }
}

#[test]
fn micro_detail_depends_on_the_asset_class() {
    // The same rivet seen from a first-person weapon's distance is 8 px.
    let mut doc = Document::new();
    doc.settings.asset_class = AssetClass::WeaponFp;
    let mut a = doc_agent(doc);
    call(
        &mut a,
        "spec",
        json!({ "action": "set", "target": "crate", "spec": { "parts": {
            "box": { "primitive": { "box": { "width": 0.5, "height": 0.5, "depth": 0.5 } } },
            "rivet": { "primitive": { "uv_sphere": { "radius": 0.003 } }, "anchor": { "on": "box", "landmark": "front" } }
        } } }),
    );
    let (_, data) = call(&mut a, "diagnose", json!({}));
    assert!(find(&findings(&data), "DETAIL_SCALE", "rivet").is_none(), "{data}");
}
