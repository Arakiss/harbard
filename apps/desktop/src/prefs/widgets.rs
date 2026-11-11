//! The console's dialog vocabulary, drawn with our tokens: `.hb-btn`,
//! `.hb-close`, `.hb-mode` (a checkbox or radio in a bordered box),
//! `.hb-prefs-section`, `.hb-prefs-hint`, `.hb-chip`, `.hb-input`, and the
//! modal frame every dialog here opens in.

use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, control, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, BoxShadow, ClickEvent, Div, ElementId, Entity, Hsla,
    InteractiveElement, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use std::rc::Rc;

pub type Handler = Rc<dyn Fn(&mut Window, &mut App)>;

/// The width of one line of a key/value row's label (About).
pub const LABEL_W: f32 = 144.;
/// The box a checkbox or radio is drawn in.
const MARK: f32 = icon_size::ROW;

/// `.hb-btn` variants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Primary,
    Danger,
    Quiet,
}

/// `.hb-btn` sizes: `Large` is the one primary action of a dialog
/// (`control::LG`), `Normal` every other button (`control::MD`), `Small`
/// an action inside a row (`control::SM`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Large,
    Normal,
    Small,
}

/// The keyboard focus ring (F1) on anything clickable: reachable with Tab,
/// Enter and Space click it.
pub fn focusable(el: Stateful<Div>, theme: &Theme, inset: bool) -> Stateful<Div> {
    let ring = ui::focus_ring(theme, inset);
    el.focusable()
        .tab_stop(true)
        .focus_visible(move |s| s.shadow(vec![ring]))
}

/// A tooltip: the name and, after it, the key (K2, K3).
pub fn tip(
    label: impl Into<SharedString>,
    key: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label: SharedString = label.into();
    move |window, cx| {
        let label = label.clone();
        Tooltip::element(move |_, cx| {
            let theme = cx.global::<Theme>();
            div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .child(label.clone())
                .when_some(key, |d, key| d.child(ui::kbd(key, theme)))
        })
        .build(window, cx)
    }
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
    let (h, pad) = match size {
        Size::Large => (control::LG, space::XL),
        Size::Normal => (control::MD, space::LG),
        Size::Small => (control::SM, space::LG),
    };
    let (bg, border, ink, hover, pressed) = match kind {
        Kind::Plain => (
            theme.c("surface"),
            theme.c("line-strong"),
            theme.c("ink"),
            theme.c("btn-hover"),
            theme.c("control"),
        ),
        Kind::Primary => (
            theme.c("accent-deep"),
            theme.c("accent-deep"),
            theme.c("accent-ink"),
            theme.c("accent-hover"),
            theme.c("accent-hover"),
        ),
        Kind::Danger => (
            theme.c("surface"),
            theme.c("red-line"),
            theme.c("red-ink"),
            theme.c("red-bg"),
            theme.c("red-bg"),
        ),
        Kind::Quiet => (
            transparent(),
            transparent(),
            theme.c("ink-2"),
            theme.c("btn-hover"),
            theme.c("control"),
        ),
    };
    let el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .h(px(h))
        .px(px(pad))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(ink)
        .font_weight(weight::STRONG)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .whitespace_nowrap()
        .when_some(icon, |d, name| d.child(ui::icon(name, icon_size::ROW, ink)))
        .child(label.into());
    if disabled {
        return el.opacity(0.5);
    }
    focusable(el, theme, false)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .on_click(on)
}

pub fn transparent() -> Hsla {
    gpui::transparent_black()
}

/// `.hb-close`: the X of a dialog, `control::SM` with its name and key.
pub fn close_button(
    id: impl Into<ElementId>,
    theme: &Theme,
    on: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = theme.c("hover");
    let pressed = theme.c("control");
    focusable(div().id(id), theme, false)
        .ml_auto()
        .flex_none()
        .size(px(control::SM))
        .rounded(px(radius::BASE))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(pressed))
        .child(ui::icon("x", icon_size::ROW, theme.c("ink-3")))
        .tooltip(|window, cx| tip(t(cx, "shell.close"), Some("Esc"))(window, cx))
        .on_click(on)
}

/// A dialog's header (T4): 56 px, the one `TITLE` 18/900 at the gutter and
/// the X at the trailing edge.
pub fn dialog_head(title: impl Into<SharedString>, theme: &Theme) -> Div {
    dialog_head_closing(title, theme, close_top)
}

/// [`dialog_head`] whose X runs `close` (a dialog with something to undo).
pub fn dialog_head_closing(
    title: impl Into<SharedString>,
    theme: &Theme,
    close: fn(&mut Window, &mut App),
) -> Div {
    ui::pane_header(title, theme)
        .pr(px(space::LG))
        .child(close_button("dialog-close", theme, move |_, w, cx| {
            close(w, cx)
        }))
}

/// A dialog's footer: its actions at the trailing edge, over a hairline.
pub fn dialog_foot(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_end()
        .gap(px(space::MD))
        .px(px(space::GUTTER))
        .py(px(space::LG))
        .border_t_1()
        .border_color(theme.c("line"))
}

/// `.hb-prefs-legend`: a setting's title, `UI` 14/700.
pub fn legend(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::UI, weight::STRONG, theme.c("ink"))
}

/// A field's label over or beside its control: `UI` 14/700.
pub fn label(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::UI, weight::STRONG, theme.c("ink")).flex_none()
}

/// `.hb-prefs-hint`: one sentence under a title or a control, `SMALL` in `ink-3`.
pub fn hint(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-3")).mt(px(space::XS))
}

/// `.hb-error`.
pub fn error(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::SMALL, weight::REGULAR, theme.c("red-ink")).mt(px(space::XS))
}

/// `.hb-prefs-status`: the live line under a block ("Saved").
pub fn status_line(child: Option<AnyElement>, theme: &Theme) -> Div {
    div()
        .min_h(px(leading::SMALL))
        .mt(px(space::MD))
        .text_size(px(size::SMALL))
        .line_height(px(leading::SMALL))
        .text_color(theme.c("ink-2"))
        .children(child)
}

/// `.hb-banner`: harbard isn't responding.
pub fn banner(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .mt(px(space::MD))
        .px(px(space::LG))
        .py(px(space::MD))
        .rounded(px(radius::MD))
        .bg(theme.c("red-bg"))
        .text_color(theme.c("banner-ink"))
        .text_size(px(size::SMALL))
        .line_height(px(leading::SMALL))
        .child(text.into())
}

/// `.hb-chip`: a state word. Only a state takes its colour (`amber`, `red`,
/// `green`); anything else is neutral (C3).
pub fn chip(text: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    ui::chip(text, tone, theme).whitespace_nowrap()
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

/// A list of `.hb-prefs-section` blocks, `SECTION` of space apart (S3).
pub fn sections(blocks: Vec<AnyElement>, _theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(space::SECTION))
        .children(blocks)
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
        .size(px(MARK))
        .flex()
        .items_center()
        .justify_center()
        .border_1();
    let d = match kind {
        Mark::Check => d.rounded(px(radius::SM)),
        Mark::Radio => d.rounded_full(),
    };
    if on {
        let ink = theme.c("accent-ink");
        d.bg(theme.c("accent"))
            .border_color(theme.c("accent"))
            .child(match kind {
                Mark::Check => ui::icon("check", icon_size::INLINE, ink).into_any_element(),
                Mark::Radio => div()
                    .size(px(space::SM))
                    .rounded_full()
                    .bg(ink)
                    .into_any_element(),
            })
    } else {
        d.bg(theme.c("surface")).border_color(theme.c("line-hover"))
    }
}

/// `.hb-mode`: one choice of a few, a `control::MD` box with the mark, an
/// optional icon and the label.
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
    let hover = theme.c("line-hover");
    let el = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::MD))
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .bg(if on {
            theme.c("control")
        } else {
            theme.c("surface")
        })
        .border_1()
        .border_color(theme.c("line-strong"))
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .text_color(if on { theme.c("ink") } else { theme.c("ink-2") })
        .whitespace_nowrap()
        .child(mark(kind, on, theme))
        .when_some(icon, |d, name| {
            d.child(ui::icon(name, icon_size::ROW, theme.c("ink-3")))
        })
        .child(label.into());
    if disabled {
        return el.opacity(0.5);
    }
    focusable(el, theme, false)
        .cursor_pointer()
        .hover(move |s| s.border_color(hover))
        .on_click(click)
}

/// A setting that is on or off: the mark and its label on one
/// `control::MD` line, no box around it.
pub fn check_row(
    id: impl Into<ElementId>,
    on: bool,
    label: impl Into<SharedString>,
    disabled: bool,
    theme: &Theme,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(space::MD))
        .min_h(px(control::MD))
        .mt(px(space::XS))
        .rounded(px(radius::BASE))
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .text_color(theme.c("ink"))
        .child(mark(Mark::Check, on, theme))
        .child(label.into());
    if disabled {
        return el.opacity(0.5);
    }
    focusable(el, theme, false).cursor_pointer().on_click(click)
}

/// A row of `.hb-mode` boxes.
pub fn mode_row(children: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap(px(space::MD))
        .mt(px(space::MD))
        .children(children)
}

/// `.hb-input` around a GPUI Kit input: `control::MD`, 12 px inside (S4).
pub fn text_input(state: &Entity<InputState>, theme: &Theme) -> Input {
    // `Input::h` sizes only multi-line inputs; the style's height wins for one line.
    Styled::h(Input::new(state), px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(size::UI))
}

/// `.hb-mono`: something to copy, two pixels under the text beside it (T6).
pub fn mono(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .font_family(ui::MONO)
        .text_size(px(size::META))
        .line_height(px(leading::SMALL))
        .text_color(theme.c("ink-2"))
        .child(text.into())
}

/// The face, size and ink every dialog body starts from (the dialog layer
/// renders outside the window's own root).
pub fn base(theme: &Theme) -> Div {
    div()
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
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
        let vh = f32::from(window.viewport_size().height);
        let view = view.clone();
        let close = on_close.clone();
        ui::kit_theme::hb_dialog(dialog, cx)
            .w(px(width))
            .p_0()
            .gap_0()
            .close_button(false)
            .overflow_hidden()
            .when_some(height, |d, h| d.h(px(h.min(vh - 2. * space::SECTION))))
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

/// A `<select>`: the current label with a chevron, `control::MD`; when
/// `open`, the choices float below it. `toggle` opens and closes it; a click
/// outside closes it too.
pub fn dropdown(
    id: impl Into<SharedString>,
    current: impl Into<SharedString>,
    open: bool,
    choices: Vec<Choice>,
    disabled: bool,
    theme: &Theme,
    toggle: Handler,
) -> AnyElement {
    let id: SharedString = id.into();
    let t2 = toggle.clone();
    let hover = theme.c("menu-hover");
    let hover_ink = theme.c("menu-hover-ink");
    let edge = theme.c("line-hover");
    let trigger = div()
        .id(SharedString::from(format!("{id}-trigger")))
        .flex()
        .items_center()
        .gap(px(space::MD))
        .h(px(control::MD))
        .pl(px(space::LG))
        .pr(px(space::MD))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::REGULAR)
        .whitespace_nowrap()
        .child(div().flex_1().min_w_0().truncate().child(current.into()))
        .child(ui::icon("chevron-down", icon_size::ROW, theme.c("ink-3")));
    let trigger = if disabled {
        trigger.opacity(0.5)
    } else {
        focusable(trigger, theme, false)
            .cursor_pointer()
            .hover(move |s| s.border_color(edge))
            .on_click(move |_, w, cx| toggle(w, cx))
    };
    let menu = div()
        .id(SharedString::from(format!("{id}-menu")))
        .occlude()
        .min_w(px(MENU_MIN_W))
        .max_h(px(MENU_MAX_H))
        .overflow_y_scroll()
        .p(px(space::XS))
        .rounded(px(radius::MD))
        .bg(theme.c("surface-raised"))
        .shadow(theme.pop_shadow())
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::REGULAR)
        .text_color(theme.c("ink"))
        .on_mouse_down_out(move |_, w, cx| t2(w, cx))
        .children(choices.into_iter().enumerate().map(|(i, c)| {
            let on = c.on.clone();
            div()
                .id(SharedString::from(format!("{id}-opt-{i}")))
                .flex()
                .items_center()
                .gap(px(space::MD))
                .h(px(row::MENU))
                .px(px(space::MD))
                .rounded(px(radius::BASE))
                .whitespace_nowrap()
                .cursor_pointer()
                .hover(move |s| s.bg(hover).text_color(hover_ink))
                .child(
                    div()
                        .flex_none()
                        .size(px(icon_size::ROW))
                        .when(c.selected, |d| {
                            d.child(ui::icon("check", icon_size::ROW, theme.c("ink-3")))
                        }),
                )
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
                    .top(px(control::MD + space::XS))
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

/// A floating menu is at least this wide and scrolls beyond this height.
const MENU_MIN_W: f32 = 160.;
const MENU_MAX_H: f32 = 320.;
