//! Home (`home.tsx`): the sidebar every Home route shares, the channel
//! screen (`/`, `/c/<id>`), the Threads screen and the thread pane.
//!
//! [`HomeShell`] puts the shared [`sidebar::Sidebar`] beside any screen, as
//! the console's `HomeShell` did for Home, Threads, Deliveries and Lobby.

pub mod channel;
pub mod header;
pub mod members;
pub mod sidebar;
pub mod thread_pane;
pub mod threads;
pub mod work_panel;

use crate::chat::attachments;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{self, MONO, SANS};
use gpui::{
    AnyElement, AnyView, App, Context, Div, Entity, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Styled, StyledText, TextRun, Window, div, font, px,
};

/// The sidebar beside a Home screen. Shows the loading state, with no
/// sidebar, until the first read arrives. It also hosts the lightbox and
/// the report pane (a right-hand column while open).
pub struct HomeShell {
    sidebar: Entity<sidebar::Sidebar>,
    content: AnyView,
}

impl HomeShell {
    pub fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        HomeShell {
            sidebar: sidebar::shared(window, cx),
            content,
        }
    }
}

impl Render for HomeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = cx.global::<Theme>().c("surface");
        if Desk::get(cx).read(cx).view.is_none() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .bg(surface)
                .child(loading(cx))
                .into_any_element();
        }
        // Docked first, so the floating copy in `overlays` stays out.
        let report = attachments::report_pane(window, cx);
        let overlays = attachments::overlays(window, cx);
        div()
            .size_full()
            .flex()
            .child(self.sidebar.clone())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .child(self.content.clone()),
            )
            .children(report)
            .child(overlays)
            .into_any_element()
    }
}

/// `Loading` (`parts.tsx`): reading, or serve is not answering.
pub fn loading(cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let error = Desk::get(cx).read(cx).error.clone();
    let Some(error) = error else {
        return empty_state(None, t(cx, "messages.loading"), theme).into_any_element();
    };
    // "Couldn't read the workspace ({error}). Check that `harbard serve` is running."
    let line = tv(cx, "messages.loadFailed", &[("error", &error)]);
    let (before, after) = line.split_once("{command}").unwrap_or((&line, ""));
    let command = "harbard serve";
    let text = format!("{before}{command}{after}");
    let run = |len: usize, family: &'static str| TextRun {
        len,
        font: font(family),
        color: theme.c("ink-3"),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs = vec![
        run(before.len(), SANS),
        run(command.len(), MONO),
        run(after.len(), SANS),
    ]
    .into_iter()
    .filter(|r| r.len > 0)
    .collect();
    empty_state(
        Some(t(cx, "messages.serveDown")),
        StyledText::new(text).with_runs(runs),
        theme,
    )
    .into_any_element()
}

/// `div.hb-empty`: a centred note at the top of a column, with an optional
/// `<strong>` title on its own line.
pub fn empty_state(title: Option<SharedString>, body: impl IntoElement, theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(28.))
        .py(px(40.))
        .flex()
        .flex_col()
        .items_center()
        .text_center()
        .font_family(SANS)
        .text_size(px(ui::size::BODY))
        .text_color(theme.c("ink-3"))
        .children(title.map(|title| {
            div()
                .mb(px(6.))
                .text_size(px(17.))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.c("ink"))
                .child(title)
        }))
        .child(body)
}
