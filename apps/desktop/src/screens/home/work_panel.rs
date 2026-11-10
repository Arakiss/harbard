//! "In progress" above a channel's stream (`WorkPanel`, `work-panel.tsx`).

use crate::derive::work_items::WorkItem;
use crate::model::WorkspaceView;
use gpui::{AnyElement, App, Window};
use std::rc::Rc;

/// Chooses a work item (`Some(key)`) or the whole channel (`None`).
pub type SelectWork = Rc<dyn Fn(Option<String>, &mut Window, &mut App)>;

/// The panel; `None` when there is no work item.
pub fn work_panel(view: &WorkspaceView, items: &[WorkItem], selected: Option<&str>, on_select: SelectWork, cx: &App) -> Option<AnyElement> {
    let _ = (view, items, selected, on_select, cx);
    None
}
