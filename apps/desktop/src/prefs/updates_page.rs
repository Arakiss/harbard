//! Preferences › Updates (`update-section.tsx`): when it last looked, which
//! changes count as a new version, the background check and the checkout it
//! updates from.

use super::shell_update::UpdateStore;
use super::stores::{check_updates, set_update_prefs};
use super::widgets::{self, Kind, Mark, Size};
use crate::derive::update::{source_problem_text, status_sentence};
use crate::i18n::t;
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use serde_json::json;
use std::time::Duration;

pub struct UpdatesPage {
    _clock: Task<()>,
}

impl UpdatesPage {
    pub fn new(_window: &mut gpui::Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify()).detach();
        // "Last checked: 5 min ago" moves on its own.
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(30)).await;
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
        (theme.c("warn-line"), theme.c("warn-bg"), theme.c("warn-ink"))
    };
    div()
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(line)
        .bg(bg)
        .text_color(ink)
        .text_size(px(14.))
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
            .child(widgets::legend("info", t(cx, "update.statusLegend"), &theme))
            .child(widgets::hint(status_sentence(view.as_ref(), now, cx), &theme))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(12.))
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
            .when_some(error, |d, e| d.child(div().mt(px(8.)).child(widgets::error(e, &theme))))
            .into_any_element();

        let channel = view.as_ref().map(|v| v.prefs.channel.clone());
        let channels = div()
            .child(widgets::legend("git-branch", t(cx, "update.channelLegend"), &theme))
            .child(
                div().mt(px(6.)).flex().flex_col().gap(px(8.)).children(
                    [
                        ("main", "update.channelMainLabel", "update.channelMainHint"),
                        ("releases", "update.channelReleasesLabel", "update.channelReleasesHint"),
                    ]
                    .into_iter()
                    .map(|(id, label, hint)| {
                        let on = channel.as_deref() == Some(id);
                        div()
                            .id(SharedString::from(format!("channel-{id}")))
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .px(px(12.))
                            .py(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if on { theme.c("accent") } else { theme.c("line") })
                            .when(on, |d| d.bg(theme.c("accent-soft")))
                            .when(disabled, |d| d.opacity(0.5))
                            .when(!disabled, |d| {
                                d.cursor_pointer().on_click(move |_, _, cx| {
                                    set_update_prefs(json!({ "channel": id }), cx);
                                })
                            })
                            .child(div().mt(px(4.)).child(widgets::mark(Mark::Radio, on, &theme)))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(FontWeight::BOLD).child(t(cx, label)))
                                    .child(div().text_size(px(14.)).text_color(theme.c("ink-3")).child(t(cx, hint))),
                            )
                    }),
                ),
            )
            .into_any_element();

        let auto = view.as_ref().map(|v| v.prefs.auto_check).unwrap_or(true);
        let background = div()
            .child(widgets::legend("clock", t(cx, "update.backgroundLegend"), &theme))
            .child(
                div().mt(px(6.)).child(
                    widgets::mode_box(
                        "update-auto-check",
                        Mark::Check,
                        auto,
                        None,
                        t(cx, "update.autoCheckLabel"),
                        view.is_none(),
                        &theme,
                        move |_, _, cx| set_update_prefs(json!({ "auto_check": !auto }), cx),
                    )
                    .w_full(),
                ),
            )
            .child(div().child(widgets::hint(t(cx, "update.autoCheckHint"), &theme)))
            .into_any_element();

        let path = view.as_ref().and_then(|v| v.source.path.clone());
        let problem = view.as_ref().and_then(|v| v.source.problem.clone());
        let repo = div()
            .child(widgets::legend("folder-git-2", t(cx, "update.repoLegend"), &theme))
            .child(widgets::hint(t(cx, "update.repoHint"), &theme))
            .when_some(path.clone(), |d, p| {
                d.child(
                    div()
                        .mb(px(8.))
                        .font_family(crate::ui::MONO)
                        .text_size(px(13.))
                        .text_color(theme.c("ink-2"))
                        .child(p),
                )
            })
            .map(|d| match (&problem, &path) {
                (Some(p), _) => d.child(warn(source_problem_text(p, cx), false, &theme)),
                (None, Some(_)) => d.child(widgets::hint(t(cx, "update.repoReady"), &theme)),
                _ => d,
            })
            .into_any_element();

        let blocks: Vec<AnyElement> = vec![status, channels, background, repo];
        widgets::sections(blocks, &theme).into_any_element()
    }
}
