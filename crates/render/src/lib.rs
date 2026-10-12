//! wgpu renderer shared by the viewport and headless renders.
//!
//! Layer L2 (algorithms). See `docs/architecture.md`.
//!
//! Headless renders are optional confirmation for agents and pictures for
//! people (project spec 17.7, 17.15): shaded, wire, shaded with wire and
//! matcap modes from fixed orthographic views, tiled into one contact sheet,
//! with each part's name drawn next to it so a vision model can tie pixels
//! to part names.

pub mod camera;
pub mod font;
mod gpu;
pub mod image;
pub mod scene;
pub mod viewport;

pub use camera::{Camera, OrbitCamera, ViewKind};
pub use gpu::Renderer;
pub use image::Image;
pub use scene::{RenderScene, prepare};
pub use viewport::{PickResult, Shading, ViewFrame, ViewStyle, ViewportRenderer};

/// wgpu as this crate uses it, so frontends name the same types.
pub use wgpu;

/// Empty border around the framed scene, as a fraction of the image side.
pub const MARGIN: f32 = 0.06;

/// Smallest and largest image side in pixels.
pub const MIN_SIZE: u32 = 64;
pub const MAX_SIZE: u32 = 2048;

/// How surfaces look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Lit, one soft color per part; inside-out faces show red.
    #[default]
    Shaded,
    /// Hidden-line wireframe: polygon edges on blank surfaces.
    Wire,
    /// Shaded with polygon edges.
    ShadedWire,
    /// Gray clay lit in view space: reads form and smoothness.
    Matcap,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Shaded, Mode::Wire, Mode::ShadedWire, Mode::Matcap];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Shaded => "shaded",
            Mode::Wire => "wire",
            Mode::ShadedWire => "shaded_wire",
            Mode::Matcap => "matcap",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// What to render.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    pub views: Vec<ViewKind>,
    pub mode: Mode,
    /// Side of each view's square image, pixels.
    pub size: u32,
    /// Tile every view into one image.
    pub sheet: bool,
    /// Draw part names.
    pub labels: bool,
}

impl Default for RenderRequest {
    fn default() -> Self {
        RenderRequest {
            views: vec![ViewKind::Front, ViewKind::Right, ViewKind::Top, ViewKind::ThreeQuarter],
            mode: Mode::Shaded,
            size: 512,
            sheet: true,
            labels: true,
        }
    }
}

/// Why a render failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    #[error("no GPU adapter is available ({0}); renders need a GPU or a software adapter such as lavapipe or WARP")]
    NoAdapter(String),
    #[error("the GPU device could not be created: {0}")]
    Device(String),
    #[error("reading the image back failed: {0}")]
    Readback(String),
    #[error("size {0} is outside {MIN_SIZE} to {MAX_SIZE}")]
    Size(u32),
    #[error("nothing to render: no visible mesh")]
    Empty,
}
