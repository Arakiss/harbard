//! "Watch live" (`/direct/<agent>?direct=1`; `live-pane.tsx`): one agent's
//! tool calls and its own words as they happen. Private and local: the log
//! is read in-process (serve's `live_route`) every second while the window
//! is visible, and not at all while it is hidden. No thinking, no output.

use crate::derive::agent_card::{step_icon, step_kind, step_text, test_summary};
use crate::derive::agent_name;
use crate::derive::live::{
    Block, Inline, LiveItem, duration, live_heading, live_items, prose_blocks, status_text,
};
use crate::desk::Desk;
use crate::i18n::{I18n, Locale, t, tv};
use crate::model::Agent;
use crate::model::inbox::{LiveStep, Prose};
use crate::nav::{self, Route};
use crate::screens::inbox as parts;
use crate::theme::Theme;
use crate::ui::agent_card::{LivePoll, StatusStyle, second_tick, status_line};
use crate::ui::format::local_hm;
use crate::ui::{
    self, MONO, avatar_size, control, icon, icon_size, leading, radius, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext, Context, Div, Entity, FollowMode,
    IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Transformation, Window, div, list,
    percentage, px,
};
use std::rc::Rc;
use std::time::Duration;

/// A timeline row, owned, so the list can draw it on its own.
#[derive(Clone)]
enum Row {
    Step(LiveStep),
    Prose(Prose),
}

pub struct LiveView {
    key: String,
    poll: Entity<LivePoll>,
    rows: Rc<Vec<Row>>,
    rev: Option<u64>,
    list: ListState,
    _tick: Task<()>,
    _subs: Vec<Subscription>,
}

impl LiveView {
    pub fn new(key: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let poll = cx.new(|cx| LivePoll::new(key.clone(), true, false, window, cx));
        let list = ListState::new(0, ListAlignment::Top, px(400.));
        list.set_follow_mode(FollowMode::Tail);
        let subs = vec![
            cx.observe(&poll, |this, _, cx| {
                this.sync(cx);
                cx.notify();
            }),
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
        ];
        LiveView {
            key,
            poll,
            rows: Rc::new(Vec::new()),
            rev: None,
            list,
            _tick: second_tick(window, cx),
            _subs: subs,
        }
    }

    /// Take the poll's newest state into the list, once per revision.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let poll = self.poll.read(cx);
        let Some(state) = &poll.state else { return };
        if self.rev == Some(state.rev) {
            return;
        }
        self.rev = Some(state.rev);
        let rows: Vec<Row> = live_items(state)
            .into_iter()
            .map(|i| match i {
                LiveItem::Step(s) => Row::Step(s.clone()),
                LiveItem::Prose(p) => Row::Prose(p.clone()),
            })
            .collect();
        self.list.reset(rows.len());
        self.rows = Rc::new(rows);
    }
}

fn hhmmss(ms: u64) -> String {
    let (h, m) = local_hm(ms);
    format!("{h:02}:{m:02}:{:02}", (ms / 1000) % 60)
}

/// The column the timeline's marks and the agent's face sit in.
const MARK_COL: f32 = avatar_size::STACK;
/// Fixed boxes for the start time, the duration and the state, so a ticking
/// duration does not shake its row (T8).
const TIME_W: f32 = 64.;
const TOOK_W: f32 = 72.;
const STATE_W: f32 = 96.;
/// The periods of the two marks that report a live state (M4).
const SPIN: Duration = Duration::from_millis(1200);
const PULSE: Duration = Duration::from_millis(1600);

/// One tool call on the timeline (`TimelineRow`): its mark, what it is,
/// when it started and how long it took; a chip only when it is not done.
fn timeline_row(step: &LiveStep, first: bool, last: bool, now: u64, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let s = &step.step;
    let kind = step_kind(s).to_string();
    let took = s.ended_ms.unwrap_or(now).saturating_sub(s.started_ms);
    let comma = cx.global::<I18n>().locale == Locale::Es;
    let still = crate::robot::clock::reduced(cx);
    let mark = match s.status.as_str() {
        "running" => {
            let spinner = icon("loader-circle", icon_size::INLINE, theme.c("ink-2"));
            if still {
                spinner.into_any_element()
            } else {
                spinner
                    .with_animation(
                        SharedString::from(format!("hb-tl-spin-{}", step.n)),
                        Animation::new(SPIN).repeat(),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    )
                    .into_any_element()
            }
        }
        "error" => icon("x", icon_size::INLINE, theme.c("red")).into_any_element(),
        "stopped" => icon("minus", icon_size::INLINE, theme.c("grey")).into_any_element(),
        _ => icon("check", icon_size::INLINE, theme.c("green")).into_any_element(),
    };
    // Done is the mark's to say; a chip is for what is not done (C3).
    let state = match s.status.as_str() {
        "running" | "stopped" => Some(ui::chip(status_text(&s.status, cx), "neutral", theme)),
        "error" => Some(ui::chip(status_text(&s.status, cx), "red", theme)),
        _ => None,
    };
    let meta = |text: String, width: f32| {
        ui::text(text, size::META, weight::REGULAR, theme.c("ink-3"))
            .flex_none()
            .w(px(width))
            .text_right()
    };
    div()
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(px(space::LG))
        .h(px(control::MD))
        .px(px(space::GUTTER))
        .child(rail(first, last, theme))
        .child(mark_slot(mark, theme))
        .child(icon(step_icon(&kind), icon_size::ROW, theme.c("ink-3")))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(
                    ui::text(step_text(s, cx), size::UI, weight::REGULAR, theme.c("ink"))
                        .min_w_0()
                        .truncate(),
                )
                .children(s.test.as_ref().map(|test| {
                    ui::text(
                        format!("· {}", test_summary(test, cx)),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .flex_none()
                })),
        )
        .child(meta(hhmmss(s.started_ms), TIME_W))
        .child(meta(duration(took, comma), TOOK_W))
        .child(
            div()
                .flex_none()
                .w(px(STATE_W))
                .flex()
                .justify_end()
                .children(state),
        )
        .into_any_element()
}

/// A mark on the timeline's line: the line stops behind it.
fn mark_slot(mark: AnyElement, theme: &Theme) -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(MARK_COL))
        .flex()
        .justify_center()
        .child(
            div()
                .size(px(MARK_COL))
                .rounded_full()
                .bg(theme.c("surface"))
                .flex()
                .items_center()
                .justify_center()
                .child(mark),
        )
}

/// The vertical line joining the marks, cut at the first and last rows.
fn rail(first: bool, last: bool, theme: &Theme) -> Div {
    div()
        .absolute()
        .left(px(space::GUTTER + MARK_COL / 2.))
        .w_0()
        .border_l_1()
        .border_color(theme.c("line"))
        .when(first, |d| d.top_1_2())
        .when(!first, |d| d.top_0())
        .when(last, |d| d.bottom_1_2())
        .when(!last, |d| d.bottom_0())
}

fn inline_runs(runs: &[Inline], theme: &Theme) -> Div {
    div().flex().flex_wrap().children(runs.iter().map(|r| {
        match r {
            Inline::Text(s) => div().child(s.clone()).into_any_element(),
            Inline::Bold(s) => div()
                .font_weight(weight::STRONG)
                .child(s.clone())
                .into_any_element(),
            // Mono sits two pixels under the prose beside it (T6).
            Inline::Code(s) => div()
                .font_family(MONO)
                .text_size(px(size::SMALL))
                .px(px(space::XS))
                .rounded(px(radius::SM))
                .bg(theme.c("control"))
                .text_color(theme.c("ink-2"))
                .child(s.clone())
                .into_any_element(),
        }
    }))
}

/// The agent's words as paragraphs, lists and headings (`ProseBlock`),
/// beside its face on the timeline's line.
fn prose_row(
    prose: &Prose,
    agent: Option<&Agent>,
    first: bool,
    last: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    let blocks = prose_blocks(&prose.text);
    let body = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(space::MD))
        .font_family(ui::SANS)
        .text_color(theme.c("ink"))
        .text_size(px(size::BODY))
        .line_height(px(leading::BODY))
        .children(blocks.iter().map(|b| {
            match b {
                Block::Heading(runs) => inline_runs(runs, theme)
                    .font_weight(weight::STRONG)
                    .into_any_element(),
                Block::Para(lines) => div()
                    .flex()
                    .flex_col()
                    .children(lines.iter().map(|l| inline_runs(l, theme)))
                    .into_any_element(),
                Block::List(items) => div()
                    .flex()
                    .flex_col()
                    .children(items.iter().map(|l| {
                        div()
                            .flex()
                            .gap(px(space::MD))
                            .child(div().flex_none().child("•"))
                            .child(inline_runs(l, theme))
                    }))
                    .into_any_element(),
            }
        }));
    let face = match agent {
        Some(a) => ui::avatar::agent_avatar(a, MARK_COL, false, cx),
        None => div().into_any_element(),
    };
    div()
        .relative()
        .w_full()
        .flex()
        .items_start()
        .gap(px(space::LG))
        .px(px(space::GUTTER))
        .py(px(space::MD))
        .child(rail(first, last, theme))
        .child(mark_slot(face, theme))
        .child(body)
        .into_any_element()
}

impl Render for LiveView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let agent = desk.view.as_ref().and_then(|v| v.agent(&self.key)).cloned();
        let Some(agent) = agent else {
            return div().flex_1().into_any_element();
        };
        let poll = self.poll.read(cx);
        let (paused, error, answered) = (poll.paused, poll.error, poll.answered);
        let live = live_heading(&agent.activity.state, &agent.presence);
        let name = agent_name(&agent, cx);
        let rows = self.rows.clone();
        let count = rows.len();
        let row_agent = agent.clone();
        let timeline = list(self.list.clone(), move |ix, _window, cx| {
            let (first, last) = (ix == 0, ix + 1 == count);
            match &rows[ix] {
                Row::Step(s) => timeline_row(s, first, last, now, cx),
                Row::Prose(p) => prose_row(p, Some(&row_agent), first, last, cx),
            }
        })
        .flex_1()
        .min_h_0();
        let private = {
            let mut s = t(cx, "agents.livePrivate").to_string();
            if paused {
                s.push_str(&format!(" · {}", t(cx, "agents.livePaused")));
            } else if error {
                s.push_str(&format!(" · {}", t(cx, "agents.liveOffline")));
            }
            s
        };
        let heading = if live {
            t(cx, "agents.liveHeadingLive")
        } else {
            t(cx, "agents.liveHeadingFinished")
        };
        let waiting = paused || error;
        let still = crate::robot::clock::reduced(cx);
        // Working is green (C3); paused or offline it is grey and holds still.
        let live_mark = if !live {
            icon("check", icon_size::INLINE, theme.c("ink-3")).into_any_element()
        } else {
            let dot = parts::presence_dot(if waiting { "grey" } else { "green" }, &theme);
            if waiting || still {
                dot.into_any_element()
            } else {
                dot.with_animation(
                    "hb-live-pulse",
                    Animation::new(PULSE).repeat(),
                    |d, delta| d.opacity(1.0 - 0.45 * (delta * std::f32::consts::PI).sin()),
                )
                .into_any_element()
            }
        };
        let key = self.key.clone();
        let title = div()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .min_w_0()
            .flex_none()
            .child(ui::text(name.clone(), size::TITLE, weight::HEAVY, theme.c("ink")).truncate())
            .child(ui::chip(
                if agent.kind == "teammate" {
                    t(cx, "agents.kindAgent")
                } else {
                    t(cx, "agents.kindSubagent")
                },
                "neutral",
                &theme,
            ))
            .into_any_element();
        let head = parts::agent_head(
            ui::avatar::agent_avatar(&agent, avatar_size::ROW, true, cx),
            title,
            agent
                .description
                .clone()
                .or_else(|| agent.brief.clone())
                .map(Into::into),
            div().child(status_line(
                &agent,
                StatusStyle {
                    size: size::SMALL,
                    color: theme.c("ink-3"),
                    time_right: false,
                    show_time: true,
                },
                cx,
            )),
            &theme,
        )
        .child(
            parts::quiet_btn(
                "hb-live-chat",
                Some("message-circle"),
                t(cx, "agents.openConversation"),
                &theme,
            )
            .on_click(move |_, _, cx| nav::replace(Route::direct(Some(key.clone())), cx)),
        );
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.c("surface"))
            .child(head)
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .h(px(control::MD))
                    .mx(px(space::GUTTER))
                    .mt(px(space::LG))
                    .px(px(space::LG))
                    .rounded(px(radius::MD))
                    .bg(theme.c("surface-2"))
                    .child(icon("lock", icon_size::INLINE, theme.c("ink-3")))
                    .child(
                        ui::text(private, size::SMALL, weight::REGULAR, theme.c("ink-2"))
                            .truncate(),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::LG))
                    .mt(px(space::XL))
                    .mb(px(space::MD))
                    .px(px(space::GUTTER))
                    .child(
                        div()
                            .flex_none()
                            .w(px(MARK_COL))
                            .flex()
                            .justify_center()
                            .child(live_mark),
                    )
                    .child(ui::text(
                        heading,
                        size::UI,
                        weight::STRONG,
                        if live && !paused {
                            theme.c("ink")
                        } else {
                            theme.c("ink-3")
                        },
                    )),
            )
            .child(if !answered {
                div()
                    .flex_1()
                    .child(parts::empty(
                        None,
                        Some(tv(cx, "agents.liveReading", &[("name", &name)])),
                        &theme,
                    ))
                    .into_any_element()
            } else if count == 0 {
                div()
                    .flex_1()
                    .child(parts::empty(
                        None,
                        Some(tv(cx, "agents.liveNothingYet", &[("name", &name)])),
                        &theme,
                    ))
                    .into_any_element()
            } else {
                timeline.into_any_element()
            })
            .child(
                ui::text(
                    t(cx, "agents.liveFoot"),
                    size::META,
                    weight::REGULAR,
                    theme.c("ink-3"),
                )
                .flex_none()
                .px(px(space::GUTTER))
                .py(px(space::MD)),
            )
            .into_any_element()
    }
}
