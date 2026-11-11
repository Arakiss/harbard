//! A row's menu (`RowMenu`, `parts.tsx`): a "⋯" button that shows while
//! the row is hovered, and the same menu on right-click. "Mark as read"
//! comes first when the row can be marked read, then the row's actions.
//! GPUI Kit's popup menu gives ↑/↓, Enter, Esc and click-outside.

use crate::i18n::{t, tv};
use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, Window, div, px,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use std::rc::Rc;

/// A row menu's least width.
const MENU_WIDTH: f32 = 220.;

/// What a menu entry does.
pub type RowRun = Rc<dyn Fn(&mut Window, &mut App)>;

/// One entry of a row's menu.
#[derive(Clone)]
pub struct RowAction {
    pub label: SharedString,
    pub run: RowRun,
}

impl RowAction {
    pub fn new(
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        RowAction {
            label: label.into(),
            run: Rc::new(run),
        }
    }
}

/// The menu with its entries.
fn fill(mut menu: PopupMenu, items: &[RowAction]) -> PopupMenu {
    menu = menu.min_w(px(MENU_WIDTH));
    for item in items {
        let run = item.run.clone();
        menu = menu.item(
            PopupMenuItem::new(item.label.clone()).on_click(move |_, window, cx| run(window, cx)),
        );
    }
    menu
}

/// The "⋯" button alone (`.hb-row-more`), opening the menu below it.
/// `label` names the row for screen readers ("More actions for {label}").
pub fn more_button(
    id: impl Into<ElementId>,
    label: SharedString,
    items: Vec<RowAction>,
    cx: &App,
) -> AnyElement {
    let _ = (label, cx);
    let items = Rc::new(items);
    Button::new(id)
        .ghost()
        .compact()
        .icon(gpui_kit::component::Icon::default().path("icons/ellipsis.svg"))
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |menu, _, _| fill(menu, &items))
        .into_any_element()
}

/// A row with its menu. Without `on_read` and `actions` the row is
/// returned as it is.
pub fn row_menu(
    id: impl Into<ElementId>,
    label: SharedString,
    on_read: Option<RowRun>,
    actions: Vec<RowAction>,
    child: AnyElement,
    cx: &App,
) -> AnyElement {
    let mut items: Vec<RowAction> = Vec::with_capacity(actions.len() + 1);
    if let Some(read) = on_read {
        items.push(RowAction {
            label: t(cx, "messages.markRead"),
            run: read,
        });
    }
    items.extend(actions);
    if items.is_empty() {
        return child;
    }
    let id: ElementId = id.into();
    let group = SharedString::from(format!("hb-row-wrap-{id:?}"));
    let aria = tv(cx, "messages.moreActions", &[("label", &label)]);
    let more = more_button(
        ElementId::Name(format!("hb-row-more-{id:?}").into()),
        aria,
        items.clone(),
        cx,
    );
    let items = Rc::new(items);
    div()
        .id(id)
        .relative()
        .group(group.clone())
        .child(child)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(crate::ui::space::SM))
                .flex()
                .items_center()
                .invisible()
                .group_hover(group, |s| s.visible())
                .child(more),
        )
        .context_menu(move |menu, _, _| fill(menu, &items))
        .into_any_element()
}
