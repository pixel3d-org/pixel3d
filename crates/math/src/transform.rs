//! Object transforms: translation, rotation, scale kept separately for
//! editing, converted to an affine matrix for evaluation.

use glam::{Affine3A, EulerRot, Quat, Vec3};

/// Translation, rotation and scale. Applied as scale, then rotation, then
/// translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trs {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Trs {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Trs {
    pub const IDENTITY: Trs = Trs { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE };

    pub fn from_translation(translation: Vec3) -> Self {
        Trs { translation, ..Self::IDENTITY }
    }

    /// The affine matrix of this transform.
    pub fn to_affine(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Decomposes an affine matrix. Shear is lost; returns `None` for
    /// non-finite or singular matrices.
    pub fn from_affine(m: &Affine3A) -> Option<Trs> {
        let det = m.matrix3.determinant();
        if !det.is_finite() || det.abs() < f32::EPSILON {
            return None;
        }
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        let t = Trs { translation, rotation: rotation.normalize(), scale };
        t.is_finite().then_some(t)
    }

    pub fn is_finite(&self) -> bool {
        self.translation.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }

    /// Whether the transform is the identity within `eps`.
    pub fn is_identity(&self, eps: f32) -> bool {
        // q and -q are the same rotation.
        let no_rotation = self.rotation.abs_diff_eq(Quat::IDENTITY, eps) || self.rotation.abs_diff_eq(-Quat::IDENTITY, eps);
        no_rotation && self.translation.abs_diff_eq(Vec3::ZERO, eps) && self.scale.abs_diff_eq(Vec3::ONE, eps)
    }
}

/// Rotation from Euler angles in degrees: about world X first, then world Y,
/// then world Z (the convention most DCC tools display as "XYZ").
pub fn rotation_from_euler_deg(x: f32, y: f32, z: f32) -> Quat {
    Quat::from_euler(EulerRot::ZYX, z.to_radians(), y.to_radians(), x.to_radians())
}

/// Inverse of [`rotation_from_euler_deg`].
pub fn rotation_to_euler_deg(q: Quat) -> [f32; 3] {
    let (z, y, x) = q.to_euler(EulerRot::ZYX);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trip() {
        let a = Trs::IDENTITY.to_affine();
        assert_eq!(a, Affine3A::IDENTITY);
        assert!(Trs::from_affine(&a).unwrap().is_identity(1e-6));
    }

    #[test]
    fn scale_rotate_translate_order() {
        let t = Trs { translation: Vec3::new(1.0, 0.0, 0.0), rotation: rotation_from_euler_deg(0.0, 90.0, 0.0), scale: Vec3::new(2.0, 1.0, 1.0) };
        // +X scaled to 2, rotated 90 deg about Y to -Z, then translated.
        let p = t.to_affine().transform_point3(Vec3::X);
        assert!(p.abs_diff_eq(Vec3::new(1.0, 0.0, -2.0), 1e-5), "{p}");
    }

    #[test]
    fn decompose_round_trip() {
        let t = Trs { translation: Vec3::new(0.5, -2.0, 3.0), rotation: rotation_from_euler_deg(10.0, 20.0, 30.0), scale: Vec3::new(1.0, 2.0, 0.5) };
        let back = Trs::from_affine(&t.to_affine()).unwrap();
        assert!(back.to_affine().abs_diff_eq(t.to_affine(), 1e-5));
    }

    #[test]
    fn singular_matrix_rejected() {
        let m = Trs { scale: Vec3::new(0.0, 1.0, 1.0), ..Trs::IDENTITY }.to_affine();
        assert!(Trs::from_affine(&m).is_none());
    }

    #[test]
    fn euler_order_is_x_then_y_then_z() {
        let q = rotation_from_euler_deg(90.0, 90.0, 0.0);
        // +Y about X by 90 goes to +Z, then about Y by 90 goes to +X.
        assert!((q * Vec3::Y).abs_diff_eq(Vec3::X, 1e-5));
        let e = rotation_to_euler_deg(rotation_from_euler_deg(10.0, 20.0, 30.0));
        assert!((e[0] - 10.0).abs() < 1e-3 && (e[1] - 20.0).abs() < 1e-3 && (e[2] - 30.0).abs() < 1e-3, "{e:?}");
    }
}
