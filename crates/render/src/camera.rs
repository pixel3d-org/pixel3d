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

/// The interactive viewport camera: a turntable orbit around a target point,
/// perspective or orthographic, with reverse-Z depth (1 at the near plane,
/// 0 infinitely far).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrbitCamera {
    /// The point the camera orbits and looks at (world).
    pub target: Vec3,
    /// Rotation around +Y in radians; 0 looks at the front (from +Z).
    pub yaw: f32,
    /// Elevation in radians; positive looks down from above.
    pub pitch: f32,
    /// Distance from the target in meters.
    pub distance: f32,
    /// Vertical field of view in radians (also sets the orthographic scale).
    pub fov_y: f32,
    pub ortho: bool,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { target: Vec3::new(0.0, 0.5, 0.0), yaw: 0.6, pitch: 0.45, distance: 4.0, fov_y: 40f32.to_radians(), ortho: false }
    }
}

/// Pitch stays just short of straight up or down so the view never flips.
const MAX_PITCH: f32 = 1.5607;
const MIN_DISTANCE: f32 = 1e-3;
const MAX_DISTANCE: f32 = 1e5;

impl OrbitCamera {
    /// Unit direction from the target toward the camera.
    pub fn toward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, cy * cp)
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.toward() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    /// Half the visible height at the target distance, meters.
    pub fn half_height(&self) -> f32 {
        self.distance * (self.fov_y / 2.0).tan()
    }

    fn near(&self) -> f32 {
        (self.distance * 1e-3).max(1e-4)
    }

    /// Projection for an image of `aspect` = width / height (wgpu clip space, reverse-Z).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(1e-3);
        if self.ortho {
            let h = self.half_height();
            let w = h * aspect;
            // Near and far swapped: reverse-Z. The range covers anything within
            // a large radius of the target.
            let depth = (self.distance * 1e3).max(1e3);
            glam::camera::rh::proj::directx::orthographic(-w, w, -h, h, self.distance + depth, (self.distance - depth).max(-depth))
        } else {
            glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect, self.near())
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    /// Turntable orbit by angles in radians (positive `dx` turns the view to
    /// the right around the target, positive `dy` raises the camera).
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw - dx).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dy).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Meters per pixel at the target distance for a viewport `height_px` tall.
    pub fn meters_per_pixel(&self, height_px: f32) -> f32 {
        2.0 * self.half_height() / height_px.max(1.0)
    }

    /// Moves the target with the pointer: `dx`, `dy` in pixels (right, down).
    pub fn pan(&mut self, dx: f32, dy: f32, height_px: f32) {
        let s = self.meters_per_pixel(height_px);
        let inv = self.view().inverse();
        let right = inv.transform_vector3(Vec3::X);
        let up = inv.transform_vector3(Vec3::Y);
        self.target += (-right * dx + up * dy) * s;
    }

    /// Zooms by `factor` (< 1 moves closer). With a `toward` point (the
    /// point under the cursor), that point stays under the cursor.
    pub fn zoom(&mut self, factor: f32, toward: Option<Vec3>) {
        let factor = factor.clamp(0.05, 20.0);
        let new_distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
        let k = new_distance / self.distance;
        if let Some(p) = toward {
            self.target = p + (self.target - p) * k;
        }
        self.distance = new_distance;
    }

    /// Fits the box `lo..hi` in a viewport of `aspect`.
    pub fn frame(&mut self, lo: Vec3, hi: Vec3, aspect: f32) {
        let radius = ((hi - lo).length() / 2.0).max(0.05);
        self.target = (lo + hi) / 2.0;
        let half = (self.fov_y / 2.0).tan();
        let fit = if aspect < 1.0 { half * aspect.max(0.1) } else { half };
        self.distance = (radius / fit * 1.1).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    /// Looks from one of the fixed views (keeps target and distance).
    pub fn set_view(&mut self, view: ViewKind) {
        let (toward, _) = view.frame();
        self.pitch = toward.y.clamp(-1.0, 1.0).asin().clamp(-MAX_PITCH, MAX_PITCH);
        if toward.x.abs() + toward.z.abs() > 1e-6 {
            self.yaw = toward.x.atan2(toward.z).rem_euclid(std::f32::consts::TAU);
        } else {
            self.yaw = 0.0;
        }
    }

    /// World ray through a pixel (`x`, `y` from the top-left) of a `w` x `h`
    /// viewport: origin on the near plane and unit direction.
    pub fn ray(&self, x: f32, y: f32, w: f32, h: f32) -> (Vec3, Vec3) {
        let inv = self.view_proj(w / h.max(1.0)).inverse();
        let ndc = glam::Vec2::new(x / w.max(1.0) * 2.0 - 1.0, 1.0 - y / h.max(1.0) * 2.0);
        let a = inv.project_point3(ndc.extend(1.0));
        let b = inv.project_point3(ndc.extend(0.5));
        (a, (b - a).normalize_or(-self.toward()))
    }

    /// A grid cell size for this view: a power of ten that keeps cells a
    /// readable size on screen.
    pub fn grid_cell(&self) -> f32 {
        let span = self.half_height() * 2.0;
        10f32.powf((span / 12.0).max(1e-3).log10().floor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_camera_looks_at_its_target_and_frames_boxes() {
        let mut c = OrbitCamera::default();
        c.set_view(ViewKind::Front);
        assert!(c.toward().abs_diff_eq(Vec3::Z, 1e-5), "{}", c.toward());
        c.frame(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0), 1.5);
        assert!(c.target.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-6));
        let vp = c.view_proj(1.5);
        for corner in [Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 2.0, 1.0)] {
            let p = vp.project_point3(corner);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{corner} projects to {p}");
            assert!(p.z > 0.0 && p.z <= 1.0, "reverse-Z depth in (0, 1]: {}", p.z);
        }
        // Reverse-Z: nearer points have larger depth.
        let near = vp.project_point3(Vec3::new(0.0, 1.0, 1.0)).z;
        let far = vp.project_point3(Vec3::new(0.0, 1.0, -1.0)).z;
        assert!(near > far);
        c.ortho = true;
        let vp = c.view_proj(1.5);
        let near = vp.project_point3(Vec3::new(0.0, 1.0, 1.0)).z;
        let far = vp.project_point3(Vec3::new(0.0, 1.0, -1.0)).z;
        assert!(near > far && (0.0..=1.0).contains(&near) && (0.0..=1.0).contains(&far));
    }

    #[test]
    fn rays_pan_and_zoom() {
        let mut c = OrbitCamera { target: Vec3::ZERO, yaw: 0.0, pitch: 0.0, distance: 5.0, ..OrbitCamera::default() };
        let (o, d) = c.ray(50.0, 50.0, 100.0, 100.0);
        assert!(d.abs_diff_eq(Vec3::NEG_Z, 1e-4) && o.z > 4.9, "{o} {d}");
        // Panning right moves the target left in the world (content follows the pointer).
        c.pan(10.0, 0.0, 100.0);
        assert!(c.target.x < 0.0 && c.target.y.abs() < 1e-6);
        // Zooming toward a point keeps it fixed relative to the target scale.
        let p = Vec3::new(1.0, 0.0, 0.0);
        let before = c.target - p;
        c.zoom(0.5, Some(p));
        assert!(((c.target - p) - before * 0.5).length() < 1e-5);
        assert!((c.distance - 2.5).abs() < 1e-5);
        // Pitch is clamped short of the poles.
        c.orbit(0.0, 10.0);
        assert!(c.pitch < std::f32::consts::FRAC_PI_2);
        assert!(c.grid_cell() > 0.0);
    }

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
