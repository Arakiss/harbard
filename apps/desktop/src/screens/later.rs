//! Later (`/later`; `later.tsx` `LaterScreen`): what the operator saved to
//! read later, with the way back to it while it is still in the day's view.

use crate::derive::cards::file_name;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Saved, WorkspaceView};
use crate::nav::{self, Route};
use crate::screens::inbox as parts;
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::format::{at_date, hhmm, monogram};
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use serde_json::json;

pub struct LaterScreen {
    _subs: Vec<Subscription>,
}

impl LaterScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        LaterScreen {
            _subs: vec![
                cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
                cx.observe_global::<Theme>(|_, cx| cx.notify()),
            ],
        }
    }
}

/// `.hb-head`: a 56px header with an icon, a title and a grey topic.
pub fn screen_head(
    icon_name: &str,
    title: SharedString,
    topic: Option<SharedString>,
    theme: &Theme,
) -> Div {
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
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(19.))
                .font_weight(FontWeight::BLACK)
                .child(icon(icon_name, 20., theme.c("ink")))
                .child(title),
        )
        .children(topic.map(|t| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(13.))
                .text_color(theme.c("ink-4"))
                .child(t)
        }))
}

/// A monogram square (`Avatar`), for what has no agent.
pub fn monogram_avatar(name: &str, size: f32, theme: &Theme) -> AnyElement {
    div()
        .size(px(size))
        .flex_none()
        .rounded(px((size * 0.22).round().max(6.)))
        .bg(theme.tone_for(name))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.c("av-ink"))
        .font_weight(FontWeight::BOLD)
        .text_size(px((size * 0.4).round()))
        .child(monogram(name))
        .into_any_element()
}

fn saved_item(view: &WorkspaceView, s: &Saved, now: u64, theme: &Theme, cx: &App) -> AnyElement {
    let live = view.message(&s.id);
    let target: Option<Route> = match live {
        Some(m) if !m.channels.is_empty() => Some(Route::Home {
            channel: Some(m.channels[0].clone()),
            thread: Some(m.reply_to.clone().unwrap_or_else(|| m.id.clone())),
            item: None,
        }),
        Some(m) => Some(Route::direct(Some(m.agent.clone()))),
        None => None,
    };
    let live_agent = live.and_then(|m| view.agent(&m.agent));
    let avatar = match (&s.kind[..], &s.agent, live_agent) {
        ("message", Some(_), Some(a)) => ui::avatar::agent_avatar(a, 36., true, cx),
        ("message", Some(name), None) => monogram_avatar(name, 36., theme),
        _ => {
            let name = s
                .agent
                .clone()
                .or_else(|| s.path.as_deref().map(|p| file_name(p).to_string()))
                .unwrap_or_else(|| "harbard".into());
            monogram_avatar(&name, 36., theme)
        }
    };
    let name: SharedString = if s.kind == "report" {
        t(cx, "later.report")
    } else {
        s.agent
            .clone()
            .map(Into::into)
            .unwrap_or_else(|| t(cx, "later.message"))
    };
    let time = |text: String| {
        div()
            .text_size(px(12.))
            .text_color(theme.c("ink-3"))
            .child(text)
    };
    let id = s.id.clone();
    div()
        .flex()
        .gap(px(10.))
        .px(px(20.))
        .py(px(8.))
        .hover(|st| st.bg(theme.c("hover")))
        .child(avatar)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight::BLACK).child(name))
                        .children(s.channel.as_ref().map(|c| time(format!("#{c}"))))
                        .children(s.ts_ms.map(|ms| time(hhmm(ms))))
                        .child(time(
                            tv(
                                cx,
                                "later.savedWhen",
                                &[("when", &at_date(s.saved_ms, now, cx))],
                            )
                            .to_string(),
                        )),
                )
                .child(div().child(s.text.clone()))
                .children(s.path.as_ref().map(|p| crate::chat::file_card(p, cx)))
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .gap(px(16.))
                        .text_size(px(13.))
                        .children(match target {
                            Some(route) => Some(
                                div()
                                    .id(SharedString::from(format!("hb-later-go-{}", s.id)))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.c("link"))
                                    .cursor_pointer()
                                    .hover(|st| st.underline())
                                    .child(t(cx, "later.viewInContext"))
                                    .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
                                    .into_any_element(),
                            ),
                            None if s.kind == "message" => Some(
                                div()
                                    .text_color(theme.c("ink-3"))
                                    .child(t(cx, "later.outOfWindow"))
                                    .into_any_element(),
                            ),
                            None => None,
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("hb-later-remove-{}", s.id)))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.c("ink-3"))
                                .cursor_pointer()
                                .hover(|st| st.underline())
                                .child(t(cx, "later.remove"))
                                .on_click(move |_, _, cx| {
                                    post("unsave", json!({ "id": id }), cx).detach();
                                }),
                        ),
                ),
        )
        .into_any_element()
}

impl Render for LaterScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div()
                .size_full()
                .bg(theme.c("surface"))
                .child(parts::loading(cx))
                .into_any_element();
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.c("surface"))
            .child(screen_head(
                "bookmark",
                t(cx, "later.title"),
                Some(t(cx, "later.topic")),
                &theme,
            ))
            .child(
                div()
                    .id("hb-later")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(8.))
                    .pb(px(12.))
                    .when(view.saved.is_empty(), |d| {
                        d.child(parts::empty(
                            Some(t(cx, "later.emptyTitle")),
                            Some(t(cx, "later.emptyHint")),
                            &theme,
                        ))
                    })
                    .children(
                        view.saved
                            .iter()
                            .map(|s| saved_item(&view, s, now, &theme, cx)),
                    ),
            )
            .into_any_element()
    }
}
