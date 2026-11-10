//! Pieces the inbox screens share (Direct, Activity, Later, Search), drawn
//! with the console's classes: the 420px list pane (`.hb-list`), its head
//! and search field, rows (`.hb-item`), empty states (`.hb-empty`), buttons
//! (`.hb-btn`, `.hb-link-btn`, `.hb-icon-btn`), chips, the presence dot, the
//! file card, the toast, the "Hidden (N)" group and the row's ⋯ menu.

use crate::derive::cards::file_name;
use crate::derive::direct::hidden_agents;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, WorkspaceView};
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::{self, SANS, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};

/// `.hb-list`: the 420px list column.
pub fn list_pane(theme: &Theme) -> Div {
    div()
        .w(px(420.))
        .flex_none()
        .flex()
        .flex_col()
        .min_h_0()
        .bg(theme.c("list"))
        .border_r_1()
        .border_color(theme.c("line"))
}

/// `.hb-list-head`: the title, then whatever goes right of it.
pub fn list_head(title: SharedString, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(64.))
        .pl(px(22.))
        .pr(px(16.))
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::BLACK)
                .text_color(theme.c("ink"))
                .child(title),
        )
}

/// `.hb-unread`: the outlined count pill at the head's right.
pub fn unread_pill(text: SharedString, theme: &Theme) -> Div {
    div()
        .ml_auto()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(17.))
        .border_2()
        .border_color(theme.c("accent"))
        .text_color(theme.c("accent-text"))
        .font_weight(FontWeight::BLACK)
        .text_size(px(15.))
        .flex()
        .items_center()
        .child(text)
}

/// `.hb-search`: the search icon and a borderless input in a 40px box.
pub fn search_box(input: &Entity<InputState>, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(40.))
        .px(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(15.))
        .child(icon("search", 20., theme.c("ink-4")))
        .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false)))
}

/// `.hb-item`: one row of a list pane.
pub fn item_row(id: impl Into<ElementId>, active: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .gap(px(12.))
        .w_full()
        .pt(px(14.))
        .pb(px(14.))
        .pl(px(22.))
        .pr(px(16.))
        .border_b_1()
        .border_color(theme.c("line"))
        .cursor_pointer()
        .when(active, |d| d.bg(theme.c("surface")))
        .when(!active, |d| d.hover(|s| s.bg(theme.c("item-hover"))))
}

/// `.hb-item-name`.
pub fn item_name(name: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(px(16.))
        .font_weight(FontWeight::BLACK)
        .text_color(theme.c("ink"))
        .child(name.into())
}

/// `.hb-item-preview`: two lines at most.
pub fn item_preview(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div().text_color(theme.c("ink-2")).line_clamp(2).child(text.into())
}

/// `.hb-item-time`.
pub fn item_time(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .ml_auto()
        .flex_none()
        .text_size(px(12.))
        .text_color(theme.c("ink-3"))
        .child(text.into())
}

/// `.hb-count`: the filled unread count.
pub fn count(n: u64, theme: &Theme) -> Div {
    div()
        .flex_none()
        .min_w(px(26.))
        .h(px(22.))
        .px(px(8.))
        .rounded(px(11.))
        .bg(theme.c("accent-deep"))
        .text_color(theme.c("accent-ink"))
        .text_size(px(13.))
        .font_weight(FontWeight::BLACK)
        .flex()
        .items_center()
        .justify_center()
        .child(n.to_string())
}

/// `.hb-empty`: a bold title over a grey hint, centred.
pub fn empty(title: Option<SharedString>, hint: Option<SharedString>, theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(28.))
        .py(px(40.))
        .flex()
        .flex_col()
        .items_center()
        .text_center()
        .text_color(theme.c("ink-3"))
        .text_size(px(15.))
        .line_height(px(22.))
        .children(title.map(|s| {
            div()
                .mb(px(6.))
                .text_size(px(17.))
                .font_weight(FontWeight::BLACK)
                .text_color(theme.c("ink"))
                .child(s)
        }))
        .children(hint.map(|h| div().child(h)))
}

/// What a screen shows while the workspace is not read yet (`Loading`).
pub fn loading(cx: &App) -> Div {
    let theme = cx.global::<Theme>();
    let desk = Desk::get(cx).read(cx);
    match &desk.error {
        Some(error) => empty(
            Some(t(cx, "messages.serveDown")),
            Some(tv(cx, "messages.loadFailed", &[("error", error), ("command", "harbard serve")])),
            theme,
        ),
        None => empty(None, Some(t(cx, "messages.loading")), theme),
    }
}

/// `.hb-btn` (and `.hb-btn-primary`).
pub fn btn(id: impl Into<ElementId>, label: impl Into<SharedString>, primary: bool, disabled: bool, theme: &Theme) -> Stateful<Div> {
    let (bg, edge, ink, hover) = if primary {
        ("accent-deep", "accent-deep", "accent-ink", "accent-hover")
    } else {
        ("surface", "line-strong", "ink", "btn-hover")
    };
    div()
        .id(id)
        .flex_none()
        .h(px(36.))
        .px(px(16.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.c(edge))
        .bg(theme.c(bg))
        .text_color(theme.c(ink))
        .font_weight(FontWeight::BOLD)
        .text_size(px(15.))
        .flex()
        .items_center()
        .justify_center()
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(theme.c(hover))))
        .child(label.into())
}

/// `.hb-link-btn`: a bold link-coloured word.
pub fn link_btn(id: impl Into<ElementId>, label: impl Into<SharedString>, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .text_size(px(14.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("link"))
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(label.into())
}

/// `.hb-icon-btn`: a 32px outlined square with an icon.
pub fn icon_btn(id: impl Into<ElementId>, name: &str, pressed: bool, border: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(32.))
        .rounded(px(6.))
        .when(border, |d| d.border_1().border_color(theme.c("line-strong")))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(pressed, |d| d.bg(theme.c("accent-soft")).border_color(theme.c("accent")))
        .when(!pressed, |d| d.hover(|s| s.bg(theme.c("hover-2"))))
        .child(icon(name, 20., if pressed { theme.c("accent") } else { theme.c("ink-2") }))
}

/// `.hb-chip` with a tone: neutral, amber, red, green.
pub fn chip(text: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    let (bg, ink, ring) = match tone {
        "amber" => ("amber-bg", "amber-ink", None),
        "red" => ("red-bg", "red-ink", Some("red-line")),
        "green" => ("green-bg", "green-ink", Some("green-line")),
        _ => ("chip", "chip-ink", None),
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .bg(theme.c(bg))
        .text_color(theme.c(ink))
        .when_some(ring, |d, ring| d.border_1().border_color(theme.c(ring)))
        .child(text.into())
}

/// `LabelChip`: nothing when the message has no label.
pub fn label_chip(label: Option<&str>, theme: &Theme, cx: &App) -> Option<Div> {
    let (text, tone) = crate::derive::label_chip(label?, cx);
    Some(chip(text, tone, theme))
}

/// `.hb-presence-dot`: 12px, the tone's colour with its edge.
pub fn presence_dot(tone: &str, size: f32, theme: &Theme) -> Div {
    let (fill, edge) = ui::tone_colors(theme, tone);
    ui::dot(fill, edge, px(size))
}

/// `FileCard`: a local file, opened on this Mac when clicked.
pub fn file_card(id: impl Into<ElementId>, path: &str, full_width: bool, cx: &App) -> Stateful<Div> {
    let theme = cx.global::<Theme>();
    let open = path.to_string();
    div()
        .id(id)
        .mt(px(8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .when(full_width, |d| d.w_full())
        .when(!full_width, |d| d.min_w(px(240.)).max_w(px(420.)))
        .overflow_hidden()
        .px(px(16.))
        .py(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface-2"))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("file-hover")).border_color(theme.c("line-hover")))
        .child(icon("file-text", 24., theme.c("ink-2")))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(15.))
                        .text_color(theme.c("ink"))
                        .child(file_name(path).to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme.c("ink-4"))
                        .child(ui::format::short_path(path)),
                ),
        )
        .on_click(move |_, _, cx| {
            // `open` asks serve to show the file on this Mac; no refresh follows.
            post("open", serde_json::json!({ "path": open }), cx).detach();
        })
}

/// `.hb-toast`: centred at the bottom, over everything.
pub fn toast(theme: &Theme) -> Div {
    div()
        .absolute()
        .bottom(px(24.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(16.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(theme.c("st-term"))
                .text_color(theme.c("st-term-ink"))
                .shadow(theme.pop_shadow())
                .text_size(px(15.))
                .font_family(SANS),
        )
}

/// The toggle of a folded group (`.hb-hidden-toggle`): chevron and words.
pub fn fold_toggle(id: impl Into<ElementId>, label: SharedString, open: bool, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(6.))
        .w_full()
        .px(px(16.))
        .py(px(10.))
        .text_size(px(14.))
        .text_color(theme.c("ink-3"))
        .cursor_pointer()
        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 16., theme.c("ink-3")))
        .child(label)
}

/// "Hidden (N)" folded at the end of the agents, "Show" per agent.
pub fn hidden_group(view: &WorkspaceView, open: bool, on_toggle: impl Fn(&mut Window, &mut App) + 'static, cx: &App) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let hidden: Vec<&Agent> = hidden_agents(view);
    if hidden.is_empty() {
        return None;
    }
    let show_btn = |id: SharedString, label: SharedString| {
        div()
            .id(id)
            .flex_none()
            .px(px(8.))
            .py(px(2.))
            .rounded(px(4.))
            .border_1()
            .border_color(theme.c("line-strong"))
            .bg(theme.c("surface"))
            .text_size(px(13.))
            .font_weight(FontWeight::BOLD)
            .text_color(theme.c("link"))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("btn-hover")))
            .child(label)
    };
    Some(
        div()
            .flex()
            .flex_col()
            .child(
                fold_toggle("hb-hidden-toggle", tv(cx, "shell.hiddenCount", &[("count", &hidden.len().to_string())]), open, theme)
                    .on_click(move |_, window, cx| on_toggle(window, cx)),
            )
            .when(open, |d| {
                d.children(hidden.iter().map(|a| {
                    let key = a.key.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .min_h(px(32.))
                        .pl(px(38.))
                        .pr(px(16.))
                        .py(px(2.))
                        .text_size(px(14.))
                        .text_color(theme.c("ink-2"))
                        .child(ui::avatar::agent_avatar(a, 24., false, cx))
                        .child(div().flex_1().min_w_0().truncate().child(crate::derive::agent_name(a, cx)))
                        .child(
                            show_btn(SharedString::from(format!("hb-show-{}", a.key)), t(cx, "shell.show")).on_click(
                                move |_, _, cx| {
                                    post("show-agent", serde_json::json!({ "agent": key }), cx).detach();
                                },
                            ),
                        )
                }))
                .child(div().ml(px(38.)).mt(px(4.)).mb(px(10.)).child(
                    show_btn("hb-show-all".into(), t(cx, "shell.showAll")).on_click(|_, _, cx| {
                        post("show-all-agents", serde_json::json!({}), cx).detach();
                    }),
                ))
            })
            .into_any_element(),
    )
}

/// A row action: its words and what it does.
pub type RowAction = (SharedString, std::rc::Rc<dyn Fn(&mut Window, &mut App)>);

/// `RowMenu`: right-click the row, or its ⋯ on hover, for "Mark as read"
/// and the row's actions.
pub fn row_menu(id: impl Into<ElementId>, row: impl IntoElement, actions: Vec<RowAction>, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let id: ElementId = id.into();
    let group = SharedString::from(format!("{id:?}"));
    let items = std::rc::Rc::new(actions);
    let menu_items = items.clone();
    div()
        .id(id)
        .relative()
        .group(group.clone())
        .child(row)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right(px(6.))
                .flex()
                .items_center()
                .opacity(0.)
                .group_hover(group, |s| s.opacity(1.))
                .child(
                    div()
                        .size(px(24.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme.c("list"))
                        .hover(|s| s.bg(theme.c("hover-2")))
                        .child(icon("ellipsis", 16., theme.c("ink-2"))),
                ),
        )
        .context_menu(move |mut menu, _, _| {
            for (label, run) in menu_items.iter() {
                let run = run.clone();
                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| run(window, cx)));
            }
            menu
        })
        .into_any_element()
}
