//! The strip's pure parts, "The factory" (`components/strip/model.ts`): who
//! stands on the shelf and in what order, what "Needs you" shows, what
//! "Today" counts, the clock's words and when the strip dims. The whole
//! hub's view is the only input; nothing here invents a state serve did not
//! send.

use crate::derive::agent_label;
use crate::i18n::{t, tv};
use crate::model::{Agent, Card, Message, WorkspaceView};
use crate::ui::format::{duration, hhmm, local_date, local_day, local_hm};
use gpui::{App, SharedString};

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;

/// An agent stands on the shelf when it did something this recently.
const SHELF_ACTIVE_MS: u64 = 2 * HOUR;
/// Robots on the shelf; more show as "+N" at its end.
pub const SHELF_MAX: usize = 8;
/// Cards in "Needs you"; more show as "+N more".
pub const NEEDS_MAX: usize = 3;
/// "Today" closes by itself after this.
pub const TODAY_OPEN_MS: u64 = 20_000;

/// The stage the strip is drawn on: a 2560×720 grid, 32:9.
pub const STAGE_W: f32 = 2560.;
pub const STAGE_H: f32 = 720.;

/// The shelf's order: working first, then waiting, sleeping and finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShelfGroup {
    Working,
    Waiting,
    Sleeping,
    Finished,
}

pub fn shelf_group(agent: &Agent) -> ShelfGroup {
    let state = agent.activity.state.as_str();
    if agent.dormant.is_some() || state == "asleep" {
        return ShelfGroup::Sleeping;
    }
    if agent.presence == "finished" || state == "finished" || state == "verified" {
        return ShelfGroup::Finished;
    }
    if agent.presence == "waiting" || state == "waiting" || state == "blocked" {
        return ShelfGroup::Waiting;
    }
    ShelfGroup::Working
}

pub struct Shelf<'a> {
    /// At most SHELF_MAX, in shelf order.
    pub shown: Vec<&'a Agent>,
    /// Active agents that did not fit.
    pub more: usize,
    /// Nobody active: `shown` are the latest agents, asleep.
    pub empty: bool,
}

/// Who stands on the shelf: the agents active in the last two hours, working
/// first, then waiting, sleeping and finished; inside a group the newest
/// first, then by key so robots never swap places on a tie. With nobody
/// active, the most recent agents sleep there instead.
pub fn shelf(view: &WorkspaceView, now: u64) -> Shelf<'_> {
    let at = |a: &Agent| a.last_action_ms.unwrap_or(0);
    let newest = |a: &&Agent, b: &&Agent| at(b).cmp(&at(a)).then_with(|| a.key.cmp(&b.key));
    let mut active: Vec<&Agent> = view
        .agents
        .iter()
        .filter(|a| !a.lost && at(a) > 0 && now.saturating_sub(at(a)) <= SHELF_ACTIVE_MS)
        .collect();
    if active.is_empty() {
        let mut recent: Vec<&Agent> = view.agents.iter().filter(|a| at(a) > 0).collect();
        recent.sort_by(newest);
        recent.truncate(SHELF_MAX);
        return Shelf {
            shown: recent,
            more: 0,
            empty: true,
        };
    }
    active.sort_by(|a, b| {
        shelf_group(a)
            .cmp(&shelf_group(b))
            .then_with(|| newest(a, b))
    });
    let more = active.len().saturating_sub(SHELF_MAX);
    active.truncate(SHELF_MAX);
    Shelf {
        shown: active,
        more,
        empty: false,
    }
}

/// The shelf's name sizes: bold names, down to what a long name needs.
pub const NAME_MAX: f32 = 38.;
pub const NAME_MIN: f32 = 28.;

/// A name's size in its slot from its length: Lato Black averages a little
/// over half an em per character.
pub fn name_size(name: &str, slot: f32) -> f32 {
    let chars = name.chars().count().max(1) as f32;
    ((slot - 12.) / (chars * 0.56))
        .floor()
        .clamp(NAME_MIN, NAME_MAX)
}

/// The shelf's geometry: inner width 1444, a slot at most 206 wide, a robot
/// at most 250, and room for "+N" (104) when some did not fit.
pub fn shelf_geometry(shown: usize, more: bool) -> (f32, f32) {
    const SHELF_INNER: f32 = 1444.;
    const SLOT_MAX: f32 = 206.;
    const ROBOT_MAX: f32 = 250.;
    const SHOULDERS: f32 = 0.84;
    const MORE_W: f32 = 104.;
    let room = SHELF_INNER - if more { MORE_W } else { 0. };
    let slot = SLOT_MAX.min((room / shown.max(1) as f32).floor());
    let size = ROBOT_MAX.min((slot / SHOULDERS).round());
    (slot, size)
}

// ---------------------------------------------------------------------------
// Needs you
// ---------------------------------------------------------------------------

/// Activity's open cards, in the view's order.
pub fn open_cards(view: &WorkspaceView) -> Vec<&Card> {
    view.cards
        .iter()
        .filter(|c| c.needs && c.resolved_ms.is_none())
        .collect()
}

/// "Needs you": at most three, and how many more.
pub fn needs(view: &WorkspaceView) -> (Vec<&Card>, usize) {
    let mut open = open_cards(view);
    let more = open.len().saturating_sub(NEEDS_MAX);
    open.truncate(NEEDS_MAX);
    (open, more)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Warn,
    Quiet,
    Alert,
}

/// The pill beside the name: why the card is there.
pub fn card_pill(card: &Card, cx: &App) -> (SharedString, Tone) {
    match card.kind.as_str() {
        "blocked" => (t(cx, "strip.pillBlocked"), Tone::Warn),
        "approval" => (t(cx, "strip.pillApproval"), Tone::Warn),
        "dormant" => (
            tv(cx, "strip.pillIdleSince", &[("time", &hhmm(card.since_ms))]),
            Tone::Quiet,
        ),
        _ => (t(cx, "strip.pillAlert"), Tone::Alert),
    }
}

/// "BLOCKED · keychain-review · Can it…?" → "Can it…?": the sentence,
/// without the protocol's label and name.
pub fn sentence(text: &str) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let Some(unlabelled) = strip_label(first) else {
        return first.trim().to_string();
    };
    // The project or agent name the protocol puts second: one word, then "·".
    let rest = unlabelled.trim_start();
    if let Some(word_end) = rest.find(|c: char| c.is_whitespace() || c == '·') {
        let after = rest[word_end..].trim_start();
        if let Some(tail) = after.strip_prefix('·') {
            let tail = tail.trim();
            if !tail.is_empty() {
                return tail.to_string();
            }
        }
    }
    unlabelled.trim().to_string()
}

/// `^\s*(START|BLOCKED|FINDING|DONE)(\s+v\d+)?\s*[·:-]\s*` removed, or None.
fn strip_label(line: &str) -> Option<&str> {
    let s = line.trim_start();
    let label = ["START", "BLOCKED", "FINDING", "DONE"]
        .into_iter()
        .find(|l| s.starts_with(l))?;
    let mut rest = &s[label.len()..];
    // An optional " v3".
    let trimmed = rest.trim_start();
    if trimmed.len() < rest.len()
        && let Some(v) = trimmed.strip_prefix('v')
    {
        let digits = v.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            rest = &v[digits..];
        }
    }
    let rest = rest.trim_start();
    let sep = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '·' | ':' | '-'))?;
    Some(rest[sep.len_utf8()..].trim_start())
}

fn message<'a>(view: &'a WorkspaceView, id: &str) -> Option<&'a Message> {
    view.messages.iter().find(|m| m.id == id)
}

fn list(names: &[SharedString], cx: &App) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [head @ .., last] => {
            let head = head
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<&str>>()
                .join(", ");
            tv(cx, "workspace.listJoin", &[("head", &head), ("last", last)]).to_string()
        }
    }
}

fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Title and sentence of a card when it has no message to quote
/// (`cardBody` in `lib/workspace.ts`).
pub fn card_body(card: &Card, view: &WorkspaceView, now: u64, cx: &App) -> (String, String) {
    let names: Vec<SharedString> = card
        .agents
        .iter()
        .map(|k| agent_label(view, k, cx))
        .collect();
    let first = || names.first().map(|s| s.to_string());
    let an_agent = || t(cx, "workspace.anAgent").to_string();
    let quoted = |m: Option<&Message>, key_none: &str| match m {
        Some(m) => tv(
            cx,
            "workspace.quotedAt",
            &[("text", &m.text), ("time", &hhmm(card.since_ms))],
        )
        .to_string(),
        None => tv(cx, key_none, &[("time", &hhmm(card.since_ms))]).to_string(),
    };
    let msg = card.message.as_deref().and_then(|id| message(view, id));
    match card.kind.as_str() {
        "two_writers" => {
            let n = names.len();
            let title = if n == 2 {
                t(cx, "workspace.twoWritersTitleTwo").to_string()
            } else {
                tv(
                    cx,
                    "workspace.twoWritersTitleMany",
                    &[("n", &n.to_string())],
                )
                .to_string()
            };
            let text = tv(
                cx,
                "workspace.twoWritersText",
                &[
                    ("names", &list(&names, cx)),
                    (
                        "checkout",
                        file_name(card.checkout.as_deref().unwrap_or("")),
                    ),
                ],
            )
            .to_string();
            let text = if card.needs {
                text
            } else {
                format!("{text} {}", t(cx, "workspace.twoWritersHistory"))
            };
            (title, text)
        }
        "mention" => (
            tv(
                cx,
                "workspace.mentionTitle",
                &[("who", &first().unwrap_or_else(an_agent))],
            )
            .to_string(),
            quoted(msg, "workspace.atTime"),
        ),
        "handoff" => (
            tv(
                cx,
                "workspace.handoffTitle",
                &[("who", &first().unwrap_or_else(an_agent))],
            )
            .to_string(),
            quoted(msg, "workspace.atTime"),
        ),
        "unanswered" => {
            let title = match (&card.channel, first()) {
                (Some(ch), None) => {
                    tv(cx, "workspace.unansweredInChannel", &[("channel", ch)]).to_string()
                }
                (_, Some(agent)) => {
                    tv(cx, "workspace.unansweredToAgent", &[("agent", &agent)]).to_string()
                }
                (None, None) => t(cx, "workspace.unanswered").to_string(),
            };
            let nudged = msg
                .and_then(|m| m.delivery.as_ref())
                .and_then(|d| d.nudged_ms)
                .map(|ms| format!(" {}", tv(cx, "workspace.resentAt", &[("time", &hhmm(ms))])))
                .unwrap_or_default();
            (
                title,
                format!("{}{nudged}", quoted(msg, "workspace.writtenAt")),
            )
        }
        "dormant" => (
            t(cx, "workspace.dormantTitle").to_string(),
            tv(
                cx,
                "workspace.dormantText",
                &[
                    ("name", &first().unwrap_or_else(an_agent)),
                    ("time", &hhmm(card.since_ms)),
                    ("duration", &duration(now.saturating_sub(card.since_ms))),
                ],
            )
            .to_string(),
        ),
        "blocked" => {
            let gone = card
                .agents
                .first()
                .and_then(|k| view.agents.iter().find(|a| &a.key == k))
                .is_some_and(|a| a.lost || a.delivery == "not_on_bus" || a.delivery == "ended");
            let text = if gone {
                format!("{}.", t(cx, "workspace.sessionGone"))
            } else {
                String::new()
            };
            (first().unwrap_or_default(), text)
        }
        _ => (first().unwrap_or_default(), String::new()),
    }
}

pub struct CardView {
    pub who: String,
    pub pill: SharedString,
    pub tone: Tone,
    pub text: String,
    /// "Reply" when there is an agent to answer, "Open" otherwise.
    pub reply: bool,
    pub hidden: bool,
}

/// A card as the strip draws it: who, why, what it says, and its button.
pub fn card_view(card: &Card, view: &WorkspaceView, now: u64, cx: &App) -> CardView {
    let msg = card.message.as_deref().and_then(|id| message(view, id));
    let agent = card.agents.first();
    let (title, body) = card_body(card, view, now, cx);
    let who = match (card.kind.as_str(), agent) {
        ("two_writers", _) | (_, None) => title,
        (_, Some(k)) => agent_label(view, k, cx).to_string(),
    };
    let text = msg.map(|m| sentence(&m.text)).unwrap_or(body);
    let (pill, tone) = card_pill(card, cx);
    let hidden = card
        .agents
        .iter()
        .any(|k| view.agents.iter().any(|a| &a.key == k && a.hidden));
    CardView {
        who,
        pill,
        tone,
        text,
        reply: card.kind != "two_writers" && agent.is_some(),
        hidden,
    }
}

// ---------------------------------------------------------------------------
// Today
// ---------------------------------------------------------------------------

pub struct Today<'a> {
    /// Today's deliveries, newest first.
    pub items: Vec<&'a Message>,
    /// Only those with harbard's evidence (a landed receipt).
    pub verified: usize,
}

/// The agents' DONE messages of today on the operator's clock, each once,
/// and how many harbard verified on a landed receipt.
pub fn today(view: &WorkspaceView, now: u64) -> Today<'_> {
    let day = local_day(now);
    let mut items: Vec<&Message> = view
        .messages
        .iter()
        .filter(|m| {
            m.from == "agent"
                && m.label.as_deref() == Some("DONE")
                && m.repeat_of.is_none()
                && m.structured.is_none()
                && local_day(m.ts_ms) == day
        })
        .collect();
    items.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms).then_with(|| a.id.cmp(&b.id)));
    let verified = items.iter().filter(|m| m.verified.is_some()).count();
    Today { items, verified }
}

/// "Today: 6 deliveries · 4 verified".
pub fn today_line(deliveries: usize, verified: usize, cx: &App) -> SharedString {
    if deliveries == 0 {
        return t(cx, "strip.todayNone");
    }
    let n = deliveries.to_string();
    let v = verified.to_string();
    let e = tv(
        cx,
        if deliveries == 1 {
            "strip.deliveriesOne"
        } else {
            "strip.deliveriesMany"
        },
        &[("count", &n)],
    );
    let ver = tv(
        cx,
        if verified == 1 {
            "strip.verifiedOne"
        } else {
            "strip.verifiedMany"
        },
        &[("count", &v)],
    );
    tv(
        cx,
        "strip.todayLine",
        &[("deliveries", &e), ("verified", &ver)],
    )
}

// ---------------------------------------------------------------------------
// Clock, night, fit
// ---------------------------------------------------------------------------

const WEEKDAYS_EN: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAYS_ES: [&str; 7] = [
    "domingo",
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
];
const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_ES: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
];

/// "14:52" and "Saturday 7 Nov" ("sábado 7 nov"), on the operator's clock.
pub fn clock(ms: u64, spanish: bool) -> (String, String) {
    let (_, month, day, weekday) = local_date(ms);
    let (days, months) = if spanish {
        (WEEKDAYS_ES, MONTHS_ES)
    } else {
        (WEEKDAYS_EN, MONTHS_EN)
    };
    let date = format!(
        "{} {day} {}",
        days[weekday as usize % 7],
        months[(month as usize + 11) % 12]
    );
    (hhmm(ms), date)
}

/// The strip dims from 22:00 to 7:00 when the OS is dark, unless turned off
/// in Preferences.
pub fn dimmed(ms: u64, os_dark: bool, enabled: bool) -> bool {
    if !enabled || !os_dark {
        return false;
    }
    let (h, _) = local_hm(ms);
    !(7..22).contains(&h)
}

/// How the 2560×720 stage fits a window: one uniform scale, the largest
/// that shows the whole strip, and the margins left on each side (which
/// show the frame colour). A degenerate window keeps the stage at 1.
pub fn strip_fit(width: f32, height: f32) -> (f32, f32, f32) {
    let raw = (width / STAGE_W).min(height / STAGE_H);
    let k = if raw.is_finite() && raw > 0. { raw } else { 1. };
    let left = ((width - STAGE_W * k) / 2.).max(0.);
    let top = ((height - STAGE_H * k) / 2.).max(0.);
    (k, left, top)
}

/// The fallback window's size: 32:9 like a wide display, as wide as 90 % of
/// the work area allows (in width and in height), at most 1600 wide, a whole
/// multiple of 32 so the height is whole too.
pub fn fallback_size(area_w: f32, area_h: f32) -> (f32, f32) {
    const FALLBACK_MAX_WIDTH: f32 = 1600.;
    let fit = (area_w * 0.9)
        .min(area_h * 0.9 * 32. / 9.)
        .min(FALLBACK_MAX_WIDTH);
    let w = ((fit / 32.).floor() * 32.).max(32.);
    (w, w * 9. / 32.)
}

/// The smallest fallback window, still 32:9.
pub const FALLBACK_MIN_W: f32 = 640.;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(agents: serde_json::Value, messages: serde_json::Value, now: u64) -> WorkspaceView {
        serde_json::from_value(json!({
            "v": 1, "now_ms": now, "operator": "o", "operator_initials": "O",
            "thresholds": { "amber_ms": 1, "red_ms": 2, "thinking_long_ms": 3, "waiting_sleepy_ms": 4, "running_long_ms": 5 },
            "channels": [], "agents": agents, "messages": messages, "cards": [], "deliveries": [], "saved": [],
            "unread": { "directs": 0, "activity": 0, "channels": 0, "todo": 0 },
        }))
        .unwrap()
    }

    fn agent(key: &str, presence: &str, state: &str, last: Option<u64>) -> serde_json::Value {
        json!({ "key": key, "name": key, "label": key, "presence": presence, "last_action_ms": last,
                "activity": { "state": state, "since_ms": 0, "source_ms": 0 } })
    }

    #[test]
    fn the_shelf_orders_working_waiting_sleeping_finished_then_newest() {
        let now = 10 * HOUR;
        let v = view(
            json!([
                agent("fin", "finished", "finished", Some(now - 10)),
                agent("wait", "waiting", "waiting", Some(now - 20)),
                agent("work-old", "working", "running", Some(now - 30 * MIN)),
                agent("work-new", "working", "thinking", Some(now - MIN)),
                agent("gone", "working", "running", Some(now - 3 * HOUR)),
            ]),
            json!([]),
            now,
        );
        let s = shelf(&v, now);
        let keys: Vec<&str> = s.shown.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["work-new", "work-old", "wait", "fin"]);
        assert!(!s.empty);
        assert_eq!(s.more, 0);
    }

    #[test]
    fn with_nobody_active_the_latest_sleep_on_the_shelf() {
        let now = 10 * HOUR;
        let v = view(
            json!([
                agent("a", "working", "normal", Some(now - 5 * HOUR)),
                agent("b", "working", "normal", None)
            ]),
            json!([]),
            now,
        );
        let s = shelf(&v, now);
        assert!(s.empty);
        assert_eq!(s.shown.len(), 1);
    }

    #[test]
    fn more_than_eight_show_as_more() {
        let now = 10 * HOUR;
        let agents: Vec<_> = (0..11)
            .map(|i| {
                agent(
                    &format!("a{i:02}"),
                    "working",
                    "running",
                    Some(now - i * 1000),
                )
            })
            .collect();
        let v = view(json!(agents), json!([]), now);
        let s = shelf(&v, now);
        assert_eq!(s.shown.len(), SHELF_MAX);
        assert_eq!(s.more, 3);
    }

    #[test]
    fn the_sentence_drops_the_protocols_label_and_name() {
        assert_eq!(
            sentence("BLOCKED · keychain-review · Can it ship?"),
            "Can it ship?"
        );
        assert_eq!(
            sentence("DONE v2 · harbard · Landed the fix\nmore"),
            "Landed the fix"
        );
        assert_eq!(sentence("DONE: shipped it"), "shipped it");
        assert_eq!(sentence("\n  plain words  "), "plain words");
        assert_eq!(sentence("STARTLING news"), "STARTLING news");
    }

    #[test]
    fn today_counts_only_todays_agent_deliveries_once() {
        let now = 1_793_000_000_000u64;
        let msg = |id: &str, from: &str, label: &str, ts: u64, verified: bool, repeat: bool| {
            json!({ "id": id, "agent": "a", "from": from, "author": "a", "ts_ms": ts, "text": "DONE · x · did it",
                    "label": label, "verified": if verified { json!({ "sha": "s", "branch": "b", "decided_ms": ts }) } else { json!(null) },
                    "repeat_of": if repeat { json!("m0") } else { json!(null) } })
        };
        let v = view(
            json!([]),
            json!([
                msg("m1", "agent", "DONE", now - 1000, true, false),
                msg("m2", "agent", "DONE", now - 2000, false, false),
                msg("m3", "agent", "DONE", now - 3000, false, true),
                msg("m4", "operator", "DONE", now - 1000, false, false),
                msg("m5", "agent", "START", now - 1000, false, false),
                msg("m6", "agent", "DONE", now - 2 * 24 * HOUR, false, false),
            ]),
            now,
        );
        let d = today(&v, now);
        assert_eq!(
            d.items.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["m1", "m2"]
        );
        assert_eq!(d.verified, 1);
    }

    #[test]
    fn the_stage_fits_whole_and_letterboxes() {
        assert_eq!(strip_fit(2560., 720.), (1., 0., 0.));
        let (k, left, top) = strip_fit(1280., 720.);
        assert_eq!(k, 0.5);
        assert_eq!(left, 0.);
        assert_eq!(top, 180.);
        assert_eq!(strip_fit(0., 0.).0, 1.);
    }

    #[test]
    fn the_fallback_window_is_32_by_9_and_fits_the_work_area() {
        assert_eq!(fallback_size(1440., 900.), (1280., 360.));
        assert_eq!(fallback_size(1024., 560.), (896., 252.));
        assert_eq!(fallback_size(1920., 1080.), (1600., 450.));
        assert_eq!(fallback_size(3440., 300.), (960., 270.));
    }

    #[test]
    fn names_shrink_to_fit_their_slot_within_bounds() {
        assert_eq!(name_size("Ann", 206.), NAME_MAX);
        assert_eq!(name_size("Bartholomew-the-long", 120.), NAME_MIN);
        let (slot, size) = shelf_geometry(8, true);
        assert_eq!(slot, 167.);
        assert_eq!(size, 199.);
        assert_eq!(shelf_geometry(2, false), (206., 245.));
    }

    #[test]
    fn the_night_dims_only_when_dark_and_allowed() {
        // Noon and midnight in local time.
        let base = 1_793_000_000_000u64;
        let (h, m) = local_hm(base);
        let midnight = base - (h as u64 * HOUR + m as u64 * MIN);
        let noon = midnight + 12 * HOUR;
        assert!(dimmed(midnight + 30 * MIN, true, true));
        assert!(!dimmed(noon, true, true));
        assert!(!dimmed(midnight + 30 * MIN, false, true));
        assert!(!dimmed(midnight + 30 * MIN, true, false));
    }
}
