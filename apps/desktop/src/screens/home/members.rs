//! Who has worked in a channel (`ChannelMembers`): the avatar stack and its
//! popover of members working now and earlier.

use super::header::{Dismiss, eid, pop_surface, pop_title, popover, tip};
use crate::derive::{self, members as m, workspace as ws};
use crate::desk::Desk;
use crate::model::{Agent, Channel, WorkspaceView};
use crate::nav::{self, Route};
use crate::robot::{self, Ring};
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};

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
            div().when(i > 0, |d| d.ml(px(-4.))).child(
                robot::robot(derive::robot_name(a).to_string(), 24.)
                    .agent(a)
                    .dot(false)
                    .tile_ring(surface),
            )
        }));
    // Who works now is in the header line; the button keeps the total.
    let button = div()
        .id(eid("hb-members-button", &channel.id))
        .flex()
        .items_center()
        .gap(px(6.))
        .whitespace_nowrap()
        .h(px(32.))
        .pl(px(4.))
        .pr(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("hover")))
        .tooltip(tip(label))
        .child(stack)
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.c("ink-3"))
                .child(SharedString::from(members.total.to_string())),
        )
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
    let max_h = (window.viewport_size().height * 0.7).min(px(520.));
    let group = |title: SharedString| {
        div()
            .pt(px(10.))
            .px(px(16.))
            .pb(px(4.))
            .text_size(px(12.))
            .font_weight(FontWeight::BOLD)
            .text_color(theme.c("ink-3"))
            .child(title)
    };
    pop_surface(theme)
        .id("hb-members-pop")
        .w(px(360.))
        .max_h(max_h)
        .overflow_y_scroll()
        .child(pop_title(m::members_label(&members, channel, cx), theme))
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
        div()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(12.))
            .text_color(theme.c("ink-3"))
            .group_hover(row.clone(), |s| s.text_color(hover_ink))
            .child(text)
    };
    let key = a.key.clone();
    div()
        .id(eid("hb-member", &a.key))
        .group(row.clone())
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(16.))
        .py(px(6.))
        .text_color(theme.c("ink"))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("menu-hover")).text_color(hover_ink))
        .child(
            robot::robot(derive::robot_name(a).to_string(), 28.)
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
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(14.))
                        .font_weight(FontWeight::BOLD)
                        .child(derive::agent_name(a, cx)),
                )
                .child(muted(m::member_role(a, cx))),
        )
        .child(
            muted(derive::presence_line(a, now, cx))
                .flex_shrink_0()
                .max_w(relative(0.45))
                .when(working, |d| {
                    d.text_color(theme.c("green-ink"))
                        .font_weight(FontWeight::BOLD)
                }),
        )
        .on_click(move |_, window, cx| {
            dismiss(window, cx);
            nav::navigate(Route::direct(Some(key.clone())), cx);
        })
        .into_any_element()
}
