//! Preferences and the operator's dialogs. Owned by the prefs lane; other
//! lanes open these only through the functions below (the contract).
//!
//! Besides the dialogs this module keeps two shared reads, installed by
//! [`init`]: [`ProfileStore`] (the profile, for the rail's avatar and menu)
//! and [`UpdateStore`] (the update state, for the title bar's pill and the
//! menu's dot).
//!
//! While testing, `HARBARD_APP_OPEN` opens one dialog once at startup:
//! `preferences`, `preferences:<page>` (appearance, general, notifications,
//! teams, updates), `update`, `about`, `profile`, `status` or `workspace`;
//! `HARBARD_APP_DRIVE` then plays input into the window (see `drive.rs`).

mod about;
mod appearance;
mod drive;
mod general;
mod os;
mod pairing;
mod preferences;
mod profile_dialog;
mod status_dialog;
mod stores;
mod update_dialog;
mod update_pill;
mod updates_page;
mod widgets;
mod workspace_settings;

// The shared reads and hooks other lanes use (the rail, the title bar, the Strip).
#[allow(unused_imports)]
pub use general::StripInfo;
#[allow(unused_imports)]
pub use stores::{ProfileStore, UpdateStore, app_commit};
#[allow(unused_imports)]
pub use update_pill::update_pill;

use gpui::{App, Window};
use std::time::Duration;

/// Install the shared reads; open the dialog `HARBARD_APP_OPEN` names.
pub fn init(cx: &mut App) {
    stores::ProfileStore::init(cx);
    stores::UpdateStore::init(cx);
    if let Ok(what) = std::env::var("HARBARD_APP_OPEN") {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(600)).await;
            cx.update(|cx| {
                let Some(window) = cx.windows().into_iter().next() else {
                    return;
                };
                let _ = window.update(cx, |_, window, cx| open_named(&what, window, cx));
            });
        })
        .detach();
    }
    if let Ok(script) = std::env::var("HARBARD_APP_DRIVE") {
        drive::start(script, cx);
    }
}

/// Open a dialog by name (the debug opener's vocabulary).
pub fn open_named(what: &str, window: &mut Window, cx: &mut App) {
    let (name, arg) = what.split_once(':').unwrap_or((what, ""));
    match name {
        "preferences" | "prefs" => open_preferences((!arg.is_empty()).then_some(arg), window, cx),
        "update" => open_update_dialog(window, cx),
        "about" => open_about(window, cx),
        "profile" => open_profile(window, cx),
        "status" => open_status(window, cx),
        "workspace" => open_workspace_settings(window, cx),
        _ => {}
    }
}

/// Open Preferences, optionally on a page (`"themes"`, `"general"`,
/// `"notifications"`, `"teams"`/`"pairing"`, `"updates"`).
pub fn open_preferences(section: Option<&str>, window: &mut Window, cx: &mut App) {
    preferences::open(section, window, cx);
}

/// The update dialog ("Update harbard…", what's new, install).
pub fn open_update_dialog(window: &mut Window, cx: &mut App) {
    update_dialog::open(window, cx);
}

/// The tray's or a notification's "Update…": the dialog, after a fresh look
/// when the last one is older than five minutes.
pub fn open_update_from_tray(window: &mut Window, cx: &mut App) {
    UpdateStore::check_if_stale(cx);
    update_dialog::open(window, cx);
}

/// "About harbard".
pub fn open_about(window: &mut Window, cx: &mut App) {
    about::open(window, cx);
}

/// Profile and context (photo, name, hours, context).
pub fn open_profile(window: &mut Window, cx: &mut App) {
    profile_dialog::open(window, cx);
}

/// The status message dialog.
pub fn open_status(window: &mut Window, cx: &mut App) {
    status_dialog::open(window, cx);
}

/// "<workspace> settings".
pub fn open_workspace_settings(window: &mut Window, cx: &mut App) {
    workspace_settings::open(window, cx);
}
