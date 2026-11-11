//! Pieces the inbox screens share (Direct, Activity, Later, Search): the
//! list pane and its header, rows of one structure and height, the unread
//! dot, empty states, buttons, the toast, the "Hidden (N)" group, the row's
//! ⋯ menu, and the keys every list answers to (`DESIGN.md` K4).

use crate::chat::RowMenuItem;
use crate::derive::direct::hidden_agents;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, WorkspaceView};
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::{
    self, SANS, avatar_size, control, icon, icon_size, leading, radius, row, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, Div, ElementId, Entity, FocusHandle, Global, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;

/// The list column's width (`.hb-list`).
pub const LIST_W: f32 = 420.;
/// A list row (Z2): a title line and two lines under it, `LG` above and
/// below. Every row of every inbox list is this tall.
pub const ROW_H: f32 = leading::UI + 2. * leading::SMALL + 2. * space::LG;
/// The unread dot (F2).
const UNREAD_DOT: f32 = 8.;
/// The widest an empty state's text runs (F3).
const EMPTY_W: f32 = 320.;

/// The handle of the screen's list. Controls hand keyboard focus back to it
/// when they are clicked, so J and K keep moving the focus row.
#[derive(Clone)]
pub struct ListFocus(pub FocusHandle);

impl Global for ListFocus {}

/// What a key asks of a list (K4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListKey {
    Next,
    Prev,
    Open,
    Back,
    Read,
}

/// `J`/`K` and the arrows move, `Enter` opens, `Esc` goes back, `U` marks
/// read. Keys with a modifier are someone else's.
pub fn list_key(ev: &KeyDownEvent) -> Option<ListKey> {
    let k = &ev.keystroke;
    if k.modifiers.platform || k.modifiers.control || k.modifiers.alt || k.modifiers.shift {
        return None;
    }
    match k.key.as_str() {
        "j" | "down" => Some(ListKey::Next),
        "k" | "up" => Some(ListKey::Prev),
        "enter" => Some(ListKey::Open),
        "escape" => Some(ListKey::Back),
        "u" => Some(ListKey::Read),
        _ => None,
    }
}

/// Where the focus row lands after `Next` or `Prev` in a list of `len`.
pub fn step(cursor: Option<usize>, key: ListKey, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (cursor, key) {
        (None, ListKey::Prev) => len - 1,
        (None, _) => 0,
        (Some(i), ListKey::Prev) => i.saturating_sub(1),
        (Some(i), _) => (i + 1).min(len - 1),
    })
}

/// A control with a hover: reachable with Tab, ringed under keyboard focus
/// (F1), and giving focus back to the list when the pointer clicks it.
pub fn control(el: Stateful<Div>, theme: &Theme) -> Stateful<Div> {
    let ring = ui::focus_ring(theme, false);
    el.tab_index(0)
        .focus_visible(move |s| s.shadow(vec![ring]))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            if let Some(list) = cx.try_global::<ListFocus>().cloned() {
                window.focus(&list.0, cx);
            }
        })
}

/// A tooltip: the name, then its keys (K2, K3).
pub fn tip(
    label: impl Into<SharedString>,
    keys: &'static [&'static str],
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label: SharedString = label.into();
    move |window, cx| {
        let label = label.clone();
        Tooltip::element(move |_, cx| {
            let theme = cx.global::<Theme>();
            div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .child(ui::text(
                    label.clone(),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink"),
                ))
                .when(!keys.is_empty(), |d| {
                    d.child(ui::kbd_keys(keys.iter().copied(), theme))
                })
        })
        .build(window, cx)
    }
}

/// `.hb-list`: the list column.
pub fn list_pane(theme: &Theme) -> Div {
    div()
        .w(px(LIST_W))
        .flex_none()
        .flex()
        .flex_col()
        .min_h_0()
        .bg(theme.c("list"))
        .border_r_1()
        .border_color(theme.c("line"))
}

/// `.hb-list-head`: the pane's title on the shared 56 px baseline (T4),
/// then whatever goes right of it.
pub fn list_head(title: SharedString, theme: &Theme) -> Div {
    ui::pane_header(title, theme)
}

/// A count said once in the header, as plain text (`.hb-unread`).
pub fn head_note(text: SharedString, theme: &Theme) -> Div {
    ui::text(text, size::META, weight::REGULAR, theme.c("ink-3"))
        .ml_auto()
        .flex_none()
}

/// `.hb-search`: the search icon and a borderless input in one field.
pub fn search_box(input: &Entity<InputState>, focused: bool, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(control::MD))
        .pl(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(if focused {
            theme.c("focus-line")
        } else {
            theme.c("line-strong")
        })
        .bg(theme.c("surface"))
        .child(icon("search", icon_size::ROW, theme.c("ink-3")))
        // The input pads its own text, which is the gap after the icon.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).appearance(false)),
        )
}

/// How a row stands in its list.
#[derive(Clone, Copy, Default)]
pub struct RowState {
    /// The open item (`selected`).
    pub active: bool,
    /// The keyboard's focus row (an inset ring).
    pub cursor: bool,
    /// Unread with no count to show: the dot at the leading edge.
    pub dot: bool,
}

/// `.hb-item`: one row of a list pane, [`ROW_H`] tall, starting at the gutter.
pub fn item_row(id: impl Into<ElementId>, state: RowState, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .gap(px(space::LG))
        .w_full()
        .h(px(ROW_H))
        .px(px(space::GUTTER))
        .py(px(space::LG))
        .border_b_1()
        .border_color(theme.c("line"))
        .cursor_pointer()
        .when(state.active, |d| d.bg(theme.c("selected")))
        .when(!state.active, |d| d.hover(|s| s.bg(theme.c("hover"))))
        .when(state.cursor, |d| {
            d.shadow(vec![ui::focus_ring(theme, true)])
        })
        .when(state.dot, |d| d.child(unread_dot(theme)))
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            if let Some(list) = cx.try_global::<ListFocus>().cloned() {
                window.focus(&list.0, cx);
            }
        })
}

/// The unread dot, centred in the gutter before the avatar (F2).
fn unread_dot(theme: &Theme) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(space::SM))
        .flex()
        .items_center()
        .child(
            div()
                .size(px(UNREAD_DOT))
                .rounded_full()
                .bg(theme.c("accent")),
        )
}

/// The slot an avatar fills, for a row about no agent: one icon, alone.
pub fn icon_slot(name: &str, color: gpui::Hsla) -> Div {
    div()
        .size(px(avatar_size::ROW))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(icon(name, icon_size::RAIL, color))
}

/// The text column of a row, beside its avatar.
pub fn item_body() -> Div {
    div().flex_1().min_w_0().flex().flex_col()
}

/// The first line of a row: the title, then chips and the trailing mark.
pub fn item_line() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .h(px(leading::UI))
}

/// `.hb-item-name`: the row's title, 700 while unread (F2).
pub fn item_title(name: impl Into<SharedString>, unread: bool, theme: &Theme) -> Div {
    ui::text(
        name,
        size::UI,
        if unread {
            weight::STRONG
        } else {
            weight::REGULAR
        },
        theme.c("ink"),
    )
    .min_w_0()
    .truncate()
}

/// Words after the title on its line (a role, a place): never the first to go.
pub fn item_aside(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::META, weight::REGULAR, theme.c("ink-3"))
        .min_w_0()
        .truncate()
}

/// `.hb-item-preview`: `lines` lines at most, ended with an ellipsis (T7).
pub fn item_preview(text: impl Into<SharedString>, lines: usize, theme: &Theme) -> Div {
    let text = ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-2"));
    if lines == 1 {
        text.truncate()
    } else {
        text.line_clamp(lines).text_ellipsis()
    }
}

/// `.hb-item-time`: the trailing mark of a row's first line.
pub fn item_time(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::META, weight::REGULAR, theme.c("ink-3"))
        .ml_auto()
        .flex_none()
}

/// `.hb-count`: the unread count, at a row's trailing edge or beside a tab.
pub fn count(n: u64, theme: &Theme) -> Div {
    ui::count_badge(n, theme.c("accent-deep"), theme.c("accent-ink")).flex_none()
}

/// `.hb-empty` (F3): a line and a sentence, centred, `BREAK` below the header.
pub fn empty(title: Option<SharedString>, hint: Option<SharedString>, theme: &Theme) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(space::XS))
        .pt(px(space::BREAK))
        .px(px(space::GUTTER))
        .children(title.map(|s| {
            ui::text(s, size::UI, weight::STRONG, theme.c("ink-2"))
                .max_w(px(EMPTY_W))
                .text_center()
        }))
        .children(hint.map(|h| {
            ui::text(h, size::UI, weight::REGULAR, theme.c("ink-3"))
                .max_w(px(EMPTY_W))
                .text_center()
        }))
}

/// What a screen shows while the workspace is not read yet (`Loading`).
pub fn loading(cx: &App) -> Div {
    let theme = cx.global::<Theme>();
    let desk = Desk::get(cx).read(cx);
    match &desk.error {
        Some(error) => empty(
            Some(t(cx, "messages.serveDown")),
            Some(tv(
                cx,
                "messages.loadFailed",
                &[("error", error), ("command", "harbard serve")],
            )),
            theme,
        ),
        None => empty(None, Some(t(cx, "messages.loading")), theme),
    }
}

/// `.hb-btn` (and `.hb-btn-primary`): `control::MD`, 14/700.
pub fn btn(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let (bg, edge, ink, hover) = if primary {
        ("accent-deep", "accent-deep", "accent-ink", "accent-hover")
    } else {
        ("surface", "line-strong", "ink", "btn-hover")
    };
    let el = div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c(edge))
        .bg(theme.c(bg))
        .child(ui::text(label, size::UI, weight::STRONG, theme.c(ink)).whitespace_nowrap());
    if disabled {
        el.opacity(0.5)
    } else {
        control(el, theme)
            .cursor_pointer()
            .hover(|s| s.bg(theme.c(hover)))
    }
}

/// `.hb-link-btn`: a quiet `control::SM` button, an optional icon and words.
/// Its label sits on the line its row starts at; the hover wash reaches past it.
pub fn quiet_btn(
    id: impl Into<ElementId>,
    icon_name: Option<&str>,
    label: impl Into<SharedString>,
    theme: &Theme,
) -> Stateful<Div> {
    control(
        div()
            .id(id)
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .h(px(control::SM))
            .px(px(space::MD))
            .rounded(px(radius::BASE))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover-2")))
            .children(icon_name.map(|n| icon(n, icon_size::ROW, theme.c("ink-3"))))
            .child(
                ui::text(label, size::UI, weight::REGULAR, theme.c("ink-2")).whitespace_nowrap(),
            ),
        theme,
    )
}

/// `.hb-icon-btn`: a `control::SM` square with one icon. Give it a tooltip.
pub fn icon_btn(
    id: impl Into<ElementId>,
    name: &str,
    pressed: bool,
    theme: &Theme,
) -> Stateful<Div> {
    control(
        div()
            .id(id)
            .flex_none()
            .size(px(control::SM))
            .rounded(px(radius::BASE))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(pressed, |d| d.bg(theme.c("control")))
            .when(!pressed, |d| d.hover(|s| s.bg(theme.c("hover-2"))))
            .child(icon(
                name,
                icon_size::ROW,
                if pressed {
                    theme.c("ink")
                } else {
                    theme.c("ink-3")
                },
            )),
        theme,
    )
}

/// `LabelChip`: nothing when the message has no label.
pub fn label_chip(label: Option<&str>, theme: &Theme, cx: &App) -> Option<Div> {
    let (text, tone) = crate::derive::label_chip(label?, cx);
    Some(ui::chip(text, tone, theme))
}

/// `.hb-presence-dot`: the tone's colour with its edge.
pub fn presence_dot(tone: &str, theme: &Theme) -> Div {
    let (fill, edge) = ui::tone_colors(theme, tone);
    ui::dot(fill, edge, px(UNREAD_DOT))
}

/// `.hb-toast`: centred at the bottom, over everything.
pub fn toast(theme: &Theme, content: impl IntoElement) -> Div {
    div()
        .absolute()
        .bottom(px(space::SECTION))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::XL))
                .h(px(control::LG))
                .px(px(space::XL))
                .rounded(px(radius::MD))
                .bg(theme.c("st-term"))
                .text_color(theme.c("st-term-ink"))
                .shadow(theme.pop_shadow())
                .font_family(SANS)
                .text_size(px(size::UI))
                .line_height(px(leading::UI))
                .child(content),
        )
}

/// The toggle of a folded group (`.hb-hidden-toggle`): chevron and words.
pub fn fold_toggle(
    id: impl Into<ElementId>,
    label: SharedString,
    open: bool,
    theme: &Theme,
) -> Stateful<Div> {
    control(
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(space::SM))
            .w_full()
            .h(px(control::MD))
            .px(px(space::GUTTER))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover")))
            .child(icon(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                icon_size::INLINE,
                theme.c("ink-3"),
            ))
            .child(ui::text(
                label,
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-3"),
            )),
        theme,
    )
}

/// "Hidden (N)" folded at the end of the agents, "Show" per agent.
pub fn hidden_group(
    view: &WorkspaceView,
    open: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let hidden: Vec<&Agent> = hidden_agents(view);
    if hidden.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_col()
            .child(
                fold_toggle(
                    "hb-hidden-toggle",
                    tv(
                        cx,
                        "shell.hiddenCount",
                        &[("count", &hidden.len().to_string())],
                    ),
                    open,
                    theme,
                )
                .on_click(move |_, window, cx| on_toggle(window, cx)),
            )
            .when(open, |d| {
                d.children(hidden.iter().map(|a| {
                    let key = a.key.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .h(px(control::MD))
                        .pl(px(space::GUTTER))
                        .pr(px(space::LG))
                        .child(ui::avatar::agent_avatar(a, avatar_size::STACK, false, cx))
                        .child(
                            ui::text(
                                crate::derive::agent_name(a, cx),
                                size::SMALL,
                                weight::REGULAR,
                                theme.c("ink-2"),
                            )
                            .flex_1()
                            .min_w_0()
                            .truncate(),
                        )
                        .child(
                            quiet_btn(
                                SharedString::from(format!("hb-show-{}", a.key)),
                                None,
                                t(cx, "shell.show"),
                                theme,
                            )
                            .on_click(move |_, _, cx| {
                                post("show-agent", serde_json::json!({ "agent": key }), cx)
                                    .detach();
                            }),
                        )
                }))
                .child(div().flex().pl(px(space::LG)).pb(px(space::MD)).child(
                    quiet_btn("hb-show-all", None, t(cx, "shell.showAll"), theme).on_click(
                        |_, _, cx| {
                            post("show-all-agents", serde_json::json!({}), cx).detach();
                        },
                    ),
                ))
            })
            .into_any_element(),
    )
}

/// `RowMenu`: right-click the row, or its ⋯ on hover (`chat::row_menu`),
/// for "Mark as read" and the row's actions.
pub fn row_menu(
    id: impl Into<ElementId>,
    row: impl IntoElement,
    items: Vec<RowMenuItem>,
    cx: &App,
) -> AnyElement {
    let id: ElementId = id.into();
    let group = SharedString::from(format!("{id:?}"));
    let more = crate::chat::row_menu(
        SharedString::from(format!("{id:?}-more")),
        items.clone(),
        cx,
    );
    div()
        .id(id)
        .relative()
        .group(group.clone())
        .child(row)
        .child(
            div()
                .absolute()
                .bottom(px(space::MD))
                .right(px(space::LG))
                .flex()
                .items_center()
                .opacity(0.)
                .group_hover(group, |s| s.opacity(1.))
                .child(more),
        )
        .context_menu(move |mut menu, _, _| {
            for item in &items {
                let run = item.action.clone();
                menu = menu.item(
                    PopupMenuItem::new(item.label.clone())
                        .on_click(move |_, window, cx| run(window, cx)),
                );
            }
            menu
        })
        .into_any_element()
}

/// A header beside a list pane (T4): `row::HEADER`, the title at the gutter.
pub fn detail_head(theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .h(px(row::HEADER))
        .px(px(space::GUTTER))
        .border_b_1()
        .border_color(theme.c("line"))
}

/// The header of a conversation (T4): the robot, the name as the pane's
/// title on the shared baseline with what it is beside it, and its status
/// at the trailing side, before whatever the caller adds.
pub fn agent_head(
    avatar: AnyElement,
    name: AnyElement,
    aside: Option<SharedString>,
    status: Div,
    theme: &Theme,
) -> Div {
    detail_head(theme)
        .gap(px(space::LG))
        .child(avatar)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_baseline()
                .gap(px(space::MD))
                .child(name)
                .children(aside.map(|a| {
                    ui::text(a, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                        .min_w_0()
                        .truncate()
                })),
        )
        .child(
            status
                .flex()
                .items_center()
                .gap(px(space::SM))
                .min_w_0()
                .max_w(gpui::relative(0.5))
                .overflow_hidden(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focus_row_starts_at_an_end_and_stops_at_the_other() {
        assert_eq!(step(None, ListKey::Next, 3), Some(0));
        assert_eq!(step(None, ListKey::Prev, 3), Some(2));
        assert_eq!(step(Some(0), ListKey::Prev, 3), Some(0));
        assert_eq!(step(Some(1), ListKey::Next, 3), Some(2));
        assert_eq!(step(Some(2), ListKey::Next, 3), Some(2));
        assert_eq!(step(Some(1), ListKey::Next, 0), None);
    }

    #[test]
    fn every_row_is_as_tall_as_three_lines_and_their_padding() {
        assert_eq!(ROW_H, 80.);
    }
}
