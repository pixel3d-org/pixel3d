//! The Op trait, its metadata and its output.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use p3d_mesh::ElementKind;
use p3d_scene::{Document, ObjectKey};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::error::{ErrorCode, OpError};
use crate::tx::Tx;

/// An operation: the unit of every action in pixel3d. The same op serves the
/// GUI, CLI, MCP, scripts, addons and the node graph.
pub trait Op: Send + Sync + 'static {
    /// Stable id `category.name`, for example `prim.box`.
    const ID: &'static str;
    /// Parameters: strict JSON (unknown fields are errors), every field
    /// documented (doc comments become schema descriptions).
    type Params: DeserializeOwned + Serialize + JsonSchema + Default;

    fn meta() -> OpMeta;
    /// Cheap checks against the document, no mutation.
    fn validate(doc: &Document, p: &Self::Params) -> Result<(), OpError>;
    /// Mutates the transaction's working copy. Must be deterministic.
    fn apply(tx: &mut Tx, p: &Self::Params, ctx: &OpCtx) -> Result<OpOutput, OpError>;
}

/// Op families, used for discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Scene,
    Prim,
    Select,
    Edit,
    Expect,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Scene => "scene",
            Category::Prim => "prim",
            Category::Select => "select",
            Category::Edit => "edit",
            Category::Expect => "expect",
        }
    }
}

/// An example call: arguments and the expected one-line result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpExample {
    pub args: &'static str,
    pub result: &'static str,
}

/// Op behaviour flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OpFlags(u8);

impl OpFlags {
    pub const NONE: OpFlags = OpFlags(0);
    pub const UNDOABLE: OpFlags = OpFlags(1);
    pub const LONG_RUNNING: OpFlags = OpFlags(1 << 1);
    pub const PREVIEWABLE: OpFlags = OpFlags(1 << 2);
    pub const CREATES_GROUPS: OpFlags = OpFlags(1 << 3);
    pub const CREATES_OBJECTS: OpFlags = OpFlags(1 << 4);

    pub const fn union(self, other: OpFlags) -> OpFlags {
        OpFlags(self.0 | other.0)
    }

    pub fn contains(self, other: OpFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Flag names for docs and schemas.
    pub fn names(self) -> Vec<&'static str> {
        [
            (Self::UNDOABLE, "undoable"),
            (Self::LONG_RUNNING, "long_running"),
            (Self::PREVIEWABLE, "previewable"),
            (Self::CREATES_GROUPS, "creates_groups"),
            (Self::CREATES_OBJECTS, "creates_objects"),
        ]
        .into_iter()
        .filter(|(f, _)| self.contains(*f))
        .map(|(_, n)| n)
        .collect()
    }
}

/// Everything people and agents need to find and use an op.
#[derive(Debug, Clone, PartialEq)]
pub struct OpMeta {
    /// Short label, for example "Box".
    pub label: &'static str,
    pub category: Category,
    /// One sentence written for agents.
    pub summary: &'static str,
    /// Markdown: what it does, when to use it, when not to, typical values.
    pub docs: &'static str,
    pub examples: &'static [OpExample],
    /// Names other tools use (Blender, Maya, 3ds Max).
    pub aliases: &'static [&'static str],
    /// For example "O(n) in selected faces".
    pub complexity: &'static str,
    pub flags: OpFlags,
    /// Bump when semantics change (part of graph cache keys).
    pub version: u16,
}

/// Seed, cancellation and progress for one op run.
#[derive(Debug, Default)]
pub struct OpCtx<'a> {
    pub seed: u64,
    /// The call's explicit label, if any: groups an op creates are named
    /// `<label>.<role>`.
    pub label: Option<&'a str>,
    pub cancel: Option<&'a AtomicBool>,
    /// Progress in millionths (0 to 1,000,000).
    pub progress: Option<&'a AtomicU32>,
}

impl OpCtx<'_> {
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// `CANCELLED` if cancellation was requested.
    pub fn check_cancel(&self) -> Result<(), OpError> {
        if self.is_cancelled() { Err(OpError::new(ErrorCode::Cancelled, "cancelled").retryable()) } else { Ok(()) }
    }

    /// Reports progress as a fraction in [0, 1].
    pub fn report(&self, fraction: f32) {
        if let Some(p) = self.progress {
            p.store((fraction.clamp(0.0, 1.0) * 1e6) as u32, Ordering::Relaxed);
        }
    }
}

/// A group an op created or changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInfo {
    pub object: ObjectKey,
    pub name: String,
    pub kind: ElementKind,
    pub count: u32,
}

/// A ready-to-run op call (for warnings with a fix).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpCall {
    pub op: String,
    pub args: serde_json::Value,
}

/// Something the caller should know, ideally with a fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Warning {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<OpCall>,
}

/// Changes in totals over the objects an op touched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsDelta {
    pub objects: i64,
    pub verts: i64,
    pub faces: i64,
    pub tris: i64,
}

/// A file or image an op produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub kind: String,
    pub path: String,
}

/// What an op did. Never lists more than a summary's worth of elements:
/// counts, names and groups, not element indices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpOutput {
    /// One line, for example "crate: +24 faces (+48 tris)".
    pub summary: String,
    /// Objects whose data changed (their history gets this op).
    pub changed: SmallVec<[ObjectKey; 4]>,
    /// Objects the op created (also recorded in their history).
    pub created: SmallVec<[ObjectKey; 2]>,
    /// Public ids of objects the op deleted.
    pub deleted: Vec<u32>,
    pub groups: Vec<GroupInfo>,
    /// Filled by the engine from before/after totals.
    pub metrics: MetricsDelta,
    pub warnings: Vec<Warning>,
    pub artifacts: Vec<Artifact>,
}

impl OpOutput {
    pub fn summary(s: impl Into<String>) -> Self {
        OpOutput { summary: s.into(), ..OpOutput::default() }
    }
}
