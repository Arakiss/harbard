//! The block under a message: views, the before/after pair, thumbnails,
//! report cards and text attachments, in the console's order.

use super::{
    File, Watch, file, files_named_by, huginn_view, open_lightbox, open_report, shown_label,
};
use crate::derive::markdown::{DiffKind, PREVIEW_LINES, diff_lines, pretty_json, preview};
use crate::i18n::{t, tv};
use crate::model::{Attachment, Message};
use crate::theme::Theme;
use crate::ui;
use crate::ui::{control, icon_size, leading, radius, size, space, weight};

/// The widest a message's attachments run.
const COLUMN: f32 = 760.;
/// A thumbnail's box, and its placeholder while the file loads.
const THUMB: (f32, f32) = (280., 180.);
const THUMB_WAIT: (f32, f32) = (96., 64.);
/// A full-width image's height while it loads.
const IMAGE_WAIT: f32 = 120.;
/// The compare slider: its rule, its track and its knob.
const RULE: f32 = space::HAIR;
const TRACK: f32 = space::XS;
const KNOB: f32 = icon_size::INLINE;
/// A report card's least and greatest width.
const REPORT_CARD: (f32, f32) = (240., 420.);
/// The tallest a diff or a log grows before it scrolls.
const MONO_MAX: f32 = 420.;
use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Entity, EntityId, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, StyledImage, Subscription,
    Window, canvas, div, img, px, relative,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Lines of a diff shown before "Show everything".
const DIFF_LINES: usize = 60;

/// What the block remembers between frames.
#[derive(Default)]
struct Ui {
    /// The pair under the slider instead of side by side.
    slider: bool,
    /// The slider's position, 0–100 (the console starts at 50).
    at: Option<f32>,
    /// Attachments opened in full ("Show everything").
    full: HashSet<String>,
    /// Dragging the slider: over the stage or along its track.
    drag: Option<Drag>,
    stage: Rc<Cell<Bounds<Pixels>>>,
    track: Rc<Cell<Bounds<Pixels>>>,
    /// Re-render when an embedded view finishes reading.
    subs: HashMap<EntityId, Subscription>,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Stage,
    Track,
}

#[derive(IntoElement)]
pub(super) struct MessageAttachments {
    pub msg: Message,
    pub whole: bool,
}

impl RenderOnce for MessageAttachments {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = format!("hb-atts-{}-{}", self.msg.id, self.whole);
        let ui = window.use_keyed_state(ElementId::Name(format!("{key}-ui").into()), cx, |_, _| {
            Ui::default()
        });
        let watch = window.use_keyed_state(
            ElementId::Name(format!("{key}-watch").into()),
            cx,
            |_, _| Watch,
        );
        let all = &self.msg.attachments;
        let views: Vec<&Attachment> = all.iter().filter(|a| a.kind == "huginn").collect();
        if views.is_empty() {
            return attachments(&key, all.iter().collect(), &ui, &watch, cx)
                .unwrap_or_else(|| div().into_any_element());
        }
        let mut named: HashSet<String> = HashSet::new();
        let mut pending = false;
        let mut out = div().flex().flex_col().min_w_0();
        for a in &views {
            let view = huginn_view(a, &self.msg, self.whole, window, cx);
            if !ui.read(cx).subs.contains_key(&view.entity_id()) {
                let v = view.clone();
                ui.update(cx, |u, cx| {
                    let sub = cx.observe(&v, |_, _, cx| cx.notify());
                    u.subs.insert(v.entity_id(), sub);
                });
            }
            match files_named_by(&view, cx) {
                Some(files) => named.extend(files),
                None => pending = true,
            }
            out = out.child(view);
        }
        // While the views load, their images are held back so none flashes as a thumbnail.
        let rest: Vec<&Attachment> = all
            .iter()
            .filter(|a| {
                a.kind != "huginn" && !named.contains(&a.name) && !(pending && a.kind == "image")
            })
            .collect();
        out.children(attachments(&key, rest, &ui, &watch, cx))
            .into_any_element()
    }
}

/// `Attachments`: everything but views. None when there is nothing to draw.
fn attachments(
    key: &str,
    list: Vec<&Attachment>,
    ui: &Entity<Ui>,
    watch: &Entity<Watch>,
    cx: &mut App,
) -> Option<AnyElement> {
    let pair = before_after(&list);
    let images: Vec<&Attachment> = list
        .iter()
        .copied()
        .filter(|a| a.kind == "image" && pair.is_none_or(|(b, af)| a.id != b.id && a.id != af.id))
        .collect();
    let reports: Vec<&Attachment> = list
        .iter()
        .copied()
        .filter(|a| a.kind == "markdown")
        .collect();
    let texts: Vec<&Attachment> = list
        .iter()
        .copied()
        .filter(|a| matches!(a.kind.as_str(), "diff" | "text" | "json"))
        .collect();
    if pair.is_none() && images.is_empty() && reports.is_empty() && texts.is_empty() {
        return None;
    }
    let mut root = div()
        .flex()
        .flex_col()
        .gap(px(space::MD))
        .mt(px(space::MD))
        .max_w(px(COLUMN))
        .min_w_0();
    if let Some((before, after)) = pair {
        root = root.child(compare(key, before, after, ui, watch, cx));
    }
    if !images.is_empty() {
        let thumbs: Vec<AnyElement> = images.iter().map(|a| thumb(key, a, watch, cx)).collect();
        root = root.child(div().flex().flex_wrap().gap(px(space::MD)).children(thumbs));
    }
    for a in reports {
        root = root.child(report_card(key, a, cx));
    }
    for a in texts {
        root = root.child(text_attachment(key, a, ui, watch, cx));
    }
    Some(root.into_any_element())
}

/// The first image labelled before and the first labelled after (`beforeAfter`).
fn before_after<'a>(list: &[&'a Attachment]) -> Option<(&'a Attachment, &'a Attachment)> {
    let find = |label: &str| {
        list.iter()
            .copied()
            .find(|a| a.kind == "image" && a.label.as_deref() == Some(label))
    };
    Some((find("before")?, find("after")?))
}

fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

/// `alt`: "{label}: {name}", or the name.
fn alt(a: &Attachment, cx: &App) -> String {
    match &a.label {
        Some(l) => format!("{}: {}", shown_label(l, cx), a.name),
        None => a.name.clone(),
    }
}

/// The wait line (`p.hb-att-wait`): reading, or gone.
pub(super) fn wait(failed: bool, cx: &App) -> Div {
    let key = if failed {
        "attachments.readFailed"
    } else {
        "attachments.loading"
    };
    div()
        .font_family(ui::SANS)
        .text_size(px(size::SMALL))
        .text_color(theme(cx).c("ink-4"))
        .child(t(cx, key))
}

/// An image scaled down to fit a box, never up.
pub(super) fn fit(w: u32, h: u32, max_w: f32, max_h: f32) -> (f32, f32) {
    let (w, h) = (w as f32, h as f32);
    let scale = (max_w / w).min(max_h / h).min(1.0);
    (w * scale, h * scale)
}

// ---------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------

fn thumb(key: &str, a: &Attachment, watch: &Entity<Watch>, cx: &mut App) -> AnyElement {
    let got = file(a, watch, cx);
    let theme = theme(cx);
    let inner: AnyElement = match got {
        File::Image(image, w, h) => {
            let (w, h) = fit(w, h, THUMB.0, THUMB.1);
            img(image)
                .w(px(w))
                .h(px(h))
                .object_fit(ObjectFit::Cover)
                .into_any_element()
        }
        File::Loading => div()
            .w(px(THUMB_WAIT.0))
            .h(px(THUMB_WAIT.1))
            .into_any_element(),
        _ => div()
            .p(px(space::MD))
            .max_w(px(THUMB.0))
            .text_size(px(size::SMALL))
            .text_color(theme.c("ink-4"))
            .child(alt(a, cx))
            .into_any_element(),
    };
    let label = a.label.as_ref().map(|l| {
        div()
            .absolute()
            .left(px(space::SM))
            .bottom(px(space::SM))
            .px(px(space::SM))
            .rounded(px(radius::SM))
            .text_size(px(size::META))
            .line_height(px(leading::META))
            .font_weight(weight::STRONG)
            .bg(theme.c("surface"))
            .text_color(theme.c("ink"))
            .child(shown_label(l, cx))
    });
    let att = a.clone();
    let title: SharedString = tv(cx, "attachments.enlarge", &[("name", &a.name)]);
    div()
        .id(ElementId::Name(format!("{key}-thumb-{}", a.id).into()))
        .relative()
        .overflow_hidden()
        .rounded(px(radius::MD))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface-2"))
        .cursor_pointer()
        .tooltip(move |window, cx| ui_tooltip(title.clone(), window, cx))
        .on_click(move |_, window, cx| open_lightbox(&att, window, cx))
        .child(inner)
        .children(label)
        .into_any_element()
}

/// A plain tooltip (the console's `title`).
fn ui_tooltip(text: SharedString, window: &mut Window, cx: &mut App) -> gpui::AnyView {
    crate::ui::kit_theme::tip(text)(window, cx)
}

// ---------------------------------------------------------------------------
// Before and after
// ---------------------------------------------------------------------------

fn compare(
    key: &str,
    before: &Attachment,
    after: &Attachment,
    ui: &Entity<Ui>,
    watch: &Entity<Watch>,
    cx: &mut App,
) -> AnyElement {
    let b = file(before, watch, cx);
    let a = file(after, watch, cx);
    let (slider, at) = {
        let u = ui.read(cx);
        (u.slider, u.at.unwrap_or(50.))
    };
    let theme = theme(cx).clone();
    let tab = |id: &str, icon: &'static str, label: SharedString, on: bool, to_slider: bool| {
        let ui = ui.clone();
        let mut el = div()
            .id(ElementId::Name(format!("{key}-tab-{id}").into()))
            .flex()
            .items_center()
            .gap(px(space::SM))
            .h(px(control::SM))
            .px(px(space::MD))
            .rounded(px(radius::BASE))
            .text_size(px(size::SMALL))
            .line_height(px(leading::SMALL))
            .font_weight(if on { weight::STRONG } else { weight::REGULAR })
            .cursor_pointer()
            .tab_index(0)
            .focus_visible({
                let ring = ui::focus_ring(&theme, false);
                move |s| s.shadow(vec![ring])
            })
            .on_click(move |_, _, cx| {
                ui.update(cx, |u, cx| {
                    u.slider = to_slider;
                    cx.notify();
                })
            });
        let color = if on { theme.c("ink") } else { theme.c("ink-3") };
        if on {
            el = el.bg(theme.c("surface"));
        }
        el.text_color(color)
            .child(ui::icon(icon, icon_size::ROW, color))
            .child(label)
    };
    let tabs = div().flex().child(
        div()
            .flex()
            .gap(px(space::HAIR))
            .p(px(space::HAIR))
            .mb(px(space::MD))
            .rounded(px(radius::MD))
            .bg(theme.c("control"))
            .child(tab(
                "side",
                "columns-2",
                t(cx, "attachments.sideBySide"),
                !slider,
                false,
            ))
            .child(tab(
                "slider",
                "chevrons-left-right",
                t(cx, "attachments.slider"),
                slider,
                true,
            )),
    );
    let caption = |s: SharedString| {
        div()
            .mt(px(space::XS))
            .text_size(px(size::SMALL))
            .font_weight(weight::STRONG)
            .text_color(theme.c("ink-3"))
            .child(s)
    };
    let body: AnyElement = if !slider {
        let figure = |att: &Attachment, got: &File, word: SharedString| {
            let att2 = att.clone();
            let title: SharedString = tv(cx, "attachments.enlarge", &[("name", &att.name)]);
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id(ElementId::Name(format!("{key}-side-{}", att.id).into()))
                        .w_full()
                        .cursor_pointer()
                        .tooltip(move |window, cx| ui_tooltip(title.clone(), window, cx))
                        .on_click(move |_, window, cx| open_lightbox(&att2, window, cx))
                        .child(full_width(got, &theme, cx)),
                )
                .child(caption(word))
        };
        div()
            .flex()
            .gap(px(space::MD))
            .child(figure(before, &b, t(cx, "attachments.before")))
            .child(figure(after, &a, t(cx, "attachments.after")))
            .into_any_element()
    } else {
        slider_view(key, &b, &a, at, ui, &theme, cx)
    };
    div()
        .border_1()
        .border_color(theme.c("line-strong"))
        .rounded(px(radius::MD))
        .bg(theme.c("surface-2"))
        .p(px(space::MD))
        .child(tabs)
        .child(body)
        .into_any_element()
}

/// An image at the full width of its column, its height by its shape
/// (`.hb-compare img`).
fn full_width(got: &File, theme: &Theme, cx: &App) -> AnyElement {
    match got {
        File::Image(image, w, h) => div()
            .w_full()
            .aspect_ratio(*w as f32 / *h as f32)
            .rounded(px(radius::BASE))
            .border_1()
            .border_color(theme.c("line"))
            .bg(theme.c("surface"))
            .overflow_hidden()
            .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
            .into_any_element(),
        File::Loading => div().w_full().h(px(IMAGE_WAIT)).into_any_element(),
        _ => wait(true, cx).into_any_element(),
    }
}

fn slider_view(
    key: &str,
    b: &File,
    a: &File,
    at: f32,
    ui: &Entity<Ui>,
    theme: &Theme,
    cx: &mut App,
) -> AnyElement {
    let frac = at / 100.;
    let (stage_cell, track_cell, dragging) = {
        let u = ui.read(cx);
        (u.stage.clone(), u.track.clone(), u.drag)
    };
    // The after image, clipped from the left at the slider (`inset(0 0 0 at%)`).
    let top = match a {
        File::Image(image, ..) if frac < 1. => Some(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .right_0()
                .left(relative(frac))
                .overflow_hidden()
                .child(
                    img(image.clone())
                        .absolute()
                        .top_0()
                        .h_full()
                        .left(relative(-frac / (1. - frac)))
                        .w(relative(1. / (1. - frac)))
                        .object_fit(ObjectFit::Cover),
                ),
        ),
        _ => None,
    };
    let base: AnyElement = match b {
        File::Image(image, w, h) => div()
            .w_full()
            .aspect_ratio(*w as f32 / *h as f32)
            .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
            .into_any_element(),
        File::Loading => div().w_full().h(px(IMAGE_WAIT)).into_any_element(),
        _ => wait(true, cx).into_any_element(),
    };
    let set = |which: Drag| {
        let ui = ui.clone();
        move |ev: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            ui.update(cx, |u, cx| {
                let bounds = if which == Drag::Stage {
                    u.stage.get()
                } else {
                    u.track.get()
                };
                u.at = Some(percent(ev.position.x, bounds));
                u.drag = Some(which);
                cx.notify();
            });
        }
    };
    let bounds_of = |cell: Rc<Cell<Bounds<Pixels>>>| {
        let ui = ui.clone();
        canvas(
            move |bounds, _, _| cell.set(bounds),
            move |_, _, window, _| {
                let Some(which) = dragging else { return };
                let ui_move = ui.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    if ev.pressed_button != Some(MouseButton::Left) {
                        return;
                    }
                    ui_move.update(cx, |u, cx| {
                        let bounds = if which == Drag::Stage {
                            u.stage.get()
                        } else {
                            u.track.get()
                        };
                        u.at = Some(percent(ev.position.x, bounds));
                        cx.notify();
                    });
                });
                let ui_up = ui.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                    ui_up.update(cx, |u, cx| {
                        u.drag = None;
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .size_full()
    };
    let stage = div()
        .id(ElementId::Name(format!("{key}-stage").into()))
        .relative()
        .w_full()
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c("line"))
        .bg(theme.c("surface"))
        .overflow_hidden()
        .cursor(gpui::CursorStyle::ResizeLeftRight)
        .on_mouse_down(MouseButton::Left, set(Drag::Stage))
        .child(base)
        .children(top)
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(RULE))
                .ml(px(-RULE / 2.))
                .left(relative(frac))
                .bg(theme.c("accent")),
        )
        .child(bounds_of(stage_cell));
    let track = div()
        .id(ElementId::Name(format!("{key}-track").into()))
        .relative()
        .flex_1()
        .h(px(leading::SMALL))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, set(Drag::Track))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px((leading::SMALL - TRACK) / 2.))
                .h(px(TRACK))
                .rounded(px(radius::SM))
                .bg(theme.c("line-strong")),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .top(px((leading::SMALL - TRACK) / 2.))
                .h(px(TRACK))
                .rounded(px(radius::SM))
                .w(relative(frac))
                .bg(theme.c("accent")),
        )
        .child(
            div()
                .absolute()
                .top(px((leading::SMALL - KNOB) / 2.))
                .left(relative(frac))
                .ml(px(-KNOB / 2.))
                .size(px(KNOB))
                .rounded_full()
                .bg(theme.c("accent")),
        )
        .child(bounds_of(track_cell));
    let legend = div()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .mt(px(space::XS))
        .text_size(px(size::SMALL))
        .font_weight(weight::STRONG)
        .text_color(theme.c("ink-3"))
        .child(t(cx, "attachments.before"))
        .child(track)
        .child(t(cx, "attachments.after"));
    div().child(stage).child(legend).into_any_element()
}

/// Where along `bounds` a point falls, 0–100 in whole steps like the range input.
fn percent(x: Pixels, bounds: Bounds<Pixels>) -> f32 {
    let w = f32::from(bounds.size.width);
    if w <= 0. {
        return 50.;
    }
    ((f32::from(x - bounds.origin.x) / w) * 100.)
        .round()
        .clamp(0., 100.)
}

// ---------------------------------------------------------------------------
// Reports and text
// ---------------------------------------------------------------------------

fn report_card(key: &str, a: &Attachment, cx: &App) -> AnyElement {
    let theme = theme(cx);
    let att = a.clone();
    let title: SharedString = tv(cx, "attachments.readInPane", &[("name", &a.name)]);
    let (hover_bg, hover_line) = (theme.c("file-hover"), theme.c("line-hover"));
    div()
        .id(ElementId::Name(format!("{key}-report-{}", a.id).into()))
        .overflow_hidden()
        .flex()
        .items_center()
        .gap(px(space::LG))
        .w_full()
        .max_w(px(REPORT_CARD.1))
        .min_w(px(REPORT_CARD.0))
        .px(px(space::LG))
        .py(px(space::MD))
        .rounded(px(radius::MD))
        .border_1()
        .border_color(theme.c("line"))
        .bg(theme.c("surface-2"))
        .font_family(ui::SANS)
        .font_weight(weight::STRONG)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .text_color(theme.c("ink"))
        .tab_index(0)
        .focus_visible({
            let ring = ui::focus_ring(theme, false);
            move |s| s.shadow(vec![ring])
        })
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg).border_color(hover_line))
        .tooltip(move |window, cx| ui_tooltip(title.clone(), window, cx))
        .on_click(move |_, _, cx| open_report(&att, cx))
        .child(ui::icon("file-text", icon_size::RAIL, theme.c("ink-2")))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .overflow_hidden()
                .child(div().truncate().child(a.name.clone()))
                .child(
                    div()
                        .truncate()
                        .font_weight(weight::REGULAR)
                        .text_size(px(size::META))
                        .line_height(px(leading::META))
                        .text_color(theme.c("ink-3"))
                        .child(tv(
                            cx,
                            "attachments.reportSummary",
                            &[("size", &ui::format::size(a.bytes))],
                        )),
                ),
        )
        .into_any_element()
}

fn text_attachment(
    key: &str,
    a: &Attachment,
    ui: &Entity<Ui>,
    watch: &Entity<Watch>,
    cx: &mut App,
) -> AnyElement {
    let got = file(a, watch, cx);
    let full = ui.read(cx).full.contains(&a.id);
    let theme = theme(cx);
    let icon = if a.kind == "diff" {
        "file-code-corner"
    } else {
        "scroll-text"
    };
    let name = div()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .mb(px(space::XS))
        .font_family(ui::SANS)
        .text_size(px(size::SMALL))
        .font_weight(weight::STRONG)
        .text_color(theme.c("ink-2"))
        .child(ui::icon(icon, icon_size::ROW, theme.c("ink-3")))
        .child(a.name.clone())
        .child(
            div()
                .font_weight(weight::REGULAR)
                .text_color(theme.c("ink-3"))
                .child(ui::format::size(a.bytes)),
        );
    let id = a.id.clone();
    let ui = ui.clone();
    let open_full = move |_: &mut Window, cx: &mut App| {
        ui.update(cx, |u, cx| {
            u.full.insert(id.clone());
            cx.notify();
        })
    };
    let eid = format!("{key}-text-{}", a.id);
    let content: AnyElement = match got {
        File::Text(text) if a.kind == "diff" => diff_view(&eid, &text, full, open_full, cx),
        File::Text(text) => text_view(&eid, &text, a.kind == "json", full, open_full, cx),
        File::Loading => wait(false, cx).into_any_element(),
        _ => wait(true, cx).into_any_element(),
    };
    div()
        .min_w_0()
        .child(name)
        .child(content)
        .into_any_element()
}

/// The frame a diff or a log sits in (`.hb-diff`, `.hb-log`).
fn mono_box(id: &str, theme: &Theme) -> gpui::Stateful<Div> {
    div()
        .id(ElementId::Name(id.to_string().into()))
        .max_h(px(MONO_MAX))
        .overflow_y_scroll()
        .rounded(px(radius::MD))
        .bg(theme.c("surface-2"))
        .font_family(ui::MONO)
        .text_size(px(size::META))
        .line_height(px(leading::SMALL))
}

/// "Show everything ({n} more lines)" (`.hb-att-more`).
pub(super) fn more_button(
    id: &str,
    n: usize,
    on: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = theme(cx);
    div()
        .flex()
        .child(
            div()
                .id(ElementId::Name(format!("{id}-more").into()))
                .mt(px(space::XS))
                .font_family(ui::SANS)
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                .font_weight(weight::REGULAR)
                .text_color(theme.c("link"))
                .rounded(px(radius::SM))
                .cursor_pointer()
                .hover(|s| s.underline())
                .tab_index(0)
                .focus_visible({
                    let ring = ui::focus_ring(theme, false);
                    move |s| s.shadow(vec![ring])
                })
                .on_click(move |_, window, cx| on(window, cx))
                .child(tv(cx, "attachments.showFull", &[("n", &n.to_string())])),
        )
        .into_any_element()
}

/// A unified diff with its add and remove colours (`DiffView`).
fn diff_view(
    id: &str,
    source: &str,
    full: bool,
    on_full: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = theme(cx);
    let lines = diff_lines(source);
    let total = lines.len();
    let shown = if full { total } else { total.min(DIFF_LINES) };
    let rows: Vec<AnyElement> = lines
        .into_iter()
        .take(shown)
        .map(|(kind, text)| {
            let (bg, fg) = match kind {
                DiffKind::Add => (Some(theme.c("green-bg")), theme.c("green-ink")),
                DiffKind::Del => (Some(theme.c("red-bg")), theme.c("red-ink")),
                DiffKind::Hunk => (Some(theme.c("tint")), theme.c("link")),
                DiffKind::File => (None, theme.c("ink")),
                DiffKind::Context => (None, theme.c("ink-2")),
            };
            let mut row = div()
                .px(px(space::MD))
                .text_color(fg)
                .child(if text.is_empty() {
                    " ".to_string()
                } else {
                    text
                });
            if let Some(bg) = bg {
                row = row.bg(bg);
            }
            if kind == DiffKind::File {
                row = row.font_weight(weight::STRONG);
            }
            row.into_any_element()
        })
        .collect();
    let mut out = div()
        .min_w_0()
        .child(mono_box(&format!("{id}-diff"), theme).children(rows));
    if total > shown {
        out = out.child(more_button(id, total - shown, on_full, cx));
    }
    out.into_any_element()
}

/// A log or a text file in monospace: the first lines, then "Show everything" (`TextView`).
pub(super) fn text_view(
    id: &str,
    source: &str,
    json: bool,
    full: bool,
    on_full: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = theme(cx);
    let text = if json {
        pretty_json(source)
    } else {
        source.to_string()
    };
    let (head, more) = preview(&text, PREVIEW_LINES);
    let shown = if full { text } else { head };
    let mut out = div().min_w_0().child(
        mono_box(&format!("{id}-log"), theme)
            .px(px(space::MD))
            .py(px(space::MD))
            .text_color(theme.c("ink"))
            .child(shown),
    );
    if !full && more > 0 {
        out = out.child(more_button(id, more, on_full, cx));
    }
    out.into_any_element()
}
