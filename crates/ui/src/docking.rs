//! Panel identities and layouts around craft-ui's docking renderer. The
//! viewport is protected: it cannot be closed, floated or tabbed with other
//! panels. Every dock change goes through [`Workspace::apply`], also used by
//! the `ui.dock` command, so agents can rearrange panels too.

use craft_ui::docking::{Action, DockArea, DockStyle, Layout, Location, Node, PanelLimits, Permissions, Placement, StackEntry, Zone};
use craft_ui::layout::{SplitAxis, SplitSize};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::i18n::t;
use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Panel {
    Viewport,
    Outliner,
    Properties,
    Health,
    History,
    Console,
    /// The editors of the other workspaces.
    UvEditor,
    Timeline,
    Bake,
    Export,
    /// The feature template library (ears, bolts, vents...).
    Features,
}

impl Panel {
    pub const ALL: &[Panel] = &[
        Panel::Viewport,
        Panel::Outliner,
        Panel::Properties,
        Panel::Health,
        Panel::History,
        Panel::Console,
        Panel::UvEditor,
        Panel::Timeline,
        Panel::Bake,
        Panel::Export,
        Panel::Features,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Panel::Viewport => "viewport",
            Panel::Outliner => "outliner",
            Panel::Properties => "properties",
            Panel::Health => "health",
            Panel::History => "history",
            Panel::Console => "console",
            Panel::UvEditor => "uvEditor",
            Panel::Timeline => "timeline",
            Panel::Bake => "bake",
            Panel::Export => "export",
            Panel::Features => "features",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Panel::Viewport => t("Viewport"),
            Panel::Outliner => t("Outliner"),
            Panel::Properties => t("Properties"),
            Panel::Health => t("Health"),
            Panel::History => t("History"),
            Panel::Console => t("Console"),
            Panel::UvEditor => t("UV Editor"),
            Panel::Timeline => t("Timeline"),
            Panel::Bake => t("Bake"),
            Panel::Export => t("Export"),
            Panel::Features => t("Features"),
        }
    }

    fn default_side(self) -> Zone {
        match self {
            Panel::History | Panel::Console | Panel::Timeline => Zone::Bottom,
            _ => Zone::Right,
        }
    }

    pub fn parse(id: &str) -> Result<Panel, String> {
        Panel::ALL.iter().copied().find(|p| p.id() == id).ok_or_else(|| format!("unknown panel `{id}`"))
    }
}

pub fn permissions(panel: &Panel) -> Permissions {
    if *panel == Panel::Viewport { Permissions::PROTECTED } else { Permissions::default() }
}

fn tabs(panels: &[Panel]) -> Node<Panel> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(axis: SplitAxis, size: SplitSize, first: Node<Panel>, second: Node<Panel>) -> Node<Panel> {
    Node::Split { axis, size, first: Box::new(first), second: Box::new(second) }
}

/// A dock layout plus where closed panels go back to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub layout: Layout<Panel>,
    pub hidden: Vec<(Panel, Location<Panel>)>,
}

/// The preset layout of a workspace (project spec 20.4): which editors
/// sit around the viewport for that kind of work.
pub fn preset(kind: crate::WorkspaceKind) -> Workspace {
    use crate::WorkspaceKind as W;
    let stack = |entries: &[(Panel, f32)]| Node::Stack {
        entries: entries.iter().map(|&(panel, h)| StackEntry { panel, open: true, height: Some(h) }).collect(),
    };
    let bottom = |panels: &[Panel], h: f32| split(SplitAxis::Vertical, SplitSize::FixedSecond(h), tabs(&[Panel::Viewport]), tabs(panels));
    let root = match kind {
        W::Model => return Workspace::default(),
        // The viewport and the UV editor side by side.
        W::Uv => split(
            SplitAxis::Horizontal,
            SplitSize::FixedSecond(320.0),
            split(SplitAxis::Horizontal, SplitSize::Ratio(0.5), bottom(&[Panel::History, Panel::Console], 150.0), tabs(&[Panel::UvEditor])),
            stack(&[(Panel::Outliner, 260.0), (Panel::Properties, 520.0)]),
        ),
        W::Bake => split(
            SplitAxis::Horizontal,
            SplitSize::FixedSecond(340.0),
            bottom(&[Panel::History, Panel::Console], 150.0),
            stack(&[(Panel::Outliner, 240.0), (Panel::Bake, 360.0), (Panel::Health, 220.0)]),
        ),
        W::Rig => split(
            SplitAxis::Horizontal,
            SplitSize::FixedSecond(320.0),
            bottom(&[Panel::History, Panel::Console], 150.0),
            stack(&[(Panel::Outliner, 460.0), (Panel::Properties, 360.0)]),
        ),
        // The timeline under the viewport.
        W::Animate => split(
            SplitAxis::Horizontal,
            SplitSize::FixedSecond(320.0),
            bottom(&[Panel::Timeline, Panel::History], 200.0),
            stack(&[(Panel::Outliner, 300.0), (Panel::Properties, 460.0)]),
        ),
        W::Export => split(
            SplitAxis::Horizontal,
            SplitSize::FixedSecond(340.0),
            bottom(&[Panel::History, Panel::Console], 150.0),
            stack(&[(Panel::Export, 420.0), (Panel::Health, 240.0), (Panel::Outliner, 200.0)]),
        ),
    };
    Workspace { layout: Layout { root: Some(root), floating: Vec::new() }, hidden: Vec::new() }
}

impl Default for Workspace {
    /// The Model layout: the viewport with History and Console tabs below
    /// it, Outliner and Properties stacked on the right. The tool column on
    /// the left is window chrome, not a dock panel.
    fn default() -> Self {
        let center = split(SplitAxis::Vertical, SplitSize::FixedSecond(170.0), tabs(&[Panel::Viewport]), tabs(&[Panel::History, Panel::Console]));
        let right = Node::Stack {
            entries: vec![
                StackEntry { panel: Panel::Outliner, open: true, height: Some(300.0) },
                StackEntry { panel: Panel::Properties, open: true, height: Some(460.0) },
                StackEntry { panel: Panel::Health, open: true, height: Some(180.0) },
            ],
        };
        let root = split(SplitAxis::Horizontal, SplitSize::FixedSecond(320.0), center, right);
        // The Features panel opens in the right column, below the Outliner.
        let features = Location {
            floating: None,
            anchor: Some(Panel::Outliner),
            placement: Placement::Split(Zone::Center),
            size: None,
            stack: Some((true, Some(420.0))),
            sibling: Vec::new(),
        };
        Workspace { layout: Layout { root: Some(root), floating: Vec::new() }, hidden: vec![(Panel::Features, features)] }
    }
}

impl Workspace {
    /// Checks the layout and the viewport's protection.
    pub fn validate(&self) -> Result<(), String> {
        self.layout.validate().map_err(|e| e.to_string())?;
        if !self.layout.contains(&Panel::Viewport) {
            return Err("the layout must contain the viewport".into());
        }
        if self.layout.floating.iter().any(|g| g.panels.contains(&Panel::Viewport)) {
            return Err("the viewport cannot float".into());
        }
        let mut nodes: Vec<&Node<Panel>> = self.layout.root.iter().collect();
        while let Some(node) = nodes.pop() {
            match node {
                Node::Split { first, second, .. } => {
                    nodes.push(first);
                    nodes.push(second);
                }
                Node::Tabs { panels, .. } if panels.contains(&Panel::Viewport) && panels.len() != 1 => {
                    return Err("the viewport cannot share a tab group".into());
                }
                Node::Stack { entries } if entries.iter().any(|e| e.panel == Panel::Viewport) => {
                    return Err("the viewport cannot be collapsed".into());
                }
                _ => {}
            }
        }
        if self.hidden.iter().any(|(p, _)| *p == Panel::Viewport || self.layout.contains(p)) || self.hidden.len() > Panel::ALL.len() {
            return Err("a hidden panel is invalid".into());
        }
        Ok(())
    }

    /// Applies one dock action as a transaction: all or nothing.
    pub fn apply(&mut self, action: Action<Panel>) -> Result<(), String> {
        self.validate()?;
        let mut next = self.clone();
        if let Action::Close { panel } = &action {
            let location = next.layout.location(panel).map_err(|e| e.to_string())?;
            next.hidden.retain(|(id, _)| id != panel);
            next.hidden.push((*panel, location));
        } else if let Action::Open { panel, .. } | Action::OpenAt { panel, .. } = &action {
            next.hidden.retain(|(id, _)| id != panel);
        }
        next.layout.apply_with_permissions(action, permissions).map_err(|e| e.to_string())?;
        next.validate()?;
        *self = next;
        Ok(())
    }

    pub fn is_visible(&self, panel: Panel) -> bool {
        self.layout.contains(&panel)
    }

    /// Shows or hides a panel; a reopened panel returns to where it was.
    pub fn set_visible(&mut self, panel: Panel, visible: bool) -> Result<(), String> {
        if self.layout.contains(&panel) == visible {
            return Ok(());
        }
        if !visible {
            return self.apply(Action::Close { panel });
        }
        self.validate()?;
        let mut next = self.clone();
        let location = next.hidden.iter().find(|(id, _)| *id == panel).map(|(_, l)| l.clone());
        next.hidden.retain(|(id, _)| *id != panel);
        let restored = location.as_ref().is_some_and(|l| next.layout.restore(panel, l).is_ok());
        if !restored {
            next.layout
                .apply_with_permissions(
                    Action::OpenAt { panel, anchor: Panel::Viewport, placement: Placement::Split(panel.default_side()) },
                    permissions,
                )
                .map_err(|e| e.to_string())?;
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Runs a `ui.dock` command: `{"action": <craft-ui action>}`, or
    /// `{"operation": "open"|"close"|"activate", "panel": "<id>"}`.
    pub fn command(&mut self, params: &Value) -> Result<Value, String> {
        if let Some(action) = params.get("action") {
            let action: Action<Panel> = serde_json::from_value(action.clone()).map_err(|e| e.to_string())?;
            self.apply(action)?;
        } else {
            let op = params.get("operation").and_then(Value::as_str).ok_or("operation is required: open, close or activate")?;
            let panel = Panel::parse(params.get("panel").and_then(Value::as_str).ok_or("panel is required")?)?;
            match op {
                "open" => {
                    self.set_visible(panel, true)?;
                    self.apply(Action::Activate { panel })?;
                }
                "close" => self.set_visible(panel, false)?,
                "activate" => self.apply(Action::Activate { panel })?,
                other => return Err(format!("unknown operation `{other}`: use open, close or activate")),
            }
        }
        Ok(json!({ "docking": self }))
    }
}

fn limits(panel: &Panel) -> PanelLimits {
    let (min, max_width) = match panel {
        Panel::Viewport => (egui::vec2(240.0, 160.0), 1_000_000.0),
        Panel::History | Panel::Console | Panel::Timeline => (egui::vec2(200.0, 80.0), 1_000_000.0),
        Panel::UvEditor => (egui::vec2(240.0, 160.0), 1_000_000.0),
        Panel::Outliner | Panel::Properties | Panel::Health | Panel::Bake | Panel::Export | Panel::Features => (egui::vec2(220.0, 90.0), 720.0),
    };
    PanelLimits { min, max: egui::vec2(max_width, 1_000_000.0) }
}

/// Shows the dock area; panel bodies come from `body`. Returns the dock
/// actions the user requested this frame.
pub fn show(ui: &mut egui::Ui, workspace: &Workspace, body: impl FnMut(&mut egui::Ui, &Panel)) -> Vec<Action<Panel>> {
    let tk = Tokens::get();
    let mut style = DockStyle::from_ui(ui);
    style.min_pane = 80.0;
    style.background = tk.panel;
    style.tab_background = tk.chrome;
    style.active_background = tk.tab_active;
    style.text = tk.text;
    style.inactive_text = tk.text_dim;
    style.border = egui::Stroke::new(1.0, tk.border);
    style.accent = tk.accent;
    style.font = crate::theme::body();
    let out = DockArea::new(egui::Id::new("p3d-docking")).show_with_limits(
        ui,
        &workspace.layout,
        &style,
        |p| p.label().to_owned(),
        permissions,
        limits,
        body,
    );
    if let Some(e) = out.error {
        tracing::warn!(error = ?e, "dock layout not shown");
    }
    out.actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_valid_and_viewport_is_protected() {
        let mut w = Workspace::default();
        w.validate().unwrap();
        // The Model layout: every panel but the other workspaces' editors
        // and the Features panel, which opens on demand.
        let editors = [Panel::UvEditor, Panel::Timeline, Panel::Bake, Panel::Export, Panel::Features];
        assert!(Panel::ALL.iter().all(|p| w.is_visible(*p) != editors.contains(p)));
        // It opens in the right column, below the Outliner.
        let mut f = Workspace::default();
        f.set_visible(Panel::Features, true).unwrap();
        f.validate().unwrap();
        let at = f.layout.location(&Panel::Features).unwrap();
        assert_eq!(at.anchor, Some(Panel::Outliner), "{at:?}");
        assert!(w.apply(Action::Close { panel: Panel::Viewport }).is_err());
        assert!(w.set_visible(Panel::Viewport, false).is_err());
        w.validate().unwrap();
    }

    #[test]
    fn closed_panels_come_back_where_they_were() {
        let mut w = Workspace::default();
        let before = w.clone();
        w.command(&json!({ "operation": "close", "panel": "properties" })).unwrap();
        assert!(!w.is_visible(Panel::Properties));
        w.command(&json!({ "operation": "open", "panel": "properties" })).unwrap();
        assert!(w.is_visible(Panel::Properties));
        assert_eq!(w.layout, before.layout);
        assert!(w.command(&json!({ "operation": "close", "panel": "nope" })).is_err());
        // Round trip through JSON (the persisted UI state).
        let back: Workspace = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
        assert_eq!(back, w);
    }
}
