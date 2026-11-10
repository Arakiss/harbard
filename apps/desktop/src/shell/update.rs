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
use crate::i18n::{t, tv};
use crate::model::update::{UpdateCheck, UpdateRun, UpdateView};
use crate::theme::Theme;
use crate::ui::{self, icon};
use gpui::{
    Animation, AnimationExt as _, App, AppContext, Context, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Task, Transformation, Window, div, px,
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
        cx.notify();
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
        if page_is_stale(run, app_commit) {
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

/// The app was built before the run's target: it needs a restart to show it.
pub fn page_is_stale(run: &UpdateRun, app_commit: &str) -> bool {
    let Some(to) = run.to.as_deref().filter(|_| run.state == "done") else {
        return false;
    };
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
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        UpdatePill
    }
}

fn open_dialog(window: &mut Window, cx: &mut App) {
    crate::prefs::open_update_dialog(window, cx);
}

impl Render for UpdatePill {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = UpdateStore::get(cx);
        let s = store.read(cx);
        let Some(mode) = s.banner() else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        let ink = theme.c("rail-ink");
        let hover = theme.c("rail-hover");
        let text_btn = |id: &'static str| {
            div()
                .id(id)
                .occlude()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(12.))
                .font_weight(FontWeight::BOLD)
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
        };
        let pill = div()
            .id("hb-update-pill")
            .occlude()
            .mr(px(12.))
            .flex()
            .items_center()
            .gap(px(2.))
            .h(px(28.))
            .pl(px(2.))
            .pr(px(if mode == BannerMode::Running { 2. } else { 4. }))
            .rounded(px(14.))
            .bg(theme.c("rail-tile"))
            .border_1()
            .border_color(theme.c("rail-ring"))
            .text_color(ink)
            .font_family(ui::SANS)
            .text_size(px(13.))
            .whitespace_nowrap();
        let el = match mode {
            BannerMode::Running => {
                let label = if s.restarting {
                    t(cx, "update.restarting")
                } else {
                    t(cx, "update.bannerRunning")
                };
                pill.child(
                    text_btn("hb-update-run")
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(icon("loader-circle", 16., ink).with_animation(
                            "hb-spin",
                            Animation::new(Duration::from_millis(900)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(gpui::percentage(
                                    delta,
                                )))
                            },
                        ))
                        .child(label),
                )
            }
            BannerMode::Done => pill
                .child(
                    div()
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .font_weight(FontWeight::BOLD)
                        .child(t(cx, "update.bannerDone")),
                )
                .child(
                    div()
                        .id("hb-update-reload")
                        .occlude()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .h(px(22.))
                        .px(px(10.))
                        .rounded(px(11.))
                        .font_weight(FontWeight::BLACK)
                        .bg(ink)
                        .text_color(theme.c("rail-on-ink"))
                        .cursor_pointer()
                        .hover(|st| st.opacity(0.9))
                        .on_click(|_, _, cx| cx.restart())
                        .child(icon("refresh-cw", 14., theme.c("rail-on-ink")))
                        .child(t(cx, "update.reload")),
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
                    text_btn("hb-update-line")
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(icon("circle-arrow-up", 16., ink))
                        .child(line),
                )
                .child(
                    div()
                        .id("hb-update-link")
                        .occlude()
                        .h(px(24.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(12.))
                        .text_color(theme.c("rail-text"))
                        .underline()
                        .cursor_pointer()
                        .hover(move |st| st.bg(hover))
                        .on_click(|_, window, cx| open_dialog(window, cx))
                        .child(t(cx, "update.seeNotes")),
                )
                .child(
                    div()
                        .id("hb-update-action")
                        .occlude()
                        .flex()
                        .items_center()
                        .h(px(22.))
                        .px(px(10.))
                        .rounded(px(11.))
                        .font_weight(FontWeight::BLACK)
                        .bg(ink)
                        .text_color(theme.c("rail-on-ink"))
                        .cursor_pointer()
                        .hover(|st| st.opacity(0.9))
                        .on_click(move |_, window, cx| {
                            open_dialog(window, cx);
                            // With a checkout that cannot fast-forward, the dialog says why instead.
                            if !problem {
                                store_install.update(cx, |s, cx| s.install(cx));
                            }
                        })
                        .child(t(cx, "update.update")),
                )
                .child(
                    div()
                        .id("hb-update-later")
                        .occlude()
                        .size(px(22.))
                        .rounded(px(11.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |st| st.bg(hover))
                        .on_click(move |_, _, cx| store_dismiss.update(cx, |s, cx| s.dismiss(cx)))
                        .child(icon("x", 14., theme.c("rail-text"))),
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
            banner_mode(Some(&v), None, "", false),
            Some(BannerMode::Available)
        );
        assert_eq!(banner_mode(Some(&v), Some("abc123"), "", false), None);
        assert_eq!(
            banner_mode(Some(&v), Some("older"), "", false),
            Some(BannerMode::Available)
        );
        assert_eq!(banner_mode(None, None, "", false), None);
        assert_eq!(banner_mode(None, None, "", true), Some(BannerMode::Running));
    }

    #[test]
    fn a_run_shows_while_it_goes_and_asks_for_a_restart_when_newer() {
        let mut v = available("abc");
        v.run = Some(UpdateRun {
            state: "running".into(),
            ..Default::default()
        });
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "", false),
            Some(BannerMode::Running)
        );
        v.run = Some(UpdateRun {
            state: "done".into(),
            to: Some("def4567".into()),
            ..Default::default()
        });
        assert_eq!(
            banner_mode(Some(&v), Some("abc"), "abc0000", false),
            Some(BannerMode::Done)
        );
        assert_eq!(banner_mode(Some(&v), Some("abc"), "def4567aa", false), None);
    }
}
