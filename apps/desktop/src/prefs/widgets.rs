//! The console's dialog vocabulary, drawn with our tokens: `.hb-btn`,
//! `.hb-close`, `.hb-mode` (a checkbox or radio in a bordered box),
//! `.hb-prefs-section`, `.hb-prefs-hint`, `.hb-chip`, `.hb-input`, and the
//! modal frame every dialog here opens in.

use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, BoxShadow, ClickEvent, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use std::rc::Rc;

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// `.hb-btn` variants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Primary,
    Danger,
    Quiet,
}

/// `.hb-btn` sizes: 36 (dialogs), 32 (device rows), 28 (About's row).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Normal,
    Small,
    Tiny,
}

#[allow(clippy::too_many_arguments)]
/// A button: `kind`, `size`, an optional leading icon, the label; does
/// nothing and fades to half when disabled.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: Kind,
    size: Size,
    icon: Option<&str>,
    disabled: bool,
    theme: &Theme,
    on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (h, pad, text) = match size {
        Size::Normal => (36., 16., 15.),
        Size::Small => (32., 12., 14.),
        Size::Tiny => (28., 12., 13.),
    };
    let (bg, border, ink, hover) = match kind {
        Kind::Plain => (
            theme.c("surface"),
            theme.c("line-strong"),
            theme.c("ink"),
            theme.c("btn-hover"),
        ),
        Kind::Primary => (
            theme.c("accent-deep"),
            theme.c("accent-deep"),
            theme.c("accent-ink"),
            theme.c("accent-hover"),
        ),
        Kind::Danger => (
            theme.c("surface"),
            theme.c("red-line"),
            theme.c("red-ink"),
            theme.c("red-bg"),
        ),
        Kind::Quiet => (
            theme.c("surface"),
            transparent(),
            theme.c("ink-2"),
            theme.c("btn-hover"),
        ),
    };
    let ink2 = ink;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(h))
        .px(px(pad))
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(ink)
        .font_weight(FontWeight::BOLD)
        .text_size(px(text))
        .whitespace_nowrap()
        .when_some(icon, |d, name| d.child(ui::icon(name, 16., ink2)))
        .child(label.into())
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| {
            d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(on)
        })
}

pub fn transparent() -> Hsla {
    gpui::hsla(0., 0., 0., 0.)
}

/// `.hb-close`: a 32px square with the X.
pub fn close_button(
    id: impl Into<ElementId>,
    theme: &Theme,
    on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = theme.c("control");
    div()
        .id(id)
        .ml_auto()
        .flex_none()
        .size(px(32.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(ui::icon("x", 20., theme.c("ink-3")))
        .on_click(on)
}

/// `.hb-prefs-legend`: a setting's title with its icon.
pub fn legend(icon: &str, text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(17.))
        .font_weight(FontWeight::BLACK)
        .text_color(theme.c("ink"))
        .child(ui::icon(icon, 18., theme.c("ink-3")))
        .child(text.into())
}

/// `.hb-prefs-hint`.
pub fn hint(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .mt(px(2.))
        .mb(px(12.))
        .text_size(px(14.))
        .line_height(px(20.))
        .text_color(theme.c("ink-3"))
        .child(text.into())
}

/// `.hb-error`.
pub fn error(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_size(px(13.))
        .text_color(theme.c("red-ink"))
        .child(text.into())
}

/// `.hb-prefs-status`: the live line under a block.
pub fn status_line(child: Option<AnyElement>, theme: &Theme) -> Div {
    div()
        .min_h(px(22.))
        .mt(px(12.))
        .text_size(px(14.))
        .text_color(theme.c("ink-2"))
        .children(child)
}

/// `.hb-banner`: harbard isn't responding.
pub fn banner(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .mt(px(10.))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(theme.c("red-bg"))
        .text_color(theme.c("banner-ink"))
        .text_size(px(14.))
        .child(text.into())
}

/// `.hb-chip` with an optional tone (`amber`, `red`, `green`, `accent`).
pub fn chip(text: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    let (bg, ink, ring) = match tone {
        "amber" => (theme.c("amber-bg"), theme.c("amber-ink"), None),
        "red" => (
            theme.c("red-bg"),
            theme.c("red-ink"),
            Some(theme.c("red-line")),
        ),
        "green" => (
            theme.c("green-bg"),
            theme.c("green-ink"),
            Some(theme.c("green-line")),
        ),
        "accent" => (theme.c("accent-soft"), theme.c("accent-text"), None),
        _ => (theme.c("chip"), theme.c("chip-ink"), None),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .rounded(px(6.))
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .bg(bg)
        .text_color(ink)
        .when_some(ring, |d, c| d.shadow(vec![ring_shadow(c, 1., true)]))
        .child(text.into())
}

pub fn ring_shadow(color: Hsla, spread: f32, inset: bool) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset,
    }
}

/// A list of `.hb-prefs-section` blocks: each ruled below but the last.
pub fn sections(blocks: Vec<AnyElement>, theme: &Theme) -> Div {
    let n = blocks.len();
    div()
        .flex()
        .flex_col()
        .children(blocks.into_iter().enumerate().map(|(i, b)| {
            div()
                .py(px(18.))
                .when(i + 1 < n, |d| d.border_b_1().border_color(theme.c("line")))
                .child(b)
        }))
}

/// The box a checkbox or radio sits in (`.hb-mode`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Check,
    Radio,
}

/// The mark itself, macOS-like: a box or a ring, filled with the accent when on.
pub fn mark(kind: Mark, on: bool, theme: &Theme) -> Div {
    let d = div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .border_1();
    let d = match kind {
        Mark::Check => d.rounded(px(3.5)),
        Mark::Radio => d.rounded_full(),
    };
    if on {
        let ink = theme.c("accent-ink");
        d.bg(theme.c("accent"))
            .border_color(theme.c("accent"))
            .child(match kind {
                Mark::Check => ui::icon("check", 11., ink).into_any_element(),
                Mark::Radio => div().size(px(5.)).rounded_full().bg(ink).into_any_element(),
            })
    } else {
        d.bg(theme.c("surface")).border_color(theme.c("line-hover"))
    }
}

/// `.hb-mode`: a bordered box with the mark, an optional icon and the label.
#[allow(clippy::too_many_arguments)]
pub fn mode_box(
    id: impl Into<ElementId>,
    kind: Mark,
    on: bool,
    icon: Option<&str>,
    label: impl Into<SharedString>,
    disabled: bool,
    theme: &Theme,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let accent = theme.c("accent");
    let icon_ink = if on { theme.c("ink") } else { theme.c("ink-3") };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(theme.c("surface-raised"))
        .border_1()
        .border_color(if on { accent } else { theme.c("line-strong") })
        .when(on, |d| d.shadow(vec![ring_shadow(accent, 1., false)]))
        .font_weight(FontWeight::BOLD)
        .text_size(px(15.))
        .text_color(theme.c("ink"))
        .child(mark(kind, on, theme))
        .when_some(icon, |d, name| d.child(ui::icon(name, 16., icon_ink)))
        .child(label.into())
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| d.cursor_pointer().on_click(click))
}

/// A row of `.hb-mode` boxes.
pub fn mode_row(children: Vec<AnyElement>) -> Div {
    div().flex().flex_wrap().gap(px(12.)).children(children)
}

/// `.hb-input` around a GPUI Kit input.
pub fn text_input(state: &Entity<InputState>, theme: &Theme) -> Input {
    // `Input::h` sizes only multi-line inputs; the style's height wins for one line.
    Styled::h(Input::new(state), px(36.))
        .px(px(10.))
        .rounded(px(6.))
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(15.))
}

/// `.hb-mono`.
pub fn mono(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .font_family(ui::MONO)
        .text_size(px(13.))
        .text_color(theme.c("ink"))
        .child(text.into())
}

/// The face, size and ink every dialog body starts from (the dialog layer
/// renders outside the window's own root).
pub fn base(theme: &Theme) -> Div {
    div()
        .font_family(ui::SANS)
        .text_size(px(ui::size::BODY))
        .line_height(px(22.))
        .text_color(theme.c("ink"))
}

/// Where a dialog sits: centred (`.hb-scrim`) or this far down the window
/// (`.hb-finder-scrim`, 12vh; the update dialog, 8vh).
#[derive(Clone, Copy)]
pub enum Place {
    Center,
    Top(f32),
}

/// Open `view` as a modal: our surface (`--hb-surface-raised`, radius 12,
/// the pop ring), Esc and a click outside close it and run `on_close`.
pub fn open_modal(
    view: AnyView,
    width: f32,
    height: Option<f32>,
    place: Place,
    on_close: Option<Handler>,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, window, cx| {
        let theme = cx.global::<Theme>();
        let vh = f32::from(window.viewport_size().height);
        let ring = if theme.dark {
            gpui::hsla(0., 0., 1., 0.10)
        } else {
            gpui::hsla(0., 0., 0.11, 0.13)
        };
        let view = view.clone();
        let close = on_close.clone();
        dialog
            .w(px(width))
            .p_0()
            .gap_0()
            .close_button(false)
            .bg(theme.c("surface-raised"))
            .border_color(ring)
            .rounded(px(12.))
            .overflow_hidden()
            .when_some(height, |d, h| d.h(px(h.min(vh - 48.))))
            .map(|d| match place {
                Place::Center => d.centered(),
                Place::Top(f) => d.margin_top(px(vh * f)),
            })
            .when_some(close, |d, close| {
                d.on_close(move |_, window, cx| close(window, cx))
            })
            .content(move |c, _, _| c.p_0().gap_0().min_h_0().child(view.clone()))
    });
}

/// Put the caret in `input` once the dialog that holds it has taken focus
/// (`data-autofocus`).
pub fn autofocus(input: Entity<InputState>, window: &mut Window, cx: &mut App) {
    window.defer(cx, move |window, cx| {
        input.update(cx, |s, cx| s.focus(window, cx))
    });
}

/// Close the newest dialog (our X, Done, Cancel).
pub fn close_top(window: &mut Window, cx: &mut App) {
    window.close_dialog(cx);
}

/// One choice of a [`dropdown`].
pub struct Choice {
    pub label: SharedString,
    pub selected: bool,
    pub on: Handler,
}

/// A `<select>`: the current label with a chevron; when `open`, the
/// choices float below it. `toggle` opens and closes it; a click outside
/// closes it too.
#[allow(clippy::too_many_arguments)]
pub fn dropdown(
    id: impl Into<SharedString>,
    current: impl Into<SharedString>,
    open: bool,
    height: f32,
    choices: Vec<Choice>,
    disabled: bool,
    theme: &Theme,
    toggle: Handler,
) -> AnyElement {
    let id: SharedString = id.into();
    let t2 = toggle.clone();
    let hover = theme.c("menu-hover");
    let hover_ink = theme.c("menu-hover-ink");
    let trigger = div()
        .id(SharedString::from(format!("{id}-trigger")))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(height))
        .pl(px(10.))
        .pr(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .font_weight(FontWeight::NORMAL)
        .whitespace_nowrap()
        .child(div().flex_1().child(current.into()))
        .child(ui::icon("chevron-down", 14., theme.c("ink-3")))
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| {
            d.cursor_pointer().on_click(move |_, w, cx| toggle(w, cx))
        });
    let menu = div()
        .id(SharedString::from(format!("{id}-menu")))
        .occlude()
        .min_w(px(160.))
        .max_h(px(320.))
        .overflow_y_scroll()
        .py(px(6.))
        .rounded(px(8.))
        .bg(theme.c("surface-raised"))
        .shadow(theme.pop_shadow())
        .text_size(px(14.))
        .text_color(theme.c("ink"))
        .on_mouse_down_out(move |_, w, cx| t2(w, cx))
        .children(choices.into_iter().enumerate().map(|(i, c)| {
            let on = c.on.clone();
            div()
                .id(SharedString::from(format!("{id}-opt-{i}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(6.))
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(hover_ink))
                .child(div().w(px(14.)).when(c.selected, |d| {
                    d.child(ui::icon("check", 14., theme.c("ink-3")))
                }))
                .child(c.label)
                .on_click(move |_, w, cx| on(w, cx))
        }));
    div()
        .relative()
        .flex_none()
        .child(trigger)
        .when(open, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(height + 4.))
                    .left_0()
                    // Above the dialog layer (10 + its index), as GPUI Kit's own popups (100).
                    .child(
                        gpui::deferred(gpui::anchored().snap_to_window().child(menu))
                            .with_priority(100),
                    ),
            )
        })
        .into_any_element()
}
