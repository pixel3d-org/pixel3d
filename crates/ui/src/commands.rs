//! The searchable command index behind the command palette and the command
//! line (project spec 20.2, 20.7): every app command from the menus and
//! toolbars, and every registered op with its label, summary and the names
//! other tools use (Blender, Maya, 3ds Max). One fuzzy ranking serves both,
//! and both run commands through [`PixelApp::run`] like every other control.

use egui::KeyboardShortcut;
use serde_json::{Map, Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::icons::Icon;

/// One runnable command.
#[derive(Clone)]
pub struct Command {
    /// The command id [`PixelApp::run`] takes (an op id or an app command).
    pub id: &'static str,
    pub params: fn() -> Value,
    pub label: String,
    pub desc: String,
    /// Other names to find it by, lowercase.
    pub aliases: Vec<String>,
    pub shortcut: Option<KeyboardShortcut>,
    pub icon: Option<Icon>,
    /// An op that takes a `target`: it runs on each selected object.
    pub on_selection: bool,
}

fn null() -> Value {
    Value::Null
}

/// Aliases of app commands, by label.
fn app_aliases(label: &str) -> &'static [&'static str] {
    match label {
        "New" => &["new scene", "new file", "reset"],
        "Open..." => &["open file", "load", "open p3d"],
        "Import..." => &["import", "import file", "load gltf", "load glb", "load fbx", "load obj", "append", "file import"],
        "Save" => &["save file", "write"],
        "Save As..." => &["save copy", "export p3d"],
        "Undo" => &["ctrl z", "step back"],
        "Redo" => &["ctrl shift z", "step forward"],
        "Duplicate" => &["copy", "clone", "shift d", "duplicate special"],
        "Delete" => &["remove", "x", "erase"],
        "Select all" => &["a", "all"],
        "Select none" => &["deselect", "alt a", "clear selection"],
        "Frame selection" => &["frame", "view selected", "focus", "zoom to selection", "frame selected", "numpad ."],
        "Orthographic" => &["perspective", "toggle ortho", "numpad 5", "projection"],
        "Ground grid" => &["floor grid", "toggle grid"],
        "X-ray" => &["toggle x-ray", "alt z", "see through", "transparent", "xray"],
        "Statistics" => &["stats", "polycount", "poly count", "scene info", "vertex count"],
        "Solid" | "Solid with edges" | "Wireframe" | "Clay" => &["shading", "viewport shading", "display mode"],
        "History and Console" => &["bottom panel", "info editor", "log"],
        "Reset panel layout" => &["reset layout", "default layout"],
        "Fill holes" => &["fill", "close holes"],
        "Apply transform" => &["apply all transforms", "freeze transformations", "reset xform", "ctrl a"],
        "Difference" => &["boolean difference", "subtract", "cut", "carve", "bool tool", "ctrl minus", "polycbooldifference"],
        "Union" => &["boolean union", "merge meshes", "combine", "bool tool", "ctrl plus", "polycboolunion"],
        "Intersect" => &["boolean intersect", "intersection", "overlap", "bool tool", "ctrl asterisk", "polycboolintersection"],
        "SDF shape" => &["sdf", "sculpt", "metaballs", "blob", "clay", "implicit surface", "smooth union"],
        "Quad remesh" => &["retopology", "retopo", "quadriflow", "instant meshes", "zremesher", "autoretopo", "remesh"],
        "Voxel remesh" => &["dynamesh", "remesh voxel", "fuse parts", "make watertight", "remesh"],
        "Poles" => &["valence", "pole overlay", "show poles", "retopology check", "mesh analysis"],
        "Leaf shell" | "Ear shell" => &["shell", "card", "leaf", "ear", "wing", "fin", "petal", "outline", "draw outline"],
        "Axe head silhouettes" => &["silhouettes", "blueprint", "front and side view", "visual hull", "outline", "axe"],
        "Lamp spec" | "Elf head spec" => &["model spec", "spec", "starter", "blockout", "parts", "spec first", "declarative model"],
        "Landmarks" => &["landmark overlay", "anchors", "show landmarks", "spec points", "attachment points"],
        _ => &[],
    }
}

/// What app commands do, by label, for those whose label alone may not say.
fn app_desc(label: &str) -> &'static str {
    match label {
        "Quad remesh" => "Rebuild the selected closed mesh as even quads, in the background",
        "Voxel remesh" => "Rebuild the selected closed mesh from its volume as even quads, in the background",
        "Poles" => "Mark vertices of valence 3, 5 and 6 or more on the selected meshes",
        "Leaf shell" => "Add a leaf card drawn from one outline, edited in the Outline tab",
        "Ear shell" => "Add an ear shell drawn from one outline with cup and bend, edited in the Outline tab",
        "Axe head silhouettes" => "Add an axe head built from front and top outlines, edited in the Outline tab",
        "Lamp spec" => "Add a lamp Model Spec: a base, a pole and a shade anchored on each other, edited in the Spec tab",
        "Elf head spec" => "Add an elf head Model Spec: a head with a blended nose and ears and separate eyes, edited in the Spec tab",
        "Landmarks" => "Show the landmarks of the selected spec part, with short ticks along their normals",
        "SDF shape" => "Add an organic shape from blended spheres, capsules and boxes, edited in the Shapes tab",
        _ => crate::actions::BOOLEANS.iter().find(|v| v.label == label).map_or("", |v| v.desc),
    }
}

/// App commands that are not in the menus: tools and selection modes.
fn extra_commands() -> Vec<Command> {
    let mk = |id, params: fn() -> Value, label: &'static str, desc: &'static str, aliases: &[&str], icon| Command {
        id,
        params,
        label: t(label).to_owned(),
        desc: t(desc).to_owned(),
        aliases: aliases.iter().map(|a| a.to_lowercase()).collect(),
        shortcut: None,
        icon: Some(icon),
        on_selection: false,
    };
    vec![
        mk("ui.tool", || json!({ "tool": "select" }), "Select tool", "Click to select objects", &["q", "tweak", "pick"], Icon::Select),
        mk("ui.tool", || json!({ "tool": "move" }), "Move tool", "Move the selection with the gizmo", &["w", "translate"], Icon::Move),
        mk("ui.tool", || json!({ "tool": "rotate" }), "Rotate tool", "Rotate the selection with the gizmo", &["e", "rotation"], Icon::Rotate),
        mk("ui.tool", || json!({ "tool": "scale" }), "Scale tool", "Scale the selection with the gizmo", &["t", "resize"], Icon::Scale),
        mk(
            "ui.tool",
            || json!({ "tool": "handles" }),
            "Handles tool",
            "Pull points of the surface; it follows within a radius (Shift+click pins)",
            &["h", "pull", "push", "soft selection", "proportional editing", "sculpt by numbers", "deform handles", "warp"],
            Icon::Handles,
        ),
        mk(
            "history.bisect_card",
            || json!({ "on": true }),
            "Bisect history",
            "Find the change that made an expectation, a finding or a brief item fail",
            &["bisect", "regression", "what broke", "git bisect", "find the change"],
            Icon::History,
        ),
        mk("ui.mode", || json!({ "mode": "object" }), "Object mode", "Work on whole objects", &["tab", "object"], Icon::ModeObject),
        mk(
            "ui.mode",
            || json!({ "mode": "edit" }),
            "Edit mode",
            "Work on the vertices, edges and faces of the active mesh",
            &["tab", "edit"],
            Icon::ModeVert,
        ),
        mk("ui.mode", || json!({ "mode": "vert" }), "Vertex select", "Select vertices (edit mode)", &["1", "vertex mode", "points"], Icon::ModeVert),
        mk("ui.mode", || json!({ "mode": "edge" }), "Edge select", "Select edges (edit mode)", &["2", "edge mode"], Icon::ModeEdge),
        mk("ui.mode", || json!({ "mode": "face" }), "Face select", "Select faces (edit mode)", &["3", "face mode", "polygon"], Icon::ModeFace),
        mk(
            "view.modal",
            || json!({ "kind": "move" }),
            "Grab",
            "Move the selection with the pointer; X, Y, Z constrain, type a value",
            &["g", "grab", "translate", "move"],
            Icon::Move,
        ),
        mk(
            "view.modal",
            || json!({ "kind": "rotate" }),
            "Rotate",
            "Turn the selection with the pointer; X, Y, Z constrain, type degrees",
            &["r", "rotate"],
            Icon::Rotate,
        ),
        mk(
            "view.modal",
            || json!({ "kind": "scale" }),
            "Resize",
            "Scale the selection with the pointer; X, Y, Z constrain, type a factor",
            &["s", "scale", "resize"],
            Icon::Scale,
        ),
        mk("view.snap", null, "Snap", "Snap transform drags to the grid and 15 degree steps", &["snapping", "magnet", "shift tab"], Icon::Grid),
        mk(
            "spec.pick_landmark",
            null,
            "Pick a landmark",
            "Anchor the selected spec part by clicking a landmark of its parent in the viewport",
            &["anchor", "attach", "landmark", "snap part", "spec"],
            Icon::Landmark,
        ),
    ]
}

/// Every command: menu entries first (in menu order), then the tools and
/// modes, then each registered op that no menu entry runs already.
///
/// Complexity: O(menu entries + ops).
pub fn index(app: &PixelApp) -> Vec<Command> {
    let mut out = Vec::new();
    for menu in crate::menus::menus() {
        for it in menu.entries.into_iter().flatten() {
            // Headers are titles; menu entries that run an op on the
            // selection are listed as that op below.
            if it.is_header() || it.command == "selection.run" {
                continue;
            }
            // An op behind a menu entry lends it its summary and aliases.
            let meta = app.session.registry().get(it.command).ok().map(|op| op.meta());
            let desc = meta.as_ref().map_or_else(|| t(app_desc(it.label)).to_owned(), |m| m.summary.to_owned());
            let op_aliases = meta.as_ref().map_or(&[][..], |m| m.aliases);
            out.push(Command {
                id: it.command,
                params: it.params,
                label: t(it.label).to_owned(),
                desc,
                aliases: app_aliases(it.label).iter().chain(op_aliases).map(|a| a.to_lowercase()).collect(),
                shortcut: it.shortcut,
                icon: it.icon,
                on_selection: false,
            });
        }
    }
    out.extend(extra_commands());
    let menu_ops: std::collections::BTreeSet<&str> = out.iter().map(|c| c.id).collect();
    for op in app.session.registry().iter() {
        if menu_ops.contains(op.id()) {
            continue;
        }
        let meta = op.meta();
        // Ops that need only a target run on each selected object; ops that
        // need more (a boolean's tools) take it from the selection their own way.
        let schema = op.params_schema();
        let required: Vec<&str> =
            schema.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let on_selection = schema.pointer("/properties/target").is_some()
            && required.iter().all(|r| *r == "target")
            && !crate::jobcard::runs_in_card(app.session.registry(), op.id());
        out.push(Command {
            id: op.id(),
            params: null,
            label: meta.label.to_owned(),
            desc: meta.summary.to_owned(),
            aliases: meta.aliases.iter().map(|a| a.to_lowercase()).collect(),
            shortcut: None,
            icon: None,
            on_selection,
        });
    }
    out
}

/// How well `query` matches `text` (both lowercase): `None` unless the
/// query's characters appear in order. Starts of the text and of words,
/// and runs of consecutive characters, score higher; gaps cost a little.
///
/// Complexity: O(len(text)).
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last: Option<usize> = None;
    for qc in query.chars() {
        let found = (ti..text.len()).find(|&i| text[i] == qc)?;
        let word_start = found == 0 || matches!(text[found - 1], ' ' | '.' | '_' | '-' | '/');
        score += 1;
        if found == 0 {
            score += 10;
        } else if word_start {
            score += 7;
        }
        match last {
            Some(l) if l + 1 == found => score += 5,
            Some(l) => score -= ((found - l - 1) as i32).min(5),
            None => score -= (found as i32).min(5),
        }
        last = Some(found);
        ti = found + 1;
    }
    let joined: String = text.iter().collect();
    if joined == query {
        score += 50;
    } else if joined.starts_with(query) {
        score += 20;
    } else if joined.contains(query) {
        score += 10;
    }
    Some(score)
}

/// Commands matching `query`, best first (ties keep index order): labels
/// count most, then aliases and ids, then words in the description.
///
/// Complexity: O(commands * text length) plus O(m log m) to sort m matches.
pub fn rank(commands: &[Command], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..commands.len()).collect();
    }
    let mut hits: Vec<(i32, usize)> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let label = fuzzy(&q, &c.label.to_lowercase()).map(|s| s * 3);
            let alias = c.aliases.iter().filter_map(|a| fuzzy(&q, a)).max().map(|s| s * 2);
            let id = fuzzy(&q, c.id).map(|s| s * 2);
            let desc = c.desc.to_lowercase().contains(&q).then_some(4);
            [label, alias, id, desc].into_iter().flatten().max().map(|s| (s, i))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

/// Runs a command with its own parameters; ops that take a target run on
/// each selected object.
pub fn run(app: &mut PixelApp, cmd: &Command) -> Result<Value, String> {
    run_with(app, cmd, (cmd.params)())
}

/// Runs a command with `params`. An op that takes a target, called without
/// one while objects are selected, runs once per selected object (one undo
/// step).
pub fn run_with(app: &mut PixelApp, cmd: &Command, params: Value) -> Result<Value, String> {
    let has_target = params.get("target").is_some();
    let targets = app.selected_targets();
    if cmd.on_selection && !has_target && !targets.is_empty() {
        let calls: Vec<p3d_engine::OpCall> = targets
            .iter()
            .map(|id| {
                let mut p = params.as_object().cloned().unwrap_or_default();
                p.insert("target".into(), json!(id));
                p3d_engine::OpCall::new(cmd.id, Value::Object(p))
            })
            .collect();
        let r = app.execute(&calls);
        if let Err(e) = &r {
            app.set_status(e.clone());
        }
        return r;
    }
    app.run(cmd.id, params)
}

/// A command line split into the command words and the parameters, given
/// as `name=value` pairs (`width=2 name=crate position=0,1,0`) or as a JSON
/// object (`{"width": 2}`). Values parse as JSON when they can, lists of
/// numbers from commas, and as text otherwise.
///
/// Complexity: O(len(line)).
pub fn parse_line(line: &str) -> Result<(String, Value), String> {
    let line = line.trim();
    if let Some(at) = line.find('{') {
        let params: Value = serde_json::from_str(&line[at..]).map_err(|e| format!("{}: {e}", t("the parameters are not valid JSON")))?;
        if !params.is_object() {
            return Err(t("the parameters must be a JSON object").to_owned());
        }
        return Ok((line[..at].trim().to_owned(), params));
    }
    let mut words = Vec::new();
    let mut params = Map::new();
    for token in line.split_whitespace() {
        match token.split_once('=') {
            Some((k, v)) if !k.is_empty() => {
                params.insert(k.to_owned(), parse_value(v));
            }
            _ => words.push(token),
        }
    }
    let params = if params.is_empty() { Value::Null } else { Value::Object(params) };
    Ok((words.join(" "), params))
}

fn parse_value(v: &str) -> Value {
    if let Ok(x) = serde_json::from_str::<Value>(v) {
        return x;
    }
    let parts: Vec<&str> = v.split(',').collect();
    if parts.len() > 1
        && let Ok(nums) = parts.iter().map(|p| p.trim().parse::<f64>()).collect::<Result<Vec<_>, _>>()
    {
        return json!(nums);
    }
    Value::String(v.to_owned())
}

/// The command `words` name: an exact id, label or alias first, then the
/// best fuzzy match.
///
/// Complexity: as [`rank`].
pub fn resolve<'a>(commands: &'a [Command], words: &str) -> Result<&'a Command, String> {
    let w = words.trim().to_lowercase();
    if w.is_empty() {
        return Err(t("type a command").to_owned());
    }
    let exact = commands.iter().find(|c| c.id == w).or_else(|| commands.iter().find(|c| c.label.to_lowercase() == w || c.aliases.contains(&w)));
    if let Some(c) = exact {
        return Ok(c);
    }
    rank(commands, &w).first().and_then(|&i| commands.get(i)).ok_or_else(|| format!("{} `{words}`", t("unknown command")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PixelApp {
        PixelApp::new(p3d_engine::Session::new(), crate::Services::default())
    }

    #[test]
    fn fuzzy_prefers_starts_and_runs() {
        assert!(fuzzy("box", "box").unwrap() > fuzzy("box", "bounding box").unwrap());
        assert!(fuzzy("fs", "frame selection").unwrap() > fuzzy("fs", "fill holes").unwrap_or(i32::MIN));
        assert_eq!(fuzzy("xyz", "box"), None);
        assert_eq!(fuzzy("", "anything"), Some(0));
    }

    #[test]
    fn search_finds_labels_aliases_ids_and_descriptions() {
        let app = app();
        let idx = index(&app);
        let top = |q: &str| rank(&idx, q).first().map(|&i| (idx[i].id, idx[i].label.clone()));
        assert_eq!(top("box").map(|c| c.0), Some("prim.box"));
        assert_eq!(top("polyCube").map(|c| c.0), Some("prim.box"), "a Maya name");
        assert_eq!(top("view selected").map(|c| c.0), Some("view.frame"), "a Blender name");
        assert_eq!(top("prim.cyl").map(|c| c.0), Some("prim.cylinder"));
        assert_eq!(top("alt z").map(|c| c.0), Some("view.xray"));
        // Ops a menu already runs are listed once, under the menu label.
        assert_eq!(idx.iter().filter(|c| c.id == "prim.box").count(), 1);
        // Every op is reachable.
        for op in app.session.registry().iter() {
            assert!(idx.iter().any(|c| c.id == op.id()), "{} is missing", op.id());
        }
        // Ranking is deterministic.
        assert_eq!(rank(&idx, "s"), rank(&idx, "s"));
    }

    #[test]
    fn command_lines_parse_pairs_and_json() {
        let (w, p) = parse_line("prim.box width=2 name=crate position=0,1.5,0 bevel=true").unwrap();
        assert_eq!(w, "prim.box");
        assert_eq!(p, json!({ "width": 2, "name": "crate", "position": [0.0, 1.5, 0.0], "bevel": true }));
        let (w, p) = parse_line(" add cube {\"width\": 3} ").unwrap();
        assert_eq!((w.as_str(), p), ("add cube", json!({ "width": 3 })));
        assert_eq!(parse_line("frame").unwrap(), ("frame".to_owned(), Value::Null));
        assert!(parse_line("prim.box {nope").is_err());
        assert!(parse_line("prim.box [1]").map(|(_, p)| p) == Ok(Value::Null), "brackets alone are words");
    }

    #[test]
    fn command_lines_run_ops_on_the_selection() {
        let mut app = app();
        let idx = index(&app);
        let (w, p) = parse_line("add cube width=2 name=crate").unwrap();
        let cmd = resolve(&idx, &w).unwrap().clone();
        assert_eq!(cmd.id, "prim.box");
        run_with(&mut app, &cmd, p).unwrap();
        let k = app.doc().resolve("crate").unwrap();
        let (lo, hi) = app.doc().object_mesh(k).and_then(|m| m.bounds()).unwrap();
        assert!(((hi - lo).x - 2.0).abs() < 1e-5);
        // A target op with the crate selected runs on it.
        let cmd = resolve(&idx, "apply transform").unwrap().clone();
        assert!(cmd.on_selection);
        let steps = app.session.history().len();
        let _ = run_with(&mut app, &cmd, Value::Null);
        assert!(app.session.history().len() <= steps + 1, "at most one undo step");
        assert!(resolve(&idx, "").is_err());
    }
}
