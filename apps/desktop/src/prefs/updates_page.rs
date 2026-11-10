//! Preferences › Updates (`update-section.tsx`): when it last looked, which
//! changes count as a new version, the background check and the checkout it
//! updates from.

use super::shell_update::UpdateStore;
use super::stores::{check_updates, set_update_prefs};
use super::widgets::{self, Kind, Mark, Size};
use crate::derive::update::{source_problem_text, status_sentence};
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use serde_json::json;
use std::time::Duration;

pub struct UpdatesPage {
    _clock: Task<()>,
}

impl UpdatesPage {
    pub fn new(_window: &mut gpui::Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        // "Last checked: 5 min ago" moves on its own.
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        UpdatesPage { _clock: clock }
    }
}

/// `.hb-update-warn`.
pub fn warn(text: impl Into<SharedString>, red: bool, theme: &crate::theme::Theme) -> gpui::Div {
    let (line, bg, ink) = if red {
        (theme.c("red-line"), theme.c("red-bg"), theme.c("red-ink"))
    } else {
        (
            theme.c("warn-line"),
            theme.c("warn-bg"),
            theme.c("warn-ink"),
        )
    };
    div()
        .px(px(space::LG))
        .py(px(space::MD))
        .rounded(px(radius::MD))
        .border_1()
        .border_color(line)
        .bg(bg)
        .text_color(ink)
        .text_size(px(size::SMALL))
        .line_height(px(leading::SMALL))
        .child(text.into())
}

impl Render for UpdatesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let store = UpdateStore::get(cx).read(cx);
        if store.unsupported {
            return div().into_any_element();
        }
        let view = store.view.clone();
        let checking = store.checking;
        let available = store.available();
        let error = store.error.clone();
        let disabled = view.is_none() || checking;
        let now = harbard_core::clock::now_ms();

        let status = div()
            .child(widgets::legend(t(cx, "update.statusLegend"), &theme))
            .child(widgets::hint(
                status_sentence(view.as_ref(), now, cx),
                &theme,
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::MD))
                    .mt(px(space::MD))
                    .child(widgets::button(
                        "update-check-now",
                        if checking {
                            t(cx, "update.checking")
                        } else {
                            t(cx, "update.checkNow")
                        },
                        Kind::Plain,
                        Size::Normal,
                        checking.then_some("loader-circle"),
                        disabled,
                        &theme,
                        |_, _, cx| check_updates(cx),
                    ))
                    .when(available, |d| {
                        d.child(widgets::button(
                            "update-see-notes",
                            t(cx, "update.seeNotes"),
                            Kind::Primary,
                            Size::Normal,
                            None,
                            false,
                            &theme,
                            |_, window, cx| super::open_update_dialog(window, cx),
                        ))
                    }),
            )
            .when_some(error, |d, e| d.child(widgets::error(e, &theme)))
            .into_any_element();

        let channel = view.as_ref().map(|v| v.prefs.channel.clone());
        let channels = div()
            .child(widgets::legend(t(cx, "update.channelLegend"), &theme))
            .child(
                div()
                    .mt(px(space::MD))
                    .flex()
                    .flex_col()
                    .gap(px(space::MD))
                    .children(
                        [
                            ("main", "update.channelMainLabel", "update.channelMainHint"),
                            (
                                "releases",
                                "update.channelReleasesLabel",
                                "update.channelReleasesHint",
                            ),
                        ]
                        .into_iter()
                        .map(|(id, label, hint)| {
                            let on = channel.as_deref() == Some(id);
                            let edge = theme.c("line-hover");
                            let el = div()
                                .id(SharedString::from(format!("channel-{id}")))
                                .flex()
                                .items_start()
                                .gap(px(space::MD))
                                .px(px(space::LG))
                                .py(px(space::MD))
                                .rounded(px(radius::MD))
                                .border_1()
                                .border_color(theme.c("line-strong"))
                                .bg(if on {
                                    theme.c("control")
                                } else {
                                    theme.c("surface")
                                })
                                // The mark sits on the title's line.
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .h(px(leading::UI))
                                        .child(widgets::mark(Mark::Radio, on, &theme)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .child(ui::text(
                                            t(cx, label),
                                            size::UI,
                                            weight::STRONG,
                                            theme.c("ink"),
                                        ))
                                        .child(ui::text(
                                            t(cx, hint),
                                            size::SMALL,
                                            weight::REGULAR,
                                            theme.c("ink-3"),
                                        )),
                                );
                            if disabled {
                                el.opacity(0.5)
                            } else {
                                widgets::focusable(el, &theme, false)
                                    .cursor_pointer()
                                    .hover(move |s| s.border_color(edge))
                                    .on_click(move |_, _, cx| {
                                        set_update_prefs(json!({ "channel": id }), cx);
                                    })
                            }
                        }),
                    ),
            )
            .into_any_element();

        let auto = view.as_ref().map(|v| v.prefs.auto_check).unwrap_or(true);
        let background = div()
            .child(widgets::legend(t(cx, "update.backgroundLegend"), &theme))
            .child(div().child(widgets::check_row(
                "update-auto-check",
                auto,
                t(cx, "update.autoCheckLabel"),
                view.is_none(),
                &theme,
                move |_, _, cx| set_update_prefs(json!({ "auto_check": !auto }), cx),
            )))
            .child(div().child(widgets::hint(t(cx, "update.autoCheckHint"), &theme)))
            .into_any_element();

        let path = view.as_ref().and_then(|v| v.source.path.clone());
        let problem = view.as_ref().and_then(|v| v.source.problem.clone());
        let repo = div()
            .child(widgets::legend(t(cx, "update.repoLegend"), &theme))
            .child(widgets::hint(t(cx, "update.repoHint"), &theme))
            .when_some(path.clone(), |d, p| {
                d.child(widgets::mono(p, &theme).mt(px(space::MD)).truncate())
            })
            .map(|d| match (&problem, &path) {
                (Some(p), _) => d.child(div().mt(px(space::MD)).child(warn(
                    source_problem_text(p, cx),
                    false,
                    &theme,
                ))),
                (None, Some(_)) => d.child(widgets::hint(t(cx, "update.repoReady"), &theme)),
                _ => d,
            })
            .into_any_element();

        let blocks: Vec<AnyElement> = vec![status, channels, background, repo];
        widgets::sections(blocks, &theme).into_any_element()
    }
}
