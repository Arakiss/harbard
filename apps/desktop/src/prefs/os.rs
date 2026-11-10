//! What macOS lets harbard do, read for Preferences: notifications and the
//! Dock badge (`UNUserNotificationCenter`), and the system's "Reduce motion".

/// The app may show notifications, and badge its Dock icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsNotifications {
    pub allowed: bool,
    pub badge: bool,
}

/// The state, or `None` where it cannot be read: not a bundled app (the
/// notification centre throws without one), or no answer in time. Blocks up
/// to two seconds: call it off the UI thread.
#[cfg(target_os = "macos")]
pub fn notifications() -> Option<OsNotifications> {
    use block2::RcBlock;
    use objc2_foundation::NSBundle;
    use objc2_user_notifications::{
        UNAuthorizationStatus, UNNotificationSetting, UNNotificationSettings, UNUserNotificationCenter,
    };
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::Duration;

    if !NSBundle::mainBundle().bundlePath().to_string().ends_with(".app") {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let done = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: the framework passes a valid object for the block's call.
        let settings = unsafe { settings.as_ref() };
        let status = settings.authorizationStatus();
        let _ = tx.send(OsNotifications {
            allowed: status == UNAuthorizationStatus::Authorized || status == UNAuthorizationStatus::Provisional,
            badge: settings.badgeSetting() == UNNotificationSetting::Enabled,
        });
    });
    UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&done);
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

#[cfg(not(target_os = "macos"))]
pub fn notifications() -> Option<OsNotifications> {
    None
}

/// The bundle's identifier, else the app's own.
fn identifier() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Some(id) = objc2_foundation::NSBundle::mainBundle().bundleIdentifier() {
            return id.to_string();
        }
    }
    "com.harbard.desktop".into()
}

/// Open harbard's page in System Settings › Notifications. The switch itself
/// is the operator's.
pub fn open_notification_settings() {
    let url = format!(
        "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id={}",
        identifier()
    );
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// macOS "Reduce motion" (Accessibility › Display). Spawns `defaults`: call
/// it off the UI thread.
pub fn reduce_motion() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}
