//! Automation hooks: every interactive element records its stable egui id,
//! label, role and rectangle each frame, so agents and tests can find and
//! drive it through the control server (`ui.elements`).

use serde_json::{Value, json};

/// Most elements recorded per frame.
const MAX_WIDGETS: usize = 4096;

#[derive(Clone, Debug)]
pub struct Widget {
    pub id: egui::Id,
    pub label: String,
    pub role: &'static str,
    pub rect: egui::Rect,
    pub enabled: bool,
    pub selected: Option<bool>,
}

fn widgets_id() -> egui::Id {
    egui::Id::new("p3d-observed-widgets")
}

/// Forgets last frame's elements.
pub(crate) fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Vec<Widget>>(widgets_id()));
}

/// Records a real response. Never changes focus or input routing.
pub(crate) fn record_widget(ui: &egui::Ui, response: &egui::Response, label: &str, role: &'static str, selected: Option<bool>) {
    if !response.rect.is_finite() || !response.rect.is_positive() || !ui.is_rect_visible(response.rect) {
        return;
    }
    let w = Widget { id: response.id, label: label.chars().take(256).collect(), role, rect: response.rect, enabled: response.enabled(), selected };
    ui.ctx().data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<Widget>>(widgets_id());
        if list.len() < MAX_WIDGETS {
            list.push(w);
        }
    });
}

/// This frame's elements: the last record of each id, in recording order.
///
/// Complexity: O(n log n) in recorded elements.
pub fn widgets(ctx: &egui::Context) -> Vec<Widget> {
    let mut list = ctx.data_mut(|d| d.get_temp::<Vec<Widget>>(widgets_id()).unwrap_or_default());
    let mut order: Vec<(u64, usize)> = list.iter().enumerate().map(|(i, w)| (w.id.value(), i)).collect();
    order.sort_by_key(|&(id, i)| (id, std::cmp::Reverse(i)));
    order.dedup_by_key(|&mut (id, _)| id);
    let mut keep: Vec<usize> = order.into_iter().map(|(_, i)| i).collect();
    keep.sort_unstable();
    let mut out = Vec::with_capacity(keep.len());
    for i in keep {
        out.push(std::mem::replace(
            &mut list[i],
            Widget { id: egui::Id::NULL, label: String::new(), role: "", rect: egui::Rect::NOTHING, enabled: false, selected: None },
        ));
    }
    out
}

pub fn widget_json(w: &Widget) -> Value {
    json!({
        "id": format!("{:016x}", w.id.value()),
        "label": w.label,
        "role": w.role,
        "rect": [w.rect.left(), w.rect.top(), w.rect.width(), w.rect.height()],
        "enabled": w.enabled,
        "selected": w.selected,
    })
}
