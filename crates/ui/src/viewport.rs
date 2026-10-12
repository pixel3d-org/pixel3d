//! The 3D viewport panel: camera navigation, picking and the GPU paint
//! callback.
//!
//! Rendering happens offscreen in `p3d_render::ViewportRenderer` (stored in
//! egui_wgpu's callback resources): `prepare` encodes the 3D passes, `paint`
//! composites the image into egui's pass. Geometry is rebuilt and uploaded
//! only when the document revision changes; each frame sends a camera.

use std::sync::Arc;

use egui::{Pos2, Rect, Sense, Vec2};
use glam::Vec3;
use p3d_render::viewport::PickSlot;
use p3d_render::{OrbitCamera, RenderScene, Shading, ViewFrame, ViewKind, ViewportRenderer};
use p3d_scene::{Document, ObjectKey};
use serde::{Deserialize, Serialize};

/// Mouse and trackpad navigation presets (project spec 20.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NavPreset {
    /// Alt+left drag orbits, middle drag pans, wheel zooms; trackpad: two
    /// fingers orbit, Shift + two fingers pan, pinch zooms.
    #[default]
    Default,
    /// Middle drag orbits, Shift+middle drag pans.
    Blender,
}

/// Viewport settings that persist with the UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ViewSettings {
    pub shading: String,
    pub grid: bool,
    pub ortho: bool,
    pub nav: NavPreset,
    pub show_view_cube: bool,
}

impl Default for ViewSettings {
    fn default() -> Self {
        ViewSettings { shading: Shading::Solid.name().into(), grid: true, ortho: false, nav: NavPreset::Default, show_view_cube: true }
    }
}

impl ViewSettings {
    pub fn shading(&self) -> Shading {
        Shading::parse(&self.shading).unwrap_or_default()
    }
}

/// The world-space scene built for one document state.
#[derive(Debug, Clone)]
pub struct SceneCache {
    pub key: u64,
    pub scene: Arc<RenderScene>,
}

/// Per-window viewport state (not persisted).
pub struct ViewportState {
    pub camera: OrbitCamera,
    cache: Option<SceneCache>,
    /// Selection as a bitset over the cached scene's parts, and its version.
    selection_bits: Arc<Vec<u32>>,
    selection_version: u64,
    picks: Option<PickSlot>,
    next_pick: u64,
    pending_pick: Option<(u64, bool)>,
    /// The viewport rectangle last frame (points) and its pixel size.
    pub rect: Rect,
    pub size_px: [u32; 2],
    /// Frame the scene on the next frame (after loading a file).
    pub frame_pending: bool,
    pub gpu: bool,
    pub fps: f32,
    last_frame: Option<std::time::Instant>,
}

impl Default for ViewportState {
    fn default() -> Self {
        ViewportState {
            camera: OrbitCamera::default(),
            cache: None,
            selection_bits: Arc::new(Vec::new()),
            selection_version: 0,
            picks: None,
            next_pick: 1,
            pending_pick: None,
            rect: Rect::NOTHING,
            size_px: [0, 0],
            frame_pending: true,
            gpu: false,
            fps: 0.0,
            last_frame: None,
        }
    }
}

/// What the viewport asks the app to do after a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewportEvent {
    /// A click picked an object (`None`: empty space). `add` toggles it in
    /// the selection instead of replacing it.
    Picked { object: Option<ObjectKey>, add: bool },
}

/// GPU resources stored in egui_wgpu's callback resources.
struct Gpu {
    renderer: ViewportRenderer,
    selection_version: u64,
}

/// Creates the viewport renderer on eframe's device. Call once at startup.
pub fn install(rs: &egui_wgpu::RenderState) -> PickSlot {
    let renderer = ViewportRenderer::new(&rs.device, rs.target_format, 1);
    let picks = renderer.picks();
    rs.renderer.write().callback_resources.insert(Gpu { renderer, selection_version: u64::MAX });
    picks
}

struct Callback {
    frame: ViewFrame,
    scene: Arc<RenderScene>,
    key: u64,
    selection: Arc<Vec<u32>>,
    selection_version: u64,
}

impl egui_wgpu::CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &egui_wgpu::wgpu::Device,
        queue: &egui_wgpu::wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut egui_wgpu::wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<egui_wgpu::wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else { return Vec::new() };
        gpu.renderer.upload(device, &self.scene, self.key);
        if gpu.selection_version != self.selection_version {
            gpu.renderer.set_selection(device, queue, &self.selection);
            gpu.selection_version = self.selection_version;
        }
        gpu.renderer.render(device, queue, encoder, &self.frame);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut egui_wgpu::wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(gpu) = resources.get::<Gpu>() else { return };
        let vp = info.viewport_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 {
            return;
        }
        pass.set_viewport(vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32, 0.0, 1.0);
        gpu.renderer.composite(pass);
    }
}

impl ViewportState {
    pub fn set_picks(&mut self, picks: PickSlot) {
        self.picks = Some(picks);
        self.gpu = true;
    }

    /// The scene for the current document, rebuilt when `key` changes.
    ///
    /// Complexity: O(objects + total mesh size) when rebuilt, O(1) otherwise.
    pub fn scene(&mut self, doc: &Document, key: u64) -> SceneCache {
        if let Some(c) = &self.cache
            && c.key == key
        {
            return c.clone();
        }
        let _span = tracing::info_span!("viewport scene", key).entered();
        let c = SceneCache { key, scene: Arc::new(p3d_render::prepare(doc, None)) };
        self.cache = Some(c.clone());
        // Part indices changed: the selection bitset must be rebuilt.
        self.selection_version = self.selection_version.wrapping_add(1);
        c
    }

    /// Rebuilds the selection bitset over the cached scene's parts.
    ///
    /// Complexity: O(parts + selection log selection).
    pub fn set_selection(&mut self, selected: &std::collections::BTreeSet<ObjectKey>) {
        let Some(c) = &self.cache else { return };
        let mut bits = vec![0u32; c.scene.parts.len().div_ceil(32)];
        for (i, part) in c.scene.parts.iter().enumerate() {
            if selected.contains(&part.object) {
                bits[i / 32] |= 1 << (i % 32);
            }
        }
        if *self.selection_bits != bits {
            self.selection_bits = Arc::new(bits);
            self.selection_version = self.selection_version.wrapping_add(1);
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection_bits.iter().any(|w| *w != 0)
    }

    /// Bounds of the scene (or of `only` parts) for framing.
    ///
    /// Complexity: O(parts).
    pub fn bounds(&self, only: Option<&std::collections::BTreeSet<ObjectKey>>) -> Option<(Vec3, Vec3)> {
        let c = self.cache.as_ref()?;
        let mut parts = c.scene.parts.iter().filter(|p| only.is_none_or(|s| s.contains(&p.object))).peekable();
        parts.peek()?;
        Some(parts.fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), p| (lo.min(p.lo), hi.max(p.hi))))
    }

    /// Frames the selection, or everything when nothing is selected.
    pub fn frame(&mut self, selected: &std::collections::BTreeSet<ObjectKey>) {
        let only = (!selected.is_empty()).then_some(selected);
        let b = self.bounds(only).or_else(|| self.bounds(None));
        let aspect = self.aspect();
        match b {
            Some((lo, hi)) => self.camera.frame(lo, hi, aspect),
            None => self.camera.frame(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 1.0, 1.0), aspect),
        }
    }

    fn aspect(&self) -> f32 {
        if self.rect.is_positive() { self.rect.width() / self.rect.height() } else { 16.0 / 9.0 }
    }

    pub fn set_view(&mut self, view: ViewKind) {
        self.camera.set_view(view);
    }

    /// Lays out the viewport in `ui`: handles navigation and clicks and adds
    /// the paint callback.
    pub fn show(&mut self, ui: &mut egui::Ui, doc: &Document, key: u64, settings: &ViewSettings, style: p3d_render::ViewStyle) -> Vec<ViewportEvent> {
        let rect = ui.max_rect();
        self.rect = rect;
        let resp = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
        crate::control::record_widget(ui, &resp, "Viewport", "viewport", None);
        let cache = self.scene(doc, key);
        if self.frame_pending && rect.is_positive() {
            self.frame_pending = false;
            self.frame(&Default::default());
        }
        self.camera.ortho = settings.ortho;
        self.navigate(ui, &resp, settings.nav);
        let mut events = Vec::new();
        self.poll_pick(&cache, &mut events);

        let ppp = ui.ctx().pixels_per_point();
        let left = (rect.min.x * ppp).round();
        let top = (rect.min.y * ppp).round();
        let w = ((rect.max.x * ppp).round() - left).max(1.0) as u32;
        let h = ((rect.max.y * ppp).round() - top).max(1.0) as u32;
        self.size_px = [w, h];
        let mut pick = None;
        if resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
            && self.gpu
        {
            let x = ((p.x * ppp).round() - left).clamp(0.0, (w - 1) as f32) as u32;
            let y = ((p.y * ppp).round() - top).clamp(0.0, (h - 1) as f32) as u32;
            let request = self.next_pick;
            self.next_pick += 1;
            let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            self.pending_pick = Some((request, add));
            pick = Some((x, y, request));
        }
        if self.pending_pick.is_some() {
            ui.ctx().request_repaint();
        }
        let frame = ViewFrame {
            width: w,
            height: h,
            camera: self.camera,
            shading: settings.shading(),
            grid: settings.grid,
            style,
            ids: self.has_selection(),
            pick,
        };
        if self.gpu {
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                Callback {
                    frame,
                    scene: Arc::clone(&cache.scene),
                    key: cache.key,
                    selection: Arc::clone(&self.selection_bits),
                    selection_version: self.selection_version,
                },
            ));
        } else {
            ui.painter().rect_filled(rect, 0.0, crate::theme::Tokens::get().canvas);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                crate::i18n::t("No GPU viewport"),
                crate::theme::body(),
                crate::theme::Tokens::get().text_dim,
            );
        }
        let now = std::time::Instant::now();
        if let Some(prev) = self.last_frame.replace(now) {
            let dt = now.duration_since(prev).as_secs_f32().max(1e-4);
            self.fps = if self.fps == 0.0 { 1.0 / dt } else { self.fps * 0.9 + 0.1 / dt };
        }
        events
    }

    fn poll_pick(&mut self, cache: &SceneCache, events: &mut Vec<ViewportEvent>) {
        let Some((request, add)) = self.pending_pick else { return };
        let Some(slot) = &self.picks else { return };
        let result = slot.lock().ok().and_then(|mut s| s.take());
        if let Some(r) = result
            && r.request == request
        {
            self.pending_pick = None;
            let object = r.part.and_then(|i| cache.scene.parts.get(i as usize)).map(|p| p.object);
            events.push(ViewportEvent::Picked { object, add });
        }
    }

    /// Orbit, pan and zoom from mouse, wheel and trackpad input.
    fn navigate(&mut self, ui: &egui::Ui, resp: &egui::Response, nav: NavPreset) {
        let h = self.rect.height().max(1.0);
        let orbit_speed = 0.008;
        let (modifiers, delta) = ui.input(|i| (i.modifiers, i.pointer.delta()));
        if resp.dragged() && delta != Vec2::ZERO {
            let middle = resp.dragged_by(egui::PointerButton::Middle);
            let primary = resp.dragged_by(egui::PointerButton::Primary);
            let secondary = resp.dragged_by(egui::PointerButton::Secondary);
            let (orbit, pan, zoom) = match nav {
                NavPreset::Default => (primary && modifiers.alt, middle || (secondary && modifiers.shift), secondary && modifiers.alt),
                NavPreset::Blender => (middle && !modifiers.shift && !modifiers.command, middle && modifiers.shift, middle && modifiers.command),
            };
            if orbit {
                self.camera.orbit(delta.x * orbit_speed, delta.y * orbit_speed);
            } else if pan {
                self.camera.pan(delta.x, delta.y, h);
            } else if zoom {
                self.camera.zoom((1.0 + delta.y * 0.01).clamp(0.5, 2.0), None);
            }
        }
        if !resp.hovered() {
            return;
        }
        let pointer = resp.hover_pos();
        // Wheel and trackpad scrolling: lines are mouse wheels (zoom), points
        // are trackpads (orbit, or pan with Shift).
        let (scroll_events, pinch) = ui.input(|i| {
            let wheels: Vec<(egui::MouseWheelUnit, Vec2, egui::Modifiers)> = i
                .raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, *modifiers)),
                    _ => None,
                })
                .collect();
            (wheels, i.zoom_delta())
        });
        for (unit, d, m) in scroll_events {
            match unit {
                egui::MouseWheelUnit::Point if !m.command && !m.ctrl => {
                    if m.shift {
                        self.camera.pan(-d.x, -d.y, h);
                    } else {
                        self.camera.orbit(-d.x * orbit_speed * 0.6, -d.y * orbit_speed * 0.6);
                    }
                }
                egui::MouseWheelUnit::Point => {}
                _ => {
                    let steps = d.y.clamp(-10.0, 10.0);
                    self.zoom_at(pointer, 0.88f32.powf(steps));
                }
            }
        }
        if (pinch - 1.0).abs() > 1e-4 {
            self.zoom_at(pointer, 1.0 / pinch);
        }
    }

    /// Zooms toward the point under the pointer (on the plane through the target).
    fn zoom_at(&mut self, pointer: Option<Pos2>, factor: f32) {
        let toward = pointer.map(|p| {
            let (o, d) = self.camera.ray(p.x - self.rect.min.x, p.y - self.rect.min.y, self.rect.width(), self.rect.height());
            let n = self.camera.toward();
            let denom = d.dot(n);
            if denom.abs() < 1e-6 { self.camera.target } else { o + d * ((self.camera.target - o).dot(n) / denom) }
        });
        self.camera.zoom(factor, toward);
    }
}
