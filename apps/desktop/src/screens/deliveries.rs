//! Deliveries (`/deliveries`, the console's `DeliveriesView`): every DONE an
//! agent sent and how far its commit has got — delivered, in review, with
//! fixes, integrated, installed — then the old integration queue.

use super::secondary::{self as kit, head, main_column, monogram_avatar, sync_list};
use crate::derive::deliveries::{
    Installed, app_commit, installed, state_label, steps, times_line, untracked_text, verdict_text,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{DeliveryRow, Shipment, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, format::hhmm, size};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, BoxShadow, Context, FontWeight, InteractiveElement, IntoElement,
    ListAlignment, ListState, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, list, point, px,
};
use std::sync::Arc;

/// One row of the virtual list.
#[derive(Clone)]
enum Row {
    Installed(Installed),
    Empty,
    Heading(SharedString),
    Ship(usize),
    QueueHead(usize),
    Queue(usize),
    Bottom,
}

pub struct DeliveriesScreen {
    list: ListState,
    rows: Vec<Row>,
    view: Option<Arc<WorkspaceView>>,
}

impl DeliveriesScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this, _, cx| this.refresh(cx))
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<crate::i18n::I18n>(|this, cx| this.refresh(cx))
            .detach();
        let mut this = DeliveriesScreen {
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            rows: Vec::new(),
            view: None,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let view = Desk::get(cx).read(cx).view.clone();
        self.rows = match &view {
            Some(v) => rows(v, cx),
            None => Vec::new(),
        };
        self.view = view;
        sync_list(&self.list, self.rows.len());
        cx.notify();
    }

    fn render_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(view), Some(row)) = (self.view.clone(), self.rows.get(ix).cloned()) else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        match row {
            Row::Installed(line) => installed_line(&line, &theme, cx),
            Row::Empty => kit::empty_block(
                Some(t(cx, "deliveries.emptyTitle")),
                vec![t(cx, "deliveries.emptyHint").into_any_element()],
                &theme,
            )
            .into_any_element(),
            Row::Heading(text) => section_head(text, &theme).into_any_element(),
            Row::Ship(i) => ship_row(&view, &view.shipments[i], &theme, cx),
            Row::QueueHead(n) => div()
                .pt(px(8.))
                .child(section_head(
                    tv(cx, "deliveries.queueTitle", &[("count", &n.to_string())]),
                    &theme,
                ))
                .child(
                    div()
                        .px(px(20.))
                        .pb(px(8.))
                        .text_size(px(size::UI))
                        .text_color(theme.c("ink-3"))
                        .child(t(cx, "deliveries.queueLine")),
                )
                .into_any_element(),
            Row::Queue(i) => queue_row(&view.deliveries[i], &theme, cx),
            Row::Bottom => div().h(px(12.)).into_any_element(),
        }
    }
}

/// Installed line, then the pending section, the rest, the queue.
fn rows(view: &WorkspaceView, cx: &App) -> Vec<Row> {
    let all = &view.shipments;
    let installs = all.iter().any(|s| s.last_state == "installed");
    let mut out = vec![];
    if installs {
        out.push(Row::Installed(installed(
            view.running_commit.as_deref(),
            app_commit(),
        )));
    }
    if all.is_empty() {
        out.push(Row::Empty);
    }
    let pending: Vec<usize> = (0..all.len()).filter(|&i| all[i].pending).collect();
    let rest: Vec<usize> = (0..all.len()).filter(|&i| !all[i].pending).collect();
    if !pending.is_empty() {
        let key = if installs {
            "deliveries.pendingTitle"
        } else {
            "deliveries.pendingTitleBranch"
        };
        out.push(Row::Heading(tv(
            cx,
            key,
            &[("count", &pending.len().to_string())],
        )));
        out.extend(pending.into_iter().map(Row::Ship));
    }
    if !rest.is_empty() {
        out.push(Row::Heading(tv(
            cx,
            "deliveries.restTitle",
            &[("count", &rest.len().to_string())],
        )));
        out.extend(rest.into_iter().map(Row::Ship));
    }
    if !view.deliveries.is_empty() {
        out.push(Row::QueueHead(view.deliveries.len()));
        out.extend((0..view.deliveries.len()).map(Row::Queue));
    }
    out.push(Row::Bottom);
    out
}

fn section_head(text: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .pt(px(20.))
        .px(px(20.))
        .pb(px(6.))
        .text_size(px(15.))
        .font_weight(FontWeight::BLACK)
        .text_color(theme.c("ink-2"))
        .child(text)
}

fn installed_line(line: &Installed, theme: &Theme, cx: &App) -> AnyElement {
    let (bg, ink, edge, icon) = if line.mismatch() {
        (
            theme.c("warn-bg"),
            theme.c("warn-ink"),
            theme.c("warn-line"),
            "triangle-alert",
        )
    } else {
        (
            theme.c("ok-bg"),
            theme.c("ok-ink"),
            theme.c("ok-line"),
            "circle-check",
        )
    };
    div()
        .pt(px(12.))
        .px(px(20.))
        .pb(px(4.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(bg)
                .border_1()
                .border_color(edge)
                .text_color(ink)
                .text_size(px(size::UI))
                .child(ui::icon(icon, 16., ink))
                .child(line.text(cx)),
        )
        .into_any_element()
}

fn chip_colors(state: &str, theme: &Theme) -> (gpui::Hsla, gpui::Hsla) {
    match state {
        "in_review" => (theme.c("amber-bg"), theme.c("amber-ink")),
        "with_fixes" => (theme.c("red-bg"), theme.c("red-ink")),
        "integrated" => (theme.c("accent-soft"), theme.c("accent-text")),
        "installed" => (theme.c("green-bg"), theme.c("green-ink")),
        _ => (theme.c("chip"), theme.c("chip-ink")),
    }
}

/// One delivery: who, the human line, where it is, when each step happened.
fn ship_row(view: &WorkspaceView, s: &Shipment, theme: &Theme, cx: &App) -> AnyElement {
    let agent = view.agent(&s.agent);
    let short_key: String = s.agent.chars().take(8).collect();
    let name: SharedString = agent.map(|a| a.name.clone()).unwrap_or(short_key).into();
    let avatar = match agent {
        Some(a) => ui::avatar::agent_avatar(a, 40., true, cx),
        None => monogram_avatar(&name, 40., theme).into_any_element(),
    };
    let (chip_bg, chip_ink) = chip_colors(&s.state, theme);
    let sha = s.sha.as_deref().map(|sha| {
        let mut text: String = sha.chars().take(7).collect();
        if let Some(landed) = s.landed_as.as_deref() {
            text.push_str(" → ");
            text.extend(landed.chars().take(7));
        }
        kit::mono(text).text_color(theme.c("ink-3"))
    });
    let why = untracked_text(s.untracked.as_deref(), cx);
    let target = s.agent.clone();
    let hover = theme.c("hover");
    div()
        .id(SharedString::from(format!("ship-{}", s.id)))
        .w_full()
        .flex()
        .gap(px(12.))
        .px(px(20.))
        .py(px(10.))
        .cursor_pointer()
        .hover(move |st| st.bg(hover))
        .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(target.clone())), cx))
        .child(avatar)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::BLACK)
                                .text_size(px(15.))
                                .child(name),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(chip_bg)
                                .text_color(chip_ink)
                                .text_size(px(size::META))
                                .font_weight(FontWeight::BOLD)
                                .child(state_label(&s.state, cx)),
                        )
                        .when_some(s.channel.as_deref(), |d, ch| {
                            d.child(
                                div()
                                    .text_color(theme.c("ink-3"))
                                    .text_size(px(size::SMALL))
                                    .child(format!("#{ch}")),
                            )
                        })
                        .children(sha),
                )
                .child(
                    div()
                        .text_size(px(15.))
                        .text_color(theme.c("ink"))
                        .child(s.text.clone()),
                )
                .child(
                    div()
                        .text_size(px(size::SMALL))
                        .text_color(theme.c("ink-3"))
                        .child(times_line(s, view.now_ms, cx)),
                )
                .child(match why {
                    Some(text) => div()
                        .text_size(px(size::SMALL))
                        .text_color(theme.c("ink-3"))
                        .child(text)
                        .into_any_element(),
                    None => step_track(s, theme, cx),
                }),
        )
        .into_any_element()
}

/// The steps a delivery went through, the current one marked.
fn step_track(s: &Shipment, theme: &Theme, cx: &App) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .mt(px(4.))
        .text_size(px(size::META))
        .text_color(theme.c("ink-4"));
    for (i, step) in steps(s).into_iter().enumerate() {
        if i > 0 {
            row = row.child(
                div()
                    .w(px(18.))
                    .h(px(2.))
                    .mx(px(6.))
                    .bg(theme.c("line-strong")),
            );
        }
        let mut dot = div().size(px(9.)).rounded_full().border_2().flex_none();
        dot = if step.reached {
            dot.border_color(theme.c("accent")).bg(theme.c("accent"))
        } else {
            dot.border_color(theme.c("line-strong"))
                .bg(theme.c("surface"))
        };
        if step.current {
            dot = dot.shadow(vec![BoxShadow {
                color: theme.c("accent-soft"),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(3.),
                inset: false,
            }]);
        }
        let ink = if step.current {
            theme.c("ink")
        } else if step.reached {
            theme.c("ink-2")
        } else {
            theme.c("ink-4")
        };
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .text_color(ink)
                .when(step.current, |d| d.font_weight(FontWeight::BLACK))
                .child(dot)
                .child(state_label(step.id, cx)),
        );
    }
    row.into_any_element()
}

/// One delivery of the old `harbard deliver` / `integrate` queue.
fn queue_row(d: &DeliveryRow, theme: &Theme, cx: &App) -> AnyElement {
    let who = d.channel.clone().unwrap_or_else(|| "harbard".into());
    let name: SharedString = match &d.channel {
        Some(ch) => format!("#{ch}").into(),
        None => t(cx, "deliveries.repositoryNoChannel"),
    };
    let sha: String = d.sha.chars().take(10).collect();
    let verdict = verdict_text(&d.verdict, cx);
    let template = match d.decided_ms {
        Some(_) => t(cx, "deliveries.deliveryLineAt"),
        None => t(cx, "deliveries.deliveryLine"),
    };
    let time = d.decided_ms.map(hhmm).unwrap_or_default();
    let text = rich_line(&template, &sha, &d.branch, &verdict, &time);
    div()
        .flex()
        .gap(px(10.))
        .px(px(20.))
        .py(px(8.))
        .child(monogram_avatar(&who, 36., theme))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_weight(FontWeight::BLACK)
                                .text_size(px(15.))
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(size::META))
                                .text_color(theme.c("ink-3"))
                                .child(hhmm(d.created_ms)),
                        ),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .children(text),
                )
                .when(d.verdict == "landed", |el| {
                    el.child(
                        div().flex().child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .h(px(24.))
                                .px(px(8.))
                                .mt(px(6.))
                                .rounded(px(12.))
                                .bg(theme.c("ok-bg"))
                                .text_color(theme.c("ok-ink"))
                                .border_1()
                                .border_color(theme.c("ok-line"))
                                .text_size(px(size::META))
                                .font_weight(FontWeight::BOLD)
                                .child(ui::icon("circle-check", 14., theme.c("ok-ink")))
                                .child(t(cx, "deliveries.verified")),
                        ),
                    )
                }),
        )
        .into_any_element()
}

/// "{sha} to {branch}: {verdict}." with the SHA and branch in mono, word
/// order kept from the dictionary.
fn rich_line(
    template: &str,
    sha: &str,
    branch: &str,
    verdict: &str,
    time: &str,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            out.push(div().child(rest[..open].to_string()).into_any_element());
        }
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let name = &rest[open + 1..open + close];
        out.push(match name {
            "sha" => kit::mono(sha.to_string()).into_any_element(),
            "branch" => kit::mono(branch.to_string()).into_any_element(),
            "verdict" => div().child(verdict.to_string()).into_any_element(),
            "time" => div().child(time.to_string()).into_any_element(),
            other => div().child(format!("{{{other}}}")).into_any_element(),
        });
        rest = &rest[open + close + 1..];
    }
    if !rest.is_empty() {
        out.push(div().child(rest.to_string()).into_any_element());
    }
    out
}

impl Render for DeliveriesScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        main_column(&theme)
            .child(head(
                "package-check",
                t(cx, "deliveries.title"),
                Some(t(cx, "deliveries.line")),
                &theme,
            ))
            .child(
                div().flex_1().min_h_0().pt(px(8.)).child(
                    list(
                        self.list.clone(),
                        cx.processor(|this: &mut Self, ix, window, cx| {
                            super::secondary::full_row(this.render_row(ix, window, cx))
                        }),
                    )
                    .size_full(),
                ),
            )
    }
}
