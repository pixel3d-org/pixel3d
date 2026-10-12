//! Orthographic cameras that frame the scene from fixed views.

use glam::{Mat4, Vec3};

/// Where the camera looks from (project spec 17.15 `render` views).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewKind {
    /// From +Z, the asset's front.
    Front,
    Back,
    /// From +X.
    Right,
    Left,
    /// From +Y, front at the bottom of the image.
    Top,
    Bottom,
    /// From the front, right and above.
    ThreeQuarter,
}

impl ViewKind {
    pub const ALL: [ViewKind; 7] =
        [ViewKind::Front, ViewKind::Back, ViewKind::Right, ViewKind::Left, ViewKind::Top, ViewKind::Bottom, ViewKind::ThreeQuarter];

    pub fn name(self) -> &'static str {
        match self {
            ViewKind::Front => "front",
            ViewKind::Back => "back",
            ViewKind::Right => "right",
            ViewKind::Left => "left",
            ViewKind::Top => "top",
            ViewKind::Bottom => "bottom",
            ViewKind::ThreeQuarter => "three_quarter",
        }
    }

    pub fn parse(s: &str) -> Option<ViewKind> {
        ViewKind::ALL.into_iter().find(|v| v.name() == s)
    }

    /// Direction from the scene toward the camera, and the camera's up.
    fn frame(self) -> (Vec3, Vec3) {
        match self {
            ViewKind::Front => (Vec3::Z, Vec3::Y),
            ViewKind::Back => (Vec3::NEG_Z, Vec3::Y),
            ViewKind::Right => (Vec3::X, Vec3::Y),
            ViewKind::Left => (Vec3::NEG_X, Vec3::Y),
            ViewKind::Top => (Vec3::Y, Vec3::NEG_Z),
            ViewKind::Bottom => (Vec3::NEG_Y, Vec3::Z),
            ViewKind::ThreeQuarter => (Vec3::new(1.0, 0.8, 1.2).normalize(), Vec3::Y),
        }
    }
}

/// A camera's matrices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub view: Mat4,
    pub view_proj: Mat4,
    /// Direction toward the camera (world).
    pub toward: Vec3,
}

impl Camera {
    /// Projects a world point to pixel coordinates (origin top-left) and
    /// depth in 0..1.
    pub fn to_pixel(&self, p: Vec3, width: u32, height: u32) -> (f32, f32, f32) {
        let c = self.view_proj.project_point3(p);
        ((c.x * 0.5 + 0.5) * width as f32, (0.5 - c.y * 0.5) * height as f32, c.z)
    }
}

/// An orthographic camera framing the box `lo..hi` from `view` with a margin
/// (fraction of the larger image side) for an image of `aspect` = width /
/// height.
pub fn frame(view: ViewKind, lo: Vec3, hi: Vec3, aspect: f32, margin: f32) -> Camera {
    let (toward, up) = view.frame();
    let center = (lo + hi) / 2.0;
    let radius = ((hi - lo).length() / 2.0).max(1e-3);
    let eye = center + toward * (radius * 4.0);
    let v = glam::camera::rh::view::look_at_mat4(eye, center, up);
    // Extents of the box corners in view space.
    let (mut vlo, mut vhi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for i in 0..8 {
        let c = Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
        let q = v.transform_point3(c);
        vlo = vlo.min(q);
        vhi = vhi.max(q);
    }
    let mid = (vlo + vhi) / 2.0;
    let mut half_w = ((vhi.x - vlo.x) / 2.0).max(1e-4);
    let mut half_h = ((vhi.y - vlo.y) / 2.0).max(1e-4);
    // Keep the image aspect, then add the margin.
    if half_w / half_h < aspect {
        half_w = half_h * aspect;
    } else {
        half_h = half_w / aspect;
    }
    let scale = 1.0 / (1.0 - 2.0 * margin.clamp(0.0, 0.4));
    half_w *= scale;
    half_h *= scale;
    // View-space z is negative in front of the camera.
    let near = (-vhi.z - radius * 0.1).max(1e-4);
    let far = -vlo.z + radius * 0.1;
    // wgpu clip space: depth 0 to 1, y up (the Direct3D convention).
    let p = glam::camera::rh::proj::directx::orthographic(mid.x - half_w, mid.x + half_w, mid.y - half_h, mid.y + half_h, near, far);
    Camera { view: v, view_proj: p * v, toward }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_view_frames_the_box() {
        let c = frame(ViewKind::Front, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0), 1.0, 0.0);
        let (x, y, z) = c.to_pixel(Vec3::new(0.0, 2.0, 1.0), 100, 100);
        assert!((x - 50.0).abs() < 1e-3 && y.abs() < 1e-3, "top center at the image top: {x} {y}");
        assert!((0.0..=1.0).contains(&z));
        let (x, _, _) = c.to_pixel(Vec3::new(0.5, 1.0, 0.0), 100, 100);
        assert!((x - 75.0).abs() < 1e-3, "x = +0.5 is halfway to the right edge: {x}");
        // The front face is nearer than the back face.
        let (_, _, zf) = c.to_pixel(Vec3::new(0.0, 1.0, 1.0), 100, 100);
        let (_, _, zb) = c.to_pixel(Vec3::new(0.0, 1.0, -1.0), 100, 100);
        assert!(zf < zb);
    }

    #[test]
    fn top_view_puts_the_front_at_the_bottom() {
        let c = frame(ViewKind::Top, Vec3::splat(-1.0), Vec3::splat(1.0), 1.0, 0.0);
        let (_, yf, _) = c.to_pixel(Vec3::new(0.0, 1.0, 1.0), 100, 100);
        let (_, yb, _) = c.to_pixel(Vec3::new(0.0, 1.0, -1.0), 100, 100);
        assert!(yf > yb, "front {yf} below back {yb}");
        assert_eq!(ViewKind::parse("three_quarter"), Some(ViewKind::ThreeQuarter));
    }
}
