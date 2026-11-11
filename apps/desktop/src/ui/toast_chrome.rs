//! In-app toasts (the console's `ui/sonner.tsx`): a short line after an
//! action that otherwise ends silently — a room or a workspace created,
//! settings saved — stacked at the window's bottom-right, newest at the
//! bottom, each gone after a few seconds or on a click.
//!
//! ```ignore
//! toast_chrome::success(t(cx, "toast.profileSaved"), cx);
//! toast_chrome::error(reason, cx);
//! ```
//!
//! A toast slides up 8 px and fades in over `motion::BASE` and fades out
//! over `motion::FAST`; with reduced motion (M7) it simply appears and goes.

use super::{icon, icon_size, motion, radius, size, space, weight};
use crate::theme::Theme;
use gpui::{
    Animation, AnimationExt as _, App, AppContext, Context, ElementId, Entity, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, deferred, div, ease_out_quint, px,
};
use std::time::Duration;

/// What a toast reports; only its icon and the icon's colour differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

impl ToastKind {
    fn icon(self) -> &'static str {
        match self {
            ToastKind::Success => "circle-check",
            ToastKind::Error => "octagon-x",
            ToastKind::Info => "info",
        }
    }

    /// A state colour only for a state (C3): done is green, failed is red.
    fn tone(self) -> &'static str {
        match self {
            ToastKind::Success => "green",
            ToastKind::Error => "red",
            ToastKind::Info => "ink-3",
        }
    }

    /// How long it stays: sonner's four seconds; a failure a little longer.
    pub fn lifetime(self) -> Duration {
        match self {
            ToastKind::Error => ERROR_LIFETIME,
            _ => LIFETIME,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Toast {
    id: u64,
    kind: ToastKind,
    text: SharedString,
    leaving: bool,
}

/// The toasts on screen, oldest first.
#[derive(Default)]
pub struct ToastStack {
    items: Vec<Toast>,
    next: u64,
}

impl ToastStack {
    /// Add one; past [`MAX`] the oldest goes. Returns its id.
    fn push(&mut self, kind: ToastKind, text: SharedString) -> u64 {
        self.next += 1;
        let id = self.next;
        // The same words twice in a row are one toast, renewed.
        self.items.retain(|t| !(t.text == text && t.kind == kind));
        self.items.push(Toast {
            id,
            kind,
            text,
            leaving: false,
        });
        let live = self.items.iter().filter(|t| !t.leaving).count();
        if live > MAX
            && let Some(oldest) = self.items.iter_mut().find(|t| !t.leaving)
        {
            oldest.leaving = true;
        }
        id
    }

    /// Start `id`'s exit; false when it is already gone or going.
    fn leave(&mut self, id: u64) -> bool {
        match self.items.iter_mut().find(|t| t.id == id && !t.leaving) {
            Some(t) => {
                t.leaving = true;
                true
            }
            None => false,
        }
    }

    fn remove(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    fn removable(&self) -> Vec<u64> {
        self.items
            .iter()
            .filter(|t| t.leaving)
            .map(|t| t.id)
            .collect()
    }
}

/// The layer, drawn by the shell over everything (dialogs too).
pub struct ToastLayer {
    stack: ToastStack,
}

struct Layer(Entity<ToastLayer>);

impl Global for Layer {}

/// Create the layer once, at startup.
pub fn init(cx: &mut App) {
    if cx.try_global::<Layer>().is_none() {
        let layer = cx.new(|_| ToastLayer {
            stack: ToastStack::default(),
        });
        cx.set_global(Layer(layer));
    }
}

/// The layer, for the shell to place as its last child.
pub fn layer(cx: &mut App) -> impl IntoElement + use<> {
    init(cx);
    cx.global::<Layer>().0.clone()
}

pub fn success(text: impl Into<SharedString>, cx: &mut App) {
    show(ToastKind::Success, text.into(), cx);
}

pub fn error(text: impl Into<SharedString>, cx: &mut App) {
    show(ToastKind::Error, text.into(), cx);
}

pub fn info(text: impl Into<SharedString>, cx: &mut App) {
    show(ToastKind::Info, text.into(), cx);
}

/// Show `text`; it leaves by itself after its kind's lifetime.
pub fn show(kind: ToastKind, text: SharedString, cx: &mut App) {
    init(cx);
    let layer = cx.global::<Layer>().0.clone();
    let id = layer.update(cx, |l, cx| {
        let id = l.stack.push(kind, text);
        l.sweep(cx);
        cx.notify();
        id
    });
    let weak = layer.downgrade();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(kind.lifetime()).await;
        let _ = weak.update(cx, |l, cx| l.dismiss(id, cx));
    })
    .detach();
}

impl ToastLayer {
    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.stack.leave(id) {
            self.sweep(cx);
            cx.notify();
        }
    }

    /// Drop the toasts that are leaving once their fade is over.
    fn sweep(&mut self, cx: &mut Context<Self>) {
        let gone = self.stack.removable();
        if gone.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(motion::FAST).await;
            let _ = this.update(cx, |l, cx| {
                for id in gone {
                    l.stack.remove(id);
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ToastLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.stack.items.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        let still = crate::robot::clock::reduced(cx);
        let toasts = self.stack.items.clone().into_iter().map(|toast| {
            let id = toast.id;
            let card = div()
                .id(ElementId::NamedInteger("hb-toast".into(), id))
                .occlude()
                .w(px(WIDTH))
                .flex()
                .items_start()
                .gap(px(space::MD))
                .px(px(space::XL))
                .py(px(space::LG))
                .rounded(px(radius::MD))
                .bg(theme.c("surface-raised"))
                // It floats: the pop shadow and its ring, no border (C6).
                .shadow(theme.pop_shadow())
                .font_family(super::SANS)
                .cursor_pointer()
                .child(div().flex_none().pt(px(space::HAIR)).child(icon(
                    toast.kind.icon(),
                    icon_size::ROW,
                    theme.c(toast.kind.tone()),
                )))
                .child(
                    super::text(toast.text.clone(), size::UI, weight::STRONG, theme.c("ink"))
                        .flex_1()
                        .min_w_0()
                        .line_clamp(2)
                        .text_ellipsis(),
                )
                .on_click(cx.listener(move |l, _, _, cx| l.dismiss(id, cx)));
            if still {
                return card.into_any_element();
            }
            if toast.leaving {
                card.with_animation(
                    ElementId::NamedInteger("hb-toast-out".into(), id),
                    Animation::new(motion::FAST),
                    |d, t| d.opacity(1. - t),
                )
                .into_any_element()
            } else {
                card.with_animation(
                    ElementId::NamedInteger("hb-toast-in".into(), id),
                    Animation::new(motion::BASE).with_easing(ease_out_quint()),
                    |d, t| d.opacity(t).mt(px(TRAVEL * (1. - t))),
                )
                .into_any_element()
            }
        });
        div()
            .absolute()
            .right(px(space::SECTION))
            .bottom(px(space::SECTION))
            .child(
                deferred(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(space::MD))
                        .children(toasts),
                )
                // Over the dialog layer and its popups.
                .with_priority(PRIORITY),
            )
            .into_any_element()
    }
}

/// sonner's width; the most on screen at once; how far one rises in.
const WIDTH: f32 = 356.;
const MAX: usize = 3;
const TRAVEL: f32 = 8.;
const LIFETIME: Duration = Duration::from_secs(4);
const ERROR_LIFETIME: Duration = Duration::from_secs(6);
const PRIORITY: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn past_the_most_the_oldest_starts_to_leave() {
        let mut s = ToastStack::default();
        for n in 0..4 {
            s.push(ToastKind::Info, format!("t{n}").into());
        }
        assert_eq!(s.removable(), vec![1]);
        assert_eq!(s.items.iter().filter(|t| !t.leaving).count(), MAX);
    }

    #[test]
    fn the_same_words_again_renew_the_toast() {
        let mut s = ToastStack::default();
        s.push(ToastKind::Success, "Saved".into());
        let id = s.push(ToastKind::Success, "Saved".into());
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].id, id);
        // A different kind with the same words is a different toast.
        s.push(ToastKind::Error, "Saved".into());
        assert_eq!(s.items.len(), 2);
    }

    #[test]
    fn a_toast_leaves_once() {
        let mut s = ToastStack::default();
        let id = s.push(ToastKind::Success, "Done".into());
        assert!(s.leave(id));
        assert!(!s.leave(id));
        s.remove(id);
        assert!(s.items.is_empty());
        assert!(!s.leave(id));
    }

    #[test]
    fn a_failure_stays_longer() {
        assert!(ToastKind::Error.lifetime() > ToastKind::Success.lifetime());
        assert_eq!(ToastKind::Success.lifetime(), Duration::from_secs(4));
    }
}
