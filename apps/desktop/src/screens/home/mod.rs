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
use crate::ui::{self, MONO, SANS, leading, size, space, weight};
use gpui::{
    AnyElement, AnyView, App, Context, Div, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, StyledText, TextRun, Window, div, font, px,
};

/// An empty state's text is at most this wide (F3).
const EMPTY_WIDTH: f32 = 320.;

/// The sidebar beside a Home screen. Shows the loading state, with no
/// sidebar, until the first read arrives. It lays the report pane out as a
/// right-hand column while open; the lightbox comes from the window root.
pub struct HomeShell {
    sidebar: Entity<sidebar::Sidebar>,
    content: AnyView,
}

impl HomeShell {
    pub fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let me = cx.entity_id();
        cx.on_release(move |_, cx| attachments::undock(me, cx))
            .detach();
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
        // Docked here, so the window root's floating copy stays out.
        let report = attachments::report_pane(window, cx);
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

/// The empty state (F3): an optional line of `UI` 14/700 in `ink-2` and one
/// sentence of `UI` 14/400 in `ink-3`, at most 320 px wide, `BREAK` below
/// the header and centred in the column.
pub fn empty_state(title: Option<SharedString>, body: impl IntoElement, theme: &Theme) -> Div {
    div()
        .w_full()
        .px(px(space::GUTTER))
        .pt(px(space::BREAK))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(space::XS))
        .text_center()
        .font_family(SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::REGULAR)
        .text_color(theme.c("ink-3"))
        .children(title.map(|title| {
            ui::text(title, size::UI, weight::STRONG, theme.c("ink-2")).max_w(px(EMPTY_WIDTH))
        }))
        .child(div().max_w(px(EMPTY_WIDTH)).child(body))
}
