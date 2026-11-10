//! Session aliases (F5): a name a person can hold in their head.
//!
//! `019fa3be-ce60-70e1-9cd0-f99326873d69` is not a participant you can talk
//! about. You cannot ask "what is 019fa3be doing" out loud, you cannot tell two
//! of them apart at a glance, and you certainly cannot remember on Tuesday who
//! it was on Monday. A name fixes all three at once.
//!
//! ## Why deterministic, and why still persisted
//!
//! The alias is derived from the session id by hash, so the same id always
//! yields the same name with no state at all — a session that appears on
//! another machine, or after the store is wiped, keeps its name. But two ids
//! can hash to one name, so the assignment is ALSO written down: on collision
//! the newcomer takes the next free name and that choice is remembered. Hash
//! first, ledger second — determinism where it is free, memory where it is not.
//!
//! The ledger is the harness-agnostic half of "each harness remembers": it
//! lives in the store, so Claude, Codex and the console all read one name for
//! one session without anybody syncing anything.
//!
//! ## Why the agent names itself, and harbard only keeps names fresh
//!
//! A curated list of names was the other option and it loses twice: it repeats
//! as soon as the fleet outgrows it, and somebody has to keep extending it.
//! The agent instead picks a proper name inspired by its mission, and harbard
//! refuses what makes a board unreadable — a generic word, a name held by
//! another session, or one another session carried in the last
//! [`FRESHNESS_DAYS`]. A session that never introduces itself gets a
//! realistic English given name drawn from its id (Nora, Owen), fresh while
//! the list allows it, and otherwise the name that has rested longest.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// The epithet's weather: adjectives a crossing earns.
const EPITHET_ADJ: &[&str] = &[
    "quiet",
    "steady",
    "salt-worn",
    "far-seeing",
    "night-eyed",
    "patient",
    "swift",
    "watchful",
    "stubborn",
    "gentle",
    "fierce",
    "lucky",
    "careful",
    "wayward",
    "certain",
    "drifting",
    "anchored",
    "luminous",
    "bold",
    "restless",
    "north-bound",
    "leeward",
    "windward",
    "deep-keeled",
    "moonlit",
    "storm-bred",
    "tide-wise",
    "iron-willed",
    "soft-spoken",
    "keen-eared",
    "foam-born",
    "star-led",
    "fog-loving",
    "ember-bright",
    "glass-calm",
    "wild-hearted",
    "chart-true",
    "dusk-walking",
    "brine-kissed",
    "unhurried",
    "radiant",
    "weathered",
    "unsleeping",
    "sideways",
];

/// The epithet's ground: what a session is, in one word of sea.
const EPITHET_NOUN: &[&str] = &[
    "crossing",
    "watch",
    "helm",
    "chart",
    "current",
    "tide",
    "bearing",
    "anchor",
    "sail",
    "rudder",
    "horizon",
    "beacon",
    "prow",
    "keel",
    "voyage",
    "passage",
    "sound",
    "sea-lane",
    "star-path",
    "meridian",
    "drift",
    "mooring",
    "harbor-light",
    "night-watch",
    "lee-shore",
    "tide-mark",
    "wind-rose",
    "lookout",
    "compass",
    "lantern",
    "signal",
    "reef",
    "channel",
    "spindrift",
    "undertow",
    "fairway",
    "landfall",
    "outward",
    "homebound",
    "pilotage",
    "skyline",
    "afterglow",
    "sea-glass",
];

/// How long a name stays off the board after the last session that carried it.
pub const FRESHNESS_DAYS: u64 = 90;
const FRESHNESS_MS: u64 = FRESHNESS_DAYS * 24 * 60 * 60 * 1000;

/// Words that are not names. Left to themselves, models reach for the tool
/// they feel like (Wrench, Conduit) or the role they were given (Helper,
/// Agent); a board of those tells nobody apart. Small on purpose: freshness
/// does most of the work, this only stops the reflexes.
const GENERIC: &[&str] = &[
    "agent",
    "assistant",
    "helper",
    "bot",
    "robot",
    "worker",
    "runner",
    "executor",
    "tool",
    "wrench",
    "hammer",
    "conduit",
    "catchup",
    "scout",
    "explore",
    "explorer",
    "builder",
    "fixer",
    "coder",
    "reviewer",
    "planner",
    "daemon",
    "server",
    "client",
    "main",
    "subagent",
    "session",
    "test",
    "codex",
    "claude",
    "harbard",
    "user",
    "operator",
];

/// Hash-derived candidates a nameless session tries before it takes the
/// name that has rested longest.
const FALLBACK_CANDIDATES: u64 = 64;

/// The persisted assignments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Aliases {
    /// session id -> alias
    #[serde(default)]
    pub by_session: BTreeMap<String, String>,
    /// Sessions whose own harness or agent chose the alias.
    #[serde(default)]
    pub self_named: BTreeSet<String>,
    /// name -> the last session that carried it, and when. Unlike
    /// `by_session` this outlives [`release_inactive`]: releasing frees a name
    /// for the living, the history keeps it off the board until it is no
    /// longer recent. Entries older than [`FRESHNESS_DAYS`] are pruned.
    #[serde(default)]
    pub history: BTreeMap<String, NameUse>,
    /// The names agents go by, lower-cased ([`read_for_naming`]). Never
    /// written: `names.json` owns them.
    #[serde(skip)]
    pub reserved: BTreeSet<String>,
}

/// One past use of a name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameUse {
    pub session: String,
    pub last_seen_ms: u64,
}

/// FNV-1a over the session id. Stable across machines and across builds —
/// `DefaultHasher` guarantees neither, and a name that changes when the
/// standard library does is not a name.
fn hash(session: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in session.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The splitmix64 finalizer: FNV leaves inputs that differ only in their last
/// byte close together, and the candidates below are exactly that.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The `k`-th English given name derived from a session id.
fn candidate(session: &str, k: u64) -> String {
    let at = mix(hash(&format!("{session}#{k}"))) % crate::workshop::name_count() as u64;
    crate::workshop::nth_name(at as usize)
        .0
        .to_ascii_lowercase()
}

/// The name of the list no session holds and that was last used longest ago.
fn longest_rested(a: &Aliases) -> Option<String> {
    (0..crate::workshop::name_count())
        .map(|i| crate::workshop::nth_name(i).0.to_ascii_lowercase())
        .filter(|name| {
            !is_generic(name)
                && !a.reserved.contains(name)
                && !a.by_session.values().any(|held| held == name)
        })
        .min_by_key(|name| a.history.get(name).map_or(0, |used| used.last_seen_ms))
}

/// Neutral fallback while a session has not introduced itself: the first
/// candidate for its id. Pure, so surfaces that must not assign
/// (and traffic from released sessions) still render a stable name.
pub fn preferred(session: &str) -> String {
    candidate(session, 0)
}

fn roman(mut n: u32) -> String {
    const DIGITS: &[(u32, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for &(value, digits) in DIGITS {
        while n >= value {
            out.push_str(digits);
            n -= value;
        }
    }
    out
}

/// The session's epithet — its one line of soul, derived like the name.
///
/// A name says WHO a session is; the epithet says HOW it sails, and it never
/// changes: "Fulla — the quiet crossing". Pure derivation (two hash draws,
/// no state, no writes), so it costs nothing and lies about nothing — it is
/// flavor, not a fact about the work, and it stays out of the thread where
/// facts live.
pub fn epithet(session: &str) -> String {
    let h = hash(session);
    let adj = EPITHET_ADJ[(h % EPITHET_ADJ.len() as u64) as usize];
    let noun = EPITHET_NOUN[((h / EPITHET_ADJ.len() as u64) % EPITHET_NOUN.len() as u64) as usize];
    format!("the {adj} {noun}")
}

fn path(store: &Store) -> PathBuf {
    store.root().join("aliases.json")
}

/// Read the ledger; missing or corrupt reads as empty — a broken alias file
/// must never break a session, it just means names get
/// re-derived.
pub fn read(store: &Store) -> Aliases {
    fs::read_to_string(path(store))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// The ledger for a command that may hand out a name: it also knows the names
/// agents go by. Sessions answer for their own in `by_session`.
pub fn read_for_naming(store: &Store) -> Aliases {
    let mut a = read(store);
    a.reserved = crate::workshop::Registry::load(store.root())
        .agents
        .iter()
        .filter(|(key, _)| !key.starts_with("session:"))
        .map(|(_, e)| e.name.to_lowercase())
        .collect();
    a
}

/// Persist the ledger by write-then-rename.
pub fn write(store: &Store, a: &Aliases) -> std::io::Result<()> {
    fs::create_dir_all(store.root())?;
    let json = serde_json::to_string_pretty(a)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    // The temp name carries our pid: a fixed `*.json.tmp` is a shared file,
    // so two processes writing at once clobber each other's staging area and
    // one rename lands a half-written or foreign snapshot.
    let tmp = path(store).with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path(store))
}

fn is_recent(used: &NameUse, now_ms: u64) -> bool {
    now_ms.saturating_sub(used.last_seen_ms) < FRESHNESS_MS
}

/// Note that `session` carried `name` at `now_ms`, and drop what has aged out.
fn remember(a: &mut Aliases, name: &str, session: &str, now_ms: u64) {
    a.history.insert(
        name.to_ascii_lowercase(),
        NameUse {
            session: session.to_string(),
            last_seen_ms: now_ms,
        },
    );
    a.history.retain(|_, used| is_recent(used, now_ms));
}

/// A word that names a tool or a role rather than somebody.
pub fn is_generic(name: &str) -> bool {
    GENERIC.iter().any(|word| word.eq_ignore_ascii_case(name))
}

/// Why `session` cannot carry `name` right now, if it cannot. A session may
/// always take back a name it carried itself.
fn unavailable(a: &Aliases, session: &str, name: &str, now_ms: u64) -> Option<String> {
    if a.by_session
        .iter()
        .any(|(owner, alias)| owner != session && alias.eq_ignore_ascii_case(name))
    {
        return Some(format!(
            "\"{}\" is already in use by another session; choose another name",
            display(name)
        ));
    }
    if a.reserved.contains(&name.to_lowercase()) {
        return Some(format!(
            "\"{}\" is already the name of an agent; choose another name",
            display(name)
        ));
    }
    match a.history.get(&name.to_ascii_lowercase()) {
        Some(used) if used.session != session && is_recent(used, now_ms) => Some(format!(
            "\"{}\" was used by another session in the last {FRESHNESS_DAYS} days and names stay fresh; choose another name",
            display(name)
        )),
        _ => None,
    }
}

/// This session's name, assigning one if it has none.
///
/// Returns `(alias, newly_assigned)` so the caller can stay quiet on the
/// common path instead of announcing a name that was already settled.
pub fn resolve(a: &mut Aliases, session: &str, now_ms: u64) -> (String, bool) {
    if let Some(existing) = a.by_session.get(session) {
        // A generated name from before the list stops being carried.
        if a.self_named.contains(session) || crate::workshop::listed(existing).is_some() {
            return (existing.clone(), false);
        }
        a.by_session.remove(session);
    }

    let free = |a: &Aliases, name: &str| unavailable(a, session, name, now_ms).is_none();
    let chosen = (0..FALLBACK_CANDIDATES)
        .map(|k| candidate(session, k))
        .find(|name| !is_generic(name) && free(a, name))
        .or_else(|| longest_rested(a))
        .unwrap_or_else(|| {
            let base = preferred(session);
            (2_u32..)
                .map(|n| format!("{base}-{}", roman(n)))
                .find(|name| free(a, name))
                .expect("the numeral fallback is unbounded")
        });

    a.by_session.insert(session.to_string(), chosen.clone());
    remember(a, &chosen, session, now_ms);
    (chosen, true)
}

/// Pin a harness-provided nickname (e.g. Codex `Hilbert`). A provisional
/// fallback may be replaced once; an established self-name never moves. A
/// nickname the rules refuse (stale, generic, misshapen) leaves the session on
/// its fallback.
pub fn claim(a: &mut Aliases, session: &str, name: &str, now_ms: u64) -> (String, bool) {
    match self_name(a, session, name, now_ms) {
        Ok(result) => result,
        Err(_) => resolve(a, session, now_ms),
    }
}

/// Release every name held by a session that is no longer among `active`.
///
/// The registry names who is HERE, not who has ever been. Without this it only
/// ever grew: 2,124 names taken, 67 of them by a session with activity in the
/// last day, and more than half of all claims already carrying a numeric
/// anticollision suffix — so the first verb every session runs had become a
/// lottery against two thousand ghosts.
///
/// A released name moves to the history, stamped `now_ms`, so it stays off the
/// board for [`FRESHNESS_DAYS`] instead of being handed straight to the next
/// session. A released session keeps a stable identity: [`preferred`] derives
/// a name from the session id alone, so old traffic still renders under a
/// name, just not the one its author chose.
pub fn release_inactive(
    a: &mut Aliases,
    active: &std::collections::HashSet<String>,
    now_ms: u64,
) -> usize {
    let dead: Vec<(String, String)> = a
        .by_session
        .iter()
        .filter(|(session, _)| !active.contains(*session))
        .map(|(session, alias)| (session.clone(), alias.clone()))
        .collect();
    for (session, alias) in &dead {
        a.by_session.remove(session);
        a.self_named.remove(session);
        remember(a, alias, session, now_ms);
    }
    a.history.retain(|_, used| is_recent(used, now_ms));
    dead.len()
}

/// Let one session introduce itself exactly once.
///
/// Callsigns are ENGLISH-readable, whatever language the work or the operator
/// speaks. This register is one shared namespace across every harness and
/// every project on the machine, and it is read by people scanning a board:
/// mixing scripts in it makes collisions, sorting and recall worse for
/// everyone and buys nothing, because a callsign is an address, not prose.
///
/// The agent chooses; harbard only keeps the choice fresh. A name must be one
/// word of 3-12 letters, not a generic tool or role word, not held by another
/// session, and not carried by another session in the last
/// [`FRESHNESS_DAYS`]. Every refusal says why, so the agent can try again.
pub fn self_name(
    a: &mut Aliases,
    session: &str,
    name: &str,
    now_ms: u64,
) -> Result<(String, bool), String> {
    let wanted = name.trim().to_ascii_lowercase();
    if a.self_named.contains(session) {
        let existing = a
            .by_session
            .get(session)
            .expect("self-named sessions have an alias");
        return if existing == &wanted {
            Ok((existing.clone(), false))
        } else {
            Err(format!(
                "this session has already introduced itself as {}",
                display(existing)
            ))
        };
    }
    let letters = wanted.chars().count();
    if !(3..=12).contains(&letters) || !wanted.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!(
            "\"{}\" is not a callsign: use one word of 3-12 letters (A-Z, no digits, spaces or hyphens); choose another name",
            name.trim()
        ));
    }
    if is_generic(&wanted) {
        return Err(format!(
            "\"{}\" is a generic word, not a name; choose a proper name with character, the kind you would give a person or a character",
            display(&wanted)
        ));
    }
    if let Some(why) = unavailable(a, session, &wanted, now_ms) {
        return Err(why);
    }
    if let Some(previous) = a.by_session.insert(session.to_string(), wanted.clone())
        && previous != wanted
    {
        remember(a, &previous, session, now_ms);
    }
    remember(a, &wanted, session, now_ms);
    a.self_named.insert(session.to_string());
    Ok((wanted, true))
}

/// Read-only lookup, for surfaces that must not assign (the API, the console).
pub fn get(a: &Aliases, session: &str) -> Option<String> {
    a.by_session.get(session).cloned()
}

/// Title case for display: `nova` -> `Nova`.
pub fn display(alias: &str) -> String {
    let mut c = alias.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use std::collections::HashSet;

    const DAY_MS: u64 = 24 * 60 * 60 * 1000;
    const T0: u64 = 1_800_000_000_000;

    fn is_pronounceable(name: &str) -> bool {
        (3..=12).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_lowercase())
            && name.chars().any(|c| "aeiou".contains(c))
    }

    #[test]
    fn the_same_session_always_gets_the_same_name() {
        // Determinism is the point: a name that moves is not a name.
        let a = preferred("019fa3be-ce60-70e1-9cd0-f99326873d69");
        let b = preferred("019fa3be-ce60-70e1-9cd0-f99326873d69");
        assert_eq!(a, b);
        assert!(is_pronounceable(&a), "fallback reads as a name: {a}");
    }

    #[test]
    fn fallback_names_are_pinned() {
        // A session's fallback name is what the operator knows it by: a
        // change here renames agents under their feet.
        assert_eq!(
            preferred("019fa3be-ce60-70e1-9cd0-f99326873d69"),
            "caroline"
        );
        assert_eq!(preferred("s1"), "kathleen");
        assert_eq!(preferred("root"), "harry");
        assert_eq!(preferred("victim"), "vivian");
    }

    #[test]
    fn fallback_names_are_pronounceable_and_varied() {
        let names: HashSet<String> = (0..200).map(|i| preferred(&format!("s-{i}"))).collect();
        assert!(
            names.len() >= 130,
            "two hundred sessions, varied: {}",
            names.len()
        );
        for name in &names {
            assert!(is_pronounceable(name), "{name}");
            assert!(!name.starts_with("agent"), "{name}");
        }
    }

    #[test]
    fn resolve_is_stable_and_idempotent() {
        let mut a = Aliases::default();
        let (first, fresh) = resolve(&mut a, "s1", T0);
        assert!(fresh);
        let (again, fresh_again) = resolve(&mut a, "s1", T0);
        assert_eq!(first, again);
        assert!(!fresh_again, "an assigned session is not newly named");
    }

    #[test]
    fn a_fallback_collision_never_shares_a_name() {
        let mut a = Aliases::default();
        let target = preferred("victim");
        a.by_session.insert("squatter".to_string(), target.clone());
        let (name, _) = resolve(&mut a, "victim", T0);
        assert_ne!(name, target, "two sessions must never share one name");
        assert!(
            is_pronounceable(&name),
            "the next candidate is a name too: {name}"
        );
    }

    #[test]
    fn a_fallback_skips_names_used_recently_by_others() {
        let mut a = Aliases::default();
        remember(&mut a, &preferred("victim"), "ghost", T0);
        let (name, _) = resolve(&mut a, "victim", T0 + DAY_MS);
        assert_ne!(name, preferred("victim"));
    }

    #[test]
    fn with_every_name_recent_the_one_rested_longest_comes_back() {
        let mut a = Aliases::default();
        for i in 0..crate::workshop::name_count() {
            let name = crate::workshop::nth_name(i).0.to_ascii_lowercase();
            remember(&mut a, &name, "ghost", T0 + 1 + i as u64);
        }
        let oldest = crate::workshop::nth_name(0).0.to_ascii_lowercase();
        let (name, _) = resolve(&mut a, "victim", T0 + DAY_MS);
        assert_eq!(name, oldest);
    }

    #[test]
    fn with_every_name_held_the_fallback_takes_a_roman_numeral() {
        let mut a = Aliases::default();
        for i in 0..crate::workshop::name_count() {
            let name = crate::workshop::nth_name(i).0.to_ascii_lowercase();
            a.by_session.insert(format!("squatter-{i}"), name);
        }
        let (name, _) = resolve(&mut a, "victim", T0);
        assert_eq!(name, format!("{}-ii", preferred("victim")));
        assert_eq!(roman(4), "iv");
        assert_eq!(roman(14), "xiv");
    }

    #[test]
    fn a_generated_name_from_before_the_list_is_replaced_and_a_chosen_one_stays() {
        let mut a = Aliases::default();
        a.by_session.insert("old".into(), "ferde".into());
        a.by_session.insert("mine".into(), "vesper".into());
        a.self_named.insert("mine".into());
        let (name, fresh) = resolve(&mut a, "old", T0);
        assert!(fresh, "the new name is announced");
        assert!(crate::workshop::listed(&name).is_some(), "{name}");
        assert_eq!(resolve(&mut a, "mine", T0).0, "vesper");
    }

    #[test]
    fn a_new_session_does_not_take_a_name_an_agent_goes_by() {
        let natural = preferred("s1");
        let mut a = Aliases::default();
        a.reserved.insert(natural.clone());
        let (name, fresh) = resolve(&mut a, "s1", T0);
        assert!(fresh);
        assert_ne!(name, natural);
    }

    #[test]
    fn a_session_keeps_its_name_when_an_agent_goes_by_it() {
        let mut a = Aliases::default();
        let (name, _) = resolve(&mut a, "s1", T0);
        a.reserved.insert(name.clone());
        assert_eq!(resolve(&mut a, "s1", T0 + 1), (name, false));
    }

    #[test]
    fn a_session_cannot_introduce_itself_with_an_agents_name() {
        let mut a = Aliases::default();
        a.reserved.insert("hilbert".into());
        let refused = self_name(&mut a, "s1", "Hilbert", T0).unwrap_err();
        assert!(refused.contains("Hilbert"), "{refused}");
    }

    #[test]
    fn the_names_of_agents_are_read_but_never_written_to_the_ledger() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let named = |name: &str, source: &str| -> crate::workshop::Entry {
            serde_json::from_value(serde_json::json!({
                "name": name, "gender": "m", "team": null, "role": null,
                "source": source, "reason": null, "assigned_ms": 1
            }))
            .unwrap()
        };
        crate::workshop::Registry::update(store.root(), |r| {
            r.agents.insert("a1".into(), named("Oscar", "auto"));
            r.agents
                .insert("session:s9".into(), named("Nora", "picked"));
        })
        .unwrap();
        let a = read_for_naming(&store);
        assert_eq!(a.reserved, BTreeSet::from(["oscar".to_string()]));
        write(&store, &a).unwrap();
        assert!(read(&store).reserved.is_empty());
    }

    #[test]
    fn ledger_round_trips_and_survives_corruption() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut a = Aliases::default();
        let (name, _) = resolve(&mut a, "s1", T0);
        write(&store, &a).unwrap();
        let back = read(&store);
        assert_eq!(get(&back, "s1").as_deref(), Some(name.as_str()));
        assert_eq!(back.history[&name].session, "s1");

        std::fs::write(store.root().join("aliases.json"), b"{oops").unwrap();
        assert!(read(&store).by_session.is_empty());
    }

    #[test]
    fn a_ledger_written_before_history_still_loads() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        std::fs::create_dir_all(store.root()).unwrap();
        std::fs::write(
            store.root().join("aliases.json"),
            br#"{"by_session":{"root":"agent-1acfa1-1","peer":"nova"},"self_named":["peer"]}"#,
        )
        .unwrap();
        let a = read(&store);
        assert_eq!(get(&a, "root").as_deref(), Some("agent-1acfa1-1"));
        assert!(a.self_named.contains("peer"));
        assert!(a.history.is_empty());
    }

    #[test]
    fn display_titlecases() {
        assert_eq!(display("nova"), "Nova");
        assert_eq!(display(""), "");
    }

    #[test]
    fn claim_pins_a_free_nickname() {
        let mut a = Aliases::default();
        let (name, fresh) = claim(&mut a, "child-1", "Hilbert", T0);
        assert!(fresh);
        assert_eq!(name, "hilbert");
        let (again, fresh_again) = claim(&mut a, "child-1", "Hooke", T0);
        assert_eq!(again, "hilbert");
        assert!(!fresh_again);
    }

    #[test]
    fn claim_falls_back_when_nickname_taken() {
        let mut a = Aliases::default();
        let _ = claim(&mut a, "a", "Hilbert", T0);
        let (name, fresh) = claim(&mut a, "b", "Hilbert", T0);
        assert!(fresh);
        assert_ne!(name, "hilbert");
        assert_eq!(name, preferred("b"));
    }

    #[test]
    fn a_session_can_replace_its_provisional_name_once() {
        let mut aliases = Aliases::default();
        let (fallback, _) = resolve(&mut aliases, "root", T0);
        assert_eq!(fallback, preferred("root"));

        let (chosen, fresh) = self_name(&mut aliases, "root", "Ottoline", T0).unwrap();
        assert!(fresh);
        assert_eq!(chosen, "ottoline");
        assert_eq!(get(&aliases, "root").as_deref(), Some("ottoline"));
        let again = self_name(&mut aliases, "root", "Vesper", T0).unwrap_err();
        assert!(
            again.contains("already introduced itself as Ottoline"),
            "{again}"
        );
        // The provisional name was shown on the board, so it counts as used.
        assert_eq!(aliases.history[&fallback].session, "root");
    }

    #[test]
    fn two_sessions_cannot_introduce_themselves_with_one_name() {
        let mut aliases = Aliases::default();
        self_name(&mut aliases, "a", "Nova", T0).unwrap();
        let err = self_name(&mut aliases, "b", "nova", T0).unwrap_err();
        assert!(err.contains("already in use"), "{err}");
    }

    #[test]
    fn generic_words_are_refused_with_a_reason() {
        let mut aliases = Aliases::default();
        for word in ["Wrench", "helper", "AGENT", "Conduit", "Claude"] {
            let err = self_name(&mut aliases, "s", word, T0).unwrap_err();
            assert!(err.contains("generic word"), "{word}: {err}");
            assert!(err.contains("choose a proper name"), "{word}: {err}");
        }
        assert!(aliases.by_session.is_empty(), "a refusal assigns nothing");
    }

    #[test]
    fn misshapen_names_are_refused_with_a_reason() {
        let mut aliases = Aliases::default();
        for bad in [
            "Al",
            "Nightingaleous",
            "R2D2",
            "night-owl",
            "two words",
            "",
            "Zoë",
        ] {
            let err = self_name(&mut aliases, "s", bad, T0).unwrap_err();
            assert!(err.contains("3-12 letters"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn a_fresh_proper_name_is_accepted() {
        let mut aliases = Aliases::default();
        let (name, fresh) = self_name(&mut aliases, "s", "  Bramwell ", T0).unwrap();
        assert!(fresh);
        assert_eq!(name, "bramwell");
        assert!(aliases.self_named.contains("s"));
    }

    #[test]
    fn a_recent_name_survives_release_and_expires_after_ninety_days() {
        let mut aliases = Aliases::default();
        self_name(&mut aliases, "old", "Vesper", T0).unwrap();
        let released = release_inactive(&mut aliases, &HashSet::new(), T0 + DAY_MS);
        assert_eq!(released, 1);
        assert!(get(&aliases, "old").is_none(), "the dead do not hold names");

        let err = self_name(&mut aliases, "new", "VESPER", T0 + 89 * DAY_MS).unwrap_err();
        assert!(err.contains("last 90 days"), "{err}");
        assert!(err.contains("choose another"), "{err}");

        // The session that carried it may take it back.
        assert!(self_name(&mut aliases.clone(), "old", "Vesper", T0 + 2 * DAY_MS).is_ok());

        // Ninety days after its last use the name is free again.
        let later = T0 + DAY_MS + FRESHNESS_DAYS * DAY_MS;
        let (name, _) = self_name(&mut aliases, "new", "Vesper", later).unwrap();
        assert_eq!(name, "vesper");
    }

    #[test]
    fn history_prunes_what_has_aged_out() {
        let mut aliases = Aliases::default();
        remember(&mut aliases, "tavor", "a", T0);
        remember(&mut aliases, "mirel", "b", T0 + 50 * DAY_MS);
        release_inactive(&mut aliases, &HashSet::new(), T0 + 100 * DAY_MS);
        assert!(!aliases.history.contains_key("tavor"));
        assert!(aliases.history.contains_key("mirel"));
    }

    #[test]
    fn the_epithet_is_deterministic_and_shaped() {
        let a = epithet("019fa3be-ce60-70e1-9cd0-f99326873d69");
        let b = epithet("019fa3be-ce60-70e1-9cd0-f99326873d69");
        assert_eq!(a, b);
        assert!(a.starts_with("the "), "shape is `the <adj> <noun>`: {a}");
        assert_eq!(a.split(' ').count(), 3, "one adj, one noun: {a}");
    }

    #[test]
    fn epithets_vary_across_sessions() {
        let set: std::collections::HashSet<String> =
            (0..40).map(|i| epithet(&format!("session-{i}"))).collect();
        assert!(set.len() >= 10, "forty sessions should not share one soul");
    }
}
