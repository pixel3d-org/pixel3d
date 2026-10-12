//! Axis conventions and conversions between them.
//!
//! pixel3d uses the glTF convention: right-handed, +Y up, +Z is the asset's
//! front, 1 unit = 1 meter. Engines use other conventions; [`AxisSystem::conversion_to`]
//! returns the matrix that maps pixel3d coordinates into theirs while keeping
//! the asset's up, front and left/right sides physically the same.

use glam::{Mat3, Vec3};

/// A signed coordinate axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Axis {
    /// Unit vector of this axis.
    pub fn vector(self) -> Vec3 {
        match self {
            Axis::PosX => Vec3::X,
            Axis::NegX => Vec3::NEG_X,
            Axis::PosY => Vec3::Y,
            Axis::NegY => Vec3::NEG_Y,
            Axis::PosZ => Vec3::Z,
            Axis::NegZ => Vec3::NEG_Z,
        }
    }

    /// Index of the coordinate (0 for X, 1 for Y, 2 for Z).
    pub fn index(self) -> usize {
        match self {
            Axis::PosX | Axis::NegX => 0,
            Axis::PosY | Axis::NegY => 1,
            Axis::PosZ | Axis::NegZ => 2,
        }
    }
}

/// Handedness of a coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Handedness {
    Right,
    Left,
}

/// Up axis, front axis, handedness and unit size of a coordinate convention.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisSystem {
    pub up: Axis,
    pub front: Axis,
    pub handedness: Handedness,
    /// Size of one unit in meters (0.01 for centimeters).
    pub unit_meters: f32,
}

impl AxisSystem {
    /// pixel3d internal convention (same as glTF).
    pub const PIXEL3D: AxisSystem = AxisSystem { up: Axis::PosY, front: Axis::PosZ, handedness: Handedness::Right, unit_meters: 1.0 };
    /// glTF 2.0: +Y up, +Z front, right-handed, meters.
    pub const GLTF: AxisSystem = Self::PIXEL3D;
    /// Godot 4: +Y up, models face +Z, right-handed, meters.
    pub const GODOT: AxisSystem = Self::PIXEL3D;
    /// Unity: +Y up, +Z front, left-handed, meters.
    pub const UNITY: AxisSystem = AxisSystem { up: Axis::PosY, front: Axis::PosZ, handedness: Handedness::Left, unit_meters: 1.0 };
    /// Unreal Engine: +Z up, +X front, left-handed, centimeters.
    pub const UNREAL: AxisSystem = AxisSystem { up: Axis::PosZ, front: Axis::PosX, handedness: Handedness::Left, unit_meters: 0.01 };
    /// Blender: +Z up, -Y front, right-handed, meters.
    pub const BLENDER: AxisSystem = AxisSystem { up: Axis::PosZ, front: Axis::NegY, handedness: Handedness::Right, unit_meters: 1.0 };

    /// Whether up and front are distinct, perpendicular axes.
    pub fn is_valid(&self) -> bool {
        self.up.index() != self.front.index() && self.unit_meters.is_finite() && self.unit_meters > 0.0
    }

    /// Basis with columns (side, up, front), where side is the asset's left
    /// for every convention: `up x front` in right-handed systems and its
    /// negation in left-handed ones, expressed in this system's coordinates.
    fn basis(&self) -> Mat3 {
        let up = self.up.vector();
        let front = self.front.vector();
        let cross = up.cross(front);
        let side = match self.handedness {
            Handedness::Right => cross,
            Handedness::Left => -cross,
        };
        Mat3::from_cols(side, up, front)
    }

    /// Linear map from this system's coordinates to `target`'s, including
    /// the unit change. Up maps to up, front to front, and the asset's left
    /// side stays its left side (a reflection when the handedness differs).
    ///
    /// Returns `None` when either system is invalid.
    pub fn conversion_to(&self, target: &AxisSystem) -> Option<Mat3> {
        if !self.is_valid() || !target.is_valid() {
            return None;
        }
        // Both bases are orthonormal, so the inverse is the transpose.
        let rotation = target.basis() * self.basis().transpose();
        Some(rotation * (self.unit_meters / target.unit_meters))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(m: Mat3, v: [f32; 3]) -> [f32; 3] {
        let r = m * Vec3::from(v);
        [r.x, r.y, r.z].map(|c| if c.abs() < 1e-6 { 0.0 } else { c })
    }

    #[test]
    fn identity_for_same_system() {
        let m = AxisSystem::PIXEL3D.conversion_to(&AxisSystem::GLTF).unwrap();
        assert!(m.abs_diff_eq(Mat3::IDENTITY, 1e-6));
    }

    #[test]
    fn unity_mirrors_x() {
        let m = AxisSystem::PIXEL3D.conversion_to(&AxisSystem::UNITY).unwrap();
        assert_eq!(map(m, [1.0, 2.0, 3.0]), [-1.0, 2.0, 3.0]);
        assert!(m.determinant() < 0.0);
    }

    #[test]
    fn unreal_is_z_up_x_front_in_centimeters() {
        let m = AxisSystem::PIXEL3D.conversion_to(&AxisSystem::UNREAL).unwrap();
        // glTF (x, y, z) in meters becomes Unreal (z, -x, y) in centimeters.
        assert_eq!(map(m, [1.0, 0.0, 0.0]), [0.0, -100.0, 0.0]);
        assert_eq!(map(m, [0.0, 1.0, 0.0]), [0.0, 0.0, 100.0]);
        assert_eq!(map(m, [0.0, 0.0, 1.0]), [100.0, 0.0, 0.0]);
    }

    #[test]
    fn blender_is_z_up_minus_y_front() {
        let m = AxisSystem::PIXEL3D.conversion_to(&AxisSystem::BLENDER).unwrap();
        assert_eq!(map(m, [1.0, 2.0, 3.0]), [1.0, -3.0, 2.0]);
        assert!(m.determinant() > 0.0);
    }

    #[test]
    fn round_trip_is_identity() {
        for sys in [AxisSystem::UNITY, AxisSystem::UNREAL, AxisSystem::BLENDER] {
            let there = AxisSystem::PIXEL3D.conversion_to(&sys).unwrap();
            let back = sys.conversion_to(&AxisSystem::PIXEL3D).unwrap();
            assert!((back * there).abs_diff_eq(Mat3::IDENTITY, 1e-6), "{sys:?}");
        }
    }

    #[test]
    fn invalid_systems_rejected() {
        let bad = AxisSystem { up: Axis::PosY, front: Axis::NegY, handedness: Handedness::Right, unit_meters: 1.0 };
        assert!(AxisSystem::PIXEL3D.conversion_to(&bad).is_none());
    }
}
