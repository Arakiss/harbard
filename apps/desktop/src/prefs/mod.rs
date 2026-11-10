//! Preferences and the operator's dialogs. Owned by the prefs lane; other
//! lanes open these only through the functions below (the contract).

use gpui::{App, Window};

/// Open Preferences, optionally scrolled to a section (`"themes"`, `"pairing"`…).
pub fn open_preferences(section: Option<&str>, window: &mut Window, cx: &mut App) {
    let _ = (section, window, cx);
}

/// The update dialog ("Update harbard…", what's new, install).
pub fn open_update_dialog(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// "About harbard".
pub fn open_about(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// Profile and context (photo, name, hours, context).
pub fn open_profile(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// The status message dialog.
pub fn open_status(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// "<workspace> settings".
pub fn open_workspace_settings(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}
