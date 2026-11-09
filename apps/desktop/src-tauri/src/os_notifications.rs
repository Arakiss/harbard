//! Whether macOS lets harbard notify and badge its Dock icon.
//!
//! macOS drops the Dock badge and every banner without an error when the
//! operator has notifications off for the app, and the app cannot tell from
//! the calls that fail silently. Asking the notification centre is the only
//! way to say so in Preferences.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OsNotifications {
    /// The app may show notifications at all.
    pub allowed: bool,
    /// The Dock badge is allowed too.
    pub badge: bool,
}

/// The state, or `None` where it cannot be read: not macOS, not a bundled
/// app (the notification centre throws without one), or no answer in time.
#[cfg(target_os = "macos")]
pub fn read() -> Option<OsNotifications> {
    use block2::RcBlock;
    use objc2_foundation::NSBundle;
    use objc2_user_notifications::{
        UNAuthorizationStatus, UNNotificationSetting, UNNotificationSettings,
        UNUserNotificationCenter,
    };
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::Duration;

    if !NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".app")
    {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let done = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the framework passes a valid object for the block's call.
        let settings = unsafe { settings.as_ref() };
        let status = settings.authorizationStatus();
        let _ = tx.send(OsNotifications {
            allowed: status == UNAuthorizationStatus::Authorized
                || status == UNAuthorizationStatus::Provisional,
            badge: settings.badgeSetting() == UNNotificationSetting::Enabled,
        });
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&done);
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

#[cfg(not(target_os = "macos"))]
pub fn read() -> Option<OsNotifications> {
    None
}

/// Open harbard's page in System Settings. The switch itself is the operator's.
pub fn open_settings(identifier: &str) -> Result<(), String> {
    let url = format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={identifier}"
    );
    std::process::Command::new("open")
        .arg(url)
        .status()
        .map_err(|e| e.to_string())
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err("open failed".to_string())
            }
        })
}
