//! A channel's header (`ChannelHeader`, `ChannelWhy`, `ChannelPlaces`,
//! `WorkElsewhere` in `home.tsx`), a room's header (`RoomHeader` in
//! `rooms.tsx`) and the two-writers strips.
//!
//! The header's popovers (the sessions list, the checkouts, the members)
//! are GPUI Kit popovers: they open below their trigger, take focus, close
//! on Escape and on a click outside. Escape is handled by the popover, so
//! it never reaches the screen's own Escape (mark as read).

use crate::derive::{self, rooms, workspace as ws};
use crate::desk::Desk;
use crate::i18n::t;
use crate::model::{Channel, WorkspaceTile, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, MONO, SANS};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, BoxShadow, Div, ElementId, FontWeight, HighlightStyle,
    InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window, div, point, px,
};
use gpui_kit::component::Selectable;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::tooltip::Tooltip;
use std::rc::Rc;

/// Closes the popover the caller sits in.
pub type Dismiss = Rc<dyn Fn(&mut Window, &mut App)>;

/// Whether a header popover (or any other GPUI Kit popup) is open. A
/// screen that listens to Escape outside the normal key dispatch (for
/// instance with `intercept_keystrokes`) checks this before marking read.
pub fn popover_open(cx: &App) -> bool {
    gpui_kit::component::GlobalState::is_in_deferred_context(cx)
}

/// `prefix-key` as an element id.
pub(crate) fn eid(prefix: &str, key: &str) -> ElementId {
    ElementId::Name(format!("{prefix}-{key}").into())
}

/// A text tooltip (the console's `title`).
pub(crate) fn tip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// A plain element as a popover trigger.
#[derive(IntoElement)]
struct Trigger {
    el: AnyElement,
    open: bool,
}

impl Selectable for Trigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl RenderOnce for Trigger {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        self.el
    }
}

/// A header popover: `trigger` toggles it, `body` draws its surface each
/// frame it is open and gets a [`Dismiss`] for the links inside.
/// `anchor` is the popover's own corner (`TopLeft`: below, left-aligned).
pub(crate) fn popover(
    id: ElementId,
    anchor: Anchor,
    trigger: AnyElement,
    body: impl Fn(Dismiss, &mut Window, &mut App) -> AnyElement + 'static,
) -> AnyElement {
    Popover::new(id)
        .anchor(anchor)
        .offset(px(6.))
        .appearance(false)
        .trigger(Trigger {
            el: trigger,
            open: false,
        })
        .content(move |_state, window, cx| {
            let me = cx.entity();
            let dismiss: Dismiss = Rc::new(move |window, cx| {
                me.update(cx, |state, cx| state.dismiss(window, cx));
                window.refresh();
            });
            body(dismiss, window, cx)
        })
        .into_any_element()
}

/// The raised popover surface (`.hb-info-pop`, `.hb-members-pop`).
pub(crate) fn pop_surface(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .py(px(8.))
        .rounded(px(10.))
        .bg(theme.c("surface-raised"))
        .text_color(theme.c("ink"))
        .shadow(theme.pop_shadow())
        .font_family(SANS)
}

/// `.hb-pop-title`.
pub(crate) fn pop_title(text: SharedString, theme: &Theme) -> Div {
    div()
        .pt(px(6.))
        .px(px(16.))
        .pb(px(2.))
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("ink-3"))
        .child(text)
}

/// `.hb-head`: 56 px, one line, a hairline below.
fn head(theme: &Theme) -> Div {
    div()
        .h(px(56.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .pl(px(20.))
        .pr(px(16.))
        .border_b_1()
        .border_color(theme.c("line"))
        .font_family(SANS)
        .text_color(theme.c("ink"))
}

/// `.hb-head-title`: `#` + the id, with the ⌄ on a project channel.
fn title(id: &str, chevron: bool, theme: &Theme) -> Div {
    let ink = theme.c("ink");
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .whitespace_nowrap()
        .text_size(px(19.))
        .font_weight(FontWeight::BLACK)
        .text_color(ink)
        .child(ui::icon("hash", 20., ink))
        .child(id.to_string())
        .when(chevron, |d| d.child(ui::icon("chevron-down", 16., ink)))
}

/// `.hb-head-right`: the members (or the room's note) and the menu.
fn right(members: Option<AnyElement>, menu: Option<AnyElement>) -> Div {
    div()
        .ml_auto()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .children(members)
        .children(menu)
}

/// `.hb-topic`: one muted line that ellipsizes, with the whole text as
/// its tooltip.
fn topic(id: ElementId, text: SharedString, theme: &Theme) -> AnyElement {
    div()
        .id(id)
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(13.))
        .text_color(theme.c("ink-4"))
        .tooltip(tip(text.clone()))
        .child(text)
        .into_any_element()
}

/// `header.hb-head` for a project channel or a room: title, why, places,
/// members and `menu` (the channel's ⋯).
pub fn channel_header(
    view: &WorkspaceView,
    channel: &Channel,
    menu: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    head(theme)
        .child(title(&channel.id, true, theme))
        // The why line gives way first when a pane narrows the header.
        .child(
            div()
                .min_w_0()
                .flex_shrink(1.)
                .overflow_hidden()
                .child(why(view, channel, cx)),
        )
        .children(places(channel, cx))
        .child(right(
            super::members::members_button(view, channel, cx),
            menu,
        ))
        .into_any_element()
}

/// `RoomHeader` (`rooms.tsx`): `#` + id, the purpose as topic, the members
/// (or "No members right now") and `menu`.
pub fn room_header(
    view: &WorkspaceView,
    channel: &Channel,
    menu: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    let purpose = rooms::room_purpose(channel, cx);
    let members = if channel.members.is_empty() && !channel.built_in && !channel.archived {
        Some(topic(
            eid("hb-room-nobody", &channel.id),
            t(cx, "rooms.noMembers"),
            theme,
        ))
    } else {
        super::members::members_button(view, channel, cx)
    };
    head(theme)
        .child(title(&channel.id, false, theme))
        .children(purpose.map(|p| topic(eid("hb-room-topic", &channel.id), p, theme)))
        .child(right(members, menu))
        .into_any_element()
}

/// `ChannelWhy`: "4 sessions today · 6 working now", the sessions opening
/// the list of those agents, each a link to its conversation. Nobody
/// working here and somebody elsewhere adds "· 3 working in Acme".
fn why(view: &WorkspaceView, channel: &Channel, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let h = ws::channel_headline(view, channel, cx);
    // From the count, not the wording: the sentence changes with the language.
    let nobody = h.now.is_some() && h.working == 0;
    let now = h.now.clone().map(|now| {
        div()
            .flex_none()
            .text_color(if nobody {
                theme.c("ink-3")
            } else {
                theme.c("green-ink")
            })
            .font_weight(if nobody {
                FontWeight::NORMAL
            } else {
                FontWeight::BOLD
            })
            .child(SharedString::from(format!("\u{a0}·\u{a0}{now}")))
    });
    let elsewhere = if nobody {
        work_elsewhere(&view.workspaces, "\u{a0}·\u{a0}", cx)
    } else {
        None
    };
    let line = div()
        .flex_none()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .text_size(px(13.))
        .text_color(theme.c("ink-3"));
    if h.agents.is_empty() {
        return line
            .child(h.sessions)
            .children(now)
            .children(elsewhere)
            .into_any_element();
    }
    let agents: Vec<(String, SharedString)> = h
        .agents
        .iter()
        .map(|a| (a.key.clone(), derive::agent_name(a, cx)))
        .collect();
    let trigger = div()
        .id(eid("hb-why-summary", &channel.id))
        .flex()
        .items_center()
        .cursor_pointer()
        .child(div().text_color(theme.c("link")).child(h.sessions))
        .children(now)
        .into_any_element();
    let list = popover(
        eid("hb-why", &channel.id),
        Anchor::TopLeft,
        trigger,
        move |dismiss, _window, cx| why_list(&agents, dismiss, cx),
    );
    line.child(list).children(elsewhere).into_any_element()
}

/// `.hb-why-list`: the agents behind the sessions count.
fn why_list(agents: &[(String, SharedString)], dismiss: Dismiss, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let shade = {
        let mut c = theme.c("ink");
        c.a = 0.18;
        c
    };
    div()
        .flex()
        .flex_col()
        .min_w(px(200.))
        .py(px(6.))
        .bg(theme.c("surface"))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(8.))
        .shadow(vec![BoxShadow {
            color: shade,
            offset: point(px(0.), px(6.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_family(SANS)
        .text_size(px(13.))
        .whitespace_nowrap()
        .children(agents.iter().map(|(key, name)| {
            let key = key.clone();
            let dismiss = dismiss.clone();
            div()
                .id(eid("hb-why-agent", &key))
                .px(px(12.))
                .py(px(4.))
                .text_color(theme.c("ink"))
                .cursor_pointer()
                .hover(|s| s.bg(theme.c("tint")))
                .child(name.clone())
                .on_click(move |_, window, cx| {
                    dismiss(window, cx);
                    nav::navigate(Route::direct(Some(key.clone())), cx);
                })
        }))
        .into_any_element()
}

/// `ChannelPlaces`: the checkouts and branches behind an "ℹ", never in
/// the header line. None when the channel has no checkout.
fn places(channel: &Channel, cx: &App) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let places = ws::channel_places(channel, cx);
    if places.copies.is_empty() {
        return None;
    }
    let group: SharedString = "hb-info".into();
    let trigger = div()
        .id(eid("hb-info-summary", &channel.id))
        .group(group.clone())
        .flex_none()
        .flex()
        .items_center()
        .p(px(4.))
        .rounded(px(4.))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("hover")))
        .tooltip(tip(t(cx, "home.copiesAndBranches")))
        .child(
            ui::icon("info", 16., theme.c("ink-4"))
                .group_hover(group, |s| s.text_color(theme.c("ink-2"))),
        )
        .into_any_element();
    let places = Rc::new(places);
    Some(popover(
        eid("hb-info", &channel.id),
        Anchor::TopLeft,
        trigger,
        move |_dismiss, window, cx| places_pop(&places, window, cx),
    ))
}

/// `.hb-info-pop`: the repository, then one line per checkout and branch.
fn places_pop(places: &ws::Places, window: &mut Window, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let viewport = window.viewport_size();
    pop_surface(theme)
        .id("hb-info-pop")
        .w((viewport.width * 0.7).min(px(560.)))
        .max_h((viewport.height * 0.6).min(px(420.)))
        .overflow_y_scroll()
        .text_size(px(12.))
        .child(pop_title(places.title.clone(), theme))
        .children(places.copies.iter().enumerate().map(|(i, c)| {
            div()
                .flex()
                .justify_between()
                .gap(px(12.))
                .px(px(16.))
                .py(px(3.))
                .font_family(MONO)
                .text_size(px(13.))
                .child(
                    div()
                        .id(("hb-info-path", i))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .tooltip(tip(c.full.clone().into()))
                        .child(c.path.clone()),
                )
                .children(
                    c.branch
                        .clone()
                        .map(|b| div().flex_none().text_color(theme.c("ink-3")).child(b)),
                )
        }))
        .into_any_element()
}

/// "{count} working in {name}": switches to the busy workspace and goes Home.
/// `prefix` is drawn before it (" · " or " ") in the caller's colour.
pub fn work_elsewhere(tiles: &[WorkspaceTile], prefix: &str, cx: &App) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let busy = ws::working_elsewhere(tiles)?;
    let label = ws::working_in_text(busy, cx);
    let id = busy.id.clone();
    Some(
        div()
            .flex_none()
            .flex()
            .whitespace_nowrap()
            .when(!prefix.is_empty(), |d| {
                d.child(SharedString::from(prefix.to_string()))
            })
            .child(
                div()
                    .id(eid("hb-why-elsewhere", &id))
                    .text_color(theme.c("green-ink"))
                    .font_weight(FontWeight::BOLD)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child(label)
                    .on_click(move |_, _window, cx| {
                        cx.stop_propagation();
                        let id = id.clone();
                        Desk::get(cx).update(cx, |d, cx| d.switch_workspace(Some(id), cx));
                        nav::navigate(Route::home(), cx);
                    }),
            )
            .into_any_element(),
    )
}

/// One `.hb-strip[role=alert]` per writer warning of the channel:
/// "**Two agents in the same checkout:** a, b writing in x since 22:41."
pub fn warning_strips(view: &WorkspaceView, channel: &Channel, cx: &App) -> Vec<AnyElement> {
    let theme = cx.global::<Theme>();
    channel
        .warnings
        .iter()
        .map(|w| {
            let lead = ws::warning_lead(w, cx);
            let body = ws::warning_text(view, w, cx);
            let text = format!("{lead} {body}");
            let bold = HighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                ..Default::default()
            };
            div()
                .flex_none()
                .flex()
                .items_start()
                .gap(px(8.))
                .px(px(20.))
                .py(px(8.))
                .bg(theme.c("warn-bg"))
                .text_color(theme.c("warn-ink"))
                .border_b_1()
                .border_color(theme.c("warn-line"))
                .font_family(SANS)
                .text_size(px(14.))
                .child(div().flex_none().pt(px(2.)).child(ui::icon(
                    "triangle-alert",
                    16.,
                    theme.c("warn-ink"),
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(StyledText::new(text).with_highlights([(0..lead.len(), bold)])),
                )
                .into_any_element()
        })
        .collect()
}
