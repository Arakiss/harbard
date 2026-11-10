//! The title bar's update pill (`update-banner.tsx`): a new version is
//! available, a run is going, or one finished and the app should restart.
//! The shell places [`update_pill`] at the right end of the top band.

use super::stores::UpdateStore;
use crate::derive::update::{BannerMode, banner_line};
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, px,
};

/// The pill, or `None` when there is nothing to say. Re-render when
/// [`UpdateStore`] notifies.
pub fn update_pill(cx: &App) -> Option<AnyElement> {
    let store = UpdateStore::get(cx).read(cx);
    let mode = store.banner()?;
    let theme = cx.global::<Theme>();
    let hover = theme.c("rail-hover");
    let text_ink = theme.c("rail-text");
    let pill = div()
        .flex()
        .items_center()
        .gap(px(2.))
        .h(px(28.))
        .pl(px(2.))
        .pr(px(if mode == BannerMode::Running { 2. } else { 4. }))
        .rounded(px(14.))
        .bg(theme.c("rail-tile"))
        .text_color(theme.c("rail-ink"))
        .border_1()
        .border_color(theme.c("rail-ring"))
        .text_size(px(13.))
        .whitespace_nowrap();
    let label = |id: &'static str| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(8.))
            .rounded(px(12.))
            .font_weight(FontWeight::BOLD)
    };
    let ink = theme.c("rail-ink");
    let out = match mode {
        BannerMode::Running => pill.child(
            label("update-pill-text")
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(ui::icon("loader-circle", 14., ink))
                .child(if store.restarting {
                    t(cx, "update.restarting")
                } else {
                    t(cx, "update.bannerRunning")
                })
                .on_click(|_, window, cx| super::open_update_dialog(window, cx)),
        ),
        // The app restarts itself after an update; until it does, say so.
        BannerMode::Done => pill.child(label("update-pill-text").child(t(cx, "update.bannerDone"))),
        BannerMode::Available => {
            let check = store.view.as_ref().and_then(|v| v.check.as_ref())?;
            let problem = store.view.as_ref().and_then(|v| v.source.problem.clone());
            let on_ink = theme.c("rail-on-ink");
            pill.child(
                label("update-pill-text")
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(ui::icon("circle-arrow-up", 14., ink))
                    .child(banner_line(check, cx))
                    .on_click(|_, window, cx| super::open_update_dialog(window, cx)),
            )
            .child(
                div()
                    .id("update-pill-link")
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(12.))
                    .text_color(text_ink)
                    .underline()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(t(cx, "update.seeNotes"))
                    .on_click(|_, window, cx| super::open_update_dialog(window, cx)),
            )
            .child(
                div()
                    .id("update-pill-action")
                    .h(px(22.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(11.))
                    .font_weight(FontWeight::BLACK)
                    .bg(ink)
                    .text_color(on_ink)
                    .cursor_pointer()
                    .child(t(cx, "update.update"))
                    .on_click(move |_, window, cx| {
                        super::open_update_dialog(window, cx);
                        if problem.is_none() {
                            UpdateStore::install(cx);
                        }
                    }),
            )
            .child(
                div()
                    .id("update-pill-close")
                    .size(px(22.))
                    .rounded(px(11.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(ui::icon("x", 14., text_ink))
                    .on_click(|_, _, cx| UpdateStore::dismiss(cx)),
            )
        }
    };
    Some(out.into_any_element())
}
