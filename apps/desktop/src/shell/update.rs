//! "Update harbard" in the shell (`update-state.tsx`, `update-banner.tsx`,
//! `lib/update.ts`): one reader of `GET /api/v1/update` for the pill, the
//! workspace menu's dot, About, Preferences and the update dialog.
//!
//! [`UpdateStore`] reads every ten minutes (serve keeps the check cached), and
//! every 1.5 s while a run goes on. serve restarts mid-run, so a failed read
//! during a run means "restarting", never an error; a 404 means this serve
//! has no update route and every surface stays hidden.
//!
//! [`UpdatePill`] is the title band's right end: "A new version…", "See
//! what's new", Update and Later; a spinner while a run goes; Reload once the
//! run installed a newer build than this one.

// serve's error travels by value, as in `api` (built once per failed request).
#![allow(clippy::result_large_err)]

use crate::api::{self, ApiError};
use crate::derive::update::{Progress, mmss, run_progress, step_label};
use crate::i18n::{t, tv};
use crate::model::update::{UpdateCheck, UpdateRun, UpdateView};
use crate::robot::clock;
use crate::theme::Theme;
use crate::ui::kit_theme::{self, FocusRing as _};
use crate::ui::{self, icon, icon_size, size, space, weight};
use gpui::{
    App, AppContext, Context, Entity, Global, Hsla, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Task, Transformation, Window, div,
    px,
};
use std::time::Duration;

pub const IDLE_POLL: Duration = Duration::from_secs(10 * 60);
pub const RUN_POLL: Duration = Duration::from_millis(1_500);

/// The commit this app was built from, when the build stamped one
/// (`HARBARD_APP_COMMIT`, from `build.rs`).
pub const APP_COMMIT: &str = match option_env!("HARBARD_APP_COMMIT") {
    Some(c) => c,
    None => "",
};

/// When this app process started, in ms. Set by `init`.
static LAUNCHED_MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

/// When this app process started: a run that finished earlier was already on
/// disk at launch, so a restart would only start this same build again.
pub fn launched_ms() -> u64 {
    *LAUNCHED_MS.get_or_init(harbard_core::clock::now_ms)
}

/// The `desk.json` key that remembers "Later" for one target commit.
const DISMISSED: &str = "update_dismissed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerMode {
    Available,
    Running,
    Done,
}

pub struct UpdateStore {
    pub view: Option<UpdateView>,
    /// This serve has no update route: every surface stays hidden.
    pub unsupported: bool,
    /// A run was going on and serve stopped answering: it is restarting.
    pub restarting: bool,
    pub checking: bool,
    pub installing: bool,
    /// The run this app watched going.
    pub followed: Option<String>,
    /// The last refusal or failure of a write, in plain words.
    pub error: Option<SharedString>,
    dismissed: Option<String>,
    poller: Option<Task<()>>,
    /// The step clock: a redraw on each whole second of the step in hand,
    /// only while a run goes on and one of the app's windows is in front.
    ticker: Option<Task<()>>,
}

#[derive(Clone)]
struct Handle(Entity<UpdateStore>);

impl Global for Handle {}

impl UpdateStore {
    pub fn get(cx: &App) -> Entity<UpdateStore> {
        cx.global::<Handle>().0.clone()
    }

    fn running(&self) -> bool {
        self.view
            .as_ref()
            .and_then(|v| v.run.as_ref())
            .is_some_and(|r| r.state == "running")
            || self.installing
            || self.restarting
    }

    /// Read now, then keep reading at the pace the state asks for.
    pub fn read(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async { api::get_as::<UpdateView>("/update") });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |s, cx| s.apply(got, cx));
        })
        .detach();
    }

    fn apply(&mut self, got: Result<UpdateView, ApiError>, cx: &mut Context<Self>) {
        let was_running = self.running();
        let before = (
            self.view.clone(),
            self.unsupported,
            self.restarting,
            self.installing,
            self.followed.clone(),
        );
        match got {
            Ok(view) => {
                if let Some(run) = view.run.as_ref().filter(|r| r.state == "running") {
                    self.followed = Some(run.id.clone());
                }
                if view.run.as_ref().is_none_or(|r| r.state != "running") {
                    self.installing = false;
                }
                self.view = Some(view);
                self.unsupported = false;
                self.restarting = false;
            }
            Err(e) => {
                if was_running {
                    self.restarting = true;
                } else if e.missing() {
                    self.unsupported = true;
                }
            }
        }
        if self.running() != was_running || self.poller.is_none() {
            self.schedule(cx);
        }
        self.pace_clock(cx);
        // A reading every 1.5 s during a run: redraw only for what changed,
        // and only with a window in front (a window drawn again on
        // activation, see `robot::clock::still`, reads the store afresh).
        let after = (
            self.view.clone(),
            self.unsupported,
            self.restarting,
            self.installing,
            self.followed.clone(),
        );
        if after != before && cx.active_window().is_some() {
            crate::perf::count(crate::perf::Source::Update, 1);
            cx.notify();
        }
    }

    /// The step in hand, when a run goes on and serve answers.
    fn progress(&self) -> Option<Progress> {
        let run = self.view.as_ref()?.run.as_ref()?;
        run_progress(run, harbard_core::clock::now_ms())
    }

    /// Run the step clock while a step is timed, stop it otherwise.
    fn pace_clock(&mut self, cx: &mut Context<Self>) {
        let timed = !self.restarting && self.progress().is_some_and(|p| p.elapsed_ms.is_some());
        if !timed {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                // To the next whole second of the step's clock, so the pill
                // changes once a second and in step with the dialog.
                let Ok(Some(wait)) = this.update(cx, |s, _| {
                    s.progress()
                        .and_then(|p| p.elapsed_ms)
                        .map(|ms| Duration::from_millis(1_000 - ms % 1_000 + 5))
                }) else {
                    break;
                };
                cx.background_executor().timer(wait).await;
                let go_on = this.update(cx, |s, cx| {
                    // With every window behind another app nobody reads the
                    // clock: no frame; the next reading in front catches up.
                    if cx.active_window().is_some() {
                        crate::perf::count(crate::perf::Source::Update, 1);
                        cx.notify();
                    }
                    s.progress().is_some_and(|p| p.elapsed_ms.is_some())
                });
                if !matches!(go_on, Ok(true)) {
                    break;
                }
            }
            let _ = this.update(cx, |s, _| s.ticker = None);
        }));
    }

    fn schedule(&mut self, cx: &mut Context<Self>) {
        let every = if self.running() { RUN_POLL } else { IDLE_POLL };
        self.poller = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(every).await;
                if this.update(cx, |s, cx| s.read(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// `POST /update/check`: look for a newer version now.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        self.checking = true;
        self.error = None;
        cx.notify();
        let task =
            cx.background_spawn(async { api::post("/update/check", &serde_json::json!({})) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |s, cx| {
                s.checking = false;
                match got
                    .and_then(|v| serde_json::from_value::<UpdateView>(v).map_err(decode_error))
                {
                    Ok(v) => s.view = Some(v),
                    Err(e) => s.error = Some(error_text(e.code.as_deref(), &e.message, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `POST /update/install`: start a run (or follow the one going).
    pub fn install(&mut self, cx: &mut Context<Self>) {
        self.installing = true;
        self.error = None;
        cx.notify();
        let task =
            cx.background_spawn(async { api::post("/update/install", &serde_json::json!({})) });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |s, cx| {
                match got
                    .and_then(|v| serde_json::from_value::<UpdateView>(v).map_err(decode_error))
                {
                    Ok(v) => {
                        if let Some(r) = &v.run {
                            s.followed = Some(r.id.clone());
                        }
                        s.view = Some(v);
                    }
                    Err(e) if e.code.as_deref() == Some("busy") => s.read(cx),
                    Err(e) => {
                        s.installing = false;
                        s.error = Some(error_text(e.code.as_deref(), &e.message, cx));
                    }
                }
                s.schedule(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// "Later": hide the pill until a newer target appears.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target_commit() else {
            return;
        };
        crate::desk::save_desk_prefs(|p| {
            p.rest
                .insert(DISMISSED.into(), serde_json::Value::String(target.clone()));
        });
        self.dismissed = Some(target);
        cx.notify();
    }

    fn target_commit(&self) -> Option<String> {
        self.view
            .as_ref()?
            .check
            .as_ref()?
            .target
            .as_ref()
            .map(|t| t.commit.clone())
    }

    /// A newer version is there (the menu's dot, About).
    pub fn available(&self) -> bool {
        !self.unsupported && self.last_check().is_some_and(|c| c.status == "available")
    }

    pub fn last_check(&self) -> Option<&UpdateCheck> {
        self.view.as_ref()?.check.as_ref()
    }

    pub fn banner(&self) -> Option<BannerMode> {
        if self.unsupported {
            return None;
        }
        banner_mode(
            self.view.as_ref(),
            self.dismissed.as_deref(),
            APP_COMMIT,
            launched_ms(),
            self.restarting || self.installing,
        )
    }
}

fn decode_error(e: serde_json::Error) -> ApiError {
    ApiError {
        status: Some(200),
        code: None,
        message: e.to_string(),
        body: serde_json::Value::Null,
    }
}

pub fn init(cx: &mut App) {
    launched_ms();
    let dismissed = crate::desk::desk_prefs()
        .rest
        .get(DISMISSED)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let store = cx.new(|cx: &mut Context<UpdateStore>| {
        let mut s = UpdateStore {
            view: None,
            unsupported: false,
            restarting: false,
            checking: false,
            installing: false,
            followed: None,
            error: None,
            dismissed,
            poller: None,
            ticker: None,
        };
        s.read(cx);
        s
    });
    cx.set_global(Handle(store));
}

// ---------------------------------------------------------------------------
// Decisions (lib/update.ts)
// ---------------------------------------------------------------------------

/// What the pill shows. A run going, or one that finished while this app
/// runs an older build, always shows; a new version until "Later" for it.
pub fn banner_mode(
    view: Option<&UpdateView>,
    dismissed: Option<&str>,
    app_commit: &str,
    launched_ms: u64,
    restarting: bool,
) -> Option<BannerMode> {
    if restarting {
        return Some(BannerMode::Running);
    }
    let view = view?;
    if let Some(run) = &view.run {
        if run.state == "running" {
            return Some(BannerMode::Running);
        }
        if page_is_stale(run, app_commit, launched_ms) {
            return Some(BannerMode::Done);
        }
    }
    let check = view.check.as_ref()?;
    let target = check
        .target
        .as_ref()
        .filter(|_| check.status == "available")?;
    (dismissed != Some(target.commit.as_str())).then_some(BannerMode::Available)
}

/// A run finished while this app was open, to a commit other than the app's:
/// a restart shows it. A run that finished before the app opened cannot be
/// shown by a restart (the app was installed some other way since, such as
/// `just update`), so asking for one would only ask again after it.
pub fn page_is_stale(run: &UpdateRun, app_commit: &str, launched_ms: u64) -> bool {
    let Some(to) = run.to.as_deref().filter(|_| run.state == "done") else {
        return false;
    };
    if run.finished_ms.is_some_and(|f| f < launched_ms) {
        return false;
    }
    if app_commit.is_empty() {
        return false;
    }
    let (a, b) = (to.to_lowercase(), app_commit.to_lowercase());
    !(a.starts_with(&b) || b.starts_with(&a))
}

/// "1 change", "11 changes".
pub fn changes_count(n: u64, cx: &App) -> SharedString {
    if n == 1 {
        t(cx, "update.changesOne")
    } else {
        tv(cx, "update.changesMany", &[("count", &n.to_string())])
    }
}

/// The pill's line: the headline, and how many changes when known.
pub fn banner_line(check: &UpdateCheck, cx: &App) -> SharedString {
    if check.behind > 0 {
        format!(
            "{} · {}",
            t(cx, "update.bannerTitle"),
            changes_count(check.behind, cx)
        )
        .into()
    } else {
        t(cx, "update.bannerTitle")
    }
}

/// "Updating · 4 of 5 · Build the app · 3:12": where a run is, in steps that
/// run on this Mac, never as a percentage nobody measured.
pub fn progress_line(p: &Progress, cx: &App) -> SharedString {
    let (n, total) = (p.index.to_string(), p.total.to_string());
    let step = step_label(&p.step, cx);
    match p.elapsed_ms {
        Some(ms) => tv(
            cx,
            "update.bannerProgressTime",
            &[
                ("n", &n),
                ("total", &total),
                ("step", &step),
                ("time", &mmss(ms)),
            ],
        ),
        None => tv(
            cx,
            "update.bannerProgress",
            &[("n", &n), ("total", &total), ("step", &step)],
        ),
    }
}

/// A refusal or failure in plain words; an unknown code keeps serve's message.
pub fn error_text(code: Option<&str>, message: &str, cx: &App) -> SharedString {
    let key = match code.unwrap_or_default() {
        "fetch_failed" => "update.errorFetchFailed",
        "dirty" => "update.problemDirty",
        "not_on_main" => "update.problemNotOnMain",
        "diverged" => "update.problemDiverged",
        "no_checkout" => "update.problemNoCheckout",
        "step_failed" => "update.errorStepFailed",
        "interrupted" => "update.errorInterrupted",
        "busy" => "update.errorBusy",
        "forbidden_origin" | "loopback_only" => "update.errorLocalOnly",
        "role_forbids" => "update.errorRoleForbids",
        _ if !message.is_empty() => return message.to_string().into(),
        _ => "update.errorFallback",
    };
    t(cx, key)
}

// ---------------------------------------------------------------------------
// The pill
// ---------------------------------------------------------------------------

pub struct UpdatePill;

impl UpdatePill {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        // Back in front: draw again, so a held spinner and clock start again.
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        UpdatePill
    }
}

fn open_dialog(window: &mut Window, cx: &mut App) {
    crate::prefs::open_update_dialog(window, cx);
}

/// The pill, a part of it, and its action and close (`.hb-update-pill*`).
const PILL_H: f32 = 28.;
const PILL_PART: f32 = 24.;
const PILL_ACTION: f32 = 22.;

/// The spinner's turn while an update runs (a wait on another process, M5).
/// One turn in 8 steps, a step per loop tick: a turn every 2 s.
const SPIN_STEPS: f64 = 8.0;

impl Render for UpdatePill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spin_t = clock::clock_ms(std::time::Instant::now());
        let el = self.pill(spin_t, cx);
        // The spinner steps while the window is in front; behind it holds
        // still (the window is drawn again when it comes back).
        let spinning = UpdateStore::get(cx).read(cx).banner() == Some(BannerMode::Running);
        if spinning && window.is_window_active() && !clock::reduced(cx) {
            clock::wake_at(
                window.current_view(),
                clock::instant_at(clock::next_grid(spin_t, clock::LOOP_MS)),
                crate::perf::Source::Update,
                window,
                cx,
            );
        }
        el
    }
}

impl UpdatePill {
    fn pill(&mut self, spin_t: f64, cx: &mut Context<Self>) -> gpui::AnyElement {
        let store = UpdateStore::get(cx);
        let s = store.read(cx);
        let Some(mode) = s.banner() else {
            super::decor::forget_clear(super::decor::PILL);
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        let ink = theme.c("rail-ink");
        let quiet = theme.c("rail-text");
        let hover = theme.c("rail-hover");
        // `.hb-update-pill` (§3.1): a round pill on the band, 28 px, its
        // parts round too; the text 13/700, the action 13/900.
        let label = |text: SharedString, color: Hsla| {
            ui::text(text, size::SMALL, weight::STRONG, color).whitespace_nowrap()
        };
        // A quiet part of the pill: no edge, the rail's wash under the pointer.
        let quiet_btn = |id: &'static str| {
            div()
                .id(id)
                .occlude()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::SM))
                .h(px(PILL_PART))
                .px(px(space::MD))
                .rounded_full()
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .focus_ring(&theme, true)
        };
        // The one action: the rail's ink, filled.
        let action_btn = |id: &'static str, text: SharedString| {
            div()
                .id(id)
                .occlude()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::SM))
                .h(px(PILL_ACTION))
                .px(px(space::MD + space::HAIR))
                .rounded_full()
                .bg(ink)
                .cursor_pointer()
                .hover(move |st| st.bg(quiet))
                .focus_ring(&theme, false)
                .child(
                    ui::text(text, size::SMALL, weight::HEAVY, theme.c("rail-on-ink"))
                        .whitespace_nowrap(),
                )
        };
        let pill = div()
            .id("hb-update-pill")
            .occlude()
            .relative()
            .mr(px(space::LG))
            .flex()
            .items_center()
            .gap(px(space::HAIR))
            .h(px(PILL_H))
            .pl(px(space::HAIR))
            .pr(px(space::XS))
            .rounded_full()
            .bg(theme.c("rail-tile"))
            .border_1()
            .border_color(theme.c("rail-ring"))
            .font_family(ui::SANS)
            .child(super::decor::keep_clear(super::decor::PILL));
        let el = match mode {
            BannerMode::Running => {
                let text = match s.progress().filter(|_| !s.restarting) {
                    Some(p) => progress_line(&p, cx),
                    None if s.restarting => t(cx, "update.restarting"),
                    None => t(cx, "update.bannerRunning"),
                };
                // The spinner steps on the robots' clock (4 a second, sharing
                // their frames) while the window is in front, and holds still
                // behind: a continuous animation drew every frame of a
                // ten-minute run, in front or not.
                let turn = (spin_t / clock::LOOP_MS).floor() % SPIN_STEPS;
                pill.child(
                    quiet_btn("hb-update-run")
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(
                            icon("loader-circle", icon_size::ROW, ink).with_transformation(
                                Transformation::rotate(gpui::radians(
                                    turn as f32 * std::f32::consts::TAU / SPIN_STEPS as f32,
                                )),
                            ),
                        )
                        // Tabular figures: the seconds tick without moving the pill (T8).
                        .child(label(text, ink).font_features(gpui::FontFeatures(
                            std::sync::Arc::new(vec![("tnum".into(), 1)]),
                        ))),
                )
            }
            BannerMode::Done => pill
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(PILL_PART))
                        .px(px(space::MD))
                        .child(label(t(cx, "update.bannerDone"), ink)),
                )
                .child(
                    action_btn("hb-update-reload", t(cx, "update.reload"))
                        .on_click(|_, _, cx| cx.restart()),
                ),
            BannerMode::Available => {
                let line = s
                    .last_check()
                    .map(|c| banner_line(c, cx))
                    .unwrap_or_default();
                let problem = s.view.as_ref().is_some_and(|v| v.source.problem.is_some());
                let store_install = store.clone();
                let store_dismiss = store.clone();
                pill.child(
                    quiet_btn("hb-update-line")
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(icon("circle-arrow-up", icon_size::ROW, ink))
                        .child(label(line, ink)),
                )
                .child(
                    quiet_btn("hb-update-link")
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(label(t(cx, "update.seeNotes"), quiet)),
                )
                .child(
                    action_btn("hb-update-action", t(cx, "update.update")).on_click(
                        move |_, window, cx| {
                            open_dialog(window, cx);
                            // With a checkout that cannot fast-forward, the dialog says why instead.
                            if !problem {
                                store_install.update(cx, |s, cx| s.install(cx));
                            }
                        },
                    ),
                )
                .child(
                    div()
                        .id("hb-update-later")
                        .occlude()
                        .flex_none()
                        .size(px(PILL_ACTION))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |st| st.bg(hover))
                        .focus_ring(&theme, true)
                        .tooltip(kit_theme::tip(t(cx, "update.later")))
                        .on_click(move |_, _, cx| store_dismiss.update(cx, |s, cx| s.dismiss(cx)))
                        .child(icon("x", icon_size::ROW, quiet)),
                )
            }
        };
        el.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::update::{UpdateCheck, UpdateTarget};

    fn available(commit: &str) -> UpdateView {
        UpdateView {
            check: Some(UpdateCheck {
                status: "available".into(),
                target: Some(UpdateTarget {
                    commit: commit.into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn the_pill_shows_until_later_for_that_target() {
        let v = available("abc123");
        assert_eq!(
            banner_mode(Some(&v), None, "", 0, false),
            Some(BannerMode::Available)
        );
        assert_eq!(banner_mode(Some(&v), Some("abc123"), "", 0, false), None);
        assert_eq!(
            banner_mode(Some(&v), Some("older"), "", 0, false),
            Some(BannerMode::Available)
        );
        assert_eq!(banner_mode(None, None, "", 0, false), None);
        assert_eq!(
            banner_mode(None, None, "", 0, true),
            Some(BannerMode::Running)
        );
    }

    #[test]
    fn a_run_shows_while_it_goes_and_asks_for_a_restart_when_newer() {
        let mut v = available("abc");
        v.run = Some(UpdateRun {
            state: "running".into(),
            ..Default::default()
        });
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "", 0, false),
            Some(BannerMode::Running)
        );
        v.run = Some(UpdateRun {
            state: "done".into(),
            to: Some("def4567".into()),
            ..Default::default()
        });
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "abc0000", 0, false),
            Some(BannerMode::Done)
        );
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "def4567aa", 0, false),
            None
        );
    }

    #[test]
    fn a_run_that_finished_before_launch_never_asks_for_a_restart() {
        let mut v = available("abc");
        v.run = Some(UpdateRun {
            state: "done".into(),
            to: Some("def4567".into()),
            finished_ms: Some(1_000),
            ..Default::default()
        });
        // Installed some other way after that run: a restart starts this build again.
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "abc0000", 2_000, false),
            None
        );
        // Finished while this build was open: a restart shows it.
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "abc0000", 500, false),
            Some(BannerMode::Done)
        );
    }
}
