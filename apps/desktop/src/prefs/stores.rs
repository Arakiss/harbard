//! What Preferences and the dialogs add to the shell's two stores
//! ([`ProfileStore`] and [`UpdateStore`], see `shell_stores`): the
//! operator's photo, the update preferences, and the tray's stale check.

use super::shell_stores::profile::ProfileStore;
use super::shell_stores::update::UpdateStore;
use crate::api::{self, ApiError};
use crate::model::profile::ProfileView;
use crate::model::update::UpdateView;
use gpui::{App, AppContext, Context, Entity, Global, Image, ImageFormat, Task};
use serde_json::Value;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// The photo
// ---------------------------------------------------------------------------

/// The operator's photo, fetched again whenever the profile's photo
/// revision moves.
pub struct ProfilePhoto {
    pub image: Option<Arc<Image>>,
    rev: Option<u64>,
}

#[derive(Clone)]
struct PhotoHandle(Entity<ProfilePhoto>);

impl Global for PhotoHandle {}

fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}

impl ProfilePhoto {
    pub fn get(cx: &App) -> Entity<ProfilePhoto> {
        cx.global::<PhotoHandle>().0.clone()
    }

    pub(super) fn init(cx: &mut App) {
        let profile = ProfileStore::get(cx);
        let photo = cx.new(|cx: &mut Context<ProfilePhoto>| {
            cx.observe(&profile, |this, profile, cx| {
                let view = profile.read(cx).view.clone();
                this.follow(view.as_ref(), cx);
            })
            .detach();
            let view = profile.read(cx).view.clone();
            let mut this = ProfilePhoto { image: None, rev: None };
            this.follow(view.as_ref(), cx);
            this
        });
        cx.set_global(PhotoHandle(photo));
    }

    fn follow(&mut self, view: Option<&ProfileView>, cx: &mut Context<Self>) {
        let Some(view) = view else { return };
        let rev = view.profile.photo.as_ref().map(|_| view.profile.photo_rev);
        if rev == self.rev {
            return;
        }
        self.rev = rev;
        let Some(rev) = rev else {
            self.image = None;
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let bytes = cx
                .background_spawn(async move { api::get_bytes(&format!("/profile/photo?v={rev}")) })
                .await;
            if let Ok(bytes) = bytes {
                let image = Arc::new(Image::from_bytes(image_format(&bytes), bytes));
                let _ = this.update(cx, |s, cx| {
                    s.image = Some(image);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// `POST /profile/photo` with the raw bytes; serve's new view goes into
    /// the profile store.
    pub fn upload(bytes: Vec<u8>, mime: &'static str, cx: &mut App) -> Task<Result<ProfileView, ApiError>> {
        let profile = ProfileStore::get(cx);
        cx.spawn(async move |cx| {
            let out = cx
                .background_spawn(async move {
                    api::post_bytes("/profile/photo", &bytes, mime).and_then(decode::<ProfileView>)
                })
                .await;
            if let Ok(view) = &out {
                let view = view.clone();
                profile.update(cx, |p, cx| {
                    p.view = Some(view);
                    cx.notify();
                });
            }
            out
        })
    }
}

#[allow(clippy::result_large_err)]
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError {
        status: Some(200),
        code: None,
        message: e.to_string(),
        body: Value::Null,
    })
}

/// `POST /profile` through the profile store.
pub fn update_profile(patch: Value, cx: &mut App) -> Task<Result<ProfileView, ApiError>> {
    ProfileStore::get(cx).update(cx, |p, cx| p.update(patch, cx))
}

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

/// The commit this app was built from, for About.
pub fn app_commit() -> String {
    harbard_cli::serve::workspace_route::running_commit().unwrap_or_default()
}

/// A run is going (or serve is restarting for one).
pub fn update_running(s: &UpdateStore) -> bool {
    s.view
        .as_ref()
        .and_then(|v| v.run.as_ref())
        .is_some_and(|r| r.state == "running")
        || s.installing
        || s.restarting
}

pub fn check_updates(cx: &mut App) {
    UpdateStore::get(cx).update(cx, |s, cx| s.check(cx));
}

pub fn install_update(cx: &mut App) {
    UpdateStore::get(cx).update(cx, |s, cx| s.install(cx));
}

pub fn dismiss_update(cx: &mut App) {
    UpdateStore::get(cx).update(cx, |s, cx| s.dismiss(cx));
}

/// `POST /update/prefs {channel?, auto_check?}`; serve's answer goes into
/// the update store (a new channel checks again, so it shows as checking).
pub fn set_update_prefs(body: Value, cx: &mut App) {
    let store = UpdateStore::get(cx);
    let channel = body.get("channel").is_some();
    store.update(cx, |s, cx| {
        s.error = None;
        s.checking |= channel;
        cx.notify();
    });
    cx.spawn(async move |cx| {
        let out = cx
            .background_spawn(async move { api::post("/update/prefs", &body).and_then(decode::<UpdateView>) })
            .await;
        store.update(cx, |s, cx| {
            s.checking = false;
            match out {
                Ok(v) => s.view = Some(v),
                Err(e) => {
                    s.error = Some(super::shell_stores::update::error_text(
                        e.code.as_deref(),
                        &e.message,
                        cx,
                    ))
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// The tray's "Update…": look again first when the last look is stale.
pub fn check_if_stale(cx: &mut App) {
    let store = UpdateStore::get(cx).read(cx);
    let checked = store.last_check().map_or(0, |c| c.checked_ms);
    if !update_running(store) && harbard_core::clock::now_ms().saturating_sub(checked) > crate::derive::update::FRESH_MS
    {
        check_updates(cx);
    }
}
