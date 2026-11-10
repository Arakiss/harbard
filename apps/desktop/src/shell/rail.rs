//! The rail (`shell.tsx` `Rail`): brand or workspace tiles, the four
//! destinations plus Search, then Create and the operator's avatar at the foot.

use crate::desk::Desk;
use crate::i18n::t;
use crate::nav::{self, Destination, Nav, Route};
use crate::theme::Theme;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

pub struct Rail;

impl Rail {
    pub fn new(_window: &mut Window, cx: &mut gpui::Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        Rail
    }

    pub fn open_ws_menu(&mut self, at: Option<gpui::Point<gpui::Pixels>>, _window: &mut Window, _cx: &mut gpui::Context<Self>) {
        let _ = at;
    }
}

struct Item {
    dest: Destination,
    label_key: &'static str,
    icon: &'static str,
    route: Route,
    badge: u64,
}

impl gpui::Render for Rail {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let (directs, activity) = match (&desk.activity, &desk.view) {
            (Some(a), _) => (a.unread.directs, a.unread.activity),
            (None, Some(v)) => (v.unread.directs, v.unread.activity),
            _ => (0, 0),
        };
        let current = Nav::route(cx).destination();
        let items = [
            Item {
                dest: Destination::Home,
                label_key: "workspace.railHome",
                icon: "house",
                route: Route::home(),
                badge: 0,
            },
            Item {
                dest: Destination::Direct,
                label_key: "workspace.railDirect",
                icon: "messages-square",
                route: Route::direct(None),
                badge: directs,
            },
            Item {
                dest: Destination::Activity,
                label_key: "workspace.railActivity",
                icon: "bell",
                route: Route::activity(),
                badge: activity,
            },
            Item {
                dest: Destination::Later,
                label_key: "workspace.railLater",
                icon: "bookmark",
                route: Route::Later,
                badge: 0,
            },
            Item {
                dest: Destination::Search,
                label_key: "shell.search",
                icon: "search",
                route: Route::Search { q: String::new() },
                badge: 0,
            },
        ];
        div()
            .w(px(78.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .pt(px(4.))
            .pb(px(14.))
            .child(brand(&theme))
            .children(items.into_iter().map(|item| destination(item, current, &theme, cx)))
            .child(div().flex_1())
            .child(
                div()
                    .id("rail-create")
                    .size(px(38.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.c("rail-tile"))
                    .hover(|s| s.bg(theme.c("rail-tile-hover")))
                    .cursor_pointer()
                    .child(icon("plus", 20., theme.c("rail-ink"))),
            )
            .child(
                div()
                    .size(px(42.))
                    .rounded(px(9.))
                    .bg(theme.c("av-operator"))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.c("av-ink"))
                    .font_weight(FontWeight::BOLD)
                    .child(desk.view.as_ref().map(|v| v.operator_initials.clone()).unwrap_or_else(|| "OP".into())),
            )
    }
}

fn brand(theme: &Theme) -> impl IntoElement {
    div()
        .size(px(48.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .size(px(36.))
                .rounded(px(9.))
                .bg(theme.c("ws-tile"))
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.c("rail-ink"))
                .font_weight(FontWeight::BLACK)
                .child("hb"),
        )
}

fn destination(item: Item, current: Option<Destination>, theme: &Theme, cx: &App) -> impl IntoElement {
    let active = current == Some(item.dest);
    let label: SharedString = t(cx, item.label_key);
    let route = item.route.clone();
    let ink = theme.c("rail-ink");
    div()
        .id(SharedString::from(format!("rail-{}", item.label_key)))
        .w(px(72.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(3.))
        .cursor_pointer()
        .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
        .child(
            div()
                .relative()
                .size(px(42.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .when(active, |d| d.bg(theme.c("rail-active")).border_1().border_color(theme.c("rail-ring")))
                .when(!active, |d| d.hover(|s| s.bg(theme.c("rail-hover"))))
                .child(icon(item.icon, 24., ink))
                .when(item.badge > 0, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(-6.))
                            .right(px(-9.))
                            .child(ui::count_badge(item.badge, theme.c("badge"), theme.c("accent-deep"))),
                    )
                }),
        )
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(if active { ink } else { theme.c("rail-text") })
                .child(label),
        )
}
