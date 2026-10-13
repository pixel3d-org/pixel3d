//! The interactive viewport renderer.
//!
//! Each frame is drawn offscreen (project spec 20.5): an `Rgba16Float` color
//! target with 4x MSAA and a reverse-Z `Depth32Float` buffer holds the
//! background, the surfaces, the wireframe and the infinite ground grid; an
//! `R32Uint` buffer holds each pixel's part id for picking and selection
//! outlines. A composite pass then draws the resolved image into the window
//! (egui's pass has no depth buffer, so 3D cannot draw into it directly).
//!
//! The renderer works on a device it is given, so the desktop app shares
//! eframe's device and tests use a headless one. Geometry lives in object
//! space and is uploaded only when the meshes change; each part's world
//! matrix sits in a storage buffer, so moving an object (or dragging it with
//! the gizmo) rewrites one small array; every frame writes one uniform.

use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::camera::OrbitCamera;
use crate::image::Image;
use crate::scene::{EditOverlay, LineVertex, Vertex, ViewScene};
use crate::{MAX_SIZE, RenderError};

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const IDS: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Largest viewport side in pixels.
pub const MAX_VIEWPORT: u32 = 16_384;

/// How surfaces look in the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// Lit neutral clay with a key light and sky and ground light.
    #[default]
    Solid,
    /// Solid with polygon edges.
    SolidWire,
    /// Edges only, on blank surfaces.
    Wire,
    /// Gray clay lit in view space.
    Matcap,
}

impl Shading {
    pub const ALL: [Shading; 4] = [Shading::Solid, Shading::SolidWire, Shading::Wire, Shading::Matcap];

    pub fn name(self) -> &'static str {
        match self {
            Shading::Solid => "solid",
            Shading::SolidWire => "solid_wire",
            Shading::Wire => "wire",
            Shading::Matcap => "matcap",
        }
    }

    pub fn parse(s: &str) -> Option<Shading> {
        Shading::ALL.into_iter().find(|m| m.name() == s)
    }

    fn surface_mode(self) -> u32 {
        match self {
            Shading::Solid | Shading::SolidWire => 0,
            Shading::Matcap => 1,
            Shading::Wire => 2,
        }
    }

    fn draws_wire(self) -> bool {
        matches!(self, Shading::SolidWire | Shading::Wire)
    }
}

/// Viewport colors, linear RGBA (the app converts its theme tokens).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewStyle {
    pub bg_top: [f32; 4],
    pub bg_bottom: [f32; 4],
    /// Alpha is the line opacity.
    pub grid_minor: [f32; 4],
    pub grid_major: [f32; 4],
    pub axis_x: [f32; 4],
    pub axis_z: [f32; 4],
    pub surface: [f32; 4],
    /// Alpha is how strongly selected surfaces are tinted.
    pub selection: [f32; 4],
    pub wire: [f32; 4],
    /// Selected vertices, edges and faces in the element modes.
    pub element: [f32; 4],
    /// Unselected edges and vertices of the mesh being edited.
    pub edit_wire: [f32; 4],
    /// Diameter of vertex dots in pixels.
    pub vertex_px: f32,
}

/// sRGB 8-bit to linear.
pub fn linear(rgba: [u8; 4]) -> [f32; 4] {
    let c = |x: u8| {
        let s = f32::from(x) / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    [c(rgba[0]), c(rgba[1]), c(rgba[2]), f32::from(rgba[3]) / 255.0]
}

/// sRGB `0xrrggbb` to linear RGB with the given alpha.
pub fn linear_hex(rgb: u32, alpha: f32) -> [f32; 4] {
    let [_, r, g, b] = rgb.to_be_bytes();
    let [r, g, b, _] = linear([r, g, b, 255]);
    [r, g, b, alpha]
}

impl Default for ViewStyle {
    /// The dark theme's viewport colors (project spec 20.3).
    fn default() -> Self {
        ViewStyle {
            bg_top: linear_hex(0x2a3038, 1.0),
            bg_bottom: linear_hex(0x1d2228, 1.0),
            grid_minor: linear_hex(0x2e353f, 0.9),
            grid_major: linear_hex(0x3a4350, 1.0),
            axis_x: linear_hex(0xe5484d, 1.0),
            axis_z: linear_hex(0x3e63dd, 1.0),
            surface: linear_hex(0xa8aeb6, 1.0),
            selection: linear_hex(0x4a8fff, 0.18),
            wire: linear_hex(0x101216, 1.0),
            element: linear_hex(0xfb923c, 1.0),
            edit_wire: linear_hex(0x101216, 1.0),
            vertex_px: 6.0,
        }
    }
}

/// Everything one viewport frame needs besides the scene.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewFrame {
    /// Target size in pixels.
    pub width: u32,
    pub height: u32,
    pub camera: OrbitCamera,
    pub shading: Shading,
    pub grid: bool,
    pub style: ViewStyle,
    /// Draw part ids this frame (needed for outlines and picks).
    pub ids: bool,
    /// Pixel to read the part id of, with a request number echoed back in
    /// the [`PickResult`].
    pub pick: Option<(u32, u32, u64)>,
    /// X-ray: surfaces drawn see-through with this opacity (0 to 1), every
    /// edge visible; `None` draws them opaque.
    pub xray: Option<f32>,
}

impl ViewFrame {
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height.max(1) as f32
    }
}

/// The part under a picked pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickResult {
    pub request: u64,
    /// Part index into the scene's parts, `None` on the background.
    pub part: Option<u32>,
}

/// Shared slot the renderer publishes pick results to.
pub type PickSlot = Arc<Mutex<Option<PickResult>>>;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    eye: [f32; 4],
    light: [f32; 4],
    bg_top: [f32; 4],
    bg_bottom: [f32; 4],
    grid_minor: [f32; 4],
    grid_major: [f32; 4],
    axis_x: [f32; 4],
    axis_z: [f32; 4],
    surface: [f32; 4],
    selection: [f32; 4],
    wire: [f32; 4],
    params: [f32; 4],
    flags: [u32; 4],
    /// x: surface opacity (below 1 in x-ray); y: vertex dot diameter (px).
    extra: [f32; 4],
    element: [f32; 4],
    edit_wire: [f32; 4],
}

struct Targets {
    width: u32,
    height: u32,
    msaa: wgpu::TextureView,
    resolved: wgpu::TextureView,
    depth: wgpu::TextureView,
    ids: wgpu::Texture,
    ids_view: wgpu::TextureView,
    id_depth: wgpu::TextureView,
    composite: wgpu::BindGroup,
}

/// The uploaded [`EditOverlay`]: buffers and vertex counts.
struct EditGeometry {
    lines: Option<(wgpu::Buffer, u32)>,
    points: Option<(wgpu::Buffer, u32)>,
    faces: Option<(wgpu::Buffer, u32)>,
}

struct Geometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    lines: Option<(wgpu::Buffer, u32)>,
}

enum Pick {
    /// The texel was copied in a frame not yet known to be submitted.
    Copied { buffer: wgpu::Buffer, request: u64 },
    /// Mapping was requested; the callback fills the slot.
    Mapping { buffer: wgpu::Buffer, request: u64, done: Arc<Mutex<Option<bool>>> },
}

/// Pipelines, targets and uploaded geometry of one viewport.
pub struct ViewportRenderer {
    output: wgpu::TextureFormat,
    globals: wgpu::Buffer,
    globals_layout: wgpu::BindGroupLayout,
    globals_bind: wgpu::BindGroup,
    selection: wgpu::Buffer,
    selection_words: usize,
    /// Per-part world transforms ([`PartTransform`]).
    transforms: wgpu::Buffer,
    transform_slots: usize,
    composite_layout: wgpu::BindGroupLayout,
    background: wgpu::RenderPipeline,
    surfaces: wgpu::RenderPipeline,
    /// Surfaces for x-ray: blended, without depth writes.
    surfaces_xray: wgpu::RenderPipeline,
    /// The element overlay: edges, vertex dots and selected faces.
    edit_lines: wgpu::RenderPipeline,
    edit_points: wgpu::RenderPipeline,
    edit_faces: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    ids: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    targets: Option<Targets>,
    geometry: Option<Geometry>,
    key: Option<u64>,
    edit: Option<EditGeometry>,
    edit_key: Option<u64>,
    pick: Option<Pick>,
    picks: PickSlot,
}

impl std::fmt::Debug for ViewportRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewportRenderer").field("output", &self.output).field("key", &self.key).finish()
    }
}

/// A part's world transform as the shaders read it: the model matrix, the
/// normal matrix (inverse transpose) and whether it mirrors (flips winding).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PartTransform {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    flags: [u32; 4],
}

impl PartTransform {
    fn new(model: Mat4) -> Self {
        let m3 = glam::Mat3::from_mat4(model);
        let det = m3.determinant();
        let normal = if det.abs() > 1e-20 { Mat4::from_mat3(m3.inverse().transpose()) } else { Mat4::IDENTITY };
        PartTransform { model: model.to_cols_array_2d(), normal: normal.to_cols_array_2d(), flags: [u32::from(det < 0.0), 0, 0, 0] }
    }
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(16),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bind_globals(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    selection: &wgpu::Buffer,
    transforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewport globals"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: selection.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: transforms.as_entire_binding() },
        ],
    })
}

fn depth_state(write: bool, compare: wgpu::CompareFunction, bias: wgpu::DepthBiasState) -> Option<wgpu::DepthStencilState> {
    Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias })
}

impl ViewportRenderer {
    /// Builds the pipelines on `device`. `output` is the format of the
    /// target the composite pass draws into, with `output_samples` samples.
    pub fn new(device: &wgpu::Device, output: wgpu::TextureFormat, output_samples: u32) -> ViewportRenderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewport shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
        });
        let buffer_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport globals"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                buffer_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport composite"),
            entries: &[texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }), texture_entry(1, wgpu::TextureSampleType::Uint)],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let selection = storage(device, "viewport selection", 4);
        let transforms = storage(device, "viewport transforms", std::mem::size_of::<PartTransform>() as u64);
        let globals_bind = bind_globals(device, &globals_layout, &globals, &selection, &transforms);

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport scene"),
            bind_group_layouts: &[Some(&globals_layout)],
            ..Default::default()
        });
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport composite"),
            bind_group_layouts: &[Some(&globals_layout), Some(&composite_layout)],
            ..Default::default()
        });
        let surface_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Float32],
        })];
        let line_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
        })];
        let opaque = [Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let blended =
            [Some(wgpu::ColorTargetState { format: HDR, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })];
        let id_target = [Some(wgpu::ColorTargetState { format: IDS, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let out_target = [Some(wgpu::ColorTargetState { format: output, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let msaa = wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false };
        let single = wgpu::MultisampleState::default();
        let pipeline = |label, layout: &wgpu::PipelineLayout, vs, fs, buffers, targets, topology, depth, multisample| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets }),
                primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
                depth_stencil: depth,
                multisample,
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::CompareFunction::{Always, Greater, GreaterEqual};
        use wgpu::PrimitiveTopology::{LineList, TriangleList};
        let none: &[Option<wgpu::VertexBufferLayout<'_>>] = &[];
        let background = pipeline(
            "viewport background",
            &scene_layout,
            "vs_full",
            "fs_background",
            none,
            &opaque[..],
            TriangleList,
            depth_state(false, Always, Default::default()),
            msaa,
        );
        let surfaces = pipeline(
            "viewport surfaces",
            &scene_layout,
            "vs_surface",
            "fs_surface",
            &surface_buffers[..],
            &opaque[..],
            TriangleList,
            depth_state(true, Greater, Default::default()),
            msaa,
        );
        let surfaces_xray = pipeline(
            "viewport surfaces x-ray",
            &scene_layout,
            "vs_surface",
            "fs_surface",
            &surface_buffers[..],
            &blended[..],
            TriangleList,
            depth_state(false, Greater, Default::default()),
            msaa,
        );
        let lines = pipeline(
            "viewport wire",
            &scene_layout,
            "vs_line",
            "fs_line",
            &line_buffers[..],
            &opaque[..],
            LineList,
            depth_state(false, GreaterEqual, Default::default()),
            msaa,
        );
        let point_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
        })];
        let edit_lines = pipeline(
            "viewport edit edges",
            &scene_layout,
            "vs_edit_line",
            "fs_edit",
            &line_buffers[..],
            &opaque[..],
            LineList,
            depth_state(false, GreaterEqual, Default::default()),
            msaa,
        );
        let edit_points = pipeline(
            "viewport edit vertices",
            &scene_layout,
            "vs_edit_point",
            "fs_edit_point",
            &point_buffers[..],
            &blended[..],
            TriangleList,
            depth_state(false, GreaterEqual, Default::default()),
            msaa,
        );
        let edit_faces = pipeline(
            "viewport edit faces",
            &scene_layout,
            "vs_edit_face",
            "fs_edit_face",
            &line_buffers[..],
            &blended[..],
            TriangleList,
            depth_state(false, GreaterEqual, Default::default()),
            msaa,
        );
        let grid = pipeline(
            "viewport grid",
            &scene_layout,
            "vs_full",
            "fs_grid",
            none,
            &blended[..],
            TriangleList,
            depth_state(false, GreaterEqual, Default::default()),
            msaa,
        );
        let ids = pipeline(
            "viewport ids",
            &scene_layout,
            "vs_surface",
            "fs_id",
            &surface_buffers[..],
            &id_target[..],
            TriangleList,
            depth_state(true, Greater, Default::default()),
            single,
        );
        let composite = pipeline(
            "viewport composite",
            &composite_pipeline_layout,
            "vs_full",
            "fs_composite",
            none,
            &out_target[..],
            TriangleList,
            None,
            wgpu::MultisampleState { count: output_samples.max(1), mask: !0, alpha_to_coverage_enabled: false },
        );
        ViewportRenderer {
            output,
            globals,
            globals_layout,
            globals_bind,
            selection,
            selection_words: 1,
            transforms,
            transform_slots: 1,
            composite_layout,
            background,
            surfaces,
            surfaces_xray,
            edit_lines,
            edit_points,
            edit_faces,
            lines,
            grid,
            ids,
            composite,
            targets: None,
            geometry: None,
            key: None,
            edit: None,
            edit_key: None,
            pick: None,
            picks: Arc::new(Mutex::new(None)),
        }
    }

    /// Where pick results appear; share it with the code that asks for picks.
    pub fn picks(&self) -> PickSlot {
        Arc::clone(&self.picks)
    }

    /// The scene key of the uploaded geometry.
    pub fn key(&self) -> Option<u64> {
        self.key
    }

    /// Uploads a scene's geometry unless `key` is already uploaded. Set the
    /// part transforms with [`ViewportRenderer::set_transforms`].
    ///
    /// Complexity: O(vertices + indices + lines) when the key changed, O(1) otherwise.
    pub fn upload(&mut self, device: &wgpu::Device, scene: &ViewScene, key: u64) {
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        if scene.is_empty() {
            self.geometry = None;
            return;
        }
        let init =
            |label, contents: &[u8], usage| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        self.geometry = Some(Geometry {
            vertices: init("viewport vertices", bytemuck::cast_slice(&scene.vertices), wgpu::BufferUsages::VERTEX),
            indices: init("viewport indices", bytemuck::cast_slice(&scene.indices), wgpu::BufferUsages::INDEX),
            index_count: scene.indices.len() as u32,
            lines: (!scene.lines.is_empty())
                .then(|| (init("viewport wire", bytemuck::cast_slice(&scene.lines), wgpu::BufferUsages::VERTEX), scene.lines.len() as u32)),
        });
    }

    /// Sets the selected parts as a bitset over part indices (32 per word).
    ///
    /// Complexity: O(words).
    pub fn set_selection(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bits: &[u32]) {
        let words = bits.len().max(1);
        if words > self.selection_words {
            self.selection_words = words.next_power_of_two();
            self.selection = storage(device, "viewport selection", 4 * self.selection_words as u64);
            self.globals_bind = bind_globals(device, &self.globals_layout, &self.globals, &self.selection, &self.transforms);
        }
        let mut data = vec![0u32; self.selection_words];
        data[..bits.len()].copy_from_slice(bits);
        queue.write_buffer(&self.selection, 0, bytemuck::cast_slice(&data));
    }

    /// Sets the world matrix of every part (index = part id). Cheap enough
    /// to call every frame while an object is dragged.
    ///
    /// Complexity: O(parts).
    pub fn set_transforms(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, models: &[Mat4]) {
        let n = models.len().max(1);
        if n > self.transform_slots {
            self.transform_slots = n.next_power_of_two();
            self.transforms = storage(device, "viewport transforms", (self.transform_slots * std::mem::size_of::<PartTransform>()) as u64);
            self.globals_bind = bind_globals(device, &self.globals_layout, &self.globals, &self.selection, &self.transforms);
        }
        let data: Vec<PartTransform> = models.iter().map(|m| PartTransform::new(*m)).collect();
        if !data.is_empty() {
            queue.write_buffer(&self.transforms, 0, bytemuck::cast_slice(&data));
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.targets.as_ref().is_some_and(|t| t.width == width && t.height == height) {
            return;
        }
        let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa = view(&texture("viewport color msaa", HDR, SAMPLES, attach));
        let resolved = view(&texture("viewport color", HDR, 1, attach | wgpu::TextureUsages::TEXTURE_BINDING));
        let depth = view(&texture("viewport depth", DEPTH, SAMPLES, attach));
        let ids = texture("viewport ids", IDS, 1, attach | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let ids_view = view(&ids);
        let id_depth = view(&texture("viewport id depth", DEPTH, 1, attach));
        let composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolved) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&ids_view) },
            ],
        });
        self.targets = Some(Targets { width, height, msaa, resolved, depth, ids, ids_view, id_depth, composite });
    }

    /// Uploads the element overlay unless `key` is the one uploaded last.
    ///
    /// Complexity: O(overlay vertices) on a new key, O(1) otherwise.
    pub fn set_edit(&mut self, device: &wgpu::Device, overlay: &EditOverlay, key: u64) {
        if self.edit_key == Some(key) {
            return;
        }
        let buffer = |label, list: &[LineVertex]| {
            (!list.is_empty()).then(|| {
                let b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: bytemuck::cast_slice(list),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                (b, list.len() as u32)
            })
        };
        self.edit = Some(EditGeometry {
            lines: buffer("viewport edit edges", &overlay.lines),
            points: buffer("viewport edit vertices", &overlay.points),
            faces: buffer("viewport edit faces", &overlay.faces),
        });
        self.edit_key = Some(key);
    }

    /// Stops drawing the element overlay.
    pub fn clear_edit(&mut self) {
        self.edit = None;
        self.edit_key = None;
    }

    /// Encodes the offscreen passes of one frame into `encoder`. Call
    /// [`ViewportRenderer::composite`] in a later pass of the same submission.
    ///
    /// Complexity: GPU work proportional to the uploaded geometry; O(1) on the CPU.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &ViewFrame) {
        self.advance_pick(device);
        let (w, h) = (frame.width.clamp(1, MAX_VIEWPORT), frame.height.clamp(1, MAX_VIEWPORT));
        self.ensure_targets(device, w, h);
        let ids_needed = frame.ids || frame.pick.is_some();
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&self.globals_for(frame, ids_needed)));
        let Some(t) = &self.targets else { return };
        let depth_clear = |view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa,
                    depth_slice: None,
                    resolve_target: Some(&t.resolved),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(depth_clear(&t.depth)),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            // In x-ray the grid goes first so it shows through the surfaces,
            // which then write no depth: everything behind stays visible.
            let xray = frame.xray.is_some();
            if frame.grid && xray {
                pass.set_pipeline(&self.grid);
                pass.draw(0..3, 0..1);
            }
            if let Some(g) = &self.geometry {
                pass.set_pipeline(if xray { &self.surfaces_xray } else { &self.surfaces });
                pass.set_vertex_buffer(0, g.vertices.slice(..));
                pass.set_index_buffer(g.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.index_count, 0, 0..1);
                if frame.shading.draws_wire()
                    && let Some((lines, count)) = &g.lines
                {
                    pass.set_pipeline(&self.lines);
                    pass.set_vertex_buffer(0, lines.slice(..));
                    pass.draw(0..*count, 0..1);
                }
            }
            if frame.grid && !xray {
                pass.set_pipeline(&self.grid);
                pass.draw(0..3, 0..1);
            }
            // The element overlay last, over the grid: it writes no depth.
            if let Some(e) = &self.edit {
                if let Some((b, n)) = &e.faces {
                    pass.set_pipeline(&self.edit_faces);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..*n, 0..1);
                }
                if let Some((b, n)) = &e.lines {
                    pass.set_pipeline(&self.edit_lines);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..*n, 0..1);
                }
                if let Some((b, n)) = &e.points {
                    pass.set_pipeline(&self.edit_points);
                    pass.set_vertex_buffer(0, b.slice(..));
                    pass.draw(0..6, 0..*n);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport ids"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.ids_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(depth_clear(&t.id_depth)),
                ..Default::default()
            });
            if ids_needed && let Some(g) = &self.geometry {
                pass.set_bind_group(0, &self.globals_bind, &[]);
                pass.set_pipeline(&self.ids);
                pass.set_vertex_buffer(0, g.vertices.slice(..));
                pass.set_index_buffer(g.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.index_count, 0, 0..1);
            }
        }
        if let Some((x, y, request)) = frame.pick
            && self.pick.is_none()
        {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport pick"),
                size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &t.ids,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: x.min(w - 1), y: y.min(h - 1), z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            self.pick = Some(Pick::Copied { buffer, request });
        }
    }

    /// Whether a pick is still on its way back from the GPU (keep repainting).
    pub fn pick_pending(&self) -> bool {
        self.pick.is_some()
    }

    /// Moves a pending pick one step: map the buffer copied last frame (it
    /// has been submitted since), then publish the value once mapped.
    fn advance_pick(&mut self, device: &wgpu::Device) {
        match self.pick.take() {
            None => {}
            Some(Pick::Copied { buffer, request }) => {
                let done = Arc::new(Mutex::new(None));
                let flag = Arc::clone(&done);
                buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                    if let Ok(mut d) = flag.lock() {
                        *d = Some(r.is_ok());
                    }
                });
                let _ = device.poll(wgpu::PollType::Poll);
                self.pick = Some(Pick::Mapping { buffer, request, done });
                self.advance_pick_mapped();
            }
            Some(p @ Pick::Mapping { .. }) => {
                let _ = device.poll(wgpu::PollType::Poll);
                self.pick = Some(p);
                self.advance_pick_mapped();
            }
        }
    }

    fn advance_pick_mapped(&mut self) {
        let Some(Pick::Mapping { buffer, request, done }) = &self.pick else { return };
        let state = done.lock().ok().and_then(|d| *d);
        let Some(ok) = state else { return };
        let part = if ok {
            let id = buffer.slice(..).get_mapped_range().ok().map(|d| u32::from_le_bytes([d[0], d[1], d[2], d[3]])).unwrap_or(0);
            buffer.unmap();
            id.checked_sub(1)
        } else {
            None
        };
        let request = *request;
        self.pick = None;
        if let Ok(mut slot) = self.picks.lock() {
            *slot = Some(PickResult { request, part });
        }
    }

    fn globals_for(&self, frame: &ViewFrame, ids_valid: bool) -> Globals {
        let cam = &frame.camera;
        let view = cam.view();
        let view_proj = cam.view_proj(frame.aspect());
        // Key light from the camera's upper left, so the visible side is lit.
        let inv = view.inverse();
        let left = inv.transform_vector3(Vec3::NEG_X);
        let light = (cam.toward() + Vec3::Y * 0.7 + left * 0.45).normalize_or(Vec3::Y);
        let eye = cam.eye();
        let s = &frame.style;
        let encode = u32::from(!self.output.is_srgb());
        Globals {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, if cam.ortho { 1.0 } else { 0.0 }],
            light: [light.x, light.y, light.z, 0.0],
            bg_top: s.bg_top,
            bg_bottom: s.bg_bottom,
            grid_minor: s.grid_minor,
            grid_major: s.grid_major,
            axis_x: s.axis_x,
            axis_z: s.axis_z,
            surface: s.surface,
            selection: s.selection,
            wire: s.wire,
            params: [cam.grid_cell(), (cam.distance * 8.0).max(cam.grid_cell() * 40.0), frame.width as f32, frame.height as f32],
            flags: [frame.shading.surface_mode(), u32::from(frame.grid), encode, u32::from(ids_valid)],
            extra: [frame.xray.map_or(1.0, |a| a.clamp(0.0, 1.0)), s.vertex_px, 0.0, 0.0],
            element: s.element,
            edit_wire: s.edit_wire,
        }
    }

    /// Draws the last rendered frame into `pass` (its viewport set to the
    /// target rectangle by the caller).
    pub fn composite(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(t) = &self.targets else { return };
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &self.globals_bind, &[]);
        pass.set_bind_group(1, &t.composite, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Renders one frame offscreen and reads it back as 8-bit sRGB. For
    /// tests and automation; the renderer must have been created with an
    /// `Rgba8UnormSrgb` output and 1 sample.
    ///
    /// Complexity: GPU work plus O(pixels) readback.
    pub fn snapshot(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &ViewFrame) -> Result<Image, RenderError> {
        if frame.width == 0 || frame.height == 0 || frame.width > MAX_SIZE * 8 || frame.height > MAX_SIZE * 8 {
            return Err(RenderError::Size(frame.width.max(frame.height)));
        }
        let (w, h) = (frame.width, frame.height);
        let out = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("viewport snapshot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport snapshot") });
        self.render(device, queue, &mut encoder, frame);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport snapshot composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            self.composite(&mut pass);
        }
        let row = w * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport snapshot readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &out, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let submitted = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None }).map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv().map_err(|e| RenderError::Readback(e.to_string()))?.map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut pixels = Vec::with_capacity((row * h) as usize);
        {
            let data = slice.get_mapped_range().map_err(|e| RenderError::Readback(e.to_string()))?;
            for y in 0..h as usize {
                let start = y * padded as usize;
                pixels.extend_from_slice(&data[start..start + row as usize]);
            }
        }
        buffer.unmap();
        // Advance a pick copied in this frame now that it was submitted.
        self.advance_pick(device);
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        self.advance_pick(device);
        Ok(Image { width: w, height: h, pixels })
    }
}

/// The camera's view-projection for a frame (for overlays drawn by the app).
pub fn view_proj(frame: &ViewFrame) -> Mat4 {
    frame.camera.view_proj(frame.aspect())
}
