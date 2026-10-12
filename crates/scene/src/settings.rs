//! Document settings: asset class and the defaults derived from it.

use serde::{Deserialize, Serialize};

/// Kind of asset being made; sets budgets and default detail (section 11.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    PropSmall,
    #[default]
    PropMedium,
    PropLarge,
    HeroProp,
    WeaponFp,
    Character,
    EnvironmentModular,
}

impl AssetClass {
    pub const ALL: [AssetClass; 7] = [
        AssetClass::PropSmall,
        AssetClass::PropMedium,
        AssetClass::PropLarge,
        AssetClass::HeroProp,
        AssetClass::WeaponFp,
        AssetClass::Character,
        AssetClass::EnvironmentModular,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AssetClass::PropSmall => "prop_small",
            AssetClass::PropMedium => "prop_medium",
            AssetClass::PropLarge => "prop_large",
            AssetClass::HeroProp => "hero_prop",
            AssetClass::WeaponFp => "weapon_fp",
            AssetClass::Character => "character",
            AssetClass::EnvironmentModular => "environment_modular",
        }
    }

    /// Upper end of the LOD0 triangle budget.
    pub fn tri_budget(self) -> u32 {
        match self {
            AssetClass::PropSmall => 1_500,
            AssetClass::PropMedium => 6_000,
            AssetClass::PropLarge => 25_000,
            AssetClass::HeroProp => 40_000,
            AssetClass::WeaponFp => 50_000,
            AssetClass::Character => 80_000,
            AssetClass::EnvironmentModular => 10_000,
        }
    }

    /// Default texel density in pixels per meter (lower end of the range).
    pub fn texel_density(self) -> f32 {
        match self {
            AssetClass::PropSmall | AssetClass::PropMedium | AssetClass::PropLarge | AssetClass::EnvironmentModular => 512.0,
            AssetClass::HeroProp | AssetClass::Character => 1024.0,
            AssetClass::WeaponFp => 2048.0,
        }
    }

    /// Default target edge length in meters for adaptive segment counts
    /// (`segments = clamp(round(circumference / target), 8, 64)`).
    pub fn target_edge_length(self) -> f32 {
        match self {
            AssetClass::PropSmall => 0.01,
            AssetClass::PropMedium => 0.04,
            AssetClass::PropLarge => 0.1,
            AssetClass::HeroProp => 0.02,
            AssetClass::WeaponFp => 0.005,
            AssetClass::Character => 0.02,
            AssetClass::EnvironmentModular => 0.1,
        }
    }
}

/// How lengths are displayed to people (internal units are always meters).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayUnits {
    #[default]
    Metric,
    Imperial,
}

/// Per-document settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DocSettings {
    pub asset_class: AssetClass,
    /// Mobile variant: about a third of the triangle budget, half the texel density.
    pub mobile: bool,
    /// Pixels per meter; `None` uses the asset class default.
    pub texel_density: Option<f32>,
    /// Meters; `None` uses the asset class default.
    pub target_edge_length: Option<f32>,
    pub display_units: DisplayUnits,
}

impl Default for DocSettings {
    fn default() -> Self {
        DocSettings {
            asset_class: AssetClass::default(),
            mobile: false,
            texel_density: None,
            target_edge_length: None,
            display_units: DisplayUnits::Metric,
        }
    }
}

impl DocSettings {
    pub fn tri_budget(&self) -> u32 {
        let b = self.asset_class.tri_budget();
        if self.mobile { b / 3 } else { b }
    }

    pub fn texel_density(&self) -> f32 {
        self.texel_density.unwrap_or_else(|| {
            let d = self.asset_class.texel_density();
            if self.mobile { d / 2.0 } else { d }
        })
    }

    pub fn target_edge_length(&self) -> f32 {
        self.target_edge_length.unwrap_or_else(|| self.asset_class.target_edge_length())
    }

    /// Adaptive segment count for a round shape: `clamp(round(circumference /
    /// target_edge_length), 8, 64)`, rounded up to a multiple of `multiple`.
    pub fn round_segments(&self, circumference: f64, multiple: u32) -> u32 {
        let raw = (circumference / f64::from(self.target_edge_length())).round().clamp(8.0, 64.0) as u32;
        let m = multiple.max(1);
        raw.div_ceil(m) * m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_mobile() {
        let mut s = DocSettings::default();
        assert_eq!(s.asset_class, AssetClass::PropMedium);
        assert_eq!(s.tri_budget(), 6_000);
        s.mobile = true;
        assert_eq!((s.tri_budget(), s.texel_density()), (2_000, 256.0));
        s.texel_density = Some(700.0);
        assert_eq!(s.texel_density(), 700.0);
    }

    #[test]
    fn adaptive_segments() {
        let s = DocSettings::default();
        // 0.5 m radius: circumference 3.14 m / 0.04 m = 79 -> clamped to 64.
        assert_eq!(s.round_segments(std::f64::consts::PI, 4), 64);
        // 5 cm radius: 0.314 / 0.04 = 8.
        assert_eq!(s.round_segments(0.314, 4), 8);
        assert_eq!(s.round_segments(0.5, 4), 16);
        assert_eq!(s.round_segments(0.01, 1), 8);
    }

    #[test]
    fn serde_is_snake_case_and_strict() {
        let json = serde_json::to_string(&DocSettings::default()).unwrap();
        assert!(json.contains("\"prop_medium\""), "{json}");
        let back: DocSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, DocSettings::default());
        assert!(serde_json::from_str::<DocSettings>(r#"{"asset_klass":"prop_small"}"#).is_err());
        for c in AssetClass::ALL {
            assert_eq!(serde_json::to_value(c).unwrap(), serde_json::Value::String(c.as_str().into()));
        }
    }
}
