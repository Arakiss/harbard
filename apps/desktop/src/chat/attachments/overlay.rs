//! The lightbox (`.hb-lightbox`) and the report pane (`aside.hb-report-pane`).

use super::blocks::{fit, text_view, wait};
use super::{File, Top, Watch, close, file, listen_for_escape, md, shown_label, store};
use crate::derive::markdown::{MARKDOWN_MAX, parse};
use crate::i18n::t;
use crate::model::Attachment;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, RenderOnce, StatefulInteractiveElement, Styled, Window, anchored,
    deferred, div, img, point, px,
};
use std::sync::Arc;

/// Most views told about an overlay change.
const MOUNTS: usize = 16;

pub(super) fn overlays(_window: &mut Window, _cx: &mut App) -> AnyElement {
    Overlays.into_any_element()
}

pub(super) fn docked_pane(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let me = window.current_view();
    let s = store(cx);
    s.docked.insert(me);
    remember(me, cx);
    store(cx)
        .report
        .is_some()
        .then(|| DockedPane.into_any_element())
}

fn remember(me: gpui::EntityId, cx: &mut App) {
    let s = store(cx);
    if !s.mounts.contains(&me) {
        s.mounts.push(me);
        if s.mounts.len() > MOUNTS {
            s.mounts.remove(0);
        }
    }
}

#[derive(IntoElement)]
struct Overlays;

impl RenderOnce for Overlays {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        listen_for_escape(cx);
        let me = window.current_view();
        remember(me, cx);
        let s = store(cx);
        let lightbox = s.lightbox.clone();
        // A live view lays the pane out as a column (`report_pane`).
        let report = s.report.clone().filter(|_| s.docked.is_empty());
        let watch =
            window.use_keyed_state(ElementId::Name("hb-overlays-watch".into()), cx, |_, _| {
                Watch
            });
        let vp = window.viewport_size();
        let mut layer = div();
        if let Some(att) = report {
            let w = px(680.).min(vp.width * 0.94);
            let pane = pane(&att, &watch, window, cx)
                .w(w)
                .h(vp.height)
                .shadow(cx.global::<Theme>().pop_shadow());
            layer = layer.child(
                deferred(anchored().position(point(vp.width - w, px(0.))).child(pane))
                    .with_priority(60),
            );
        }
        if let Some(att) = lightbox {
            let el = lightbox_view(&att, &watch, vp.width, vp.height, cx);
            layer = layer.child(
                deferred(anchored().position(point(px(0.), px(0.))).child(el)).with_priority(70),
            );
        }
        layer
    }
}

#[derive(IntoElement)]
struct DockedPane;

impl RenderOnce for DockedPane {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let watch = window.use_keyed_state(
            ElementId::Name("hb-docked-pane-watch".into()),
            cx,
            |_, _| Watch,
        );
        match store(cx).report.clone() {
            Some(att) => pane(&att, &watch, window, cx)
                .w(px(680.))
                .flex_none()
                .h_full()
                .into_any_element(),
            None => div().into_any_element(),
        }
    }
}

// ---------------------------------------------------------------------------
// The lightbox
// ---------------------------------------------------------------------------

fn lightbox_view(
    att: &Attachment,
    watch: &gpui::Entity<Watch>,
    vw: Pixels,
    vh: Pixels,
    cx: &mut App,
) -> AnyElement {
    let got = file(att, watch, cx);
    let theme = cx.global::<Theme>().clone();
    let (focus, id) = {
        let s = store(cx);
        (s.lightbox_focus.clone(), att.id.clone())
    };
    let picture: AnyElement = match got {
        File::Image(image, w, h) => {
            let (w, h) = fit(w, h, f32::from(vw) * 0.92, f32::from(vh) * 0.86);
            img(image)
                .w(px(w))
                .h(px(h))
                .rounded(px(8.))
                .bg(theme.c("surface"))
                .shadow(theme.pop_shadow())
                .into_any_element()
        }
        File::Loading => wait(false, cx).into_any_element(),
        _ => wait(true, cx).into_any_element(),
    };
    let caption = div()
        .mt(px(8.))
        .flex()
        .justify_center()
        .font_family(ui::SANS)
        .text_size(px(14.))
        .text_color(theme.c("accent-ink"))
        .children(att.label.as_ref().map(|l| {
            div()
                .font_weight(FontWeight::BOLD)
                .whitespace_nowrap()
                .child(format!("{} · ", shown_label(l, cx)))
        }))
        .child(att.name.clone());
    let close_id = id.clone();
    let mut button = div()
        .id("hb-lightbox-close")
        .absolute()
        .top(px(16.))
        .right(px(16.))
        .size(px(40.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.c("surface"))
        .cursor_pointer()
        .on_click(move |_, window, cx| close(Top::Lightbox(close_id.clone()), Some(window), cx))
        .child(ui::icon("x", 24., theme.c("ink")));
    if let Some(focus) = &focus {
        button = button.track_focus(focus);
    }
    let backdrop_id = id.clone();
    let key_id = id;
    div()
        .id("hb-lightbox")
        .occlude()
        .w(vw)
        .h(vh)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .bg(theme.c("scrim"))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            close(Top::Lightbox(backdrop_id.clone()), Some(window), cx)
        })
        .on_key_down(move |ev, window, cx| {
            if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                close(Top::Lightbox(key_id.clone()), Some(window), cx);
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .id("hb-lightbox-figure")
                .flex()
                .flex_col()
                .items_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(picture)
                .child(caption),
        )
        .child(button)
        .into_any_element()
}

// ---------------------------------------------------------------------------
// The report pane
// ---------------------------------------------------------------------------

fn pane(
    att: &Attachment,
    watch: &gpui::Entity<Watch>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Div {
    let got = file(att, watch, cx);
    let theme = cx.global::<Theme>().clone();
    let close_id = att.id.clone();
    let head = div()
        .h(px(56.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(18.))
        .pr(px(12.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(ui::icon("file-text", 20., theme.c("ink")))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(18.))
                .font_weight(FontWeight::BLACK)
                .text_color(theme.c("ink"))
                .child(att.name.clone()),
        )
        .children(att.label.as_ref().map(|l| {
            div()
                .flex_none()
                .text_size(px(13.))
                .text_color(theme.c("ink-3"))
                .child(shown_label(l, cx))
        }))
        .child(
            div()
                .id("hb-report-close")
                .ml_auto()
                .flex_none()
                .size(px(32.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover({
                    let c = theme.c("control");
                    move |s| s.bg(c)
                })
                .on_click(move |_, window, cx| {
                    close(Top::Report(close_id.clone()), Some(window), cx)
                })
                .child(ui::icon("x", 20., theme.c("ink-3"))),
        );
    let body: AnyElement = match got {
        File::Text(source) => report_body(att, &source, window, cx),
        File::Loading => wait(false, cx).into_any_element(),
        _ => wait(true, cx).into_any_element(),
    };
    let scroll = store(cx).report_scroll.clone();
    div()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(theme.c("line"))
        .bg(theme.c("surface"))
        .font_family(ui::SANS)
        .occlude()
        .child(head)
        .child(
            div()
                .id("hb-report-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .px(px(24.))
                .pt(px(18.))
                .pb(px(32.))
                .child(body),
        )
}

/// `MarkdownView`: parsed once per report; past `MARKDOWN_MAX` it is text.
fn report_body(att: &Attachment, source: &str, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.global::<Theme>().clone();
    if source.chars().count() > MARKDOWN_MAX {
        let full = window.use_keyed_state(
            ElementId::Name(format!("hb-report-full-{}", att.id).into()),
            cx,
            |_, _| false,
        );
        let shown = *full.read(cx);
        let on_full = move |_: &mut Window, cx: &mut App| {
            full.update(cx, |f, cx| {
                *f = true;
                cx.notify();
            })
        };
        return div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .text_size(px(15.))
            .text_color(theme.c("ink"))
            .child(t(cx, "attachments.reportTooLong"))
            .child(text_view(
                &format!("hb-report-{}", att.id),
                source,
                false,
                shown,
                on_full,
                cx,
            ))
            .into_any_element();
    }
    let blocks = {
        let s = store(cx);
        match &s.parsed {
            Some((id, blocks)) if id == &att.id => blocks.clone(),
            _ => {
                let blocks = Arc::new(parse(source));
                s.parsed = Some((att.id.clone(), blocks.clone()));
                blocks
            }
        }
    };
    md::view(&format!("hb-report-{}", att.id), &blocks, cx).into_any_element()
}
