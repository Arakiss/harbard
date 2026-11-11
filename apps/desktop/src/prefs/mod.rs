//! Preferences and the operator's dialogs. Owned by the prefs lane; other
//! lanes open these only through the functions below (the contract).
//!
//! The profile and the update state are the shell's stores
//! (`shell::profile`, `shell::update`). [`init`] adds the operator's photo on
//! top (`stores::ProfilePhoto`).
//!
//! While testing, `HARBARD_APP_OPEN` opens one dialog once at startup:
//! `preferences`, `preferences:<page>` (appearance, general, notifications,
//! teams, updates), `update`, `about`, `profile`, `status`, `workspace` or
//! `new-workspace`;
//! `HARBARD_APP_DRIVE_SCRIPT` then plays an inline input script into the window
//! (see `prefs/drive.rs`; `HARBARD_APP_DRIVE` reads a script file, `src/drive.rs`).

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
mod updates_page;
mod widgets;
mod workspace_dialogs;
mod workspace_settings;

// The shell's stores, by the names the dialogs use; `crate::shell::{profile, update}` once it is in.
pub(crate) use crate::shell::{profile as shell_profile, update as shell_update};

use gpui::{App, Window};
use std::time::Duration;

/// Install the photo (and, for now, the shell's stores); open the dialog `HARBARD_APP_OPEN` names.
pub fn init(cx: &mut App) {
    // Goes once the shell, which installs its own stores, is in.
    stores::ProfilePhoto::init(cx);
    if let Ok(what) = std::env::var("HARBARD_APP_OPEN") {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            cx.update(|cx| {
                let Some(window) = cx.windows().into_iter().next() else {
                    return;
                };
                let _ = window.update(cx, |_, window, cx| open_named(&what, window, cx));
            });
        })
        .detach();
    }
    if let Ok(script) = std::env::var("HARBARD_APP_DRIVE_SCRIPT") {
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
        "new-workspace" => open_new_workspace(window, cx),
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
    stores::check_if_stale(cx);
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

/// "New workspace": a name and, if wanted, the owners whose repositories
/// join it; the app goes to it once created.
pub fn open_new_workspace(window: &mut Window, cx: &mut App) {
    workspace_dialogs::open_new(window, cx);
}
