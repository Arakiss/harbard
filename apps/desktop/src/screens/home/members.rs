//! Who has worked in a channel (`ChannelMembers`): the avatar stack and its
//! popover of members working now and earlier.

use super::header::{Dismiss, eid, pop_surface, pop_title, popover, tip};
use crate::chat::focusable;
use crate::derive::{self, members as m, workspace as ws};
use crate::desk::Desk;
use crate::model::{Agent, Channel, WorkspaceView};
use crate::nav::{self, Route};
use crate::robot::{self, Ring};
use crate::theme::Theme;
use crate::ui::{self, avatar_size, control, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};

/// The members popover's width and tallest height.
const POP: (f32, f32) = (360., 520.);

/// The members button with its popover; `None` when nobody has worked here.
pub fn members_button(view: &WorkspaceView, channel: &Channel, cx: &App) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let members = m::channel_members(view, channel);
    if members.total == 0 {
        return None;
    }
    let label = m::members_label(&members, &channel.id, cx);
    let surface = theme.c("surface");
    let stack = div()
        .flex()
        .children(members.stack.iter().enumerate().map(|(i, a)| {
            div().when(i > 0, |d| d.ml(px(-space::XS))).child(
                robot::robot(derive::robot_name(a).to_string(), avatar_size::STACK)
                    .agent(a)
                    .dot(false)
                    .tile_ring(surface),
            )
        }));
    // Who works now is in the header line; the button keeps the total.
    let button = focusable(
        div()
            .id(eid("hb-members-button", &channel.id))
            .flex()
            .items_center()
            .gap(px(space::SM))
            .whitespace_nowrap()
            .h(px(control::SM))
            .pl(px(space::SM))
            .pr(px(space::MD))
            .rounded(px(radius::BASE))
            .border_1()
            .border_color(theme.c("line-strong"))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover")).border_color(theme.c("line-hover")))
            .tooltip(tip(label)),
        false,
        theme,
    )
    .child(stack)
    .child(ui::text(
        members.total.to_string(),
        size::UI,
        weight::REGULAR,
        theme.c("ink-2"),
    ))
    .into_any_element();
    let id = channel.id.clone();
    Some(popover(
        eid("hb-members", &channel.id),
        Anchor::TopRight,
        button,
        move |dismiss, window, cx| members_pop(&id, dismiss, window, cx),
    ))
}

/// `.hb-members-pop`, read afresh each frame it is open: the title, who
/// works now, then everyone else by last activity.
fn members_pop(channel: &str, dismiss: Dismiss, window: &mut Window, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let Some(view) = Desk::get(cx).read(cx).view.clone() else {
        return div().into_any_element();
    };
    let Some(ch) = ws::find_channel(&view, channel) else {
        return div().into_any_element();
    };
    let members = m::channel_members(&view, &ch);
    let (working_title, earlier_title) = m::members_group_titles(&members, cx);
    let now = view.now_ms;
    let max_h = (window.viewport_size().height * 0.7).min(px(POP.1));
    let group = |title: SharedString| pop_title(title, theme).mt(px(space::XS));
    pop_surface(theme)
        .id("hb-members-pop")
        .w(px(POP.0))
        .max_h(max_h)
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .items_center()
                .h(px(row::MENU))
                .px(px(space::LG))
                .child(
                    ui::text(
                        m::members_label(&members, channel, cx),
                        size::UI,
                        weight::STRONG,
                        theme.c("ink"),
                    )
                    .truncate(),
                ),
        )
        .when(!members.working.is_empty(), |d| {
            d.child(group(working_title)).children(
                members
                    .working
                    .iter()
                    .map(|a| member_row(a, now, true, dismiss.clone(), cx)),
            )
        })
        .when(!members.rest.is_empty(), |d| {
            d.child(group(earlier_title)).children(
                members
                    .rest
                    .iter()
                    .map(|a| member_row(a, now, false, dismiss.clone(), cx)),
            )
        })
        .into_any_element()
}

/// `MemberRow`: robot, name, role · team and state; opens its conversation.
fn member_row(a: &Agent, now: u64, working: bool, dismiss: Dismiss, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let row: SharedString = "hb-member-row".into();
    let hover_ink = theme.c("menu-hover-ink");
    let muted = |text: SharedString| {
        ui::text(text, size::META, weight::REGULAR, theme.c("ink-3"))
            .truncate()
            .group_hover(row.clone(), |s| s.text_color(hover_ink))
    };
    let key = a.key.clone();
    focusable(
        div()
            .id(eid("hb-member", &a.key))
            .group(row.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::MD))
            .h(px(ui::row::AGENT))
            .mx(px(space::XS))
            .px(px(space::MD))
            .rounded(px(radius::BASE))
            .text_color(theme.c("ink"))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("menu-hover")).text_color(hover_ink)),
        true,
        theme,
    )
    .child(
        robot::robot(derive::robot_name(a).to_string(), avatar_size::SIDEBAR)
            .agent(a)
            .dot(true)
            .ring(Ring::Raised),
    )
    .child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .font_weight(weight::STRONG)
                    .child(derive::agent_name(a, cx)),
            )
            .child(muted(m::member_role(a, cx))),
    )
    .child(
        muted(derive::presence_line(a, now, cx))
            .flex_shrink_0()
            .max_w(relative(0.45))
            .when(working, |d| d.text_color(theme.c("green-ink"))),
    )
    .on_click(move |_, window, cx| {
        dismiss(window, cx);
        nav::navigate(Route::direct(Some(key.clone())), cx);
    })
    .into_any_element()
}
