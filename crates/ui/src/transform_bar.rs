//! The middle of the viewport header, as in Blender: transform orientation,
//! pivot point, snapping (a toggle and its popover) and proportional editing
//! (a toggle and its falloff popover). Every choice runs `ui.transform` or
//! `view.snap`, so menus, the palette and agents can set them too.

use egui::{Rect, Sense, pos2, vec2};
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::icons::{self, Icon, Look};
use crate::theme::{self, Tokens};
use crate::transform::{Falloff, Orientation, Pivot, SnapBase, SnapTarget};
use crate::widgets::{self, Tip};

const H: f32 = 26.0;
const PAD: f32 = 2.0;

type Chosen = Option<(&'static str, Value)>;

/// A dropdown button on the header track: icon, optional label, chevron.
fn dropdown(ui: &mut egui::Ui, rect: Rect, icon: Icon, label: Option<&str>, tip: Tip<'_>) -> egui::Response {
    let tk = Tokens::get();
    let resp = ui.interact(rect, ui.id().with(("dropdown", tip.title)), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered() || open, 0.1);
    let p = ui.painter();
    if hot > 0.0 {
        p.rect_filled(rect, 6.0, tk.control.gamma_multiply(hot));
    }
    let cy = rect.center().y;
    icons::paint_colored(
        p,
        Rect::from_min_size(pos2(rect.left() + 6.0, cy - 8.0), vec2(16.0, 16.0)),
        icon,
        widgets::mix(tk.icon, tk.text, hot),
        tk.accent,
    );
    if let Some(l) = label {
        p.text(pos2(rect.left() + 27.0, cy), egui::Align2::LEFT_CENTER, l, theme::body(), tk.text);
    }
    widgets::chevron(p, pos2(rect.right() - 9.0, cy), 1.0, tk.text_faint);
    icons::describe_control(ui, &resp, tip.title, Some(open));
    widgets::tooltip(&resp, tip, egui::RectAlign::BOTTOM);
    resp
}

/// Width of a dropdown with `label`.
fn dropdown_width(ui: &egui::Ui, label: Option<&str>) -> f32 {
    let text = label.map_or(0.0, |l| ui.painter().layout_no_wrap(l.to_owned(), theme::body(), egui::Color32::WHITE).size().x + 7.0);
    6.0 + 16.0 + 4.0 + text + 14.0
}

/// A popover under `resp`; `keep_open` keeps it up while rows are clicked.
fn popover<R>(resp: &egui::Response, ui: &egui::Ui, width: f32, keep_open: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let mut popup = egui::Popup::menu(resp).frame(widgets::popup_frame(ui)).gap(4.0).align(egui::RectAlign::BOTTOM_START).width(width);
    if keep_open {
        popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    }
    popup
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            body(ui)
        })
        .map(|r| r.inner)
}

/// One radio row: checked when it is the current choice.
fn radio(ui: &mut egui::Ui, label: &str, on: bool, value: Value, chosen: &mut Chosen) {
    if widgets::menu_row(ui, None, label, None, Some(on), false).clicked() {
        *chosen = Some(("ui.transform", value));
    }
}

/// Lays out the controls centered in `header` between `left` and `right`
/// (x); returns a chosen command.
pub fn show(app: &PixelApp, ui: &mut egui::Ui, header: Rect, left: f32, right: f32) -> Chosen {
    let tk = Tokens::get();
    let s = &app.ui.transform;
    let orient_label = s.orientation.label();
    let w_orient = dropdown_width(ui, Some(orient_label));
    let w_pivot = dropdown_width(ui, None);
    let w_toggle = H + 18.0;
    let gap = 6.0;
    let total = w_orient + gap + w_pivot + gap + (w_toggle + PAD * 2.0) * 2.0 + gap;
    let x0 = (header.center().x - total / 2.0).max(left + 12.0);
    if x0 + total > right - 12.0 {
        return None;
    }
    let cy = header.center().y;
    let mut chosen: Chosen = None;
    let track = |ui: &egui::Ui, x: f32, w: f32| {
        let r = Rect::from_min_size(pos2(x, cy - H / 2.0 - PAD), vec2(w, H + PAD * 2.0));
        ui.painter().rect_filled(r, 8.0, tk.chrome_dark);
        r
    };

    // Orientation and pivot dropdowns share one track.
    let t1 = track(ui, x0, w_orient + w_pivot + PAD * 3.0);
    let r_orient = Rect::from_min_size(pos2(t1.left() + PAD, cy - H / 2.0), vec2(w_orient, H));
    let tip = Tip::new(t("Transform orientation")).desc(t("The axes the gizmo moves, turns and scales along"));
    let resp = dropdown(ui, r_orient, Icon::Orientation, Some(orient_label), tip);
    popover(&resp, ui, 210.0, false, |ui| {
        widgets::menu_header(ui, t("Transform Orientation"));
        for o in Orientation::ALL {
            radio(ui, o.label(), s.orientation == *o, json!({ "orientation": o }), &mut chosen);
        }
    });
    let r_pivot = Rect::from_min_size(pos2(r_orient.right() + PAD, cy - H / 2.0), vec2(w_pivot, H));
    let tip = Tip::new(t("Pivot point")).desc(s.pivot.name());
    let resp = dropdown(ui, r_pivot, Icon::Pivot, None, tip);
    popover(&resp, ui, 230.0, false, |ui| {
        widgets::menu_header(ui, t("Pivot Point"));
        for p in [Pivot::BoundsCenter, Pivot::Cursor, Pivot::Individual, Pivot::Median, Pivot::Active] {
            radio(ui, p.name(), s.pivot == p, json!({ "pivot": p }), &mut chosen);
        }
    });

    // Snapping: the toggle and its settings.
    let t2 = track(ui, t1.right() + gap, w_toggle + PAD * 2.0);
    let r_snap = Rect::from_min_size(pos2(t2.left() + PAD, cy - H / 2.0), vec2(H, H));
    let snap_tip = Tip::new(t("Snap")).desc(t("Snap moves, turns and scaling as set in the menu next to it")).keys("Shift+Tab");
    let on = ui
        .scope_builder(egui::UiBuilder::new().max_rect(r_snap), |ui| {
            icons::button_with(ui, Icon::Snap, r_snap.size(), snap_tip, app.ui.view.snap, Look::Segment, egui::RectAlign::BOTTOM)
        })
        .inner;
    if on.clicked() {
        chosen = Some(("view.snap", Value::Null));
    }
    let r_snap_menu = Rect::from_min_size(pos2(r_snap.right(), cy - H / 2.0), vec2(18.0, H));
    let resp = chevron_button(ui, r_snap_menu, t("Snapping"));
    popover(&resp, ui, 250.0, true, |ui| {
        widgets::menu_header(ui, t("Snap With"));
        for b in SnapBase::ALL {
            radio(ui, b.label(), s.snap_base == *b, json!({ "snapBase": b }), &mut chosen);
        }
        widgets::menu_separator(ui);
        widgets::menu_header(ui, t("Snap To"));
        for target in SnapTarget::ALL {
            radio(ui, target.label(), s.snaps_to(*target), json!({ "snapTarget": target }), &mut chosen);
        }
        widgets::menu_separator(ui);
        widgets::menu_header(ui, t("Snap Individual Elements To"));
        radio(ui, t("Face Project"), s.face_project, json!({ "faceProject": "toggle" }), &mut chosen);
        radio(ui, t("Face Nearest"), s.face_nearest, json!({ "faceNearest": "toggle" }), &mut chosen);
        widgets::menu_separator(ui);
        widgets::menu_header(ui, t("Target Selection"));
        radio(ui, t("Include Active"), s.include_active, json!({ "includeActive": "toggle" }), &mut chosen);
        radio(ui, t("Include Edited"), s.include_edited, json!({ "includeEdited": "toggle" }), &mut chosen);
        radio(ui, t("Include Non-edited"), s.include_nonedited, json!({ "includeNonedited": "toggle" }), &mut chosen);
        radio(ui, t("Exclude Non-selectable"), s.exclude_nonselectable, json!({ "excludeNonselectable": "toggle" }), &mut chosen);
        widgets::menu_separator(ui);
        widgets::menu_header(ui, t("Affect"));
        radio(ui, t("Move"), s.affect_move, json!({ "affect": "move" }), &mut chosen);
        radio(ui, t("Rotate"), s.affect_rotate, json!({ "affect": "rotate" }), &mut chosen);
        radio(ui, t("Scale"), s.affect_scale, json!({ "affect": "scale" }), &mut chosen);
    });

    // Proportional editing: the toggle and its falloff.
    let t3 = track(ui, t2.right() + gap, w_toggle + PAD * 2.0);
    let r_prop = Rect::from_min_size(pos2(t3.left() + PAD, cy - H / 2.0), vec2(H, H));
    let prop_tip = Tip::new(t("Proportional editing")).desc(t("Moves nearby vertices too, fading with distance")).keys("O");
    let on = ui
        .scope_builder(egui::UiBuilder::new().max_rect(r_prop), |ui| {
            icons::button_with(ui, Icon::Proportional, r_prop.size(), prop_tip, s.proportional, Look::Segment, egui::RectAlign::BOTTOM)
        })
        .inner;
    if on.clicked() {
        chosen = Some(("ui.transform", json!({ "proportional": "toggle" })));
    }
    let r_prop_menu = Rect::from_min_size(pos2(r_prop.right(), cy - H / 2.0), vec2(18.0, H));
    let resp = chevron_button(ui, r_prop_menu, t("Proportional editing falloff"));
    popover(&resp, ui, 230.0, true, |ui| {
        widgets::menu_header(ui, t("Falloff"));
        for f in Falloff::ALL {
            radio(ui, f.label(), s.falloff == *f, json!({ "falloff": f }), &mut chosen);
        }
        widgets::menu_separator(ui);
        radio(ui, t("Connected Only"), s.connected_only, json!({ "connectedOnly": "toggle" }), &mut chosen);
        radio(ui, t("Projected from View"), s.projected, json!({ "projected": "toggle" }), &mut chosen);
    });
    chosen
}

/// The narrow chevron next to a toggle that opens its settings.
fn chevron_button(ui: &mut egui::Ui, rect: Rect, title: &str) -> egui::Response {
    let tk = Tokens::get();
    let resp = ui.interact(rect, ui.id().with(("chevron", title)), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), resp.hovered() || open, 0.1);
    if hot > 0.0 {
        ui.painter().rect_filled(rect, 6.0, tk.control.gamma_multiply(hot));
    }
    widgets::chevron(ui.painter(), rect.center(), 1.0, widgets::mix(tk.text_faint, tk.text, hot));
    icons::describe_control(ui, &resp, title, Some(open));
    widgets::tooltip(&resp, Tip::new(title), egui::RectAlign::BOTTOM);
    resp
}
