//! `manifest.json`: everything but mesh data, in a stable, readable order.

use std::collections::BTreeMap;

use p3d_scene::{DocSettings, Expectation, OpNode, SpecRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Value of the `format` field.
pub const FORMAT: &str = "pixel3d";
/// Current `format_version`.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    #[serde(default)]
    pub settings: DocSettings,
    /// Next object id: ids are never reused.
    pub next_id: u32,
    /// Meshes in first-use order; several may share one blob (copies).
    #[serde(default)]
    pub meshes: Vec<MeshEntry>,
    /// Objects with parents before children, siblings in order.
    #[serde(default)]
    pub objects: Vec<ObjectEntry>,
    /// Expectations (model tests) in the order they were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expectations: Vec<Expectation>,
    /// Briefs (structured intent) by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub briefs: BTreeMap<String, Value>,
    /// Fields this version does not know; written back unchanged.
    #[serde(flatten)]
    pub extras: BTreeMap<String, Value>,
}

/// A mesh: its file-local id and its content-addressed blob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshEntry {
    pub id: u32,
    /// blake3 hex of the blob bytes; the blob is `blobs/<blob>.bin`.
    pub blob: String,
    pub verts: u32,
    pub faces: u32,
    pub tris: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformEntry {
    pub translation: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectEntry {
    pub id: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u32>,
    /// `empty`, `mesh` or `curve`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<u32>,
    /// The curve of a curve object, inline (curves are small).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<p3d_scene::Curve>,
    pub transform: TransformEntry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<OpNode>,
    /// The Model Spec rooted here, with its last compile record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<SpecRecord>,
}

fn yes() -> bool {
    true
}
