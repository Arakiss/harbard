//! An operator's answer as it reads in the conversation (`AnswerEcho`): the
//! note itself holds only ids, so its words come from the view it answers.
//!
//! The chat lane shows it in place of the message text when
//! [`operator_answer`] says the note is one:
//! `cx.new(|cx| AnswerEcho::new(message.clone(), window, cx))`.

use super::answer::{self, Answer, DescribedLine};
use super::fetch_view;
use crate::desk::Desk;
use crate::model::Message;
use crate::theme::Theme;
use gpui::{
    AppContext, Context, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px,
};

/// The note as an answer to a view, when it is one and the operator wrote it.
pub fn operator_answer(message: &Message) -> Option<Answer> {
    (message.from == "operator")
        .then(|| answer::read_answer(&message.text))
        .flatten()
}

pub struct AnswerEcho {
    answer: Option<Answer>,
    lines: Vec<DescribedLine>,
    /// The view attachment it was worded against, once found.
    worded: Option<String>,
    message_id: String,
}

impl AnswerEcho {
    pub fn new(message: Message, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let answer = operator_answer(&message);
        let lines = answer
            .as_ref()
            .map(answer::describe_raw)
            .unwrap_or_default();
        cx.observe(&Desk::get(cx), |this, _, cx| this.word(cx))
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let mut echo = AnswerEcho {
            answer,
            lines,
            worded: None,
            message_id: message.id,
        };
        echo.word(cx);
        echo
    }

    /// Find the message this note replies to, read its view, and word the lines.
    fn word(&mut self, cx: &mut Context<Self>) {
        let Some(answer) = self.answer.clone() else {
            return;
        };
        if self.worded.is_some() {
            return;
        }
        let desk = Desk::get(cx).read(cx);
        let Some(view) = desk.view.as_ref() else {
            return;
        };
        let Some(parent) = view
            .messages
            .iter()
            .find(|m| m.replies.contains(&self.message_id))
        else {
            return;
        };
        let Some(id) = parent
            .attachments
            .iter()
            .find(|a| a.kind == "huginn" && a.name == answer.file)
            .map(|a| a.id.clone())
        else {
            return;
        };
        self.worded = Some(id.clone());
        cx.spawn(async move |this, cx| {
            let parsed = cx.background_spawn(async move { fetch_view(&id) }).await;
            if let Some(Ok(spec)) = parsed {
                this.update(cx, |echo, cx| {
                    echo.lines = answer::describe(&spec, &answer);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for AnswerEcho {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let ink3 = theme.c("ink-3");
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(self.lines.iter().map(|line| {
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(div().text_color(ink3).child(line.label.clone()))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(line.value.clone()),
                    )
            }))
    }
}
