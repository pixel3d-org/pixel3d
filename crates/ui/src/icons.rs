//! The icon set, painted in code on a 24 x 24 grid (no image assets): light
//! strokes for existing geometry, accent blue for what a tool creates,
//! orange dots for points (cadcraft's convention).

use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    New,
    Open,
    Save,
    Undo,
    Redo,
    Select,
    Move,
    Rotate,
    Scale,
    ModeObject,
    ModeVert,
    ModeEdge,
    ModeFace,
    Box,
    Cylinder,
    Sphere,
    Plane,
    Frame,
    ShadeSolid,
    ShadeWire,
    ShadeSolidWire,
    ShadeMatcap,
    Grid,
    Perspective,
    Orthographic,
    Search,
    Gear,
    Help,
    Plus,
    Close,
    ChevronDown,
    ChevronRight,
    Menu,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    Mesh,
    Empty,
    Health,
    History,
    Outliner,
    Properties,
    Command,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    w: f32,
    base: Color32,
    acc: Color32,
    pt: Color32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        let s = self.r.width() / 24.0;
        pos2(self.r.min.x + x * s, self.r.min.y + y * s)
    }
    fn sc(&self) -> f32 {
        self.r.width() / 24.0
    }
    fn st(&self, c: Color32) -> Stroke {
        Stroke::new(self.w, c)
    }
    fn l(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color32) {
        self.p.line_segment([self.at(x1, y1), self.at(x2, y2)], self.st(c));
    }
    fn poly(&self, pts: &[(f32, f32)], closed: bool, c: Color32) {
        let mut v: Vec<Pos2> = pts.iter().map(|(x, y)| self.at(*x, *y)).collect();
        if closed && let Some(f) = v.first().copied() {
            v.push(f);
        }
        self.p.line(v, self.st(c));
    }
    fn fill(&self, pts: &[(f32, f32)], c: Color32) {
        self.p.add(egui::Shape::convex_polygon(pts.iter().map(|(x, y)| self.at(*x, *y)).collect(), c, Stroke::NONE));
    }
    fn c(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_stroke(self.at(x, y), r * self.sc(), self.st(c));
    }
    fn fill_c(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.at(x, y), r * self.sc(), c);
    }
    /// Arc in degrees, counter-clockwise on screen.
    fn arc(&self, x: f32, y: f32, r: f32, a0: f32, a1: f32, c: Color32) {
        self.ellipse_arc((x, y), (r, r), (a0, a1), c);
    }
    fn ellipse_arc(&self, (x, y): (f32, f32), (rx, ry): (f32, f32), (a0, a1): (f32, f32), c: Color32) {
        let n = (((a1 - a0).abs() / 10.0).ceil() as usize).max(2);
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / n as f32).to_radians();
                self.at(x + rx * a.cos(), y - ry * a.sin())
            })
            .collect();
        self.p.line(pts, self.st(c));
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        self.poly(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], true, c);
    }
    fn dot(&self, x: f32, y: f32) {
        self.fill_c(x, y, 1.7, self.pt);
    }
    fn arrow_tip(&self, x: f32, y: f32, dx: f32, dy: f32, c: Color32) {
        let l = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / l, dy / l);
        let (nx, ny) = (-uy, ux);
        self.fill(&[(x, y), (x - ux * 3.2 + nx * 1.6, y - uy * 3.2 + ny * 1.6), (x - ux * 3.2 - nx * 1.6, y - uy * 3.2 - ny * 1.6)], c);
    }
    fn arrow(&self, x1: f32, y1: f32, x2: f32, y2: f32, c: Color32) {
        self.l(x1, y1, x2, y2, c);
        self.arrow_tip(x2, y2, x2 - x1, y2 - y1, c);
    }
}

/// Corners of an isometric cube: top face (4) then bottom face (4).
/// Front corner at the bottom center; `s` is the half width.
fn iso_cube(cx: f32, cy: f32, s: f32) -> [(f32, f32); 8] {
    let h = s * 0.58;
    let top = [(cx, cy - 2.0 * h), (cx + s, cy - h), (cx, cy), (cx - s, cy - h)];
    let d = 2.0 * h + 0.6;
    [top[0], top[1], top[2], top[3], (top[0].0, top[0].1 + d), (top[1].0, top[1].1 + d), (top[2].0, top[2].1 + d), (top[3].0, top[3].1 + d)]
}

fn draw_cube(pen: &Pen<'_>, k: &[(f32, f32); 8], c: Color32) {
    pen.poly(&k[..4], true, c);
    pen.l(k[1].0, k[1].1, k[5].0, k[5].1, c);
    pen.l(k[2].0, k[2].1, k[6].0, k[6].1, c);
    pen.l(k[3].0, k[3].1, k[7].0, k[7].1, c);
    pen.poly(&[k[3], k[7], k[6], k[5], k[1]], false, c);
}

/// Paints `icon` into the square `rect`; `dim` for disabled controls.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, dim: bool) {
    let t = Tokens::get();
    let alpha = |c: Color32| if dim { c.gamma_multiply(0.4) } else { c };
    let pen = Pen { p, r: rect, w: (rect.width() / 16.0).clamp(1.0, 1.6), base: alpha(t.icon), acc: alpha(t.icon_accent), pt: alpha(t.icon_point) };
    let (b, a) = (pen.base, pen.acc);
    use Icon::*;
    match icon {
        New => {
            pen.poly(&[(6.0, 3.0), (15.0, 3.0), (19.0, 7.0), (19.0, 21.0), (6.0, 21.0)], true, b);
            pen.poly(&[(15.0, 3.0), (15.0, 7.0), (19.0, 7.0)], false, b);
            pen.l(12.5, 11.0, 12.5, 17.0, a);
            pen.l(9.5, 14.0, 15.5, 14.0, a);
        }
        Open => {
            pen.poly(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (20.0, 8.0), (20.0, 19.0), (3.0, 19.0)], true, b);
            pen.poly(&[(3.0, 19.0), (6.0, 11.0), (22.0, 11.0), (20.0, 19.0)], false, b);
        }
        Save => {
            pen.poly(&[(4.0, 4.0), (17.0, 4.0), (20.0, 7.0), (20.0, 20.0), (4.0, 20.0)], true, b);
            pen.rect(8.0, 4.0, 8.0, 5.0, b);
            pen.rect(7.0, 13.0, 10.0, 7.0, b);
        }
        Undo => {
            pen.arc(12.0, 13.0, 7.0, 150.0, -40.0, b);
            pen.arrow_tip(5.0, 9.0, -1.0, 2.5, b);
        }
        Redo => {
            pen.arc(12.0, 13.0, 7.0, 30.0, 220.0, b);
            pen.arrow_tip(19.0, 9.0, 1.0, 2.5, b);
        }
        Select => {
            pen.poly(&[(6.0, 3.0), (6.0, 19.0), (10.0, 15.0), (13.0, 21.0), (15.5, 20.0), (12.5, 14.0), (18.0, 14.0)], true, b);
        }
        Move => {
            pen.arrow(12.0, 12.0, 12.0, 3.0, a);
            pen.arrow(12.0, 12.0, 12.0, 21.0, a);
            pen.arrow(12.0, 12.0, 3.0, 12.0, a);
            pen.arrow(12.0, 12.0, 21.0, 12.0, a);
            pen.fill_c(12.0, 12.0, 1.6, b);
        }
        Rotate => {
            pen.c(12.0, 12.0, 3.0, b);
            pen.arc(12.0, 12.0, 8.0, 100.0, 380.0, a);
            pen.arrow_tip(10.6, 4.1, -1.0, 0.1, a);
        }
        Scale => {
            pen.rect(3.0, 9.0, 12.0, 12.0, b);
            pen.rect(3.0, 3.0, 18.0, 18.0, b.gamma_multiply(0.5));
            pen.arrow(10.0, 14.0, 20.0, 4.0, a);
        }
        ModeObject => {
            let k = iso_cube(12.0, 12.5, 8.0);
            pen.fill(&[k[0], k[1], k[2], k[3]], a.gamma_multiply(0.35));
            draw_cube(&pen, &k, b);
        }
        ModeVert => {
            let k = iso_cube(12.0, 12.5, 8.0);
            draw_cube(&pen, &k, b.gamma_multiply(0.7));
            for &(x, y) in &[k[0], k[1], k[2], k[3], k[5], k[6], k[7]] {
                pen.dot(x, y);
            }
        }
        ModeEdge => {
            let k = iso_cube(12.0, 12.5, 8.0);
            draw_cube(&pen, &k, b.gamma_multiply(0.7));
            let w = pen.w * 1.8;
            p.line_segment([pen.at(k[2].0, k[2].1), pen.at(k[6].0, k[6].1)], Stroke::new(w, a));
            p.line_segment([pen.at(k[1].0, k[1].1), pen.at(k[2].0, k[2].1)], Stroke::new(w, a));
        }
        ModeFace => {
            let k = iso_cube(12.0, 12.5, 8.0);
            pen.fill(&[k[2], k[1], k[5], k[6]], a.gamma_multiply(0.7));
            draw_cube(&pen, &k, b.gamma_multiply(0.7));
        }
        Box => {
            let k = iso_cube(12.0, 12.5, 8.0);
            draw_cube(&pen, &k, a);
        }
        Cylinder => {
            pen.ellipse_arc((12.0, 6.0), (7.0, 2.5), (0.0, 360.0), a);
            pen.ellipse_arc((12.0, 18.0), (7.0, 2.5), (180.0, 360.0), a);
            pen.l(5.0, 6.0, 5.0, 18.0, a);
            pen.l(19.0, 6.0, 19.0, 18.0, a);
        }
        Sphere => {
            pen.c(12.0, 12.0, 8.0, a);
            pen.ellipse_arc((12.0, 12.0), (8.0, 2.8), (180.0, 360.0), a);
            pen.ellipse_arc((12.0, 12.0), (2.8, 8.0), (-90.0, 90.0), a.gamma_multiply(0.6));
        }
        Plane => {
            pen.poly(&[(3.0, 15.0), (12.0, 10.0), (21.0, 15.0), (12.0, 20.0)], true, a);
            pen.l(7.5, 12.5, 16.5, 17.5, a.gamma_multiply(0.6));
            pen.l(16.5, 12.5, 7.5, 17.5, a.gamma_multiply(0.6));
        }
        Frame => {
            for (x, y, dx, dy) in [(3.0, 3.0, 1.0, 1.0), (21.0, 3.0, -1.0, 1.0), (3.0, 21.0, 1.0, -1.0), (21.0, 21.0, -1.0, -1.0)] {
                pen.l(x, y, x + 5.0 * dx, y, b);
                pen.l(x, y, x, y + 5.0 * dy, b);
            }
            let k = iso_cube(12.0, 12.0, 5.0);
            draw_cube(&pen, &k, a);
        }
        ShadeSolid | ShadeWire | ShadeSolidWire | ShadeMatcap => {
            let fill = match icon {
                ShadeSolid | ShadeSolidWire => Some(b.gamma_multiply(0.75)),
                ShadeMatcap => Some(Color32::from_rgb(0xb4, 0xa8, 0x98).gamma_multiply(if dim { 0.4 } else { 1.0 })),
                _ => None,
            };
            if let Some(f) = fill {
                pen.fill_c(12.0, 12.0, 8.0, f);
                pen.fill_c(9.5, 9.0, 2.5, Color32::WHITE.gamma_multiply(if icon == ShadeMatcap { 0.7 } else { 0.35 }));
            }
            if matches!(icon, ShadeWire | ShadeSolidWire) {
                let c = if icon == ShadeWire { b } else { t.canvas };
                pen.ellipse_arc((12.0, 12.0), (8.0, 3.0), (0.0, 360.0), c);
                pen.ellipse_arc((12.0, 12.0), (3.0, 8.0), (0.0, 360.0), c);
            }
            pen.c(12.0, 12.0, 8.0, b);
        }
        Grid => {
            for i in 0..4 {
                let x = 4.0 + 16.0 * i as f32 / 3.0;
                pen.l(x, 4.0, x, 20.0, b);
                pen.l(4.0, x, 20.0, x, b);
            }
        }
        Perspective => {
            pen.poly(&[(3.0, 8.0), (21.0, 4.0), (21.0, 20.0), (3.0, 16.0)], true, b);
            pen.l(3.0, 12.0, 21.0, 12.0, a);
        }
        Orthographic => {
            pen.rect(4.0, 6.0, 16.0, 12.0, b);
            pen.l(4.0, 12.0, 20.0, 12.0, a);
        }
        Search => {
            pen.c(10.0, 10.0, 6.0, b);
            pen.l(14.5, 14.5, 20.0, 20.0, b);
        }
        Gear => {
            pen.c(12.0, 12.0, 3.0, b);
            for k in 0..8 {
                let ang = (k as f32 * 45.0).to_radians();
                let (s, c) = ang.sin_cos();
                pen.l(12.0 + 5.5 * c, 12.0 + 5.5 * s, 12.0 + 8.5 * c, 12.0 + 8.5 * s, b);
            }
            pen.c(12.0, 12.0, 6.0, b);
        }
        Help => {
            pen.c(12.0, 12.0, 9.0, b);
            pen.arc(12.0, 9.5, 3.0, 160.0, -60.0, b);
            pen.l(13.5, 12.0, 12.0, 14.0, b);
            pen.fill_c(12.0, 17.5, 1.2, b);
        }
        Plus => {
            pen.l(12.0, 5.0, 12.0, 19.0, b);
            pen.l(5.0, 12.0, 19.0, 12.0, b);
        }
        Close => {
            pen.l(6.0, 6.0, 18.0, 18.0, b);
            pen.l(18.0, 6.0, 6.0, 18.0, b);
        }
        ChevronDown => pen.poly(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)], false, b),
        ChevronRight => pen.poly(&[(9.0, 6.0), (15.0, 12.0), (9.0, 18.0)], false, b),
        Menu => {
            for y in [7.0, 12.0, 17.0] {
                pen.l(5.0, y, 19.0, y, b);
            }
        }
        Eye | EyeOff => {
            pen.ellipse_arc((12.0, 12.0), (9.0, 5.5), (0.0, 360.0), b);
            pen.fill_c(12.0, 12.0, 2.6, b);
            if icon == EyeOff {
                pen.l(4.0, 20.0, 20.0, 4.0, b);
            }
        }
        Lock | Unlock => {
            pen.rect(6.0, 11.0, 12.0, 9.0, b);
            if icon == Lock {
                pen.arc(12.0, 11.0, 4.0, 0.0, 180.0, b);
            } else {
                pen.arc(16.0, 9.0, 4.0, 10.0, 180.0, b);
            }
        }
        Mesh => {
            pen.poly(&[(4.0, 18.0), (12.0, 4.0), (20.0, 18.0)], true, b);
            pen.poly(&[(8.0, 11.0), (16.0, 11.0), (12.0, 18.0)], true, b);
        }
        Empty => {
            pen.l(12.0, 4.0, 12.0, 20.0, b);
            pen.l(4.0, 12.0, 20.0, 12.0, b);
            pen.l(6.5, 17.5, 17.5, 6.5, b.gamma_multiply(0.6));
        }
        Health => {
            pen.poly(&[(3.0, 13.0), (8.0, 13.0), (10.0, 8.0), (13.0, 18.0), (15.5, 11.0), (17.0, 13.0), (21.0, 13.0)], false, a);
        }
        History => {
            pen.c(12.0, 12.0, 8.0, b);
            pen.l(12.0, 12.0, 12.0, 7.0, b);
            pen.l(12.0, 12.0, 15.5, 14.0, b);
        }
        Outliner => {
            pen.rect(3.0, 4.0, 6.0, 4.0, b);
            pen.rect(9.0, 10.0, 6.0, 4.0, b);
            pen.rect(9.0, 16.0, 6.0, 4.0, b);
            pen.poly(&[(6.0, 8.0), (6.0, 18.0), (9.0, 18.0)], false, b);
            pen.l(6.0, 12.0, 9.0, 12.0, b);
        }
        Properties => {
            for (y, x) in [(6.0, 15.0), (12.0, 8.0), (18.0, 13.0)] {
                pen.l(4.0, y, 20.0, y, b);
                pen.fill_c(x, y, 2.0, a);
            }
        }
        Command => {
            pen.rect(3.0, 5.0, 18.0, 14.0, b);
            pen.poly(&[(6.0, 9.0), (9.0, 12.0), (6.0, 15.0)], false, a);
            pen.l(11.0, 15.0, 16.0, 15.0, a);
        }
    }
}

fn button_style() -> IconButtonStyle {
    let t = Tokens::get();
    let visuals = |fill| ButtonVisuals { fill, stroke: Stroke::NONE, icon: t.icon };
    IconButtonStyle {
        normal: visuals(Color32::TRANSPARENT),
        hovered: visuals(t.control_hover.gamma_multiply(0.6)),
        pressed: None,
        selected: visuals(t.tab_active),
        corner_radius: egui::CornerRadius::same(3),
        focus_stroke: Stroke::new(1.0, t.accent),
    }
}

/// An icon button with an accessibility label and a tooltip.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, tooltip: &str, selected: bool) -> egui::Response {
    let style = button_style();
    let resp = IconButton::new(tooltip, egui::vec2(size, size), &style).selected(selected).show(ui, |ui, rect, _| {
        paint(ui.painter(), rect.shrink(size * 0.12), icon, !ui.is_enabled());
    });
    crate::control::record_widget(ui, &resp, tooltip, "button", Some(selected));
    resp.on_hover_text(tooltip)
}

/// Accessibility info and focus outline for controls the app paints itself.
pub fn describe_control(ui: &egui::Ui, response: &egui::Response, label: &str, selected: Option<bool>) {
    response.widget_info(|| match selected {
        Some(selected) => egui::WidgetInfo::selected(egui::WidgetType::Button, response.enabled(), selected, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label),
    });
    if response.has_focus() && response.rect.width() > 4.0 && response.rect.height() > 4.0 {
        ui.painter().rect_stroke(response.rect.shrink(2.0), 2.0, Stroke::new(1.0, Tokens::get().accent), egui::StrokeKind::Inside);
    }
    crate::control::record_widget(ui, response, label, "button", selected);
}
