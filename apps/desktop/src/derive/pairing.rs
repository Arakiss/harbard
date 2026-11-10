//! Preferences › Teams (`lib/pairing.ts`): each paired device's standing,
//! its log in words, and serve's refusals.

use crate::i18n::{t, tv};
use crate::model::pairing::{Device, DeviceAction, Invite};
use gpui::{App, SharedString};

/// (role id, label key).
pub const ROLES: [(&str, &str); 3] = [
    ("operator", "teams.roleOperator"),
    ("workshop", "teams.roleWorkshop"),
    ("display", "teams.roleDisplay"),
];

pub fn role_label(role: &str, cx: &App) -> SharedString {
    ROLES
        .iter()
        .find(|r| r.0 == role)
        .map(|r| t(cx, r.1))
        .unwrap_or_else(|| role.to_string().into())
}

/// "0.3.0-alpha.9 · a955a72", and whether the device runs another build.
pub fn device_version(d: &Device, hub: Option<(&str, Option<&str>)>) -> Option<(String, bool)> {
    let version = d.version.as_deref().filter(|v| !v.is_empty());
    let commit = d.commit.as_deref().filter(|c| !c.is_empty());
    if version.is_none() && commit.is_none() {
        return None;
    }
    let text = [version.map(str::to_string), commit.map(|c| c.chars().take(7).collect())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let Some((hub_version, hub_commit)) = hub.filter(|_| !d.hub) else {
        return Some((text, false));
    };
    let behind = match (commit, hub_commit) {
        (Some(c), Some(h)) => !(h.starts_with(c) || c.starts_with(h)),
        _ => version.is_some_and(|v| v != hub_version),
    };
    Some((text, behind))
}

/// How long a pending invite still works.
pub fn expires_line(i: &Invite, now: u64, cx: &App) -> SharedString {
    let Some(exp) = i.expires_ms.filter(|e| *e > 0) else {
        return t(cx, "teams.expiresNever");
    };
    let m = (exp as i64 - now as i64).div_euclid(60_000);
    if m < 1 {
        t(cx, "teams.expiresUnderMinute")
    } else if m < 60 {
        tv(cx, "teams.expiresMinutes", &[("minutes", &m.to_string())])
    } else {
        tv(cx, "teams.expiresHours", &[("hours", &(m / 60).to_string())])
    }
}

/// Operator only once the device's name is typed (case-insensitive, trimmed).
pub fn confirm_matches(typed: &str, name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && typed.trim().to_lowercase() == name.to_lowercase()
}

/// "just now", "12 s ago", "5 min ago", "3 h ago" (`lib/progress.ts` `relative`).
pub fn relative(ms: u64, now: u64, cx: &App) -> SharedString {
    let s = now.saturating_sub(ms) / 1000;
    if s < 5 {
        return t(cx, "progress.relativeNow");
    }
    if s < 60 {
        return tv(cx, "progress.relativeSeconds", &[("count", &s.to_string())]);
    }
    let m = s / 60;
    if m < 60 {
        return tv(cx, "progress.relativeMinutes", &[("count", &m.to_string())]);
    }
    tv(cx, "progress.relativeHours", &[("count", &(m / 60).to_string())])
}

/// Where a device stands: the hub itself, revoked, without credential, or last seen.
pub fn status_line(d: &Device, now: u64, cx: &App) -> SharedString {
    if d.hub {
        return t(cx, "teams.statusThisMac");
    }
    if let Some(r) = d.revoked_ms.filter(|r| *r > 0) {
        return tv(cx, "teams.statusRevoked", &[("when", &relative(r, now, cx))]);
    }
    if !d.credential {
        return t(cx, "teams.statusNoCredential");
    }
    if d.last_seen_ms == 0 {
        return t(cx, "teams.statusNotConnected");
    }
    tv(cx, "teams.statusSeen", &[("when", &relative(d.last_seen_ms, now, cx))])
}

/// The place a path names, as a dictionary key.
pub fn place_key(path: &str) -> Option<&'static str> {
    let p = path.strip_prefix("/api/v1/")?;
    Some(match p {
        "events" => "teams.placeEvents",
        "devices" => "teams.placeCard",
        "summary" => "teams.placeSummary",
        "profile/brief" => "teams.placeContext",
        "profile" | "profile/photo" => "teams.placeProfile",
        "meta" | "health" | "protocol" => "teams.placeHubState",
        "workspace" => "teams.placeDesktop",
        _ if p.starts_with("attachments/") => "teams.placeAttachment",
        _ if p.starts_with("live/") => "teams.placeLive",
        _ if p.starts_with("workspace/") => "teams.placeDesktop",
        _ => return None,
    })
}

/// One log line as words, refusals and errors marked.
pub fn describe_action(a: &DeviceAction, cx: &App) -> SharedString {
    let place: SharedString = place_key(&a.path)
        .map(|k| t(cx, k))
        .unwrap_or_else(|| a.path.clone().into());
    let refused = a.status == 401 || a.status == 403;
    let what = if a.write && a.path == "/api/v1/events" && !refused {
        t(cx, "teams.actionPublishedEvents")
    } else {
        let key = match (a.write, refused) {
            (true, true) => "teams.actionTriedWrite",
            (true, false) => "teams.actionWrote",
            (false, true) => "teams.actionTriedRead",
            (false, false) => "teams.actionRead",
        };
        tv(cx, key, &[("place", &place)])
    };
    if refused {
        return tv(cx, "teams.actionDenied", &[("what", &what)]);
    }
    if a.status >= 400 {
        return tv(
            cx,
            "teams.actionError",
            &[("what", &what), ("status", &a.status.to_string())],
        );
    }
    what
}

/// serve's refusal codes, in the operator's words; unreachable → not responding.
pub fn refusal_text(e: &crate::api::ApiError, cx: &App) -> SharedString {
    if e.unreachable() {
        return t(cx, "teams.notResponding");
    }
    match e.code.as_deref() {
        Some("confirm_required") => t(cx, "teams.refusalConfirmRequired"),
        Some("revoked") => t(cx, "teams.refusalRevoked"),
        Some("hub_itself") => t(cx, "teams.refusalHubItself"),
        Some("forbidden_origin") => t(cx, "teams.refusalForbiddenOrigin"),
        Some("unknown_invite") => t(cx, "teams.refusalUnknownInvite"),
        _ => tv(
            cx,
            "teams.refusalDefault",
            &[("status", &e.status.unwrap_or(0).to_string())],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(version: Option<&str>, commit: Option<&str>) -> Device {
        Device {
            version: version.map(str::to_string),
            commit: commit.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn versions_compare_by_commit_then_number() {
        let hub = Some(("0.3.0", Some("a955a72bb")));
        assert_eq!(device_version(&dev(None, None), hub), None);
        assert_eq!(
            device_version(&dev(Some("0.3.0"), Some("a955a72bbcc")), hub),
            Some(("0.3.0 · a955a72".into(), false))
        );
        assert_eq!(
            device_version(&dev(Some("0.3.0"), Some("1234567")), hub).unwrap().1,
            true
        );
        assert_eq!(device_version(&dev(Some("0.2.0"), None), hub).unwrap().1, true);
        assert_eq!(device_version(&dev(Some("0.2.0"), None), None).unwrap().1, false);
    }

    #[test]
    fn operator_needs_the_name() {
        assert!(confirm_matches("  mac mini ", "Mac Mini"));
        assert!(!confirm_matches("mac", "Mac Mini"));
        assert!(!confirm_matches("", " "));
    }

    #[test]
    fn places_by_path() {
        assert_eq!(place_key("/api/v1/events"), Some("teams.placeEvents"));
        assert_eq!(place_key("/api/v1/profile/photo"), Some("teams.placeProfile"));
        assert_eq!(place_key("/api/v1/workspace/reply"), Some("teams.placeDesktop"));
        assert_eq!(place_key("/api/v1/live/x"), Some("teams.placeLive"));
        assert_eq!(place_key("/api/v1/other"), None);
    }
}
