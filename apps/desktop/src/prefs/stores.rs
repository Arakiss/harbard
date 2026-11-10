//! The two reads every operator surface shares: the profile (the rail's
//! avatar and menu, Profile, Status, Notifications) and the update state
//! (the title bar's pill, the menu's dot, About, Preferences, the dialog).
//! Each is one entity installed as a global; screens read it in `render`
//! and observe it.

use crate::api::{self, ApiError};
use crate::derive::update::{BannerMode, IDLE_POLL_MS, RUN_POLL_MS, banner_mode, error_text, toast_for};
use crate::model::profile::ProfileView;
use crate::model::update::{UpdateError, UpdateView};
use gpui::{App, AppContext, Context, Entity, Global, Image, ImageFormat, SharedString, Task};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

/// How often the profile is read again (the console's 30 s).
const PROFILE_POLL: Duration = Duration::from_secs(30);

pub struct ProfileStore {
    pub view: Option<ProfileView>,
    /// The operator's photo, when one is set.
    pub photo: Option<Arc<Image>>,
    photo_rev: Option<u64>,
    _poll: Task<()>,
}

#[derive(Clone)]
struct ProfileHandle(Entity<ProfileStore>);

impl Global for ProfileHandle {}

fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}

impl ProfileStore {
    pub fn get(cx: &App) -> Entity<ProfileStore> {
        cx.global::<ProfileHandle>().0.clone()
    }

    pub(super) fn init(cx: &mut App) {
        let store = cx.new(|cx: &mut Context<ProfileStore>| {
            let poll = cx.spawn(async move |this, cx| {
                loop {
                    let read = cx
                        .background_spawn(async { api::get_as::<ProfileView>("/profile") })
                        .await;
                    if this.update(cx, |s, cx| s.apply(read.ok(), cx)).is_err() {
                        break;
                    }
                    cx.background_executor().timer(PROFILE_POLL).await;
                }
            });
            ProfileStore {
                view: None,
                photo: None,
                photo_rev: None,
                _poll: poll,
            }
        });
        cx.set_global(ProfileHandle(store));
    }

    /// Take a fresh view; fetch the photo again when its revision moved.
    fn apply(&mut self, view: Option<ProfileView>, cx: &mut Context<Self>) {
        let Some(view) = view else { return };
        let rev = view.profile.photo.as_ref().map(|_| view.profile.photo_rev);
        if rev != self.photo_rev {
            self.photo_rev = rev;
            match rev {
                None => self.photo = None,
                Some(rev) => {
                    cx.spawn(async move |this, cx| {
                        let bytes = cx
                            .background_spawn(async move { api::get_bytes(&format!("/profile/photo?v={rev}")) })
                            .await;
                        if let Ok(bytes) = bytes {
                            let image = Arc::new(Image::from_bytes(image_format(&bytes), bytes));
                            let _ = this.update(cx, |s, cx| {
                                s.photo = Some(image);
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                }
            }
        }
        if self.view.as_ref() != Some(&view) {
            self.view = Some(view);
            cx.notify();
        }
    }

    /// Read again now.
    pub fn reload(cx: &mut App) {
        let this = Self::get(cx);
        cx.spawn(async move |cx| {
            let read = cx
                .background_spawn(async { api::get_as::<ProfileView>("/profile") })
                .await;
            this.update(cx, |s, cx| s.apply(read.ok(), cx));
        })
        .detach();
    }

    /// `POST /profile` (all or nothing; `null` clears a field), with this
    /// Mac's UTC offset so serve words times in the operator's clock.
    pub fn update(patch: Value, cx: &mut App) -> Task<Result<ProfileView, ApiError>> {
        let this = Self::get(cx);
        let mut body =
            json!({ "utc_offset_min": crate::derive::profile::utc_offset_min(harbard_core::clock::now_ms()) });
        if let (Some(b), Some(p)) = (body.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                b.insert(k.clone(), v.clone());
            }
        }
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async move { api::post("/profile", &body).and_then(parse_view) })
                .await;
            if let Ok(view) = &out {
                let view = view.clone();
                this.update(cx, |s, cx| s.apply(Some(view), cx));
            }
            out
        })
    }

    /// `POST /profile/photo` with the raw bytes.
    pub fn upload_photo(bytes: Vec<u8>, mime: &'static str, cx: &mut App) -> Task<Result<ProfileView, ApiError>> {
        let this = Self::get(cx);
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async move { api::post_bytes("/profile/photo", &bytes, mime).and_then(parse_view) })
                .await;
            if let Ok(view) = &out {
                let view = view.clone();
                this.update(cx, |s, cx| s.apply(Some(view), cx));
            }
            out
        })
    }
}

fn parse_view(v: Value) -> Result<ProfileView, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError {
        status: Some(200),
        code: None,
        message: e.to_string(),
        body: Value::Null,
    })
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

const DISMISS_KEY: &str = "update_dismissed";
const ACK_KEY: &str = "update_acked_run";

pub struct UpdateStore {
    pub view: Option<UpdateView>,
    /// An older serve without the route: every update surface hides.
    pub unsupported: bool,
    /// A read failed while a run was going: serve is restarting.
    pub restarting: bool,
    pub checking: bool,
    pub installing: bool,
    /// The run this app started or watched.
    pub followed: Option<String>,
    pub error: Option<SharedString>,
    dismissed: Option<String>,
    /// A run finished minutes ago that the operator was not told about yet.
    pub toast: Option<String>,
    _poll: Task<()>,
}

#[derive(Clone)]
struct UpdateHandle(Entity<UpdateStore>);

impl Global for UpdateHandle {}

/// The commit this app was built from.
pub fn app_commit() -> String {
    harbard_cli::serve::workspace_route::running_commit().unwrap_or_default()
}

fn desk_str(key: &str) -> Option<String> {
    crate::desk::desk_prefs()
        .rest
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

impl UpdateStore {
    pub fn get(cx: &App) -> Entity<UpdateStore> {
        cx.global::<UpdateHandle>().0.clone()
    }

    pub(super) fn init(cx: &mut App) {
        let store = cx.new(|cx: &mut Context<UpdateStore>| {
            let poll = cx.spawn(async move |this, cx| {
                let mut first = true;
                loop {
                    let read = cx
                        .background_spawn(async { api::get_as::<UpdateView>("/update") })
                        .await;
                    let Ok(fast) = this.update(cx, |s, cx| {
                        s.apply(read, first, cx);
                        s.running()
                    }) else {
                        break;
                    };
                    first = false;
                    let every = if fast { RUN_POLL_MS } else { IDLE_POLL_MS };
                    cx.background_executor().timer(Duration::from_millis(every)).await;
                }
            });
            UpdateStore {
                view: None,
                unsupported: false,
                restarting: false,
                checking: false,
                installing: false,
                followed: None,
                error: None,
                dismissed: desk_str(DISMISS_KEY),
                toast: None,
                _poll: poll,
            }
        });
        cx.set_global(UpdateHandle(store));
    }

    /// A run is (or may be) going on: read fast, and a failure is a restart.
    pub fn running(&self) -> bool {
        self.installing
            || self.restarting
            || self
                .view
                .as_ref()
                .and_then(|v| v.run.as_ref())
                .is_some_and(|r| r.state == "running")
    }

    fn apply(&mut self, read: Result<UpdateView, ApiError>, first: bool, cx: &mut Context<Self>) {
        match read {
            Ok(next) => {
                if let Some(run) = next.run.as_ref().filter(|r| r.state == "running") {
                    self.followed = Some(run.id.clone());
                }
                self.unsupported = false;
                self.restarting = false;
                if next.run.as_ref().is_none_or(|r| r.state != "running") {
                    self.installing = false;
                }
                if first {
                    let acked = desk_str(ACK_KEY);
                    if let Some(run) = next.run.as_ref()
                        && toast_for(Some(run), harbard_core::clock::now_ms(), acked.as_deref())
                    {
                        let id = run.id.clone();
                        crate::desk::save_desk_prefs(|p| {
                            p.rest.insert(ACK_KEY.into(), Value::String(id.clone()));
                        });
                        self.toast = Some(next.installed.version.clone());
                    }
                }
                self.view = Some(next);
            }
            Err(e) => {
                if self.running() {
                    self.restarting = true;
                } else if e.missing() {
                    self.unsupported = true;
                }
            }
        }
        cx.notify();
    }

    pub fn available(&self) -> bool {
        !self.unsupported
            && self
                .view
                .as_ref()
                .and_then(|v| v.check.as_ref())
                .is_some_and(|c| c.status == "available")
    }

    /// What the title bar's pill shows.
    pub fn banner(&self) -> Option<BannerMode> {
        if self.unsupported {
            return None;
        }
        banner_mode(
            self.view.as_ref(),
            self.dismissed.as_deref(),
            &app_commit(),
            self.restarting || self.installing,
        )
    }

    fn refused(e: &ApiError, cx: &App) -> SharedString {
        let err = UpdateError {
            code: e.code.clone().unwrap_or_default(),
            message: e.message.clone(),
        };
        error_text(Some(&err), cx)
    }

    /// `POST /update/check` (up to ~30 s).
    pub fn check(cx: &mut App) {
        let this = Self::get(cx);
        this.update(cx, |s, cx| {
            s.checking = true;
            s.error = None;
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async { api::post("/update/check", &json!({})) })
                .await;
            this.update(cx, |s, cx| {
                s.checking = false;
                match out.and_then(parse_update) {
                    Ok(v) => s.view = Some(v),
                    Err(e) => s.error = Some(Self::refused(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `POST /update/install`: builds and restarts harbard. Never call it
    /// from a test.
    pub fn install(cx: &mut App) {
        let this = Self::get(cx);
        this.update(cx, |s, cx| {
            s.installing = true;
            s.error = None;
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async { api::post("/update/install", &json!({})) })
                .await;
            this.update(cx, |s, cx| {
                match out.and_then(parse_update) {
                    Ok(v) => {
                        if let Some(run) = &v.run {
                            s.followed = Some(run.id.clone());
                        }
                        s.view = Some(v);
                    }
                    // One already runs: follow it on the next read.
                    Err(e) if e.code.as_deref() == Some("busy") => {}
                    Err(e) => {
                        s.installing = false;
                        s.error = Some(Self::refused(&e, cx));
                    }
                }
                cx.notify();
            });
            Self::reread(this, cx).await;
        })
        .detach();
    }

    async fn reread(this: Entity<UpdateStore>, cx: &mut gpui::AsyncApp) {
        let read = cx
            .background_spawn(async { api::get_as::<UpdateView>("/update") })
            .await;
        this.update(cx, |s, cx| s.apply(read, false, cx));
    }

    /// `POST /update/prefs {channel?, auto_check?}`.
    pub fn set_prefs(body: Value, cx: &mut App) {
        let this = Self::get(cx);
        let channel = body.get("channel").is_some();
        this.update(cx, |s, cx| {
            s.error = None;
            s.checking |= channel;
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async move { api::post("/update/prefs", &body) })
                .await;
            this.update(cx, |s, cx| {
                s.checking = false;
                match out.and_then(parse_update) {
                    Ok(v) => s.view = Some(v),
                    Err(e) => s.error = Some(Self::refused(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Later": hide the pill until a newer target.
    pub fn dismiss(cx: &mut App) {
        let this = Self::get(cx);
        this.update(cx, |s, cx| {
            let Some(target) = s
                .view
                .as_ref()
                .and_then(|v| v.check.as_ref())
                .and_then(|c| c.target.as_ref())
            else {
                return;
            };
            let commit = target.commit.clone();
            crate::desk::save_desk_prefs(|p| {
                p.rest.insert(DISMISS_KEY.into(), Value::String(commit.clone()));
            });
            s.dismissed = Some(commit);
            cx.notify();
        });
    }

    /// The tray's "Update…": look again first when the last look is stale.
    pub fn check_if_stale(cx: &mut App) {
        let store = Self::get(cx).read(cx);
        let checked = store
            .view
            .as_ref()
            .and_then(|v| v.check.as_ref())
            .map_or(0, |c| c.checked_ms);
        let running = store.running();
        if !running && harbard_core::clock::now_ms().saturating_sub(checked) > crate::derive::update::FRESH_MS {
            Self::check(cx);
        }
    }
}

fn parse_update(v: Value) -> Result<UpdateView, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError {
        status: Some(200),
        code: None,
        message: e.to_string(),
        body: Value::Null,
    })
}
