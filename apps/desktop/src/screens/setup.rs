//! Set up harbard (`/setup`): the first-run wizard. It finds Claude Code,
//! Codex and Cursor on this Mac (`GET /setup`), lets the operator choose
//! which ones harbard connects (`POST /setup/install`), and records the
//! choice (`POST /setup/complete`). When it finishes the app goes Home.
//!
//! An agent only hears the operator's notes through a hook, so a partial
//! connection never celebrates: the ones that failed stay on screen.

use super::secondary::{self as kit, button, chip, head_bar, main_column, quiet_button};
use crate::i18n::{I18n, t, tv};
use crate::model::setup::{SetupHarness, SetupInstallResponse, SetupInstallResult, SetupStatus};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::pill_people::{self as character, type_px};
use crate::ui::{self, icon_size, motion, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Task, Window, div,
    ease_out_quint, px, svg,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::Duration;

/// The board must appear even when serve is dead or half-open.
const GATE_CAP: Duration = Duration::from_millis(2500);
/// How long "All set" stays before Home: after "Not now", after connecting.
const LEAVE_AFTER_SKIP: Duration = Duration::from_millis(900);
const LEAVE_AFTER_WIRE: Duration = Duration::from_millis(1100);
/// The wizard's card and the widest its prose runs.
const CARD_W: f32 = 640.;
const PROSE_W: f32 = 420.;
const PROSE_WIDE_W: f32 = 560.;

// `.hb-setup-*` in the console's `workspace.css` (§3.1): Setup keeps its
// large type, the real mark at 88 px and its staged entrance.
const CARD_PAD_X: f32 = 36.;
const CARD_RADIUS: f32 = 14.;
const TILE: f32 = 88.;
const TILE_RADIUS: f32 = 22.;
const MARK: f32 = 54.;
const TILE_GAP: f32 = 18.;
const GO_H: f32 = 44.;
const GO_PAD_X: f32 = 22.;
const GO_GAP: f32 = 28.;
const LIST_GAP: f32 = 10.;
const LIST_TOP: f32 = 22.;
const ROW_PAD_Y: f32 = 10.;
const ROW_PAD_X: f32 = 14.;
const ROW_RADIUS: f32 = 10.;
const ROW_RISE: f32 = 4.;
const CARD_RISE: f32 = 10.;
const TOOL_PAD_Y: f32 = 14.;
const TOOL_CHECK: f32 = 18.;
const DONE_ICON: f32 = 40.;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    Scan,
    Choose,
    Wire,
    Done,
}

impl Step {
    /// The step's name, which keys its card's entrance.
    fn name(self) -> &'static str {
        match self {
            Step::Welcome => "welcome",
            Step::Scan => "scan",
            Step::Choose => "choose",
            Step::Wire => "wire",
            Step::Done => "done",
        }
    }
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
        for (wait, tick) in [(motion::FAST, 1), (motion::SLOW, 2)] {
            let timer = cx.background_executor().timer(wait);
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
                        this.leave_after(LEAVE_AFTER_SKIP, cx);
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
                        let all: Vec<String> =
                            this.harnesses.iter().map(|h| h.target.clone()).collect();
                        this.harnesses = res.setup.harnesses;
                        let wired: Vec<String> = res
                            .results
                            .iter()
                            .filter(|r| r.ok)
                            .map(|r| r.target.clone())
                            .collect();
                        let failed: Vec<&SetupInstallResult> =
                            res.results.iter().filter(|r| !r.ok).collect();
                        // A partial connection must not celebrate.
                        if let Some(first) = failed.first() {
                            this.error = Some(if failed.len() == 1 {
                                tv(
                                    cx,
                                    "setup.errorWireOne",
                                    &[("tool", &harness_label(&first.target, cx))],
                                )
                            } else {
                                tv(
                                    cx,
                                    "setup.errorWireMany",
                                    &[("count", &failed.len().to_string())],
                                )
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

    fn complete_after_wire(
        &mut self,
        skipped: Vec<String>,
        wired: Vec<String>,
        cx: &mut Context<Self>,
    ) {
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
                        this.leave_after(LEAVE_AFTER_WIRE, cx);
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

/// `.hb-setup-card`: the step's card, 32 px by 36 px inside, 14 px corners.
fn card(center: bool, theme: &Theme) -> gpui::Div {
    div()
        .w_full()
        .max_w(px(CARD_W))
        .mx_auto()
        .py(px(space::BREAK))
        .px(px(CARD_PAD_X))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(CARD_RADIUS))
        .bg(theme.c("surface-2"))
        .when(center, |d| d.flex().flex_col().items_center().text_center())
}

/// `.hb-setup-title`: a step's heading, 24/900.
fn title(text: SharedString, theme: &Theme) -> gpui::Div {
    character::text_px(text, type_px::LARGE, weight::HEAVY, theme.c("ink"))
}

/// `.hb-setup-note`: 15 px in `ink-3`.
fn note(text: SharedString, theme: &Theme) -> gpui::Div {
    character::text_px(text, size::BODY, weight::REGULAR, theme.c("ink-3"))
        .max_w(px(PROSE_W))
        .mt(px(space::MD))
}

/// Where each part of the welcome is in its staged entrance (`setup-*-in`):
/// the tile first, the name after it, then the words and the button. Each
/// is (delay, run) as shares of [`ENTRANCE`].
const STAGE_MARK: (f32, f32) = (0.0, 0.5);
const STAGE_BRAND: (f32, f32) = (0.2, 0.5);
const STAGE_TEXT: (f32, f32) = (0.4, 0.6);
/// The whole welcome's entrance: three 200 ms steps, overlapping.
const ENTRANCE: Duration = Duration::from_millis(400);

/// An element that enters with the welcome: it fades in and rises `rise` px
/// on its share of the entrance. Still when motion is reduced (M7).
fn staged(
    el: gpui::Div,
    id: &'static str,
    stage: (f32, f32),
    rise: f32,
    still: bool,
) -> AnyElement {
    if still {
        return el.into_any_element();
    }
    el.relative()
        .with_animation(id, Animation::new(ENTRANCE), move |el, t| {
            let p = character::entrance(t, stage.0, stage.1);
            el.opacity(p).top(px(rise * (1. - p)))
        })
        .into_any_element()
}

/// `.hb-setup-go` (and the step's other primary): 44 px, 16 px words.
fn big_button(
    id: &'static str,
    label: SharedString,
    primary: bool,
    disabled: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let (bg, edge, ink, hover) = if primary {
        ("accent-deep", "accent-deep", "accent-ink", "accent-hover")
    } else {
        ("surface", "line-strong", "ink", "btn-hover")
    };
    let hover = theme.c(hover);
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(GO_H))
        .px(px(GO_PAD_X))
        .rounded(px(radius::MD))
        .border_1()
        .border_color(theme.c(edge))
        .bg(theme.c(bg))
        .child(
            character::text_px(label, type_px::ACTION, weight::STRONG, theme.c(ink))
                .whitespace_nowrap(),
        );
    if disabled {
        base.opacity(0.5)
    } else {
        kit::control_focus(base, theme)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
    }
}

fn error_line(text: Option<SharedString>, theme: &Theme) -> Option<gpui::Div> {
    text.map(|e| {
        div()
            .flex()
            .mt(px(space::LG))
            .child(kit::error_note(e, theme))
    })
}

/// `.hb-setup-row`: one tool and what the wizard found, on its own box.
/// It slides in when the scan reaches it (180 ms, 4 px), still when
/// motion is reduced (M7).
fn row(
    ix: usize,
    name: SharedString,
    status: AnyElement,
    revealed: bool,
    still: bool,
    theme: &Theme,
) -> AnyElement {
    let el = div()
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(space::LG))
        .py(px(ROW_PAD_Y))
        .px(px(ROW_PAD_X))
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(ROW_RADIUS))
        .bg(theme.c("surface"))
        .child(character::text_px(
            name,
            size::BODY,
            weight::STRONG,
            theme.c("ink"),
        ))
        .child(status);
    if !revealed {
        return el.opacity(0.).top(px(ROW_RISE)).into_any_element();
    }
    if still {
        return el.into_any_element();
    }
    el.with_animation(
        SharedString::from(format!("setup-row-{ix}")),
        Animation::new(character::ROW_ENTER).with_easing(ease_out_quint()),
        |el, t| el.opacity(t).top(px(ROW_RISE * (1. - t))),
    )
    .into_any_element()
}

/// `.hb-setup-mark`: the real harbard mark, 54 px on its 88 px tile with
/// 22 px corners, and its presence dot where the brand draws it.
fn brand_mark(theme: &Theme) -> gpui::Div {
    // The mark's 64-unit box: the dot sits at (54, 13) with radius 7.
    let k = MARK / 64.;
    let inset = (TILE - MARK) / 2.;
    let r = 7. * k;
    div()
        .relative()
        .size(px(TILE))
        .flex_none()
        .rounded(px(TILE_RADIUS))
        .bg(theme.c("ws-tile"))
        .child(
            svg()
                .path("brand/mark-ink.svg")
                .absolute()
                .left(px(inset))
                .top(px(inset))
                .size(px(MARK))
                .text_color(theme.c("rail-ink")),
        )
        .child(
            div()
                .absolute()
                .left(px(inset + 54. * k - r))
                .top(px(inset + 13. * k - r))
                .size(px(2. * r))
                .rounded_full()
                .bg(theme.c("mark-dot")),
        )
}

impl SetupScreen {
    /// One tool to connect: the tick is the whole selection, so the row
    /// needs no edge of its own inside the card.
    fn harness_card(&self, h: &SetupHarness, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.contains(&h.target);
        // Connected is verified; found but not connected waits on the
        // operator; missing is nothing to act on (C3).
        let (word, tone) = if h.ok {
            (t(cx, "setup.connected"), "green")
        } else if h.present {
            (t(cx, "setup.detected"), "amber")
        } else {
            (t(cx, "setup.missing"), "neutral")
        };
        let target = h.target.clone();
        let hover = theme.c("hover");
        let accent = theme.c("accent");
        let check = div()
            .flex_none()
            .mt(px(space::HAIR))
            .size(px(TOOL_CHECK))
            .rounded(px(radius::SM))
            .flex()
            .items_center()
            .justify_center()
            .map(|d| {
                if selected {
                    d.bg(accent)
                        .child(ui::icon("check", icon_size::INLINE, theme.c("accent-ink")))
                } else {
                    d.border_1().border_color(theme.c("line-strong"))
                }
            });
        kit::row_focus(
            div().id(SharedString::from(format!("setup-{}", h.target))),
            theme,
        )
        // `.hb-setup-tool`: its own box; chosen, an accent edge (§3.1).
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(space::LG))
        .py(px(TOOL_PAD_Y))
        .px(px(space::XL))
        .rounded(px(radius::LG))
        .border_1()
        .border_color(if selected {
            accent
        } else {
            theme.c("line-strong")
        })
        .bg(theme.c("surface"))
        .when(selected, |d| {
            d.shadow(vec![gpui::BoxShadow {
                color: accent,
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(1.),
                inset: false,
            }])
        })
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
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
                .gap(px(space::HAIR))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .child(character::text_px(
                            harness_label(&h.harness, cx),
                            size::BODY,
                            weight::STRONG,
                            theme.c("ink"),
                        ))
                        .child(chip(word, tone, theme)),
                )
                .child(ui::text(
                    blurb_for(&h.target, &h.blurb, cx),
                    size::UI,
                    weight::REGULAR,
                    theme.c("ink-2"),
                ))
                .child(
                    kit::mono(crate::ui::format::short_path(&h.path))
                        .text_color(theme.c("ink-3"))
                        .truncate(),
                ),
        )
        .into_any_element()
    }

    fn body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let still = crate::robot::clock::reduced(cx);
        match self.step {
            Step::Welcome => card(true, theme)
                .child(staged(
                    div().mb(px(TILE_GAP)).child(brand_mark(theme)),
                    "setup-mark",
                    STAGE_MARK,
                    TILE_GAP / 2.,
                    still,
                ))
                .child(staged(
                    character::text_px("harbard", type_px::HERO, weight::HEAVY, theme.c("ink")),
                    "setup-brand",
                    STAGE_BRAND,
                    0.,
                    still,
                ))
                .child(staged(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(
                            character::text_px(
                                t(cx, "setup.welcomeLead"),
                                type_px::LEAD,
                                weight::REGULAR,
                                theme.c("ink-2"),
                            )
                            .max_w(px(PROSE_W))
                            .mt(px(space::LG)),
                        )
                        .child(note(t(cx, "setup.welcomeNote"), theme))
                        .children(error_line(self.error.clone(), theme))
                        .child(
                            big_button(
                                "setup-go",
                                t(cx, "setup.searchThisMac"),
                                true,
                                false,
                                theme,
                            )
                            .mt(px(GO_GAP))
                            .on_click(cx.listener(|this, _, _, cx| this.scan(cx))),
                        ),
                    "setup-text",
                    STAGE_TEXT,
                    14.,
                    still,
                ))
                .into_any_element(),
            Step::Scan => {
                let mut list = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(LIST_GAP))
                    .mt(px(LIST_TOP))
                    .text_left();
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
                        i,
                        name,
                        chip(word, if present { "amber" } else { "neutral" }, theme)
                            .into_any_element(),
                        revealed,
                        still,
                        theme,
                    ));
                }
                card(true, theme)
                    .child(title(t(cx, "setup.searching"), theme))
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
                // The rows carry their own padding for the hover wash; pull
                // them back so their ticks sit on the card's inset.
                let mut list = div().flex().flex_col().gap(px(LIST_GAP)).mt(px(LIST_TOP));
                for h in &harnesses {
                    list = list.child(self.harness_card(h, theme, cx));
                }
                let busy = self.busy;
                card(false, theme)
                    .child(title(heading, theme))
                    .child(note(hint, theme).max_w(px(PROSE_WIDE_W)))
                    .child(list)
                    .children(error_line(self.error.clone(), theme))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(space::LG))
                            .mt(px(LIST_TOP))
                            .child(
                                big_button("setup-connect", label, n > 0, busy, theme)
                                    .when(!busy, |b| {
                                        b.on_click(cx.listener(|this, _, _, cx| this.wire(cx)))
                                    }),
                            )
                            .child(ui::text(
                                tv(
                                    cx,
                                    "setup.alreadyConnected",
                                    &[
                                        ("wired", &wired.to_string()),
                                        ("total", &self.harnesses.len().to_string()),
                                    ],
                                ),
                                size::UI,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )),
                    )
                    .into_any_element()
            }
            Step::Wire => {
                let failed = self.error.is_some();
                let rows = self.results.clone().unwrap_or_default();
                let mut list = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(LIST_GAP))
                    .mt(px(LIST_TOP))
                    .text_left();
                for (i, r) in rows.iter().enumerate() {
                    let (word, tone) = if r.ok {
                        (t(cx, "setup.connected"), "green")
                    } else {
                        (t(cx, "setup.failed"), "red")
                    };
                    list = list.child(row(
                        i,
                        harness_label(&r.target, cx),
                        chip(word, tone, theme).into_any_element(),
                        true,
                        still,
                        theme,
                    ));
                }
                let busy = self.busy;
                card(true, theme)
                    .child(title(
                        if failed {
                            t(cx, "setup.wireIncompleteTitle")
                        } else {
                            t(cx, "setup.wireTitle")
                        },
                        theme,
                    ))
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
                                .gap(px(space::MD))
                                .mt(px(space::XL))
                                .child(
                                    button(
                                        "setup-again",
                                        t(cx, "setup.chooseAgain"),
                                        true,
                                        false,
                                        theme,
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.error = None;
                                            this.results = None;
                                            this.step = Step::Choose;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    quiet_button("setup-retry", t(cx, "setup.retry"), busy, theme)
                                        .when(!busy, |b| {
                                            b.on_click(cx.listener(|this, _, _, cx| this.wire(cx)))
                                        }),
                                ),
                        )
                    })
                    .into_any_element()
            }
            Step::Done => card(true, theme)
                .child(div().mb(px(space::LG)).child(ui::icon(
                    "circle-check",
                    DONE_ICON,
                    theme.c("green"),
                )))
                .child(title(t(cx, "setup.doneTitle"), theme))
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
                .child(ui::empty(t(cx, "setup.opening"), &theme))
                .into_any_element();
        }
        let show_skip = !matches!(self.step, Step::Done | Step::Wire);
        let busy = self.busy;
        let header = head_bar(&theme)
            .child(
                ui::text(
                    t(cx, "setup.title"),
                    size::TITLE,
                    weight::HEAVY,
                    theme.c("ink"),
                )
                .flex_none(),
            )
            .when(show_skip, |d| {
                d.child(
                    div().ml_auto().child(
                        quiet_button("setup-skip", t(cx, "setup.notNow"), busy, &theme)
                            .when(!busy, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.skip_all(cx)))
                            }),
                    ),
                )
            });
        let body = self.body(&theme, cx);
        // `setup-card-in`: each step's card fades in and rises 10 px.
        let body = if crate::robot::clock::reduced(cx) {
            body
        } else {
            div()
                .relative()
                .child(body)
                .with_animation(
                    SharedString::from(format!("setup-card-{}", self.step.name())),
                    Animation::new(character::ENTER).with_easing(ease_out_quint()),
                    |el, t| el.opacity(t).top(px(CARD_RISE * (1. - t))),
                )
                .into_any_element()
        };
        main_column(&theme)
            .child(header)
            .child(
                div()
                    .id("setup-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(space::GUTTER))
                    .py(px(space::BREAK))
                    .child(body),
            )
            .into_any_element()
    }
}
