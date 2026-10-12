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
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        chrome: Color32::from_rgb(0x3b, 0x44, 0x53),
        chrome_dark: Color32::from_rgb(0x31, 0x38, 0x44),
        panel: Color32::from_rgb(0x3c, 0x44, 0x52),
        tab_active: Color32::from_rgb(0x4e, 0x5a, 0x6e),
        control: Color32::from_rgb(0x50, 0x5a, 0x6d),
        control_hover: Color32::from_rgb(0x5c, 0x67, 0x7c),
        border: Color32::from_rgb(0x23, 0x29, 0x30),
        canvas: Color32::from_rgb(0x22, 0x28, 0x2f),
        text: Color32::from_rgb(0xd7, 0xdb, 0xe0),
        text_dim: Color32::from_rgb(0xa3, 0xaa, 0xb4),
        text_faint: Color32::from_rgb(0x87, 0x8d, 0x96),
        icon: Color32::from_rgb(0xcf, 0xd4, 0xdb),
        icon_accent: Color32::from_rgb(0x4f, 0xa3, 0xf7),
        icon_point: Color32::from_rgb(0xf0, 0x7a, 0x3a),
        accent: Color32::from_rgb(0x3d, 0x8b, 0xfd),
        selection: Color32::from_rgb(0x4a, 0x8f, 0xff),
        hover: Color32::from_rgb(0x9c, 0xc6, 0xff),
        grip: Color32::from_rgb(0x2f, 0x6b, 0xff),
        grip_hot: Color32::from_rgb(0xff, 0x3b, 0x3b),
        snap: Color32::from_rgb(0x2f, 0xd0, 0x62),
        window_fill: Color32::from_rgba_premultiplied(0x18, 0x30, 0x60, 0x50),
        crossing_fill: Color32::from_rgba_premultiplied(0x18, 0x48, 0x18, 0x50),
        window_stroke: Color32::from_rgb(0x6f, 0xa8, 0xff),
        crossing_stroke: Color32::from_rgb(0x7f, 0xe0, 0x7f),
        cmd_bg: Color32::from_rgba_premultiplied(0x34, 0x3c, 0x4a, 0xf0),
        toggle_on: Color32::from_rgb(0x3d, 0x8b, 0xfd),
        field: Color32::from_rgb(0x31, 0x38, 0x44),
        select_bg: Color32::from_rgb(0x3d, 0x8b, 0xfd),
        cmd_history: Color32::from_rgba_unmultiplied_const(0x2a, 0x30, 0x3a, 170),
        cmd_border: Color32::from_rgb(0x55, 0x5f, 0x70),
        cmd_keyword: Color32::from_rgb(0x8f, 0xc1, 0xff),
        cmd_keyword_hover: Color32::from_rgb(0x2f, 0x5e, 0xa8),
        list_bg: Color32::from_rgb(0x2b, 0x31, 0x3b),
        list_row: Color32::from_rgb(0x3a, 0x42, 0x50),
        warn: Color32::from_rgb(0xff, 0xd0, 0x80),
        viewport_top: Color32::from_rgb(0x2a, 0x30, 0x38),
        viewport_bottom: Color32::from_rgb(0x1d, 0x22, 0x28),
        grid_minor: Color32::from_rgb(0x2e, 0x35, 0x3f),
        grid_major: Color32::from_rgb(0x3a, 0x43, 0x50),
        axis_x: Color32::from_rgb(0xe5, 0x48, 0x4d),
        axis_y: Color32::from_rgb(0x46, 0xa7, 0x58),
        axis_z: Color32::from_rgb(0x3e, 0x63, 0xdd),
        element_selected: Color32::from_rgb(0xff, 0xb2, 0x24),
        seam: Color32::from_rgb(0xff, 0x3b, 0x3b),
        sharp: Color32::from_rgb(0x4f, 0xa3, 0xf7),
        crease: Color32::from_rgb(0xb7, 0x8c, 0xff),
        health_ok: Color32::from_rgb(0x2f, 0xd0, 0x62),
        health_warn: Color32::from_rgb(0xff, 0xb2, 0x24),
        health_error: Color32::from_rgb(0xff, 0x3b, 0x3b),
        surface: Color32::from_rgb(0xa8, 0xae, 0xb6),
        wire: Color32::from_rgb(0x10, 0x12, 0x16),
    };

    /// Light gray chrome with dark text; the selection, grip and health
    /// colors are the dark theme's.
    pub const LIGHT: Tokens = Tokens {
        chrome: Color32::from_rgb(0xe4, 0xe7, 0xeb),
        chrome_dark: Color32::from_rgb(0xd5, 0xd9, 0xdf),
        panel: Color32::from_rgb(0xee, 0xf0, 0xf3),
        tab_active: Color32::from_rgb(0xc8, 0xd3, 0xe2),
        control: Color32::from_rgb(0xdc, 0xe0, 0xe6),
        control_hover: Color32::from_rgb(0xcb, 0xd2, 0xdc),
        border: Color32::from_rgb(0xb3, 0xba, 0xc4),
        text: Color32::from_rgb(0x1d, 0x23, 0x2b),
        text_dim: Color32::from_rgb(0x47, 0x50, 0x5c),
        text_faint: Color32::from_rgb(0x62, 0x6a, 0x76),
        icon: Color32::from_rgb(0x34, 0x3c, 0x48),
        icon_accent: Color32::from_rgb(0x1b, 0x6f, 0xd6),
        icon_point: Color32::from_rgb(0xd3, 0x5a, 0x1c),
        accent: Color32::from_rgb(0x2f, 0x7c, 0xf6),
        cmd_bg: Color32::from_rgba_unmultiplied_const(0xf9, 0xfa, 0xfb, 0xf0),
        toggle_on: Color32::from_rgb(0x2f, 0x7c, 0xf6),
        field: Color32::from_rgb(0xff, 0xff, 0xff),
        select_bg: Color32::from_rgb(0xb6, 0xd3, 0xfb),
        cmd_history: Color32::from_rgba_unmultiplied_const(0xf0, 0xf2, 0xf5, 0xdc),
        cmd_border: Color32::from_rgb(0xa9, 0xb0, 0xba),
        cmd_keyword: Color32::from_rgb(0x15, 0x59, 0xc0),
        cmd_keyword_hover: Color32::from_rgb(0xcf, 0xe0, 0xfa),
        list_bg: Color32::from_rgb(0xfb, 0xfc, 0xfd),
        list_row: Color32::from_rgb(0xe3, 0xe8, 0xef),
        warn: Color32::from_rgb(0x8a, 0x53, 0x00),
        viewport_top: Color32::from_rgb(0xc9, 0xce, 0xd6),
        viewport_bottom: Color32::from_rgb(0xb5, 0xbb, 0xc5),
        grid_minor: Color32::from_rgb(0xaa, 0xb1, 0xbb),
        grid_major: Color32::from_rgb(0x94, 0x9c, 0xa8),
        surface: Color32::from_rgb(0xb4, 0xb9, 0xc0),
        wire: Color32::from_rgb(0x20, 0x24, 0x2a),
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
    v.panel_fill = t.panel;
    v.window_fill = t.chrome;
    v.extreme_bg_color = t.field;
    v.faint_bg_color = t.chrome_dark;
    v.override_text_color = Some(t.text);
    v.widgets.noninteractive.bg_fill = t.panel;
    v.widgets.noninteractive.fg_stroke.color = t.text;
    v.widgets.noninteractive.bg_stroke.color = t.border;
    v.widgets.inactive.bg_fill = t.control;
    v.widgets.inactive.weak_bg_fill = t.control;
    v.widgets.inactive.fg_stroke.color = t.text;
    v.widgets.hovered.bg_fill = t.control_hover;
    v.widgets.hovered.weak_bg_fill = t.control_hover;
    v.widgets.active.bg_fill = t.accent;
    v.widgets.active.weak_bg_fill = t.accent;
    v.selection.bg_fill = t.select_bg;
    v.window_stroke = egui::Stroke::new(1.0, t.border);
    let shadow = Color32::from_black_alpha(if theme == Theme::Dark { 90 } else { 40 });
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: shadow };
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(5);
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
        s.spacing.interact_size.y = 20.0;
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
    fn spec_colors_hold() {
        let d = Tokens::DARK;
        assert_eq!(d.chrome, Color32::from_rgb(0x3b, 0x44, 0x53));
        assert_eq!(d.viewport_top, Color32::from_rgb(0x2a, 0x30, 0x38));
        assert_eq!(Tokens::LIGHT.accent, Color32::from_rgb(0x2f, 0x7c, 0xf6));
        assert_eq!(Tokens::LIGHT.selection, d.selection, "selection is the same in both themes");
        let s = d.view_style();
        assert!(s.bg_top[0] > s.bg_bottom[0], "the gradient is lighter at the top");
    }
}
