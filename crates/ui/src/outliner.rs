//! The Outliner, laid out like Blender's: a search field, then a tree under
//! "Scene Collection" with collections, LOD groups and objects, each object
//! with its mesh data as a child row. Arrows expand and collapse; the
//! selection is highlighted (the active object brighter); an eye per row
//! hides it (a collection or LOD group hides everything in it) in the
//! viewport. Double-click renames in place; right-click opens the actions,
//! including new collections and LOD groups from the selection.
//!
//! Collections and LOD groups need no file format of their own (project
//! spec 6): a collection is an empty tagged `collection`, a LOD group an
//! empty tagged `lod_group` whose children are tagged `lod0`, `lod1`, ...
//! (or named `<asset>_LOD<n>`).

use std::collections::BTreeSet;

use egui::{Rect, Sense, Stroke, pos2, vec2};
use p3d_engine::OpCall;
use p3d_scene::{Document, Object, ObjectKey};
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::viewport::ViewportEvent;
use crate::widgets;

const ROW: f32 = 28.0;
const INDENT: f32 = 16.0;

/// Tags that make an empty a collection or a LOD group.
pub const COLLECTION_TAG: &str = "collection";
pub const LOD_GROUP_TAG: &str = "lod_group";

/// A row of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Node {
    Scene,
    Object(ObjectKey),
    /// The mesh data of an object.
    Mesh(ObjectKey),
}

/// What kind of thing a row is, for its icon, default state and actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Scene,
    Collection,
    LodGroup,
    Mesh,
    Curve,
    Empty,
    MeshData,
}

/// The LOD level of an object: a `lodN` tag, else a `_LOD<n>` name suffix.
pub fn lod_level(obj: &Object) -> Option<u32> {
    let tagged = obj.tags.iter().find_map(|t| t.strip_prefix("lod").and_then(|n| n.parse().ok()));
    tagged.or_else(|| {
        let name = obj.name();
        let at = name.to_ascii_lowercase().rfind("_lod")?;
        name[at + 4..].parse().ok()
    })
}

/// The kind of an object row.
pub fn kind_of(obj: &Object) -> Kind {
    let tagged = |tag: &str| obj.tags.iter().any(|t| t == tag);
    if tagged(LOD_GROUP_TAG) {
        Kind::LodGroup
    } else if obj.mesh_key().is_some() {
        Kind::Mesh
    } else if obj.curve_key().is_some() {
        Kind::Curve
    } else if tagged(COLLECTION_TAG) {
        Kind::Collection
    } else {
        Kind::Empty
    }
}

struct Row {
    node: Node,
    kind: Kind,
    depth: usize,
    expandable: bool,
}

/// Open by default: the scene, collections, LOD groups and objects with
/// child objects; an object with only its mesh data starts closed.
fn open_by_default(doc: &Document, node: Node) -> bool {
    match node {
        Node::Scene => true,
        Node::Mesh(_) => false,
        Node::Object(k) => doc.object(k).is_some_and(|o| !matches!(kind_of(o), Kind::Mesh | Kind::Curve | Kind::Empty) || !o.children().is_empty()),
    }
}

/// Children in display order: LOD levels by level, others as stored.
fn children(doc: &Document, obj: &Object) -> Vec<ObjectKey> {
    let mut c: Vec<ObjectKey> = obj.children().to_vec();
    if kind_of(obj) == Kind::LodGroup {
        c.sort_by_key(|k| doc.object(*k).and_then(lod_level).unwrap_or(u32::MAX));
    }
    c
}

/// The visible rows: the tree under expanded nodes, or with a filter the
/// matching objects in a flat list.
///
/// Complexity: O(objects + children sorting of LOD groups).
fn rows(doc: &Document, toggled: &BTreeSet<Node>, filter: &str) -> Vec<Row> {
    let open = |n: Node| open_by_default(doc, n) != toggled.contains(&n);
    let needle = filter.trim().to_lowercase();
    let mut out = Vec::new();
    if !needle.is_empty() {
        for (k, o) in doc.objects() {
            if o.name().to_lowercase().contains(&needle) {
                out.push(Row { node: Node::Object(k), kind: kind_of(o), depth: 0, expandable: false });
            }
        }
        return out;
    }
    let roots: Vec<ObjectKey> = doc.roots().collect();
    out.push(Row { node: Node::Scene, kind: Kind::Scene, depth: 0, expandable: !roots.is_empty() });
    if !open(Node::Scene) {
        return out;
    }
    let mut stack: Vec<(Node, usize)> = roots.into_iter().rev().map(|k| (Node::Object(k), 1)).collect();
    while let Some((node, depth)) = stack.pop() {
        match node {
            Node::Object(k) => {
                let Some(o) = doc.object(k) else { continue };
                let kind = kind_of(o);
                let kids = children(doc, o);
                let has_mesh = o.mesh_key().is_some();
                out.push(Row { node, kind, depth, expandable: has_mesh || !kids.is_empty() });
                if open(node) {
                    for c in kids.into_iter().rev() {
                        stack.push((Node::Object(c), depth + 1));
                    }
                    if has_mesh {
                        stack.push((Node::Mesh(k), depth + 1));
                    }
                }
            }
            Node::Mesh(_) => out.push(Row { node, kind: Kind::MeshData, depth, expandable: false }),
            Node::Scene => {}
        }
    }
    out
}

/// Every object under `key`, itself included.
///
/// Complexity: O(subtree).
fn subtree(doc: &Document, key: ObjectKey) -> Vec<ObjectKey> {
    let mut out = Vec::new();
    let mut stack = vec![key];
    while let Some(k) = stack.pop() {
        out.push(k);
        if let Some(o) = doc.object(k) {
            stack.extend(o.children().iter().copied());
        }
    }
    out
}

/// A free object name: `base`, else `base_2`, `base_3`, ...
///
/// Complexity: O(tries x name lookup).
pub fn free_name(doc: &Document, base: &str) -> String {
    if doc.resolve(base).is_err() {
        return base.to_owned();
    }
    (2..).map(|i| format!("{base}_{i}")).find(|n| doc.resolve(n).is_err()).unwrap_or_else(|| base.to_owned())
}

/// Puts `members` (object ids) in a new collection, one undo step.
pub fn new_collection(app: &mut PixelApp, members: &[String]) -> Result<Value, String> {
    let name = free_name(app.doc(), "Collection");
    let mut calls = vec![
        OpCall::new("scene.create_empty", json!({ "name": name })),
        OpCall::new("scene.tag", json!({ "target": name, "add": [COLLECTION_TAG] })),
    ];
    calls.extend(members.iter().map(|m| OpCall::new("scene.parent", json!({ "target": m, "parent": name }))));
    app.execute_labeled(&calls, Some(t("New collection")))
}

/// Groups the selected mesh objects as the LOD levels of a new LOD group:
/// the most triangles is LOD 0. Levels after the first are hidden in the
/// viewport so they do not overlap. One undo step.
pub fn lod_group_from_selection(app: &mut PixelApp) -> Result<Value, String> {
    let doc = std::sync::Arc::clone(app.session.doc());
    let mut members: Vec<(usize, String, String, ObjectKey)> = app
        .selection
        .iter()
        .filter_map(|&k| {
            let o = doc.object(k)?;
            let tris = doc.object_mesh(k)?.triangle_count();
            Some((tris, o.name().to_owned(), o.public_id(), k))
        })
        .collect();
    if members.is_empty() {
        return Err(t("select the mesh objects that are the LOD levels").to_owned());
    }
    members.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    // Named after the most detailed level without its LOD suffix.
    let first = &members[0].1;
    let base = first.to_ascii_lowercase().rfind("_lod").map_or(first.as_str(), |at| &first[..at]).to_owned();
    let name = free_name(&doc, &format!("{base}_lod_group"));
    let parent = app.active_object().and_then(|k| doc.object(k)?.parent()).and_then(|p| doc.object(p)).map(Object::public_id);
    let mut calls = vec![
        OpCall::new("scene.create_empty", json!({ "name": name, "parent": parent })),
        OpCall::new("scene.tag", json!({ "target": name, "add": [LOD_GROUP_TAG] })),
    ];
    for (level, (_, _, id, k)) in members.iter().enumerate() {
        let stale: Vec<&String> =
            doc.object(*k).map(|o| o.tags.iter().filter(|t| t.starts_with("lod") && t.as_str() != LOD_GROUP_TAG).collect()).unwrap_or_default();
        calls.push(OpCall::new("scene.parent", json!({ "target": id, "parent": name })));
        calls.push(OpCall::new("scene.tag", json!({ "target": id, "add": [format!("lod{level}")], "remove": stale })));
    }
    let r = app.execute_labeled(&calls, Some(t("LOD group")))?;
    for (level, (_, _, _, k)) in members.iter().enumerate() {
        app.set_hidden(*k, level > 0);
    }
    Ok(r)
}

/// What a row asked for this frame.
enum RowAction {
    Select { key: Option<ObjectKey>, add: bool, mesh_tab: bool },
    Toggle(Node),
    Hide(ObjectKey, bool),
    Rename(ObjectKey),
    Run(&'static str, Option<ObjectKey>),
}

/// The Outliner body.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let state = ui.id().with("outliner");
    let (mut toggled, mut filter, mut renaming): (BTreeSet<Node>, String, Option<(ObjectKey, String)>) =
        ui.data_mut(|d| d.get_temp(state).unwrap_or_default());
    let mut action: Option<RowAction> = None;

    // Search, and a button for a new collection.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let w = ui.available_width() - widgets::SEARCH - 6.0;
        ui.allocate_ui(vec2(w, widgets::SEARCH), |ui| widgets::search_field(ui, &mut filter, t("Search objects")));
        let tip = widgets::Tip::new(t("New collection")).desc(t("Puts the selected objects in a new collection"));
        if icons::button_with(ui, Icon::Plus, vec2(widgets::SEARCH, widgets::SEARCH), tip, false, icons::Look::Segment, egui::RectAlign::BOTTOM)
            .clicked()
        {
            action = Some(RowAction::Run("collection", None));
        }
    });
    ui.add_space(6.0);

    let doc = std::sync::Arc::clone(app.session.doc());
    let list = rows(&doc, &toggled, &filter);
    // Triangle counts come from the stats cache (counted once per edit).
    app.stats();
    if (list.len() <= 1 && doc.object_count() == 0) || list.is_empty() {
        ui.add_space(8.0);
        let msg = if filter.trim().is_empty() {
            t("The scene is empty. Add a shape with the tools on the left.")
        } else {
            t("No object matches the search.")
        };
        ui.label(egui::RichText::new(msg).color(tk.text_dim));
    }
    let active = app.active_object();
    let mut commit_rename: Option<(ObjectKey, String)> = None;
    egui::ScrollArea::vertical().id_salt("outliner-rows").auto_shrink([false, false]).show_rows(ui, ROW, list.len(), |ui, range| {
        for i in range {
            let row = &list[i];
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let key = match row.node {
                Node::Object(k) | Node::Mesh(k) => Some(k),
                Node::Scene => None,
            };
            let obj = key.and_then(|k| doc.object(k));
            let is_object_row = matches!(row.node, Node::Object(_));
            let selected = is_object_row && key.is_some_and(|k| app.selection.contains(&k));
            // A collection or LOD group shows as hidden when all of it is
            // (a walk of its subtree, for the few rows on screen).
            let hidden = key.is_some_and(|k| match row.kind {
                Kind::Collection | Kind::LodGroup => subtree(&doc, k).iter().all(|c| app.hidden.contains(c)),
                _ => app.hidden.contains(&k),
            });
            // Rounded rows: a light accent tint for the selection, stronger
            // for the active object, an eased tint on hover.
            let pill = rect.shrink2(vec2(2.0, 1.0));
            let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered(), 0.12);
            if selected {
                let is_active = active == key;
                ui.painter().rect_filled(pill, 5.0, tk.accent.gamma_multiply(if is_active { 0.2 } else { 0.1 } + 0.05 * hot));
            } else if hot > 0.0 {
                ui.painter().rect_filled(pill, 5.0, tk.control.gamma_multiply(0.8 * hot));
            }
            let x0 = rect.left() + 4.0 + row.depth as f32 * INDENT;
            // Guide lines toward the parent.
            if row.depth > 0 {
                let lx = x0 - INDENT / 2.0;
                ui.painter().vline(lx, rect.top()..=rect.center().y, Stroke::new(1.0, tk.card_border));
                ui.painter().hline(lx..=x0 + 2.0, rect.center().y, Stroke::new(1.0, tk.card_border));
            }
            if row.expandable {
                let cr = Rect::from_min_size(pos2(x0, rect.center().y - 7.0), vec2(14.0, 14.0));
                let c = ui.interact(cr, ui.id().with(("expand", row.node)), Sense::click());
                let open = open_by_default(&doc, row.node) != toggled.contains(&row.node);
                let turn = ui.ctx().animate_bool_with_time(c.id.with("open"), open, 0.12);
                widgets::chevron(ui.painter(), cr.center(), turn, if c.hovered() { tk.text } else { tk.text_faint });
                icons::describe_control(ui, &c, t("Expand"), Some(open));
                if c.clicked() {
                    action = Some(RowAction::Toggle(row.node));
                }
            }
            let ir = Rect::from_min_size(pos2(x0 + 16.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            let (icon, icon_color) = match row.kind {
                Kind::Scene => (Icon::SceneRoot, tk.text_dim),
                Kind::Collection => (Icon::Collection, tk.text_dim),
                Kind::LodGroup => (Icon::LodGroup, tk.icon_accent),
                Kind::Mesh => (Icon::Mesh, tk.icon),
                Kind::Curve => (Icon::Curve, tk.icon),
                Kind::Empty => (Icon::Empty, tk.icon),
                Kind::MeshData => (Icon::MeshData, tk.health_ok),
            };
            icons::paint_colored(ui.painter(), ir, icon, if hidden { tk.text_faint } else { icon_color }, tk.accent);
            let name_x = ir.right() + 6.0;
            // Eye toggle on the right (not on the scene or mesh data rows).
            let mut text_right = rect.right() - 6.0;
            if let (Some(k), true) = (key, is_object_row) {
                let eye = Rect::from_min_size(pos2(rect.right() - 24.0, rect.center().y - 9.0), vec2(18.0, 18.0));
                let er = ui.interact(eye, ui.id().with(("eye", k)), Sense::click());
                icons::paint(ui.painter(), eye.shrink(1.0), if hidden { Icon::EyeOff } else { Icon::Eye }, !er.hovered() && hidden);
                let tip = if hidden { t("Show in viewport") } else { t("Hide in viewport") };
                icons::describe_control(ui, &er, tip, Some(!hidden));
                widgets::tooltip(&er, widgets::Tip::new(tip), egui::RectAlign::LEFT);
                if er.clicked() {
                    action = Some(RowAction::Hide(k, !hidden));
                }
                text_right = eye.left() - 6.0;
            }
            // Right column: triangles, vertices or contents.
            let detail = match (row.kind, key) {
                (Kind::Mesh, Some(k)) => app.stats.tris_of(k).map(crate::chrome::thousands),
                (Kind::MeshData, Some(k)) => doc.object_mesh(k).map(|m| format!("{} {}", crate::chrome::thousands(m.vert_count()), t("verts"))),
                (Kind::Collection | Kind::LodGroup, _) => obj.map(|o| o.children().len().to_string()),
                (Kind::Scene, _) => Some(doc.object_count().to_string()),
                _ => None,
            };
            if let Some(d) = &detail {
                let g = ui.painter().layout_no_wrap(d.clone(), theme::small(), tk.text_faint);
                text_right -= g.size().x;
                ui.painter().galley(pos2(text_right, rect.center().y - g.size().y / 2.0), g, tk.text_faint);
                text_right -= 8.0;
            }
            // A LOD badge after the name.
            let badge = match (row.kind, obj) {
                (Kind::Mesh | Kind::Curve | Kind::Empty, Some(o)) => lod_level(o).map(|n| format!("LOD{n}")),
                (Kind::LodGroup, _) => Some("LOD".to_owned()),
                _ => None,
            };
            let label = match (row.node, obj) {
                (Node::Scene, _) => t("Scene Collection").to_owned(),
                (Node::Mesh(_), _) => t("Mesh").to_owned(),
                (_, Some(o)) => o.name().to_owned(),
                _ => String::new(),
            };
            match (&mut renaming, key) {
                (Some((k, text)), Some(rk)) if *k == rk && is_object_row => {
                    let er = Rect::from_min_max(pos2(name_x, rect.top() + 2.0), pos2(text_right, rect.bottom() - 2.0));
                    let te = ui.put(er, egui::TextEdit::singleline(text).margin(egui::Margin::symmetric(4, 2)).vertical_align(egui::Align::Center));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    if te.lost_focus() {
                        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        commit_rename = Some((rk, if cancel { String::new() } else { text.clone() }));
                    }
                }
                _ => {
                    let color = if hidden {
                        tk.text_faint
                    } else if matches!(row.kind, Kind::MeshData | Kind::Scene) {
                        tk.text_dim
                    } else {
                        tk.text
                    };
                    let badge_w = if badge.is_some() { 44.0 } else { 0.0 };
                    let mut job = egui::text::LayoutJob::simple_singleline(label.clone(), theme::body(), color);
                    job.wrap = egui::text::TextWrapping {
                        max_width: (text_right - name_x - badge_w).max(20.0),
                        max_rows: 1,
                        break_anywhere: true,
                        overflow_character: Some('\u{2026}'),
                    };
                    let g = ui.painter().layout_job(job);
                    let name_w = g.size().x;
                    ui.painter().galley(pos2(name_x, rect.center().y - g.size().y / 2.0), g, color);
                    if let Some(b) = badge {
                        lod_badge(ui.painter(), pos2(name_x + name_w + 6.0, rect.center().y), &b);
                    }
                }
            }
            crate::control::record_widget(ui, &resp, &label, "treeitem", Some(selected));
            let is_group = matches!(row.kind, Kind::Collection | Kind::LodGroup);
            resp.context_menu(|ui| {
                ui.set_min_width(220.0);
                let mut entry = |ui: &mut egui::Ui, label: &str, what: RowAction| {
                    if ui.button(label).clicked() {
                        action = Some(what);
                        ui.close();
                    }
                };
                if let (Some(k), true) = (key, is_object_row) {
                    entry(ui, t("Rename"), RowAction::Rename(k));
                    entry(ui, t("Duplicate"), RowAction::Run("duplicate", Some(k)));
                    entry(ui, t("Frame in view"), RowAction::Run("frame", Some(k)));
                    entry(ui, if hidden { t("Show in viewport") } else { t("Hide in viewport") }, RowAction::Hide(k, !hidden));
                    if obj.is_some_and(|o| o.parent().is_some()) {
                        entry(ui, t("Move out of its parent"), RowAction::Run("unparent", Some(k)));
                    }
                    ui.separator();
                }
                entry(ui, t("New collection from selection"), RowAction::Run("collection", None));
                entry(ui, t("LOD group from selection"), RowAction::Run("lod_group", None));
                if let (Some(k), true) = (key, is_object_row) {
                    ui.separator();
                    entry(ui, if is_group { t("Delete with contents") } else { t("Delete") }, RowAction::Run("delete", Some(k)));
                }
            });
            let resp = match key.filter(|_| is_object_row) {
                Some(k) => resp.on_hover_text(format!("{} ({})", doc.path(k).unwrap_or_default(), obj.map(Object::public_id).unwrap_or_default())),
                None => resp,
            };
            if resp.double_clicked() && is_object_row {
                if let Some(k) = key {
                    action = Some(RowAction::Rename(k));
                }
            } else if resp.clicked() {
                let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                action = Some(RowAction::Select { key, add, mesh_tab: row.kind == Kind::MeshData });
            }
        }
    });
    if let Some((k, name)) = commit_rename {
        renaming = None;
        let current = app.doc().object(k).map(|o| (o.name().to_owned(), o.public_id()));
        if let Some((old, id)) = current
            && !name.trim().is_empty()
            && name.trim() != old
        {
            let _ = app.execute(&[OpCall::new("scene.rename", json!({ "target": id, "name": name.trim() }))]);
        }
    }
    match action {
        Some(RowAction::Select { key: Some(k), add, mesh_tab }) => {
            app.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add }]);
            if mesh_tab {
                app.ui.inspector_tab = crate::inspector::Tab::Mesh;
            }
        }
        Some(RowAction::Select { key: None, add, .. }) => {
            if !add {
                app.selection.clear();
            }
        }
        Some(RowAction::Toggle(n)) => {
            if !toggled.remove(&n) {
                toggled.insert(n);
            }
        }
        Some(RowAction::Hide(k, on)) => {
            for c in subtree(&doc, k) {
                app.set_hidden(c, on);
            }
        }
        Some(RowAction::Rename(k)) => renaming = app.doc().object(k).map(|o| (k, o.name().to_owned())),
        Some(RowAction::Run(cmd, k)) => run(app, cmd, k),
        None => {}
    }
    ui.data_mut(|d| d.insert_temp(state, (toggled, filter, renaming)));
}

/// Runs a context menu action on `key` (selecting it first) or on the
/// selection.
fn run(app: &mut PixelApp, cmd: &str, key: Option<ObjectKey>) {
    if let Some(k) = key
        && !app.selection.contains(&k)
    {
        app.handle_viewport(vec![ViewportEvent::Picked { object: Some(k), add: false }]);
    }
    let r = match cmd {
        "frame" => app.run("view.frame", Value::Null),
        "duplicate" => app.run("edit.duplicate", Value::Null),
        "delete" => app.run("edit.delete", Value::Null),
        "unparent" => app.run_on_selection("scene.unparent"),
        "collection" => {
            let members = app.selected_targets();
            new_collection(app, &members)
        }
        _ => lod_group_from_selection(app),
    };
    if let Err(e) = r {
        app.set_status(e);
    }
}

/// A small `LOD0`-style pill.
fn lod_badge(p: &egui::Painter, left_center: egui::Pos2, text: &str) {
    let tk = Tokens::get();
    let g = p.layout_no_wrap(text.to_owned(), egui::FontId::proportional(9.5), tk.icon_accent);
    let r = Rect::from_min_size(pos2(left_center.x, left_center.y - 8.0), vec2(g.size().x + 10.0, 16.0));
    p.rect_filled(r, 8.0, tk.accent_soft);
    p.galley(r.center() - g.size() / 2.0, g, tk.icon_accent);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PixelApp {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        for (name, w) in [("crate", 1.0), ("crate_LOD1", 0.5), ("barrel", 1.0)] {
            app.run("prim.box", json!({ "name": name, "width": w })).unwrap();
        }
        app
    }

    #[test]
    fn rows_nest_under_the_scene_with_mesh_data() {
        let app = app();
        let doc = app.doc();
        let list = rows(doc, &BTreeSet::new(), "");
        assert_eq!(list[0].node, Node::Scene);
        assert_eq!(list.iter().filter(|r| r.depth == 1).count(), 3, "three top-level objects");
        // Objects with only mesh data start closed: no mesh rows.
        assert!(list.iter().all(|r| r.kind != Kind::MeshData));
        let k = doc.resolve("crate").unwrap();
        let open: BTreeSet<Node> = [Node::Object(k)].into();
        let list = rows(doc, &open, "");
        let at = list.iter().position(|r| r.node == Node::Object(k)).unwrap();
        assert_eq!(list[at + 1].node, Node::Mesh(k));
        assert_eq!(list[at + 1].depth, 2);
        // A filter lists matches flat.
        assert_eq!(rows(doc, &BTreeSet::new(), "lod").len(), 1);
    }

    #[test]
    fn lod_levels_come_from_tags_or_names() {
        let app = app();
        let doc = app.doc();
        assert_eq!(doc.object(doc.resolve("crate_LOD1").unwrap()).and_then(lod_level), Some(1));
        assert_eq!(doc.object(doc.resolve("crate").unwrap()).and_then(lod_level), None);
    }

    #[test]
    fn collections_and_lod_groups_from_the_selection() {
        let mut app = app();
        let (a, b, c) = (app.doc().resolve("crate").unwrap(), app.doc().resolve("crate_LOD1").unwrap(), app.doc().resolve("barrel").unwrap());
        // A LOD group: the most detailed level first, one undo step.
        app.selection = [b, a].into();
        let steps = app.session.history().len();
        lod_group_from_selection(&mut app).unwrap();
        assert_eq!(app.session.history().len(), steps + 1);
        let doc = std::sync::Arc::clone(app.session.doc());
        let g = doc.resolve("crate_lod_group").unwrap();
        let group = doc.object(g).unwrap();
        assert_eq!(kind_of(group), Kind::LodGroup);
        assert_eq!(children(&doc, group), vec![a, b]);
        assert!(doc.object(a).unwrap().tags.iter().any(|t| t == "lod0"));
        assert!(app.hidden.contains(&b) && !app.hidden.contains(&a), "only LOD 0 shows");
        // A collection holding the barrel.
        app.selection = [c].into();
        let members = app.selected_targets();
        new_collection(&mut app, &members).unwrap();
        let doc = app.doc();
        let col = doc.resolve("Collection").unwrap();
        assert_eq!(kind_of(doc.object(col).unwrap()), Kind::Collection);
        assert_eq!(doc.object(c).and_then(|o| o.parent()), Some(col));
        assert_eq!(free_name(doc, "Collection"), "Collection_2");
    }
}
