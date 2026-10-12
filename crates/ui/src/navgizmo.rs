//! The navigation gizmo in the top right corner of the viewport, modeled on
//! Blender's: one ball per axis direction (labeled balls for +X, +Y, +Z,
//! fainter balls for the negative directions), drawn as the camera sees the
//! world axes. Clicking a ball looks along that axis (again: from the other
//! side), dragging the gizmo orbits, and the round buttons below it zoom,
//! pan, switch between perspective and orthographic, and frame the
//! selection.

use egui::{Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};
use glam::Vec3;

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::viewport::ViewportState;

/// Radius of the gizmo area.
const RADIUS: f32 = 46.0;
/// Radius of an axis ball.
const BALL: f32 = 9.0;
/// Distance of the balls from the center.
const REACH: f32 = 34.0;
/// Radius and spacing of the round buttons below the gizmo.
const BUTTON: f32 = 14.0;
const BUTTON_GAP: f32 = 34.0;

/// What the gizmo asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavCommand {
    ToggleOrtho,
    Frame,
}

/// One axis ball.
struct Ball {
    axis: Vec3,
    label: &'static str,
    color: Color32,
    positive: bool,
    pos: Pos2,
    /// Toward the camera: larger is nearer.
    depth: f32,
}

fn balls(vp: &ViewportState, center: Pos2) -> Vec<Ball> {
    let tk = Tokens::get();
    let view = vp.camera.view();
    let axes = [
        (Vec3::X, "X", tk.axis_x, true),
        (Vec3::Y, "Y", tk.axis_y, true),
        (Vec3::Z, "Z", tk.axis_z, true),
        (Vec3::NEG_X, "-X", tk.axis_x, false),
        (Vec3::NEG_Y, "-Y", tk.axis_y, false),
        (Vec3::NEG_Z, "-Z", tk.axis_z, false),
    ];
    let mut out: Vec<Ball> = axes
        .into_iter()
        .map(|(axis, label, color, positive)| {
            let v = view.transform_vector3(axis);
            Ball { axis, label, color, positive, pos: center + vec2(v.x, -v.y) * REACH, depth: v.z }
        })
        .collect();
    // Far balls first, so near ones draw on top.
    out.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    out
}

/// The front-most ball under `p`.
fn ball_at(balls: &[Ball], p: Pos2) -> Option<usize> {
    balls.iter().enumerate().rev().find(|(_, b)| b.pos.distance(p) <= BALL + 1.5).map(|(i, _)| i)
}

/// Draws the gizmo and its buttons in the top right of `view` and handles
/// their input. Call after the viewport has handled its own input, so the
/// gizmo wins where they overlap.
pub fn show(ui: &mut egui::Ui, view: Rect, vp: &mut ViewportState, ortho: bool) -> Option<NavCommand> {
    let tk = Tokens::get();
    let center = pos2(view.right() - RADIUS - 10.0, view.top() + RADIUS + 10.0);
    if !view.contains(center - vec2(RADIUS, RADIUS)) {
        return None;
    }
    let painter = ui.painter_at(view);
    let area = Rect::from_center_size(center, vec2(2.0 * RADIUS, 2.0 * RADIUS));
    let resp = ui.interact(area, ui.id().with("navgizmo"), Sense::click_and_drag());
    let pointer = resp.hover_pos().filter(|p| p.distance(center) <= RADIUS);
    let list = balls(vp, center);
    let hovered = pointer.and_then(|p| ball_at(&list, p));
    if pointer.is_some() || resp.dragged() {
        painter.circle_filled(center, RADIUS, Color32::from_white_alpha(18));
    }
    for b in &list {
        if b.positive {
            painter.line_segment([center, b.pos], Stroke::new(2.0, b.color));
        }
    }
    for (i, b) in list.iter().enumerate() {
        let hot = hovered == Some(i);
        if b.positive {
            painter.circle_filled(b.pos, BALL, if hot { b.color.gamma_multiply(1.25) } else { b.color });
            painter.text(b.pos, Align2::CENTER_CENTER, b.label, FontId::proportional(11.0), Color32::from_rgb(0x12, 0x14, 0x18));
        } else {
            painter.circle_filled(b.pos, BALL - 1.0, b.color.gamma_multiply(if hot { 0.75 } else { 0.35 }));
            painter.circle_stroke(b.pos, BALL - 1.0, Stroke::new(1.2, b.color.gamma_multiply(0.85)));
            if hot {
                painter.text(b.pos, Align2::CENTER_CENTER, b.label, FontId::proportional(10.0), Color32::WHITE);
            }
        }
    }
    let label = match hovered {
        Some(i) => format!("{} {}{}", t("View from"), if list[i].positive { "+" } else { "" }, list[i].label),
        None => t("Drag to orbit; click an axis to view from it").to_owned(),
    };
    icons::describe_control(ui, &resp, t("Navigation gizmo"), None);
    let resp = resp.on_hover_text(label);
    if resp.dragged() {
        let d = resp.drag_delta();
        vp.cancel_anim();
        vp.camera.orbit(d.x * 0.012, d.y * 0.012);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if pointer.is_some() {
        ui.ctx().set_cursor_icon(if hovered.is_some() { CursorIcon::PointingHand } else { CursorIcon::Grab });
    }
    if resp.clicked()
        && let Some(i) = hovered
    {
        let axis = list[i].axis;
        // Clicking the axis already looked along flips to the other side.
        let toward = if vp.looks_from(axis) { -axis } else { axis };
        vp.look_from(toward);
    }

    // Round buttons: zoom and pan by dragging, perspective toggle, frame.
    let mut command = None;
    let buttons: [(Icon, &str, &str); 4] = [
        (Icon::Search, "zoom", "Drag up or down to zoom"),
        (Icon::Move, "pan", "Drag to pan the view"),
        (if ortho { Icon::Orthographic } else { Icon::Perspective }, "ortho", "Switch between perspective and orthographic (5)"),
        (Icon::Frame, "frame", "Frame the selection, or everything (F)"),
    ];
    for (k, (icon, id, tip)) in buttons.into_iter().enumerate() {
        let c = pos2(center.x, center.y + RADIUS + 8.0 + BUTTON + k as f32 * BUTTON_GAP);
        if c.y + BUTTON > view.bottom() {
            break;
        }
        let rect = Rect::from_center_size(c, vec2(2.0 * BUTTON, 2.0 * BUTTON));
        let r = ui.interact(rect, ui.id().with(("navbutton", id)), Sense::click_and_drag());
        let fill = if r.hovered() || r.dragged() { tk.control_hover } else { tk.chrome_dark.gamma_multiply(0.85) };
        painter.circle_filled(c, BUTTON, fill);
        icons::paint(&painter, Rect::from_center_size(c, vec2(16.0, 16.0)), icon, false);
        icons::describe_control(ui, &r, t(tip), None);
        let r = r.on_hover_text(t(tip));
        match id {
            "zoom" if r.dragged() => {
                vp.camera.zoom((1.0 + r.drag_delta().y * 0.01).clamp(0.5, 2.0), None);
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            }
            "pan" if r.dragged() => {
                let d = r.drag_delta();
                vp.camera.pan(d.x, d.y, view.height());
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            }
            "ortho" if r.clicked() => command = Some(NavCommand::ToggleOrtho),
            "frame" if r.clicked() => command = Some(NavCommand::Frame),
            _ => {}
        }
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use p3d_render::ViewKind;

    #[test]
    fn balls_follow_the_camera() {
        let mut vp = ViewportState::default();
        vp.camera.set_view(ViewKind::Front);
        let c = pos2(100.0, 100.0);
        let list = balls(&vp, c);
        let find = |label: &str| list.iter().find(|b| b.label == label).map(|b| (b.pos, b.depth)).unwrap_or_default();
        let (x, _) = find("X");
        let (y, _) = find("Y");
        let (_, z_depth) = find("Z");
        assert!(x.x > c.x + 30.0 && (x.y - c.y).abs() < 1.0, "+X points right from the front: {x:?}");
        assert!(y.y < c.y - 30.0, "+Y points up: {y:?}");
        assert!(z_depth > 0.9, "+Z points at the camera");
        // The nearest ball is drawn last and wins the hit test.
        assert_eq!(list.last().map(|b| b.label), Some("Z"));
        assert_eq!(ball_at(&list, c).map(|i| list[i].label), Some("Z"));
    }

    #[test]
    fn clicking_the_current_axis_flips_to_the_other_side() {
        let mut vp = ViewportState::default();
        vp.look_from(Vec3::X);
        assert!(vp.looks_from(Vec3::X), "the target view counts while animating");
        assert!(!vp.looks_from(Vec3::NEG_X));
        vp.camera.look_from(Vec3::Y);
        vp.cancel_anim();
        assert!(vp.looks_from(Vec3::Y));
    }
}
