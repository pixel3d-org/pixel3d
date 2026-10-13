//! The icon set: line icons from Tabler Icons (see `icon_data`), stroked
//! thin and crisp at any size, plus the icon button every toolbar uses.
//! The selection mode icons mark their element (vertex, edge, face) on a
//! cube in the accent color, as Blender's do.

use std::sync::OnceLock;

use egui::{Color32, Painter, Rect, Sense, Stroke, vec2};

use crate::icon_data as d;
use crate::theme::Tokens;
use crate::vector_icon::{self, Outline};
use crate::widgets::{self, Tip};

macro_rules! icons {
    ($($name:ident => $paths:expr, $fill:expr;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($name,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$name,)*];

            fn source(self) -> (&'static [&'static str], bool) {
                match self {
                    $(Icon::$name => ($paths, $fill),)*
                }
            }
        }
    };
}

icons! {
    New => d::FILE_PLUS, false;
    Open => d::FOLDER_OPEN, false;
    Save => d::DEVICE_FLOPPY, false;
    Undo => d::ARROW_BACK_UP, false;
    Redo => d::ARROW_FORWARD_UP, false;
    Select => d::POINTER, false;
    Move => d::ARROWS_MOVE, false;
    Rotate => d::ROTATE_3D, false;
    Scale => d::RESIZE, false;
    ModeObject => d::CUBE, false;
    ModeVert => d::CUBE, false;
    ModeEdge => d::CUBE, false;
    ModeFace => d::CUBE, false;
    Box => d::CUBE, false;
    Cylinder => d::CYLINDER, false;
    Sphere => d::SPHERE, false;
    Plane => d::PLANE, false;
    Frame => d::FOCUS_CENTERED, false;
    ShadeSolid => d::FILLED_CIRCLE, true;
    ShadeWire => d::WORLD, false;
    ShadeSolidWire => d::SPHERE, false;
    ShadeMatcap => d::INNER_SHADOW_BOTTOM_RIGHT, false;
    XRay => d::LAYERS_INTERSECT, false;
    Grid => d::GRID_4X4, false;
    Perspective => d::PERSPECTIVE, false;
    Orthographic => d::BOX_MODEL_2, false;
    Search => d::SEARCH, false;
    Gear => d::SETTINGS, false;
    Help => d::HELP, false;
    Plus => d::PLUS, false;
    Close => d::X, false;
    ChevronDown => d::CHEVRON_DOWN, false;
    ChevronRight => d::CHEVRON_RIGHT, false;
    Menu => d::MENU_2, false;
    Eye => d::EYE, false;
    EyeOff => d::EYE_OFF, false;
    Lock => d::LOCK, false;
    Unlock => d::LOCK_OPEN, false;
    Mesh => d::VECTOR_TRIANGLE, false;
    Empty => d::ASTERISK, false;
    Health => d::ACTIVITY_HEARTBEAT, false;
    History => d::HISTORY, false;
    Outliner => d::LIST_TREE, false;
    Properties => d::ADJUSTMENTS_HORIZONTAL, false;
    Command => d::TERMINAL_2, false;
    Tool => d::TOOL, false;
    Scene => d::STACK_2, false;
    Copy => d::COPY, false;
    Trash => d::TRASH, false;
    Apply => d::CHECKS, false;
    Fill => d::BUCKET_DROPLET, false;
    Normals => d::ARROWS_MAXIMIZE, false;
    Info => d::INFO_CIRCLE, false;
    Warning => d::ALERT_TRIANGLE, false;
    Keyboard => d::KEYBOARD, false;
    Collection => d::FOLDER, false;
    LodGroup => d::STACK_3, false;
    MeshData => d::TRIANGLE, false;
    SceneRoot => d::BOX_MULTIPLE, false;
    Extrude => d::ARROW_BAR_UP, false;
    Inset => d::BORDER_INNER, false;
    Bevel => d::BORDER_CORNERS, false;
    LoopCut => d::LAYOUT_COLUMNS, false;
    Knife => d::KNIFE, false;
    Smooth => d::WAVE_SINE, false;
    EdgeSlide => d::ARROWS_HORIZONTAL, false;
    ShrinkFatten => d::ARROW_AUTOFIT_HEIGHT, false;
    Rip => d::SCISSORS, false;
    Poke => d::SQUARE_DOT, false;
    Triangulate => d::TRIANGLES, false;
    Merge => d::ARROWS_JOIN, false;
    Dissolve => d::ERASER, false;
    Subdivide => d::GRID_3X3, false;
    Bridge => d::BUILDING_BRIDGE, false;
    Split => d::ARROWS_SPLIT, false;
    Symmetrize => d::FLIP_VERTICAL, false;
    Orientation => d::COMPASS, false;
    Pivot => d::FOCUS_2, false;
    Snap => d::MAGNET, false;
    Proportional => d::CIRCLE_DASHED, false;
    Cursor => d::CROSSHAIR, false;
    ChevronUp => d::CHEVRON_UP, false;
    Check => d::CHECK, false;
    Cone => d::CONE, false;
    Capsule => d::CAPSULE, false;
    IcoSphere => d::HEXAGON, false;
    Torus => d::TORUS, false;
    Wedge => d::WEDGE, false;
    BoxSelect => d::MARQUEE_2, false;
    Tube => d::CIRCLE_DOT, false;
    Curve => d::VECTOR_SPLINE, false;
    Sweep => d::SWEEP, false;
    Pipe => d::PIPE, false;
    Loft => d::LOFT, false;
    Array => d::ARRAY, false;
    Scatter => d::SCATTER, false;
    Import => d::IMPORT, false;
    BoolUnion => d::BOOL_UNION, false;
    BoolDifference => d::BOOL_DIFFERENCE, false;
    BoolIntersect => d::BOOL_INTERSECT, false;
    CleanUp => d::SPARKLES, false;
    Report => d::REPORT, false;
    Shapes => d::BLOBS, false;
    Poles => d::POLES, false;
    Spec => d::SPEC, false;
    Landmark => d::MAP_PIN, false;
    Outline => d::BEZIER, false;
    Leaf => d::LEAF, false;
    Blueprint => d::BLUEPRINT, false;
    Handles => d::HANDLES, false;
}

/// The flattened outlines of an icon, parsed on first use.
///
/// Complexity: O(1) after the first call, which parses every icon once.
pub fn outlines(icon: Icon) -> &'static [Outline] {
    static CACHE: OnceLock<Vec<Vec<Outline>>> = OnceLock::new();
    let all = CACHE.get_or_init(|| Icon::ALL.iter().map(|i| vector_icon::parse_all(i.source().0)).collect());
    // Variants are declared in `ALL` order, so the discriminant is the index.
    all.get(icon as usize).map_or(&[], Vec::as_slice)
}

/// Draws `icon` in the theme's icon color, faint when `dim`.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, dim: bool) {
    let tk = Tokens::get();
    let color = if dim { tk.text_faint } else { tk.icon };
    paint_colored(p, rect, icon, color, tk.accent);
}

/// Draws `icon` in `color`; the mode icons mark their element in `mark`.
pub fn paint_colored(p: &Painter, rect: Rect, icon: Icon, color: Color32, mark: Color32) {
    let fill = icon.source().1;
    let marked = matches!(icon, Icon::ModeVert | Icon::ModeEdge | Icon::ModeFace);
    // The cube recedes so the marked element reads first.
    let base = if marked { color.gamma_multiply(0.55) } else { color };
    vector_icon::paint(p, rect, outlines(icon), base, fill);
    if !marked {
        return;
    }
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0, 12.0) * s;
    let at = |x: f32, y: f32| o + vec2(x, y) * s;
    let w = vector_icon::stroke_width(rect.width()) * 1.6;
    match icon {
        Icon::ModeVert => {
            p.circle_filled(at(12.0, 12.0), 3.4 * s, mark);
        }
        Icon::ModeEdge => {
            p.line_segment([at(12.0, 12.0), at(12.0, 21.5)], Stroke::new(w, mark));
            p.circle_filled(at(12.0, 12.0), w / 2.0, mark);
            p.circle_filled(at(12.0, 21.5), w / 2.0, mark);
        }
        _ => {
            let face = vec![at(3.6, 7.6), at(11.6, 12.2), at(11.6, 21.0), at(3.6, 16.4)];
            p.add(egui::Shape::convex_polygon(face, mark.gamma_multiply(0.85), Stroke::NONE));
        }
    }
}

/// How an icon button looks when selected: a solid accent pill (tools) or
/// a raised neutral segment with an accent icon (toggles in a group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Solid,
    Segment,
}

/// An icon button with eased hover and selection, and a designed tooltip
/// on the `side` of the button.
pub fn button_with(
    ui: &mut egui::Ui,
    icon: Icon,
    size: egui::Vec2,
    tip: Tip<'_>,
    selected: bool,
    look: Look,
    side: egui::RectAlign,
) -> egui::Response {
    let tk = Tokens::get();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    let hot = ui.ctx().animate_bool_with_time(resp.id.with("hot"), enabled && resp.hovered(), 0.12);
    let on = ui.ctx().animate_bool_with_time(resp.id.with("on"), selected, 0.14);
    let radius = (size.y * 0.24).round();
    let (on_fill, on_icon) = match look {
        Look::Solid => (tk.accent, tk.on_accent),
        Look::Segment => (tk.control, tk.icon_accent),
    };
    let mut fill = widgets::mix(Color32::TRANSPARENT, tk.control, hot);
    fill = widgets::mix(fill, on_fill, on);
    if resp.is_pointer_button_down_on() {
        fill = widgets::mix(fill, tk.control_hover, 0.6);
    }
    let p = ui.painter();
    if fill.a() > 0 {
        p.rect_filled(rect, radius, fill);
    }
    let icon_size = (size.y * 0.56).round().clamp(12.0, 20.0);
    let ir = Rect::from_center_size(rect.center(), vec2(icon_size, icon_size));
    let base = if enabled { widgets::mix(tk.icon, tk.text, hot) } else { tk.text_faint.gamma_multiply(0.7) };
    let mark = if look == Look::Solid { widgets::mix(tk.accent, tk.on_accent, on) } else { tk.icon_accent };
    paint_colored(p, ir, icon, widgets::mix(base, on_icon, on), mark);
    describe_control(ui, &resp, tip.title, Some(selected));
    widgets::tooltip(&resp, tip, side);
    resp
}

/// A square icon button with a plain tooltip (toolbars, headers).
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, tooltip: &str, selected: bool) -> egui::Response {
    button_with(ui, icon, vec2(size, size), Tip::new(tooltip), selected, Look::Segment, egui::RectAlign::BOTTOM)
}

/// Accessibility info and focus outline for controls the app paints itself.
pub fn describe_control(ui: &egui::Ui, response: &egui::Response, label: &str, selected: Option<bool>) {
    response.widget_info(|| match selected {
        Some(selected) => egui::WidgetInfo::selected(egui::WidgetType::Button, response.enabled(), selected, label),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label),
    });
    if response.has_focus() && response.rect.width() > 4.0 && response.rect.height() > 4.0 {
        ui.painter().rect_stroke(response.rect.shrink(1.0), 4.0, Stroke::new(1.5, Tokens::get().accent), egui::StrokeKind::Inside);
    }
    crate::control::record_widget(ui, response, label, "button", selected);
}
