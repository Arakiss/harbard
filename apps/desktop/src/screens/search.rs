//! Search (`/search?q=`; `later.tsx` `SearchScreen`): one field over the
//! agents, reports and messages the day's view holds. The address follows
//! what is typed, 250 ms after the last key.

use crate::chat::{self, MessageOpts};
use crate::derive::agent_name;
use crate::derive::search::search;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::nav::{self, Nav, Route};
use crate::screens::inbox as parts;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(250);

pub struct SearchScreen {
    input: Entity<InputState>,
    debounce: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl SearchScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let q = match Nav::route(cx) {
            Route::Search { q } => q.clone(),
            _ => String::new(),
        };
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(t(cx, "later.searchPlaceholder"));
            s.set_value(q.clone(), window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let subs = vec![
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.subscribe(&input, |this, input, ev: &InputEvent, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                cx.notify();
                let q = input.read(cx).value().to_string();
                this.debounce = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    cx.update(|cx| {
                        if matches!(Nav::route(cx), Route::Search { .. }) {
                            nav::replace(Route::Search { q }, cx);
                        }
                    });
                }));
            }),
        ];
        SearchScreen {
            input,
            debounce: None,
            _subs: subs,
        }
    }
}

impl Render for SearchScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let q = self.input.read(cx).value().to_string();
        let desk = Desk::get(cx).read(cx);
        let Some(view) = desk.view.clone() else {
            return div().size_full().bg(theme.c("surface")).child(parts::loading(cx)).into_any_element();
        };
        let r = search(&view, &q);
        let h2 = |s: SharedString| {
            div()
                .mt(px(18.))
                .mx(px(20.))
                .mb(px(6.))
                .text_size(px(14.))
                .font_weight(FontWeight::BLACK)
                .text_color(theme.c("ink-3"))
                .child(s.to_uppercase())
        };
        let opts = MessageOpts {
            in_thread: true,
            fold: true,
            continued: false,
        };
        let mut body = div()
            .id("hb-results")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(8.))
            .pb(px(12.));
        if q.trim().is_empty() {
            body = body.child(parts::empty(None, Some(t(cx, "later.searchHint")), &theme));
        } else if r.total() == 0 {
            body = body.child(parts::empty(None, Some(tv(cx, "later.noMatches", &[("query", q.trim())])), &theme));
        }
        if !r.agents.is_empty() {
            body = body.child(h2(t(cx, "later.agents"))).children(r.agents.iter().map(|a| {
                let key = a.key.clone();
                div()
                    .id(SharedString::from(format!("hb-found-{}", a.key)))
                    .flex()
                    .gap(px(10.))
                    .px(px(20.))
                    .py(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("hover")))
                    .child(ui::avatar::agent_avatar(a, 36., true, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::BLACK).child(agent_name(a, cx)))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(theme.c("ink-3"))
                                    .child(a.description.clone().or_else(|| a.brief.clone()).unwrap_or_default()),
                            ),
                    )
                    .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
            }));
        }
        if !r.reports.is_empty() {
            body = body.child(h2(t(cx, "later.reports"))).children(r.reports.iter().enumerate().map(|(i, (path, _))| {
                div()
                    .px(px(20.))
                    .py(px(4.))
                    .child(parts::file_card(SharedString::from(format!("hb-found-report-{i}")), path, false, cx))
            }));
        }
        if !r.messages.is_empty() {
            body = body
                .child(h2(t(cx, "later.messages")))
                .children(r.messages.iter().take(100).map(|m| chat::message_item(m, &view, &opts, cx)));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.c("surface"))
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(20.))
                    .pr(px(16.))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(div().flex_1().child(parts::search_box(&self.input, &theme))),
            )
            .child(body)
            .into_any_element()
    }
}
