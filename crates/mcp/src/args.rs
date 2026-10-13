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
pub enum BriefAction {
    /// How well the model answers the brief (default).
    #[default]
    Coverage,
    Set,
    Get,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BriefArgs {
    /// `coverage` (default), `set` or `get`.
    #[serde(default)]
    pub action: BriefAction,
    /// The brief's name; a spec answers it with `"brief": "<name>"`.
    pub name: String,
    /// `set`: the brief: title, category, style, size, budget, must_have, must_not (see the spec guide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpecAction {
    /// The spec source and its parts (default).
    #[default]
    Get,
    Set,
    Patch,
    Compile,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpecArgs {
    /// `get` (default), `set`, `patch` or `compile`.
    #[serde(default)]
    pub action: SpecAction,
    /// The spec's root object (`set` creates it).
    pub target: String,
    /// `set`: the Model Spec `{"parts": {...}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Value>,
    /// `patch`: a JSON Merge Patch (`null` removes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<Value>,
    /// `patch`: JSON Patch operations instead of a merge patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ops: Option<Vec<p3d_spec::patch::PatchOp>>,
    /// `compile`: rebuild every part.
    #[serde(default)]
    pub force: bool,
    /// Preview the result without changing anything.
    #[serde(default)]
    pub dry_run: bool,
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

/// A camera view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenderView {
    /// From +Z (the asset's front).
    Front,
    Back,
    /// From +X.
    Right,
    Left,
    /// From above, front at the bottom.
    Top,
    Bottom,
    /// From the front, right and above.
    ThreeQuarter,
}

/// How surfaces look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    /// Lit, one color per part; inside-out faces show red.
    #[default]
    Shaded,
    /// Hidden-line wireframe of the polygon edges.
    Wire,
    /// Shaded with polygon edges.
    ShadedWire,
    /// Gray clay: reads form and smoothness.
    Matcap,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderArgs {
    /// Object with its children; the whole scene when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Views (default front, right, top, three_quarter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<Vec<RenderView>>,
    /// `shaded` (default), `wire`, `shaded_wire` or `matcap`.
    #[serde(default)]
    pub mode: RenderMode,
    /// Pixels per view side, 64 to 2048 (default 512).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// One contact sheet image instead of one image per view (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<bool>,
    /// Draw part names next to the parts (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiScreenshotArgs {
    /// Only the 3D viewport instead of the whole window.
    #[serde(default)]
    pub viewport_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiStateArgs {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiCommandArgs {
    /// Command id, for example `ui.mode`, `view.frame` or `file.save` (an
    /// op id also works); omit it to list the commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Command parameters, for example `{"mode": "face"}` for `ui.mode`.
    #[serde(default)]
    pub params: Value,
}
