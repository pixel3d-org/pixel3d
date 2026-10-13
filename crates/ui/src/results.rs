//! What the last transaction reported: each op's summary and warnings. A
//! warning that carries a fix (a ready op call, for example `cleanup.auto`
//! after an import) gets a button that runs it, wherever op results are
//! shown: the card over the viewport (`quick.rs`) and the Properties Tool
//! tab. Reports, ops that change nothing (`cleanup.report`), add no undo
//! step and are shown as short lines instead of one long sentence.

use p3d_engine::{BatchResult, OpCall};
use p3d_ops::{Registry, Warning};
use serde_json::Value;

use crate::PixelApp;
use crate::i18n::t;
use crate::theme::{self, Tokens};

/// Entries and warnings drawn at most (a selection may hold many objects).
const SHOWN: usize = 8;

/// One op's report.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub op: String,
    pub summary: String,
    pub warnings: Vec<Warning>,
}

/// The last transaction's report.
#[derive(Debug, Clone, PartialEq)]
pub struct LastResult {
    /// The revision it produced (a report: the one it looked at).
    pub revision: u64,
    pub entries: Vec<Entry>,
    /// Nothing changed: a report, with nothing to undo or adjust.
    pub read_only: bool,
}

impl LastResult {
    /// The report of a batch. A cleanup report that finds work gets a
    /// warning whose fix runs `cleanup.auto` with the same settings.
    ///
    /// Complexity: O(results + summary and warning text).
    pub fn from_batch(r: &BatchResult, registry: &Registry) -> Self {
        let entries = r
            .results
            .iter()
            .map(|res| {
                let mut warnings = res.output.warnings.clone();
                if res.op == "cleanup.report"
                    && let Some(would) = res.output.summary.split("; ").nth(1)
                    && !would.ends_with("change nothing")
                {
                    let fix = p3d_ops::OpCall { op: "cleanup.auto".into(), args: res.params.clone() };
                    warnings.insert(0, Warning { message: op_names(would, registry), fix: Some(fix) });
                }
                Entry { op: res.op.clone(), summary: res.output.summary.clone(), warnings }
            })
            .collect();
        LastResult { revision: r.revision, entries, read_only: r.dry_run }
    }

    pub fn warning_count(&self) -> usize {
        self.entries.iter().map(|e| e.warnings.len()).sum()
    }

    /// Whether there is more to show than the status line says.
    pub fn worth_showing(&self) -> bool {
        self.read_only || self.warning_count() > 0
    }

    /// The title of the card: the op's label, or how many ran.
    pub fn title(&self, registry: &Registry) -> String {
        match self.entries.as_slice() {
            [one] => registry.get(&one.op).map_or_else(|_| one.op.clone(), |o| o.meta().label.to_owned()),
            many => format!("{} {}", many.len(), t("operations")),
        }
    }
}

/// `text` with a leading op id written as the op's label
/// (`cleanup.auto would weld 2 vertices` reads `Clean up would weld ...`).
///
/// Complexity: O(len(text) + log ops).
pub fn op_names(text: &str, registry: &Registry) -> String {
    let (first, rest) = text.split_once(' ').unwrap_or((text, ""));
    match registry.get(first) {
        Ok(op) if first.contains('.') => format!("{}{}{rest}", op.meta().label, if rest.is_empty() { "" } else { " " }),
        _ => text.to_owned(),
    }
}

/// A summary as a subject and short lines: `crate (o:1): 2 parts (0
/// closed), 2 open borders; cleanup.auto would weld 2 vertices` becomes
/// `crate (o:1)` with `2 parts (0 closed)`, `2 open borders` and `Clean up
/// would weld 2 vertices`. The first clause splits at commas outside
/// parentheses; later clauses stay whole.
///
/// Complexity: O(len(summary)).
pub fn readable(summary: &str, registry: &Registry) -> (String, Vec<String>) {
    let (subject, rest) = match summary.split_once(": ") {
        Some((s, r)) => (s.to_owned(), r),
        None => (String::new(), summary),
    };
    let mut lines = Vec::new();
    for (i, clause) in rest.split("; ").enumerate() {
        if i > 0 {
            lines.push(op_names(clause, registry));
            continue;
        }
        let (mut depth, mut start) = (0i32, 0usize);
        let bytes = clause.as_bytes();
        for (j, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b',' if depth == 0 && bytes.get(j + 1) == Some(&b' ') => {
                    lines.push(clause[start..j].to_owned());
                    start = j + 2;
                }
                _ => {}
            }
        }
        lines.push(clause[start..].to_owned());
    }
    lines.retain(|l| !l.is_empty());
    (subject, lines)
}

/// Draws the warnings of `result`, each fix as a button (disabled unless
/// `enabled`). Returns the fix clicked.
pub fn warnings_ui(ui: &mut egui::Ui, result: &LastResult, registry: &Registry, enabled: bool) -> Option<OpCall> {
    let tk = Tokens::get();
    let mut clicked = None;
    let all: Vec<&Warning> = result.entries.iter().flat_map(|e| &e.warnings).collect();
    for w in all.iter().take(SHOWN) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), egui::Sense::hover());
            crate::icons::paint_colored(ui.painter(), r.shrink(1.0), crate::icons::Icon::Warning, tk.health_warn, tk.accent);
            ui.add(egui::Label::new(egui::RichText::new(&w.message).font(theme::small()).color(tk.text)).wrap());
        });
        if let Some(fix) = &w.fix {
            let op = registry.get(&fix.op).ok();
            let name = op.map_or(fix.op.as_str(), |o| o.meta().label);
            let label = format!("{}: {name}", t("Fix"));
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let b = ui.add_enabled(enabled, egui::Button::new(egui::RichText::new(&label).font(theme::small())));
                crate::control::record_widget(ui, &b, &label, "button", None);
                let tip = format!("{} ({} {})", op.map_or("", |o| o.meta().summary), fix.op, fix.args);
                if b.on_hover_text(tip).clicked() {
                    clicked = Some(OpCall::new(&fix.op, fix.args.clone()));
                }
            });
        }
    }
    if all.len() > SHOWN {
        ui.label(
            egui::RichText::new(format!("{} {}", all.len() - SHOWN, t("more warnings in the Console"))).font(theme::small()).color(tk.text_faint),
        );
    }
    clicked
}

/// Draws a report: each entry's subject and short lines, then the
/// warnings. Returns the fix clicked.
pub fn report_ui(ui: &mut egui::Ui, result: &LastResult, registry: &Registry, enabled: bool) -> Option<OpCall> {
    let tk = Tokens::get();
    for e in result.entries.iter().take(SHOWN) {
        let (subject, lines) = readable(&e.summary, registry);
        if !subject.is_empty() {
            ui.label(egui::RichText::new(subject).font(theme::body()).color(tk.text));
        }
        // The last clause (what cleanup would do) shows as the warning.
        let skip = usize::from(e.warnings.first().is_some_and(|w| w.fix.is_some()) && e.op == "cleanup.report");
        for l in &lines[..lines.len() - skip.min(lines.len())] {
            ui.label(egui::RichText::new(format!("\u{2022} {l}")).font(theme::small()).color(tk.text_dim));
        }
    }
    if result.entries.len() > SHOWN {
        ui.label(
            egui::RichText::new(format!("{} {}", result.entries.len() - SHOWN, t("more in the Console"))).font(theme::small()).color(tk.text_faint),
        );
    }
    ui.add_space(2.0);
    warnings_ui(ui, result, registry, enabled)
}

/// Runs a warning's fix as one transaction (one undo step).
pub fn run_fix(app: &mut PixelApp, fix: OpCall) -> Result<Value, String> {
    let r = app.execute(&[fix]);
    if let Err(e) = &r {
        app.set_status(e.clone());
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summaries_read_as_short_lines() {
        let r = Registry::builtin();
        let (subject, lines) =
            readable("panels (o:9): 2 parts (0 closed), 2 open borders, 0 degenerate faces; cleanup.auto would weld 2 vertices, fill 1 hole", &r);
        assert_eq!(subject, "panels (o:9)");
        assert_eq!(lines, ["2 parts (0 closed)", "2 open borders", "0 degenerate faces", "Clean up would weld 2 vertices, fill 1 hole"]);
        assert_eq!(readable("no subject here", &r), (String::new(), vec!["no subject here".to_owned()]));
        assert_eq!(op_names("prim.box made", &r), "Box made");
        assert_eq!(op_names("nothing.known here", &r), "nothing.known here");
    }

    #[test]
    fn warnings_with_a_fix_run_it_and_reports_change_nothing() {
        let mut app = PixelApp::new(p3d_engine::Session::new(), crate::Services::default());
        // An open box: the report finds a hole, adds no undo step, and its
        // fix runs cleanup.auto with the same settings.
        app.run("prim.box", json!({ "name": "bin" })).unwrap();
        let k = app.doc().resolve("bin").unwrap();
        let top = app.doc().object_mesh(k).unwrap().faces().last().unwrap();
        app.execute(&[OpCall::new("edit.delete_elements", json!({ "target": "bin", "select": { "faces": { "ids": [top] } } }))]).unwrap();
        let (steps, revision) = (app.session.history().len(), app.session.revision());
        app.run("selection.run", json!({ "op": "cleanup.report" })).unwrap();
        assert_eq!((app.session.history().len(), app.session.revision()), (steps, revision), "a report changes nothing");
        let result = app.result.clone().unwrap();
        assert!(result.read_only && result.worth_showing());
        let fix = result.entries[0].warnings[0].fix.clone().expect("the report offers Clean up");
        assert_eq!((fix.op.as_str(), &fix.args["target"]), ("cleanup.auto", &json!("o:1")));
        let faces = app.doc().object_mesh(k).unwrap().face_count();
        run_fix(&mut app, OpCall::new(&fix.op, fix.args)).unwrap();
        assert_eq!(app.doc().object_mesh(k).unwrap().face_count(), faces + 1, "the hole is filled");
        assert_eq!(app.session.history().len(), steps + 1, "the fix is one undo step");
        assert!(!app.result.as_ref().unwrap().read_only);
    }
}
