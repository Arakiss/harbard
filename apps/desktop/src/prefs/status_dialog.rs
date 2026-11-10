//! "Your status" (`status-dialog.tsx`): an emoji, a few words, and when it
//! clears.

use super::profile_dialog::modal_head;
use super::shell_profile::{ProfileStore, refusal_text, status_expiry};
use super::stores::update_profile;
use super::widgets::{self, Choice, Kind, Size};
use crate::derive::profile::{CLEAR_AFTER, clear_at};
use crate::i18n::t;
use crate::model::profile::ProfileStatus;
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::rc::Rc;

const PRESETS: [(&str, &str); 4] = [
    ("💻", "profile.presetAtComputer"),
    ("📅", "profile.presetInMeeting"),
    ("🍽️", "profile.presetEating"),
    ("🚆", "profile.presetOnMyWay"),
];

pub struct StatusDialog {
    current: Option<ProfileStatus>,
    emoji: Entity<InputState>,
    text: Entity<InputState>,
    /// `nunca`, `30m`, `1h`, `4h`, `today`.
    after: &'static str,
    /// The current status's own end, kept until another choice.
    keep_end: bool,
    menu_open: bool,
    error: Option<SharedString>,
    saving: bool,
    _subs: Vec<Subscription>,
}

impl StatusDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let store = ProfileStore::get(cx).read(cx);
        let now = harbard_core::clock::now_ms();
        let current = store.status(now).cloned();
        let max = store
            .view
            .as_ref()
            .map(|v| v.limits.status_text as usize)
            .filter(|m| *m > 0)
            .unwrap_or(100);
        let emoji = cx.new(|cx| {
            InputState::new(window, cx).placeholder("💬").default_value(
                current
                    .as_ref()
                    .map(|s| s.emoji.clone())
                    .unwrap_or_default(),
            )
        });
        let text = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t(cx, "profile.whatAreYouDoing"))
                .default_value(current.as_ref().map(|s| s.text.clone()).unwrap_or_default())
        });
        let subs = vec![
            cx.subscribe_in(&emoji, window, |_, s, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = s.read(cx).value().to_string();
                    if v.chars().count() > 8 {
                        let cut: String = v.chars().take(8).collect();
                        s.update(cx, |s, cx| s.set_value(cut, window, cx));
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&text, window, move |_, s, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = s.read(cx).value().to_string();
                    if v.chars().count() > max {
                        let cut: String = v.chars().take(max).collect();
                        s.update(cx, |s, cx| s.set_value(cut, window, cx));
                    }
                    cx.notify();
                }
            }),
        ];
        let keep_end = current.as_ref().is_some_and(|s| s.clear_at_ms.is_some());
        StatusDialog {
            after: if keep_end { "today" } else { "nunca" },
            keep_end,
            current,
            emoji,
            text,
            menu_open: false,
            error: None,
            saving: false,
            _subs: subs,
        }
    }

    fn save(&mut self, clear: bool, window: &mut Window, cx: &mut Context<Self>) {
        let now = harbard_core::clock::now_ms();
        let emoji = self.emoji.read(cx).value().to_string();
        let text = self.text.read(cx).value().to_string();
        let end = match (&self.current, self.keep_end) {
            (Some(c), true) if c.clear_at_ms.is_some() => c.clear_at_ms,
            _ => clear_at(self.after, now),
        };
        let status = if clear || (emoji.trim().is_empty() && text.trim().is_empty()) {
            Value::Null
        } else {
            json!({ "emoji": emoji, "text": text, "clear_at_ms": end })
        };
        self.saving = true;
        self.error = None;
        let task = update_profile(json!({ "status": status }), cx);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => {
                        let _ = handle.update(cx, |_, window, cx| widgets::close_top(window, cx));
                    }
                    Err(e) => {
                        this.error = Some(refusal_text(&e, cx));
                        this.saving = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| StatusDialog::new(window, cx));
    let first = view.read(cx).text.clone();
    widgets::open_modal(
        view.into(),
        520.,
        None,
        widgets::Place::Center,
        None,
        window,
        cx,
    );
    widgets::autofocus(first, window, cx);
}

impl Render for StatusDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let loaded = ProfileStore::get(cx).read(cx).view.is_some();
        let now = harbard_core::clock::now_ms();
        let emoji = self.emoji.read(cx).value().to_string();
        let text = self.text.read(cx).value().to_string();
        let hover = theme.c("hover-2");

        let presets = div()
            .mt(px(18.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .mb(px(6.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.c("ink-3"))
                    .child(t(cx, "profile.suggestions")),
            )
            .children(PRESETS.iter().map(|(e, key)| {
                let label = t(cx, key);
                let pressed = emoji == *e && text == label.as_ref();
                let (e2, l2) = (e.to_string(), label.to_string());
                div()
                    .id(SharedString::from(format!("preset-{key}")))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .text_size(px(15.))
                    .cursor_pointer()
                    .when(pressed, |d| {
                        d.bg(theme.c("accent-soft"))
                            .text_color(theme.c("accent-text"))
                            .font_weight(FontWeight::BOLD)
                    })
                    .when(!pressed, |d| d.hover(move |s| s.bg(hover)))
                    .child(e.to_string())
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let (e, l) = (e2.clone(), l2.clone());
                        this.emoji.update(cx, |s, cx| s.set_value(e, window, cx));
                        this.text.update(cx, |s, cx| s.set_value(l, window, cx));
                        cx.notify();
                    }))
            }));

        let current_end = self.current.as_ref().and_then(|c| c.clear_at_ms);
        let shown: SharedString = match (self.keep_end, current_end) {
            (true, Some(end)) => status_expiry(Some(end), now, cx).unwrap_or_default(),
            _ => CLEAR_AFTER
                .iter()
                .find(|c| c.0 == self.after)
                .map(|c| t(cx, c.1))
                .unwrap_or_default(),
        };
        let this = cx.entity().downgrade();
        let mut choices: Vec<Choice> = Vec::new();
        if let (true, Some(end)) = (self.keep_end, current_end) {
            choices.push(Choice {
                label: status_expiry(Some(end), now, cx).unwrap_or_default(),
                selected: true,
                on: {
                    let this = this.clone();
                    Rc::new(move |_, cx| {
                        let _ = this.update(cx, |s, cx| {
                            s.menu_open = false;
                            cx.notify();
                        });
                    })
                },
            });
        }
        for (id, key) in CLEAR_AFTER {
            let this = this.clone();
            choices.push(Choice {
                label: t(cx, key),
                selected: !self.keep_end && self.after == id,
                on: Rc::new(move |_, cx| {
                    let _ = this.update(cx, |s, cx| {
                        s.keep_end = false;
                        s.after = id;
                        s.menu_open = false;
                        cx.notify();
                    });
                }),
            });
        }
        let toggle_this = this.clone();
        let clear_after = div()
            .mt(px(18.))
            .flex()
            .items_center()
            .gap(px(12.))
            .font_weight(FontWeight::BOLD)
            .child(t(cx, "profile.clearAfter"))
            .child(div().w(px(240.)).child(widgets::dropdown(
                "clear-after",
                shown,
                self.menu_open,
                36.,
                choices,
                false,
                &theme,
                Rc::new(move |_, cx| {
                    let _ = toggle_this.update(cx, |s, cx| {
                        s.menu_open = !s.menu_open;
                        cx.notify();
                    });
                }),
            )));

        let foot = div()
            .mt(px(20.))
            .flex()
            .flex_none()
            .gap(px(10.))
            .px(px(20.))
            .py(px(12.))
            .border_t_1()
            .border_color(theme.c("line"))
            .when(self.current.is_some(), |d| {
                d.child(widgets::button(
                    "status-remove",
                    t(cx, "profile.removeStatus"),
                    Kind::Plain,
                    Size::Normal,
                    None,
                    self.saving,
                    &theme,
                    cx.listener(|this, _, window, cx| this.save(true, window, cx)),
                ))
            })
            .child(div().flex_1())
            .child(widgets::button(
                "status-cancel",
                t(cx, "profile.cancel"),
                Kind::Plain,
                Size::Normal,
                None,
                false,
                &theme,
                |_, w, cx| widgets::close_top(w, cx),
            ))
            .child(widgets::button(
                "status-save",
                t(cx, "profile.save"),
                Kind::Primary,
                Size::Normal,
                None,
                self.saving || !loaded,
                &theme,
                cx.listener(|this, _, window, cx| this.save(false, window, cx)),
            ));

        widgets::base(&theme)
            .flex()
            .flex_col()
            .child(modal_head(t(cx, "profile.yourStatus"), &theme))
            .child(
                div()
                    .px(px(28.))
                    .pt(px(20.))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                div()
                                    .w(px(56.))
                                    .flex_none()
                                    .text_size(px(18.))
                                    .child(widgets::text_input(&self.emoji, &theme)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(widgets::text_input(&self.text, &theme)),
                            ),
                    )
                    .child(presets)
                    .child(clear_after)
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().mt(px(12.)).child(widgets::error(e, &theme)))
                    }),
            )
            .child(foot)
    }
}
