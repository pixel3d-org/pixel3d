//! Manifest migrations: bring older manifests up to the current version.

use serde_json::Value;

use super::manifest::FORMAT_VERSION;

/// Why a manifest cannot be migrated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrateError {
    #[error("manifest has no numeric format_version")]
    NoVersion,
    #[error("file format version {found} is newer than this pixel3d ({supported}); update pixel3d to open it")]
    TooNew { found: u64, supported: u32 },
}

/// Upgrades a manifest in place, one version step at a time.
pub fn migrate(mut manifest: Value) -> Result<Value, MigrateError> {
    let version = manifest.get("format_version").and_then(Value::as_u64).ok_or(MigrateError::NoVersion)?;
    if version > u64::from(FORMAT_VERSION) {
        return Err(MigrateError::TooNew { found: version, supported: FORMAT_VERSION });
    }
    // Version 1 is the first; future steps go here:
    //   if version < 2 { manifest = v1_to_v2(manifest); }
    if let Some(v) = manifest.get_mut("format_version") {
        *v = Value::from(FORMAT_VERSION);
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_version_passes_and_newer_is_refused() {
        assert!(migrate(json!({ "format_version": 1 })).is_ok());
        assert_eq!(migrate(json!({ "format_version": 99 })), Err(MigrateError::TooNew { found: 99, supported: FORMAT_VERSION }));
        assert_eq!(migrate(json!({})), Err(MigrateError::NoVersion));
    }
}
