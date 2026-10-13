//! The agent surface without a transport: tool calls in (name and JSON
//! arguments), text for the model plus structured JSON out. The MCP server
//! and the tests drive the same [`Agent`].

use std::sync::{Arc, Weak};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use p3d_engine::predict::{apply_fix, diagnose_predicted, explain, op_error};
use p3d_engine::{BatchOptions, OpCall, Session};
use p3d_ops::catalog::catalog_json;
use p3d_ops::{ErrorCode, OpError, parse_params};
use p3d_perceive::diagnose::{Finding, diagnose, findings_text};
use p3d_perceive::fmt::{dims, point, thousands};
use p3d_perceive::section::SectionOptions;
use p3d_perceive::silhouette::SilhouetteOptions;
use p3d_perceive::{Report, View, expect as checks};
use p3d_scene::Document;
use serde_json::{Value, json};

use crate::args::*;
use crate::describe::{describe, overview};
use crate::docs;

/// A tool's answer: text for the model, the same data as JSON, and images.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub structured: Value,
    /// PNG images (renders).
    pub images: Vec<Vec<u8>>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(text: String, structured: Value) -> Self {
        ToolOutput { text, structured, images: Vec::new(), is_error: false }
    }

    pub fn error(e: &OpError) -> Self {
        ToolOutput { text: e.to_string(), structured: json!({ "error": e }), images: Vec::new(), is_error: true }
    }

    fn report(r: Report) -> Self {
        Self::ok(r.text, json!({ "aspect": r.aspect, "data": r.data }))
    }

    /// The output as JSON for another process (the desktop app answering
    /// `mcp.call`), images as base64 PNG.
    ///
    /// Complexity: O(text and image bytes).
    pub fn to_wire(&self) -> Value {
        let images: Vec<String> = self.images.iter().map(|png| BASE64.encode(png)).collect();
        json!({ "text": self.text, "structured": self.structured, "images": images, "isError": self.is_error })
    }

    /// Reads [`ToolOutput::to_wire`]; `None` when `v` is not one.
    ///
    /// Complexity: O(text and image bytes).
    pub fn from_wire(v: &Value) -> Option<ToolOutput> {
        let text = v.get("text")?.as_str()?.to_owned();
        let images = match v.get("images") {
            None => Vec::new(),
            Some(list) => list.as_array()?.iter().map(|s| s.as_str().and_then(|s| BASE64.decode(s).ok())).collect::<Option<Vec<_>>>()?,
        };
        let structured = v.get("structured").cloned().unwrap_or(Value::Null);
        Some(ToolOutput { text, structured, images, is_error: v.get("isError").and_then(Value::as_bool).unwrap_or(false) })
    }
}

/// A tool: name, one-sentence description and strict input schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

pub(crate) fn schema<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// Every tool, in the order clients list them.
pub fn tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "scene_overview",
            description: "What is in the scene: parts with sizes and positions, contacts, problems and test results, in a few lines.",
            schema: schema::<SceneOverviewArgs>(),
        },
        ToolInfo {
            name: "describe",
            description: "One-screen summary of an object: size, origin, parts, shape, symmetry, topology, groups, problems.",
            schema: schema::<DescribeArgs>(),
        },
        ToolInfo {
            name: "run",
            description: "Run ops as one undoable transaction (all or nothing); returns per-op results, created groups, warnings, test results.",
            schema: schema::<RunArgs>(),
        },
        ToolInfo {
            name: "perceive",
            description: "Understand the model without images: structure, shape, silhouette, section, relations, probe, diff, visibility, proportions, brief or dna, as compact text.",
            schema: schema::<PerceiveArgs>(),
        },
        ToolInfo {
            name: "diagnose",
            description: "Problems like compiler errors (part, place, cause) with fixes and their predicted results; explain or apply a fix.",
            schema: schema::<DiagnoseArgs>(),
        },
        ToolInfo {
            name: "expect",
            description: "Tests for the model (dims, symmetric, attached, not_intersecting, budget, watertight): add, remove, list, check.",
            schema: schema::<ExpectToolArgs>(),
        },
        ToolInfo {
            name: "brief",
            description: "Capture what the user asked for (category, style, size, budget, must-haves) as checks, and see how well the model answers it.",
            schema: schema::<BriefArgs>(),
        },
        ToolInfo {
            name: "spec",
            description: "Author the model as a Model Spec (named parts, shapes, anchors at landmarks, joins): set, patch or compile; returns parts, brief coverage, problems and tests.",
            schema: schema::<SpecArgs>(),
        },
        ToolInfo {
            name: "deform",
            description: "Sculpt by numbers: move named points (landmarks, shapes, groups) or coordinates by offsets with pins held; spec parts keep the handles across recompiles.",
            schema: schema::<DeformArgs>(),
        },
        ToolInfo {
            name: "query",
            description: "Count and locate mesh elements picked by a selector (groups, normals, boundaries, boxes); optionally save them as a group.",
            schema: schema::<QueryArgs>(),
        },
        ToolInfo {
            name: "analyze",
            description: "Topology metrics per part: counts, face kinds, poles, angles, holes, pieces, degenerate elements, volume.",
            schema: schema::<AnalyzeArgs>(),
        },
        ToolInfo {
            name: "render",
            description: "Optional pictures to confirm what text already says: views tiled into one PNG with part names drawn on.",
            schema: schema::<RenderArgs>(),
        },
        ToolInfo {
            name: "list_ops",
            description: "List ops with one-line summaries; search matches ids, labels, summaries and Blender or Maya names.",
            schema: schema::<ListOpsArgs>(),
        },
        ToolInfo {
            name: "describe_op",
            description: "One op in full: what it does, parameters with schema and defaults, examples, aliases.",
            schema: schema::<DescribeOpArgs>(),
        },
        ToolInfo { name: "history", description: "Undo, redo, checkpoints, revert and diffs between states.", schema: schema::<HistoryArgs>() },
        ToolInfo {
            name: "import",
            description: "Bring a glTF, GLB, OBJ or FBX file into the scene in meters with +Y up, repairing its meshes; the path must be inside the allowed folders.",
            schema: schema::<p3d_ops::io::ImportParams>(),
        },
        ToolInfo {
            name: "docs",
            description: "Read the guides: quickstart, modeling, debugging and the op reference.",
            schema: schema::<DocsArgs>(),
        },
    ]
}

/// Findings of the latest revision, cached: diagnose costs up to about
/// 100 ms on large scenes and several tools need it.
struct FindingsCache {
    revision: u64,
    /// The document itself: a host that loads another file can reuse a
    /// revision number. The weak pointer keeps the address from being reused.
    doc: Weak<Document>,
    target: Option<String>,
    findings: Arc<Vec<Finding>>,
}

/// The agent surface: one session and the caches tools share.
pub struct Agent {
    session: Session,
    cache: Option<FindingsCache>,
    /// Created on the first render (it takes the GPU), kept after.
    renderer: Option<Result<p3d_render::Renderer, p3d_render::RenderError>>,
}

impl Agent {
    pub fn new(session: Session) -> Agent {
        Agent { session, cache: None, renderer: None }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Exchanges the agent's session with `other`: a host that owns its
    /// session (the desktop app) lends it for one call and takes it back.
    /// The caches stay valid: they are keyed by the document.
    pub fn swap_session(&mut self, other: &mut Session) {
        std::mem::swap(&mut self.session, other);
    }

    /// Calls a tool. Errors come back as an error output, never a panic.
    pub fn call(&mut self, tool: &str, args: Value) -> ToolOutput {
        let span = tracing::info_span!("tool", %tool);
        let _g = span.enter();
        let result = match tool {
            "scene_overview" => parse_params(&args).and_then(|a| self.scene_overview(&a)),
            "describe" => parse_params(&args).and_then(|a| self.describe(&a)),
            "run" => parse_params(&args).and_then(|a| self.run(&a)),
            "perceive" => parse_params(&args).and_then(|a| self.perceive(&a)),
            "diagnose" => parse_params(&args).and_then(|a| self.diagnose(&a)),
            "expect" => parse_params(&args).and_then(|a| self.expect(&a)),
            "brief" => parse_params(&args).and_then(|a| self.brief(&a)),
            "spec" => parse_params(&args).and_then(|a| self.spec(&a)),
            "deform" => parse_params(&args).and_then(|a| self.deform(&a)),
            "query" => parse_params(&args).and_then(|a| self.query(&a)),
            "analyze" => parse_params(&args).and_then(|a| self.analyze(&a)),
            "list_ops" => parse_params(&args).and_then(|a| self.list_ops(&a)),
            "describe_op" => parse_params(&args).and_then(|a| self.describe_op(&a)),
            "history" => parse_params(&args).and_then(|a| self.history(&a)),
            // A shortcut for `run` with one `io.import` op.
            "import" => parse_params::<p3d_ops::io::ImportParams>(&args)
                .and_then(|_| parse_params(&serde_json::json!({ "ops": [{ "op": "io.import", "args": args }] })))
                .and_then(|a| self.run(&a)),
            "render" => parse_params(&args).and_then(|a| self.render(&a)),
            "docs" => parse_params(&args).and_then(|a| docs::tool(&a)),
            other => Err(OpError::new(ErrorCode::UnknownOp, format!("no tool `{other}`")).with_suggestions(p3d_scene::name::suggest(
                other,
                tools().iter().map(|t| t.name),
                3,
            ))),
        };
        result.unwrap_or_else(|e| ToolOutput::error(&e))
    }

    fn doc(&self) -> Arc<Document> {
        Arc::clone(self.session.doc())
    }

    /// Findings of the current revision for a target (cached).
    fn findings(&mut self, target: Option<&str>) -> Result<Arc<Vec<Finding>>, OpError> {
        let rev = self.session.revision();
        let doc = self.doc();
        if let Some(c) = &self.cache
            && c.revision == rev
            && std::ptr::eq(c.doc.as_ptr(), Arc::as_ptr(&doc))
            && c.target.as_deref() == target
        {
            return Ok(Arc::clone(&c.findings));
        }
        let view = View::new(&doc, target).map_err(op_error)?;
        let f = Arc::new(diagnose(&view));
        let doc = Arc::downgrade(&doc);
        self.cache = Some(FindingsCache { revision: rev, doc, target: target.map(str::to_owned), findings: Arc::clone(&f) });
        Ok(f)
    }

    /// `problems: 1 error, 2 warnings (diagnose for details)`.
    fn problems_line(&mut self) -> String {
        let Ok(f) = self.findings(None) else { return String::new() };
        if f.is_empty() {
            return "problems: none".into();
        }
        let n = |s| f.iter().filter(|x| x.severity == s).count();
        use p3d_perceive::diagnose::Severity::*;
        let (e, w, i) = (n(Error), n(Warn), n(Info));
        let codes: Vec<String> = f.iter().filter(|x| x.severity != Info).take(3).map(|x| format!("{} {}", x.code, x.part)).collect();
        let mut s = format!("problems: {e} error{}, {w} warning{}, {i} info", if e == 1 { "" } else { "s" }, if w == 1 { "" } else { "s" });
        if !codes.is_empty() {
            s.push_str(&format!(" ({}; diagnose for fixes)", codes.join(", ")));
        }
        s
    }

    /// Appends the problems and test lines every state-changing answer ends with.
    fn finish(&mut self, mut text: String) -> String {
        let p = self.problems_line();
        if !p.is_empty() {
            text.push_str(&p);
            text.push('\n');
        }
        // Every report carries the coverage of each brief (project spec 17.2).
        let doc = self.doc();
        for (name, _) in doc.briefs().take(2) {
            if let Ok(c) = p3d_spec::coverage(&doc, name) {
                text.push_str(&c.line());
                text.push('\n');
            }
        }
        let t = checks::tests_line(&checks::check_all(self.session.doc()));
        if !t.is_empty() {
            text.push_str(&t);
            text.push('\n');
        }
        text
    }

    fn scene_overview(&mut self, a: &SceneOverviewArgs) -> Result<ToolOutput, OpError> {
        let doc = self.doc();
        let (text, data) = overview(&doc, self.session.revision(), a.detail == Detail::Full);
        Ok(ToolOutput::ok(self.finish(text), data))
    }

    fn describe(&mut self, a: &DescribeArgs) -> Result<ToolOutput, OpError> {
        let doc = self.doc();
        let (text, data) = describe(&doc, &a.target).map_err(op_error)?;
        Ok(ToolOutput::ok(self.finish(text), data))
    }

    fn execute(&mut self, calls: &[OpCall], opts: &BatchOptions) -> Result<p3d_engine::BatchResult, OpError> {
        self.session.execute(calls, opts).map_err(|e| *e.error)
    }

    fn run(&mut self, a: &RunArgs) -> Result<ToolOutput, OpError> {
        if a.ops.is_empty() {
            return Err(OpError::invalid("/ops", "give at least one op").with_hint("list_ops shows them; describe_op shows one op's parameters"));
        }
        let calls: Vec<OpCall> = a.ops.iter().map(|o| OpCall { op: o.op.clone(), args: o.args.clone(), label: o.label.clone() }).collect();
        let opts = BatchOptions { dry_run: a.dry_run, checkpoint: a.checkpoint.clone(), label: a.label.clone() };
        let r = self.execute(&calls, &opts)?;
        let ms = r.elapsed.as_secs_f64() * 1000.0;
        let n = r.results.len();
        let mut t = if r.dry_run {
            format!("dry run: nothing changed, still rev {} ({n} op{}, {ms:.1} ms)\n", r.revision, if n == 1 { "" } else { "s" })
        } else {
            format!("ok rev {} ({n} op{}, {ms:.1} ms)\n", r.revision, if n == 1 { "" } else { "s" })
        };
        let doc = self.doc();
        let mut ops = Vec::new();
        for res in &r.results {
            t.push_str(&format!("{}: {}\n", res.op, res.output.summary));
            let groups: Vec<String> = res.output.groups.iter().map(|g| format!("{} ({})", g.name, count_of(g.count as usize, g.kind))).collect();
            if !groups.is_empty() {
                t.push_str(&format!("  groups: {}\n", groups.join(", ")));
            }
            for w in &res.output.warnings {
                let fix = w.fix.as_ref().map_or(String::new(), |f| format!(" (fix: {} {})", f.op, f.args));
                t.push_str(&format!("  warning: {}{fix}\n", w.message));
            }
            ops.push(json!({
                "op": res.op,
                "label": res.label,
                "summary": res.output.summary,
                "params": res.params,
                "created": res.output.created.iter().filter_map(|k| doc.object(*k).map(|o| o.public_id())).collect::<Vec<_>>(),
                "groups": res.output.groups.iter().map(|g| json!({ "name": g.name, "kind": g.kind.as_str(), "count": g.count })).collect::<Vec<_>>(),
                "warnings": res.output.warnings,
                "metrics": res.output.metrics,
                "ms": (res.elapsed.as_secs_f64() * 1e4).round() / 10.0,
            }));
        }
        if let Some(c) = &a.checkpoint
            && !r.dry_run
        {
            t.push_str(&format!("checkpoint {c} saved\n"));
        }
        let text = if r.dry_run { t } else { self.finish(t) };
        Ok(ToolOutput::ok(text, json!({ "revision": r.revision, "dry_run": r.dry_run, "ops": ops })))
    }

    fn perceive(&mut self, a: &PerceiveArgs) -> Result<ToolOutput, OpError> {
        use p3d_perceive::{probe, relations, section, shape, silhouette, structure};
        let doc = self.doc();
        let target = a.target.as_deref();
        let report = match a.aspect {
            Aspect::Diff => return self.diff(a.from.as_deref(), None, target),
            Aspect::Proportions => {
                let root = doc.resolve(target.unwrap_or("")).map_err(|e| OpError::from_scene(e, "/target"))?;
                let (text, data) = p3d_spec::proportions(&doc, root)?;
                return Ok(ToolOutput::ok(text, data));
            }
            Aspect::Brief => {
                // The brief of the target's spec, else the only brief.
                let from_spec = target
                    .and_then(|t| doc.resolve(t).ok())
                    .and_then(|k| doc.spec(k))
                    .and_then(|r| p3d_spec::parse(&r.source).ok())
                    .and_then(|s| s.brief);
                let names: Vec<String> = doc.briefs().map(|(n, _)| n.to_owned()).collect();
                let name = from_spec.or_else(|| (names.len() == 1).then(|| names[0].clone())).ok_or_else(|| {
                    let hint = if names.is_empty() { "brief set creates one".to_owned() } else { format!("briefs: {}", names.join(", ")) };
                    OpError::invalid("/target", "say which spec: its brief is measured").with_hint(&hint)
                })?;
                return self.brief(&BriefArgs { action: BriefAction::Coverage, name, brief: None });
            }
            aspect => {
                let view = View::new(&doc, target).map_err(op_error)?;
                match aspect {
                    Aspect::Structure => structure::report(&view),
                    Aspect::Shape => shape::report(&view),
                    Aspect::Relations => relations::report(&view),
                    Aspect::Silhouette => {
                        let mut o = SilhouetteOptions::default();
                        if let Some(v) = a.view {
                            o.view = v;
                        }
                        if let Some(p) = a.points {
                            o.points = p;
                        }
                        silhouette::report(&view, &o).map_err(op_error)?
                    }
                    Aspect::Section => {
                        let mut o = SectionOptions { at: a.at.map(|l| l.0 as f32), ..SectionOptions::default() };
                        if let Some(x) = a.axis {
                            o.axis = x;
                        }
                        if let Some(p) = a.points {
                            o.points = p;
                        }
                        section::report(&view, &o).map_err(op_error)?
                    }
                    Aspect::Probe => {
                        let at = a.point.as_ref().ok_or_else(|| {
                            OpError::invalid("/point", "probe needs a point").with_hint("for example {\"on\": \"crate\", \"landmark\": \"top\"}")
                        })?;
                        probe::report(&view, at).map_err(|e| {
                            let mut o = op_error(e);
                            o.path = o.path.map(|p| p.replacen("/at", "/point", 1));
                            o
                        })?
                    }
                    Aspect::Visibility => p3d_perceive::visibility::report(&view),
                    Aspect::Dna => {
                        let genes = a
                            .dna
                            .clone()
                            .or_else(|| target.and_then(|t| doc.resolve(t).ok()).and_then(|k| p3d_spec::dna_of(&doc, k)))
                            .unwrap_or_default();
                        p3d_perceive::dna::report(&view, &genes).map_err(|(path, message)| OpError::invalid(&format!("/dna{path}"), message))?
                    }
                    Aspect::Diff | Aspect::Proportions | Aspect::Brief => return Err(OpError::internal("unreachable")),
                }
            }
        };
        Ok(ToolOutput::report(report))
    }

    /// The document at `rev` (a revision number or a checkpoint name).
    fn state(&self, rev: &str, path: &str) -> Result<Arc<Document>, OpError> {
        let found = match rev.trim_start_matches("rev").trim().parse::<u64>() {
            Ok(n) => self.session.doc_at(n),
            Err(_) => self.session.checkpoint_doc(rev),
        };
        found.ok_or_else(|| {
            let mut names: Vec<String> = self.session.checkpoints().map(str::to_owned).collect();
            names.extend(self.session.history().iter().map(|h| h.revision.to_string()));
            OpError::new(ErrorCode::NotFound, format!("no state `{rev}` (revisions still in the history, or checkpoint names)"))
                .at(path)
                .with_suggestions(names.into_iter().rev().take(5).collect())
        })
    }

    fn diff(&mut self, from: Option<&str>, to: Option<&str>, target: Option<&str>) -> Result<ToolOutput, OpError> {
        let after = match to {
            Some(t) => self.state(t, "/to")?,
            None => self.doc(),
        };
        let before = match from {
            Some(f) => self.state(f, "/from")?,
            None => {
                let prev = after.revision.saturating_sub(1);
                self.session
                    .doc_at(prev)
                    .ok_or_else(|| OpError::new(ErrorCode::NotFound, format!("revision {prev} is no longer in the history")).at("/from"))?
            }
        };
        let r = p3d_perceive::diff::report(&before, &after, target).map_err(op_error)?;
        Ok(ToolOutput::report(r))
    }

    fn diagnose(&mut self, a: &DiagnoseArgs) -> Result<ToolOutput, OpError> {
        let target = a.target.as_deref();
        let registry = Arc::clone(self.session.registry());
        let seed = self.session.seed();
        if let Some(id) = &a.fix {
            let (f, r) = apply_fix(&mut self.session, target, id, a.option.unwrap_or(0))?;
            let text = format!(
                "fixed {} {} {}: rev {} ({})\n",
                f.id,
                f.code,
                f.part,
                r.revision,
                r.results.first().map_or("", |x| x.output.summary.as_str())
            );
            return Ok(ToolOutput::ok(self.finish(text), json!({ "fixed": f, "revision": r.revision })));
        }
        let doc = self.doc();
        if let Some(id) = &a.explain {
            let f = explain(&registry, &doc, target, seed, id)?;
            let mut text = findings_text(&format!("{} (explain {})", target.unwrap_or("scene"), f.id), std::slice::from_ref(&f));
            if f.fixes.is_empty() {
                text.push_str("no fix option yet\n");
            }
            return Ok(ToolOutput::ok(text, json!({ "finding": f })));
        }
        let f = diagnose_predicted(&registry, &doc, target, seed).map_err(op_error)?;
        let title = target.unwrap_or("scene").to_owned();
        Ok(ToolOutput::ok(findings_text(&title, &f), json!({ "findings": f })))
    }

    fn expect(&mut self, a: &ExpectToolArgs) -> Result<ToolOutput, OpError> {
        match a.action {
            ExpectAction::Check => Ok(ToolOutput::report(checks::report(self.session.doc()))),
            ExpectAction::List => {
                let doc = self.doc();
                let mut t = String::new();
                if doc.expectations().is_empty() {
                    t.push_str("no expectations\n");
                }
                for e in doc.expectations() {
                    t.push_str(&format!("{} {}\n", e.name, e.check.label()));
                }
                Ok(ToolOutput::ok(t, json!({ "expectations": doc.expectations() })))
            }
            ExpectAction::Add => {
                let expect = a.expect.as_ref().ok_or_else(|| {
                    OpError::invalid("/expect", "add needs `expect`").with_hint("for example {\"attached\": {\"part\": \"lid\", \"to\": \"crate\"}}")
                })?;
                let mut args = json!({ "expect": expect });
                if let Some(n) = &a.name {
                    args["name"] = json!(n);
                }
                self.one_op("expect.add", args)
            }
            ExpectAction::Remove => {
                let name = a.name.as_ref().ok_or_else(|| OpError::invalid("/name", "remove needs the expectation's name"))?;
                self.one_op("expect.remove", json!({ "name": name }))
            }
        }
    }

    /// Runs one op as a transaction; errors point into the tool's arguments.
    fn brief(&mut self, a: &BriefArgs) -> Result<ToolOutput, OpError> {
        match a.action {
            BriefAction::Set => {
                let brief = a.brief.clone().ok_or_else(|| {
                    OpError::invalid("/brief", "set needs `brief`").with_hint("{\"category\": \"humanoid_head\", \"size\": {\"height\": \"0.26m\"}}")
                })?;
                self.one_op("brief.set", json!({ "name": a.name, "brief": brief }))
            }
            BriefAction::Get => {
                let doc = self.doc();
                let source =
                    doc.brief(&a.name).cloned().ok_or_else(|| OpError::new(ErrorCode::NotFound, format!("no brief `{}`", a.name)).at("/name"))?;
                let brief = p3d_spec::parse_brief(&source).map_err(|e| OpError::invalid(&e.path, e.message))?;
                let mut t = format!("brief {}: {}\n", a.name, serde_json::to_string(&source).unwrap_or_default());
                for i in p3d_spec::brief::expand(&brief) {
                    t.push_str(&format!("  {}\n", i.label));
                }
                Ok(ToolOutput::ok(t, json!({ "brief": source })))
            }
            BriefAction::Coverage => {
                let doc = self.doc();
                let c = p3d_spec::coverage(&doc, &a.name)?;
                let mut t = format!("{}\n", c.line());
                for i in &c.items {
                    let mark = match i.status {
                        p3d_spec::brief::Status::Pass => "ok",
                        p3d_spec::brief::Status::Fail => "FAIL",
                        p3d_spec::brief::Status::Pending => "pending",
                    };
                    t.push_str(&format!("  {mark} {}: {}\n", i.label, i.detail));
                }
                Ok(ToolOutput::ok(t, serde_json::to_value(&c).unwrap_or(Value::Null)))
            }
        }
    }

    fn spec(&mut self, a: &SpecArgs) -> Result<ToolOutput, OpError> {
        let (op, args) = match a.action {
            SpecAction::Get => {
                let doc = self.doc();
                let key = doc.resolve(&a.target).map_err(|e| OpError::from_scene(e, "/target"))?;
                let record = doc
                    .spec(key)
                    .ok_or_else(|| OpError::invalid("/target", format!("`{}` has no spec", a.target)).with_hint("spec set creates one"))?;
                let mut t = format!("spec {}: {}\n", a.target, serde_json::to_string(&record.source).unwrap_or_default());
                for (name, p) in &record.parts {
                    match p.object {
                        Some(id) => t.push_str(&format!(
                            "  {name}: o:{id}, {} faces, {} provenance nodes\n",
                            thousands(p.provenance.face_count()),
                            p.provenance.entries.len()
                        )),
                        None => t.push_str(&format!("  {name}: blended into its parent\n")),
                    }
                }
                return Ok(ToolOutput::ok(t, json!({ "spec": record.source })));
            }
            SpecAction::Set => {
                let spec =
                    a.spec.clone().ok_or_else(|| OpError::invalid("/spec", "set needs `spec`").with_hint("docs topic spec shows the format"))?;
                ("spec.set", json!({ "name": a.target, "spec": spec }))
            }
            SpecAction::Patch => {
                let mut args = json!({ "target": a.target });
                if let Some(m) = &a.merge {
                    args["merge"] = m.clone();
                }
                if let Some(o) = &a.ops {
                    args["ops"] = serde_json::to_value(o).unwrap_or(Value::Null);
                }
                ("spec.patch", args)
            }
            SpecAction::Compile => ("spec.compile", json!({ "target": a.target, "force": a.force })),
        };
        let opts = BatchOptions { dry_run: a.dry_run, ..BatchOptions::default() };
        let r = self.execute(&[OpCall::new(op, args)], &opts).map_err(|mut e| {
            e.path = e.path.map(|p| p.trim_start_matches("/ops/0/args").to_owned()).filter(|p| !p.is_empty());
            e
        })?;
        let out = r.results.first().map(|x| x.output.clone()).unwrap_or_default();
        let ms = r.elapsed.as_secs_f64() * 1000.0;
        let head = if r.dry_run { format!("dry run, nothing changed ({ms:.0} ms)") } else { format!("ok rev {} ({ms:.0} ms)", r.revision) };
        let mut t = format!("{head}: {}\n", out.summary);
        for w in &out.warnings {
            t.push_str(&format!("  warning: {}\n", w.message));
        }
        // A spec with DNA reports its genes on every change.
        let doc = self.doc();
        if let Ok(root) = doc.resolve(&a.target)
            && let Some(genes) = p3d_spec::dna_of(&doc, root)
            && let Ok(view) = View::new(&doc, Some(&a.target))
            && let Some(line) = p3d_perceive::dna::line(&view, &genes)
        {
            t.push_str(&line);
            t.push('\n');
        }
        Ok(ToolOutput::ok(self.finish(t), json!({ "revision": r.revision, "summary": out.summary })))
    }

    fn deform(&mut self, a: &DeformArgs) -> Result<ToolOutput, OpError> {
        if a.handles.is_empty() {
            return Err(OpError::invalid("/handles", "give at least one handle").with_hint("{\"move\": \"tip\", \"by\": [0, 0.01, 0]}"));
        }
        let doc = self.doc();
        let key = doc.resolve(&a.target).map_err(|e| OpError::from_scene(e, "/target"))?;
        let o = doc.object(key).ok_or_else(|| OpError::internal("object vanished"))?;
        // A spec part: the handles join its `deform` block.
        let part = o.parent().and_then(|p| {
            let rec = doc.spec(p)?;
            rec.parts.get(o.name()).filter(|r| r.object == Some(o.id()))?;
            Some((p, rec.source.clone(), o.name().to_owned()))
        });
        if let Some((root, source, name)) = part {
            let old =
                source.pointer(&format!("/parts/{name}/deform")).cloned().filter(|_| !a.replace).unwrap_or(json!({ "handles": [], "pins": [] }));
            let mut handles = old.get("handles").and_then(Value::as_array).cloned().unwrap_or_default();
            handles.extend(a.handles.iter().map(|h| json!({ "move": h.at, "by": h.by })));
            let mut pins = old.get("pins").and_then(Value::as_array).cloned().unwrap_or_default();
            pins.extend(a.pins.iter().cloned());
            let mut deform = json!({ "handles": handles, "pins": pins });
            if let Some(r) = a.radius.or_else(|| old.get("radius").and_then(|v| serde_json::from_value(v.clone()).ok())) {
                deform["radius"] = json!(r);
            }
            let target = doc.path(root).unwrap_or_default();
            return self.spec(&SpecArgs {
                action: SpecAction::Patch,
                target,
                merge: Some(json!({ "parts": { name: { "deform": deform } } })),
                ..SpecArgs::default()
            });
        }
        let mut args =
            json!({ "target": a.target, "handles": a.handles.iter().map(|h| json!({ "at": h.at, "by": h.by })).collect::<Vec<_>>(), "pins": a.pins });
        if let Some(r) = a.radius {
            args["radius"] = json!(r);
        }
        self.one_op("deform.handles", args)
    }

    fn one_op(&mut self, op: &str, args: Value) -> Result<ToolOutput, OpError> {
        let r = self.execute(&[OpCall::new(op, args)], &BatchOptions::default()).map_err(|mut e| {
            e.path = e.path.map(|p| p.trim_start_matches("/ops/0/args").to_owned()).filter(|p| !p.is_empty());
            e
        })?;
        let summary = r.results.first().map_or(String::new(), |x| x.output.summary.clone());
        let text = format!("ok rev {}: {summary}\n", r.revision);
        Ok(ToolOutput::ok(self.finish(text), json!({ "revision": r.revision, "summary": summary })))
    }

    fn query(&mut self, a: &QueryArgs) -> Result<ToolOutput, OpError> {
        let doc = self.doc();
        let key = doc.resolve(&a.target).map_err(|e| OpError::from_scene(e, "/target"))?;
        let mesh = doc.object_mesh(key).ok_or_else(|| OpError::invalid("/target", format!("`{}` is not a mesh", a.target)))?;
        let sel = p3d_query::select(mesh, &a.select).map_err(|e| OpError::from_query(e, "/select"))?;
        let world = doc.world_affine(key).unwrap_or_default();
        let (mut lo, mut hi) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
        let mut add = |p: glam::Vec3| {
            let w = world.transform_point3(p);
            lo = lo.min(w);
            hi = hi.max(w);
        };
        for i in sel.bits.ones().map(|i| i as u32) {
            match sel.kind {
                p3d_mesh::ElementKind::Vert => add(mesh.positions[i as usize]),
                p3d_mesh::ElementKind::Edge => mesh.edge_verts(i).iter().for_each(|&v| add(mesh.positions[v as usize])),
                p3d_mesh::ElementKind::Face => mesh.face_verts(i).for_each(|v| add(mesh.positions[v as usize])),
            }
        }
        let count = sel.count();
        let kind = sel.kind.as_str();
        let mut t = if count == 0 {
            format!("{}: 0 {kind} match\n", a.target)
        } else {
            format!("{}: {}, world box {} from {} to {}\n", a.target, count_of(count, sel.kind), dims(hi - lo), point(lo), point(hi))
        };
        let mut revision = self.session.revision();
        if let Some(name) = &a.save_as {
            let r = self
                .execute(&[OpCall::new("select.save", json!({ "target": a.target, "select": a.select, "name": name }))], &BatchOptions::default())?;
            revision = r.revision;
            t.push_str(&format!("saved as group {name} (rev {revision})\n"));
            t = self.finish(t);
        }
        let bbox = (count > 0).then(|| json!({ "min": lo, "max": hi }));
        Ok(ToolOutput::ok(t, json!({ "count": count, "kind": kind, "bbox": bbox, "revision": revision })))
    }

    fn analyze(&mut self, a: &AnalyzeArgs) -> Result<ToolOutput, OpError> {
        let doc = self.doc();
        let view = View::new(&doc, a.target.as_deref()).map_err(op_error)?;
        let mut t = format!("analyze {}: {} part{}\n", view.title(), view.parts.len(), if view.parts.len() == 1 { "" } else { "s" });
        let mut rows = Vec::new();
        let (mut tris, mut quads, mut faces) = (0usize, 0usize, 0usize);
        for p in &view.parts {
            let m = p.topology();
            tris += m.tris;
            quads += m.quad_faces;
            faces += m.faces;
            t.push_str(&format!(
                "{}: {} tris, {} faces (quads {:.0}%, tris {}, ngons {}), poles 3:{} 5:{} 6+:{}, angles {:.0} to {:.0} deg, holes {}, pieces {}{}{}\n",
                p.label,
                thousands(m.tris),
                thousands(m.faces),
                m.quad_percent,
                m.tri_faces,
                m.ngon_faces,
                m.poles_3,
                m.poles_5,
                m.poles_6_plus,
                m.min_angle_deg,
                m.max_angle_deg,
                m.boundary_loops,
                m.parts,
                if m.degenerate_faces + m.short_edges > 0 { format!(", degenerate {}", m.degenerate_faces + m.short_edges) } else { String::new() },
                if m.inverted_parts > 0 { format!(", inverted {}", m.inverted_parts) } else { String::new() },
            ));
            rows.push(json!({ "part": p.label, "id": p.id, "topology": m }));
        }
        let budget = doc.settings.tri_budget() as usize;
        let quad_pct = if faces == 0 { 0.0 } else { 100.0 * quads as f64 / faces as f64 };
        t.push_str(&format!(
            "total {} tris (budget {}, {}), quads {quad_pct:.0}%\n",
            thousands(tris),
            thousands(budget),
            if tris <= budget { "ok" } else { "over" }
        ));
        Ok(ToolOutput::ok(t, json!({ "parts": rows, "tris": tris, "budget": budget })))
    }

    fn list_ops(&mut self, a: &ListOpsArgs) -> Result<ToolOutput, OpError> {
        let registry = Arc::clone(self.session.registry());
        let mut ops: Vec<&dyn p3d_ops::DynOp> = match &a.search {
            Some(q) => registry.search(q),
            None => registry.iter().collect(),
        };
        if let Some(c) = &a.category {
            let known: Vec<&str> = registry.iter().map(|o| o.meta().category.as_str()).collect();
            if !known.contains(&c.as_str()) {
                let mut cats = known.clone();
                cats.dedup();
                return Err(OpError::invalid("/category", format!("no category `{c}`")).with_suggestions(p3d_scene::name::suggest(c, cats, 3)));
            }
            ops.retain(|o| o.meta().category.as_str() == c);
        }
        let mut t = format!("{} op{}\n", ops.len(), if ops.len() == 1 { "" } else { "s" });
        for o in &ops {
            t.push_str(&format!("{}: {}\n", o.id(), o.meta().summary));
        }
        let list: Vec<Value> =
            ops.iter().map(|o| json!({ "id": o.id(), "summary": o.meta().summary, "category": o.meta().category.as_str() })).collect();
        Ok(ToolOutput::ok(t, json!({ "ops": list })))
    }

    fn describe_op(&mut self, a: &DescribeOpArgs) -> Result<ToolOutput, OpError> {
        let registry = Arc::clone(self.session.registry());
        let op = match registry.get(&a.op) {
            Ok(op) => op,
            // An alias names exactly one op: describe it.
            Err(e) => match e.suggestions.first().and_then(|s| registry.get(s).ok()) {
                Some(op) if op.meta().aliases.iter().any(|x| x.eq_ignore_ascii_case(&a.op)) => op,
                _ => return Err(e.at("/op")),
            },
        };
        let catalog = catalog_json(&registry);
        let entry = catalog["ops"].as_array().and_then(|v| v.iter().find(|o| o["id"] == op.id())).cloned().unwrap_or(Value::Null);
        let m = op.meta();
        let mut t = format!("{} ({}): {}\n\n{}\n\n", op.id(), m.label, m.summary, m.docs);
        if !m.aliases.is_empty() {
            t.push_str(&format!("aliases: {}\n", m.aliases.join(", ")));
        }
        t.push_str(&format!("complexity: {}\n", m.complexity));
        t.push_str(&format!("parameters (JSON Schema): {}\n", entry["params"]));
        t.push_str(&format!("defaults: {}\n", entry["defaults"]));
        for ex in m.examples {
            t.push_str(&format!("example: {} -> {}\n", ex.args, ex.result));
        }
        Ok(ToolOutput::ok(t, entry))
    }

    fn render(&mut self, a: &RenderArgs) -> Result<ToolOutput, OpError> {
        use p3d_render::{Mode, RenderRequest, ViewKind};
        let doc = self.doc();
        let root = match &a.target {
            Some(t) => Some(doc.resolve(t).map_err(|e| OpError::from_scene(e, "/target"))?),
            None => None,
        };
        let size = a.size.unwrap_or(512);
        if !(p3d_render::MIN_SIZE..=p3d_render::MAX_SIZE).contains(&size) {
            return Err(OpError::out_of_range("/size", format!("{size} is outside {} to {}", p3d_render::MIN_SIZE, p3d_render::MAX_SIZE)));
        }
        let views: Vec<ViewKind> = match &a.views {
            None => RenderRequest::default().views,
            Some(v) if v.is_empty() => return Err(OpError::invalid("/views", "give at least one view")),
            Some(v) => v
                .iter()
                .map(|v| match v {
                    RenderView::Front => ViewKind::Front,
                    RenderView::Back => ViewKind::Back,
                    RenderView::Right => ViewKind::Right,
                    RenderView::Left => ViewKind::Left,
                    RenderView::Top => ViewKind::Top,
                    RenderView::Bottom => ViewKind::Bottom,
                    RenderView::ThreeQuarter => ViewKind::ThreeQuarter,
                })
                .collect(),
        };
        let mode = match a.mode {
            RenderMode::Shaded => Mode::Shaded,
            RenderMode::Wire => Mode::Wire,
            RenderMode::ShadedWire => Mode::ShadedWire,
            RenderMode::Matcap => Mode::Matcap,
        };
        let req = RenderRequest { views, mode, size, sheet: a.sheet.unwrap_or(true), labels: a.labels.unwrap_or(true) };
        let scene = p3d_render::prepare(&doc, root);
        if scene.is_empty() {
            return Err(OpError::new(ErrorCode::EmptySelection, "nothing to render: no visible mesh").at(if a.target.is_some() {
                "/target"
            } else {
                ""
            }));
        }
        let renderer = self.renderer.get_or_insert_with(p3d_render::Renderer::new).as_ref().map_err(|e| {
            OpError::new(ErrorCode::Unsupported, e.to_string()).with_hint("perceive answers the same questions as text without a GPU")
        })?;
        let images = renderer.render(&scene, &req).map_err(|e| OpError::new(ErrorCode::Internal, e.to_string()))?;
        let mut pngs = Vec::new();
        let mut sizes = Vec::new();
        for (title, img) in &images {
            pngs.push(img.to_png().map_err(|e| OpError::new(ErrorCode::Internal, format!("PNG encoding failed: {e}")))?);
            sizes.push(format!("{title} {} x {} px", img.width, img.height));
        }
        let names: Vec<&str> = req.views.iter().map(|v| v.name()).collect();
        let what = a.target.as_deref().unwrap_or("scene");
        let text = format!(
            "render {} of {what}: {}, {} px per view; {}\n{} parts, {} tris, size {} (W x H x D); GPU {}\n",
            names.join(", "),
            mode.name(),
            size,
            sizes.join(", "),
            scene.parts.len(),
            thousands(scene.triangle_count()),
            dims(scene.hi - scene.lo),
            renderer.adapter()
        );
        let mut out = ToolOutput::ok(
            text,
            json!({ "views": names, "mode": mode.name(), "size": size, "images": sizes, "parts": scene.parts.len(), "tris": scene.triangle_count() }),
        );
        out.images = pngs;
        Ok(out)
    }

    fn history(&mut self, a: &HistoryArgs) -> Result<ToolOutput, OpError> {
        let steps = a.steps.unwrap_or(1).clamp(1, 1000);
        match a.action {
            HistoryAction::List => {
                let h = self.session.history();
                let mut t = format!("rev {}; {} undo step{}", self.session.revision(), h.len(), if h.len() == 1 { "" } else { "s" });
                if self.session.can_redo() {
                    t.push_str(", redo available");
                }
                t.push('\n');
                for e in h.iter().rev().take(20) {
                    t.push_str(&format!("rev {}: {}\n", e.revision, e.label));
                }
                let cps: Vec<&str> = self.session.checkpoints().collect();
                if !cps.is_empty() {
                    t.push_str(&format!("checkpoints: {}\n", cps.join(", ")));
                }
                let entries: Vec<Value> = h.iter().map(|e| json!({ "revision": e.revision, "label": e.label })).collect();
                Ok(ToolOutput::ok(t, json!({ "revision": self.session.revision(), "history": entries, "checkpoints": cps })))
            }
            HistoryAction::Undo | HistoryAction::Redo => {
                let mut labels = Vec::new();
                for _ in 0..steps {
                    let r = if a.action == HistoryAction::Undo { self.session.undo() } else { self.session.redo() };
                    match r {
                        Ok(l) => labels.push(l),
                        Err(e) if labels.is_empty() => return Err(e),
                        Err(_) => break,
                    }
                }
                let verb = if a.action == HistoryAction::Undo { "undid" } else { "redid" };
                let t = format!("{verb} {}; now rev {}\n", labels.join(", "), self.session.revision());
                Ok(ToolOutput::ok(self.finish(t), json!({ "revision": self.session.revision(), "labels": labels })))
            }
            HistoryAction::Checkpoint => {
                let name = a.name.as_ref().ok_or_else(|| OpError::invalid("/name", "checkpoint needs a name"))?;
                self.session.checkpoint(name);
                Ok(ToolOutput::ok(format!("checkpoint {name} at rev {}\n", self.session.revision()), json!({ "revision": self.session.revision() })))
            }
            HistoryAction::Revert => {
                let name = a.name.as_ref().ok_or_else(|| OpError::invalid("/name", "revert needs a checkpoint name"))?;
                let rev = self.session.revert(name).map_err(|e| e.at("/name"))?;
                let t = format!("reverted to {name}; now rev {rev}\n");
                Ok(ToolOutput::ok(self.finish(t), json!({ "revision": rev })))
            }
            HistoryAction::Diff => self.diff(a.from.as_deref(), a.to.as_deref(), a.target.as_deref()),
        }
    }
}

/// `1 face`, `24 faces`, `3 verts`.
fn count_of(n: usize, kind: p3d_mesh::ElementKind) -> String {
    let plural = kind.as_str();
    if n == 1 { format!("1 {}", plural.trim_end_matches('s')) } else { format!("{} {plural}", thousands(n)) }
}
