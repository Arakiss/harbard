//! The operator's profile (`profile-state.tsx`, `lib/profile.ts`): one
//! reader of `GET /api/v1/profile` for the whole app, and its writes.
//!
//! [`ProfileStore`] is a global entity. It reads at start, whenever the main
//! window comes to the front, and every 30 s (a status or "Do not disturb"
//! can end while nothing else moves). Writes go through [`ProfileStore::update`],
//! which stamps the machine's UTC offset as the console did, so agents read
//! times in the operator's clock. It also holds "While you were away"
//! ([`Away`]): an unread digest, one a write returned, or what Away holds so
//! far when the operator comes back while still Away.

use crate::api::{self, ApiError};
use crate::i18n::{t, tv};
use crate::model::profile::{DigestItem, Digests, PendingDigest, ProfileHours, ProfileStatus, ProfileView};
use crate::ui::format::{local_date, local_hm};
use gpui::{App, AppContext, Context, Entity, Global, SharedString, Task};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Duration;

const MIN: u64 = 60_000;
/// A status or "Do not disturb" can end while nothing else moves.
const REFRESH: Duration = Duration::from_secs(30);
/// Away this long before coming back opens "While you were away" while still Away.
const RETURN_AFTER_MS: u64 = 2 * MIN;
/// "Do not disturb" lasts at most this long.
const DND_MAX_MS: u64 = 14 * 24 * 60 * MIN;

/// What "While you were away" shows: a cut digest, or what Away holds so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Away {
    /// The digest's id when it was cut (marked read on close).
    pub digest: Option<String>,
    pub state: String,
    pub since: u64,
    pub until: u64,
    pub items: Vec<DigestItem>,
    /// Still in that state: the card offers "Set me to Available".
    pub still_away: bool,
}

pub struct ProfileStore {
    /// `None` until serve answers (or while it does not).
    pub view: Option<ProfileView>,
    pub away: Option<Away>,
    shown: HashSet<String>,
    greeted: Option<u64>,
    loading: bool,
}

#[derive(Clone)]
struct Handle(Entity<ProfileStore>);

impl Global for Handle {}

impl ProfileStore {
    pub fn get(cx: &App) -> Entity<ProfileStore> {
        cx.global::<Handle>().0.clone()
    }

    pub fn init(cx: &mut App) {
        let store = cx.new(|cx: &mut Context<ProfileStore>| {
            cx.spawn(async move |this, cx| {
                loop {
                    let alive = this.update(cx, |p, cx| p.refresh(cx));
                    if alive.is_err() {
                        break;
                    }
                    cx.background_executor().timer(REFRESH).await;
                }
            })
            .detach();
            ProfileStore {
                view: None,
                away: None,
                shown: HashSet::new(),
                greeted: None,
                loading: false,
            }
        });
        cx.set_global(Handle(store));
    }

    /// Read the profile again (and, outside Swamped, the unread digests).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        let task = cx.background_spawn(async { api::get_as::<ProfileView>("/profile") });
        cx.spawn(async move |this, cx| {
            let got = task.await;
            let _ = this.update(cx, |p, cx| {
                p.loading = false;
                if let Ok(view) = got {
                    if p.view.as_ref() != Some(&view) {
                        p.view = Some(view);
                        cx.notify();
                    }
                    p.look_for_digests(cx);
                }
            });
        })
        .detach();
    }

    /// An unread digest (hourly, or cut elsewhere) less than a day old opens
    /// on its own, except in Swamped: calm mode shows one thing only.
    fn look_for_digests(&mut self, cx: &mut Context<Self>) {
        if self.view.is_none() || self.state_id(now_ms()) == "swamped" {
            return;
        }
        let task = cx.background_spawn(async { api::get_as::<Digests>("/digests") });
        cx.spawn(async move |this, cx| {
            if let Ok(all) = task.await {
                let now = now_ms();
                let last = all
                    .items
                    .into_iter()
                    .rfind(|d| !d.read && now.saturating_sub(d.to_ms) < 24 * 60 * MIN);
                if let Some(d) = last {
                    let _ = this.update(cx, |p, cx| {
                        p.show_digest(d.id, d.state, d.from_ms, d.to_ms, d.items, cx)
                    });
                }
            }
        })
        .detach();
    }

    fn show_digest(&mut self, id: String, state: String, since: u64, until: u64, items: Vec<DigestItem>, cx: &mut Context<Self>) {
        if !self.shown.insert(id.clone()) {
            return;
        }
        self.away = Some(Away {
            digest: Some(id),
            state,
            since,
            until,
            items,
            still_away: false,
        });
        cx.notify();
    }

    /// The main window came to the front: read again, and greet a return
    /// while still Away (once per Away, after two minutes of it).
    pub fn window_activated(&mut self, cx: &mut Context<Self>) {
        self.refresh(cx);
        let now = now_ms();
        let Some(since) = self.view.as_ref().and_then(|v| v.profile.state.as_ref()).map(|s| s.since_ms) else {
            return;
        };
        if self.state_id(now) != "away" || self.greeted == Some(since) || now.saturating_sub(since) < RETURN_AFTER_MS {
            return;
        }
        self.greeted = Some(since);
        let task = cx.background_spawn(async { api::get_as::<PendingDigest>("/digests/pending") });
        cx.spawn(async move |this, cx| {
            if let Ok(p) = task.await {
                let _ = this.update(cx, |store, cx| {
                    store.away = Some(Away {
                        digest: None,
                        state: "away".into(),
                        since,
                        until: now_ms(),
                        items: p.items,
                        still_away: true,
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Close "While you were away"; a digest is marked read.
    pub fn close_away(&mut self, cx: &mut Context<Self>) {
        if let Some(Away { digest: Some(id), .. }) = self.away.take() {
            cx.background_spawn(async move {
                let _ = api::post("/digests/read", &json!({ "id": id }));
            })
            .detach();
        }
        cx.notify();
    }

    /// `POST /api/v1/profile` with the patch (all or nothing). Resolves to
    /// serve's new view, already applied here.
    pub fn update(&mut self, patch: Value, cx: &mut Context<Self>) -> Task<Result<ProfileView, ApiError>> {
        let changes_state = patch.get("state").is_some();
        let mut body = json!({ "utc_offset_min": utc_offset_min(now_ms()) });
        if let (Some(obj), Some(p)) = (body.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        let task = cx.background_spawn(async move { api::post("/profile", &body) });
        cx.spawn(async move |this, cx| {
            let answer = task.await?;
            let view: ProfileView = serde_json::from_value(answer).map_err(|e| ApiError {
                status: Some(200),
                code: None,
                message: e.to_string(),
                body: Value::Null,
            })?;
            let _ = this.update(cx, |p, cx| {
                p.view = Some(view.clone());
                if let Some(d) = view.digest.clone() {
                    p.show_digest(d.id, d.state, d.from_ms, d.to_ms, d.items, cx);
                } else if changes_state && p.away.as_ref().is_some_and(|a| a.still_away) {
                    p.away = None;
                }
                cx.notify();
            });
            Ok(view)
        })
    }

    /// The coordination state in force at `now`.
    pub fn state_id(&self, now: u64) -> &str {
        state_of(self.view.as_ref(), now)
    }

    /// The status message in force at `now`.
    pub fn status(&self, now: u64) -> Option<&ProfileStatus> {
        let s = self.view.as_ref()?.status.as_ref()?;
        match s.clear_at_ms {
            Some(at) if now >= at => None,
            _ => Some(s),
        }
    }

    /// "Do not disturb" in force at `now`.
    pub fn quiet(&self, now: u64) -> bool {
        let Some(v) = &self.view else { return false };
        v.quiet && v.quiet_until_ms.is_none_or(|u| now < u)
    }

    /// The Dock's one-shots (and the logo's): off under DND and the quiet states.
    pub fn one_shots(&self, now: u64) -> bool {
        one_shots_allowed(self.state_id(now), self.quiet(now))
    }

    /// The desktop's away suggestion is on unless the operator turned it off.
    pub fn away_suggest(&self) -> bool {
        self.view.as_ref().and_then(|v| v.profile.away_suggest).unwrap_or(true)
    }
}

pub fn init(cx: &mut App) {
    ProfileStore::init(cx);
}

// ---------------------------------------------------------------------------
// Pure wording and moments (lib/profile.ts)
// ---------------------------------------------------------------------------

pub fn now_ms() -> u64 {
    harbard_core::clock::now_ms()
}

/// One coordination state, as the menu lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateInfo {
    pub id: &'static str,
    label: &'static str,
    hint: &'static str,
    /// green, accent, red, grey, blue, amber
    pub tone: &'static str,
    /// The Lucide icon (`state-icon.tsx`).
    pub icon: &'static str,
}

impl StateInfo {
    pub fn label(&self, cx: &App) -> SharedString {
        t(cx, self.label)
    }

    pub fn hint(&self, cx: &App) -> SharedString {
        t(cx, self.hint)
    }
}

/// The six states, in the menu's order.
pub const STATES: [StateInfo; 6] = [
    StateInfo { id: "available", label: "profile.stateAvailable", hint: "profile.stateAvailableHint", tone: "green", icon: "circle" },
    StateInfo { id: "focused", label: "profile.stateFocused", hint: "profile.stateFocusedHint", tone: "accent", icon: "focus" },
    StateInfo { id: "swamped", label: "profile.stateSwamped", hint: "profile.stateSwampedHint", tone: "red", icon: "flame" },
    StateInfo { id: "away", label: "profile.stateAway", hint: "profile.stateAwayHint", tone: "grey", icon: "moon" },
    StateInfo { id: "reviewing", label: "profile.stateReviewing", hint: "profile.stateReviewingHint", tone: "blue", icon: "scan-search" },
    StateInfo { id: "done", label: "profile.stateDoneForDay", hint: "profile.stateDoneForDayHint", tone: "amber", icon: "flag" },
];

pub fn state_info(id: &str) -> &'static StateInfo {
    STATES.iter().find(|s| s.id == id).unwrap_or(&STATES[0])
}

/// The state in force at `now` (Available when none or ended).
pub fn state_of(view: Option<&ProfileView>, now: u64) -> &str {
    let Some(v) = view else { return "available" };
    if v.state_until_ms.is_some_and(|u| now >= u) {
        return "available";
    }
    state_info(&v.state).id
}

/// The state tones' colour token (`[data-tone]` → `--hb-tone`).
pub fn tone_token(tone: &str) -> &'static str {
    match tone {
        "green" => "green",
        "accent" => "accent",
        "red" => "red",
        "blue" => "focus",
        "amber" => "amber-edge",
        _ => "ink-3",
    }
}

pub fn one_shots_allowed(state: &str, quiet: bool) -> bool {
    !quiet && !matches!(state, "swamped" | "away" | "done")
}

/// Minutes east of UTC at `ms`.
pub fn utc_offset_min(ms: u64) -> i64 {
    let secs = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes only into `tm`.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm.tm_gmtoff as i64 / 60
}

/// A local wall-clock moment as epoch ms; days past the month's end roll over.
pub fn local_ms(year: i32, month: u32, day: i64, hour: u32, minute: u32) -> u64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month as i32 - 1;
    tm.tm_mday = day as i32;
    tm.tm_hour = hour as i32;
    tm.tm_min = minute as i32;
    tm.tm_isdst = -1;
    // SAFETY: mktime reads and normalises `tm` only.
    let secs = unsafe { libc::mktime(&mut tm) };
    (secs.max(0) as u64) * 1000
}

/// Midnight-relative moment `days` after `now`'s local day, at `hour:minute`.
fn day_at(now: u64, days: i64, hour: u32, minute: u32) -> u64 {
    let (y, m, d, _) = local_date(now);
    local_ms(y, m, d as i64 + days, hour, minute)
}

/// Done for the day ends at the next start of the working hours; without
/// hours, tomorrow at 9:00.
pub fn done_until(hours: Option<&ProfileHours>, now: u64) -> u64 {
    let fallback = day_at(now, 1, 9, 0);
    let Some(hours) = hours.filter(|h| !h.days.is_empty()) else {
        return fallback;
    };
    let mut parts = hours.start.split(':').map(|p| p.parse::<u32>().ok());
    let h = parts.next().flatten().unwrap_or(9);
    let m = parts.next().flatten().unwrap_or(0);
    for d in 0..=7 {
        let at = day_at(now, d, h, m);
        let (_, _, _, wday) = local_date(at);
        let iso = (wday + 6) % 7 + 1;
        if at > now && hours.days.contains(&(iso as u8)) {
            return at;
        }
    }
    fallback
}

/// When "Do not disturb" chosen now ends: 30m, 1h, or tomorrow at 9:00.
pub fn dnd_until(choice: &str, now: u64) -> u64 {
    match choice {
        "30m" => now + 30 * MIN,
        "1h" => now + 60 * MIN,
        _ => day_at(now, 1, 9, 0),
    }
}

/// A custom end typed as `YYYY-MM-DD HH:MM` (or `…THH:MM`), when it is ahead
/// and within two weeks.
pub fn custom_until(value: &str, now: u64) -> Option<u64> {
    let v = value.trim().replace('T', " ");
    let (date, time) = v.split_once(' ')?;
    let mut d = date.split('-').map(|p| p.trim().parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let mut tm = time.split(':').map(|p| p.trim().parse::<u32>().ok());
    let (h, mi) = (tm.next()??, tm.next()??);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 {
        return None;
    }
    let at = local_ms(y as i32, mo as u32, da, h, mi);
    (at > now && at - now <= DND_MAX_MS).then_some(at)
}

fn hm(ms: u64) -> String {
    let (h, m) = local_hm(ms);
    format!("{h}:{m:02}")
}

fn weekday_key(wday: u32) -> &'static str {
    match wday {
        0 => "profile.sunday",
        1 => "profile.monday",
        2 => "profile.tuesday",
        3 => "profile.wednesday",
        4 => "profile.thursday",
        5 => "profile.friday",
        _ => "profile.saturday",
    }
}

/// today, tomorrow or dated.
fn day_of(until: u64, now: u64) -> &'static str {
    let (y, m, d, _) = local_date(until);
    let (ny, nm, nd, _) = local_date(now);
    if (y, m, d) == (ny, nm, nd) {
        return "today";
    }
    let tomorrow = local_date(day_at(now, 1, 12, 0));
    if (y, m, d) == (tomorrow.0, tomorrow.1, tomorrow.2) { "tomorrow" } else { "dated" }
}

/// "until 18:30", "until tomorrow at 9:00", "until Thursday 12 at 9:00".
pub fn quiet_label(until: u64, now: u64, cx: &App) -> SharedString {
    let time = hm(until);
    match day_of(until, now) {
        "today" => tv(cx, "profile.untilTime", &[("time", &time)]),
        "tomorrow" => tv(cx, "profile.untilTomorrow", &[("time", &time)]),
        _ => {
            let (_, _, day, wday) = local_date(until);
            let weekday = t(cx, weekday_key(wday));
            tv(cx, "profile.untilDated", &[("weekday", &weekday), ("day", &day.to_string()), ("time", &time)])
        }
    }
}

/// "Clears at 17:30", or `None` when it stays.
pub fn status_expiry(clear: Option<u64>, now: u64, cx: &App) -> Option<SharedString> {
    let until = clear?;
    let time = hm(until);
    Some(match day_of(until, now) {
        "today" => tv(cx, "profile.clearsAt", &[("time", &time)]),
        "tomorrow" => tv(cx, "profile.clearsTomorrow", &[("time", &time)]),
        _ => {
            let (_, _, day, wday) = local_date(until);
            let weekday = t(cx, weekday_key(wday));
            tv(cx, "profile.clearsDated", &[("weekday", &weekday), ("day", &day.to_string()), ("time", &time)])
        }
    })
}

/// serve's refusals in the operator's language; the rest as serve said it.
pub fn refusal_text(e: &ApiError, cx: &App) -> SharedString {
    match e.code.as_deref() {
        Some("too_large") => t(cx, "profile.photoTooLarge"),
        Some("unsupported_type") => t(cx, "profile.photoWrongType"),
        Some("symlink") => t(cx, "profile.profileFolderSymlink"),
        Some("too_long") => t(cx, "profile.tooLong"),
        _ if e.status.is_none() => t(cx, "profile.couldNotSave"),
        _ => e.message.clone().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: &str, until: Option<u64>) -> ProfileView {
        ProfileView {
            state: state.into(),
            state_until_ms: until,
            ..Default::default()
        }
    }

    #[test]
    fn a_state_that_ended_is_available() {
        assert_eq!(state_of(None, 10), "available");
        assert_eq!(state_of(Some(&view("away", None)), 10), "away");
        assert_eq!(state_of(Some(&view("away", Some(5))), 10), "available");
        assert_eq!(state_of(Some(&view("nonsense", None)), 10), "available");
    }

    #[test]
    fn one_shots_stay_off_when_quiet() {
        assert!(one_shots_allowed("available", false));
        assert!(one_shots_allowed("focused", false));
        assert!(!one_shots_allowed("available", true));
        assert!(!one_shots_allowed("swamped", false));
        assert!(!one_shots_allowed("away", false));
        assert!(!one_shots_allowed("done", false));
    }

    #[test]
    fn dnd_tomorrow_is_nine_the_next_day() {
        let now = local_ms(2026, 11, 10, 22, 30);
        assert_eq!(dnd_until("30m", now), now + 30 * MIN);
        assert_eq!(dnd_until("1h", now), now + 60 * MIN);
        assert_eq!(dnd_until("tomorrow", now), local_ms(2026, 11, 11, 9, 0));
        // Month end rolls over.
        let eom = local_ms(2026, 11, 30, 20, 0);
        assert_eq!(dnd_until("tomorrow", eom), local_ms(2026, 12, 1, 9, 0));
    }

    #[test]
    fn done_until_follows_the_working_hours() {
        // Tuesday 10 Nov 2026, 19:00.
        let now = local_ms(2026, 11, 10, 19, 0);
        let weekdays = ProfileHours {
            start: "08:30".into(),
            end: "17:00".into(),
            days: vec![1, 2, 3, 4, 5],
        };
        assert_eq!(done_until(Some(&weekdays), now), local_ms(2026, 11, 11, 8, 30));
        // Friday evening goes to Monday.
        let friday = local_ms(2026, 11, 13, 19, 0);
        assert_eq!(done_until(Some(&weekdays), friday), local_ms(2026, 11, 16, 8, 30));
        // Before today's start: today.
        let early = local_ms(2026, 11, 10, 7, 0);
        assert_eq!(done_until(Some(&weekdays), early), local_ms(2026, 11, 10, 8, 30));
        // Without hours: tomorrow at 9.
        assert_eq!(done_until(None, now), local_ms(2026, 11, 11, 9, 0));
    }

    #[test]
    fn a_custom_moment_must_be_ahead_and_near() {
        let now = local_ms(2026, 11, 10, 12, 0);
        assert_eq!(custom_until("2026-11-10T18:30", now), Some(local_ms(2026, 11, 10, 18, 30)));
        assert_eq!(custom_until("2026-11-10 18:30", now), Some(local_ms(2026, 11, 10, 18, 30)));
        assert_eq!(custom_until("2026-11-10T11:00", now), None);
        assert_eq!(custom_until("2026-12-30T11:00", now), None);
        assert_eq!(custom_until("tomorrow", now), None);
        assert_eq!(custom_until("", now), None);
    }
}
