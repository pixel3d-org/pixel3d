//! Design tokens and egui styling (project spec 20.3).
//!
//! The chrome tokens are cadcraft's values (MIT OR Apache-2.0) with the
//! same names; the viewport tokens are new for 3D. Panel code never uses
//! literal colors: it reads [`Tokens::get`], the palette of the theme in use
//! on the UI thread.

use std::cell::Cell;

use egui::{Color32, FontFamily, FontId, Theme, Visuals};
use serde::{Deserialize, Serialize};

/// The interface theme the user chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    /// Follow the operating system's light or dark appearance, live.
    System,
    Light,
    #[default]
    Dark,
}

/// The theme `System` shows when the OS reports none.
pub const SYSTEM_FALLBACK: Theme = Theme::Dark;

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            ThemePref::System => "system",
            ThemePref::Light => "light",
            ThemePref::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<ThemePref> {
        ThemePref::ALL.into_iter().find(|t| t.as_str().eq_ignore_ascii_case(s.trim()))
    }

    /// The theme to show, given what the OS reports.
    pub fn resolve(self, system: Option<Theme>) -> Theme {
        match self {
            ThemePref::System => system.unwrap_or(SYSTEM_FALLBACK),
            ThemePref::Light => Theme::Light,
            ThemePref::Dark => Theme::Dark,
        }
    }
}

thread_local! {
    static ACTIVE: Cell<Theme> = const { Cell::new(Theme::Dark) };
}

/// The theme [`Tokens::get`] returns colors for.
pub fn active() -> Theme {
    ACTIVE.with(Cell::get)
}

pub fn set_active(theme: Theme) {
    ACTIVE.with(|a| a.set(theme));
}

/// Every color the interface uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub chrome: Color32,
    pub chrome_dark: Color32,
    pub panel: Color32,
    pub tab_active: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub border: Color32,
    pub canvas: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub icon: Color32,
    pub icon_accent: Color32,
    pub icon_point: Color32,
    pub accent: Color32,
    /// Selected objects.
    pub selection: Color32,
    /// Hovered or active object.
    pub hover: Color32,
    pub grip: Color32,
    pub grip_hot: Color32,
    pub snap: Color32,
    pub window_fill: Color32,
    pub crossing_fill: Color32,
    pub window_stroke: Color32,
    pub crossing_stroke: Color32,
    pub cmd_bg: Color32,
    pub toggle_on: Color32,
    /// Text fields and other inset backgrounds.
    pub field: Color32,
    /// Selected text and selected list rows.
    pub select_bg: Color32,
    pub cmd_history: Color32,
    pub cmd_border: Color32,
    pub cmd_keyword: Color32,
    pub cmd_keyword_hover: Color32,
    pub list_bg: Color32,
    pub list_row: Color32,
    /// Transient warnings (status bar messages).
    pub warn: Color32,
    // ---- 3D viewport ----
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    /// Selected vertices, edges and faces (faces at 25% alpha).
    pub element_selected: Color32,
    pub seam: Color32,
    pub sharp: Color32,
    pub crease: Color32,
    pub health_ok: Color32,
    pub health_warn: Color32,
    pub health_error: Color32,
    /// Lit surface color of meshes in the viewport.
    pub surface: Color32,
    /// Polygon edges in wire modes.
    pub wire: Color32,
    /// The accent mixed into the background: selected buttons and tabs.
    pub accent_soft: Color32,
    /// Text and icons drawn on an accent fill.
    pub on_accent: Color32,
    /// Property cards (collapsible sections): fill, hovered header, edge.
    pub card: Color32,
    pub card_hover: Color32,
    pub card_border: Color32,
    /// Menus and tooltips, and the key caps that show shortcuts in them.
    pub popup: Color32,
    pub popup_border: Color32,
    pub keycap: Color32,
}

/// `0xrrggbb` as a color.
const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Tokens {
    /// Neutral 900 to 950 surfaces with a yellow 400 accent (ADR 0006).
    pub const DARK: Tokens = Tokens {
        chrome: hex(0x171717),
        chrome_dark: hex(0x0f0f0f),
        panel: hex(0x141414),
        tab_active: hex(0x2a2a2a),
        control: hex(0x262626),
        control_hover: hex(0x333333),
        border: hex(0x0a0a0a),
        canvas: hex(0x0a0a0a),
        text: hex(0xe5e5e5),
        text_dim: hex(0xa3a3a3),
        text_faint: hex(0x737373),
        icon: hex(0xd4d4d4),
        icon_accent: hex(0xfacc15),
        icon_point: hex(0xfb923c),
        accent: hex(0xfacc15),
        selection: hex(0xfacc15),
        hover: hex(0xfde047),
        grip: hex(0xfacc15),
        grip_hot: hex(0xf87171),
        snap: hex(0x4ade80),
        window_fill: Color32::from_rgba_premultiplied(0x3b, 0x30, 0x05, 0x50),
        crossing_fill: Color32::from_rgba_premultiplied(0x12, 0x3b, 0x1f, 0x50),
        window_stroke: hex(0xfde047),
        crossing_stroke: hex(0x86efac),
        cmd_bg: Color32::from_rgba_premultiplied(0x17, 0x17, 0x17, 0xf0),
        toggle_on: hex(0xfacc15),
        field: hex(0x111111),
        select_bg: hex(0x5c4a0b),
        cmd_history: Color32::from_rgba_unmultiplied_const(0x0f, 0x0f, 0x0f, 190),
        cmd_border: hex(0x333333),
        cmd_keyword: hex(0xfde047),
        cmd_keyword_hover: hex(0x3b3109),
        list_bg: hex(0x141414),
        list_row: hex(0x262626),
        warn: hex(0xfbbf24),
        viewport_top: hex(0x1f1f1f),
        viewport_bottom: hex(0x0f0f0f),
        grid_minor: hex(0x262626),
        grid_major: hex(0x363636),
        axis_x: hex(0xef4444),
        axis_y: hex(0x22c55e),
        axis_z: hex(0x3b82f6),
        element_selected: hex(0xfb923c),
        seam: hex(0xf87171),
        sharp: hex(0x38bdf8),
        crease: hex(0xc084fc),
        health_ok: hex(0x4ade80),
        health_warn: hex(0xfbbf24),
        health_error: hex(0xf87171),
        surface: hex(0xb3b3b3),
        wire: hex(0x0a0a0a),
        accent_soft: hex(0x3a3110),
        on_accent: hex(0x171717),
        card: hex(0x1b1b1b),
        card_hover: hex(0x242424),
        card_border: hex(0x262626),
        popup: hex(0x1f1f1f),
        popup_border: hex(0x333333),
        keycap: hex(0x2e2e2e),
    };

    /// Neutral 50 to 200 surfaces with dark text. Fills use the same yellow
    /// 400 accent; yellow lines and text on white use `icon_accent`.
    pub const LIGHT: Tokens = Tokens {
        chrome: hex(0xf5f5f5),
        chrome_dark: hex(0xe5e5e5),
        panel: hex(0xfafafa),
        tab_active: hex(0xffffff),
        control: hex(0xe5e5e5),
        control_hover: hex(0xd4d4d4),
        border: hex(0xd4d4d4),
        canvas: hex(0xd4d4d4),
        text: hex(0x171717),
        text_dim: hex(0x525252),
        text_faint: hex(0x737373),
        icon: hex(0x404040),
        icon_accent: hex(0xca8a04),
        icon_point: hex(0xea580c),
        accent: hex(0xfacc15),
        selection: hex(0xeab308),
        hover: hex(0xca8a04),
        cmd_bg: Color32::from_rgba_unmultiplied_const(0xfa, 0xfa, 0xfa, 0xf0),
        toggle_on: hex(0xeab308),
        field: hex(0xffffff),
        select_bg: hex(0xfde68a),
        cmd_history: Color32::from_rgba_unmultiplied_const(0xf5, 0xf5, 0xf5, 0xdc),
        cmd_border: hex(0xd4d4d4),
        cmd_keyword: hex(0xa16207),
        cmd_keyword_hover: hex(0xfef3c7),
        list_bg: hex(0xffffff),
        list_row: hex(0xf5f5f5),
        warn: hex(0x92400e),
        viewport_top: hex(0xe5e5e5),
        viewport_bottom: hex(0xcfcfcf),
        grid_minor: hex(0xc7c7c7),
        grid_major: hex(0xb0b0b0),
        surface: hex(0xa3a3a3),
        wire: hex(0x262626),
        accent_soft: hex(0xfef3c7),
        on_accent: hex(0x171717),
        card: hex(0xffffff),
        card_hover: hex(0xf5f5f5),
        card_border: hex(0xe5e5e5),
        popup: hex(0xffffff),
        popup_border: hex(0xd4d4d4),
        keycap: hex(0xf0f0f0),
        ..Tokens::DARK
    };

    pub fn of(theme: Theme) -> Tokens {
        match theme {
            Theme::Dark => Tokens::DARK,
            Theme::Light => Tokens::LIGHT,
        }
    }

    /// The palette of the theme in use.
    pub fn get() -> Tokens {
        Tokens::of(active())
    }

    /// The viewport colors as the renderer takes them (linear RGBA).
    pub fn view_style(&self) -> p3d_render::ViewStyle {
        let l = |c: Color32, a: f32| {
            let [r, g, b, _] = p3d_render::viewport::linear([c.r(), c.g(), c.b(), 255]);
            [r, g, b, a]
        };
        p3d_render::ViewStyle {
            bg_top: l(self.viewport_top, 1.0),
            bg_bottom: l(self.viewport_bottom, 1.0),
            grid_minor: l(self.grid_minor, 0.75),
            grid_major: l(self.grid_major, 1.0),
            axis_x: l(self.axis_x, 1.0),
            axis_z: l(self.axis_z, 1.0),
            surface: l(self.surface, 1.0),
            selection: l(self.selection, 0.18),
            wire: l(self.wire, 1.0),
            element: l(self.element_selected, 1.0),
            edit_wire: l(self.wire, 1.0),
            // Points; the viewport scales it to pixels.
            vertex_px: 5.0,
        }
    }
}

pub fn small() -> FontId {
    FontId::new(11.5, FontFamily::Proportional)
}

pub fn body() -> FontId {
    FontId::new(12.5, FontFamily::Proportional)
}

pub fn mono() -> FontId {
    FontId::new(12.0, FontFamily::Monospace)
}

pub fn heading() -> FontId {
    FontId::new(15.0, FontFamily::Proportional)
}

fn visuals(theme: Theme) -> Visuals {
    let t = Tokens::of(theme);
    let mut v = match theme {
        Theme::Dark => Visuals::dark(),
        Theme::Light => Visuals::light(),
    };
    let radius = egui::CornerRadius::same(6);
    let line = |c: Color32| egui::Stroke::new(1.0, c);
    v.panel_fill = t.panel;
    v.window_fill = t.popup;
    v.extreme_bg_color = t.field;
    v.text_edit_bg_color = Some(t.field);
    v.faint_bg_color = t.chrome_dark;
    v.override_text_color = Some(t.text);
    v.hyperlink_color = t.accent;
    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.noninteractive.weak_bg_fill = t.panel;
    v.widgets.noninteractive.fg_stroke.color = t.text;
    v.widgets.noninteractive.bg_stroke = line(t.control);
    v.widgets.noninteractive.corner_radius = radius;
    v.widgets.inactive.bg_fill = t.control;
    v.widgets.inactive.weak_bg_fill = t.control;
    v.widgets.inactive.fg_stroke.color = t.text;
    v.widgets.inactive.bg_stroke = line(t.card_border);
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.bg_fill = t.control_hover;
    v.widgets.hovered.weak_bg_fill = t.control_hover;
    v.widgets.hovered.fg_stroke.color = t.text;
    v.widgets.hovered.bg_stroke = line(t.text_faint);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.bg_fill = t.accent;
    v.widgets.active.weak_bg_fill = t.accent;
    v.widgets.active.fg_stroke.color = t.on_accent;
    v.widgets.active.bg_stroke = line(t.accent);
    v.widgets.active.corner_radius = radius;
    v.widgets.open.bg_fill = t.control_hover;
    v.widgets.open.weak_bg_fill = t.control_hover;
    v.widgets.open.corner_radius = radius;
    v.selection.bg_fill = t.select_bg;
    v.selection.stroke = line(t.accent);
    v.window_stroke = line(t.popup_border);
    let shadow = Color32::from_black_alpha(if theme == Theme::Dark { 120 } else { 40 });
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: shadow };
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(8);
    v.window_shadow = v.popup_shadow;
    v
}

/// Installs the styles of both themes and shows `theme`. `follow_system`
/// keeps egui on the System preference so later OS changes still arrive.
pub fn apply(ctx: &egui::Context, theme: Theme, follow_system: bool) {
    set_active(theme);
    ctx.set_visuals_of(Theme::Dark, visuals(Theme::Dark));
    ctx.set_visuals_of(Theme::Light, visuals(Theme::Light));
    ctx.set_theme(if follow_system { egui::ThemePreference::System } else { egui::ThemePreference::from(theme) });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.tooltip_width = 280.0;
        // Short eased transitions everywhere (hover fades, sections opening).
        s.animation_time = 0.16;
        s.interaction.tooltip_delay = 0.35;
        s.interaction.tooltip_grace_time = 0.4;
        s.text_styles.insert(egui::TextStyle::Body, body());
        s.text_styles.insert(egui::TextStyle::Button, body());
        s.text_styles.insert(egui::TextStyle::Small, small());
        s.text_styles.insert(egui::TextStyle::Monospace, mono());
        s.text_styles.insert(egui::TextStyle::Heading, heading());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_parse_and_resolve() {
        assert_eq!(ThemePref::parse(" Light "), Some(ThemePref::Light));
        assert_eq!(ThemePref::parse("sepia"), None);
        assert_eq!(ThemePref::System.resolve(None), SYSTEM_FALLBACK);
        assert_eq!(ThemePref::System.resolve(Some(Theme::Light)), Theme::Light);
        assert_eq!(ThemePref::Dark.resolve(Some(Theme::Light)), Theme::Dark);
    }

    #[test]
    fn palette_follows_adr_0006() {
        let d = Tokens::DARK;
        assert_eq!(d.chrome, hex(0x171717), "neutral 900");
        assert_eq!(d.border, hex(0x0a0a0a), "neutral 950");
        assert_eq!(d.accent, hex(0xfacc15), "yellow 400");
        assert_eq!(Tokens::LIGHT.accent, hex(0xfacc15), "the same primary in both themes");
        assert_eq!(Tokens::LIGHT.panel, hex(0xfafafa), "neutral 50");
        let s = d.view_style();
        assert!(s.bg_top[0] > s.bg_bottom[0], "the gradient is lighter at the top");
    }

    /// Relative luminance contrast between two colors (WCAG).
    fn contrast(a: Color32, b: Color32) -> f32 {
        let l = |c: Color32| {
            let [r, g, b, _] = p3d_render::viewport::linear([c.r(), c.g(), c.b(), 255]);
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (x, y) = (l(a), l(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_stays_readable() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            assert!(contrast(t.text, t.panel) >= 7.0, "body text: {}", contrast(t.text, t.panel));
            assert!(contrast(t.text_dim, t.panel) >= 4.5, "secondary text: {}", contrast(t.text_dim, t.panel));
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "text on the accent");
        }
    }
}
