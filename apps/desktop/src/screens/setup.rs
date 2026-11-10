//! Set up harbard (`/setup`): the first-run wizard. It finds Claude Code,
//! Codex and Cursor on this Mac (`GET /setup`), lets the operator choose
//! which ones harbard connects (`POST /setup/install`), and records the
//! choice (`POST /setup/complete`). When it finishes the app goes Home.
//!
//! An agent only hears the operator's notes through a hook, so a partial
//! connection never celebrates: the ones that failed stay on screen.

use super::secondary::{self as kit, button, chip, head_bar, main_column};
use crate::i18n::{I18n, t, tv};
use crate::model::setup::{SetupHarness, SetupInstallResponse, SetupInstallResult, SetupStatus};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, size};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::Duration;

/// The board must appear even when serve is dead or half-open.
const GATE_CAP: Duration = Duration::from_millis(2500);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Scan,
    Choose,
    Wire,
    Done,
}

pub struct SetupScreen {
    opening: bool,
    step: Step,
    harnesses: Vec<SetupHarness>,
    selected: BTreeSet<String>,
    busy: bool,
    error: Option<SharedString>,
    results: Option<Vec<SetupInstallResult>>,
    scan_tick: u8,
    _tasks: Vec<Task<()>>,
}

/// "Claude Code", "Codex", "Cursor"; anything else as it came.
fn harness_label(h: &str, cx: &App) -> SharedString {
    match h {
        "claude-code" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "cursor" => "Cursor".into(),
        "human" => t(cx, "workspace.harnessHuman"),
        other => other.to_string().into(),
    }
}

/// serve describes each tool in English; the wizard words it by its id.
fn blurb_for(target: &str, fallback: &str, cx: &App) -> SharedString {
    match target {
        "claude" => t(cx, "setup.blurbClaude"),
        "codex" => t(cx, "setup.blurbCodex"),
        "cursor" => t(cx, "setup.blurbCursor"),
        _ => fallback.to_string().into(),
    }
}

/// The list shown when serve does not answer: nothing found, nothing wired.
fn fallback(cx: &App) -> Vec<SetupHarness> {
    [
        ("claude", "claude-code", "~/.claude/settings.json"),
        ("codex", "codex", "~/.codex/hooks.json"),
        ("cursor", "cursor", "~/.cursor/hooks.json"),
    ]
    .into_iter()
    .map(|(target, harness, path)| SetupHarness {
        target: target.into(),
        harness: harness.into(),
        path: path.into(),
        hint: Some(format!("harbard install {target}")),
        blurb: blurb_for(target, "", cx).to_string(),
        ..Default::default()
    })
    .collect()
}

/// Every recommended harness, and (this route always forces the wizard)
/// every one not wired yet.
fn initial_selection(harnesses: &[SetupHarness]) -> BTreeSet<String> {
    harnesses
        .iter()
        .filter(|h| h.recommended || !h.ok)
        .map(|h| h.target.clone())
        .collect()
}

impl SetupScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<I18n>(|_, cx| cx.notify()).detach();
        let mut this = SetupScreen {
            opening: true,
            step: Step::Welcome,
            harnesses: Vec::new(),
            selected: BTreeSet::new(),
            busy: false,
            error: None,
            results: None,
            scan_tick: 0,
            _tasks: Vec::new(),
        };
        this.open(cx);
        this
    }

    /// The gate: serve's status, or the fixed list after 2.5 s.
    fn open(&mut self, cx: &mut Context<Self>) {
        let read = kit::fetch::<SetupStatus>("/setup".into(), cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let status = read.await;
            let _ = this.update(cx, |this, cx| {
                if !this.opening {
                    return;
                }
                this.opening = false;
                this.harnesses = match status {
                    Ok(s) => s.harnesses,
                    Err(_) => fallback(cx),
                };
                this.selected = initial_selection(&this.harnesses);
                cx.notify();
            });
        }));
        let cap = cx.background_executor().timer(GATE_CAP);
        self._tasks.push(cx.spawn(async move |this, cx| {
            cap.await;
            let _ = this.update(cx, |this, cx| {
                if this.opening {
                    this.opening = false;
                    this.harnesses = fallback(cx);
                    this.selected = initial_selection(&this.harnesses);
                    cx.notify();
                }
            });
        }));
    }

    /// Search again, revealing each row as the request runs.
    fn scan(&mut self, cx: &mut Context<Self>) {
        self.step = Step::Scan;
        self.scan_tick = 0;
        self.error = None;
        for (ms, tick) in [(120, 1), (240, 2)] {
            let timer = cx.background_executor().timer(Duration::from_millis(ms));
            self._tasks.push(cx.spawn(async move |this, cx| {
                timer.await;
                let _ = this.update(cx, |this, cx| {
                    if this.step == Step::Scan && this.scan_tick < tick {
                        this.scan_tick = tick;
                        cx.notify();
                    }
                });
            }));
        }
        let read = kit::fetch::<SetupStatus>("/setup".into(), cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = read.await;
            let _ = this.update(cx, |this, cx| {
                if this.step != Step::Scan {
                    return;
                }
                match out {
                    Ok(s) => {
                        this.scan_tick = 3;
                        this.harnesses = s.harnesses;
                        this.selected = initial_selection(&this.harnesses);
                        this.step = Step::Choose;
                    }
                    Err(e) => {
                        this.error = Some(if e.message.is_empty() {
                            t(cx, "setup.errorScan")
                        } else {
                            e.message.into()
                        });
                        this.step = Step::Welcome;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// `POST /setup/complete`, then "All set" and Home.
    fn finish(&mut self, skipped: Vec<String>, wired: Vec<String>, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        let send = kit::send(
            "/setup/complete".into(),
            json!({ "skipped": skipped, "wired": wired }),
            cx,
        );
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = send.await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(_) => {
                        this.step = Step::Done;
                        this.leave_after(Duration::from_millis(900), cx);
                    }
                    Err(e) => {
                        this.error = Some(error_text(&e, "setup.errorSave", cx));
                        this.busy = false;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn leave_after(&mut self, wait: Duration, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(wait);
        self._tasks.push(cx.spawn(async move |_, cx| {
            timer.await;
            cx.update(|cx| nav::replace(Route::home(), cx));
        }));
    }

    fn skip_all(&mut self, cx: &mut Context<Self>) {
        let all = self.harnesses.iter().map(|h| h.target.clone()).collect();
        self.finish(all, Vec::new(), cx);
    }

    /// Connect the chosen ones; nothing chosen is "Not now".
    fn wire(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<String> = self.selected.iter().cloned().collect();
        if targets.is_empty() {
            self.skip_all(cx);
            return;
        }
        self.busy = true;
        self.error = None;
        self.step = Step::Wire;
        let send = kit::send("/setup/install".into(), json!({ "targets": targets }), cx);
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = send.await;
            let _ = this.update(cx, |this, cx| {
                let answer: Result<SetupInstallResponse, SharedString> = match out {
                    Ok(v) => serde_json::from_value(v).map_err(|e| e.to_string().into()),
                    Err(e) => Err(error_text(&e, "setup.errorWire", cx)),
                };
                match answer {
                    Ok(res) => {
                        let all: Vec<String> = this.harnesses.iter().map(|h| h.target.clone()).collect();
                        this.harnesses = res.setup.harnesses;
                        let wired: Vec<String> =
                            res.results.iter().filter(|r| r.ok).map(|r| r.target.clone()).collect();
                        let failed: Vec<&SetupInstallResult> = res.results.iter().filter(|r| !r.ok).collect();
                        // A partial connection must not celebrate.
                        if let Some(first) = failed.first() {
                            this.error = Some(if failed.len() == 1 {
                                tv(cx, "setup.errorWireOne", &[("tool", &harness_label(&first.target, cx))])
                            } else {
                                tv(cx, "setup.errorWireMany", &[("count", &failed.len().to_string())])
                            });
                            this.results = Some(res.results);
                            this.busy = false;
                        } else {
                            this.results = Some(res.results);
                            let skipped = all.into_iter().filter(|t| !wired.contains(t)).collect();
                            this.complete_after_wire(skipped, wired, cx);
                        }
                    }
                    Err(e) => {
                        this.error = Some(e);
                        this.busy = false;
                        this.step = Step::Choose;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn complete_after_wire(&mut self, skipped: Vec<String>, wired: Vec<String>, cx: &mut Context<Self>) {
        let send = kit::send(
            "/setup/complete".into(),
            json!({ "skipped": skipped, "wired": wired }),
            cx,
        );
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = send.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match out {
                    Ok(_) => {
                        this.step = Step::Done;
                        this.leave_after(Duration::from_millis(1100), cx);
                    }
                    Err(e) => {
                        this.error = Some(error_text(&e, "setup.errorWire", cx));
                        this.step = Step::Choose;
                    }
                }
                cx.notify();
            });
        }));
    }
}

/// serve's error, or the bus being unreachable, or the step's fallback.
fn error_text(e: &crate::api::ApiError, fallback: &str, cx: &App) -> SharedString {
    if e.unreachable() {
        return t(cx, fallback);
    }
    if e.message.is_empty() {
        t(cx, fallback)
    } else {
        e.message.clone().into()
    }
}

fn card(center: bool, theme: &Theme) -> gpui::Div {
    div()
        .w_full()
        .max_w(px(640.))
        .mx_auto()
        .px(px(36.))
        .py(px(32.))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(14.))
        .bg(theme.c("surface-2"))
        .when(center, |d| d.flex().flex_col().items_center().text_center())
}

fn title(text: SharedString) -> gpui::Div {
    div()
        .text_size(px(24.))
        .line_height(px(30.))
        .font_weight(FontWeight::BLACK)
        .child(text)
}

fn note(text: SharedString, theme: &Theme) -> gpui::Div {
    div()
        .max_w(px(420.))
        .mt(px(8.))
        .text_size(px(15.))
        .line_height(px(22.))
        .text_color(theme.c("ink-3"))
        .child(text)
}

fn error_line(text: Option<SharedString>, theme: &Theme) -> Option<gpui::Div> {
    text.map(|e| {
        div()
            .mt(px(8.))
            .text_size(px(size::SMALL))
            .text_color(theme.c("red-ink"))
            .child(e)
    })
}

fn row(name: SharedString, status: AnyElement, revealed: bool, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(14.))
        .py(px(10.))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(10.))
        .bg(theme.c("surface"))
        .when(!revealed, |d| d.opacity(0.))
        .child(div().font_weight(FontWeight::BOLD).child(name))
        .child(status)
}

/// The harbard mark on its tile.
fn brand_mark(theme: &Theme) -> impl IntoElement {
    div()
        .relative()
        .size(px(88.))
        .mb(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(22.))
        .bg(theme.c("ws-tile"))
        .text_color(theme.c("rail-ink"))
        .font_family(ui::DISPLAY)
        .text_size(px(50.))
        .font_weight(FontWeight::BLACK)
        .child("h")
        .child(
            div()
                .absolute()
                .top(px(18.))
                .right(px(18.))
                .size(px(12.))
                .rounded_full()
                .bg(theme.c("green")),
        )
}

impl SetupScreen {
    fn harness_card(&self, h: &SetupHarness, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.contains(&h.target);
        let (word, tone) = if h.ok {
            (t(cx, "setup.connected"), "green")
        } else if h.present {
            (t(cx, "setup.detected"), "amber")
        } else {
            (t(cx, "setup.missing"), "neutral")
        };
        let target = h.target.clone();
        let accent = theme.c("accent");
        let check = div()
            .flex_none()
            .mt(px(3.))
            .size(px(18.))
            .rounded(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .map(|d| {
                if selected {
                    d.bg(accent).child(ui::icon("check", 14., theme.c("accent-ink")))
                } else {
                    d.border_1().border_color(theme.c("line-strong")).bg(theme.c("surface"))
                }
            });
        div()
            .id(SharedString::from(format!("setup-{}", h.target)))
            .min_w_0()
            .flex()
            .items_start()
            .gap(px(12.))
            .px(px(16.))
            .py(px(14.))
            .rounded(px(12.))
            .bg(theme.c("surface"))
            .cursor_pointer()
            .border_1()
            .map(|d| {
                if selected {
                    // `box-shadow: 0 0 0 1px accent` beside the border: no shift.
                    d.border_color(accent).shadow(vec![gpui::BoxShadow {
                        color: accent,
                        offset: gpui::point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(1.),
                        inset: false,
                    }])
                } else {
                    d.border_color(theme.c("line-strong"))
                }
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.selected.remove(&target) {
                    this.selected.insert(target.clone());
                }
                cx.notify();
            }))
            .child(check)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(8.))
                            .child(div().font_weight(FontWeight::BOLD).child(harness_label(&h.harness, cx)))
                            .child(chip(word, tone, theme)),
                    )
                    .child(
                        div()
                            .text_size(px(size::UI))
                            .text_color(theme.c("ink-2"))
                            .child(blurb_for(&h.target, &h.blurb, cx)),
                    )
                    .child(
                        kit::mono(h.path.clone())
                            .text_color(theme.c("ink-3"))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap(),
                    ),
            )
            .into_any_element()
    }

    fn body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match self.step {
            Step::Welcome => card(true, theme)
                .child(brand_mark(theme))
                .child(
                    div()
                        .text_size(px(34.))
                        .line_height(px(40.))
                        .font_weight(FontWeight::BLACK)
                        .child("harbard"),
                )
                .child(
                    div()
                        .max_w(px(420.))
                        .mt(px(12.))
                        .text_size(px(17.))
                        .line_height(px(25.))
                        .text_color(theme.c("ink-2"))
                        .child(t(cx, "setup.welcomeLead")),
                )
                .child(note(t(cx, "setup.welcomeNote"), theme))
                .children(error_line(self.error.clone(), theme))
                .child(
                    button("setup-go", t(cx, "setup.searchThisMac"), true, false, theme)
                        .mt(px(28.))
                        .h(px(44.))
                        .px(px(22.))
                        .text_size(px(16.))
                        .on_click(cx.listener(|this, _, _, cx| this.scan(cx))),
                )
                .into_any_element(),
            Step::Scan => {
                let mut list = div().w_full().flex().flex_col().gap(px(10.)).mt(px(22.)).text_left();
                for (i, target) in ["claude", "codex", "cursor"].into_iter().enumerate() {
                    let h = self.harnesses.iter().find(|h| h.target == target);
                    let revealed = self.scan_tick as usize > i;
                    let present = revealed && h.is_some_and(|h| h.present);
                    let word: SharedString = if !revealed {
                        "…".into()
                    } else if present {
                        t(cx, "setup.found")
                    } else {
                        t(cx, "setup.missing")
                    };
                    let name = harness_label(h.map(|h| h.harness.as_str()).unwrap_or(target), cx);
                    list = list.child(row(
                        name,
                        chip(word, if present { "green" } else { "neutral" }, theme).into_any_element(),
                        revealed,
                        theme,
                    ));
                }
                card(true, theme)
                    .child(title(t(cx, "setup.searching")))
                    .child(note(t(cx, "setup.scanList"), theme))
                    .child(list)
                    .into_any_element()
            }
            Step::Choose => {
                let present = self.harnesses.iter().filter(|h| h.present).count();
                let wired = self.harnesses.iter().filter(|h| h.ok).count();
                let heading = if present > 0 {
                    let tools = crate::i18n::tn(cx, "setup.tools", present as u64, &[]);
                    tv(cx, "setup.chooseFound", &[("tools", &tools)])
                } else {
                    t(cx, "setup.chooseNone")
                };
                let hint = if present > 0 {
                    t(cx, "setup.chooseHint")
                } else {
                    t(cx, "setup.chooseNoneHint")
                };
                let n = self.selected.len();
                let label = if n == 0 {
                    t(cx, "setup.notNow")
                } else {
                    tv(cx, "setup.connectCount", &[("count", &n.to_string())])
                };
                let harnesses = self.harnesses.clone();
                let mut list = div().w_full().flex().flex_col().gap(px(10.)).mt(px(22.));
                for h in &harnesses {
                    list = list.child(self.harness_card(h, theme, cx));
                }
                let busy = self.busy;
                card(false, theme)
                    .child(title(heading))
                    .child(note(hint, theme).max_w(px(560.)))
                    .child(list)
                    .children(error_line(self.error.clone(), theme))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(14.))
                            .mt(px(22.))
                            .child(
                                button("setup-connect", label, n > 0, busy, theme)
                                    .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.wire(cx)))),
                            )
                            .child(div().text_size(px(size::UI)).text_color(theme.c("ink-3")).child(tv(
                                cx,
                                "setup.alreadyConnected",
                                &[
                                    ("wired", &wired.to_string()),
                                    ("total", &self.harnesses.len().to_string()),
                                ],
                            ))),
                    )
                    .into_any_element()
            }
            Step::Wire => {
                let failed = self.error.is_some();
                let rows = self.results.clone().unwrap_or_default();
                let mut list = div().w_full().flex().flex_col().gap(px(10.)).mt(px(22.)).text_left();
                for r in &rows {
                    let (word, tone) = if r.ok {
                        (t(cx, "setup.connected"), "green")
                    } else {
                        (t(cx, "setup.failed"), "red")
                    };
                    list = list.child(row(
                        harness_label(&r.target, cx),
                        chip(word, tone, theme).into_any_element(),
                        true,
                        theme,
                    ));
                }
                let busy = self.busy;
                card(true, theme)
                    .child(title(if failed {
                        t(cx, "setup.wireIncompleteTitle")
                    } else {
                        t(cx, "setup.wireTitle")
                    }))
                    .child(note(
                        if failed {
                            t(cx, "setup.wireIncompleteNote")
                        } else {
                            t(cx, "setup.wireNote")
                        },
                        theme,
                    ))
                    .children(error_line(self.error.clone(), theme))
                    .when(!rows.is_empty(), |d| d.child(list))
                    .when(failed, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(14.))
                                .mt(px(22.))
                                .child(
                                    button("setup-again", t(cx, "setup.chooseAgain"), true, false, theme).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            this.error = None;
                                            this.results = None;
                                            this.step = Step::Choose;
                                            cx.notify();
                                        }),
                                    ),
                                )
                                .child(
                                    button("setup-retry", t(cx, "setup.retry"), false, busy, theme)
                                        .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.wire(cx)))),
                                ),
                        )
                    })
                    .into_any_element()
            }
            Step::Done => card(true, theme)
                .child(div().mb(px(12.)).child(ui::icon("circle-check", 40., theme.c("green"))))
                .child(title(t(cx, "setup.doneTitle")))
                .child(note(t(cx, "setup.doneNote"), theme))
                .into_any_element(),
        }
    }
}

impl Render for SetupScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        if self.opening {
            return main_column(&theme)
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .px(px(24.))
                        .py(px(40.))
                        .text_color(theme.c("ink-3"))
                        .child(t(cx, "setup.opening")),
                )
                .into_any_element();
        }
        let show_skip = !matches!(self.step, Step::Done | Step::Wire);
        let busy = self.busy;
        let header = head_bar(&theme)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(19.))
                    .font_weight(FontWeight::BLACK)
                    .whitespace_nowrap()
                    .child(ui::icon("wrench", 20., theme.c("ink")))
                    .child(t(cx, "setup.title")),
            )
            .when(show_skip, |d| {
                d.child(
                    div().ml_auto().child(
                        button("setup-skip", t(cx, "setup.notNow"), false, busy, &theme)
                            .when(!busy, |b| b.on_click(cx.listener(|this, _, _, cx| this.skip_all(cx)))),
                    ),
                )
            });
        let body = self.body(&theme, cx);
        main_column(&theme)
            .child(header)
            .child(
                div()
                    .id("setup-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(40.))
                    .child(body),
            )
            .into_any_element()
    }
}
