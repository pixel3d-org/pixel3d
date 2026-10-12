//! Tool arguments. Every schema is generated from these types and strict:
//! unknown fields are errors with "did you mean" suggestions.

use p3d_ops::expect::ExpectArgs;
use p3d_ops::params::Length;
use p3d_perceive::probe::SurfacePoint;
use p3d_perceive::section::SliceAxis;
use p3d_perceive::silhouette::ViewDir;
use p3d_query::Selector;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How much `scene_overview` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// Parts, sizes, contacts, problems and tests in a few lines.
    #[default]
    Brief,
    /// Adds groups, tags and every relation.
    Full,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneOverviewArgs {
    /// `brief` (default) or `full`.
    #[serde(default)]
    pub detail: Detail,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeArgs {
    /// Object (name, path such as `crate/lid`, or id such as `o:3`).
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryArgs {
    /// Mesh object (name, path or id).
    pub target: String,
    /// Elements by meaning: `{"faces": {"group": "top"}}`, `{"faces": {"normal": {"dir": [0, 1, 0], "within_deg": 10}}}`,
    /// `{"edges": {"boundary": true}}`, `{"verts": {"in_box": {"min": [..], "max": [..]}}}`, `and`, `or`, `not`, `grow`.
    pub select: Selector,
    /// Store the result as a group with this name (one undoable step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_as: Option<String>,
}

/// One op call of a `run` batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunOp {
    /// Op id, for example `prim.box` (`list_ops` shows them all).
    pub op: String,
    /// Op parameters (`describe_op` shows the schema); omitted means defaults.
    #[serde(default)]
    pub args: Value,
    /// Label for the history and for created groups (`<label>.<role>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunArgs {
    /// Ops applied in order as one transaction: all succeed or nothing changes.
    pub ops: Vec<RunOp>,
    /// Report what would happen, then discard it.
    #[serde(default)]
    pub dry_run: bool,
    /// Name the resulting state (for `history revert` and `history diff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    /// Undo label; the op labels when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListOpsArgs {
    /// Only this category: scene, prim, select, edit, expect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Words to find in ids, labels, aliases (Blender and Maya names) and summaries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeOpArgs {
    /// Op id or alias, for example `prim.box` or `cube`.
    pub op: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    /// Undo transactions and checkpoints.
    #[default]
    List,
    Undo,
    Redo,
    /// Name the current state.
    Checkpoint,
    /// Return to a checkpoint (undoable).
    Revert,
    /// What changed between two states (`from`, `to`).
    Diff,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryArgs {
    /// `list` (default), `undo`, `redo`, `checkpoint`, `revert` or `diff`.
    #[serde(default)]
    pub action: HistoryAction,
    /// Checkpoint name for `checkpoint` and `revert`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `diff` base: a revision number or a checkpoint name; the previous revision by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `diff` end: a revision number or a checkpoint name; the current state by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// `diff` only this object and its children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Steps for `undo` and `redo` (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocsArgs {
    /// Guide name (`quickstart`, `modeling`, `debugging`, ...); the index when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aspect {
    /// What is where: parts, sizes, positions, groups, contacts, mirror pairs.
    #[default]
    Structure,
    /// Per part: best-fit primitive, axes, volume, thickness, edges, symmetry.
    Shape,
    /// Outline from a view as a short polyline labelled by part.
    Silhouette,
    /// Loops of a planar slice, with wall thickness.
    Section,
    /// Contacts, gaps, penetrations, floating parts.
    Relations,
    /// Everything at one surface point (needs `at`).
    Probe,
    /// What changed since `from` (a revision or checkpoint; the previous revision by default).
    Diff,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PerceiveArgs {
    /// Object to look at with its children; the whole scene when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// What to perceive.
    #[serde(default)]
    pub aspect: Aspect,
    /// `silhouette`: front (default), back, left, right, top, bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewDir>,
    /// `section`: slice across x, y (default, horizontal) or z.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<SliceAxis>,
    /// `section`: plane position; the middle of the target when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Length>,
    /// `silhouette` and `section`: outline points (default 32 and 48).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
    /// `probe`: the surface point, e.g. `{"on": "crate", "landmark": "top"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<SurfacePoint>,
    /// `diff`: base revision number or checkpoint name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnoseArgs {
    /// Object to check with its children; the whole scene when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Finding id (`F3`) to explain: every fix option with its predicted result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<String>,
    /// Finding id (`F3`) to fix now (one undoable step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Fix option for `fix` (0 is the first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExpectAction {
    /// Run every expectation.
    #[default]
    Check,
    List,
    Add,
    Remove,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpectToolArgs {
    /// `check` (default), `list`, `add` or `remove`.
    #[serde(default)]
    pub action: ExpectAction,
    /// `add`: optional name; `remove`: the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `add`: the test, e.g. `{"attached": {"part": "lid", "to": "crate"}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<ExpectArgs>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeArgs {
    /// Object to measure with its children; the whole scene when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
