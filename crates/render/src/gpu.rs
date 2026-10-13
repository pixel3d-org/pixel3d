//! The wgpu side: device, pipelines, offscreen targets and readback.

use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::camera::{Camera, frame};
use crate::image::{Image, contact_sheet, draw_labels};
use crate::scene::{LineVertex, RenderScene, Vertex};
use crate::{MARGIN, MAX_SIZE, MIN_SIZE, Mode, RenderError, RenderRequest};

const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const IDS: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
/// Background (linear RGB): light warm gray.
const CLEAR: wgpu::Color = wgpu::Color { r: 0.80, g: 0.80, b: 0.78, a: 1.0 };
/// Surfaces are pushed back a little so their own edges win the depth test.
const SURFACE_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.0 };

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    light: [f32; 4],
    mode: [u32; 4],
    line_color: [f32; 4],
}

/// A scene's buffers on the GPU, shared by every view of one render.
struct Uploaded {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    lines: Option<(wgpu::Buffer, u32)>,
}

/// A GPU device with the render pipelines. Creating one takes tens of
/// milliseconds; keep it and render many times.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter: String,
    layout: wgpu::BindGroupLayout,
    surfaces: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    ids: wgpu::RenderPipeline,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer").field("adapter", &self.adapter).finish()
    }
}

fn depth_state(write: bool, compare: wgpu::CompareFunction, bias: wgpu::DepthBiasState) -> Option<wgpu::DepthStencilState> {
    Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(write), depth_compare: Some(compare), stencil: Default::default(), bias })
}

impl Renderer {
    /// Opens the first suitable GPU, or a software adapter when there is no
    /// GPU.
    pub fn new() -> Result<Renderer, RenderError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .or_else(|_| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: true,
                ..Default::default()
            }))
        })
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let info = adapter.get_info();
        let name = format!("{} ({:?})", info.name, info.backend);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("pixel3d render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render layout"),
            bind_group_layouts: &[Some(&layout)],
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
        let color_target = [Some(wgpu::ColorTargetState { format: COLOR, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let id_target = [Some(wgpu::ColorTargetState { format: IDS, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let msaa = wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false };
        let pipeline = |label, vs, fs, buffers, targets, topology, depth, multisample| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets }),
                primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
                depth_stencil: depth,
                multisample,
                multiview_mask: None,
                cache: None,
            })
        };
        let surfaces = pipeline(
            "surfaces",
            "vs_surface",
            "fs_surface",
            &surface_buffers[..],
            &color_target[..],
            wgpu::PrimitiveTopology::TriangleList,
            depth_state(true, wgpu::CompareFunction::Less, SURFACE_BIAS),
            msaa,
        );
        let lines = pipeline(
            "lines",
            "vs_line",
            "fs_line",
            &line_buffers[..],
            &color_target[..],
            wgpu::PrimitiveTopology::LineList,
            depth_state(false, wgpu::CompareFunction::LessEqual, Default::default()),
            msaa,
        );
        let ids = pipeline(
            "part ids",
            "vs_surface",
            "fs_id",
            &surface_buffers[..],
            &id_target[..],
            wgpu::PrimitiveTopology::TriangleList,
            depth_state(true, wgpu::CompareFunction::Less, SURFACE_BIAS),
            wgpu::MultisampleState::default(),
        );
        tracing::info!(adapter = %name, "renderer ready");
        Ok(Renderer { device, queue, adapter: name, layout, surfaces, lines, ids })
    }

    /// Adapter name and backend, for captions.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// The device, for other renderers that share it (the viewport in tests
    /// and automation).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Renders the requested views: one image per view, or one contact
    /// sheet titled `sheet`.
    ///
    /// Complexity: O(triangles + edges) to upload, then GPU work and
    /// O(pixels) readback per view.
    pub fn render(&self, scene: &RenderScene, req: &RenderRequest) -> Result<Vec<(String, Image)>, RenderError> {
        if !(MIN_SIZE..=MAX_SIZE).contains(&req.size) {
            return Err(RenderError::Size(req.size));
        }
        if scene.is_empty() {
            return Err(RenderError::Empty);
        }
        let init =
            |label, contents: &[u8], usage| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        let up = Uploaded {
            vertices: init("vertices", bytemuck::cast_slice(&scene.vertices), wgpu::BufferUsages::VERTEX),
            indices: init("indices", bytemuck::cast_slice(&scene.indices), wgpu::BufferUsages::INDEX),
            index_count: scene.indices.len() as u32,
            lines: (!scene.lines.is_empty())
                .then(|| (init("lines", bytemuck::cast_slice(&scene.lines), wgpu::BufferUsages::VERTEX), scene.lines.len() as u32)),
        };
        let mut out = Vec::new();
        for &view in &req.views {
            let cam = frame(view, scene.lo, scene.hi, 1.0, MARGIN);
            let (mut img, ids) = self.draw(&up, &cam, req.mode, req.size, req.labels)?;
            if let Some(ids) = ids {
                draw_labels(&mut img, &visible_anchors(&ids, req.size, scene.parts.len()), &scene.parts, label_scale(req.size));
            }
            out.push((view.name().to_owned(), img));
        }
        if req.sheet && out.len() > 1 {
            return Ok(vec![("sheet".to_owned(), contact_sheet(&out, label_scale(req.size)))]);
        }
        Ok(out)
    }

    /// One view: the color image and, when asked, each pixel's part id
    /// (index + 1, 0 for background).
    fn draw(&self, up: &Uploaded, cam: &Camera, mode: Mode, size: u32, with_ids: bool) -> Result<(Image, Option<Vec<u32>>), RenderError> {
        let surface_mode = match mode {
            Mode::Shaded | Mode::ShadedWire => 0,
            Mode::Matcap => 1,
            Mode::Wire => 2,
        };
        // Key light from the camera's upper left, so every view is lit.
        let left = cam.view.inverse().transform_vector3(glam::Vec3::NEG_X);
        let light = (cam.toward + glam::Vec3::Y * 0.6 + left * 0.4).normalize();
        let globals = Globals {
            view_proj: cam.view_proj.to_cols_array_2d(),
            view: cam.view.to_cols_array_2d(),
            light: [light.x, light.y, light.z, 0.0],
            mode: [surface_mode, 0, 0, 0],
            line_color: if mode == Mode::Wire { [0.02, 0.02, 0.03, 1.0] } else { [0.05, 0.05, 0.06, 1.0] },
        };
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        let extent = wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 };
        let texture = |label, format, samples, usage| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let resolved = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR,
            usage: attach | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let resolved_view = resolved.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_view = texture("color msaa", COLOR, SAMPLES, attach);
        let depth_view = texture("depth", DEPTH, SAMPLES, attach);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render") });
        let depth_attachment = |view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
            stencil_ops: None,
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&resolved_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(depth_attachment(&depth_view)),
                ..Default::default()
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.set_pipeline(&self.surfaces);
            pass.set_vertex_buffer(0, up.vertices.slice(..));
            pass.set_index_buffer(up.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..up.index_count, 0, 0..1);
            if matches!(mode, Mode::Wire | Mode::ShadedWire)
                && let Some((lines, count)) = &up.lines
            {
                pass.set_pipeline(&self.lines);
                pass.set_vertex_buffer(0, lines.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        let color = Readback::new(&self.device, &mut encoder, &resolved, size, 4);
        let ids = if with_ids {
            let id_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("part ids"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: IDS,
                usage: attach | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let id_view = id_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let id_depth = texture("id depth", DEPTH, 1, attach);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("part ids"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &id_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(depth_attachment(&id_depth)),
                    ..Default::default()
                });
                pass.set_bind_group(0, &bind, &[]);
                pass.set_pipeline(&self.ids);
                pass.set_vertex_buffer(0, up.vertices.slice(..));
                pass.set_index_buffer(up.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..up.index_count, 0, 0..1);
            }
            Some(Readback::new(&self.device, &mut encoder, &id_texture, size, 4))
        } else {
            None
        };
        let submitted = self.queue.submit([encoder.finish()]);
        let pixels = color.read(&self.device, &submitted)?;
        let ids = match ids {
            Some(r) => Some(r.read(&self.device, &submitted)?.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect()),
            None => None,
        };
        Ok((Image { width: size, height: size, pixels }, ids))
    }
}

/// A texture copied into a mappable buffer (rows padded to the copy
/// alignment).
struct Readback {
    buffer: wgpu::Buffer,
    row: u32,
    padded: u32,
    rows: u32,
}

impl Readback {
    fn new(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture, size: u32, bytes_per_pixel: u32) -> Readback {
        let row = size * bytes_per_pixel;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(size),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size) },
            },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        );
        Readback { buffer, row, padded, rows: size }
    }

    /// Waits for the copy and returns the tightly packed rows.
    ///
    /// Complexity: O(pixels).
    fn read(self, device: &wgpu::Device, submitted: &wgpu::SubmissionIndex) -> Result<Vec<u8>, RenderError> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            // The receiver waits right below.
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait { submission_index: Some(submitted.clone()), timeout: None })
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv().map_err(|e| RenderError::Readback(e.to_string()))?.map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((self.row * self.rows) as usize);
        {
            let data = slice.get_mapped_range().map_err(|e| RenderError::Readback(e.to_string()))?;
            for y in 0..self.rows as usize {
                let start = y * self.padded as usize;
                out.extend_from_slice(&data[start..start + self.row as usize]);
            }
        }
        self.buffer.unmap();
        Ok(out)
    }
}

/// Where to anchor each part's label: the visible pixel of the part nearest
/// the center of its visible pixels, with its visible pixel count. Parts
/// with fewer than 4 visible pixels get none.
///
/// Complexity: O(pixels + parts).
fn visible_anchors(ids: &[u32], size: u32, parts: usize) -> Vec<Option<(i64, i64, usize)>> {
    let w = size as usize;
    let mut sum = vec![(0u64, 0u64, 0usize); parts];
    for (k, &id) in ids.iter().enumerate() {
        if id > 0 && (id as usize) <= parts {
            let s = &mut sum[id as usize - 1];
            s.0 += (k % w) as u64;
            s.1 += (k / w) as u64;
            s.2 += 1;
        }
    }
    let centers: Vec<Option<(f64, f64)>> = sum.iter().map(|&(x, y, n)| (n >= 4).then(|| (x as f64 / n as f64, y as f64 / n as f64))).collect();
    let mut best: Vec<Option<(f64, i64, i64)>> = vec![None; parts];
    for (k, &id) in ids.iter().enumerate() {
        if id == 0 || id as usize > parts {
            continue;
        }
        let i = id as usize - 1;
        let Some((cx, cy)) = centers[i] else { continue };
        let (x, y) = ((k % w) as f64, (k / w) as f64);
        let d = (x - cx).powi(2) + (y - cy).powi(2);
        if best[i].is_none_or(|(b, _, _)| d < b) {
            best[i] = Some((d, x as i64, y as i64));
        }
    }
    best.iter().zip(&sum).map(|(b, s)| b.map(|(_, x, y)| (x, y, s.2))).collect()
}

/// Label font scale for an image size: 1 below 256 px, 2 up to 768, then 3.
fn label_scale(size: u32) -> u32 {
    match size {
        0..256 => 1,
        256..=768 => 2,
        _ => 3,
    }
}
