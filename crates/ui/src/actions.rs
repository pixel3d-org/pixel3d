//! Contextual actions, generated from the op registry instead of listed by
//! hand: what can run on the current selection follows from each op's
//! category, flags and parameter schema.
//!
//! - Object Mode, meshes selected: ops with a `target` that need nothing
//!   else (no other required parameter), except curve ops.
//! - Object Mode, curves selected: curve ops with a `target`, and
//!   generators that take a `curve` (lathe, extruded shape, ...).
//! - Edit Mode, elements picked: ops whose `select` takes element ids.
//!
//! Hovering an action previews its result: the op runs on a working copy
//! of the document on a background thread and the viewport shows it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use p3d_engine::OpCall;
use p3d_ops::Category;
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
    /// `target` and `select`: the picked elements.
    Elements,
}

/// One action: the op, its label and summary, its group and icon.
#[derive(Clone, Debug)]
pub struct Action {
    pub op: &'static str,
    pub label: String,
    pub summary: String,
    pub category: Category,
    pub icon: Icon,
    pub input: Input,
}

/// Ops shown first, in this order, when they apply.
const FIRST: &[&str] = &[
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
        _ => "Other",
    }
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
    let curves = !edit && selected.iter().any(|o| o.curve_key().is_some());
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
        } else if has("target") && only(&["target"]) && !needs.contains(&"select") {
            let curve_op = meta.category == Category::Curve;
            ((curve_op && curves) || (!curve_op && meshes && meta.category != Category::Gen)).then_some(Input::Target)
        } else {
            None
        };
        if let Some(input) = input {
            out.push(Action {
                op: op.id(),
                label: meta.label.to_owned(),
                summary: meta.summary.to_owned(),
                category: meta.category,
                icon: op_icon(op.id(), meta.category),
                input,
            });
        }
    }
    let rank = |a: &Action| FIRST.iter().position(|f| *f == a.op).unwrap_or(FIRST.len());
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(category_label(a.category).cmp(category_label(b.category))).then(a.label.cmp(&b.label)));
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
    let same = app.action_preview.hovered.as_ref().is_some_and(|(op, _, r)| op == a.op && *r == revision);
    if !same {
        app.action_preview.clear();
        app.action_preview.hovered = Some((a.op.to_owned(), Instant::now(), revision));
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
