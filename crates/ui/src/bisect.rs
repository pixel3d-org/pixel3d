//! Bisect in the History panel: pick what fails now (an expectation, a
//! diagnostics finding or a brief item), and a binary search over the undo
//! states (`Session::bisect`) names the transaction that broke it, the
//! revisions on either side and the spec paths it changed. "Go to the
//! state before" undoes back to the last good state (redo returns).

use egui::{RichText, vec2};
use p3d_engine::Bisected;
use p3d_scene::Document;
use serde_json::{Value, json};

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

/// What to bisect with: true where it fails.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Test {
    Expect(String),
    Finding { code: String, part: Option<String> },
    Brief { name: String, item: String },
}

impl Test {
    pub fn label(&self) -> String {
        match self {
            Test::Expect(name) => format!("{} {name}", t("expectation")),
            Test::Finding { code, part: Some(p) } => format!("{code} {} {p}", t("on")),
            Test::Finding { code, part: None } => code.clone(),
            Test::Brief { name, item } => format!("{} {name}: {item}", t("brief")),
        }
    }

    /// As agents write it (the MCP history tool's `test`).
    pub fn to_value(&self) -> Value {
        match self {
            Test::Expect(name) => json!({ "expect": name }),
            Test::Finding { code, part } => json!({ "finding": { "code": code, "part": part } }),
            Test::Brief { name, item } => json!({ "brief": { "name": name, "item": item } }),
        }
    }

    pub fn from_value(v: &Value) -> Option<Test> {
        if let Some(name) = v.get("expect").and_then(Value::as_str) {
            return Some(Test::Expect(name.to_owned()));
        }
        if let Some(f) = v.get("finding") {
            let code = f.get("code")?.as_str()?.to_owned();
            return Some(Test::Finding { code, part: f.get("part").and_then(Value::as_str).map(str::to_owned) });
        }
        let b = v.get("brief")?;
        Some(Test::Brief { name: b.get("name")?.as_str()?.to_owned(), item: b.get("item")?.as_str()?.to_owned() })
    }

    /// Whether the test fails in `doc`.
    ///
    /// Complexity: one expectation check, one diagnosis, or one brief
    /// coverage of the document.
    pub fn fails(&self, doc: &Document) -> bool {
        match self {
            Test::Expect(name) => doc.expectations().iter().find(|e| &e.name == name).is_some_and(|e| !p3d_perceive::expect::check(doc, e).pass),
            Test::Finding { code, part } => p3d_perceive::View::new(doc, None).is_ok_and(|v| {
                p3d_perceive::diagnose::diagnose(&v)
                    .iter()
                    .any(|f| f.code.eq_ignore_ascii_case(code) && part.as_ref().is_none_or(|p| f.part.contains(p.as_str())))
            }),
            Test::Brief { name, item } => p3d_spec::coverage(doc, name)
                .is_ok_and(|c| c.items.iter().any(|i| i.label.starts_with(item.as_str()) && i.status == p3d_spec::brief::Status::Fail)),
        }
    }
}

/// Where a bisect found the change.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub transaction: String,
    /// The last revision that passes and the first that fails.
    pub good: u64,
    pub bad: u64,
    pub checks: usize,
    /// Spec paths the transaction changed (`/parts/ear_l/sdf/0/radius`).
    pub changes: Vec<String>,
}

/// A finished bisect.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub test: Test,
    pub found: Option<Found>,
    /// What it means, in a line.
    pub text: String,
}

/// The bisect card between frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BisectState {
    pub open: bool,
    pub test: Option<Test>,
    pub outcome: Option<Outcome>,
}

/// The JSON pointers where `b` differs from `a`.
///
/// Complexity: O(size of both values).
fn json_diff(a: &Value, b: &Value, path: String, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = format!("{path}/{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => json_diff(u, v, p, out),
                    (None, Some(_)) => out.push(format!("{p} {}", t("added"))),
                    (Some(_), None) => out.push(format!("{p} {}", t("removed"))),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                json_diff(u, v, format!("{path}/{i}"), out);
            }
        }
        _ if a != b => out.push(path),
        _ => {}
    }
}

/// The spec paths changed between two documents, per spec root.
///
/// Complexity: O(size of the specs).
fn spec_changes(before: &Document, after: &Document) -> Vec<String> {
    let mut out = Vec::new();
    for root in after.spec_roots() {
        let (Some(new), Some(old)) = (after.spec(root), before.spec(root)) else { continue };
        let mut changes = Vec::new();
        json_diff(&old.source, &new.source, String::new(), &mut changes);
        let name = after.path(root).unwrap_or_default();
        out.extend(changes.into_iter().map(|c| format!("{name}: {c}")));
    }
    out
}

impl PixelApp {
    /// What could be bisected now: expectations, findings and brief items,
    /// each with whether it fails now (failing ones first).
    ///
    /// Complexity: one check per expectation and one coverage per brief;
    /// findings come from the Health panel's latest run.
    pub fn bisect_candidates(&self) -> Vec<(Test, bool)> {
        let doc = self.doc();
        let mut out: Vec<(Test, bool)> =
            doc.expectations().iter().map(|e| (Test::Expect(e.name.clone()), !p3d_perceive::expect::check(doc, e).pass)).collect();
        let mut seen = std::collections::BTreeSet::new();
        for f in &self.health.findings {
            let test = Test::Finding { code: f.code.to_owned(), part: Some(f.part.clone()) };
            if seen.insert(test.clone()) {
                out.push((test, true));
            }
        }
        for (name, _) in doc.briefs() {
            if let Ok(c) = p3d_spec::coverage(doc, name) {
                for i in c.items.iter().filter(|i| i.status == p3d_spec::brief::Status::Fail) {
                    out.push((Test::Brief { name: name.to_owned(), item: i.label.clone() }, true));
                }
            }
        }
        out.sort_by_key(|(_, fails)| !fails);
        out
    }

    /// Bisects the undo history with a test.
    ///
    /// Complexity: O(log states) runs of the test.
    pub fn run_bisect(&mut self, test: Test) -> Outcome {
        let label = test.label();
        let outcome = match self.session.bisect(|d| test.fails(d)) {
            Bisected::NotNow => Outcome { test, found: None, text: format!("{label}: {}", t("it passes now; nothing to look for")) },
            Bisected::AlwaysBad { oldest } => Outcome {
                test,
                found: None,
                text: format!("{label}: {} {oldest}, {}", t("it already failed at rev"), t("the oldest state in the history")),
            },
            Bisected::Found { label: tx, before, after, checks } => {
                let found =
                    Found { transaction: tx.clone(), good: before.revision, bad: after.revision, checks, changes: spec_changes(&before, &after) };
                let text = format!(
                    "{label}: {} {} {} `{tx}` ({} {}; {checks} {})",
                    t("first fails at rev"),
                    found.bad,
                    t("after"),
                    t("passes at rev"),
                    found.good,
                    t("checks")
                );
                Outcome { test, found: Some(found), text }
            }
        };
        self.set_status(outcome.text.clone());
        self.bisect.outcome = Some(outcome.clone());
        outcome
    }

    /// Undoes back to `revision` (redo returns); stops when the history
    /// runs out.
    ///
    /// Complexity: O(steps) undos.
    pub fn jump_to_revision(&mut self, revision: u64) -> Result<Value, String> {
        let mut steps = 0;
        while self.session.revision() != revision {
            if self.session.history().is_empty() || steps > 10_000 {
                return Err(format!("{} {revision} {}", t("rev"), t("is not in the undo history")));
            }
            self.session.undo().map_err(|e| e.message)?;
            steps += 1;
        }
        self.prune_selection();
        self.set_status(format!("{} {revision} ({steps} {})", t("Back at rev"), t("undo steps")));
        Ok(json!({ "revision": revision, "undone": steps }))
    }

    /// Runs `history.bisect` and `history.jump`.
    pub(crate) fn bisect_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        match id {
            "history.bisect" => {
                let test = params.get("test").and_then(Test::from_value).or_else(|| self.bisect.test.clone()).ok_or(t(
                    "test is required: {\"expect\": \"E2\"}, {\"finding\": {\"code\": \"PENETRATION\", \"part\": \"eye_l\"}} or {\"brief\": {\"name\": \"elf\", \"item\": \"ear\"}}",
                ))?;
                self.bisect.test = Some(test.clone());
                let o = self.run_bisect(test);
                Ok(match &o.found {
                    Some(f) => json!({ "found": true, "transaction": f.transaction, "good": f.good, "bad": f.bad, "checks": f.checks,
                        "changes": f.changes, "text": o.text }),
                    None => json!({ "found": false, "text": o.text }),
                })
            }
            "history.jump" => {
                let revision =
                    params.get("revision").and_then(Value::as_u64).or_else(|| self.bisect.outcome.as_ref()?.found.as_ref().map(|f| f.good));
                self.jump_to_revision(revision.ok_or(t("revision is required"))?)
            }
            other => Err(format!("{} `{other}`", t("unknown command"))),
        }
    }

    /// The bisect card for agents (`ui.inspect`).
    pub(crate) fn bisect_state(&self) -> Value {
        let b = &self.bisect;
        json!({
            "open": b.open,
            "test": b.test.as_ref().map(Test::to_value),
            "outcome": b.outcome.as_ref().map(|o| json!({
                "text": o.text,
                "found": o.found.as_ref().map(|f| json!({ "transaction": f.transaction, "good": f.good, "bad": f.bad, "checks": f.checks, "changes": f.changes })),
            })),
        })
    }
}

/// The card above the History list: the tests on the left with Find, the
/// result on the right.
pub fn card(app: &mut PixelApp, ui: &mut egui::Ui) {
    let tk = Tokens::get();
    let mut run = None;
    let mut jump = None;
    let candidates = app.bisect_candidates();
    let selected = app.bisect.test.clone();
    let outcome = app.bisect.outcome.clone();
    egui::Frame::NONE.fill(tk.card).stroke(egui::Stroke::new(1.0, tk.card_border)).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
        let w = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.allocate_ui(vec2((w * 0.45).max(200.0), 150.0), |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(t("Find the change that broke...")).color(tk.text));
                    if candidates.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(t("Nothing fails: add expectations or a brief, or wait for Health to find something."))
                                    .font(theme::small())
                                    .color(tk.text_dim),
                            )
                            .wrap(),
                        );
                    }
                    egui::ScrollArea::vertical().id_salt("bisect-tests").max_height(90.0).show(ui, |ui| {
                        for (test, fails) in &candidates {
                            let text = RichText::new(test.label()).font(theme::small()).color(if *fails { tk.health_error } else { tk.text_dim });
                            let r = ui.selectable_label(selected.as_ref() == Some(test), text);
                            crate::control::record_widget(ui, &r, &test.label(), "option", Some(selected.as_ref() == Some(test)));
                            if r.clicked() {
                                app.bisect.test = Some(test.clone());
                            }
                        }
                    });
                    let b = ui.add_enabled(app.bisect.test.is_some(), egui::Button::new(t("Find the change")));
                    crate::control::record_widget(ui, &b, t("Find the change"), "button", None);
                    if b.on_hover_text(t("Binary search over the undo history for the transaction that made it fail")).clicked() {
                        run = app.bisect.test.clone();
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let Some(o) = &outcome else {
                    ui.label(
                        RichText::new(t("Pick what fails, then Find: the undo history is searched for the step that broke it."))
                            .font(theme::small())
                            .color(tk.text_faint),
                    );
                    return;
                };
                ui.add(egui::Label::new(RichText::new(&o.text).font(theme::small()).color(tk.text)).wrap());
                if let Some(f) = &o.found {
                    if !f.changes.is_empty() {
                        ui.label(RichText::new(t("It changed:")).font(theme::small()).color(tk.text_dim));
                    }
                    for c in f.changes.iter().take(5) {
                        ui.label(RichText::new(c).font(theme::mono()).color(tk.text_dim));
                    }
                    if f.changes.len() > 5 {
                        ui.label(RichText::new(format!("... {} {}", f.changes.len() - 5, t("more"))).font(theme::small()).color(tk.text_faint));
                    }
                    let b = ui.button(format!("{} {}", t("Go to the state before, rev"), f.good));
                    crate::control::record_widget(ui, &b, t("Go to the state before"), "button", None);
                    if b.on_hover_text(t("Undo back to the last state where it passed (redo returns)")).clicked() {
                        jump = Some(f.good);
                    }
                }
            });
        });
    });
    ui.add_space(4.0);
    if let Some(test) = run {
        app.run_bisect(test);
    }
    if let Some(rev) = jump
        && let Err(e) = app.jump_to_revision(rev)
    {
        app.set_status(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;

    #[test]
    fn bisect_names_the_change_that_broke_an_expectation() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("prim.box", json!({ "name": "crate", "width": 0.5 })).unwrap();
        a.run("expect.add", json!({ "name": "E1", "expect": { "dims": { "target": "crate", "width": { "max": 0.6 } } } })).unwrap();
        let good = a.session.revision();
        a.run("scene.set_transform", json!({ "target": "crate", "scale": [2, 1, 1] })).unwrap();
        a.run("prim.box", json!({ "name": "other" })).unwrap();
        let test = Test::Expect("E1".into());
        assert!(a.bisect_candidates().iter().any(|(t, fails)| *t == test && *fails));
        let o = a.run_bisect(test);
        let f = o.found.expect("found");
        assert_eq!(f.good, good, "{}", o.text);
        a.jump_to_revision(f.good).unwrap();
        assert_eq!(a.session.revision(), good);
    }

    #[test]
    fn a_spec_patch_that_broke_a_test_names_its_paths() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("spec.starter", json!({ "name": "lamp" })).unwrap();
        a.run("expect.add", json!({ "name": "E1", "expect": { "dims": { "target": "lamp", "height": { "max": 0.7 } } } })).unwrap();
        let good = a.session.revision();
        let tall = json!({ "target": "lamp", "merge": { "parts": { "pole": { "primitive": { "cylinder": { "height": 0.9 } } } } } });
        a.run("spec.patch", tall).unwrap();
        a.run("prim.box", json!({ "name": "crate", "position": [1, 0, 0] })).unwrap();
        let r = a.run("history.bisect", json!({ "test": { "expect": "E1" } })).unwrap();
        assert_eq!((r["found"].clone(), r["good"].clone()), (json!(true), json!(good)), "{r}");
        assert_eq!(r["changes"], json!(["lamp: /parts/pole/primitive/cylinder/height"]));
        a.run("history.jump", Value::Null).unwrap();
        assert_eq!(a.session.revision(), good);
    }

    #[test]
    fn spec_changes_name_their_paths() {
        let mut a = PixelApp::new(p3d_engine::Session::new(), Services::default());
        a.run("spec.starter", json!({ "name": "lamp" })).unwrap();
        let before = std::sync::Arc::clone(a.session.doc());
        a.run("spec.patch", json!({ "target": "lamp", "merge": { "parts": { "pole": { "primitive": { "cylinder": { "height": 0.6 } } } } } }))
            .unwrap();
        let changes = spec_changes(&before, a.session.doc());
        assert_eq!(changes, ["lamp: /parts/pole/primitive/cylinder/height"]);
        // Tests round-trip as agents write them.
        for test in [
            Test::Expect("E2".into()),
            Test::Finding { code: "PENETRATION".into(), part: Some("eye_l".into()) },
            Test::Brief { name: "elf".into(), item: "ear".into() },
        ] {
            assert_eq!(Test::from_value(&test.to_value()), Some(test));
        }
        // Nothing fails: nothing to look for.
        let o = a.run_bisect(Test::Finding { code: "PENETRATION".into(), part: None });
        assert!(o.found.is_none() && o.text.contains("passes now"), "{}", o.text);
    }
}
