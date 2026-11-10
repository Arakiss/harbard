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
use crate::theme::Theme;
use crate::ui::agent_card::{LivePoll, StatusStyle, second_tick, status_line};
use crate::ui::format::local_hm;
use crate::ui::{self, MONO, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext, Context, Div, Entity, FollowMode,
    FontWeight, InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Transformation, Window,
    div, list, percentage, px,
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

/// One tool call on the timeline (`TimelineRow`).
fn timeline_row(step: &LiveStep, first: bool, last: bool, now: u64, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let s = &step.step;
    let kind = step_kind(s).to_string();
    let took = s.ended_ms.unwrap_or(now).saturating_sub(s.started_ms);
    let comma = cx.global::<I18n>().locale == Locale::Es;
    let (ic_bg, ic_ink) = match kind.as_str() {
        "running" | "testing" | "editing" | "delegating" => ("accent-soft", "accent-text"),
        "reading" | "searching" | "web" => ("tint", "focus"),
        "commit" => ("red-bg", "st-orbit"),
        _ => ("ok-bg", "green-ink"),
    };
    let (pill_bg, pill_ink) = match s.status.as_str() {
        "running" => ("amber-bg", "amber-ink"),
        "error" => ("red-bg", "red-ink"),
        "stopped" => ("surface-2", "ink-3"),
        _ => ("green-bg", "green-ink"),
    };
    let mark = match s.status.as_str() {
        "running" => div()
            .size(px(24.))
            .rounded_full()
            .border_2()
            .border_color(theme.c("accent"))
            .bg(theme.c("surface"))
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(8.)).rounded_full().bg(theme.c("accent"))),
        status => {
            let (bg, glyph) = match status {
                "error" => ("red", "x"),
                "stopped" => ("grey", "minus"),
                _ => ("green", "check"),
            };
            div()
                .size(px(24.))
                .rounded_full()
                .bg(theme.c(bg))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(glyph, 14., theme.c("surface")))
        }
    };
    let mut label = div()
        .flex()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(16.))
        .child(
            div()
                .truncate()
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(step_text(s, cx)),
        );
    if let Some(test) = &s.test {
        label = label.child(
            div()
                .flex_none()
                .text_color(theme.c("ink-3"))
                .child(format!(" · {}", test_summary(test, cx))),
        );
    }
    let pill = div()
        .flex_none()
        .min_w(px(52.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .px(px(12.))
        .py(px(3.))
        .rounded_full()
        .text_size(px(14.))
        .font_weight(if s.status == "ok" || s.status == "error" {
            FontWeight::EXTRA_BOLD
        } else {
            FontWeight::BOLD
        })
        .bg(theme.c(pill_bg))
        .text_color(theme.c(pill_ink))
        .when(s.status == "running", |d| {
            d.child(
                icon("loader-circle", 16., theme.c(pill_ink)).with_animation(
                    SharedString::from(format!("hb-tl-spin-{}", step.n)),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                ),
            )
        })
        .child(status_text(&s.status, cx));
    div()
        .relative()
        .pl(px(44.))
        .py(px(4.))
        .child(rail(first, last, theme))
        .child(
            div()
                .absolute()
                .left(px(4.))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .child(mark),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(10.))
                .py(px(7.))
                .border_1()
                .border_color(theme.c("line"))
                .rounded(px(10.))
                .bg(theme.c("surface"))
                .child(
                    div()
                        .flex_none()
                        .size(px(34.))
                        .rounded(px(8.))
                        .bg(theme.c(ic_bg))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(step_icon(&kind), 18., theme.c(ic_ink))),
                )
                .child(div().flex_1().min_w_0().child(label))
                .child(
                    div()
                        .flex_none()
                        .w(px(84.))
                        .text_size(px(15.))
                        .text_color(theme.c("ink-2"))
                        .child(hhmmss(s.started_ms)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(64.))
                        .flex()
                        .justify_end()
                        .text_size(px(15.))
                        .text_color(theme.c("ink-2"))
                        .child(duration(took, comma)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(118.))
                        .flex()
                        .justify_end()
                        .child(pill),
                ),
        )
        .into_any_element()
}

/// The vertical line joining the marks, cut at the first and last rows.
fn rail(first: bool, last: bool, theme: &Theme) -> Div {
    div()
        .absolute()
        .left(px(15.))
        .w(px(2.))
        .when(first, |d| d.top_1_2())
        .when(!first, |d| d.top_0())
        .when(last, |d| d.bottom_1_2())
        .when(!last, |d| d.bottom_0())
        .bg(theme.c("line"))
}

fn inline_runs(runs: &[Inline], theme: &Theme) -> Div {
    div().flex().flex_wrap().children(runs.iter().map(|r| {
        match r {
            Inline::Text(s) => div().child(s.clone()).into_any_element(),
            Inline::Bold(s) => div()
                .font_weight(FontWeight::BOLD)
                .child(s.clone())
                .into_any_element(),
            Inline::Code(s) => div()
                .font_family(MONO)
                .text_size(px(13.))
                .px(px(4.))
                .rounded(px(4.))
                .bg(theme.c("control"))
                .border_1()
                .border_color(theme.c("line"))
                .text_color(theme.c("ink-2"))
                .child(s.clone())
                .into_any_element(),
        }
    }))
}

/// The agent's words as paragraphs, lists and headings (`ProseBlock`).
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
        .flex()
        .flex_col()
        .gap(px(8.))
        .min_h(px(56.))
        .pl(px(62.))
        .pr(px(14.))
        .py(px(10.))
        .rounded(px(10.))
        .bg(theme.c("tint"))
        .text_color(theme.c("ink"))
        .text_size(px(15.))
        .line_height(px(22.))
        .children(blocks.iter().map(|b| {
            match b {
                Block::Heading(runs) => inline_runs(runs, theme)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .into_any_element(),
                Block::Para(lines) => div()
                    .flex()
                    .flex_col()
                    .children(lines.iter().map(|l| inline_runs(l, theme)))
                    .into_any_element(),
                Block::List(items) => div()
                    .flex()
                    .flex_col()
                    .pl(px(6.))
                    .children(items.iter().map(|l| {
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(div().flex_none().child("•"))
                            .child(inline_runs(l, theme))
                    }))
                    .into_any_element(),
            }
        }));
    div()
        .relative()
        .pl(px(44.))
        .py(px(4.))
        .child(rail(first, last, theme))
        .child(body)
        .children(agent.map(|a| {
            div()
                .absolute()
                .left(px(56.))
                .top(px(14.))
                .child(ui::avatar::agent_avatar(a, 36., false, cx))
        }))
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
        let dot_paused = paused || error;
        let key = self.key.clone();
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.c("surface"))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_start()
                    .gap(px(16.))
                    .pt(px(18.))
                    .px(px(24.))
                    .pb(px(14.))
                    .child(ui::avatar::agent_avatar(&agent, 64., true, cx))
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(24.))
                                            .line_height(px(29.))
                                            .font_weight(FontWeight::BLACK)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .px(px(10.))
                                            .py(px(1.))
                                            .rounded(px(6.))
                                            .bg(theme.c("green-bg"))
                                            .text_color(theme.c("green-ink"))
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::BOLD)
                                            .child(if agent.kind == "teammate" {
                                                t(cx, "agents.kindAgent")
                                            } else {
                                                t(cx, "agents.kindSubagent")
                                            }),
                                    ),
                            )
                            .children(
                                agent
                                    .description
                                    .clone()
                                    .or_else(|| agent.brief.clone())
                                    .map(|b| {
                                        div()
                                            .mt(px(2.))
                                            .mb(px(4.))
                                            .text_color(theme.c("ink-2"))
                                            .line_clamp(2)
                                            .child(b)
                                    }),
                            )
                            .child(status_line(
                                &agent,
                                StatusStyle {
                                    size: 13.,
                                    color: theme.c("ink-3"),
                                    time_right: false,
                                    show_time: true,
                                },
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .id("hb-live-chat")
                            .ml_auto()
                            .flex_none()
                            .h(px(40.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.c("line-strong"))
                            .bg(theme.c("surface"))
                            .font_weight(FontWeight::BOLD)
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.c("btn-hover")))
                            .child(icon("message-circle", 18., theme.c("ink")))
                            .child(t(cx, "agents.openConversation"))
                            .on_click(move |_, _, cx| {
                                nav::replace(Route::direct(Some(key.clone())), cx)
                            }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .mx(px(24.))
                    .mb(px(20.))
                    .pt(px(12.))
                    .px(px(14.))
                    .pb(px(8.))
                    .border_1()
                    .border_color(theme.c("line-strong"))
                    .rounded(px(14.))
                    .bg(theme.c("surface"))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(14.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(theme.c("accent-soft"))
                            .text_color(theme.c("ink-2"))
                            .text_size(px(15.))
                            .child(icon("lock", 16., theme.c("ink-2")))
                            .child(private),
                    )
                    .child(
                        div()
                            .flex_none()
                            .mt(px(14.))
                            .mb(px(6.))
                            .pl(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .text_size(px(26.))
                            .font_weight(FontWeight::BLACK)
                            .text_color(if !live {
                                theme.c("ink-2")
                            } else if paused {
                                theme.c("ink-3")
                            } else {
                                theme.c("ink")
                            })
                            .child(if live {
                                let dot = div().size(px(18.)).rounded_full().bg(if dot_paused {
                                    theme.c("grey")
                                } else {
                                    theme.c("red")
                                });
                                if dot_paused {
                                    dot.into_any_element()
                                } else {
                                    dot.border_4()
                                        .border_color(theme.c("red-bg"))
                                        .with_animation(
                                            "hb-live-pulse",
                                            Animation::new(Duration::from_millis(1600)).repeat(),
                                            |d, delta| {
                                                d.opacity(
                                                    1.0 - 0.45
                                                        * (delta * std::f32::consts::PI).sin(),
                                                )
                                            },
                                        )
                                        .into_any_element()
                                }
                            } else {
                                div()
                                    .size(px(22.))
                                    .rounded_full()
                                    .bg(theme.c("grey"))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon("check", 16., theme.c("surface")))
                                    .into_any_element()
                            })
                            .child(heading),
                    )
                    .child(if !answered {
                        crate::screens::inbox::empty(
                            None,
                            Some(tv(cx, "agents.liveReading", &[("name", &name)])),
                            &theme,
                        )
                        .into_any_element()
                    } else if count == 0 {
                        crate::screens::inbox::empty(
                            None,
                            Some(tv(cx, "agents.liveNothingYet", &[("name", &name)])),
                            &theme,
                        )
                        .into_any_element()
                    } else {
                        timeline.into_any_element()
                    })
                    .child(
                        div()
                            .flex_none()
                            .pt(px(8.))
                            .px(px(6.))
                            .pb(px(4.))
                            .border_t_1()
                            .border_color(theme.c("line"))
                            .text_size(px(13.))
                            .text_color(theme.c("ink-4"))
                            .child(t(cx, "agents.liveFoot")),
                    ),
            )
            .into_any_element()
    }
}
