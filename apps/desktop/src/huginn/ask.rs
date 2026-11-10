//! The components that write: controls (view-local state) and answers
//! (Choice, Form and its fields), which send an operator reply note.

use super::answer::{self, Answer, AnswerLine, NO, UNCHANGED, YES, options_answer};
use super::build::{
    AskUi, B, Children, FieldReg, FieldValue, FormFrame, Sent, arr, eid, flag, n, s, txt,
};
use super::expr;
use super::{Given, HuginnView};
use crate::api::ApiError;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, anchored, canvas, deferred, div, point, px, relative,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use serde_json::{Value, json};

/// The value a field writes to state.
fn for_state(kind: &str, value: &FieldValue) -> Value {
    match value {
        FieldValue::Bool(b) => json!(b),
        FieldValue::List(l) => json!(l),
        FieldValue::Text(t) if kind == "NumberField" || kind == "SliderField" => {
            if t.trim().is_empty() {
                Value::Null
            } else {
                t.trim()
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map_or(Value::Null, Value::Number)
            }
        }
        FieldValue::Text(t) => json!(t),
    }
}

/// What the note says for a field: never the text of an option.
fn to_answer(field: &FieldReg) -> String {
    match field.kind.as_str() {
        "TextField" => match (&field.default_text, &field.value) {
            (Some(d), FieldValue::Text(v)) if v == d => UNCHANGED.to_string(),
            (_, v) => v.text().to_string(),
        },
        "SelectField" | "RadioField" => {
            match field.options.iter().position(|o| o == field.value.text()) {
                Some(i) => options_answer(&[i]),
                None => String::new(),
            }
        }
        "CheckboxField" => if field.value == FieldValue::Bool(true) {
            YES
        } else {
            NO
        }
        .to_string(),
        "MultiSelectField" => {
            let picked = match &field.value {
                FieldValue::List(l) => l.clone(),
                _ => Vec::new(),
            };
            let indexes: Vec<usize> = field
                .options
                .iter()
                .enumerate()
                .filter(|(_, o)| picked.contains(o))
                .map(|(i, _)| i)
                .collect();
            if indexes.is_empty() {
                String::new()
            } else {
                options_answer(&indexes)
            }
        }
        _ => field.value.text().to_string(),
    }
}

/// A refused reply, in the operator's language (`refusalText`).
fn refusal_text(error: &ApiError, cx: &App) -> String {
    let message = &error.message;
    let codes: Vec<&str> = message
        .match_indices("U+")
        .filter_map(|(at, _)| {
            let hex: &str = &message[at + 2..];
            let len = hex
                .chars()
                .take_while(|c| c.is_ascii_digit() || ('A'..='F').contains(c))
                .count();
            (4..=6).contains(&len).then(|| &message[at..at + 2 + len])
        })
        .collect();
    if error.status.is_some() && message.contains("characters you cannot see") {
        let list = if codes.is_empty() {
            String::new()
        } else {
            format!(" ({})", codes.join(", "))
        };
        return tv(cx, "messages.invisibleRefusal", &[("list", &list)]).to_string();
    }
    if error.status.is_some() && !codes.is_empty() {
        return message.clone();
    }
    let reason = error.body.get("reason").and_then(Value::as_str);
    match error.status {
        Some(409) if reason == Some("no_reachable_member") => {
            t(cx, "workspace.replyNoChannelReader").to_string()
        }
        Some(409) => t(cx, "workspace.noDelivery").to_string(),
        Some(422) => t(cx, "workspace.replyTooLong").to_string(),
        _ => t(cx, "workspace.replySaveFailed").to_string(),
    }
}

impl HuginnView {
    /// Send one answer, and remember it locally until the bus shows the note.
    pub(super) fn send_answer(
        &mut self,
        id: String,
        lines: Vec<AnswerLine>,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.target() else { return };
        let answer = Answer {
            file: self.attachment.name.clone(),
            id: id.clone(),
            lines,
        };
        let text = answer::compose(&answer);
        if answer::js_len(&text) > answer::SEND_LIMIT_CHARS {
            self.ui.asks.entry(id).or_default().sent =
                Sent::Failed(t(cx, "huginn.answerTooLong").to_string());
            cx.notify();
            return;
        }
        self.ui.asks.entry(id.clone()).or_default().sent = Sent::Sending;
        cx.notify();
        let body = json!({ "agent": target.agent, "reply_to": target.reply_to, "text": text });
        let task = Desk::get(cx).update(cx, |desk, cx| desk.post("reply", body, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |view, cx| {
                let described = match view.spec() {
                    Some(spec) => answer::describe(&spec, &answer),
                    None => answer::describe_raw(&answer),
                };
                let now = Desk::get(cx).read(cx).now_ms();
                let ask = view.ui.asks.entry(id).or_default();
                match result {
                    Ok(_) => {
                        ask.local = Some(Given {
                            lines: described,
                            ts_ms: Some(now),
                        });
                        ask.sent = Sent::Idle;
                    }
                    Err(e) => ask.sent = Sent::Failed(refusal_text(&e, cx)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn set_state(&mut self, path: &str, value: Value, cx: &mut Context<Self>) {
        expr::set(&mut self.state, path, value);
        cx.notify();
    }

    pub(super) fn field_changed(
        &mut self,
        form: &str,
        field: &str,
        value: FieldValue,
        state: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let mut kind = String::new();
        if let Some(f) = self
            .ui
            .asks
            .get_mut(form)
            .and_then(|a| a.fields.get_mut(field))
        {
            kind = f.kind.clone();
            f.value = value.clone();
        }
        if let Some(path) = state {
            expr::set(&mut self.state, path, for_state(&kind, &value));
        }
        cx.notify();
    }
}

/// The checkbox square (shadcn's `Checkbox`).
fn checkbox(b: &B<'_, '_>, on: bool) -> gpui::Div {
    let pal = b.pal;
    div()
        .mt(px(3.))
        .size(px(16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if on {
            pal.c("accent")
        } else {
            pal.c("line-hover")
        })
        .bg(if on {
            pal.c("accent")
        } else {
            pal.c("surface")
        })
        .when(on, |d| d.child(ui::icon("check", 12., pal.c("accent-ink"))))
}

impl<'a, 'w> B<'a, 'w> {
    fn target_author(&self) -> Option<String> {
        self.target.as_ref().map(|t| t.author.clone())
    }

    /// The answer already given (on the bus, or sent from here), its send
    /// status, and whether this message can be answered at all.
    fn ask_state(&self, id: &str) -> (Option<Given>, Sent, bool) {
        let ask = self.ui.asks.get(id);
        let given = self
            .given
            .get(id)
            .cloned()
            .or_else(|| ask.and_then(|a| a.local.clone()));
        (
            given,
            ask.map(|a| a.sent.clone()).unwrap_or_default(),
            self.target.is_some(),
        )
    }

    fn answered(&self, title: &str, given: &Given) -> AnyElement {
        let pal = self.pal;
        let when = match given.ts_ms {
            Some(ms) => self.trv("answeredAt", &[("time", &ui::format::hhmm(ms))]),
            None => self.tr("answered"),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .child(ui::icon("check", 16., pal.c("green-ink")))
                    .child(div().font_weight(FontWeight::BOLD).child(title.to_string()))
                    .child(txt(when, 13., FontWeight::NORMAL, pal.c("ink-3"))),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(given.lines.iter().map(|line| {
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(div().text_color(pal.c("ink-3")).child(line.label.clone()))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(line.value.clone()),
                            )
                    })),
            )
            .into_any_element()
    }

    fn footer(&self, sent: &Sent, can_send: bool) -> Option<AnyElement> {
        let pal = self.pal;
        if let Sent::Failed(why) = sent {
            return Some(
                txt(why.clone(), 14., FontWeight::NORMAL, pal.c("red-ink")).into_any_element(),
            );
        }
        if !can_send {
            return Some(
                txt(
                    self.tr("noReplyHere"),
                    13.,
                    FontWeight::NORMAL,
                    pal.c("ink-3"),
                )
                .into_any_element(),
            );
        }
        let author = self.target_author()?;
        Some(
            txt(
                self.trv("answerReaches", &[("author", &author)]),
                13.,
                FontWeight::NORMAL,
                pal.c("ink-3"),
            )
            .into_any_element(),
        )
    }

    /// `.hb-btn.hb-btn-primary`.
    fn primary(
        &mut self,
        id: String,
        label: SharedString,
        enabled: bool,
        on: impl Fn(&mut HuginnView, &mut Context<HuginnView>) + 'static,
    ) -> AnyElement {
        let pal = self.pal;
        div()
            .flex()
            .child(
                div()
                    .id(eid(id))
                    .h(px(36.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(pal.c("accent-deep"))
                    .bg(pal.c("accent-deep"))
                    .text_color(pal.c("accent-ink"))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(15.))
                    .child(label)
                    .when(!enabled, |d| d.opacity(0.5))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(pal.c("accent-hover")))
                            .on_click(self.cx.listener(move |this, _, _, cx| on(this, cx)))
                    }),
            )
            .into_any_element()
    }

    // -------------------------------------------------------------- controls

    pub(super) fn segmented(&mut self, p: &Value, id: &str) -> AnyElement {
        let pal = self.pal;
        let path = s(p, "state").unwrap_or_default().to_string();
        let current = expr::get(self.state, &path).cloned();
        let buttons: Vec<AnyElement> =
            arr(p, "options")
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let value = s(o, "value").unwrap_or_default().to_string();
                    let on = current.as_ref().and_then(Value::as_str) == Some(value.as_str());
                    let path = path.clone();
                    div()
                        .id(eid(format!("{id}-seg-{i}")))
                        .px(px(12.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .cursor_pointer()
                        .text_color(if on { pal.c("ink") } else { pal.c("ink-2") })
                        .when(on, |d| {
                            d.bg(pal.c("surface")).shadow(vec![BoxShadow {
                                color: pal.c("panel-ring"),
                                offset: point(px(0.), px(1.)),
                                blur_radius: px(2.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                        })
                        .child(s(o, "label").unwrap_or_default().to_string())
                        .on_click(self.cx.listener(move |this, _, _, cx| {
                            this.set_state(&path, json!(value), cx)
                        }))
                        .into_any_element()
                })
                .collect();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .text_size(px(14.))
            .when_some(s(p, "label"), |d, l| {
                d.child(div().text_color(pal.c("ink-2")).child(l.to_string()))
            })
            .child(
                div()
                    .flex()
                    .p(px(2.))
                    .rounded(px(8.))
                    .bg(pal.c("chip"))
                    .children(buttons),
            )
            .into_any_element()
    }

    pub(super) fn switch(&mut self, p: &Value, id: &str) -> AnyElement {
        let pal = self.pal;
        let path = s(p, "state").unwrap_or_default().to_string();
        let on = expr::get(self.state, &path) == Some(&Value::Bool(true));
        div()
            .flex()
            .child(
                div()
                    .id(eid(format!("{id}-switch")))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .w(px(32.))
                            .h(px(18.))
                            .p(px(2.))
                            .rounded(px(9.))
                            .bg(if on {
                                pal.c("accent")
                            } else {
                                pal.c("line-hover")
                            })
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded_full()
                                    .bg(pal.c("surface"))
                                    .when(on, |d| d.ml(px(14.))),
                            ),
                    )
                    .child(
                        div()
                            .text_color(pal.c("ink-2"))
                            .child(s(p, "label").unwrap_or_default().to_string()),
                    )
                    .on_click(
                        self.cx
                            .listener(move |this, _, _, cx| this.set_state(&path, json!(!on), cx)),
                    ),
            )
            .into_any_element()
    }

    // --------------------------------------------------------------- answers

    pub(super) fn choice(&mut self, p: &Value, _id: &str) -> AnyElement {
        let pal = self.pal;
        let cid = s(p, "id").unwrap_or_default().to_string();
        let question = s(p, "question").unwrap_or_default().to_string();
        let (given, sent, can_send) = self.ask_state(&cid);
        if let Some(given) = given {
            return self.answered(&question, &given);
        }
        let busy = matches!(sent, Sent::Sending) || !can_send;
        let picked = self
            .ui
            .asks
            .get(&cid)
            .map(|a| a.picked.clone())
            .unwrap_or_default();
        let options = arr(p, "options");
        let body: AnyElement = if flag(p, "multiple") {
            let rows: Vec<AnyElement> = options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let on = picked.contains(&i);
                    let key = cid.clone();
                    div()
                        .id(eid(format!("{cid}-opt-{i}")))
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .when(!busy, |d| d.cursor_pointer())
                        .when(busy, |d| d.opacity(0.5))
                        .child(checkbox(self, on))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(s(o, "label").unwrap_or_default().to_string())
                                .when_some(s(o, "detail"), |d, x| {
                                    d.child(txt(
                                        x.to_string(),
                                        13.,
                                        FontWeight::NORMAL,
                                        pal.c("ink-3"),
                                    ))
                                }),
                        )
                        .when(!busy, |d| {
                            d.on_click(self.cx.listener(move |this, _, _, cx| {
                                let ask = this.ui.asks.entry(key.clone()).or_default();
                                if let Some(at) = ask.picked.iter().position(|x| *x == i) {
                                    ask.picked.remove(at);
                                } else {
                                    ask.picked.push(i);
                                }
                                cx.notify();
                            }))
                        })
                        .into_any_element()
                })
                .collect();
            let key = cid.clone();
            let send = self.primary(
                format!("{cid}-send"),
                self.tr("send"),
                !busy && !picked.is_empty(),
                move |this, cx| {
                    let picked = this
                        .ui
                        .asks
                        .get(&key)
                        .map(|a| a.picked.clone())
                        .unwrap_or_default();
                    let line = AnswerLine {
                        field: key.clone(),
                        value: options_answer(&picked),
                    };
                    this.send_answer(key.clone(), vec![line], cx);
                },
            );
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(div().flex().flex_col().gap(px(8.)).children(rows))
                .child(send)
                .into_any_element()
        } else {
            let buttons: Vec<AnyElement> = options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let tone = s(o, "tone").unwrap_or("neutral");
                    let (border, bg, ink, small, hover) = match tone {
                        "primary" => (
                            pal.c("accent"),
                            pal.c("accent"),
                            pal.c("accent-ink"),
                            pal.c("accent-ink-2"),
                            pal.c("accent-hover"),
                        ),
                        "danger" => (
                            pal.c("red-edge"),
                            pal.c("surface"),
                            pal.c("red-ink"),
                            pal.c("ink-3"),
                            pal.c("btn-hover"),
                        ),
                        _ => (
                            pal.c("line-strong"),
                            pal.c("surface"),
                            pal.c("ink"),
                            pal.c("ink-3"),
                            pal.c("btn-hover"),
                        ),
                    };
                    let key = cid.clone();
                    div()
                        .id(eid(format!("{cid}-pick-{i}")))
                        .flex()
                        .flex_col()
                        .items_start()
                        .px(px(14.))
                        .py(px(7.))
                        .border_1()
                        .border_color(border)
                        .rounded(px(8.))
                        .bg(bg)
                        .text_color(ink)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(s(o, "label").unwrap_or_default().to_string())
                        .when_some(s(o, "detail"), |d, x| {
                            d.child(txt(x.to_string(), 13., FontWeight::NORMAL, small))
                        })
                        .when(busy, |d| d.opacity(0.55))
                        .when(!busy, |d| {
                            d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(
                                self.cx.listener(move |this, _, _, cx| {
                                    let line = AnswerLine {
                                        field: key.clone(),
                                        value: options_answer(&[i]),
                                    };
                                    this.send_answer(key.clone(), vec![line], cx);
                                }),
                            )
                        })
                        .into_any_element()
                })
                .collect();
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .children(buttons)
                .into_any_element()
        };
        let footer = self.footer(&sent, can_send);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().font_weight(FontWeight::BOLD).child(question))
            .when_some(s(p, "detail"), |d, x| {
                d.child(div().text_color(pal.c("ink-2")).child(x.to_string()))
            })
            .child(body)
            .children(footer)
            .into_any_element()
    }

    pub(super) fn form_el(&mut self, p: &Value, _id: &str, kids: &Children<'a>) -> AnyElement {
        let pal = self.pal;
        let fid = s(p, "id").unwrap_or_default().to_string();
        let title = s(p, "title").unwrap_or_default().to_string();
        let (given, sent, can_send) = self.ask_state(&fid);
        if let Some(given) = given {
            return self.answered(&title, &given);
        }
        let disabled = matches!(sent, Sent::Sending) || !can_send;
        self.ui.asks.entry(fid.clone()).or_default().order.clear();
        let outer = self.form.replace(FormFrame {
            id: fid.clone(),
            disabled,
        });
        let children = self.children(kids);
        self.form = outer;
        let ask: &AskUi = self.ui.asks.entry(fid.clone()).or_default();
        let ready = !ask.order.is_empty()
            && ask.order.iter().all(|f| {
                ask.fields
                    .get(f)
                    .is_none_or(|field| !field.required || field.value.filled())
            });
        let label = s(p, "submitLabel")
            .map(|l| SharedString::from(l.to_string()))
            .unwrap_or_else(|| self.tr("send"));
        let key = fid.clone();
        let submit = self.primary(
            format!("{fid}-submit"),
            label,
            !disabled && ready,
            move |this, cx| {
                let Some(ask) = this.ui.asks.get(&key) else {
                    return;
                };
                let lines: Vec<AnswerLine> = ask
                    .order
                    .iter()
                    .filter_map(|f| {
                        ask.fields.get(f).map(|field| AnswerLine {
                            field: f.clone(),
                            value: to_answer(field),
                        })
                    })
                    .collect();
                this.send_answer(key.clone(), lines, cx);
            },
        );
        let footer = self.footer(&sent, can_send);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().font_weight(FontWeight::BOLD).child(title))
            .when_some(s(p, "detail"), |d, x| {
                d.child(div().text_color(pal.c("ink-2")).child(x.to_string()))
            })
            .children(children)
            .child(submit)
            .children(footer)
            .into_any_element()
    }

    /// Register a field with its Form on mount; its current value, and
    /// whether the form is disabled. `fresh` is true on (re)mount.
    fn field(
        &mut self,
        p: &Value,
        kind: &str,
        initial: FieldValue,
        options: Vec<String>,
        default_text: Option<String>,
    ) -> Option<(String, String, FieldValue, bool, bool)> {
        let frame = self.form.clone()?;
        let fid = s(p, "id").unwrap_or_default().to_string();
        self.seen.insert(format!("{}/{fid}", frame.id));
        let ask = self.ui.asks.entry(frame.id.clone()).or_default();
        ask.order.push(fid.clone());
        let mut fresh = false;
        if !ask.fields.contains_key(&fid) {
            ask.fields.insert(
                fid.clone(),
                FieldReg {
                    kind: kind.to_string(),
                    value: initial.clone(),
                    required: flag(p, "required"),
                    options,
                    default_text,
                },
            );
            if let Some(path) = s(p, "state")
                && initial != FieldValue::Text(String::new())
            {
                self.publish
                    .push((path.to_string(), for_state(kind, &initial)));
            }
            fresh = true;
        }
        let value = ask
            .fields
            .get(&fid)
            .map(|f| f.value.clone())
            .unwrap_or(initial);
        Some((frame.id, fid, value, frame.disabled, fresh))
    }

    fn field_label(&self, p: &Value) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .items_baseline()
            .gap_x(px(4.))
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(s(p, "label").unwrap_or_default().to_string())
            .when(flag(p, "required"), |d| {
                d.child(txt(
                    self.tr("required"),
                    13.,
                    FontWeight::NORMAL,
                    self.pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    fn field_shell(&self, p: &Value, hint: Option<String>, control: AnyElement) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .min_w_0()
            .child(self.field_label(p))
            .child(control)
            .when_some(hint, |d, h| {
                d.child(txt(h, 13., FontWeight::NORMAL, self.pal.c("ink-3")))
            })
            .into_any_element()
    }

    /// The bordered box inputs sit in.
    fn input_box(&self, disabled: bool) -> gpui::Div {
        div()
            .w_full()
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(self.pal.c("line-strong"))
            .bg(self.pal.c("surface"))
            .text_size(px(14.))
            .text_color(self.pal.c("ink"))
            .when(disabled, |d| d.opacity(0.5))
    }

    pub(super) fn text_field(&mut self, kind: &str, p: &Value, _id: &str) -> AnyElement {
        let initial = match (kind, p.get("defaultValue")) {
            (_, None) => String::new(),
            (_, Some(v)) => expr::js_string(v),
        };
        let default_text = (kind == "TextField")
            .then(|| s(p, "defaultValue").map(str::to_string))
            .flatten();
        let Some((form, fid, value, disabled, fresh)) = self.field(
            p,
            kind,
            FieldValue::Text(initial.clone()),
            Vec::new(),
            default_text,
        ) else {
            return div().into_any_element();
        };
        let key = format!("{form}/{fid}");
        let state_path = s(p, "state").map(str::to_string);
        let placeholder = s(p, "placeholder").map(str::to_string).unwrap_or_else(|| {
            if kind == "DateField" {
                "YYYY-MM-DD".into()
            } else {
                String::new()
            }
        });
        let multiline = kind == "TextField" && flag(p, "multiline");
        let control: AnyElement = if multiline {
            let existed = self.ui.areas.contains_key(&key);
            let entity = match self.ui.areas.get(&key) {
                Some(e) => e.clone(),
                None => {
                    let window = &mut *self.window;
                    let e = self.cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .rows(2)
                            .placeholder(placeholder.clone())
                            .default_value(initial.clone())
                    });
                    let (f, i, sp) = (form.clone(), fid.clone(), state_path.clone());
                    self.subs.push(self.cx.subscribe_in(
                        &e,
                        self.window,
                        move |this, state, ev: &InputEvent, window, cx| {
                            if matches!(ev, InputEvent::Change) {
                                let mut v = state.read(cx).value().to_string();
                                if v.chars().count() > 1000 {
                                    v = v.chars().take(1000).collect();
                                    state.update(cx, |s, cx| s.set_value(v.clone(), window, cx));
                                }
                                this.field_changed(&f, &i, FieldValue::Text(v), sp.as_deref(), cx);
                            }
                        },
                    ));
                    self.ui.areas.insert(key.clone(), e.clone());
                    e
                }
            };
            if fresh && existed {
                let window = &mut *self.window;
                entity.update(self.cx, |s, cx| s.set_value(initial.clone(), window, cx));
            }
            entity.update(self.cx, |s, cx| s.set_disabled(disabled, cx));
            let _ = &value;
            self.input_box(disabled)
                .px_0()
                .py(px(4.))
                .min_h(px(64.))
                .flex()
                .flex_col()
                .child(Textarea::new(&entity).appearance(false).w_full())
                .into_any_element()
        } else {
            let existed = self.ui.inputs.contains_key(&key);
            let entity = match self.ui.inputs.get(&key) {
                Some(e) => e.clone(),
                None => {
                    let window = &mut *self.window;
                    let kind_owned = kind.to_string();
                    let e = self.cx.new(|cx| {
                        let state = InputState::new(window, cx)
                            .placeholder(placeholder.clone())
                            .default_value(initial.clone());
                        match kind_owned.as_str() {
                            "NumberField" => state.validate(|s, _| numeric_typing(s)),
                            "DateField" => state.validate(|s, _| date_typing(s)),
                            _ => state.validate(|s, _| s.chars().count() <= 1000),
                        }
                    });
                    let (f, i, sp) = (form.clone(), fid.clone(), state_path.clone());
                    self.subs.push(self.cx.subscribe_in(
                        &e,
                        self.window,
                        move |this, state, ev: &InputEvent, _window, cx| {
                            if matches!(ev, InputEvent::Change) {
                                let v = state.read(cx).value().to_string();
                                this.field_changed(&f, &i, FieldValue::Text(v), sp.as_deref(), cx);
                            }
                        },
                    ));
                    self.ui.inputs.insert(key.clone(), e.clone());
                    e
                }
            };
            if fresh && existed {
                let window = &mut *self.window;
                entity.update(self.cx, |s, cx| s.set_value(initial.clone(), window, cx));
            }
            entity.update(self.cx, |s, cx| s.set_disabled(disabled, cx));
            self.input_box(disabled)
                .px_0()
                .h(px(32.))
                .flex()
                .flex_col()
                .justify_center()
                .child(Input::new(&entity).appearance(false).w_full())
                .into_any_element()
        };
        let hint = s(p, "hint").map(str::to_string);
        let hint = match (kind, s(p, "unit")) {
            ("NumberField", Some(unit)) => Some(match hint {
                Some(h) => format!("{h} ({unit})"),
                None => unit.to_string(),
            }),
            _ => hint,
        };
        self.field_shell(p, hint, control)
    }

    pub(super) fn select_field(&mut self, p: &Value, _id: &str) -> AnyElement {
        let pal = self.pal;
        let options: Vec<String> = arr(p, "options")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let initial = s(p, "defaultValue").unwrap_or_default().to_string();
        let Some((form, fid, value, disabled, _)) = self.field(
            p,
            "SelectField",
            FieldValue::Text(initial),
            options.clone(),
            None,
        ) else {
            return div().into_any_element();
        };
        let key = format!("{form}/{fid}");
        let open = self.ui.select_open.as_deref() == Some(key.as_str());
        let (width, probe) = self.width_probe(&format!("{key}#select"), 240.);
        let trigger_bounds = self.bounds_cell(&format!("{key}#trigger"));
        let at = trigger_bounds.get();
        let track = canvas(move |b, _, _| trigger_bounds.set(b), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full();
        let current = value.text().to_string();
        let shown = if current.is_empty() {
            self.tr("chooseOption")
        } else {
            current.clone().into()
        };
        let toggle_key = key.clone();
        let trigger = self
            .input_box(disabled)
            .relative()
            .h(px(32.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.))
            .child(probe)
            .child(track)
            .child(
                div()
                    .text_color(if current.is_empty() {
                        pal.c("ink-4")
                    } else {
                        pal.c("ink")
                    })
                    .truncate()
                    .child(shown),
            )
            .child(ui::icon("chevron-down", 16., pal.c("ink-3")));
        let trigger = div()
            .id(eid(format!("{key}-trigger")))
            .when(!disabled, |d| d.cursor_pointer())
            .child(trigger)
            .when(!disabled, |d| {
                d.on_click(self.cx.listener(move |this, _, _, cx| {
                    this.ui.select_open =
                        if this.ui.select_open.as_deref() == Some(toggle_key.as_str()) {
                            None
                        } else {
                            Some(toggle_key.clone())
                        };
                    cx.notify();
                }))
            });
        let state_path = s(p, "state").map(str::to_string);
        let menu = open.then(|| {
            let items: Vec<AnyElement> = options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let (f, fi, sp, v) = (form.clone(), fid.clone(), state_path.clone(), o.clone());
                    let on = *o == current;
                    div()
                        .id(eid(format!("{key}-item-{i}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(6.))
                        .px(px(6.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .text_size(px(14.))
                        .cursor_pointer()
                        .hover(|s| s.bg(pal.c("btn-hover")))
                        .child(o.clone())
                        .when(on, |d| d.child(ui::icon("check", 14., pal.c("ink-2"))))
                        .on_click(self.cx.listener(move |this, _, _, cx| {
                            this.ui.select_open = None;
                            this.field_changed(
                                &f,
                                &fi,
                                FieldValue::Text(v.clone()),
                                sp.as_deref(),
                                cx,
                            );
                        }))
                        .into_any_element()
                })
                .collect();
            let close = key.clone();
            deferred(
                anchored()
                    .position(at.bottom_left())
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .id(eid(format!("{key}-menu")))
                            .mt(px(4.))
                            .w(px(width.max(144.)))
                            .max_h(px(280.))
                            .overflow_y_scroll()
                            .p(px(4.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(pal.c("line"))
                            .bg(pal.c("surface-raised"))
                            .text_color(pal.c("ink"))
                            .shadow(vec![BoxShadow {
                                color: pal.c("panel-ring"),
                                offset: point(px(0.), px(4.)),
                                blur_radius: px(12.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .children(items)
                            .on_mouse_down_out(self.cx.listener(move |this, _, _, cx| {
                                if this.ui.select_open.as_deref() == Some(close.as_str()) {
                                    this.ui.select_open = None;
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .with_priority(1)
        });
        let control = div()
            .flex()
            .flex_col()
            .child(trigger)
            .children(menu)
            .into_any_element();
        self.field_shell(p, s(p, "hint").map(str::to_string), control)
    }

    pub(super) fn radio_field(&mut self, p: &Value, _id: &str) -> AnyElement {
        let pal = self.pal;
        let options: Vec<String> = arr(p, "options")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let initial = s(p, "defaultValue").unwrap_or_default().to_string();
        let Some((form, fid, value, disabled, _)) = self.field(
            p,
            "RadioField",
            FieldValue::Text(initial),
            options.clone(),
            None,
        ) else {
            return div().into_any_element();
        };
        let state_path = s(p, "state").map(str::to_string);
        let rows: Vec<AnyElement> = options
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let on = value.text() == o;
                let (f, fi, sp, v) = (form.clone(), fid.clone(), state_path.clone(), o.clone());
                div()
                    .id(eid(format!("{form}/{fid}-radio-{i}")))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .when(disabled, |d| d.opacity(0.5))
                    .child(
                        div()
                            .mt(px(3.))
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(if on {
                                pal.c("accent")
                            } else {
                                pal.c("line-hover")
                            })
                            .when(on, |d| {
                                d.child(div().size(px(8.)).rounded_full().bg(pal.c("accent")))
                            }),
                    )
                    .child(o.clone())
                    .when(!disabled, |d| {
                        d.cursor_pointer()
                            .on_click(self.cx.listener(move |this, _, _, cx| {
                                this.field_changed(
                                    &f,
                                    &fi,
                                    FieldValue::Text(v.clone()),
                                    sp.as_deref(),
                                    cx,
                                );
                            }))
                    })
                    .into_any_element()
            })
            .collect();
        let control = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(rows)
            .into_any_element();
        self.field_shell(p, s(p, "hint").map(str::to_string), control)
    }

    pub(super) fn checkbox_field(&mut self, p: &Value, _id: &str) -> AnyElement {
        let initial = p
            .get("defaultChecked")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some((form, fid, value, disabled, _)) = self.field(
            p,
            "CheckboxField",
            FieldValue::Bool(initial),
            Vec::new(),
            None,
        ) else {
            return div().into_any_element();
        };
        let on = value == FieldValue::Bool(true);
        let state_path = s(p, "state").map(str::to_string);
        let row = div()
            .id(eid(format!("{form}/{fid}-check")))
            .flex()
            .items_start()
            .gap(px(8.))
            .when(disabled, |d| d.opacity(0.5))
            .child(checkbox(self, on))
            .child(s(p, "label").unwrap_or_default().to_string())
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .on_click(self.cx.listener(move |this, _, _, cx| {
                        this.field_changed(
                            &form,
                            &fid,
                            FieldValue::Bool(!on),
                            state_path.as_deref(),
                            cx,
                        );
                    }))
            });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(row)
            .when_some(s(p, "hint"), |d, h| {
                d.child(txt(
                    h.to_string(),
                    13.,
                    FontWeight::NORMAL,
                    self.pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    pub(super) fn multi_field(&mut self, p: &Value, _id: &str) -> AnyElement {
        let options: Vec<String> = arr(p, "options")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let initial: Vec<String> = arr(p, "defaultValue")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let Some((form, fid, value, disabled, _)) = self.field(
            p,
            "MultiSelectField",
            FieldValue::List(initial),
            options.clone(),
            None,
        ) else {
            return div().into_any_element();
        };
        let picked = match value {
            FieldValue::List(l) => l,
            _ => Vec::new(),
        };
        let state_path = s(p, "state").map(str::to_string);
        let rows: Vec<AnyElement> = options
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let on = picked.contains(o);
                let next: Vec<String> = if on {
                    picked.iter().filter(|x| *x != o).cloned().collect()
                } else {
                    options
                        .iter()
                        .filter(|x| *x == o || picked.contains(x))
                        .cloned()
                        .collect()
                };
                let (f, fi, sp) = (form.clone(), fid.clone(), state_path.clone());
                div()
                    .id(eid(format!("{form}/{fid}-multi-{i}")))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .when(disabled, |d| d.opacity(0.5))
                    .child(checkbox(self, on))
                    .child(o.clone())
                    .when(!disabled, |d| {
                        d.cursor_pointer()
                            .on_click(self.cx.listener(move |this, _, _, cx| {
                                this.field_changed(
                                    &f,
                                    &fi,
                                    FieldValue::List(next.clone()),
                                    sp.as_deref(),
                                    cx,
                                );
                            }))
                    })
                    .into_any_element()
            })
            .collect();
        let control = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(rows)
            .into_any_element();
        self.field_shell(p, s(p, "hint").map(str::to_string), control)
    }

    pub(super) fn slider_field(&mut self, p: &Value, _id: &str) -> AnyElement {
        let min = n(p, "min").unwrap_or(0.0);
        let max = n(p, "max").unwrap_or(100.0);
        let step = n(p, "step").filter(|s| *s > 0.0).unwrap_or(1.0);
        let initial = expr::js_string(&json!(n(p, "defaultValue").unwrap_or(min)));
        let Some((form, fid, value, disabled, _)) = self.field(
            p,
            "SliderField",
            FieldValue::Text(initial),
            Vec::new(),
            None,
        ) else {
            return div().into_any_element();
        };
        let current: f64 = value.text().parse().unwrap_or(f64::NAN);
        let key = format!("{form}/{fid}");
        let output = self.num(Some(current), s(p, "unit"));
        let state_path = s(p, "state").map(str::to_string);
        let track = self.slider_track(
            &key,
            current,
            min,
            max,
            step,
            disabled,
            move |this, v, cx| {
                this.field_changed(
                    &form,
                    &fid,
                    FieldValue::Text(expr::js_string(&json!(v))),
                    state_path.as_deref(),
                    cx,
                );
            },
        );
        let control = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(div().flex_1().child(track))
            .child(
                div()
                    .min_w(px(56.))
                    .flex()
                    .justify_end()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(output),
            )
            .into_any_element();
        self.field_shell(p, s(p, "hint").map(str::to_string), control)
    }

    /// A horizontal slider: 4px track, accent fill, 16px thumb; click or drag.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn slider_track(
        &mut self,
        key: &str,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        disabled: bool,
        on: impl Fn(&mut HuginnView, f64, &mut Context<HuginnView>) + Clone + 'static,
    ) -> AnyElement {
        let pal = self.pal;
        let span = if max > min { max - min } else { 1.0 };
        let ratio = if value.is_finite() {
            ((value - min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        } as f32;
        let bounds = self.bounds_cell(&format!("{key}#slider"));
        let snap = move |x: f32, b: gpui::Bounds<gpui::Pixels>| -> f64 {
            let w = f32::from(b.size.width).max(1.0);
            let r = ((x - f32::from(b.origin.x)) / w).clamp(0.0, 1.0) as f64;
            let raw = min + r * span;
            let steps = ((raw - min) / step).round();
            let v = (min + steps * step).clamp(min.min(max), max.max(min));
            let decimals = format!("{step}").split('.').nth(1).map_or(0, str::len) as i32;
            let f = 10f64.powi(decimals);
            (v * f).round() / f
        };
        let dragging = self.ui.drag.as_deref() == Some(key);
        let weak = self.cx.entity().downgrade();
        let probe_bounds = bounds.clone();
        let paint_bounds = bounds.clone();
        let drag_key = key.to_string();
        let on_drag = on.clone();
        let probe = canvas(
            move |b, _, _| paint_bounds.set(b),
            move |_, _, window, _| {
                if !dragging {
                    return;
                }
                let (weak2, b2, on2) = (weak.clone(), probe_bounds.clone(), on_drag.clone());
                window.on_mouse_event(move |ev: &MouseMoveEvent, _, _, cx| {
                    let v = snap(f32::from(ev.position.x), b2.get());
                    let on2 = on2.clone();
                    weak2.update(cx, |this, cx| on2(this, v, cx)).ok();
                });
                let weak3 = weak.clone();
                let k = drag_key.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                    weak3
                        .update(cx, |this, cx| {
                            if this.ui.drag.as_deref() == Some(k.as_str()) {
                                this.ui.drag = None;
                                cx.notify();
                            }
                        })
                        .ok();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let down_key = key.to_string();
        div()
            .id(eid(format!("{key}-slider")))
            .relative()
            .h(px(20.))
            .flex()
            .items_center()
            .when(disabled, |d| d.opacity(0.5))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(pal.c("chip"))
                    .child(probe)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .rounded(px(2.))
                            .bg(pal.c("accent"))
                            .w(relative(ratio)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-6.))
                            .left(relative(ratio))
                            .ml(px(-8.))
                            .size(px(16.))
                            .rounded_full()
                            .bg(pal.c("accent"))
                            .border_2()
                            .border_color(pal.c("surface")),
                    ),
            )
            .when(!disabled, |d| {
                d.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    self.cx
                        .listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                            let v = snap(f32::from(ev.position.x), bounds.get());
                            this.ui.drag = Some(down_key.clone());
                            on(this, v, cx);
                        }),
                )
            })
            .into_any_element()
    }
}

/// What a number input lets through while typing.
fn numeric_typing(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    body.chars().all(|c| c.is_ascii_digit() || c == '.') && body.matches('.').count() <= 1
}

/// What a date input lets through while typing: `YYYY-MM-DD`.
fn date_typing(s: &str) -> bool {
    s.len() <= 10
        && s.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(
        kind: &str,
        value: FieldValue,
        options: &[&str],
        default_text: Option<&str>,
    ) -> FieldReg {
        FieldReg {
            kind: kind.into(),
            value,
            required: false,
            options: options.iter().map(|s| s.to_string()).collect(),
            default_text: default_text.map(str::to_string),
        }
    }

    #[test]
    fn fields_answer_with_ids_and_numbers() {
        let t = |v: &str| FieldValue::Text(v.into());
        assert_eq!(
            to_answer(&reg("TextField", t("Ada"), &[], Some("Ada"))),
            UNCHANGED
        );
        assert_eq!(
            to_answer(&reg("TextField", t("Bob"), &[], Some("Ada"))),
            "Bob"
        );
        assert_eq!(
            to_answer(&reg("SelectField", t("b"), &["a", "b"], None)),
            "option 2"
        );
        assert_eq!(to_answer(&reg("RadioField", t(""), &["a", "b"], None)), "");
        assert_eq!(
            to_answer(&reg("CheckboxField", FieldValue::Bool(true), &[], None)),
            "yes"
        );
        assert_eq!(
            to_answer(&reg("CheckboxField", FieldValue::Bool(false), &[], None)),
            "no"
        );
        let multi = FieldValue::List(vec!["c".into(), "a".into()]);
        assert_eq!(
            to_answer(&reg("MultiSelectField", multi, &["a", "b", "c"], None)),
            "options 1, 3"
        );
        assert_eq!(to_answer(&reg("NumberField", t("3"), &[], None)), "3");
    }

    #[test]
    fn numbers_reach_state_as_numbers() {
        assert_eq!(
            for_state("NumberField", &FieldValue::Text("2.5".into())),
            json!(2.5)
        );
        assert_eq!(
            for_state("SliderField", &FieldValue::Text(String::new())),
            Value::Null
        );
        assert_eq!(
            for_state("TextField", &FieldValue::Text("x".into())),
            json!("x")
        );
        assert_eq!(
            for_state("MultiSelectField", &FieldValue::List(vec!["a".into()])),
            json!(["a"])
        );
    }

    #[test]
    fn typing_filters_hold() {
        assert!(numeric_typing("-12.5"));
        assert!(!numeric_typing("1.2.3"));
        assert!(date_typing("2026-11"));
        assert!(!date_typing("2026/11"));
    }
}
