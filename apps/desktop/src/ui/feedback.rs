//! One-tap feedback (`feedback.tsx`, `lib/feedback.ts`): one row of
//! controls beside what it asks about, never a modal, never shown again once
//! answered (until undone). [`feedback_row`] is the slot (`FeedbackRow`);
//! [`dock_feedback`] is "The Dock bounced at 14:02 · This bothered me"
//! (`DockFeedback`), offered for half an hour after [`record_bounce`].
//!
//! The standing answers are read once (serve's feedback read, in-process)
//! and kept in one app-wide [`FeedbackStore`]; every tap updates it from
//! serve's answer.

use crate::desk::Desk;
use crate::shell::profile::{ProfileStore, now_ms};
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{SANS, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, Global, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

/// How long the acknowledgement stays in sight after a tap.
const ACK: Duration = Duration::from_secs(6);
/// How long "This bothered me" stays offered after a bounce.
pub const BOUNCE_OFFER_MS: u64 = 30 * 60_000;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct FeedbackRecord {
    pub id: String,
    pub ms: u64,
    pub target_kind: String,
    pub target_id: String,
    pub value: String,
    pub reason: Option<String>,
    pub undone: bool,
}

/// What one target kind asks: an optional question, its options, the acknowledgements.
struct Control {
    question: Option<&'static str>,
    /// (value, label key, thumb icon, asks for a reason)
    options: &'static [(&'static str, &'static str, Option<&'static str>, bool)],
    ack: &'static [(&'static str, &'static str)],
}

fn control(kind: &str) -> Option<Control> {
    Some(match kind {
        "message" => Control {
            question: None,
            options: &[
                ("up", "feedback.messageUp", Some("thumbs-up"), false),
                ("unclear", "feedback.messageUnclear", Some("thumbs-down"), false),
            ],
            ack: &[("up", "feedback.ackMessageUp"), ("unclear", "feedback.ackMessageUnclear")],
        },
        "delivery" => Control {
            question: Some("feedback.deliveryQuestion"),
            options: &[
                ("yes", "feedback.deliveryYes", None, false),
                ("no", "feedback.deliveryNo", None, true),
                ("missing", "feedback.deliveryMissing", None, true),
            ],
            ack: &[
                ("yes", "feedback.ackDeliveryYes"),
                ("no", "feedback.ackDeliveryNo"),
                ("missing", "feedback.ackDeliveryMissing"),
            ],
        },
        "needs_you" => Control {
            question: None,
            options: &[("not_needed", "feedback.needsYouNotNeeded", None, false)],
            ack: &[("not_needed", "feedback.ackNeedsYouNotNeeded")],
        },
        "robot_state" => Control {
            question: None,
            options: &[("wrong", "feedback.robotWrong", None, false)],
            ack: &[("wrong", "feedback.ackRobotWrong")],
        },
        "dock" => Control {
            question: None,
            options: &[("annoying", "feedback.dockAnnoying", None, false)],
            ack: &[("annoying", "feedback.ackDockAnnoying")],
        },
        _ => return None,
    })
}

/// The one target a message row asks about: a DONE from an agent is a
/// delivery, any other message not the operator's is a message.
pub fn message_target(from: &str, label: Option<&str>) -> Option<&'static str> {
    match (from, label) {
        ("operator", _) => None,
        ("agent", Some("DONE")) => Some("delivery"),
        _ => Some("message"),
    }
}

/// Coordination states in which nothing asks for feedback.
pub fn feedback_allowed(operator_state: Option<&str>) -> bool {
    !matches!(operator_state, Some("focused" | "swamped" | "away"))
}

/// `bounce:<ms>`.
pub fn bounce_target(at_ms: u64) -> String {
    format!("bounce:{at_ms}")
}

/// A reason being written for one option.
struct Asking {
    kind: String,
    id: String,
    value: String,
    sample: Option<Value>,
    input: Entity<InputState>,
    _enter: Subscription,
}

/// The operator's standing answers and what is being asked right now.
pub struct FeedbackStore {
    standing: Vec<FeedbackRecord>,
    last: Option<FeedbackRecord>,
    /// The operator's coordination state id, from the shell's profile store.
    pub operator_state: Option<String>,
    asking: Option<Asking>,
    /// The record acknowledged a moment ago: shown even without hover.
    fresh: Option<String>,
    busy: bool,
    error: Option<(String, SharedString)>,
    bounce: Option<(u64, String)>,
    _tasks: Vec<Task<()>>,
    _profile: Subscription,
}

#[derive(Clone)]
struct StoreHandle(Entity<FeedbackStore>);

impl Global for StoreHandle {}

impl FeedbackStore {
    /// The app-wide store, created (and read once) on first use.
    pub fn get(cx: &mut App) -> Entity<FeedbackStore> {
        if let Some(h) = cx.try_global::<StoreHandle>() {
            return h.0.clone();
        }
        let store = cx.new(|cx: &mut Context<FeedbackStore>| {
            let hub_store = Desk::get(cx).read(cx).hub.store().clone();
            let read = cx.spawn(async move |this, cx| {
                let body = cx
                    .background_spawn(async move { harbard_cli::serve::feedback_route::feedback_body(&hub_store) })
                    .await;
                let standing: Vec<FeedbackRecord> =
                    serde_json::from_value(body.get("standing").cloned().unwrap_or_default()).unwrap_or_default();
                let last: Option<FeedbackRecord> = serde_json::from_value(body.get("last").cloned().unwrap_or_default()).ok();
                let _ = this.update(cx, |this, cx| {
                    this.standing = standing;
                    this.last = last;
                    cx.notify();
                });
            });
            // Nothing asks for feedback while the operator is Focused,
            // Swamped or Away: follow the profile's state as it changes.
            let profile = ProfileStore::get(cx);
            let follow = cx.observe(&profile, |this: &mut FeedbackStore, profile, cx| {
                let state = profile.read(cx).state_id(now_ms()).to_string();
                if this.operator_state.as_deref() != Some(state.as_str()) {
                    this.operator_state = Some(state);
                    cx.notify();
                }
            });
            let operator_state = Some(profile.read(cx).state_id(now_ms()).to_string());
            FeedbackStore {
                standing: Vec::new(),
                last: None,
                operator_state,
                asking: None,
                fresh: None,
                busy: false,
                error: None,
                bounce: None,
                _tasks: vec![read],
                _profile: follow,
            }
        });
        cx.set_global(StoreHandle(store.clone()));
        store
    }

    fn standing_for(&self, kind: &str, id: &str) -> Option<&FeedbackRecord> {
        self.standing
            .iter()
            .find(|r| r.target_kind == kind && r.target_id == id && !r.undone)
    }

    fn tap(&mut self, kind: &str, id: &str, value: &str, reason: Option<String>, sample: Option<Value>, cx: &mut Context<Self>) {
        let mut body = json!({ "target_kind": kind, "target_id": id, "value": value });
        if let Some(r) = reason.filter(|r| !r.trim().is_empty()) {
            body["reason"] = json!(r.trim());
        }
        if let Some(s) = sample {
            body["sample"] = s;
        }
        self.busy = true;
        self.error = None;
        let target = format!("{kind}:{id}");
        let task = Desk::get(cx).update(cx, |d, cx| d.post("feedback", body, cx));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match out.ok().and_then(|v| serde_json::from_value::<FeedbackRecord>(v.get("record")?.clone()).ok()) {
                    Some(rec) => {
                        this.asking = None;
                        this.standing.retain(|r| r.id != rec.id);
                        this.standing.insert(0, rec.clone());
                        this.fresh = Some(rec.id.clone());
                        this.last = Some(rec.clone());
                        let id = rec.id;
                        this._tasks.push(cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(ACK).await;
                            let _ = this.update(cx, |this, cx| {
                                if this.fresh.as_deref() == Some(id.as_str()) {
                                    this.fresh = None;
                                    cx.notify();
                                }
                            });
                        }));
                    }
                    None => this.error = Some((target, t(cx, "feedback.saveFailed"))),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn undo(&mut self, rec: FeedbackRecord, cx: &mut Context<Self>) {
        self.busy = true;
        let target = format!("{}:{}", rec.target_kind, rec.target_id);
        let task = Desk::get(cx).update(cx, |d, cx| d.post("feedback-undo", json!({ "id": rec.id }), cx));
        self._tasks.push(cx.spawn(async move |this, cx| {
            let ok = task.await.is_ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if ok {
                    this.standing.retain(|r| r.id != rec.id);
                    if this.last.as_ref().is_some_and(|l| l.id == rec.id) {
                        this.last = None;
                    }
                } else {
                    this.error = Some((target, t(cx, "feedback.undoFailed")));
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn ask(&mut self, kind: &str, id: &str, value: &str, sample: Option<Value>, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t(cx, "feedback.reasonPlaceholder")));
        let (k, i, v, s) = (kind.to_string(), id.to_string(), value.to_string(), sample.clone());
        let sub = cx.subscribe(&input, move |this, input, ev: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let reason = input.read(cx).value().to_string();
                this.tap(&k, &i, &v, Some(reason), s.clone(), cx);
            }
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        self.asking = Some(Asking {
            kind: kind.to_string(),
            id: id.to_string(),
            value: value.to_string(),
            sample,
            input,
            _enter: sub,
        });
        cx.notify();
    }

    /// The Dock bounced because what needs the operator rose (the native
    /// layer calls this when it rings one).
    pub fn record_bounce(at_ms: u64, state: &str, cx: &mut App) {
        let store = FeedbackStore::get(cx);
        store.update(cx, |s, cx| {
            s.bounce = Some((at_ms, state.to_string()));
            cx.notify();
        });
    }
}

fn btn(id: SharedString, theme: &Theme, plain: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(4.))
        .text_size(px(13.))
        .text_color(theme.c("ink-2"))
        .when(!plain, |d| d.border_1().border_color(theme.c("line")).bg(theme.c("surface")))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("control")))
}

/// The slot (`FeedbackRow`): one row for one target, or nothing. `sample` is
/// what the target looked like (`{state, label}`) for a robot's state.
pub fn feedback_row(kind: &str, id: &str, sample: Option<Value>, cx: &mut App) -> Option<AnyElement> {
    let store = FeedbackStore::get(cx);
    let s = store.read(cx);
    if !feedback_allowed(s.operator_state.as_deref()) {
        return None;
    }
    let control = control(kind)?;
    let theme = cx.global::<Theme>().clone();
    let key = format!("{kind}:{id}");
    let error = s.error.as_ref().filter(|(k, _)| *k == key).map(|(_, e)| e.clone());
    let error_el = error.map(|e| div().text_size(px(13.)).text_color(theme.c("red-ink")).child(e));
    let row = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .font_family(SANS)
        .text_size(px(13.))
        .text_color(theme.c("ink-2"))
        .whitespace_nowrap();
    if let Some(rec) = s.standing_for(kind, id).cloned() {
        let ack = control
            .ack
            .iter()
            .find(|(v, _)| *v == rec.value)
            .map(|(_, k)| t(cx, k))
            .unwrap_or_else(|| t(cx, "feedback.ackDefault"));
        let is_last = s.last.as_ref().is_some_and(|l| l.id == rec.id);
        let busy = s.busy;
        let store = store.clone();
        return Some(
            row.child(div().px(px(4.)).text_color(theme.c("ink-3")).child(ack))
                .when(is_last, |d| {
                    d.child(
                        btn(SharedString::from(format!("hb-fb-undo-{key}")), &theme, true)
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.c("link"))
                            .child(t(cx, "feedback.undo"))
                            .when(!busy, |b| {
                                b.on_click(move |_, _, cx| store.update(cx, |s, cx| s.undo(rec.clone(), cx)))
                            }),
                    )
                })
                .children(error_el)
                .into_any_element(),
        );
    }
    if let Some(a) = s.asking.as_ref().filter(|a| a.kind == kind && a.id == id) {
        let label = control
            .options
            .iter()
            .find(|o| o.0 == a.value)
            .map(|o| t(cx, o.1))
            .unwrap_or_default();
        let input = a.input.clone();
        let (k, i, v, smp) = (a.kind.clone(), a.id.clone(), a.value.clone(), a.sample.clone());
        let store_send = store.clone();
        let store_esc = store.clone();
        return Some(
            row.child(div().px(px(4.)).font_weight(FontWeight::BOLD).child(format!("{label}:")))
                .child(
                    div()
                        .w(px(220.))
                        .h(px(28.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(theme.c("line-strong"))
                        .rounded(px(4.))
                        .bg(theme.c("surface"))
                        .text_color(theme.c("ink"))
                        .on_key_down(move |ev: &gpui::KeyDownEvent, _, cx| {
                            if ev.keystroke.key == "escape" {
                                cx.stop_propagation();
                                store_esc.update(cx, |s, cx| {
                                    s.asking = None;
                                    cx.notify();
                                });
                            }
                        })
                        .child(Input::new(&input).appearance(false)),
                )
                .child(
                    btn(SharedString::from(format!("hb-fb-send-{key}")), &theme, false)
                        .child(t(cx, "feedback.send"))
                        .on_click(move |_, _, cx| {
                            let reason = input.read(cx).value().to_string();
                            store_send.update(cx, |s, cx| s.tap(&k, &i, &v, Some(reason), smp.clone(), cx));
                        }),
                )
                .children(error_el)
                .into_any_element(),
        );
    }
    let busy = s.busy;
    let mut out = row.children(control.question.map(|q| div().px(px(4.)).font_weight(FontWeight::BOLD).child(t(cx, q))));
    for (value, label_key, thumb, wants_reason) in control.options {
        let store = store.clone();
        let (kind, id, sample) = (kind.to_string(), id.to_string(), sample.clone());
        let label = t(cx, label_key);
        let b = btn(SharedString::from(format!("hb-fb-{key}-{value}")), &theme, thumb.is_some());
        let b = match thumb {
            Some(name) => b.child(icon(name, 16., theme.c("ink-2"))),
            None => b.child(label),
        };
        out = out.child(b.when(!busy, |b| {
            b.on_click(move |_, window, cx| {
                store.update(cx, |s, cx| {
                    if *wants_reason {
                        s.ask(&kind, &id, value, sample.clone(), window, cx);
                    } else {
                        s.tap(&kind, &id, value, None, sample.clone(), cx);
                    }
                })
            })
        }));
    }
    Some(out.children(error_el).into_any_element())
}

/// "The Dock bounced at 14:02" with "This bothered me", for half an hour
/// after the bounce (`DockFeedback`).
pub fn dock_feedback(now: u64, cx: &mut App) -> Option<AnyElement> {
    let store = FeedbackStore::get(cx);
    let (at, state) = store.read(cx).bounce.clone()?;
    if now.saturating_sub(at) > BOUNCE_OFFER_MS {
        return None;
    }
    let theme = cx.global::<Theme>().clone();
    let what = tv(cx, "feedback.dockBouncedAt", &[("time", &crate::ui::format::hhmm(at))]);
    let row = feedback_row("dock", &bounce_target(at), Some(json!({ "state": state })), cx)?;
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mx(px(16.))
            .mb(px(8.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(theme.c("surface-2"))
            .text_size(px(13.))
            .text_color(theme.c("ink-3"))
            .child(what)
            .child(row)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_asks_once_by_kind() {
        assert_eq!(message_target("operator", None), None);
        assert_eq!(message_target("agent", Some("DONE")), Some("delivery"));
        assert_eq!(message_target("agent", Some("FINDING")), Some("message"));
        assert_eq!(message_target("lead", None), Some("message"));
    }

    #[test]
    fn nothing_asks_while_focused_swamped_or_away() {
        assert!(feedback_allowed(None));
        assert!(feedback_allowed(Some("available")));
        assert!(!feedback_allowed(Some("focused")));
        assert!(!feedback_allowed(Some("away")));
    }

    #[test]
    fn every_kind_has_its_controls() {
        for k in ["message", "delivery", "needs_you", "robot_state", "dock"] {
            assert!(control(k).is_some());
        }
        assert!(control("nope").is_none());
    }
}
