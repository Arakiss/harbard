//! A channel's header (`ChannelHeader`, `ChannelWhy`, `ChannelPlaces`,
//! `WorkElsewhere` in `home.tsx`), a room's header (`RoomHeader` in
//! `rooms.tsx`) and the two-writers strips.
//!
//! The header's popovers (the sessions list, the checkouts, the members)
//! are GPUI Kit popovers: they open below their trigger, take focus, close
//! on Escape and on a click outside. Escape is handled by the popover, so
//! it never reaches the screen's own Escape (mark as read).

use crate::chat::focusable;
use crate::derive::{self, rooms, workspace as ws};
use crate::desk::Desk;
use crate::i18n::t;
use crate::model::{Channel, WorkspaceTile, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, MONO, SANS, control, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, Div, ElementId, HighlightStyle, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, StyledText,
    Window, div, px, relative,
};
use gpui_kit::component::Selectable;
use gpui_kit::component::popover::Popover;
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

pub(crate) use crate::chat::tip;

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
        // A trigger that holds text may shrink and ellipsize (T7).
        .min_w_0()
        .anchor(anchor)
        .offset(px(space::SM))
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

/// The raised popover surface: `surface-raised` under the pop shadow, no
/// border (C6).
pub(crate) fn pop_surface(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .py(px(space::MD))
        .rounded(px(radius::MD))
        .bg(theme.c("surface-raised"))
        .text_color(theme.c("ink"))
        .shadow(theme.pop_shadow())
        .font_family(SANS)
}

/// A popover's label: a section label at the popover's inset.
pub(crate) fn pop_title(text: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(row::MENU))
        .px(px(space::LG))
        .child(ui::section_label(text, theme).truncate())
}

/// The header: 56 px, one line, a hairline below; its title starts at the
/// gutter.
fn head(theme: &Theme) -> Div {
    div()
        .h(px(row::HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::GUTTER))
        .border_b_1()
        .border_color(theme.c("line"))
        .font_family(SANS)
        .text_color(theme.c("ink"))
}

/// The pane's title (T4): `#` + the id at 18/900, with the ⌄ on a project
/// channel. The marks recede; the name is what is read.
fn title(id: &str, chevron: bool, theme: &Theme) -> Div {
    let mark = theme.c("ink-3");
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .max_w(relative(0.5))
        .child(ui::icon("hash", icon_size::ROW, mark))
        .child(ui::text(id.to_string(), size::TITLE, weight::HEAVY, theme.c("ink")).truncate())
        .when(chevron, |d| {
            d.child(ui::icon("chevron-down", icon_size::ROW, mark))
        })
}

/// The header's right end: the members (or the room's note) and the menu.
fn right(members: Option<AnyElement>, menu: Option<AnyElement>) -> Div {
    div()
        .ml_auto()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .children(members)
        .children(menu)
}

/// One muted line that ellipsizes, with the whole text as its tooltip.
fn topic(id: ElementId, text: SharedString, theme: &Theme) -> AnyElement {
    div()
        .id(id)
        .min_w_0()
        .tooltip(tip(text.clone()))
        .child(ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-3")).truncate())
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
/// working here and somebody elsewhere adds "· 3 working in Acme". The
/// count is metadata (`ink-3`); only "working now" is a state (C3).
fn why(view: &WorkspaceView, channel: &Channel, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let h = ws::channel_headline(view, channel, cx);
    // From the count, not the wording: the sentence changes with the language.
    let nobody = h.now.is_some() && h.working == 0;
    // One text, so a narrow header ends it with an ellipsis and never
    // mid-letter (T7); "working now" keeps its state colour inside it.
    let summary = {
        let mut text = h.sessions.to_string();
        let mut marks = Vec::new();
        if let Some(now) = &h.now {
            text.push_str("\u{a0}·\u{a0}");
            let from = text.len();
            text.push_str(now);
            if !nobody {
                marks.push((
                    from..text.len(),
                    HighlightStyle {
                        color: Some(theme.c("green-ink")),
                        ..Default::default()
                    },
                ));
            }
        }
        div()
            .min_w_0()
            .truncate()
            .child(StyledText::new(text).with_highlights(marks))
    };
    let elsewhere = if nobody {
        work_elsewhere(&view.workspaces, "\u{a0}·\u{a0}", cx)
    } else {
        None
    };
    let line = div()
        .min_w_0()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .text_size(px(size::SMALL))
        .line_height(px(leading::SMALL))
        .font_weight(weight::REGULAR)
        .text_color(theme.c("ink-3"));
    if h.agents.is_empty() {
        return line.child(summary).children(elsewhere).into_any_element();
    }
    let agents: Vec<(String, SharedString)> = h
        .agents
        .iter()
        .map(|a| (a.key.clone(), derive::agent_name(a, cx)))
        .collect();
    let trigger = focusable(
        div()
            .id(eid("hb-why-summary", &channel.id))
            .flex()
            .items_center()
            .min_w_0()
            .h(px(control::SM))
            .mx(px(-space::SM))
            .px(px(space::SM))
            .rounded(px(radius::BASE))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover")))
            .child(summary),
        false,
        theme,
    )
    .into_any_element();
    let list = popover(
        eid("hb-why", &channel.id),
        Anchor::TopLeft,
        trigger,
        move |dismiss, _window, cx| why_list(&agents, dismiss, cx),
    );
    line.child(list)
        .children(elsewhere.map(|e| div().pl(px(space::SM)).child(e)))
        .into_any_element()
}

/// A popover row: 28 px, the menu's hover (C8).
fn pop_row(id: ElementId, theme: &Theme) -> gpui::Stateful<Div> {
    focusable(
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::MD))
            .h(px(row::MENU))
            .mx(px(space::XS))
            .px(px(space::MD))
            .rounded(px(radius::BASE))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .text_color(theme.c("ink"))
            .cursor_pointer()
            .hover(|s| {
                s.bg(theme.c("menu-hover"))
                    .text_color(theme.c("menu-hover-ink"))
            }),
        true,
        theme,
    )
}

/// The agents behind the sessions count.
fn why_list(agents: &[(String, SharedString)], dismiss: Dismiss, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    pop_surface(theme)
        .min_w(px(WHY_MIN))
        .py(px(space::XS))
        .gap(px(space::HAIR))
        .children(agents.iter().map(|(key, name)| {
            let key = key.clone();
            let dismiss = dismiss.clone();
            pop_row(eid("hb-why-agent", &key), theme)
                .child(div().min_w_0().truncate().child(name.clone()))
                .on_click(move |_, window, cx| {
                    dismiss(window, cx);
                    nav::navigate(Route::direct(Some(key.clone())), cx);
                })
        }))
        .into_any_element()
}

/// The sessions list is at least this wide.
const WHY_MIN: f32 = 200.;
/// The checkouts popover's bounds.
const PLACES_MAX: (f32, f32) = (560., 420.);

/// A header's icon button (K3): 28 px around a 16 px icon, with a tooltip.
pub(crate) fn icon_button(
    id: ElementId,
    icon: &str,
    label: SharedString,
    theme: &Theme,
) -> gpui::Stateful<Div> {
    focusable(
        div()
            .id(id)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(control::SM))
            .rounded(px(radius::BASE))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover")))
            .tooltip(tip(label))
            .child(ui::icon(icon, icon_size::ROW, theme.c("ink-3"))),
        false,
        theme,
    )
}

/// `ChannelPlaces`: the checkouts and branches behind an "ℹ", never in
/// the header line. None when the channel has no checkout.
fn places(channel: &Channel, cx: &App) -> Option<AnyElement> {
    let theme = cx.global::<Theme>();
    let places = ws::channel_places(channel, cx);
    if places.copies.is_empty() {
        return None;
    }
    let trigger = icon_button(
        eid("hb-info-summary", &channel.id),
        "info",
        t(cx, "home.copiesAndBranches"),
        theme,
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

/// The repository, then one line per checkout and branch (things a person
/// may copy, so in mono, T6).
fn places_pop(places: &ws::Places, window: &mut Window, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let viewport = window.viewport_size();
    pop_surface(theme)
        .id("hb-info-pop")
        .w((viewport.width * 0.7).min(px(PLACES_MAX.0)))
        .max_h((viewport.height * 0.6).min(px(PLACES_MAX.1)))
        .overflow_y_scroll()
        .child(pop_title(places.title.clone(), theme))
        .children(places.copies.iter().enumerate().map(|(i, c)| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap(px(space::LG))
                .h(px(row::MENU))
                .px(px(space::LG))
                .font_family(MONO)
                .text_size(px(size::META))
                .line_height(px(leading::META))
                .child(
                    div()
                        .id(("hb-info-path", i))
                        .min_w_0()
                        .truncate()
                        .tooltip(crate::ui::kit_theme::tip_inside(c.full.clone(), &[]))
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
                focusable(
                    div()
                        .id(eid("hb-why-elsewhere", &id))
                        .rounded(px(radius::SM))
                        .text_color(theme.c("green-ink"))
                        .cursor_pointer()
                        .hover(|s| s.underline()),
                    false,
                    theme,
                )
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

/// How many writer alerts show as rows; the rest are a count on the last.
const ALERTS_SHOWN: usize = 2;

/// The writer warnings of the channel, one quiet 32 px row each at the
/// gutter: "**Two agents in the same checkout:** a, b writing in x since
/// 22:41." The lead-in is 700, the rest 400, on one line that ellipsizes
/// (the whole sentence is its tooltip). At most two rows; further alerts
/// are a count at the end of the second, listed in its tooltip.
pub fn warning_strips(view: &WorkspaceView, channel: &Channel, cx: &App) -> Vec<AnyElement> {
    let theme = cx.global::<Theme>();
    let alerts: Vec<(SharedString, SharedString)> = channel
        .warnings
        .iter()
        .map(|w| (ws::warning_lead(w, cx), ws::warning_text(view, w, cx)))
        .collect();
    if alerts.is_empty() {
        return Vec::new();
    }
    let sentence = |(lead, body): &(SharedString, SharedString)| format!("{lead} {body}");
    let rest: Vec<String> = alerts.iter().skip(ALERTS_SHOWN).map(sentence).collect();
    let shown = alerts.len().min(ALERTS_SHOWN);
    let strong = HighlightStyle {
        font_weight: Some(weight::STRONG),
        ..Default::default()
    };
    let rows = alerts.iter().take(ALERTS_SHOWN).enumerate().map(|(i, a)| {
        let text = sentence(a);
        let more = (i + 1 == shown && !rest.is_empty()).then(|| {
            div()
                .id(("hb-strip-more", i))
                .flex_none()
                .font_weight(weight::STRONG)
                .tooltip(tip(rest.join("\n").into()))
                .child(SharedString::from(format!("+{}", rest.len())))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::MD))
            .h(px(control::MD))
            .px(px(space::LG))
            .rounded(px(radius::MD))
            .bg(theme.c("warn-bg"))
            .text_color(theme.c("warn-ink"))
            .font_family(SANS)
            .text_size(px(size::SMALL))
            .line_height(px(leading::SMALL))
            .font_weight(weight::REGULAR)
            .child(ui::icon(
                "triangle-alert",
                icon_size::INLINE,
                theme.c("warn-icon"),
            ))
            .child(
                div()
                    .id(("hb-strip", i))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .tooltip(tip(text.clone().into()))
                    .child(StyledText::new(text).with_highlights([(0..a.0.len(), strong)])),
            )
            .children(more)
    });
    vec![
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(space::XS))
            .px(px(space::GUTTER))
            .py(px(space::MD))
            // The messages scroll under this edge, not into the alert.
            .border_b_1()
            .border_color(theme.c("line"))
            .children(rows)
            .into_any_element(),
    ]
}
