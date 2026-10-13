//! The pole overlay for retopology review: interior vertices whose valence
//! is not four (three, five, six or more) marked in the viewport in their
//! own colors, with a legend of the counts. Valences come from the
//! half-edge adjacency (O(V) per mesh, cached per mesh); boundary vertices
//! are left out (their regular valence differs).

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Align2, Color32, Pos2, Rect, Stroke, pos2, vec2};
use glam::{Mat4, Vec3};
use p3d_mesh::Mesh;
use p3d_scene::ObjectKey;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

/// Markers drawn at most per frame (dense meshes stay responsive).
const MAX_MARKERS: usize = 50_000;

/// The kinds of pole: valence three (or less), five, six or more.
pub const KINDS: [&str; 3] = ["3", "5", "6+"];

/// The poles of one mesh.
#[derive(Debug, Default)]
pub struct MeshPoles {
    /// Position, normal (local space) and kind (index into [`KINDS`]).
    pub points: Vec<(Vec3, Vec3, u8)>,
    /// How many of each kind.
    pub counts: [usize; 3],
}

/// The poles of `mesh`.
///
/// Complexity: O(V + E + corners).
pub fn find(mesh: &Mesh) -> MeshPoles {
    let mut normals = vec![Vec3::ZERO; mesh.positions.len()];
    for f in mesh.faces() {
        let n = mesh.face_normal_area2(f);
        for v in mesh.face_verts(f) {
            if let Some(slot) = normals.get_mut(v as usize) {
                *slot += n;
            }
        }
    }
    let mut out = MeshPoles::default();
    for v in mesh.verts() {
        if mesh.is_boundary_vert(v) {
            continue;
        }
        let kind = match mesh.valence(v) {
            4 => continue,
            0..=3 => 0u8,
            5 => 1,
            _ => 2,
        };
        out.counts[kind as usize] += 1;
        let p = mesh.positions.get(v as usize).copied().unwrap_or_default();
        out.points.push((p, normals.get(v as usize).copied().unwrap_or_default().normalize_or_zero(), kind));
    }
    out
}

/// Poles per mesh, kept while the mesh is shown.
#[derive(Default)]
pub struct PoleCache {
    by_mesh: BTreeMap<usize, (Arc<Mesh>, Arc<MeshPoles>)>,
}

impl PoleCache {
    /// The poles of `mesh`, found once per mesh.
    ///
    /// Complexity: O(log meshes) when cached, else as [`find`].
    pub fn get(&mut self, mesh: &Arc<Mesh>) -> Arc<MeshPoles> {
        let key = Arc::as_ptr(mesh) as usize;
        if let Some((m, p)) = self.by_mesh.get(&key)
            && Arc::ptr_eq(m, mesh)
        {
            return Arc::clone(p);
        }
        let p = Arc::new(find(mesh));
        self.by_mesh.insert(key, (Arc::clone(mesh), Arc::clone(&p)));
        p
    }

    /// Forgets meshes not in `keep`.
    ///
    /// Complexity: O(cached + kept log kept).
    pub fn retain(&mut self, keep: &[Arc<Mesh>]) {
        let live: std::collections::BTreeSet<usize> = keep.iter().map(|m| Arc::as_ptr(m) as usize).collect();
        self.by_mesh.retain(|k, _| live.contains(k));
    }
}

/// The color of a kind of pole.
fn color(kind: u8) -> Color32 {
    let tk = Tokens::get();
    match kind {
        0 => tk.axis_z,
        1 => tk.health_warn,
        _ => tk.health_error,
    }
}

impl PixelApp {
    /// The mesh objects whose poles show: the edited mesh in Edit Mode,
    /// else the selected meshes.
    fn pole_objects(&self) -> Vec<(ObjectKey, Arc<Mesh>)> {
        let doc = self.doc();
        let keys: Vec<ObjectKey> =
            if self.ui.mode.is_edit() { self.edit.object.into_iter().collect() } else { self.selection.iter().copied().collect() };
        keys.into_iter()
            .filter_map(|k| {
                let m = doc.object(k)?.mesh_key()?;
                Some((k, Arc::clone(doc.mesh_arc(m)?)))
            })
            .collect()
    }

    /// Pole counts of `target` (a name, path or id), or of the meshes the
    /// overlay shows, for agents.
    ///
    /// Complexity: as [`find`] per mesh not cached.
    pub fn pole_counts(&mut self, target: Option<&str>) -> Result<Value, String> {
        let objects = match target {
            Some(t) => {
                let doc = self.doc();
                let k = doc.resolve(t).map_err(|e| e.to_string())?;
                let m = doc.object(k).and_then(|o| o.mesh_key()).and_then(|m| doc.mesh_arc(m)).ok_or(crate::i18n::t("not a mesh object"))?;
                vec![(k, Arc::clone(m))]
            }
            None => self.pole_objects(),
        };
        let list: Vec<Value> = objects
            .iter()
            .map(|(k, m)| {
                let p = self.poles.get(m);
                let id = self.doc().object(*k).map(|o| o.public_id());
                json!({ "object": id, "valence3": p.counts[0], "valence5": p.counts[1], "valence6plus": p.counts[2], "vertices": m.vert_count() })
            })
            .collect();
        Ok(json!({ "on": self.ui.view.poles, "objects": list }))
    }
}

/// Draws the pole markers over the viewport and the legend in its bottom
/// right corner, when the overlay is on. Poles facing away are faint
/// unless x-ray is on.
///
/// Complexity: O(poles of the shown meshes) per frame; finding them is
/// cached per mesh.
pub fn overlay(app: &mut PixelApp, ui: &egui::Ui) {
    let rect = app.viewport.rect;
    if !app.ui.view.poles || !rect.is_positive() {
        return;
    }
    let objects = app.pole_objects();
    app.poles.retain(&objects.iter().map(|(_, m)| Arc::clone(m)).collect::<Vec<_>>());
    let cam = app.viewport.camera;
    let vp = cam.view_proj(rect.width() / rect.height().max(1.0));
    let p = ui.painter_at(rect);
    let tk = Tokens::get();
    let mut counts = [0usize; 3];
    let mut drawn = 0;
    let outline = Stroke::new(1.0, Color32::from_black_alpha(160));
    for (k, mesh) in &objects {
        let poles = app.poles.get(mesh);
        for (c, n) in counts.iter_mut().zip(poles.counts) {
            *c += n;
        }
        let Some(world) = app.doc().world_affine(*k).map(Mat4::from) else { continue };
        let normal_m = world.inverse().transpose();
        for (pos, normal, kind) in &poles.points {
            if drawn >= MAX_MARKERS {
                break;
            }
            let w = world.transform_point3(*pos);
            let Some(s) = project(vp, rect, w) else { continue };
            if !rect.contains(s) {
                continue;
            }
            let toward = if cam.ortho { -cam.toward() } else { (cam.eye() - w).normalize_or_zero() };
            let front = normal_m.transform_vector3(*normal).dot(toward) >= 0.0;
            let c = color(*kind);
            if front || app.ui.view.xray {
                p.circle(s, 3.5, c, outline);
            } else {
                p.circle_filled(s, 2.5, c.gamma_multiply(0.35));
            }
            drawn += 1;
        }
    }
    legend(&p, rect, counts, tk.text, tk.popup.gamma_multiply(0.92), tk.popup_border);
}

fn project(vp: Mat4, rect: Rect, p: Vec3) -> Option<Pos2> {
    let c = vp * p.extend(1.0);
    if c.w <= 1e-6 {
        return None;
    }
    let n = c.truncate() / c.w;
    Some(pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height()))
}

/// The counts in a small box at the bottom right of the viewport.
fn legend(p: &egui::Painter, rect: Rect, counts: [usize; 3], text: Color32, fill: Color32, border: Color32) {
    let entries: Vec<String> = KINDS.iter().zip(counts).map(|(k, n)| format!("{k}: {}", crate::chrome::thousands(n))).collect();
    let title = t("Poles");
    let widths: Vec<f32> = entries.iter().map(|e| p.layout_no_wrap(e.clone(), theme::small(), text).size().x).collect();
    let title_w = p.layout_no_wrap(title.to_owned(), theme::small(), text).size().x;
    let w = 16.0 + title_w + widths.iter().map(|w| w + 22.0).sum::<f32>();
    let r = Rect::from_min_size(pos2(rect.right() - w - 12.0, rect.bottom() - 34.0), vec2(w, 24.0));
    p.rect(r, 6.0, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let mut x = r.left() + 8.0;
    p.text(pos2(x, r.center().y), Align2::LEFT_CENTER, title, theme::small(), text);
    x += title_w + 10.0;
    for (i, (e, ew)) in entries.iter().zip(widths).enumerate() {
        p.circle(pos2(x + 4.0, r.center().y), 4.0, color(i as u8), Stroke::new(1.0, Color32::from_black_alpha(160)));
        p.text(pos2(x + 12.0, r.center().y), Align2::LEFT_CENTER, e, theme::small(), text);
        x += ew + 22.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn a_cube_has_eight_poles_of_valence_three() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.box", json!({ "name": "crate" })).unwrap();
        let k = a.doc().resolve("crate").unwrap();
        let m = a.doc().object_mesh(k).unwrap();
        let poles = find(m);
        assert_eq!(poles.counts, [8, 0, 0]);
        // A subdivided cube keeps 8 poles of valence 3; the new vertices are regular.
        a.run("subd.catmull_clark", json!({ "target": "crate" })).unwrap();
        let counts = a.pole_counts(Some("crate")).unwrap();
        assert_eq!((counts["objects"][0]["valence3"].clone(), counts["objects"][0]["valence5"].clone()), (json!(8), json!(0)));
        // An open plane has no interior poles; boundaries are left out.
        a.run("prim.plane", json!({ "name": "floor", "advanced": { "segments": [3, 3] } })).unwrap();
        let k = a.doc().resolve("floor").unwrap();
        assert_eq!(find(a.doc().object_mesh(k).unwrap()).counts, [0, 0, 0]);
        // A cone's apex has a high valence.
        a.run("prim.cone", json!({ "name": "spike" })).unwrap();
        let k = a.doc().resolve("spike").unwrap();
        assert!(find(a.doc().object_mesh(k).unwrap()).counts[2] >= 1);
        // The overlay's own counts follow the selection.
        a.run("view.poles", json!({ "on": true })).unwrap();
        a.run("edit.select", json!({ "target": "crate" })).unwrap();
        let shown = a.pole_counts(None).unwrap();
        assert_eq!((shown["on"].clone(), shown["objects"].as_array().map(Vec::len)), (json!(true), Some(1)));
    }
}
