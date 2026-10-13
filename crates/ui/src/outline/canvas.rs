//! A 2D canvas of the outline editor: one view's outlines on a millimetre
//! grid, the other views' extents as dashed guides, points (squares for
//! corners, circles for smooth points) and the selected point's handles.
//! Dragging a point or a handle previews in the background and commits on
//! release; a double click on a segment adds a point there, on a point
//! turns it smooth or corner. Points snap to the guides, and to the grid
//! while Snap is on (or Cmd or Ctrl is held).

use egui::{Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use glam::DVec2;

use super::path;
use super::{Apply, Grab, Sel, ViewId};
use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

/// How the view's meters map onto the canvas (y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub center: DVec2,
    /// Points per meter.
    pub scale: f64,
    pub rect: Rect,
}

impl Fit {
    /// Fits `lo` to `hi` into `rect` with a margin.
    pub fn new(rect: Rect, lo: DVec2, hi: DVec2) -> Fit {
        let size = (hi - lo).max(DVec2::splat(1e-3));
        let scale = (f64::from(rect.width()) / size.x).min(f64::from(rect.height()) / size.y) * 0.78;
        Fit { center: (lo + hi) * 0.5, scale, rect }
    }

    pub fn to_screen(&self, p: DVec2) -> Pos2 {
        let c = self.rect.center();
        pos2(c.x + ((p.x - self.center.x) * self.scale) as f32, c.y - ((p.y - self.center.y) * self.scale) as f32)
    }

    pub fn to_world(&self, s: Pos2) -> DVec2 {
        let c = self.rect.center();
        DVec2::new(self.center.x + f64::from(s.x - c.x) / self.scale, self.center.y - f64::from(s.y - c.y) / self.scale)
    }

    /// The same mapping on another rectangle (the canvas moved).
    fn moved(self, rect: Rect) -> Fit {
        let scale = self.scale * f64::from(rect.width() / self.rect.width().max(1.0));
        Fit { rect, scale, ..self }
    }
}

/// The grid step in meters: the finest 1-2-5 step in millimetres at least
/// 9 points apart.
pub fn grid_step(scale: f64) -> f64 {
    const STEPS: [f64; 13] = [0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0];
    STEPS.into_iter().find(|s| s * scale >= 9.0).unwrap_or(10.0)
}

/// A dragged point snapped to the guides near it (within 6 points), else to
/// the grid when `grid` is given.
pub fn snap(fit: &Fit, p: DVec2, guides: &(Vec<f64>, Vec<f64>), grid: Option<f64>) -> DVec2 {
    let tol = 6.0 / fit.scale;
    let pick = |v: f64, lines: &[f64]| lines.iter().copied().filter(|l| (l - v).abs() <= tol).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()));
    let on_grid = |v: f64| grid.map(|g| (v / g).round() * g);
    DVec2::new(pick(p.x, &guides.0).or_else(|| on_grid(p.x)).unwrap_or(p.x), pick(p.y, &guides.1).or_else(|| on_grid(p.y)).unwrap_or(p.y))
}

/// What the pointer is on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hit {
    Handle(Sel, bool),
    Node(Sel),
    /// A segment (its first point's selection) at `t`.
    Segment(Sel, f64),
}

/// The handle, point or segment under `at`.
///
/// Complexity: O(nodes of the view).
fn hit(app: &PixelApp, view: ViewId, fit: &Fit, at: Pos2) -> Option<Hit> {
    let e = &app.outline;
    if let Some(s) = e.selected
        && let Some(n) = e.node(s)
    {
        for (out, h) in [(true, n.h_out), (false, n.h_in)] {
            if h != DVec2::ZERO && fit.to_screen(n.p + h).distance(at) <= 7.0 {
                return Some(Hit::Handle(s, out));
            }
        }
    }
    let mut best: Option<(f32, Sel)> = None;
    for (layer, l) in e.layers_in(view) {
        for (contour, c) in l.outline.contours.iter().enumerate() {
            for (node, n) in c.iter().enumerate() {
                let d = fit.to_screen(n.p).distance(at);
                if d <= 7.0 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, Sel { layer, contour, node }));
                }
            }
        }
    }
    if let Some((_, s)) = best {
        return Some(Hit::Node(s));
    }
    let q = fit.to_world(at);
    let mut seg: Option<(f64, Sel, f64)> = None;
    for (layer, l) in e.layers_in(view) {
        for (contour, c) in l.outline.contours.iter().enumerate() {
            for i in 0..c.len() {
                let (tt, d) = path::segment(c, i).nearest(q);
                if d * fit.scale <= 6.0 && seg.is_none_or(|b| d < b.0) {
                    seg = Some((d, Sel { layer, contour, node: i }, tt));
                }
            }
        }
    }
    seg.map(|(_, s, tt)| Hit::Segment(s, tt))
}

/// The fit of a view: its outlines and its guides, kept still while a
/// drag runs.
///
/// Complexity: O(nodes).
fn fit_of(app: &PixelApp, view: ViewId, rect: Rect, interactive: bool) -> Fit {
    if interactive && let Some(f) = app.outline.frozen {
        return f.moved(rect);
    }
    let e = &app.outline;
    let (gx, gy) = e.guides(view);
    let mut b = e.view_bounds(view);
    for (xs, ys) in [(gx.as_slice(), &[][..]), (&[][..], gy.as_slice())] {
        for x in xs {
            b = b.map(|(lo, hi)| (lo.min(DVec2::new(*x, lo.y)), hi.max(DVec2::new(*x, hi.y))));
        }
        for y in ys {
            b = b.map(|(lo, hi)| (lo.min(DVec2::new(lo.x, *y)), hi.max(DVec2::new(hi.x, *y))));
        }
    }
    let (lo, hi) = b.unwrap_or((DVec2::splat(-0.05), DVec2::splat(0.05)));
    Fit::new(rect, lo, hi)
}

/// Draws one view; an interactive canvas also edits it. Returns the
/// canvas response (a thumbnail's click selects its view).
///
/// Complexity: O(nodes x curve pieces + grid lines) per frame.
pub fn show(app: &mut PixelApp, ui: &mut egui::Ui, view: ViewId, size: egui::Vec2, interactive: bool) -> egui::Response {
    let tk = Tokens::get();
    let sense = if interactive { Sense::click_and_drag() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let fit = fit_of(app, view, rect, interactive);
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0, tk.viewport_bottom);
    // The grid, with the axes through the origin.
    let step = grid_step(fit.scale);
    let (lo, hi) = (fit.to_world(rect.left_bottom()), fit.to_world(rect.right_top()));
    let lines = |a: f64, b: f64| ((a / step).floor() as i64)..=((b / step).ceil() as i64);
    // Major lines every 10 mm for 2 mm steps, else every tenth line.
    let every = if (step * 1000.0).round() as i64 % 2 == 0 { 5 } else { 10 };
    let major = |i: i64| i % every == 0;
    let (ax, ay) = view.axes();
    let axis_color = |a: &str| match a {
        "x" => tk.axis_x,
        "y" => tk.axis_y,
        _ => tk.axis_z,
    };
    if (hi.x - lo.x) / step < 400.0 && (hi.y - lo.y) / step < 400.0 {
        for i in lines(lo.x, hi.x) {
            let x = fit.to_screen(DVec2::new(i as f64 * step, 0.0)).x;
            let c = if i == 0 {
                axis_color(ay).gamma_multiply(0.7)
            } else if major(i) {
                tk.grid_major
            } else {
                tk.grid_minor
            };
            p.vline(x, rect.y_range(), Stroke::new(1.0, c));
        }
        for i in lines(lo.y, hi.y) {
            let y = fit.to_screen(DVec2::new(0.0, i as f64 * step)).y;
            let c = if i == 0 {
                axis_color(ax).gamma_multiply(0.7)
            } else if major(i) {
                tk.grid_major
            } else {
                tk.grid_minor
            };
            p.hline(rect.x_range(), y, Stroke::new(1.0, c));
        }
    }
    // The other views' extents.
    let guides = app.outline.guides(view);
    let dash = Stroke::new(1.0, tk.accent.gamma_multiply(0.7));
    for x in &guides.0 {
        let sx = fit.to_screen(DVec2::new(*x, 0.0)).x;
        p.extend(Shape::dashed_line(&[pos2(sx, rect.top()), pos2(sx, rect.bottom())], dash, 4.0, 4.0));
    }
    for y in &guides.1 {
        let sy = fit.to_screen(DVec2::new(0.0, *y)).y;
        p.extend(Shape::dashed_line(&[pos2(rect.left(), sy), pos2(rect.right(), sy)], dash, 4.0, 4.0));
    }
    // The outlines.
    let selected = app.outline.selected;
    let mut screen = Vec::new();
    let empty = app.outline.layers_in(view).next().is_none();
    for (li, l) in app.outline.layers_in(view) {
        let hole = matches!(l.key, super::LayerKey::Hole(_));
        let stroke = Stroke::new(if interactive { 1.6 } else { 1.2 }, if hole { tk.health_warn } else { tk.text });
        for (ci, c) in l.outline.contours.iter().enumerate() {
            let pts: Vec<Pos2> = path::flatten(c, 16).into_iter().map(|q| fit.to_screen(q)).collect();
            p.add(Shape::line(pts, stroke));
            if !interactive {
                continue;
            }
            for (ni, n) in c.iter().enumerate() {
                let s = Sel { layer: li, contour: ci, node: ni };
                let at = fit.to_screen(n.p);
                screen.push((s, at));
                let on = selected == Some(s);
                let fill = if on { tk.accent } else { tk.viewport_bottom };
                let edge = Stroke::new(1.2, if on { tk.accent } else { tk.text });
                if n.smooth {
                    p.circle(at, 4.0, fill, edge);
                } else {
                    p.rect(Rect::from_center_size(at, vec2(7.0, 7.0)), 1.0, fill, edge, egui::StrokeKind::Middle);
                }
                if on {
                    for h in [n.h_in, n.h_out] {
                        if h != DVec2::ZERO {
                            let q = fit.to_screen(n.p + h);
                            p.line_segment([at, q], Stroke::new(1.0, tk.accent.gamma_multiply(0.8)));
                            p.circle(q, 3.5, tk.viewport_bottom, Stroke::new(1.2, tk.accent));
                        }
                    }
                }
            }
        }
    }
    let label = |text: String, pos: Pos2, align: egui::Align2| {
        p.text(pos, align, text, theme::small(), tk.text_faint);
    };
    label(view.label().to_owned(), rect.left_top() + vec2(6.0, 4.0), egui::Align2::LEFT_TOP);
    if interactive {
        label(format!("{} {}", fmt_mm(step), t("grid")), rect.left_bottom() + vec2(6.0, -4.0), egui::Align2::LEFT_BOTTOM);
        label(format!("{ax} {}, {ay} {}", t("right"), t("up")), rect.right_bottom() + vec2(-6.0, -4.0), egui::Align2::RIGHT_BOTTOM);
    }
    if empty {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, t("No outline in this view"), theme::small(), tk.text_dim);
    }
    if !interactive {
        if resp.hovered() {
            p.rect_stroke(rect, 6.0, Stroke::new(1.0, tk.accent), egui::StrokeKind::Inside);
        }
        return resp;
    }
    app.outline.screen = screen;
    app.outline.canvas = Some(rect);
    if resp.hovered() {
        app.outline.canvas_hover_next = true;
    }
    edit(app, ui, view, &fit, &resp, &guides);
    resp
}

/// The canvas's clicks and drags.
fn edit(app: &mut PixelApp, ui: &egui::Ui, view: ViewId, fit: &Fit, resp: &egui::Response, guides: &(Vec<f64>, Vec<f64>)) {
    let grid_snap = app.ui.view.snap || ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    let grid = grid_snap.then(|| grid_step(fit.scale));
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(resp.rect.center());
        app.outline.grab = match hit(app, view, fit, origin) {
            Some(Hit::Handle(s, out)) => Some(Grab::Handle(s, out)),
            Some(Hit::Node(s)) => Some(Grab::Node(s)),
            _ => None,
        };
        if let Some(Grab::Node(s) | Grab::Handle(s, _)) = app.outline.grab {
            app.outline.selected = Some(s);
            app.outline.frozen = Some(*fit);
        }
    }
    if resp.dragged()
        && let (Some(grab), Some(at)) = (app.outline.grab, resp.interact_pointer_pos())
    {
        let w = fit.to_world(at);
        let e = &mut app.outline;
        let moved = match grab {
            Grab::Node(s) => {
                let to = snap(fit, w, guides, grid);
                e.layers.get_mut(s.layer).and_then(|l| l.outline.contours.get_mut(s.contour)).and_then(|c| c.get_mut(s.node)).is_some_and(|n| {
                    let changed = n.p != to;
                    n.p = to;
                    changed
                })
            }
            Grab::Handle(s, out) => match e.layers.get_mut(s.layer).and_then(|l| l.outline.contours.get_mut(s.contour)) {
                Some(c) if s.node < c.len() => {
                    let h = w - c[s.node].p;
                    let old = if out { c[s.node].h_out } else { c[s.node].h_in };
                    path::set_handle(c, s.node, out, h);
                    old != h
                }
                _ => false,
            },
        };
        if moved {
            app.apply_outline(Apply::Live);
        }
    }
    if resp.drag_stopped() {
        let grab = app.outline.grab.take();
        app.outline.frozen = None;
        if app.outline.live {
            let label = if matches!(grab, Some(Grab::Handle(..))) { t("Shape curve") } else { t("Move point") };
            app.apply_outline(Apply::Commit(label));
        }
    }
    let at = resp.interact_pointer_pos();
    if resp.double_clicked()
        && let Some(at) = at
    {
        match hit(app, view, fit, at) {
            Some(Hit::Node(s)) => {
                let params = serde_json::json!({ "layer": app.outline.layers[s.layer].key.name(), "contour": s.contour, "index": s.node });
                if let Err(e) = app.outline_command("outline.toggle_smooth", &params) {
                    app.set_status(e);
                }
            }
            Some(Hit::Segment(s, tt)) => {
                let params = serde_json::json!({ "layer": app.outline.layers[s.layer].key.name(), "contour": s.contour, "segment": s.node, "t": tt });
                if let Err(e) = app.outline_command("outline.insert_point", &params) {
                    app.set_status(e);
                }
            }
            _ => {}
        }
    } else if resp.clicked()
        && let Some(at) = at
    {
        app.outline.selected = match hit(app, view, fit, at) {
            Some(Hit::Node(s) | Hit::Handle(s, _)) => Some(s),
            _ => None,
        };
    }
    // The hovered segment shows where a double click adds a point.
    if !resp.dragged()
        && let Some(hp) = resp.hover_pos()
        && let Some(Hit::Segment(s, tt)) = hit(app, view, fit, hp)
        && let Some(c) = app.outline.layers.get(s.layer).and_then(|l| l.outline.contours.get(s.contour))
    {
        let q = fit.to_screen(path::segment(c, s.node).at(tt));
        let tk = Tokens::get();
        ui.painter_at(resp.rect).circle_stroke(q, 3.5, Stroke::new(1.0, tk.accent));
        resp.clone().on_hover_text(t("Double-click to add a point"));
    }
}

/// A grid step as text: millimetres below a meter.
fn fmt_mm(step: f64) -> String {
    if step >= 1.0 { format!("{step} m") } else { format!("{} mm", (step * 1e4).round() / 10.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fit_maps_both_ways_with_y_up() {
        let fit = Fit::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 100.0)), DVec2::new(-0.1, 0.0), DVec2::new(0.1, 0.1));
        let s = fit.to_screen(DVec2::new(0.1, 0.1));
        assert!(s.x > 100.0 && s.y < 50.0, "{s:?}");
        assert!(fit.to_world(s).distance(DVec2::new(0.1, 0.1)) < 1e-6);
        // 0.2 m across fits 78% of 200 or 0.1 m 78% of 100: the smaller.
        assert!((fit.scale - 780.0).abs() < 1e-6, "{}", fit.scale);
    }

    #[test]
    fn grid_steps_and_snapping() {
        assert_eq!(grid_step(10_000.0), 0.001);
        assert_eq!(grid_step(1_000.0), 0.01);
        assert_eq!(grid_step(300.0), 0.05);
        let fit = Fit::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0)), DVec2::ZERO, DVec2::splat(0.1));
        let guides = (vec![0.05], vec![]);
        // Near a guide: on it; elsewhere on the grid, or left alone.
        let p = snap(&fit, DVec2::new(0.0503, 0.0234), &guides, Some(0.005));
        assert!((p.x - 0.05).abs() < 1e-12 && (p.y - 0.025).abs() < 1e-12, "{p:?}");
        let q = snap(&fit, DVec2::new(0.0234, 0.0234), &guides, None);
        assert_eq!(q, DVec2::new(0.0234, 0.0234));
        assert_eq!(fmt_mm(0.005), "5 mm");
        assert_eq!(fmt_mm(0.0005), "0.5 mm");
    }
}
