//! Contextual actions, generated from the op registry instead of listed by
//! hand: what can run on the current selection follows from each op's
//! category, flags and parameter schema.
//!
//! - Object Mode, meshes selected: ops with a `target` that need nothing
//!   else (no other required parameter), except curve ops and generators
//!   that create objects (array and mirror change the mesh, so they count).
//!   With two meshes and one active: ops that take a `source` and a
//!   `target` (scatter the others onto the active one).
//! - Object Mode, curves selected: curve ops with a `target`, generators
//!   that take a `curve` (lathe, extruded shape, ...), a `path` (sweep,
//!   pipe; a required profile starts as a small circle) or, with two curves
//!   or more, `sections` (loft).
//! - Edit Mode, elements picked: ops whose `select` takes element ids.
//!
//! Hovering an action previews its result: the op runs on a working copy
//! of the document on a background thread and the viewport shows it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use p3d_engine::OpCall;
use p3d_ops::{Category, OpFlags};
use p3d_scene::Document;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::icons::Icon;

/// How long the pointer rests on an action before its preview runs.
const PREVIEW_DELAY: Duration = Duration::from_millis(350);

/// What an action passes the op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// `target`, once per selected object.
    Target,
    /// `curve`: the selected curve, for generators.
    Curve,
    /// `path`: the selected curve, for generators that follow one.
    Path,
    /// `sections`: the selected curves in selection order.
    Sections,
    /// `source` and `target`: each other selected mesh onto the active one.
    SourceOnActive,
    /// `target` and `tool`: the active mesh, worked on with the other
    /// selected meshes (booleans).
    ToolsOnActive,
    /// `target` and `select`: the picked elements.
    Elements,
}

/// The cross section a sweep starts with when the op needs one.
const DEFAULT_PROFILE: &str = "2cm";

/// One action: the op, its label and summary, its group and icon.
#[derive(Clone, Debug)]
pub struct Action {
    pub op: &'static str,
    pub label: String,
    pub summary: String,
    pub category: Category,
    pub icon: Icon,
    pub input: Input,
    /// Parameters the action sets (a boolean's `operation`), or null.
    pub preset: Value,
    /// The order of an op's variants (difference, union, intersect).
    pub variant: u8,
    /// The keyboard shortcut in `Cmd+Shift+Z` notation, or empty.
    pub keys: &'static str,
}

impl Action {
    /// A key unique among the actions (the op and its variant).
    pub fn key(&self) -> String {
        if self.preset.is_null() { self.op.to_owned() } else { format!("{}#{}", self.op, self.variant) }
    }
}

/// One value of a parameter offered as its own action.
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    pub value: &'static str,
    pub label: &'static str,
    pub icon: Icon,
    /// What it does, for tooltips and the palette.
    pub desc: &'static str,
    /// The shortcut in `Cmd+Shift+Z` notation.
    pub keys: &'static str,
}

/// Ops offered once per value of one parameter: the op, the parameter,
/// and the values.
type Variants = (&'static str, &'static str, &'static [Variant]);

/// The boolean operations, as the menus, shortcuts and actions offer them.
pub const BOOLEANS: &[Variant] = &[
    Variant {
        value: "difference",
        label: "Difference",
        icon: Icon::BoolDifference,
        desc: "Cut the other selected meshes out of the active one",
        keys: "Cmd+Minus",
    },
    Variant { value: "union", label: "Union", icon: Icon::BoolUnion, desc: "Merge the other selected meshes into the active one", keys: "Cmd+Plus" },
    Variant {
        value: "intersect",
        label: "Intersect",
        icon: Icon::BoolIntersect,
        desc: "Keep only where the active mesh and the others overlap",
        keys: "Cmd+*",
    },
];

const VARIANTS: &[Variants] = &[("edit.boolean", "operation", BOOLEANS)];

/// Ops shown first, in this order, when they apply.
const FIRST: &[&str] = &[
    "edit.boolean",
    "edit.extrude",
    "edit.inset",
    "edit.bevel",
    "scene.apply_transform",
    "shade.set_smooth",
    "shade.set_flat",
    "subd.catmull_clark",
    "edit.recalc_normals",
    "edit.fill",
];

/// Categories never offered as actions on a selection.
fn excluded(c: Category) -> bool {
    matches!(c, Category::Prim | Category::Select | Category::Expect)
}

fn category_icon(c: Category) -> Icon {
    match c {
        Category::Scene => Icon::Scene,
        Category::Curve | Category::Gen => Icon::Curve,
        Category::Edit => Icon::Mesh,
        Category::Shade => Icon::ShadeSolid,
        Category::Subd => Icon::Subdivide,
        Category::Deform => Icon::Smooth,
        Category::Cleanup => Icon::CleanUp,
        Category::Io => Icon::Import,
        _ => Icon::Command,
    }
}

/// The icon of an op that has its own, else its category's.
fn op_icon(id: &str, c: Category) -> Icon {
    match id {
        "edit.extrude" => Icon::Extrude,
        "edit.inset" => Icon::Inset,
        "edit.bevel" => Icon::Bevel,
        "edit.loop_cut" => Icon::LoopCut,
        "edit.plane_cut" => Icon::Knife,
        "edit.smooth" => Icon::Smooth,
        "edit.edge_slide" => Icon::EdgeSlide,
        "edit.shrink_fatten" => Icon::ShrinkFatten,
        "edit.split" => Icon::Split,
        "edit.poke" => Icon::Poke,
        "edit.triangulate" => Icon::Triangulate,
        "edit.merge" | "edit.merge_by_distance" => Icon::Merge,
        "edit.dissolve" | "edit.delete_elements" => Icon::Dissolve,
        "edit.subdivide" | "subd.catmull_clark" => Icon::Subdivide,
        "edit.bridge" => Icon::Bridge,
        "edit.fill" => Icon::Fill,
        "edit.symmetrize" => Icon::Symmetrize,
        "edit.recalc_normals" | "edit.flip" => Icon::Normals,
        "scene.apply_transform" => Icon::Apply,
        "scene.duplicate" => Icon::Copy,
        "scene.delete" => Icon::Trash,
        "gen.sweep" => Icon::Sweep,
        "gen.pipe" => Icon::Pipe,
        "gen.loft" => Icon::Loft,
        "gen.array" => Icon::Array,
        "gen.mirror" => Icon::Symmetrize,
        "gen.scatter" => Icon::Scatter,
        "gen.lathe" => Icon::Tube,
        "gen.extrude_shape" => Icon::Extrude,
        "edit.boolean" => Icon::BoolDifference,
        "cleanup.auto" => Icon::CleanUp,
        "cleanup.report" => Icon::Report,
        "io.import" => Icon::Import,
        _ => category_icon(c),
    }
}

/// The group title of a category.
pub fn category_label(c: Category) -> &'static str {
    match c {
        Category::Scene => "Object",
        Category::Curve => "Curve",
        Category::Gen => "Generate",
        Category::Edit => "Mesh",
        Category::Shade => "Shading",
        Category::Subd => "Subdivision",
        Category::Deform => "Deform",
        Category::Cleanup => "Clean up",
        Category::Io => "File",
        _ => "Other",
    }
}

/// What to select before an op can run as an action.
pub fn selection_hint(app: &PixelApp, op: &str) -> String {
    let Ok(o) = app.session.registry().get(op) else { return format!("{} `{op}`", crate::i18n::t("no op")) };
    let schema = o.params_schema();
    let needs = required(&schema);
    let what = if needs.contains(&"source") {
        "select the meshes to place and, last, the mesh to place them on"
    } else if needs.contains(&"tool") {
        "select the meshes to work with and, last, the mesh that keeps the result"
    } else if needs.contains(&"sections") {
        "select two curves or more"
    } else if needs.contains(&"path") || o.meta().category == Category::Curve {
        "select a curve"
    } else if app.ui.mode.is_edit() {
        "select vertices, edges or faces"
    } else {
        "select a mesh"
    };
    format!("{}: {}", o.meta().label, crate::i18n::t(what))
}

/// Public ids of the selected curves, in selection order.
///
/// Complexity: O(s) in selected objects.
fn selected_curves(app: &PixelApp) -> Vec<String> {
    let doc = app.doc();
    app.selection.iter().filter_map(|k| doc.object(*k)).filter(|o| o.curve_key().is_some()).map(|o| o.public_id()).collect()
}

/// Required parameters of a schema.
fn required(schema: &Value) -> Vec<&str> {
    schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// The actions for the current selection, in display order.
///
/// Complexity: O(ops x schema size) (call once per selection change).
pub fn for_context(app: &PixelApp) -> Vec<Action> {
    let doc = app.doc();
    let edit = app.ui.mode.is_edit();
    let picked = edit && !app.edit.picked().0.is_empty();
    let selected: Vec<_> = app.selection.iter().filter_map(|k| doc.object(*k)).collect();
    let meshes = !edit && selected.iter().any(|o| o.mesh_key().is_some());
    let curve_count = if edit { 0 } else { selected.iter().filter(|o| o.curve_key().is_some()).count() };
    let curves = curve_count > 0;
    // Another mesh besides the active one, which is a mesh too.
    let active_mesh = app.active_object().and_then(|k| doc.object(k)).is_some_and(|o| o.mesh_key().is_some());
    let mesh_pair = !edit && active_mesh && selected.iter().filter(|o| o.mesh_key().is_some()).count() >= 2;
    let mut out = Vec::new();
    for op in app.session.registry().iter() {
        let meta = op.meta();
        if excluded(meta.category) {
            continue;
        }
        let schema = op.params_schema();
        let props = schema.get("properties");
        let has = |k: &str| props.and_then(|p| p.get(k)).is_some();
        let needs = required(&schema);
        let only = |allowed: &[&str]| needs.iter().all(|r| allowed.contains(r));
        let input = if edit {
            (picked && has("select") && has("target") && only(&["target", "select"]) && op.id() != "edit.transform").then_some(Input::Elements)
        } else if curves && meta.category == Category::Gen && has("curve") && only(&[]) {
            Some(Input::Curve)
        } else if curves && meta.category == Category::Gen && needs.contains(&"path") && only(&["path", "profile"]) {
            Some(Input::Path)
        } else if curve_count >= 2 && meta.category == Category::Gen && needs == ["sections"] {
            Some(Input::Sections)
        } else if mesh_pair && needs.len() == 2 && only(&["source", "target"]) {
            Some(Input::SourceOnActive)
        } else if mesh_pair && needs.contains(&"tool") && only(&["target", "tool"]) {
            Some(Input::ToolsOnActive)
        } else if has("target") && only(&["target"]) && !needs.contains(&"select") {
            let curve_op = meta.category == Category::Curve;
            // Generators that make new objects need more than a target.
            let makes = meta.category == Category::Gen && meta.flags.contains(OpFlags::CREATES_OBJECTS);
            ((curve_op && curves) || (!curve_op && meshes && !makes)).then_some(Input::Target)
        } else {
            None
        };
        let Some(input) = input else { continue };
        let base = Action {
            op: op.id(),
            label: meta.label.to_owned(),
            summary: meta.summary.to_owned(),
            category: meta.category,
            icon: op_icon(op.id(), meta.category),
            input,
            preset: Value::Null,
            variant: 0,
            keys: "",
        };
        match VARIANTS.iter().find(|(id, _, _)| *id == op.id()) {
            Some((_, param, values)) => {
                for (i, v) in values.iter().enumerate() {
                    out.push(Action {
                        label: crate::i18n::t(v.label).to_owned(),
                        summary: crate::i18n::t(v.desc).to_owned(),
                        icon: v.icon,
                        preset: json!({ *param: v.value }),
                        variant: u8::try_from(i).unwrap_or(u8::MAX),
                        keys: v.keys,
                        ..base.clone()
                    });
                }
            }
            None => out.push(base),
        }
    }
    let rank = |a: &Action| FIRST.iter().position(|f| *f == a.op).unwrap_or(FIRST.len());
    out.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(category_label(a.category).cmp(category_label(b.category)))
            .then(a.variant.cmp(&b.variant))
            .then(a.label.cmp(&b.label))
    });
    out
}

/// The op calls an action makes on the current selection.
pub fn calls(app: &PixelApp, a: &Action) -> Result<Vec<OpCall>, String> {
    let doc = app.doc();
    match a.input {
        Input::Target => {
            let targets = app.selected_targets();
            let calls: Vec<OpCall> = targets.iter().map(|id| OpCall::new(a.op, json!({ "target": id }))).collect();
            if calls.is_empty() { Err(crate::i18n::t("nothing selected").to_owned()) } else { Ok(calls) }
        }
        Input::Curve => {
            let curve = app.selection.iter().filter_map(|k| doc.object(*k)).find(|o| o.curve_key().is_some()).map(|o| o.public_id());
            curve.map(|c| vec![OpCall::new(a.op, json!({ "curve": c }))]).ok_or_else(|| crate::i18n::t("select a curve").to_owned())
        }
        Input::Path => {
            let needs_profile = app.session.registry().get(a.op).is_ok_and(|op| required(&op.params_schema()).contains(&"profile"));
            let curves = selected_curves(app);
            let calls: Vec<OpCall> = curves
                .iter()
                .map(|c| {
                    let mut p = json!({ "path": { "curve": c } });
                    if needs_profile {
                        p["profile"] = json!({ "circle": DEFAULT_PROFILE });
                    }
                    OpCall::new(a.op, p)
                })
                .collect();
            if calls.is_empty() { Err(crate::i18n::t("select a curve").to_owned()) } else { Ok(calls) }
        }
        Input::Sections => {
            let sections: Vec<Value> = selected_curves(app).into_iter().map(|c| json!({ "curve": c })).collect();
            if sections.len() < 2 {
                return Err(crate::i18n::t("select two curves or more").to_owned());
            }
            Ok(vec![OpCall::new(a.op, json!({ "sections": sections }))])
        }
        Input::SourceOnActive => {
            let active = app.active_object().filter(|k| doc.object(*k).is_some_and(|o| o.mesh_key().is_some()));
            let target = active.and_then(|k| doc.object(k)).map(|o| o.public_id()).ok_or(crate::i18n::t("make a mesh active"))?;
            let calls: Vec<OpCall> = app
                .selection
                .iter()
                .filter(|k| Some(**k) != active)
                .filter_map(|k| doc.object(*k))
                .filter(|o| o.mesh_key().is_some())
                .map(|o| OpCall::new(a.op, json!({ "source": o.public_id(), "target": target })))
                .collect();
            if calls.is_empty() { Err(crate::i18n::t("select another mesh besides the active one").to_owned()) } else { Ok(calls) }
        }
        Input::ToolsOnActive => app.tools_on_active_call(a.op, &a.preset).map(|(_, c)| vec![c]),
        Input::Elements => {
            let select = app.element_op_state(a.op)?.ok_or_else(|| crate::i18n::t("Select vertices, edges or faces first.").to_owned())?;
            let id = app.edit.object.and_then(|k| doc.object(k)).map(|o| o.public_id()).ok_or(crate::i18n::t("nothing to edit"))?;
            Ok(vec![OpCall::new(a.op, json!({ "target": id, "select": select }))])
        }
    }
}

/// Runs an action (one undo step).
pub fn run(app: &mut PixelApp, a: &Action) -> Result<Value, String> {
    app.action_preview.clear();
    if a.input == Input::Elements {
        return app.run_element_op(a.op);
    }
    if a.input == Input::ToolsOnActive {
        return app.run_tools_on_active(a.op, &a.preset);
    }
    let calls = calls(app, a)?;
    app.execute(&calls)
}

/// The preview of the action under the pointer.
#[derive(Default)]
pub struct ActionPreview {
    /// The hovered action and since when; the revision it was computed for.
    hovered: Option<(String, Instant, u64)>,
    running: Option<Receiver<Option<Arc<Document>>>>,
    /// The previewed document and its scene key.
    pub doc: Option<(Arc<Document>, u64)>,
    counter: u64,
}

impl ActionPreview {
    pub fn clear(&mut self) {
        self.hovered = None;
        self.running = None;
        self.doc = None;
    }
}

/// Tracks the hovered action (`None` when none is) and runs its preview on
/// a background thread after a short rest; call once per frame.
///
/// Complexity: O(1) per frame; the preview is one op run on a working copy.
pub fn hover(app: &mut PixelApp, hovered: Option<&Action>, ctx: &egui::Context) {
    let Some(a) = hovered else {
        if app.action_preview.hovered.is_some() {
            app.action_preview.clear();
        }
        return;
    };
    let revision = app.session.revision();
    let key = a.key();
    let same = app.action_preview.hovered.as_ref().is_some_and(|(k, _, r)| *k == key && *r == revision);
    if !same {
        app.action_preview.clear();
        app.action_preview.hovered = Some((key, Instant::now(), revision));
    }
    if let Some(rx) = &app.action_preview.running {
        match rx.try_recv() {
            Ok(doc) => {
                app.action_preview.running = None;
                app.action_preview.counter += 1;
                app.action_preview.doc = doc.map(|d| (d, app.action_preview.counter));
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(30)),
            Err(TryRecvError::Disconnected) => app.action_preview.running = None,
        }
        return;
    }
    let rested = app.action_preview.hovered.as_ref().is_some_and(|(_, since, _)| since.elapsed() >= PREVIEW_DELAY);
    if !rested {
        ctx.request_repaint_after(Duration::from_millis(60));
        return;
    }
    if app.action_preview.doc.is_some() {
        return;
    }
    let Ok(calls) = calls(app, a) else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    let registry = Arc::clone(app.session.registry());
    let doc = Document::clone(app.session.doc());
    let ctx2 = ctx.clone();
    let spawned = std::thread::Builder::new().name("action preview".into()).spawn(move || {
        let mut work = p3d_engine::Session::with_registry(registry);
        work.load(doc);
        let r = work.execute(&calls, &Default::default()).ok().map(|_| Arc::clone(work.doc()));
        // The app may have moved on; nothing to do then.
        let _ = tx.send(r);
        ctx2.request_repaint();
    });
    if spawned.is_ok() {
        app.action_preview.running = Some(rx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_follow_the_selection_and_the_schemas() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        let ids = |a: &[Action]| a.iter().map(|x| x.op).collect::<Vec<_>>();
        let object = for_context(&app);
        let ops = ids(&object);
        assert!(ops.contains(&"scene.apply_transform") && ops.contains(&"shade.set_smooth") && ops.contains(&"subd.catmull_clark"), "{ops:?}");
        assert!(
            !ops.iter().any(|o| o.starts_with("prim.") || o.starts_with("curve.") || *o == "scene.rename"),
            "no creation, curve or input-needing ops: {ops:?}"
        );
        assert!(object.iter().all(|a| a.input == Input::Target));
        // Edit Mode with a face picked: element ops.
        app.run("ui.mode", json!({ "mode": "face" })).unwrap();
        assert!(for_context(&app).is_empty(), "nothing picked yet");
        let face = app.doc().object_mesh(app.doc().resolve("crate").unwrap()).and_then(|m| m.faces().next()).unwrap();
        app.edit.click(Some(face), false);
        let edit = for_context(&app);
        assert!(ids(&edit).contains(&"edit.extrude") && edit.iter().all(|a| a.input == Input::Elements));
        // Running one is one undo step.
        let steps = app.session.history().len();
        let poke = edit.iter().find(|a| a.op == "edit.poke").unwrap().clone();
        run(&mut app, &poke).unwrap();
        assert_eq!(app.session.history().len(), steps + 1);
        // A curve offers its ops and the generators that take a curve.
        app.run("ui.mode", json!({ "mode": "object" })).unwrap();
        app.run("curve.circle", json!({ "name": "rim" })).unwrap();
        app.run("edit.select", json!({ "target": "rim" })).unwrap();
        let curve = for_context(&app);
        assert!(
            curve.iter().any(|a| a.op == "gen.lathe" && a.input == Input::Curve) || curve.iter().any(|a| a.op == "gen.extrude_shape"),
            "{:?}",
            ids(&curve)
        );
        assert!(curve.iter().any(|a| a.op.starts_with("curve.") && a.input == Input::Target), "{:?}", ids(&curve));
    }

    #[test]
    fn generators_take_curves_and_meshes_from_the_selection() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        let find = |app: &PixelApp, op: &str| for_context(app).into_iter().find(|a| a.op == op);
        app.run("prim.box", json!({ "name": "ground", "width": 4, "depth": 4, "height": 0.2 })).unwrap();
        // Array and mirror change the selected mesh; scatter needs two.
        assert_eq!(find(&app, "gen.array").map(|a| a.input), Some(Input::Target));
        assert_eq!(find(&app, "gen.mirror").map(|a| a.input), Some(Input::Target));
        assert!(find(&app, "gen.scatter").is_none());
        let ground = app.doc().resolve("ground").unwrap();
        app.run("prim.box", json!({ "name": "rock", "width": 0.1, "depth": 0.1, "height": 0.1 })).unwrap();
        let rock = app.doc().resolve("rock").unwrap();
        app.selection = [ground, rock].into_iter().collect();
        app.active = Some(ground);
        let scatter = find(&app, "gen.scatter").unwrap();
        assert_eq!(scatter.input, Input::SourceOnActive);
        assert_eq!(calls(&app, &scatter).unwrap()[0].args, json!({ "source": "o:2", "target": "o:1" }));
        run(&mut app, &scatter).unwrap();
        // One curve: sweep (with a starting profile) and pipe follow it.
        app.run("curve.circle", json!({ "name": "rim" })).unwrap();
        let rim = app.doc().resolve("rim").unwrap();
        app.selection = [rim].into_iter().collect();
        let sweep = find(&app, "gen.sweep").unwrap();
        assert_eq!(sweep.input, Input::Path);
        assert_eq!(calls(&app, &sweep).unwrap()[0].args["profile"], json!({ "circle": DEFAULT_PROFILE }));
        assert_eq!(find(&app, "gen.pipe").map(|a| a.input), Some(Input::Path));
        assert!(find(&app, "gen.loft").is_none(), "loft needs two sections");
        run(&mut app, &sweep).unwrap();
        // Two curves: loft through them.
        app.run("curve.circle", json!({ "name": "top", "position": [0, 1, 0] })).unwrap();
        let top = app.doc().resolve("top").unwrap();
        app.selection = [rim, top].into_iter().collect();
        let loft = find(&app, "gen.loft").unwrap();
        assert_eq!(loft.input, Input::Sections);
        let before = app.doc().object_count();
        run(&mut app, &loft).unwrap();
        assert_eq!(app.doc().object_count(), before + 1);
    }

    #[test]
    fn two_meshes_offer_the_booleans_first() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        assert!(!for_context(&app).iter().any(|a| a.op == "edit.boolean"), "one mesh: no boolean");
        app.run("prim.box", json!({ "name": "lid", "position": [0, 0.5, 0] })).unwrap();
        let (c, l) = (app.doc().resolve("crate").unwrap(), app.doc().resolve("lid").unwrap());
        app.selection = [c, l].into_iter().collect();
        app.active = Some(c);
        let acts = for_context(&app);
        let first: Vec<(&str, &str)> = acts.iter().take(3).map(|a| (a.op, a.label.as_str())).collect();
        assert_eq!(first, [("edit.boolean", "Difference"), ("edit.boolean", "Union"), ("edit.boolean", "Intersect")]);
        let union = acts[1].clone();
        assert_eq!((union.input.clone(), union.key()), (Input::ToolsOnActive, "edit.boolean#1".to_owned()));
        assert_eq!(calls(&app, &union).unwrap()[0].args, json!({ "operation": "union", "target": "o:1", "tool": ["o:2"] }));
        run(&mut app, &union).unwrap();
        assert_eq!(app.doc().object_count(), 1);
        assert_eq!(app.selection, [c].into_iter().collect());
    }

    #[test]
    fn hovering_previews_without_changing_the_document() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        app.run("prim.box", json!({ "name": "crate" })).unwrap();
        let a = for_context(&app).into_iter().find(|a| a.op == "subd.catmull_clark").unwrap();
        let ctx = egui::Context::default();
        let revision = app.session.revision();
        for _ in 0..400 {
            hover(&mut app, Some(&a), &ctx);
            if app.action_preview.doc.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let (doc, _) = app.action_preview.doc.clone().expect("a preview");
        let k = doc.resolve("crate").unwrap();
        assert!(doc.object_mesh(k).unwrap().face_count() > 6, "subdivided in the preview");
        assert_eq!(app.session.revision(), revision, "the document is untouched");
        hover(&mut app, None, &ctx);
        assert!(app.action_preview.doc.is_none());
    }
}
