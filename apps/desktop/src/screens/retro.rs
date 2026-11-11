//! Retro (`/retro`): the rules the week's retro suggests, each with the
//! evidence behind it, and the rules in force. No figures: a number the
//! operator cannot act on is not shown. The operator approves a rule
//! as written, edits it first (serve asks to confirm the operator's own
//! words), or discards it; an active rule can be retired. Every decision is
//! `POST /workspace/rule-*`, then the retro is read again.

use super::secondary::{self as kit, button, head, main_column, quiet_button};
use crate::derive::history::numeric_date;
use crate::derive::retro::{
    check_line, invisible_refusal, lesson_evidence, lesson_title, memory_sink_note,
};
use crate::i18n::{I18n, t, tv};
use crate::model::retro::{ActiveRule, Lesson, MemorySinkOutcome, RetroBody};
use crate::theme::Theme;
use crate::ui::{self, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Textarea, TextareaState};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// `.hb-lesson-rule { border-left: 3px }`.
const RULE_W: f32 = 3.;
const RULE_MAX: usize = 240;

/// One suggested rule's own state.
#[derive(Default)]
struct LessonState {
    draft: Option<Entity<TextareaState>>,
    busy: bool,
    error: Option<SharedString>,
    tick: bool,
    /// serve's one-time code, the text it will keep, and the body to resend.
    confirm: Option<(String, String, Value)>,
}

pub struct RetroScreen {
    body: Option<RetroBody>,
    error: Option<SharedString>,
    notice: Option<SharedString>,
    lessons: HashMap<String, LessonState>,
    retiring: HashSet<String>,
    scroll: ScrollHandle,
    _load: Option<Task<()>>,
    _acts: Vec<Task<()>>,
}

impl RetroScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<I18n>(|_, cx| cx.notify()).detach();
        let mut this = RetroScreen {
            body: None,
            error: None,
            notice: None,
            lessons: HashMap::new(),
            retiring: HashSet::new(),
            scroll: ScrollHandle::new(),
            _load: None,
            _acts: Vec::new(),
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let path = "/retro".to_string();
        let task = kit::fetch::<RetroBody>(path, cx);
        self._load = Some(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(body) => {
                        this.lessons
                            .retain(|k, _| body.candidates.iter().any(|l| &l.key == k));
                        this.body = Some(body);
                        this.error = None;
                    }
                    Err(_) => this.error = Some(t(cx, "retro.notResponding")),
                }
                cx.notify();
            });
        }));
    }

    fn act_lesson(
        &mut self,
        key: String,
        action: &'static str,
        body: Value,
        cx: &mut Context<Self>,
    ) {
        let state = self.lessons.entry(key.clone()).or_default();
        state.busy = true;
        state.error = None;
        let sent = body.clone();
        let task = kit::send(format!("/workspace/{action}"), body, cx);
        self._acts.push(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = this.lessons.entry(key.clone()).or_default();
                state.busy = false;
                match out {
                    Ok(answer) => {
                        // Asked to confirm: nothing is approved yet.
                        if let Some(confirm) = answer.get("confirm").filter(|c| !c.is_null()) {
                            let id = confirm["id"].as_str().unwrap_or_default().to_string();
                            let text = confirm["shown"]["text"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string();
                            state.confirm = Some((id, text, sent));
                        } else {
                            state.confirm = None;
                            this.notice = Some(if action == "rule-approve" {
                                match answer.get("memory_sink").filter(|m| !m.is_null()).and_then(
                                    |m| serde_json::from_value::<MemorySinkOutcome>(m.clone()).ok(),
                                ) {
                                    Some(o) if o.saved => t(cx, "retro.noticeApprovedMemorySink"),
                                    Some(o) => memory_sink_note(&o, cx)
                                        .unwrap_or_else(|| t(cx, "retro.noticeApproved")),
                                    None => t(cx, "retro.noticeApproved"),
                                }
                            } else {
                                t(cx, "retro.noticeDiscarded")
                            });
                            this.load(cx);
                        }
                    }
                    Err(e) => {
                        state.error = Some(
                            invisible_refusal(&e.message, cx)
                                .or_else(|| {
                                    e.message
                                        .contains("does not propose")
                                        .then(|| t(cx, "retro.errorRuleGone"))
                                })
                                .unwrap_or_else(|| t(cx, "retro.errorSaveFailed")),
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn retire(&mut self, id: String, cx: &mut Context<Self>) {
        self.retiring.insert(id.clone());
        let task = kit::send("/workspace/rule-retire".into(), json!({ "id": id }), cx);
        self._acts.push(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                this.retiring.remove(&id);
                if out.is_ok() {
                    this.notice = Some(t(cx, "retro.noticeRetired"));
                    this.load(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn approve(&mut self, lesson: &Lesson, memory_sink: bool, cx: &mut Context<Self>) {
        let state = self.lessons.entry(lesson.key.clone()).or_default();
        let draft = state.draft.as_ref().map(|d| d.read(cx).value().to_string());
        let editing = draft.is_some();
        let mut text = draft.clone().unwrap_or_else(|| lesson.rule.clone());
        if text.chars().count() > RULE_MAX {
            text = text.chars().take(RULE_MAX).collect();
        }
        let edited = editing && text.trim() != lesson.rule;
        let tick = state.tick;
        let body = json!({
            "key": lesson.key,
            "title": lesson.title,
            "text": text,
            "edited": edited,
            "evidence": lesson.evidence,
            "memory_sink": memory_sink && tick,
        });
        self.act_lesson(lesson.key.clone(), "rule-approve", body, cx);
    }

    fn lesson_card(
        &mut self,
        lesson: &Lesson,
        memory_sink: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = lesson.key.clone();
        let title = lesson_title(&lesson.key, &lesson.title, cx);
        let state = self.lessons.entry(key.clone()).or_default();
        let busy = state.busy;
        let editing = state.draft.is_some();
        let empty_draft = state
            .draft
            .as_ref()
            .is_some_and(|d| d.read(cx).value().trim().is_empty());
        let error = state.error.clone();
        let tick = state.tick;
        let confirm = state.confirm.clone();
        let draft = state.draft.clone();
        // The rule is what gets approved: it reads as text, not as a box.
        let rule: AnyElement = match draft {
            Some(d) => div()
                .p(px(space::LG))
                .rounded(px(radius::BASE))
                .border_1()
                .border_color(theme.c("line-strong"))
                .bg(theme.c("surface"))
                .text_size(px(size::UI))
                .line_height(px(leading::UI))
                .child(Textarea::new(&d).appearance(false))
                .into_any_element(),
            // `.hb-lesson-rule`: the rule on a 3 px rule, the console's quote.
            None => ui::text(
                lesson.rule.clone(),
                size::UI,
                weight::REGULAR,
                theme.c("ink"),
            )
            .py(px(space::MD))
            .px(px(space::LG))
            .border_l(px(RULE_W))
            .border_color(theme.c("line-strong"))
            .bg(theme.c("surface-2"))
            .into_any_element(),
        };
        let l_approve = lesson.clone();
        let l_edit = lesson.clone();
        let k_discard = key.clone();
        let k_tick = key.clone();
        // `.hb-lesson-actions`: Approve is the filled primary, Edit and
        // Discard are edged buttons beside it (§3.1).
        let mut actions =
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .mt(px(space::XS))
                .child(
                    button(
                        SharedString::from(format!("approve-{key}")),
                        t(cx, "retro.approve"),
                        true,
                        busy || (editing && empty_draft),
                        theme,
                    )
                    .when(!busy && !(editing && empty_draft), |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.approve(&l_approve, memory_sink, cx)
                        }))
                    }),
                );
        if !editing {
            actions = actions.child(
                button(
                    SharedString::from(format!("edit-{key}")),
                    t(cx, "retro.edit"),
                    false,
                    busy,
                    theme,
                )
                .when(!busy, |b| {
                    b.on_click(cx.listener(move |this, _, window, cx| {
                        let rule = l_edit.rule.clone();
                        let area =
                            cx.new(|cx| TextareaState::new(window, cx).rows(3).default_value(rule));
                        area.update(cx, |a, cx| a.focus(window, cx));
                        this.lessons.entry(l_edit.key.clone()).or_default().draft = Some(area);
                        cx.notify();
                    }))
                }),
            );
        }
        actions = actions.child(
            button(
                SharedString::from(format!("discard-{key}")),
                t(cx, "retro.discard"),
                false,
                busy,
                theme,
            )
            .when(!busy, |b| {
                b.on_click(cx.listener(move |this, _, _, cx| {
                    this.act_lesson(
                        k_discard.clone(),
                        "rule-discard",
                        json!({ "key": k_discard }),
                        cx,
                    )
                }))
            }),
        );
        if memory_sink {
            actions = actions.child(
                div()
                    .ml(px(space::MD))
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .text_color(theme.c("ink-2"))
                    .child(
                        Checkbox::new(SharedString::from(format!("sink-{key}")))
                            .checked(tick)
                            .label(t(cx, "retro.saveMemorySink"))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.lessons.entry(k_tick.clone()).or_default().tick = *checked;
                                cx.notify();
                            })),
                    ),
            );
        }
        if let Some(e) = error {
            actions = actions.child(kit::error_note(e, theme));
        }
        let confirm_box = confirm.map(|(id, text, body)| {
            let k_yes = key.clone();
            let k_no = key.clone();
            let template = t(cx, "retro.confirmBody");
            let shown = template.replace("{text}", &format!("“{text}”"));
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .mt(px(space::MD))
                .child(ui::text(shown, size::UI, weight::REGULAR, theme.c("ink-2")).w_full())
                .child(
                    button(
                        SharedString::from(format!("confirm-{key}")),
                        t(cx, "retro.confirm"),
                        true,
                        busy,
                        theme,
                    )
                    .when(!busy, |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            let mut body = body.clone();
                            if let Some(obj) = body.as_object_mut() {
                                obj.insert("confirm".into(), json!(id));
                            }
                            this.act_lesson(k_yes.clone(), "rule-approve", body, cx);
                        }))
                    }),
                )
                .child(
                    quiet_button(
                        SharedString::from(format!("cancel-{key}")),
                        t(cx, "retro.cancel"),
                        busy,
                        theme,
                    )
                    .when(!busy, |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.lessons.entry(k_no.clone()).or_default().confirm = None;
                            cx.notify();
                        }))
                    }),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .mt(px(space::LG))
            .p(px(space::XL))
            .border_1()
            .border_color(theme.c("line-strong"))
            .rounded(px(radius::MD))
            .child(
                div()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .child(ui::icon("lightbulb", icon_size::ROW, theme.c("ink-2")))
                            .child(
                                ui::text(title, size::BODY, weight::STRONG, theme.c("ink"))
                                    .min_w_0()
                                    .truncate(),
                            )
                            .child(
                                ui::text(
                                    lesson.dates.join(", "),
                                    size::META,
                                    weight::REGULAR,
                                    theme.c("ink-3"),
                                )
                                .ml_auto()
                                .flex_none(),
                            ),
                    )
                    .child(
                        ui::text(
                            lesson_evidence(lesson, cx),
                            size::UI,
                            weight::REGULAR,
                            theme.c("ink-2"),
                        )
                        .mt(px(space::SM)),
                    ),
            )
            .child(rule)
            .child(actions)
            .children(confirm_box)
            .into_any_element()
    }

    fn rule_row(&self, rule: &ActiveRule, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut meta = tv(
            cx,
            "retro.ruleApproved",
            &[("date", &numeric_date(rule.approved_ms, cx))],
        )
        .to_string();
        if rule.edited {
            meta.push_str(&format!(" · {}", t(cx, "retro.ruleEdited")));
        }
        if rule.memory_sink {
            meta.push_str(&format!(" · {}", t(cx, "retro.ruleSavedMemorySink")));
        }
        let busy = self.retiring.contains(&rule.id);
        let id = rule.id.clone();
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(space::XL))
            .py(px(space::LG))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .min_w_0()
                    .child(ui::text(
                        lesson_title(&rule.key, &rule.title, cx),
                        size::UI,
                        weight::STRONG,
                        theme.c("ink"),
                    ))
                    .child(ui::text(
                        rule.text.clone(),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-2"),
                    ))
                    .child(
                        ui::text(meta, size::META, weight::REGULAR, theme.c("ink-3"))
                            .mt(px(space::HAIR)),
                    ),
            )
            .child(
                quiet_button(
                    SharedString::from(format!("retire-{}", rule.id)),
                    t(cx, "retro.retire"),
                    busy,
                    theme,
                )
                .when(!busy, |b| {
                    b.on_click(cx.listener(move |this, _, _, cx| this.retire(id.clone(), cx)))
                }),
            )
            .into_any_element()
    }
}

/// The heading of one block of the page: 14/700, under the pane's title.
fn section_title(text: SharedString, theme: &Theme) -> impl IntoElement {
    ui::text(text, size::UI, weight::STRONG, theme.c("ink"))
}

impl RetroScreen {
    fn view(&mut self, body: &RetroBody, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // Suggested rules.
        let memory_sink = body
            .memory_sink
            .as_ref()
            .is_some_and(|m| m.configured && m.allowed == Some(true));
        let mut lessons = div()
            .mt(px(space::SECTION))
            .mx(px(space::GUTTER))
            .child(section_title(t(cx, "retro.proposedTitle"), theme));
        if body.candidates.is_empty() {
            lessons = lessons.child(none_line(t(cx, "retro.noCandidates"), theme));
        }
        if let Some(n) = self.notice.clone() {
            lessons = lessons.child(
                ui::text(n, size::META, weight::REGULAR, theme.c("ink-3")).mt(px(space::XS)),
            );
        }
        for lesson in body.candidates.clone() {
            lessons = lessons.child(self.lesson_card(&lesson, memory_sink, theme, cx));
        }
        // "Was it you?": what changed these days.
        let checks = (!body.checks.is_empty()).then(|| {
            div()
                .mt(px(space::SECTION))
                .mx(px(space::GUTTER))
                .child(section_title(t(cx, "retro.recentTitle"), theme))
                .child(sub_line(t(cx, "retro.recentHint"), theme))
                .child(
                    div()
                        .mt(px(space::MD))
                        .flex()
                        .flex_col()
                        .gap(px(space::XS))
                        .children(body.checks.iter().map(|c| {
                            ui::text(
                                check_line(c, cx),
                                size::UI,
                                weight::REGULAR,
                                theme.c("ink-2"),
                            )
                        })),
                )
        });
        // Active rules.
        let mut rules = div()
            .mt(px(space::SECTION))
            .mx(px(space::GUTTER))
            .child(section_title(t(cx, "retro.activeTitle"), theme))
            .child(sub_line(t(cx, "retro.activeHint"), theme));
        if body.rules.is_empty() {
            rules = rules.child(none_line(t(cx, "retro.noRules"), theme));
        }
        let mut ul = div().mt(px(space::XS));
        for rule in &body.rules {
            ul = ul.child(self.rule_row(rule, theme, cx));
        }
        rules = rules.child(ul);
        div()
            .id("retro")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pb(px(space::BREAK))
            .child(lessons)
            .children(checks)
            .child(rules)
            .into_any_element()
    }
}

fn none_line(text: SharedString, theme: &Theme) -> impl IntoElement {
    ui::text(text, size::UI, weight::REGULAR, theme.c("ink-3")).mt(px(space::XS))
}

fn sub_line(text: SharedString, theme: &Theme) -> impl IntoElement {
    ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-3"))
}

impl Render for RetroScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let content = match self.body.clone() {
            Some(body) => self.view(&body, &theme, cx),
            None => kit::empty_block(
                None,
                vec![
                    self.error
                        .clone()
                        .unwrap_or_else(|| t(cx, "retro.computing"))
                        .into_any_element(),
                ],
                &theme,
            )
            .into_any_element(),
        };
        main_column(&theme)
            .relative()
            .child(head(
                t(cx, "retro.title"),
                Some(t(cx, "retro.topic")),
                &theme,
            ))
            .child(content)
    }
}
