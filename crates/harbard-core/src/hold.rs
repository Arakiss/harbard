//! What the operator's coordination state holds back, and the digest that
//! delivers it (`{harbard home}/digests.json`).
//!
//! A note to the human (`--to harness:human`, the address agents use for the
//! person at the console) is held while the state asks for it:
//!
//! - Focused and Swamped hold the non-blocking kinds, `fyi` and `ask`; a
//!   `warning` or a `handoff` still reaches them.
//! - Away and Done for the day queue every note to them.
//! - Available and Reviewing hold nothing; switching to Reviewing flushes
//!   what the previous state held.
//!
//! Nothing is deleted or rewritten on the bus: the notes stay in the log
//! where they were appended. What changes is delivery to the human. While
//! held, the desktop's doorbell stays quiet for them (`profile::dock_rules`)
//! and the author's receipt says so ([`held_receipt`]). Leaving the state,
//! its end coming ([`settle_lapse`]), or every hour when they asked for it,
//! cuts one [`Digest`] of exactly the held notes since the last cut, and
//! that is what the console shows. A cut goes by position in the log, so a
//! note appended late with an earlier time still reaches the next one.

use crate::event::{Event, EventKind, RecipientKind};
use crate::profile::Profile;
use crate::query::event_ms;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Digests kept; the oldest go first.
pub const DIGESTS_MAX: usize = 50;
/// First line of a note in a digest, in characters.
pub const ITEM_TEXT_CHARS: usize = 200;
/// "Every 60 min if they ask for it."
pub const HOURLY_MS: u64 = 3_600_000;

/// How much a state holds back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    Nothing,
    /// `fyi` and `ask`.
    NonBlocking,
    /// Every note to the human.
    Everything,
}

pub fn hold_for(state: &str) -> Hold {
    match state {
        "focused" | "swamped" => Hold::NonBlocking,
        "away" | "done" => Hold::Everything,
        _ => Hold::Nothing,
    }
}

/// A note addressed to the person at the console.
pub fn for_human(e: &Event) -> bool {
    e.event == EventKind::Note
        && e.to
            .as_ref()
            .is_some_and(|to| to.kind == RecipientKind::Harness && to.value == crate::HARNESS_HUMAN)
}

/// Whether `state` holds this event back from the human.
pub fn holds(state: &str, e: &Event) -> bool {
    if !for_human(e) {
        return false;
    }
    match hold_for(state) {
        Hold::Nothing => false,
        Hold::NonBlocking => matches!(e.kind.as_str(), "fyi" | "ask"),
        Hold::Everything => true,
    }
}

/// What the author of a held note is told, or `None` when it is not held.
pub fn held_receipt(state: &str, e: &Event) -> Option<&'static str> {
    if !holds(state, e) {
        return None;
    }
    Some(match state {
        "focused" => "held for the operator's digest: Focused",
        "swamped" => "held for the operator's digest: Swamped",
        "away" => "queued for the operator's return: Away",
        _ => "queued for the operator's next working hours: Done for the day",
    })
}

/// One held note, as the digest carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestItem {
    pub id: String,
    pub session: String,
    pub harness: String,
    pub kind: String,
    /// The first line, bounded.
    pub text: String,
    pub ts_ms: u64,
}

/// What a state held, delivered at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    pub id: String,
    /// The state that held it.
    // The adapter exists only to read old files.
    #[serde(deserialize_with = "crate::old_spelling::read_coordination_state")]
    pub state: String,
    /// `left` (they left the state), `reviewing` (flushed by Reviewing) or
    /// `hourly`.
    // The adapter exists only to read old files.
    #[serde(deserialize_with = "crate::old_spelling::read_digest_reason")]
    pub reason: String,
    pub from_ms: u64,
    pub to_ms: u64,
    pub items: Vec<DigestItem>,
    #[serde(default)]
    pub read: bool,
}

/// `{harbard home}/digests.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digests {
    #[serde(default)]
    pub items: Vec<Digest>,
    /// The end of the last cut, so no note is delivered twice.
    #[serde(default)]
    pub cut_ms: u64,
    /// The newest event the last cut read: the next cut takes what was
    /// appended after it, whatever its `ts`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark_id: Option<String>,
}

impl Digests {
    pub fn path(root: &Path) -> PathBuf {
        root.join("digests.json")
    }

    /// Missing or unreadable is empty.
    pub fn load(root: &Path) -> Self {
        fs::read_to_string(Self::path(root))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Change and save under the desk's cross-process lock.
    fn update<T>(root: &Path, f: impl FnOnce(&mut Digests) -> T) -> std::io::Result<T> {
        use fs2::FileExt;
        fs::create_dir_all(root)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(crate::workspace::DeskState::path(root).with_extension("json.lock"))?;
        lock.lock_exclusive()?;
        let result = (|| {
            let mut d = Self::load(root);
            let out = f(&mut d);
            let bytes = serde_json::to_vec_pretty(&d)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let path = Self::path(root);
            let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
            fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, &path))?;
            Ok(out)
        })();
        let _ = FileExt::unlock(&lock);
        result
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() <= ITEM_TEXT_CHARS {
        return line.to_string();
    }
    let mut out: String = line.chars().take(ITEM_TEXT_CHARS).collect();
    out.push('…');
    out
}

/// One note as a digest carries it.
fn item(e: &Event) -> DigestItem {
    DigestItem {
        id: e.id.clone(),
        session: e.session.clone(),
        harness: e.harness.clone(),
        kind: e.kind.as_str().to_string(),
        text: first_line(&e.text),
        ts_ms: event_ms(e),
    }
}

/// The notes `state` held between `from_ms` (inclusive) and `to_ms`, by
/// their own time.
pub fn collect(events: &[Event], state: &str, from_ms: u64, to_ms: u64) -> Vec<DigestItem> {
    events
        .iter()
        .filter(|e| holds(state, e))
        .filter(|e| (from_ms..to_ms).contains(&event_ms(e)))
        .map(item)
        .collect()
}

/// What `state`, in force since `since_ms`, held after the last cut.
///
/// The cut goes by where the log was, not by the notes' own times: a note
/// appended after the last cut belongs to the next one even when its `ts`
/// is earlier (a slow writer, a satellite's clock). The last cut remembers
/// the newest event it read ([`Digests::mark_id`]); everything after it in
/// the log is new. When the mark predates the state, only what is also
/// timed inside the state counts, since the notes between the mark and the
/// state's start reached them normally. Without a mark the log still holds
/// (an old `digests.json`, or a log read in a narrower window), the times
/// decide, as before.
///
/// `end_ms` bounds a state that lapsed on its own: what came after its end
/// arrived while they were Available.
fn held(
    events: &[Event],
    state: &str,
    since_ms: u64,
    end_ms: Option<u64>,
    d: &Digests,
) -> Vec<DigestItem> {
    let mark = d
        .mark_id
        .as_deref()
        .and_then(|id| events.iter().rposition(|e| e.id == id));
    let mark_inside = d.cut_ms >= since_ms;
    let from = since_ms.max(d.cut_ms);
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| holds(state, e))
        .filter(|(_, e)| end_ms.is_none_or(|end| event_ms(e) < end))
        .filter(|(i, e)| match mark {
            Some(m) => *i > m && (mark_inside || event_ms(e) >= since_ms),
            None => event_ms(e) >= from,
        })
        .map(|(_, e)| item(e))
        .collect()
}

/// Remember where the log was at `now_ms`: the next cut starts after it.
fn mark(d: &mut Digests, events: &[Event], now_ms: u64) {
    d.cut_ms = now_ms;
    d.mark_id = events.last().map(|e| e.id.clone());
}

/// Cut a digest of what `state` held since the last cut (or since the state
/// began), store it, and return it. Nothing held: no digest.
///
/// `lapse` is the end of a state that lapsed on its own; the cut is then
/// made at most once, by whoever notices first.
fn cut(
    root: &Path,
    events: &[Event],
    state: &str,
    since_ms: u64,
    reason: &str,
    lapse: Option<u64>,
    now_ms: u64,
) -> std::io::Result<Option<Digest>> {
    Digests::update(root, |d| {
        if lapse.is_some_and(|until| d.cut_ms >= until) {
            return None;
        }
        let from = since_ms.max(d.cut_ms);
        let items = held(events, state, since_ms, lapse, d);
        mark(d, events, now_ms);
        if items.is_empty() {
            return None;
        }
        let digest = Digest {
            id: format!("digest-{now_ms}"),
            state: state.to_string(),
            reason: reason.to_string(),
            from_ms: from,
            to_ms: lapse.unwrap_or(now_ms),
            items,
            read: false,
        };
        d.items.push(digest.clone());
        if d.items.len() > DIGESTS_MAX {
            let drop = d.items.len() - DIGESTS_MAX;
            d.items.drain(..drop);
        }
        Some(digest)
    })
}

fn state_since(p: &Profile) -> u64 {
    p.state.as_ref().map_or(0, |s| s.since_ms)
}

/// A state with an end that has come, which held something, and whose end
/// no cut has covered yet: `(state, since_ms, until_ms)`.
fn lapsed(root: &Path, p: &Profile, now_ms: u64) -> Option<(String, u64, u64)> {
    let s = p.state.as_ref()?;
    let until = s.until_ms.filter(|&until| until <= now_ms)?;
    if hold_for(&s.id) == Hold::Nothing || Digests::load(root).cut_ms >= until {
        return None;
    }
    Some((s.id.clone(), s.since_ms, until))
}

/// Whether a state lapsed on its own and its digest is still to be cut,
/// without reading the log.
pub fn lapse_due(root: &Path, p: &Profile, now_ms: u64) -> bool {
    lapsed(root, p, now_ms).is_some()
}

/// The end of a timed state ("Focused until 18:30", Done for the day
/// until their next working hours) is leaving it: the first look after the
/// end cuts what it held, with reason `left`, exactly as a click on another
/// state would have. No request carries the change, so whoever comes first
/// does it: the profile and digest routes, any desk write, the minute tick.
pub fn settle_lapse(
    root: &Path,
    events: &[Event],
    p: &Profile,
    now_ms: u64,
) -> std::io::Result<Option<Digest>> {
    let Some((state, since, until)) = lapsed(root, p, now_ms) else {
        return Ok(None);
    };
    cut(root, events, &state, since, "left", Some(until), now_ms)
}

/// The operator changed state (`before` → `after`): if the state left held
/// anything, deliver it now as one digest. Switching to Reviewing is the
/// same flush, said as such. Entering a state that holds remembers where
/// the log was, so what it holds starts there.
pub fn on_state_change(
    root: &Path,
    events: &[Event],
    before: &Profile,
    after: &Profile,
    now_ms: u64,
) -> std::io::Result<Option<Digest>> {
    let lapse = settle_lapse(root, events, before, now_ms)?;
    let old = before.state_at(now_ms);
    let new = after.state_at(now_ms);
    if old == new {
        return Ok(lapse);
    }
    if hold_for(old) == Hold::Nothing {
        Digests::update(root, |d| mark(d, events, now_ms))?;
        return Ok(lapse);
    }
    let reason = if new == "reviewing" {
        "reviewing"
    } else {
        "left"
    };
    cut(root, events, old, state_since(before), reason, None, now_ms)
}

/// "Every 60 min if they ask for it": an hour after the last cut, deliver
/// what the current state held so far. Not asked for, or nothing held:
/// nothing. A state that lapsed is cut first, as leaving it.
pub fn hourly(
    root: &Path,
    events: &[Event],
    p: &Profile,
    now_ms: u64,
) -> std::io::Result<Option<Digest>> {
    if let Some(d) = settle_lapse(root, events, p, now_ms)? {
        return Ok(Some(d));
    }
    if !hourly_due(root, p, now_ms) {
        return Ok(None);
    }
    cut(
        root,
        events,
        p.state_at(now_ms),
        state_since(p),
        "hourly",
        None,
        now_ms,
    )
}

/// Whether an hourly digest is due, without reading the log.
pub fn hourly_due(root: &Path, p: &Profile, now_ms: u64) -> bool {
    let state = p.state_at(now_ms);
    if !p.digest_hourly || hold_for(state) == Hold::Nothing {
        return false;
    }
    let last = Digests::load(root).cut_ms.max(state_since(p));
    now_ms.saturating_sub(last) >= HOURLY_MS
}

/// What the current state holds so far, since the last cut, without cutting:
/// "While you were away" while they are still Away.
pub fn pending(root: &Path, events: &[Event], p: &Profile, now_ms: u64) -> Vec<DigestItem> {
    let state = p.state_at(now_ms);
    if hold_for(state) == Hold::Nothing {
        return Vec::new();
    }
    held(events, state, state_since(p), None, &Digests::load(root))
}

/// Mark one digest read; false when there is no such digest.
pub fn mark_read(root: &Path, id: &str) -> std::io::Result<bool> {
    Digests::update(root, |d| {
        let found = d.items.iter_mut().find(|x| x.id == id);
        found.map(|x| x.read = true).is_some()
    })
}

#[cfg(test)]
mod tests;
