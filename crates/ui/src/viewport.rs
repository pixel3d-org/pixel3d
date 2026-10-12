//! The 3D viewport panel: camera navigation, picking and the GPU paint
//! callback.
//!
//! Rendering happens offscreen in `p3d_render::ViewportRenderer` (stored in
//! egui_wgpu's callback resources): `prepare` encodes the 3D passes, `paint`
//! composites the image into egui's pass. Geometry is rebuilt and uploaded
//! only when a mesh changes; transforms when the document changes (or every
//! frame while a drag previews new ones); each frame sends a camera.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Pos2, Rect, Sense, Vec2};
use glam::{Mat4, Vec3};
use p3d_render::viewport::PickSlot;
use p3d_render::{OrbitCamera, Shading, ViewFrame, ViewKind, ViewScene, ViewportRenderer};
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
    /// The navigation gizmo in the corner.
    pub show_nav_gizmo: bool,
    /// Snap transform drags to the grid and to 15 degree steps.
    pub snap: bool,
    /// Scene statistics in the top left corner (View > Statistics).
    pub stats: bool,
    /// X-ray: see-through surfaces (Alt+Z), with this opacity.
    pub xray: bool,
    pub xray_alpha: f32,
}

impl Default for ViewSettings {
    fn default() -> Self {
        ViewSettings {
            shading: Shading::Solid.name().into(),
            grid: true,
            ortho: false,
            nav: NavPreset::Default,
            show_nav_gizmo: true,
            snap: false,
            stats: true,
            xray: false,
            xray_alpha: 0.5,
        }
    }
}

impl ViewSettings {
    pub fn shading(&self) -> Shading {
        Shading::parse(&self.shading).unwrap_or_default()
    }
}

/// The viewport scene for one document state.
#[derive(Debug, Clone)]
pub struct SceneCache {
    /// The document state the matrices belong to.
    pub key: u64,
    pub view: Arc<ViewScene>,
    /// Bumped whenever the geometry is rebuilt (the upload key).
    pub geometry: u64,
    /// World matrix of every part.
    pub models: Arc<Vec<Mat4>>,
    /// The hidden-set version the parts were chosen with.
    pub hidden: u64,
}

/// Per-window viewport state (not persisted).
pub struct ViewportState {
    pub camera: OrbitCamera,
    cache: Option<SceneCache>,
    geometry_counter: u64,
    /// Bumped when the set of objects hidden in the viewport changes.
    hidden_version: u64,
    /// World matrices that override the document while a drag previews
    /// them (cleared when the drag ends).
    pub preview: BTreeMap<ObjectKey, Mat4>,
    models_version: u64,
    /// Selection as a bitset over the cached scene's parts, and its version.
    selection_bits: Arc<Vec<u32>>,
    selection_version: u64,
    picks: Option<PickSlot>,
    next_pick: u64,
    pending_pick: Option<(u64, bool)>,
    /// The viewport rectangle last frame (points) and its pixel size.
    pub rect: Rect,
    pub size_px: [u32; 2],
    /// The element overlay to draw (set by the app in element modes).
    pub edit_overlay: Option<(Arc<p3d_render::EditOverlay>, u64)>,
    /// Clicks pick vertices, edges or faces instead of objects.
    pub pick_elements: bool,
    /// Frame the scene on the next frame (after loading a file).
    pub frame_pending: bool,
    pub gpu: bool,
    pub fps: f32,
    last_frame: Option<std::time::Instant>,
    /// A view change in progress: start angles, end angles, start time.
    anim: Option<ViewAnim>,
}

/// A smooth turn of the camera to new angles.
#[derive(Debug, Clone, Copy)]
struct ViewAnim {
    from: (f32, f32),
    to: (f32, f32),
    start: std::time::Instant,
}

/// How long a view change takes.
const VIEW_ANIM: f32 = 0.22;

impl Default for ViewportState {
    fn default() -> Self {
        ViewportState {
            camera: OrbitCamera::default(),
            hidden_version: 0,
            cache: None,
            geometry_counter: 0,
            preview: BTreeMap::new(),
            models_version: 0,
            selection_bits: Arc::new(Vec::new()),
            selection_version: 0,
            picks: None,
            next_pick: 1,
            pending_pick: None,
            rect: Rect::NOTHING,
            edit_overlay: None,
            pick_elements: false,
            size_px: [0, 0],
            frame_pending: true,
            gpu: false,
            fps: 0.0,
            last_frame: None,
            anim: None,
        }
    }
}

/// What the viewport asks the app to do after a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewportEvent {
    /// A click picked an object (`None`: empty space). `add` toggles it in
    /// the selection instead of replacing it.
    Picked { object: Option<ObjectKey>, add: bool },
    /// A click in an element mode at `at` (points); the app picks the
    /// vertex, edge or face there.
    ElementClick { at: egui::Pos2, add: bool },
}

/// GPU resources stored in egui_wgpu's callback resources.
struct Gpu {
    renderer: ViewportRenderer,
    selection_version: u64,
    models_version: u64,
}

/// Creates the viewport renderer on eframe's device. Call once at startup.
pub fn install(rs: &egui_wgpu::RenderState) -> PickSlot {
    let renderer = ViewportRenderer::new(&rs.device, rs.target_format, 1);
    let picks = renderer.picks();
    rs.renderer.write().callback_resources.insert(Gpu { renderer, selection_version: u64::MAX, models_version: u64::MAX });
    picks
}

struct Callback {
    frame: ViewFrame,
    /// The element overlay and its key, in element modes.
    edit: Option<(Arc<p3d_render::EditOverlay>, u64)>,
    view: Arc<ViewScene>,
    geometry: u64,
    models: Arc<Vec<Mat4>>,
    models_version: u64,
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
        gpu.renderer.upload(device, &self.view, self.geometry);
        if gpu.models_version != self.models_version {
            gpu.renderer.set_transforms(device, queue, &self.models);
            gpu.models_version = self.models_version;
        }
        if gpu.selection_version != self.selection_version {
            gpu.renderer.set_selection(device, queue, &self.selection);
            gpu.selection_version = self.selection_version;
        }
        match &self.edit {
            Some((overlay, key)) => gpu.renderer.set_edit(device, overlay, *key),
            None => gpu.renderer.clear_edit(),
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

    /// The set of objects hidden in the viewport changed.
    pub fn hidden_changed(&mut self) {
        self.hidden_version = self.hidden_version.wrapping_add(1);
    }

    /// The scene for the current document. When `key` changes the part
    /// matrices are recomputed, and the geometry only if a mesh changed.
    ///
    /// Complexity: O(objects) when only transforms changed, plus O(total
    /// mesh size) when a mesh changed, O(1) otherwise.
    pub fn scene(&mut self, doc: &Document, key: u64, hidden: &std::collections::BTreeSet<ObjectKey>) -> SceneCache {
        if let Some(c) = &self.cache
            && c.key == key
            && c.hidden == self.hidden_version
        {
            return c.clone();
        }
        let _span = tracing::info_span!("viewport scene", key).entered();
        let mut list = p3d_render::view_meshes(doc);
        list.retain(|(k, _)| !hidden.contains(k));
        let (view, geometry) = match &self.cache {
            Some(c) if c.view.same_meshes(&list) => (Arc::clone(&c.view), c.geometry),
            _ => {
                self.geometry_counter += 1;
                (Arc::new(p3d_render::prepare_view(&list)), self.geometry_counter)
            }
        };
        let models = Arc::new(p3d_render::part_matrices(doc, &view.parts));
        let c = SceneCache { key, view, geometry, models, hidden: self.hidden_version };
        self.cache = Some(c.clone());
        self.models_version = self.models_version.wrapping_add(1);
        // Part indices may have changed: the selection bitset must be rebuilt.
        self.selection_version = self.selection_version.wrapping_add(1);
        c
    }

    /// The part matrices with the drag preview applied.
    ///
    /// Complexity: O(parts) while previewing, O(1) otherwise.
    fn effective_models(&mut self, cache: &SceneCache) -> (Arc<Vec<Mat4>>, u64) {
        if self.preview.is_empty() {
            return (Arc::clone(&cache.models), self.models_version);
        }
        let models: Vec<Mat4> =
            cache.view.parts.iter().zip(cache.models.iter()).map(|(p, m)| self.preview.get(&p.object).copied().unwrap_or(*m)).collect();
        self.models_version = self.models_version.wrapping_add(1);
        (Arc::new(models), self.models_version)
    }

    /// The world matrix of an object's part, if it is drawn.
    ///
    /// Complexity: O(parts).
    /// The part index, mesh and world matrix of `object` in the cached scene.
    ///
    /// Complexity: O(parts).
    pub fn part_of(&self, object: ObjectKey) -> Option<(u32, Arc<p3d_mesh::Mesh>, Mat4)> {
        let c = self.cache.as_ref()?;
        let i = c.view.parts.iter().position(|p| p.object == object)?;
        Some((i as u32, Arc::clone(c.view.meshes.get(i)?), *c.models.get(i)?))
    }

    pub fn part_matrix(&self, object: ObjectKey) -> Option<Mat4> {
        let c = self.cache.as_ref()?;
        c.view.parts.iter().position(|p| p.object == object).map(|i| c.models[i])
    }

    /// Rebuilds the selection bitset over the cached scene's parts.
    ///
    /// Complexity: O(parts + selection log selection).
    pub fn set_selection(&mut self, selected: &std::collections::BTreeSet<ObjectKey>) {
        let Some(c) = &self.cache else { return };
        let mut bits = vec![0u32; c.view.parts.len().div_ceil(32)];
        for (i, part) in c.view.parts.iter().enumerate() {
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
        let mut parts = c.view.parts.iter().zip(c.models.iter()).filter(|(p, _)| only.is_none_or(|s| s.contains(&p.object))).peekable();
        parts.peek()?;
        Some(parts.fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(lo, hi), (p, m)| {
            let (a, b) = p3d_render::world_bounds(p, *m);
            (lo.min(a), hi.max(b))
        }))
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
        self.look_from(view.frame().0);
    }

    /// Turns the camera smoothly to look from `toward` (the shorter way
    /// around).
    pub fn look_from(&mut self, toward: Vec3) {
        let to = OrbitCamera::angles_from(toward);
        self.anim = Some(ViewAnim { from: (self.camera.yaw, self.camera.pitch), to, start: std::time::Instant::now() });
    }

    /// Whether the camera looks from `toward` (within a degree).
    pub fn looks_from(&self, toward: Vec3) -> bool {
        let target = self.anim.map_or((self.camera.yaw, self.camera.pitch), |a| a.to);
        let (yaw, pitch) = OrbitCamera::angles_from(toward);
        let dyaw = (yaw - target.0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        // Straight up or down: yaw does not matter.
        (pitch - target.1).abs() < 0.02 && (dyaw.abs() < 0.02 || pitch.abs() > 1.55)
    }

    /// Advances a view animation; returns whether one is running.
    fn step_anim(&mut self) -> bool {
        let Some(a) = self.anim else { return false };
        let t = (a.start.elapsed().as_secs_f32() / VIEW_ANIM).min(1.0);
        let s = t * t * (3.0 - 2.0 * t);
        let dyaw = (a.to.0 - a.from.0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        self.camera.yaw = (a.from.0 + dyaw * s).rem_euclid(std::f32::consts::TAU);
        self.camera.pitch = a.from.1 + (a.to.1 - a.from.1) * s;
        if t >= 1.0 {
            self.anim = None;
        }
        true
    }

    /// Stops a view animation (the user took over the camera).
    pub fn cancel_anim(&mut self) {
        self.anim = None;
    }

    /// Lays out the viewport in `ui`: handles navigation and clicks and adds
    /// the paint callback.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        key: u64,
        hidden: &std::collections::BTreeSet<ObjectKey>,
        settings: &ViewSettings,
        style: p3d_render::ViewStyle,
    ) -> Vec<ViewportEvent> {
        let element_mode = self.pick_elements;
        let rect = ui.max_rect();
        self.rect = rect;
        let resp = ui.interact(rect, ui.id().with("viewport"), Sense::click_and_drag());
        crate::control::record_widget(ui, &resp, "Viewport", "viewport", None);
        let cache = self.scene(doc, key, hidden);
        if self.frame_pending && rect.is_positive() {
            self.frame_pending = false;
            self.frame(&Default::default());
        }
        self.camera.ortho = settings.ortho;
        if self.step_anim() {
            ui.ctx().request_repaint();
        }
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
        if element_mode
            && resp.clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
            events.push(ViewportEvent::ElementClick { at: p, add });
        } else if resp.clicked()
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
            style: p3d_render::ViewStyle { vertex_px: style.vertex_px * ppp, ..style },
            ids: self.has_selection(),
            pick,
            xray: settings.xray.then_some(settings.xray_alpha),
        };
        if self.gpu {
            let (models, models_version) = self.effective_models(&cache);
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                rect,
                Callback {
                    frame,
                    edit: self.edit_overlay.clone(),
                    view: Arc::clone(&cache.view),
                    geometry: cache.geometry,
                    models,
                    models_version,
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
            let object = r.part.and_then(|i| cache.view.parts.get(i as usize)).map(|p| p.object);
            events.push(ViewportEvent::Picked { object, add });
        }
    }

    /// Orbit, pan and zoom from mouse, wheel and trackpad input.
    fn navigate(&mut self, ui: &egui::Ui, resp: &egui::Response, nav: NavPreset) {
        let h = self.rect.height().max(1.0);
        let orbit_speed = 0.008;
        let (modifiers, delta) = ui.input(|i| (i.modifiers, i.pointer.delta()));
        if resp.dragged() && delta != Vec2::ZERO {
            self.anim = None;
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
        if !scroll_events.is_empty() {
            self.anim = None;
        }
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
