//! Retro (`/retro`): the week's (or one finished team's) numbers, the rules
//! the retro suggests and the rules in force. The operator approves a rule
//! as written, edits it first (serve asks to confirm the operator's own
//! words), or discards it; an active rule can be retired. Every decision is
//! `POST /workspace/rule-*`, then the retro is read again.

use super::secondary::{self as kit, button, head, main_column, quiet_button, width_probe};
use crate::api::enc;
use crate::derive::history::{long_day, numeric_date};
use crate::derive::retro::{
    Tile, check_line, feedback_label, feedback_value_label, invisible_refusal, lesson_evidence,
    lesson_title, memory_sink_note, spark_points, tiles,
};
use crate::i18n::{I18n, t, tn, tv};
use crate::model::retro::{ActiveRule, Lesson, MemorySinkOutcome, RetroBody};
use crate::theme::Theme;
use crate::ui::{self, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, Context, Entity, Hsla, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Task,
    Window, canvas, div, point, px,
};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Textarea, TextareaState};
use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const SPARK_W: f32 = 112.;
const SPARK_H: f32 = 28.;
/// The sparkline's stroke and the mark on its last day.
const SPARK_STROKE: f32 = 1.5;
const SPARK_DOT: f32 = space::MD;
/// `.hb-tile-value { font-size: 26px }` and its line.
const FIGURE: f32 = 26.;
const FIGURE_LEADING: f32 = 32.;
/// `.hb-lesson-rule { border-left: 3px }`.
const RULE_W: f32 = 3.;
const TILE_MIN: f32 = 220.;
const TILE_GAP: f32 = space::LG;
const RULE_MAX: usize = 240;

#[derive(Clone, PartialEq, Eq)]
enum Scope {
    Week,
    Team(String),
}

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
    scope: Scope,
    body: Option<RetroBody>,
    error: Option<SharedString>,
    notice: Option<SharedString>,
    lessons: HashMap<String, LessonState>,
    retiring: HashSet<String>,
    table_open: bool,
    width: Rc<Cell<f32>>,
    scroll: ScrollHandle,
    _load: Option<Task<()>>,
    _acts: Vec<Task<()>>,
}

impl RetroScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<I18n>(|_, cx| cx.notify()).detach();
        let mut this = RetroScreen {
            scope: Scope::Week,
            body: None,
            error: None,
            notice: None,
            lessons: HashMap::new(),
            retiring: HashSet::new(),
            table_open: false,
            width: Rc::new(Cell::new(0.)),
            scroll: ScrollHandle::new(),
            _load: None,
            _acts: Vec::new(),
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let path = match &self.scope {
            Scope::Week => "/retro".to_string(),
            Scope::Team(lead) => format!("/retro?scope=team&lead={}", enc(lead)),
        };
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

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        if self.scope != scope {
            self.scope = scope;
            self.load(cx);
            cx.notify();
        }
    }

    /// Approve (or confirm), discard: the lesson's own busy and error.
    fn act_lesson(
        &mut self,
        key: String,
        action: &'static str,
        mut body: Value,
        cx: &mut Context<Self>,
    ) {
        if action == "rule-approve"
            && let (Scope::Team(lead), Some(obj)) = (&self.scope, body.as_object_mut())
        {
            obj.insert("lead".into(), json!(lead));
        }
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

/// A tile's sparkline: the line in the de-emphasis ink, today as a dot in
/// the reading ink (a count is not a state and not the accent's job).
fn spark(values: &[u64], ink: Hsla, today: Hsla) -> Option<AnyElement> {
    let points = spark_points(values, SPARK_W, SPARK_H);
    let &(lx, ly) = points.last()?;
    Some(
        div()
            .relative()
            .flex_none()
            .w(px(SPARK_W))
            .h(px(SPARK_H))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        let o = bounds.origin;
                        let mut path = PathBuilder::stroke(px(SPARK_STROKE));
                        for (i, &(x, y)) in points.iter().enumerate() {
                            let p = point(o.x + px(x), o.y + px(y));
                            if i == 0 {
                                path.move_to(p)
                            } else {
                                path.line_to(p)
                            }
                        }
                        if let Ok(p) = path.build() {
                            window.paint_path(p, ink);
                        }
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(lx - SPARK_DOT / 2.))
                    .top(px(ly - SPARK_DOT / 2.))
                    .size(px(SPARK_DOT))
                    .rounded_full()
                    .bg(today),
            )
            .into_any_element(),
    )
}

fn tile_view(tile: &Tile, theme: &Theme) -> AnyElement {
    div()
        .flex_1()
        .px(px(space::XL))
        .py(px(space::LG))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(radius::MD))
        .child(
            ui::text(
                tile.label.clone(),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-3"),
            )
            .truncate(),
        )
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap(px(space::MD))
                .mt(px(space::XS))
                // `.hb-tile-value`: 26/700, today's point in the accent (§3.1).
                .child(
                    ui::text(tile.value.clone(), FIGURE, weight::STRONG, theme.c("ink"))
                        .line_height(px(FIGURE_LEADING)),
                )
                .children(
                    tile.series
                        .as_deref()
                        .and_then(|s| spark(s, theme.c("ink-4"), theme.c("accent"))),
                ),
        )
        .children(
            tile.detail.clone().map(|d| {
                ui::text(d, size::SMALL, weight::REGULAR, theme.c("ink-2")).mt(px(space::XS))
            }),
        )
        .into_any_element()
}

/// A table as columns side by side, each as wide as its widest cell.
fn table(
    header: Option<Vec<SharedString>>,
    rows: Vec<Vec<SharedString>>,
    theme: &Theme,
) -> AnyElement {
    let cols = header
        .as_ref()
        .map(Vec::len)
        .or_else(|| rows.first().map(Vec::len))
        .unwrap_or(0);
    let cell = |text: SharedString, heft, ink: &str| {
        ui::text(text, size::SMALL, heft, theme.c(ink)).py(px(space::HAIR))
    };
    let mut out = div().flex().mt(px(space::MD));
    for c in 0..cols {
        let mut col = div().flex().flex_col().pr(px(space::XL));
        if let Some(h) = &header {
            col = col.child(cell(h[c].clone(), weight::STRONG, "ink-3"));
        }
        for row in &rows {
            let text = row.get(c).cloned().unwrap_or_default();
            // The first column names the row; the rest are its numbers.
            col = col.child(cell(
                text,
                weight::REGULAR,
                if c == 0 { "ink" } else { "ink-2" },
            ));
        }
        out = out.child(col);
    }
    out.into_any_element()
}

/// The heading of one block of the page: 14/700, under the pane's title.
fn section_title(text: SharedString, theme: &Theme) -> impl IntoElement {
    ui::text(text, size::UI, weight::STRONG, theme.c("ink"))
}

impl RetroScreen {
    fn view(&mut self, body: &RetroBody, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let r = &body.retro;
        // Scope tabs: this week, then each finished team.
        let mut tabs = div()
            .id("retro-tabs")
            .flex_none()
            .flex()
            .overflow_x_scroll()
            .gap(px(space::XL))
            .h(px(row::TABS))
            .px(px(space::GUTTER))
            .border_b_1()
            .border_color(theme.c("line"));
        let mut scopes = vec![(Scope::Week, t(cx, "retro.thisWeek"))];
        for lead in &body.finished_teams {
            let id: String = lead.chars().take(8).collect();
            scopes.push((
                Scope::Team(lead.clone()),
                tv(cx, "retro.finishedTeam", &[("id", &id)]),
            ));
        }
        for (i, (scope, label)) in scopes.into_iter().enumerate() {
            let on = scope == self.scope;
            // The selected tab is 700 in ink over its underline; the rest
            // recede to 400 in ink-3 and show a grey underline under the
            // pointer.
            let hover_edge = theme.c("line-hover");
            tabs = tabs.child(
                kit::row_focus(div().id(("retro-tab", i)), theme)
                    .flex()
                    .flex_none()
                    .items_center()
                    .h_full()
                    .border_b_2()
                    .border_color(if on {
                        theme.c("accent")
                    } else {
                        gpui::transparent_black()
                    })
                    .cursor_pointer()
                    .when(!on, |d| d.hover(move |s| s.border_color(hover_edge)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope.clone(), cx)))
                    .child(
                        ui::text(
                            label,
                            size::UI,
                            if on { weight::STRONG } else { weight::REGULAR },
                            theme.c(if on { "ink" } else { "ink-3" }),
                        )
                        .whitespace_nowrap(),
                    ),
            );
        }
        let agents = tn(cx, "retro.agents", r.agents, &[]);
        let sub = tv(
            cx,
            "retro.rangeLine",
            &[
                ("agents", &agents),
                ("from", &long_day(r.since_ms, cx)),
                ("to", &long_day(r.now_ms, cx)),
            ],
        );
        // Tiles: repeat(auto-fill, minmax(220px, 1fr)), gap 12.
        let avail = (self.width.get() - 2. * space::GUTTER).max(TILE_MIN);
        let cols = (((avail + TILE_GAP) / (TILE_MIN + TILE_GAP)).floor() as usize).max(1);
        let tile_w = ((avail - TILE_GAP * (cols as f32 - 1.)) / cols as f32).floor();
        let list = tiles(r, cx);
        let mut grid = div()
            .flex()
            .flex_col()
            .gap(px(TILE_GAP))
            .mt(px(space::XL))
            .mx(px(space::GUTTER));
        for chunk in list.chunks(cols) {
            let mut row = div().flex().items_stretch().gap(px(TILE_GAP));
            for tile in chunk {
                row = row.child(
                    div()
                        .w(px(tile_w))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .child(tile_view(tile, theme)),
                );
            }
            grid = grid.child(row);
        }
        // "See the data by day".
        let open = self.table_open;
        let hover = theme.c("hover");
        let mut details = div().mt(px(space::LG)).mx(px(space::GUTTER)).child(
            div().flex().child(
                kit::row_focus(div().id("retro-table"), theme)
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::XS))
                    .ml(px(-space::XS))
                    .rounded(px(radius::SM))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.table_open = !this.table_open;
                        cx.notify();
                    }))
                    .child(ui::icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        icon_size::INLINE,
                        theme.c("ink-3"),
                    ))
                    .child(ui::text(
                        t(cx, "retro.dataByDay"),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )),
            ),
        );
        if open {
            let header = vec![
                t(cx, "retro.colDay"),
                t(cx, "retro.colFindings"),
                t(cx, "retro.colUnclear"),
                t(cx, "retro.colStalls"),
                t(cx, "retro.colUnanswered"),
            ];
            let rows = r
                .daily
                .iter()
                .map(|d| {
                    vec![
                        d.day.clone().into(),
                        d.findings.to_string().into(),
                        d.not_understood.to_string().into(),
                        d.stalls.to_string().into(),
                        d.unanswered.to_string().into(),
                    ]
                })
                .collect();
            details = details.child(table(Some(header), rows, theme));
            if !r.feedback.is_empty() {
                let rows = r
                    .feedback
                    .iter()
                    .map(|(kind, values)| {
                        let cell = values
                            .iter()
                            .map(|(v, c)| format!("{}: {c}", feedback_value_label(v, cx)))
                            .collect::<Vec<_>>()
                            .join(" · ");
                        vec![feedback_label(kind, cx), cell.into()]
                    })
                    .collect();
                details = details
                    .child(
                        ui::text(
                            t(cx, "retro.yourTaps"),
                            size::SMALL,
                            weight::STRONG,
                            theme.c("ink-3"),
                        )
                        .pt(px(space::LG)),
                    )
                    .child(table(None, rows, theme));
            }
        }
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
            .child(tabs)
            .child(
                ui::text(sub, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                    .mt(px(space::XL))
                    .mx(px(space::GUTTER)),
            )
            .child(grid)
            .child(details)
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
            .child(width_probe(self.width.clone(), cx.entity_id()))
    }
}
