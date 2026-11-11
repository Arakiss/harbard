//! Deliveries (`/deliveries`, the console's `DeliveriesView`): every DONE an
//! agent sent and how far its commit has got — delivered, in review, with
//! fixes, integrated, installed — then the old integration queue.

use super::secondary::{self as kit, head, main_column, monogram_avatar, sync_list};
use crate::derive::deliveries::{
    Installed, app_commit, delivered_when, installed, review_times, state_label, steps,
    untracked_text, verdict_text,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{DeliveryRow, Shipment, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{
    self, avatar_size, format::hhmm, icon_size, leading, pill_chat as pill, radius, size, space,
    weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, BoxShadow, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, list, point, px,
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
    focus: FocusHandle,
    /// The row the keyboard is on (K4), as an index into `rows`.
    cursor: Option<usize>,
}

impl DeliveriesScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this, _, cx| this.refresh(cx))
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<crate::i18n::I18n>(|this, cx| this.refresh(cx))
            .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = DeliveriesScreen {
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            rows: Vec::new(),
            view: None,
            focus,
            cursor: None,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let view = Desk::get(cx).read(cx).view.clone();
        // The keyboard stays on the same delivery when the list changes.
        let held = self.cursor.and_then(|ix| match self.rows.get(ix) {
            Some(Row::Ship(i)) => self
                .view
                .as_ref()
                .and_then(|v| v.shipments.get(*i))
                .map(|s| s.id.clone()),
            _ => None,
        });
        self.rows = match &view {
            Some(v) => rows(v, cx),
            None => Vec::new(),
        };
        self.cursor = held.and_then(|id| {
            let v = view.as_ref()?;
            self.rows
                .iter()
                .position(|r| matches!(r, Row::Ship(i) if v.shipments[*i].id == id))
        });
        self.view = view;
        sync_list(&self.list, self.rows.len());
        cx.notify();
    }

    /// `J`/`K` and the arrows move the focus row, `Enter` opens it, `Esc`
    /// lets go (K4).
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.modifiers.modified() {
            return;
        }
        let forward = match ev.keystroke.key.as_str() {
            "j" | "down" => true,
            "k" | "up" => false,
            "enter" if self.focus.is_focused(window) => {
                if let (Some(view), Some(Row::Ship(i))) =
                    (&self.view, self.cursor.and_then(|ix| self.rows.get(ix)))
                {
                    let agent = view.shipments[*i].agent.clone();
                    nav::navigate(Route::direct(Some(agent)), cx);
                }
                return;
            }
            "escape" if self.cursor.is_some() => {
                self.cursor = None;
                cx.notify();
                return;
            }
            _ => return,
        };
        let is_ship = |ix: &usize| matches!(self.rows.get(*ix), Some(Row::Ship(_)));
        let next = match (self.cursor, forward) {
            (None, _) => (0..self.rows.len()).find(is_ship),
            (Some(at), true) => (at + 1..self.rows.len()).find(is_ship),
            (Some(at), false) => (0..at).rev().find(is_ship),
        };
        if let Some(ix) = next {
            self.cursor = Some(ix);
            self.list.scroll_to_reveal_item(ix);
            // A row reached with Tab gives way to the focus row.
            self.focus.focus(window, cx);
            cx.notify();
        }
        cx.stop_propagation();
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
            Row::Ship(i) => ship_row(
                &view,
                &view.shipments[i],
                self.cursor == Some(ix),
                &theme,
                cx,
            ),
            Row::QueueHead(n) => div()
                .child(section_head(
                    tv(cx, "deliveries.queueTitle", &[("count", &n.to_string())]),
                    &theme,
                ))
                .child(
                    ui::text(
                        t(cx, "deliveries.queueLine"),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .px(px(space::GUTTER))
                    .pb(px(space::XS)),
                )
                .into_any_element(),
            Row::Queue(i) => queue_row(&view.deliveries[i], &theme, cx),
            Row::Bottom => div().h(px(space::LG)).into_any_element(),
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

/// `.hb-ship-h`: the section's name in 15/700 `ink-2` (§3.1).
fn section_head(text: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .pt(px(space::SECTION))
        .px(px(space::GUTTER))
        .pb(px(space::SM))
        .child(ui::text(text, size::BODY, weight::STRONG, theme.c("ink-2")))
}

/// What this Mac runs: a quiet banner, amber only when the app and serve
/// disagree.
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
        .pt(px(space::XL))
        .px(px(space::GUTTER))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .px(px(space::LG))
                .py(px(space::MD))
                .rounded(px(radius::MD))
                .border_1()
                .border_color(edge)
                .bg(bg)
                .child(ui::icon(icon, icon_size::ROW, ink))
                .child(ui::text(line.text(cx), size::UI, weight::REGULAR, ink).min_w_0()),
        )
        .into_any_element()
}

/// One delivery: who and when, the human line, when it was reviewed, where
/// it is. The chip names where it is in its state's colour (`.hb-ship-chip`,
/// §3.1); the steps below show the way there.
fn ship_row(
    view: &WorkspaceView,
    s: &Shipment,
    focused: bool,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let agent = view.agent(&s.agent);
    let short_key: String = s.agent.chars().take(8).collect();
    let name: SharedString = agent.map(|a| a.name.clone()).unwrap_or(short_key).into();
    let avatar = match agent {
        Some(a) => ui::avatar::agent_avatar(a, avatar_size::ROW, true, cx),
        None => monogram_avatar(&name, theme).into_any_element(),
    };
    let sha = s.sha.as_deref().map(|sha| {
        let mut text: String = sha.chars().take(7).collect();
        if let Some(landed) = s.landed_as.as_deref() {
            text.push_str(" → ");
            text.extend(landed.chars().take(7));
        }
        kit::mono(text).flex_none().text_color(theme.c("ink-3"))
    });
    let why = untracked_text(s.untracked.as_deref(), cx);
    let target = s.agent.clone();
    let hover = theme.c("hover");
    kit::row_focus(
        div().id(SharedString::from(format!("ship-{}", s.id))),
        theme,
    )
    .w_full()
    .flex()
    .gap(px(space::LG))
    .px(px(space::GUTTER))
    .py(px(space::LG))
    .cursor_pointer()
    .hover(move |st| st.bg(hover))
    .when(focused, |d| d.shadow(vec![ui::focus_ring(theme, true)]))
    .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(target.clone())), cx))
    .child(avatar)
    .child(
        div()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .min_w_0()
                    .child(
                        ui::text(name, size::BODY, weight::HEAVY, theme.c("ink"))
                            .min_w_0()
                            .truncate(),
                    )
                    .child(pill::ship_chip(state_label(&s.state, cx), &s.state, theme))
                    .when_some(s.channel.as_deref(), |d, ch| {
                        d.child(
                            ui::text(
                                format!("#{ch}"),
                                size::SMALL,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )
                            .flex_none(),
                        )
                    })
                    .children(sha)
                    .child(
                        ui::text(
                            delivered_when(s, view.now_ms, cx),
                            size::META,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .ml_auto()
                        .flex_none(),
                    ),
            )
            .child(
                ui::text(s.text.clone(), size::BODY, weight::REGULAR, theme.c("ink"))
                    .line_clamp(TEXT_LINES)
                    .text_ellipsis(),
            )
            .children(review_times(s, cx).map(|times| {
                ui::text(times, size::SMALL, weight::REGULAR, theme.c("ink-3")).truncate()
            }))
            .child(match why {
                Some(text) => ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                    .into_any_element(),
                None => step_track(s, theme, cx),
            }),
    )
    .into_any_element()
}

/// A delivery's own line shows this many lines; the rest is in its DM.
const TEXT_LINES: usize = 3;
/// `.hb-ship-steps li + li::before`: 18 by 2 px, 6 px each side.
const LINK: (f32, f32) = (18., 2.);
/// `.hb-ship-dot`: 9 px with a 2 px edge.
const DOT: (f32, f32) = (9., 2.);
/// `.hb-ship-steps li { gap: 5px }`.
const DOT_GAP: f32 = 5.;
/// The current step's halo: `box-shadow: 0 0 0 3px var(--hb-accent-soft)`.
const HALO: f32 = 3.;

/// How a step looks: reached steps are filled with the accent, the current
/// one also wears a halo and bold ink, what is ahead is an empty ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepLook {
    Ahead,
    Reached,
    Current,
}

pub(crate) fn step_look(reached: bool, current: bool) -> StepLook {
    match (reached, current) {
        (_, true) => StepLook::Current,
        (true, false) => StepLook::Reached,
        (false, false) => StepLook::Ahead,
    }
}

/// The steps a delivery went through (`.hb-ship-steps`).
fn step_track(s: &Shipment, theme: &Theme, cx: &App) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_y(px(space::XS))
        .mt(px(space::XS));
    let accent = theme.c("accent");
    for (i, step) in steps(s).into_iter().enumerate() {
        if i > 0 {
            row = row.child(
                div()
                    .w(px(LINK.0))
                    .h(px(LINK.1))
                    .mx(px(space::SM))
                    .bg(theme.c("line-strong")),
            );
        }
        let look = step_look(step.reached, step.current);
        let dot = div()
            .size(px(DOT.0))
            .rounded_full()
            .flex_none()
            .border(px(DOT.1));
        let dot = match look {
            StepLook::Ahead => dot
                .border_color(theme.c("line-strong"))
                .bg(theme.c("surface")),
            StepLook::Reached => dot.border_color(accent).bg(accent),
            StepLook::Current => dot.border_color(accent).bg(accent).shadow(vec![BoxShadow {
                color: theme.c("accent-soft"),
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(HALO),
                inset: false,
            }]),
        };
        let (ink, heft) = match look {
            StepLook::Current => (theme.c("ink"), weight::STRONG),
            StepLook::Reached => (theme.c("ink-2"), weight::REGULAR),
            StepLook::Ahead => (theme.c("ink-4"), weight::REGULAR),
        };
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(DOT_GAP))
                .child(dot)
                .child(ui::text(state_label(step.id, cx), size::META, heft, ink)),
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
        .gap(px(space::LG))
        .px(px(space::GUTTER))
        .py(px(space::LG))
        .child(monogram_avatar(&who, theme))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::HAIR))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .child(
                            ui::text(name, size::UI, weight::STRONG, theme.c("ink"))
                                .min_w_0()
                                .truncate(),
                        )
                        .when(d.verdict == "landed", |el| {
                            el.child(kit::chip(t(cx, "deliveries.verified"), "green", theme))
                        })
                        .child(
                            ui::text(
                                hhmm(d.created_ms),
                                size::META,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )
                            .flex_none(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .text_size(px(size::UI))
                        .line_height(px(leading::UI))
                        .text_color(theme.c("ink-2"))
                        .children(text),
                ),
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
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(head(
                t(cx, "deliveries.title"),
                Some(t(cx, "deliveries.line")),
                &theme,
            ))
            .child(
                div().flex_1().min_h_0().child(
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

#[cfg(test)]
mod look_tests {
    use super::*;

    #[test]
    fn the_current_step_wins_over_reached() {
        assert_eq!(step_look(true, true), StepLook::Current);
        assert_eq!(step_look(false, true), StepLook::Current);
        assert_eq!(step_look(true, false), StepLook::Reached);
        assert_eq!(step_look(false, false), StepLook::Ahead);
    }
}
