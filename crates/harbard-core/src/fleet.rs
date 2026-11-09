//! The fleet projection: one read model, computed once, pushed live.
//!
//! Before this module the store had one reducer in Rust and a second one in
//! TypeScript, so the same session was `Fable-redesign` on one screen, `Claude
//! Code` on another and a raw uuid on the bus, and the live count moved between
//! 10, 11 and 12 within a minute. Nothing here derives anything new: it folds
//! [`query::sessions`], [`topology`], [`alias`], the lease/membership/operator
//! stores and the sluice receipts into a single object, so every surface reads
//! the same numbers instead of recomputing its own.
//!
//! Clock and staleness arrive as parameters (never read here) so tests inject
//! them — the same rule the rest of the read side follows.

use crate::alias::{self, Aliases};
use crate::checkout_history::DeliveryStamp;
use crate::event::{Event, EventKind, HARNESS_HUMAN, NoteKind, RecipientKind, Role};
use crate::lease::WriterLeaseView;
use crate::membership::{self, Member, Memberships, RoomVisibility};
use crate::operator::OperatorProfile;
use crate::query::{self, Liveness, SessionView, event_ms};
use crate::report;
use crate::sluice;
use crate::store::Store;
use crate::topology;
use crate::wake::AttentionIndex;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// How long an operator-addressed note stays owed once its sender is no longer
/// live. A decision handed to the human must outlive the session that asked
/// (a weekend, a closed laptop), but not forever: the same week the health
/// counters use for unacked handoffs.
pub const ORPHANED_OPERATOR_NOTE_MAX_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// Wire version of the projection. Bumped only by a spec change.
pub const FLEET_VERSION: u32 = 1;

/// How long an `ended` session stays on the fleet before it belongs to History.
pub const ENDED_VISIBLE_MS: u64 = 6 * 60 * 60 * 1000;

/// How long a delivery keeps a quiet checkout on the board.
pub const DELIVERY_VISIBLE_MS: u64 = 24 * 60 * 60 * 1000;

/// Cap on the unread badge; past this the exact number stops being information.
pub const UNREAD_CAP: u64 = 99;

/// Ceiling on a serialized `fleet` frame (§6).
///
/// The frame rides every accepted event, so its size is a streaming cost, not
/// a page-load cost. Most of its bytes can be sessions that have already ended:
/// those are History's material, so they are what gives way.
///
/// Raised from 64 KB once the budget was measured on the whole body instead of
/// on `sessions` alone: a board with twenty leased trees and fifty open asks
/// is ~90 KB with every debt intact, and `pending` is never shed to make a
/// number fit — an obligation the operator cannot see goes unpaid.
pub const MAX_FRAME_BYTES: usize = 128 * 1024;

/// The whole board in one object.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FleetView {
    pub v: u32,
    pub now_ms: u64,
    /// ULID of the newest event folded in — the stream resume anchor.
    pub as_of: Option<String>,
    pub stale_secs: u64,
    pub counts: Counts,
    pub sessions: Vec<SessionCard>,
    pub checkouts: Vec<CheckoutCard>,
    pub rooms: Vec<RoomCard>,
    pub pending: Vec<PendingItem>,
    pub operator: OperatorCard,
    /// Operator read cursors, keyed by place id.
    pub reads: BTreeMap<String, String>,
    pub hub: HubCard,
}

/// THE numbers. No surface computes its own.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct Counts {
    /// presence=live, not ending, depth 0.
    pub live: u64,
    /// presence=live, not ending, depth>0.
    pub delegated: u64,
    pub stale: u64,
    pub pending: u64,
    pub contested: u64,
    /// Unread feed items across every place with a read cursor.
    pub unread_all: u64,
}

/// Presence as it crosses the wire. Unlike `SessionView`, `ended` is a first
/// class answer here: rewriting it to `stale` in one route and filtering it out
/// in another is what made the same session read three different ways.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Live,
    Stale,
    Ended,
}

impl Presence {
    pub fn as_str(self) -> &'static str {
        match self {
            Presence::Live => "live",
            Presence::Stale => "stale",
            Presence::Ended => "ended",
        }
    }

    fn from_liveness(liveness: Liveness) -> Self {
        match liveness {
            Liveness::Live => Presence::Live,
            Liveness::Stale => Presence::Stale,
            Liveness::Ended => Presence::Ended,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionCard {
    pub session: String,
    /// Resolved server-side for every row; never a raw id.
    pub display_name: String,
    /// The session chose its own callsign.
    pub self_named: bool,
    pub epithet: String,
    pub harness: String,
    pub host: String,
    pub role: String,
    pub cwd: String,
    /// Checkout id, or null when `cwd` is not inside a git checkout (E1).
    pub checkout: Option<String>,
    pub branch: Option<String>,
    pub parent: Option<String>,
    pub root: String,
    pub depth: u32,
    pub descendants: u32,
    pub identity: topology::IdentityConfidence,
    pub intent: Option<String>,
    pub intent_ms: u64,
    pub presence: Presence,
    /// Said bye, still inside the herd grace window — never counted live.
    pub ending: bool,
    pub holds_lease: bool,
    pub first_ms: u64,
    pub last_ms: u64,
    /// `now_ms - last_ms`, so clients never need a clock of their own.
    pub signal_age_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckoutCard {
    /// Canonical worktree path.
    pub id: String,
    /// Repository identity shared by sibling worktrees.
    pub repo_key: String,
    pub label: String,
    /// Worktree name, or null for the primary checkout.
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub occupants: Vec<String>,
    pub live: u64,
    pub delegated: u64,
    pub contested: bool,
    /// Distinct writer-domain roots when contested.
    pub rivals: Vec<String>,
    pub lease: Option<WriterLeaseView>,
    pub last_delivery: Option<DeliveryStamp>,
    pub last_signal_ms: u64,
    pub latest_item: Option<String>,
    pub unread: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RoomCard {
    pub name: String,
    pub visibility: RoomVisibility,
    pub members: Vec<Member>,
    pub repo_alias: Option<String>,
    pub archived: bool,
    pub last_activity_ms: u64,
    pub latest_item: Option<String>,
    pub unread: u64,
}

/// An obligation the operator can see and act on.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PendingItem {
    pub note: String,
    pub kind: String,
    pub from: String,
    pub from_name: String,
    pub asked_ms: u64,
    pub summary: String,
    pub owed_by: Vec<OwedBy>,
    /// How many readers owe this answer in total. Equal to `owed_by.len()` on
    /// a frame that fits; larger when §6 trimmed the list to keep the frame
    /// inside its budget, so a client can say "and 63 more" without guessing.
    pub owed_total: u64,
    pub acked_by: Vec<String>,
    pub place: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OwedBy {
    pub session: String,
    pub display_name: String,
    pub presence: Presence,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OperatorCard {
    pub session: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct HubCard {
    pub enabled: bool,
    pub devices: u64,
}

/// Everything the projection needs that does not come from the event slice.
///
/// Passed in rather than read here so `serve` can hand over the values it
/// already holds (leases it fetched, the hub flag it was configured with) and
/// tests can build a board without touching those stores.
pub struct FleetInputs<'a> {
    pub store: &'a Store,
    pub events: &'a [Event],
    pub now_ms: u64,
    pub stale_secs: u64,
    pub leases: &'a [WriterLeaseView],
    pub hub_enabled: bool,
    pub devices: u64,
}

/// The place id of a checkout, as the feed and the read cursors spell it.
pub fn checkout_place(id: &str) -> String {
    format!("checkout:{id}")
}

/// The place id of a room.
pub fn room_place(name: &str) -> String {
    format!("room:{name}")
}

/// The place id of a session.
pub fn session_place(id: &str) -> String {
    format!("session:{id}")
}

/// Project name for a repository path: its last non-empty path segment.
pub fn label_for(repo: &str) -> String {
    repo.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(repo)
        .to_string()
}

/// The worktree name for a checkout inside a repository, or `None` when this
/// checkout IS the repository (the primary working tree).
pub fn worktree_name(id: &str, repo_key: &str) -> Option<String> {
    if id == repo_key {
        return None;
    }
    Some(label_for(id))
}

/// A session is a real occupant of a checkout — the human operator sits at the
/// desk, not in the tree, and rows without a checkout belong to E1.
fn occupies_checkouts(view: &SessionView) -> bool {
    view.harness != HARNESS_HUMAN && view.role != Role::Operator
}

/// The checkout a session sits in: its own working tree, never the repository.
fn checkout_of(view: &SessionView) -> Option<&str> {
    view.repo
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// The repository a checkout belongs to, shared by sibling worktrees.
fn repo_key_of(view: &SessionView) -> Option<&str> {
    view.repo_key
        .as_deref()
        .or(view.repo.as_deref())
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// True when a row counts toward `live` (never an `ending` farewell).
///
/// One predicate, two callers: `counts.live`/`delegated` fold the cards, and
/// each `CheckoutCard` folds its occupants' views. Two spellings of "counts as
/// live" is exactly how the header came to say 10, 11 and 12 within a minute,
/// so the definition lives here and the depth split is the caller's business.
fn is_live(presence: Presence, ending: bool) -> bool {
    presence == Presence::Live && !ending
}

/// [`is_live`] for a row that has not become a card yet.
fn counts_live(view: &SessionView) -> bool {
    is_live(Presence::from_liveness(view.liveness), view.ending)
}

/// [`is_live`] for a finished card.
fn card_counts_live(card: &SessionCard) -> bool {
    is_live(card.presence, card.ending)
}

/// Build the whole board.
pub fn build(inputs: &FleetInputs<'_>) -> FleetView {
    let FleetInputs {
        store,
        events,
        now_ms,
        stale_secs,
        leases,
        hub_enabled,
        devices,
    } = *inputs;

    let views = query::sessions(store, events, now_ms, stale_secs);
    let aliases = alias::read(store);
    let memberships = membership::read(store);
    let profile = OperatorProfile::load(store);
    let reads = read_cursors(store);

    let lease_holders: HashSet<&str> = leases.iter().map(|lease| lease.session.as_str()).collect();

    let sessions = session_cards(&views, &aliases, &lease_holders, events, now_ms);
    let checkouts = checkout_cards(
        store,
        &views,
        &lease_holders,
        leases,
        events,
        &reads,
        now_ms,
    );
    let rooms = room_cards(&memberships, events, &reads);
    let operator_card = OperatorCard {
        session: format!("operator@{}", crate::hostname()),
        display_name: profile.display_name.clone(),
    };
    let placement = crate::workspaces::Placement::load(store);
    let pending = pending_items(
        events,
        &views,
        &aliases,
        &memberships,
        &operator_card,
        &placement,
        now_ms,
    );

    let counts = Counts {
        live: sessions
            .iter()
            .filter(|card| card_counts_live(card) && card.depth == 0)
            .count() as u64,
        delegated: sessions
            .iter()
            .filter(|card| card_counts_live(card) && card.depth > 0)
            .count() as u64,
        stale: sessions
            .iter()
            .filter(|card| card.presence == Presence::Stale)
            .count() as u64,
        pending: pending.len() as u64,
        contested: checkouts.iter().filter(|card| card.contested).count() as u64,
        unread_all: checkouts
            .iter()
            .map(|card| card.unread)
            .chain(rooms.iter().map(|card| card.unread))
            .sum::<u64>()
            .min(UNREAD_CAP),
    };

    let view = FleetView {
        v: FLEET_VERSION,
        now_ms,
        as_of: events.last().map(|event| event.id.clone()),
        stale_secs,
        counts,
        sessions,
        checkouts,
        rooms,
        pending,
        operator: operator_card,
        reads,
        hub: HubCard {
            enabled: hub_enabled,
            devices,
        },
    };

    // The counts above are computed from the WHOLE board, so trimming below
    // never changes a number. A count that moved because a row was dropped for
    // size would be exactly the lie this projection exists to end.
    trim_to_frame_budget(view)
}

/// A display name for every row, resolved without ever writing the ledger.
///
/// `alias::resolve` assigns on read, which is right for `hello` and wrong here:
/// a browser refresh must not mint identities. So a session the ledger does not
/// know falls back to its derived name (E5) instead of claiming one.
///
/// This is the one resolver every surface uses (the board, the feed, `ls
/// --json`), so a console, the desktop app and a status line cannot name the
/// same session differently. The bool says whether the session chose its name.
pub fn display_name(aliases: &Aliases, session: &str) -> (String, bool) {
    match aliases.by_session.get(session) {
        Some(name) => (alias::display(name), aliases.self_named.contains(session)),
        None => (alias::display(&alias::preferred(session)), false),
    }
}

/// When each session last declared an intent, for `intent_ms`.
fn intent_times(events: &[Event]) -> HashMap<&str, u64> {
    let mut out: HashMap<&str, u64> = HashMap::new();
    for event in events {
        if matches!(event.event, EventKind::Intent | EventKind::Hello) && !event.text.is_empty() {
            out.insert(event.session.as_str(), event_ms(event));
        }
    }
    out
}

fn session_cards(
    views: &[SessionView],
    aliases: &Aliases,
    lease_holders: &HashSet<&str>,
    events: &[Event],
    now_ms: u64,
) -> Vec<SessionCard> {
    let intents = intent_times(events);
    let mut cards: Vec<SessionCard> = views
        .iter()
        .filter(|view| {
            // Ended rows stay on the board only while they are still recent;
            // older ones are History's job, reachable through `/feed`.
            view.liveness != Liveness::Ended
                || now_ms.saturating_sub(view.last_ms) < ENDED_VISIBLE_MS
        })
        .map(|view| {
            let (display, self_named) = display_name(aliases, &view.session);
            SessionCard {
                display_name: display,
                self_named,
                epithet: alias::epithet(&view.session),
                harness: view.harness.clone(),
                host: view.host.clone(),
                role: view.role.as_str().to_string(),
                cwd: view.cwd.clone(),
                checkout: checkout_of(view).map(str::to_string),
                branch: view.branch.clone(),
                parent: view.parent.clone(),
                root: view.root.clone(),
                depth: view.depth,
                descendants: view.descendants,
                identity: view.identity,
                intent: view.intent.clone(),
                intent_ms: intents.get(view.session.as_str()).copied().unwrap_or(0),
                presence: Presence::from_liveness(view.liveness),
                ending: view.ending,
                holds_lease: lease_holders.contains(view.session.as_str()),
                first_ms: view.first_ms,
                last_ms: view.last_ms,
                signal_age_ms: now_ms.saturating_sub(view.last_ms),
                session: view.session.clone(),
            }
        })
        .collect();

    order_sessions(&mut cards);
    cards
}

/// Presence order (live, stale, ended), newest signal first, with every child
/// immediately under its parent so a herd reads as one block.
fn order_sessions(cards: &mut Vec<SessionCard>) {
    fn rank(card: &SessionCard) -> u8 {
        match card.presence {
            Presence::Live => 0,
            Presence::Stale => 1,
            Presence::Ended => 2,
        }
    }

    let mut children: HashMap<String, Vec<SessionCard>> = HashMap::new();
    let known: HashSet<String> = cards.iter().map(|card| card.session.clone()).collect();
    let mut roots: Vec<SessionCard> = Vec::new();
    for card in cards.drain(..) {
        match card.parent.clone().filter(|parent| known.contains(parent)) {
            Some(parent) => children.entry(parent).or_default().push(card),
            None => roots.push(card),
        }
    }
    let sort = |rows: &mut Vec<SessionCard>| {
        rows.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then(b.last_ms.cmp(&a.last_ms))
                .then(a.session.cmp(&b.session))
        });
    };
    sort(&mut roots);
    for rows in children.values_mut() {
        sort(rows);
    }

    // Depth-first with a visited guard: a cycle in the parent links must leave
    // rows out of the tree walk, never spin here.
    let mut emitted: HashSet<String> = HashSet::new();
    let mut out: Vec<SessionCard> = Vec::with_capacity(roots.len());
    let mut stack: Vec<SessionCard> = roots.into_iter().rev().collect();
    while let Some(card) = stack.pop() {
        if !emitted.insert(card.session.clone()) {
            continue;
        }
        let kids = children.remove(&card.session).unwrap_or_default();
        out.push(card);
        for kid in kids.into_iter().rev() {
            stack.push(kid);
        }
    }
    // Anything still parked under an unreachable parent (a cycle) is appended
    // rather than dropped: an odd row on the board beats a missing session.
    let mut orphans: Vec<SessionCard> = children.into_values().flatten().collect();
    sort(&mut orphans);
    for card in orphans {
        if emitted.insert(card.session.clone()) {
            out.push(card);
        }
    }
    *cards = out;
}

#[allow(clippy::too_many_arguments)]
fn checkout_cards(
    store: &Store,
    views: &[SessionView],
    lease_holders: &HashSet<&str>,
    leases: &[WriterLeaseView],
    events: &[Event],
    reads: &BTreeMap<String, String>,
    now_ms: u64,
) -> Vec<CheckoutCard> {
    // Every checkout with a reason to exist: an occupant, a lease, or a recent
    // delivery. A path nobody is in and nothing happened to is not a card —
    // the harbor used to paint six empty berths out of exactly that.
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for view in views {
        if !occupies_checkouts(view) {
            continue;
        }
        if !matches!(view.liveness, Liveness::Live | Liveness::Stale) {
            continue;
        }
        if let Some(checkout) = checkout_of(view) {
            candidates.insert(checkout.to_string());
        }
    }
    for lease in leases {
        if !lease.worktree.is_empty() {
            candidates.insert(lease.worktree.clone());
        }
    }
    // A tree nobody is in still belongs on the board while its delivery is
    // fresh (§1.2): the receipt is the only thing that says work landed there.
    // One walk of the sluice directory, not one file read per card.
    let receipts = sluice::latest_receipt_by_repo(store.root());
    for (repo, receipt) in &receipts {
        if !repo.is_empty() && now_ms.saturating_sub(receipt.decided_ms) < DELIVERY_VISIBLE_MS {
            candidates.insert(repo.clone());
        }
    }

    let latest_items = latest_item_by_checkout(events, views);
    let mut cards = Vec::new();
    for id in candidates {
        let occupant_views: Vec<&SessionView> = views
            .iter()
            .filter(|view| occupies_checkouts(view))
            .filter(|view| checkout_of(view) == Some(id.as_str()))
            .filter(|view| matches!(view.liveness, Liveness::Live | Liveness::Stale))
            .collect();

        let repo_key = occupant_views
            .iter()
            .find_map(|view| repo_key_of(view))
            .unwrap_or(id.as_str())
            .to_string();
        let branch = occupant_views
            .iter()
            .find_map(|view| view.branch.clone())
            .or_else(|| {
                leases
                    .iter()
                    .find(|lease| lease.worktree == id)
                    .and_then(|lease| lease.branch.clone())
            });

        let mut occupants: Vec<&&SessionView> = occupant_views.iter().collect();
        occupants.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.session.cmp(&b.session)));
        let occupant_ids: Vec<String> = occupants.iter().map(|view| view.session.clone()).collect();

        let live = occupant_views
            .iter()
            .filter(|view| counts_live(view) && view.depth == 0)
            .count() as u64;
        let delegated = occupant_views
            .iter()
            .filter(|view| counts_live(view) && view.depth > 0)
            .count() as u64;

        let rivals = topology::distinct_writer_domains(views, &id, lease_holders);
        let contested = rivals.len() >= 2;

        let lease = leases.iter().find(|lease| lease.worktree == id).cloned();

        let last_delivery = receipts.get(&id).map(|receipt| DeliveryStamp {
            sha: receipt
                .candidate
                .clone()
                .unwrap_or_else(|| receipt.delivered_sha.clone()),
            verdict: receipt.verdict.as_str().to_string(),
            decided_ms: receipt.decided_ms,
        });

        let last_signal_ms = occupant_views
            .iter()
            .map(|view| view.last_ms)
            .max()
            .unwrap_or(0)
            .max(lease.as_ref().map_or(0, |lease| lease.renewed_at_ms))
            .max(last_delivery.as_ref().map_or(0, |stamp| stamp.decided_ms));

        // A checkout with no occupant and no lease survives only on a fresh
        // delivery; otherwise it has fallen off the live board.
        if occupant_views.is_empty()
            && lease.is_none()
            && last_delivery
                .as_ref()
                .is_none_or(|stamp| now_ms.saturating_sub(stamp.decided_ms) >= DELIVERY_VISIBLE_MS)
        {
            continue;
        }

        let latest_item = latest_items.get(&id).cloned();
        let unread = unread_since(
            reads.get(&checkout_place(&id)).map(String::as_str),
            latest_item.as_deref(),
            events,
            |event| event_reaches_checkout(event, &id, &repo_key, &occupant_ids),
        );

        cards.push(CheckoutCard {
            worktree: worktree_name(&id, &repo_key),
            label: label_for(&repo_key),
            repo_key,
            branch,
            occupants: occupant_ids,
            live,
            delegated,
            contested,
            rivals: if contested { rivals } else { Vec::new() },
            lease,
            last_delivery,
            last_signal_ms,
            latest_item,
            unread,
            id,
        });
    }

    cards.sort_by(|a, b| {
        b.contested
            .cmp(&a.contested)
            .then(b.live.cmp(&a.live))
            .then(b.last_signal_ms.cmp(&a.last_signal_ms))
            .then(a.id.cmp(&b.id))
    });
    cards
}

/// True when this event belongs to a checkout's timeline — addressed to the
/// repository (either spelling, as delivery has matched since 0.3.0-alpha.5),
/// or written from the tree by one of its occupants.
pub fn event_reaches_checkout(
    event: &Event,
    id: &str,
    repo_key: &str,
    occupants: &[String],
) -> bool {
    if let Some(to) = event.to.as_ref()
        && to.kind == RecipientKind::Repo
        && (to.value == id || to.value == repo_key)
    {
        return true;
    }
    if event.repo.as_deref() == Some(id) && event.to.is_none() {
        return true;
    }
    occupants.contains(&event.session) && event.repo.as_deref() == Some(id)
}

fn latest_item_by_checkout(events: &[Event], views: &[SessionView]) -> HashMap<String, String> {
    // Occupants and the repository each tree belongs to. A linked worktree's
    // repo_key is its primary checkout, and a note addressed to `repo:<primary>`
    // reaches every sibling — reading the tree's own path as its repo_key made
    // that note invisible here while `unread` still counted it.
    let mut occupants_by_checkout: HashMap<&str, Vec<String>> = HashMap::new();
    let mut repo_key_by_checkout: HashMap<&str, &str> = HashMap::new();
    for view in views {
        if let Some(checkout) = checkout_of(view) {
            occupants_by_checkout
                .entry(checkout)
                .or_default()
                .push(view.session.clone());
            if let Some(repo_key) = repo_key_of(view) {
                repo_key_by_checkout.entry(checkout).or_insert(repo_key);
            }
        }
    }
    let mut out: HashMap<String, String> = HashMap::new();
    for event in events {
        for (checkout, occupants) in &occupants_by_checkout {
            let repo_key = repo_key_by_checkout
                .get(checkout)
                .copied()
                .unwrap_or(*checkout);
            if event_reaches_checkout(event, checkout, repo_key, occupants) {
                out.insert((*checkout).to_string(), event.id.clone());
            }
        }
    }
    out
}

/// Items newer than the operator's cursor for a place, capped at [`UNREAD_CAP`].
///
/// No cursor means nothing has been read there yet, which would make every
/// quiet room shout on first load; the honest answer for an unvisited place is
/// the count of what is actually there, so the cap does the shouting instead.
fn unread_since(
    cursor: Option<&str>,
    latest: Option<&str>,
    events: &[Event],
    belongs: impl Fn(&Event) -> bool,
) -> u64 {
    let Some(latest) = latest else {
        return 0;
    };
    let Some(cursor) = cursor else {
        return events
            .iter()
            .filter(|event| belongs(event))
            .count()
            .min(UNREAD_CAP as usize) as u64;
    };
    if cursor >= latest {
        return 0;
    }
    events
        .iter()
        .filter(|event| event.id.as_str() > cursor && belongs(event))
        .count()
        .min(UNREAD_CAP as usize) as u64
}

fn room_cards(
    memberships: &Memberships,
    events: &[Event],
    reads: &BTreeMap<String, String>,
) -> Vec<RoomCard> {
    let mut cards = Vec::new();
    for (name, members) in &memberships.channels {
        if memberships.archived.contains(name) {
            continue;
        }
        let meta = memberships.meta.get(name);
        let latest_item = events
            .iter()
            .rev()
            .find(|event| event_reaches_room(event, name))
            .map(|event| event.id.clone());
        let unread = unread_since(
            reads.get(&room_place(name)).map(String::as_str),
            latest_item.as_deref(),
            events,
            |event| event_reaches_room(event, name),
        );
        cards.push(RoomCard {
            name: name.clone(),
            visibility: meta.map(|meta| meta.visibility).unwrap_or_default(),
            members: members.clone(),
            repo_alias: membership::repo_alias(memberships, name),
            archived: false,
            last_activity_ms: meta.and_then(|meta| meta.last_note_ms).unwrap_or(0),
            latest_item,
            unread,
        });
    }
    cards.sort_by(|a, b| {
        b.last_activity_ms
            .cmp(&a.last_activity_ms)
            .then(a.name.cmp(&b.name))
    });
    cards
}

/// True when this event belongs to a room's timeline: addressed to the channel,
/// or an ack of a note that was.
pub fn event_reaches_room(event: &Event, room: &str) -> bool {
    event
        .to
        .as_ref()
        .is_some_and(|to| to.kind == RecipientKind::Channel && to.value.eq_ignore_ascii_case(room))
}

/// Which sessions a note reached, by the same rules delivery uses.
fn recipients_of<'a>(
    note: &'a Event,
    views: &'a [SessionView],
    channels_by_session: &'a HashMap<&str, Vec<String>>,
    placement: &'a crate::workspaces::Placement,
) -> impl Iterator<Item = &'a SessionView> + 'a {
    views
        .iter()
        .filter(move |view| view.session != note.session)
        .filter(|view| view.harness != HARNESS_HUMAN && view.role != Role::Operator)
        .filter(move |view| {
            let channels = channels_by_session
                .get(view.session.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let gate = placement.gate_of_session(view);
            note.is_addressed_to_reader(&crate::event::Reader {
                session: &view.session,
                harness: &view.harness,
                repo: view.repo.as_deref(),
                repo_key: view.repo_key.as_deref().or(view.repo.as_deref()),
                channels,
                workspace: &gate,
            })
        })
}

/// Channel subscriptions per session, so an `ask` to a room owes an answer from
/// exactly the sessions that would have received it.
fn channels_by_session(memberships: &Memberships) -> HashMap<&str, Vec<String>> {
    let mut out: HashMap<&str, Vec<String>> = HashMap::new();
    for (name, members) in &memberships.channels {
        for member in members {
            if member.kind == crate::membership::AnchorKind::Session {
                out.entry(member.value.as_str())
                    .or_default()
                    .push(name.clone());
            }
        }
    }
    out
}

/// The place a note was addressed to, as a place id.
fn place_of(note: &Event) -> String {
    match note.to.as_ref() {
        None => "all".to_string(),
        Some(to) => match to.kind {
            RecipientKind::Repo => checkout_place(&to.value),
            RecipientKind::Channel => room_place(&to.value),
            RecipientKind::Session => session_place(&to.value),
            RecipientKind::Harness | RecipientKind::Unknown => "all".to_string(),
        },
    }
}

/// Obligations still open: a handoff owed by its one named session, an ask owed
/// by every live session it reached.
///
/// An item exists only while at least one owed session is still live — the
/// playbook's "needs you only while someone is still live to answer". Closure
/// is causal: an `ack`, or a note whose `ref` names the obligation. A delivery
/// cursor proves a session scanned past a log position, never that it answered.
fn pending_items(
    events: &[Event],
    views: &[SessionView],
    aliases: &Aliases,
    memberships: &Memberships,
    operator: &OperatorCard,
    placement: &crate::workspaces::Placement,
    now_ms: u64,
) -> Vec<PendingItem> {
    // Every id that closed something: canonical ACK references and note replies.
    let mut closed_by: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut attention = AttentionIndex::default();
    for event in events {
        attention.observe(event);
        let target = match event.event {
            EventKind::Ack => event.acknowledged_note_id(),
            EventKind::Note => event.note_ref.as_deref(),
            _ => None,
        };
        if let Some(target) = target.filter(|id| !id.is_empty()) {
            closed_by
                .entry(target)
                .or_default()
                .insert(event.session.as_str());
        }
    }

    let subscriptions = channels_by_session(memberships);
    let mut out = Vec::new();
    for note in events {
        if !note.kind.owes_an_answer() || !attention.latest_note(note) {
            continue;
        }
        let acked = closed_by.get(note.id.as_str());
        let current = attention.current_note(note, now_ms);
        let reached = recipients_of(note, views, &subscriptions, placement);
        // Unanswered readers, live or stale (E7). The item survives on its live
        // readers alone, but a stale one who never answered is still part of the
        // answer to "who owes this" — filtering them out here made
        // `OwedBy.presence: "stale"` a shape nothing could ever produce.
        let owed: Vec<&SessionView> = reached
            .filter(|view| current && attention.after_join(&view.session, note))
            .filter(|view| !acked.is_some_and(|sessions| sessions.contains(view.session.as_str())))
            .filter(|view| matches!(view.liveness, Liveness::Live | Liveness::Stale))
            .collect();
        // E8: the operator is a recipient too. `recipients_of` walks agent
        // sessions, so an interrupt aimed at the operator's own session — or a
        // broadcast ask, which reaches whoever is at the console — listed
        // nobody and could not be closed by the operator answering it. Unlike
        // agent recipients, the operator has no log join. The debt still ends
        // like theirs (§1.4) once nobody is left to receive the answer: a
        // sender that is no longer live — a crashed session never says `bye`,
        // so it stays stale forever — releases the operator a week after
        // asking. Without this, one orphaned handoff held "needs you" for
        // months.
        let sender_live = views
            .iter()
            .any(|view| view.session == note.session && view.liveness == Liveness::Live);
        let operator_owes = (sender_live
            || now_ms.saturating_sub(event_ms(note)) < ORPHANED_OPERATOR_NOTE_MAX_AGE_MS)
            && !acked.is_some_and(|sessions| sessions.contains(operator.session.as_str()))
            && note.session != operator.session
            && match note.to.as_ref() {
                // Agents address the person at the console as `harness:human`
                // — that is the wire convention, and the only address they
                // can know. The operator card's synthetic session id is a
                // second door for notes the console itself stamped.
                Some(to) => {
                    (to.kind == RecipientKind::Session && to.value == operator.session)
                        || (to.kind == RecipientKind::Harness && to.value == HARNESS_HUMAN)
                }
                None => note.kind == NoteKind::Ask,
            };

        // Nobody live is left to answer, so the obligation is over (§1.4). The
        // operator is always reachable through the console, so their presence
        // is live by construction.
        if !operator_owes
            && !owed
                .iter()
                .any(|view| matches!(view.liveness, Liveness::Live))
        {
            continue;
        }
        // A handoff names exactly one session: the live one it is waiting on.
        // An operator-addressed handoff names the operator, and nobody else.
        // A handoff with no target is a shape the CLI has refused since
        // alpha.1; the ones still in the log behave as an ask, because picking
        // one of seven readers at random would hide six real debts.
        let owed = match note.kind {
            NoteKind::Handoff if operator_owes => Vec::new(),
            NoteKind::Handoff if note.to.is_none() => owed,
            NoteKind::Handoff => owed
                .into_iter()
                .find(|view| matches!(view.liveness, Liveness::Live))
                .into_iter()
                .collect::<Vec<_>>(),
            _ => owed,
        };
        if owed.is_empty() && !operator_owes {
            continue;
        }
        let (from_name, _) = display_name(aliases, &note.session);
        let owed_total = owed.len() as u64 + u64::from(operator_owes);
        out.push(PendingItem {
            note: note.id.clone(),
            kind: note.kind.as_str().to_string(),
            from: note.session.clone(),
            from_name,
            asked_ms: event_ms(note),
            summary: report::summary_of(&note.text)
                .unwrap_or_else(|| note.text.trim())
                .to_string(),
            owed_by: owed
                .iter()
                .map(|view| {
                    let (display, _) = display_name(aliases, &view.session);
                    OwedBy {
                        session: view.session.clone(),
                        display_name: display,
                        presence: Presence::from_liveness(view.liveness),
                    }
                })
                .chain(operator_owes.then(|| OwedBy {
                    session: operator.session.clone(),
                    display_name: operator.display_name.clone(),
                    presence: Presence::Live,
                }))
                .collect(),
            owed_total,
            acked_by: acked
                .into_iter()
                .flatten()
                .map(|id| (*id).to_string())
                .collect(),
            place: place_of(note),
        });
    }
    out.sort_by(|a, b| b.asked_ms.cmp(&a.asked_ms).then(a.note.cmp(&b.note)));
    out
}

/// Cap on `owed_by` entries once a frame has to shed (§6, second step).
const OWED_BY_SHOWN: usize = 8;

/// Shed rows until the frame fits its budget (§6).
///
/// Measured on the whole body, not on `sessions` alone: a busy board's weight
/// is in its obligations, not its rows. Fifty open asks reaching seventy
/// readers serialize to ~377 KB of `owed_by` while `sessions` is under 50 KB,
/// so budgeting `sessions` and reserving a flat 8 KB for everything else let
/// that frame sail six times past the ceiling it was supposed to respect.
///
/// Order of giving way, cheapest loss first:
///   1. ended sessions, oldest signal first — already recoverable from `/feed`;
///   2. `owed_by` past [`OWED_BY_SHOWN`] per item, live readers kept first —
///      `owed_total` still says how many there really are;
///   3. stale sessions, oldest signal first.
///
/// `pending` is never shed. An obligation the operator cannot see is a debt
/// that goes unpaid, which costs more than a big frame does; the ceiling is
/// set high enough (128 KB) that a busy board fits with its debts intact.
///
/// Live rows are never shed: a client that cannot see a live agent has no
/// board at all. `counts` are computed before any of this, so nothing here
/// ever moves a number — a count that changed because a row was dropped for
/// size would be exactly the lie this projection exists to end.
fn trim_to_frame_budget(mut view: FleetView) -> FleetView {
    let size_of = |view: &FleetView| {
        serde_json::to_string(view)
            .map(|json| json.len())
            .unwrap_or(0)
    };
    if size_of(&view) <= MAX_FRAME_BYTES {
        return view;
    }

    // Step 1: ended rows.
    view = shed_sessions(view, Presence::Ended, &size_of);
    if size_of(&view) <= MAX_FRAME_BYTES {
        return view;
    }

    // Step 2: long owed_by lists, live readers first.
    for item in &mut view.pending {
        if item.owed_by.len() <= OWED_BY_SHOWN {
            continue;
        }
        item.owed_by
            .sort_by_key(|owed| u8::from(owed.presence != Presence::Live));
        item.owed_by.truncate(OWED_BY_SHOWN);
    }
    if size_of(&view) <= MAX_FRAME_BYTES {
        return view;
    }

    // Step 3: stale rows.
    shed_sessions(view, Presence::Stale, &size_of)
}

/// Drop as few `presence` rows as the budget allows, oldest signal first.
///
/// Binary search on how many to keep, so a big frame costs a handful of
/// serializations rather than one per row shed.
fn shed_sessions(
    mut view: FleetView,
    presence: Presence,
    size_of: &impl Fn(&FleetView) -> usize,
) -> FleetView {
    let mut shed_order: Vec<usize> = view
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, card)| card.presence == presence)
        .map(|(index, _)| index)
        .collect();
    if shed_order.is_empty() {
        return view;
    }
    shed_order.sort_by_key(|index| view.sessions[*index].last_ms);

    let all = std::mem::take(&mut view.sessions);
    let kept_with = |dropped: usize| -> Vec<SessionCard> {
        let drop: HashSet<usize> = shed_order.iter().take(dropped).copied().collect();
        all.iter()
            .enumerate()
            .filter(|(index, _)| !drop.contains(index))
            .map(|(_, card)| card.clone())
            .collect()
    };

    // Smallest number of dropped rows that fits; if even dropping all of them
    // does not, the frame is what it is and the remaining rows stand.
    let (mut lo, mut hi) = (0usize, shed_order.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        view.sessions = kept_with(mid);
        if size_of(&view) <= MAX_FRAME_BYTES {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    view.sessions = kept_with(lo);
    view
}

/// The operator's read cursors, from `operator.json`.
pub fn read_cursors(store: &Store) -> BTreeMap<String, String> {
    OperatorProfile::load(store).reads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Recipient;
    use crate::test_util::TempHome;
    use crate::{SCHEMA_VERSION, Store};

    const NOW: u64 = 1_000_000_000;

    /// An event whose ULID carries `ms`, so liveness sees the time we mean.
    fn ev(ms: u64, session: &str, kind: EventKind, repo: &str) -> Event {
        let id =
            ulid::Ulid::from_datetime(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
                .to_string();
        Event {
            v: SCHEMA_VERSION,
            id,
            ts: String::new(),
            event: kind,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "mac".to_string(),
            cwd: repo.to_string(),
            repo: Some(repo.to_string()),
            repo_key: None,
            branch: Some("main".to_string()),
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: String::new(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    fn child(mut event: Event, parent: &str) -> Event {
        event.role = Role::Herd;
        event.parent = Some(parent.to_string());
        event
    }

    fn note(mut event: Event, kind: NoteKind, text: &str, to: Option<Recipient>) -> Event {
        event.event = EventKind::Note;
        event.kind = kind;
        event.text = text.to_string();
        event.to = to;
        event
    }

    fn build_from(store: &Store, events: &[Event]) -> FleetView {
        build(&FleetInputs {
            store,
            events,
            now_ms: NOW,
            stale_secs: 900,
            leases: &[],
            hub_enabled: false,
            devices: 0,
        })
    }

    /// V2: a herd is one live row plus its delegated children, and a child that
    /// already said bye is never one of them.
    #[test]
    fn live_count_excludes_children_ending_rows_and_the_departed() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let events = vec![
            ev(NOW - 5_000, "parent", EventKind::Hello, tree),
            child(ev(NOW - 4_000, "kid-a", EventKind::Hello, tree), "parent"),
            child(ev(NOW - 3_000, "kid-b", EventKind::Hello, tree), "parent"),
            // kid-b says bye inside the herd grace window: still visible, never live.
            child(ev(NOW - 2_000, "kid-b", EventKind::Bye, tree), "parent"),
            // A session that left hours ago is stale, not live.
            ev(NOW - 3_600_000, "gone", EventKind::Hello, "/work/other"),
        ];

        let view = build_from(&store, &events);
        assert_eq!(
            view.counts.live, 1,
            "only the parent is a live top-level row"
        );
        assert_eq!(view.counts.delegated, 1, "kid-a; kid-b is ending");
        assert_eq!(view.counts.stale, 1, "the session that went quiet");

        let ending = view
            .sessions
            .iter()
            .find(|card| card.session == "kid-b")
            .expect("kid-b is on the board");
        assert!(
            ending.ending,
            "a farewell inside grace is marked, not hidden"
        );
        assert_eq!(ending.presence, Presence::Live);

        // The count is exactly the rows that satisfy the rule.
        let counted = view
            .sessions
            .iter()
            .filter(|card| card.presence == Presence::Live && !card.ending && card.depth == 0)
            .count() as u64;
        assert_eq!(view.counts.live, counted);
    }

    /// V2 (order): children follow their parent, live before stale.
    #[test]
    fn children_are_listed_directly_under_their_parent() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let events = vec![
            ev(NOW - 9_000, "parent", EventKind::Hello, tree),
            child(ev(NOW - 1_000, "kid-a", EventKind::Hello, tree), "parent"),
            ev(NOW - 2_000, "solo", EventKind::Hello, "/work/other"),
        ];
        let view = build_from(&store, &events);
        let order: Vec<&str> = view
            .sessions
            .iter()
            .map(|card| card.session.as_str())
            .collect();
        let parent_at = order.iter().position(|s| *s == "parent").unwrap();
        assert_eq!(order[parent_at + 1], "kid-a");
    }

    /// V3: no row ever renders as a raw id, including an unnamed host-pid one.
    #[test]
    fn every_row_has_a_display_name_that_is_not_its_id() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut ghost = ev(NOW - 1_000, "mac-4821", EventKind::Hello, "/work/harbard");
        ghost.harness = "other".to_string();
        let events = vec![
            ev(
                NOW - 2_000,
                "d77299cc-d35e-4f21-9a0b-4a1f2c3d4e5f",
                EventKind::Hello,
                "/work/harbard",
            ),
            ghost.clone(),
            // A second event keeps the host-pid row from being pruned as noise.
            ghost,
        ];
        let view = build_from(&store, &events);
        assert!(!view.sessions.is_empty());
        for card in &view.sessions {
            assert_ne!(card.display_name, card.session, "a uuid is not a name");
            assert!(!card.display_name.is_empty());
            assert!(!card.self_named, "nothing here claimed a callsign");
            assert!(!card.epithet.is_empty());
        }
    }

    /// V4: parent + child are one writer domain; two mains in one tree are two.
    #[test]
    fn contested_matches_writer_domains_not_head_count() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";

        let herd = vec![
            ev(NOW - 3_000, "parent", EventKind::Hello, tree),
            child(ev(NOW - 2_000, "kid", EventKind::Hello, tree), "parent"),
        ];
        let view = build_from(&store, &herd);
        let card = view
            .checkouts
            .iter()
            .find(|card| card.id == tree)
            .expect("the tree is a card");
        assert!(!card.contested, "one herd is one writer domain");
        assert!(card.rivals.is_empty());
        assert_eq!(card.live, 1);
        assert_eq!(card.delegated, 1);

        let rivals = vec![
            ev(NOW - 3_000, "one", EventKind::Hello, tree),
            ev(NOW - 2_000, "two", EventKind::Hello, tree),
        ];
        let view = build_from(&store, &rivals);
        let card = view
            .checkouts
            .iter()
            .find(|card| card.id == tree)
            .expect("the tree is a card");
        assert!(card.contested, "two mains in one tree contend");
        assert_eq!(card.rivals.len(), 2);
        assert_eq!(view.counts.contested, 1);
    }

    /// V4 / E2: sibling worktrees share a repository and never contend.
    #[test]
    fn sibling_worktrees_share_a_repo_key_and_do_not_contend() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let repo = "/work/harbard";
        let mut primary = ev(NOW - 3_000, "one", EventKind::Hello, repo);
        primary.repo_key = Some(repo.to_string());
        let mut linked = ev(
            NOW - 2_000,
            "two",
            EventKind::Hello,
            "/work/harbard/wt/fleet",
        );
        linked.repo_key = Some(repo.to_string());

        let view = build_from(&store, &[primary, linked]);
        assert_eq!(view.checkouts.len(), 2);
        for card in &view.checkouts {
            assert!(!card.contested, "different trees are different desks");
            assert_eq!(card.repo_key, repo);
            assert_eq!(card.label, "harbard");
        }
        let linked = view
            .checkouts
            .iter()
            .find(|card| card.id.ends_with("/fleet"))
            .unwrap();
        assert_eq!(linked.worktree.as_deref(), Some("fleet"));
        let primary = view.checkouts.iter().find(|card| card.id == repo).unwrap();
        assert_eq!(
            primary.worktree, None,
            "the primary checkout is not a worktree"
        );
        assert_eq!(view.counts.contested, 0);
    }

    /// E1: a session outside any checkout is a row, never a card.
    #[test]
    fn a_session_outside_a_checkout_creates_no_card() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut loose = ev(NOW - 1_000, "drifter", EventKind::Hello, "/tmp");
        loose.repo = None;
        loose.repo_key = None;
        let view = build_from(&store, &[loose]);
        assert_eq!(view.sessions.len(), 1);
        assert_eq!(view.sessions[0].checkout, None);
        assert!(view.checkouts.is_empty());
    }

    /// V5: a handoff owes exactly one session, and an ack closes it.
    #[test]
    fn a_handoff_owes_one_session_and_an_ack_closes_it() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let handoff = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Handoff,
            "take the rudder\nrest of the body",
            Some(Recipient {
                kind: RecipientKind::Session,
                value: "receiver".to_string(),
            }),
        );
        let events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            ev(NOW - 5_000, "receiver", EventKind::Hello, tree),
            handoff.clone(),
        ];

        let view = build_from(&store, &events);
        assert_eq!(view.pending.len(), 1);
        let item = &view.pending[0];
        assert_eq!(item.kind, "handoff");
        assert_eq!(item.owed_by.len(), 1, "a handoff hands work to exactly one");
        assert_eq!(item.owed_by[0].session, "receiver");
        assert_eq!(item.summary, "take the rudder");
        assert_eq!(item.place, session_place("receiver"));
        assert_eq!(view.counts.pending, 1);

        for (text, reference, actor, closes) in [
            (handoff.id.clone(), None, "receiver", true),
            (format!("{} — taken on", handoff.id), None, "receiver", true),
            (format!("{} taken on", handoff.id), None, "receiver", true),
            (
                "taken on".to_string(),
                Some(handoff.id.clone()),
                "receiver",
                true,
            ),
            (
                format!("{}extra — taken on", handoff.id),
                None,
                "receiver",
                false,
            ),
            (format!("{}— taken on", handoff.id), None, "receiver", false),
            (format!("{} — taken on", handoff.id), None, "other", false),
        ] {
            let mut ack = ev(NOW - 3_000, actor, EventKind::Ack, tree);
            ack.text = text.clone();
            ack.note_ref = reference;
            let mut acked = events.clone();
            acked.push(ack);
            let view = build_from(&store, &acked);
            assert_eq!(view.pending.is_empty(), closes, "ACK {text:?} from {actor}");
        }
    }

    /// V5 / E7: an ask is owed by every live reader it reached, and drops when
    /// the last of them goes quiet.
    #[test]
    fn an_ask_owes_every_live_reader_and_expires_with_them() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        // Addressed to the harness, so this case is about agent readers alone;
        // a broadcast ask also reaches the operator (E8, covered separately).
        let ask = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "status?",
            Some(Recipient {
                kind: RecipientKind::Harness,
                value: "codex".to_string(),
            }),
        );
        let mut events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            ev(NOW - 5_000, "one", EventKind::Hello, tree),
            ev(NOW - 5_000, "two", EventKind::Hello, tree),
            ask.clone(),
        ];

        let view = build_from(&store, &events);
        assert_eq!(view.pending.len(), 1);
        assert_eq!(
            view.pending[0].owed_by.len(),
            2,
            "both live readers owe one"
        );
        assert_eq!(view.pending[0].kind, "ask");

        // One answers with a reply that names the ask: causal closure.
        let mut reply = note(
            ev(NOW - 3_000, "one", EventKind::Note, tree),
            NoteKind::Fyi,
            "done",
            None,
        );
        reply.note_ref = Some(ask.id.clone());
        events.push(reply);
        let view = build_from(&store, &events);
        assert_eq!(view.pending[0].owed_by.len(), 1);
        assert_eq!(view.pending[0].owed_by[0].session, "two");
        assert_eq!(view.pending[0].acked_by, vec!["one".to_string()]);

        // The other never answers and goes quiet: nobody is left to answer.
        let stale = build(&FleetInputs {
            store: &store,
            events: &events,
            now_ms: NOW + 3_600_000,
            stale_secs: 900,
            leases: &[],
            hub_enabled: false,
            devices: 0,
        });
        assert!(
            stale.pending.is_empty(),
            "an obligation nobody live can answer is not pending"
        );
    }

    /// An `fyi` and a `warning` are told, not asked.
    #[test]
    fn only_handoff_and_ask_create_obligations() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            ev(NOW - 5_000, "reader", EventKind::Hello, tree),
            note(
                ev(NOW - 4_000, "author", EventKind::Note, tree),
                NoteKind::Fyi,
                "context",
                None,
            ),
            note(
                ev(NOW - 3_000, "author", EventKind::Note, tree),
                NoteKind::Warning,
                "careful",
                None,
            ),
        ];
        let view = build_from(&store, &events);
        assert!(view.pending.is_empty());
        assert_eq!(view.counts.pending, 0);
    }

    /// The projection carries its own anchor and clock so no client needs one.
    #[test]
    fn the_view_carries_its_anchor_and_ages() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let events = vec![ev(NOW - 7_000, "one", EventKind::Hello, "/work/harbard")];
        let view = build_from(&store, &events);
        assert_eq!(view.v, FLEET_VERSION);
        assert_eq!(view.as_of.as_deref(), Some(events[0].id.as_str()));
        assert_eq!(view.now_ms, NOW);
        assert_eq!(view.stale_secs, 900);
        assert_eq!(view.sessions[0].signal_age_ms, 7_000);
        assert_eq!(view.operator.display_name, "Operator");
    }

    /// E4: a lease outlives its holder — the row is not live, the lease shows.
    #[test]
    fn a_lease_held_by_a_departed_session_still_shows() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let lease = WriterLeaseView {
            host: "mac".to_string(),
            worktree: tree.to_string(),
            git_common_dir: format!("{tree}/.git"),
            repo: "harbard".to_string(),
            session: "gone".to_string(),
            harness: "codex".to_string(),
            branch: Some("main".to_string()),
            claimed_at_ms: NOW - 10_000,
            renewed_at_ms: NOW - 9_000,
            expires_at_ms: NOW + 60_000,
        };
        let events = vec![
            ev(NOW - 20_000, "gone", EventKind::Hello, tree),
            ev(NOW - 19_000, "gone", EventKind::Bye, tree),
        ];
        let view = build(&FleetInputs {
            store: &store,
            events: &events,
            now_ms: NOW,
            stale_secs: 900,
            leases: std::slice::from_ref(&lease),
            hub_enabled: false,
            devices: 0,
        });
        let card = view
            .checkouts
            .iter()
            .find(|card| card.id == tree)
            .expect("the lease keeps the berth on the board");
        assert_eq!(
            card.lease.as_ref().map(|l| l.session.as_str()),
            Some("gone")
        );
        let row = view
            .sessions
            .iter()
            .find(|card| card.session == "gone")
            .expect("the departed row is still recent");
        assert_eq!(row.presence, Presence::Ended);
        assert!(row.holds_lease);
        assert_eq!(view.counts.live, 0);
    }

    /// Ended rows age off the board; History keeps them.
    #[test]
    fn ancient_ended_rows_leave_the_board() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let old = NOW - ENDED_VISIBLE_MS - 60_000;
        let events = vec![
            ev(old, "ancient", EventKind::Hello, tree),
            ev(old + 1_000, "ancient", EventKind::Bye, tree),
        ];
        let view = build_from(&store, &events);
        assert!(view.sessions.is_empty(), "six hours out is History's job");
        assert!(view.checkouts.is_empty(), "and its berth is gone with it");
    }

    /// §6: an oversized frame sheds ended rows, never live ones, and the
    /// counts stay true because they were taken before the trim.
    #[test]
    fn an_oversized_frame_sheds_ended_rows_first() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let mut events = Vec::new();
        // Enough departed sessions to blow the budget, plus a few that matter.
        for i in 0..400u64 {
            let session = format!("ended-session-with-a-realistically-long-id-{i:04}");
            events.push(ev(NOW - 100_000 - i, &session, EventKind::Hello, tree));
            events.push(ev(NOW - 90_000 - i, &session, EventKind::Bye, tree));
        }
        for i in 0..3u64 {
            events.push(ev(
                NOW - 5_000 - i,
                &format!("live-{i}"),
                EventKind::Hello,
                tree,
            ));
        }

        let view = build_from(&store, &events);
        let bytes = serde_json::to_string(&view).expect("the frame serializes");
        assert!(
            bytes.len() <= MAX_FRAME_BYTES,
            "the frame is {} bytes, over the {MAX_FRAME_BYTES} budget",
            bytes.len()
        );

        // Every live row survived; only ended ones were shed.
        let live_rows = view
            .sessions
            .iter()
            .filter(|card| card.presence == Presence::Live)
            .count();
        assert_eq!(live_rows, 3, "a live agent is never dropped for size");
        assert!(
            view.sessions.len() < 403,
            "ended rows were actually shed: {}",
            view.sessions.len()
        );

        // And the counts describe the whole board, not the trimmed frame.
        assert_eq!(view.counts.live, 3);
    }

    /// §1.3: rooms carry their own unread, and archived ones are not cards.
    #[test]
    fn rooms_report_membership_and_unread() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut memberships = Memberships::default();
        membership::join(
            &mut memberships,
            "release-room",
            Member {
                kind: crate::membership::AnchorKind::Session,
                value: "one".to_string(),
            },
        );
        membership::create(&mut memberships, "attic");
        memberships.archived.insert("attic".to_string());
        membership::write(&store, &memberships).unwrap();

        let tree = "/work/harbard";
        let to_room = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Fyi,
            "cutting the release",
            Some(Recipient {
                kind: RecipientKind::Channel,
                value: "release-room".to_string(),
            }),
        );
        let events = vec![
            ev(NOW - 5_000, "author", EventKind::Hello, tree),
            to_room.clone(),
        ];
        let view = build_from(&store, &events);
        assert_eq!(view.rooms.len(), 1, "archived rooms are not on the board");
        let room = &view.rooms[0];
        assert_eq!(room.name, "release-room");
        assert_eq!(room.members.len(), 1);
        assert_eq!(room.latest_item.as_deref(), Some(to_room.id.as_str()));
        assert_eq!(room.unread, 1, "unvisited room counts what is there");

        // Reading up to the note clears it.
        let mut profile = OperatorProfile::load(&store);
        profile
            .reads
            .insert(room_place("release-room"), to_room.id.clone());
        profile.save(&store).unwrap();
        let view = build_from(&store, &events);
        assert_eq!(view.rooms[0].unread, 0);
        assert_eq!(
            view.reads.get(&room_place("release-room")),
            Some(&to_room.id)
        );
    }

    /// §1.2: a fresh delivery is reason enough for a card. The tree has no
    /// occupant and no lease — before this, the receipt was only consulted
    /// after a candidate existed, so the card could never be built.
    #[test]
    fn a_recent_delivery_alone_puts_a_checkout_on_the_board() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let quiet = "/work/delivered";
        sluice::record(
            home.path(),
            &sluice::Receipt {
                delivery_id: "d1".to_string(),
                repo: quiet.to_string(),
                branch: "feat/x".to_string(),
                delivered_sha: "aaaaaaa".to_string(),
                onto: "bbbbbbb".to_string(),
                candidate: Some("ccccccc".to_string()),
                verdict: sluice::Verdict::Landed,
                detail: String::new(),
                session: "gone".to_string(),
                decided_ms: NOW - 60_000,
            },
        )
        .expect("receipt written");

        // Somebody live elsewhere, so the board is not empty for another reason.
        let events = vec![ev(
            NOW - 1_000,
            "other",
            EventKind::Hello,
            "/work/elsewhere",
        )];
        let view = build_from(&store, &events);

        let card = view
            .checkouts
            .iter()
            .find(|card| card.id == quiet)
            .expect("the delivered tree is a card");
        assert_eq!(card.live, 0, "nobody is in it");
        assert!(card.occupants.is_empty());
        let stamp = card.last_delivery.as_ref().expect("the receipt is stamped");
        assert_eq!(stamp.sha, "ccccccc", "the candidate that landed");
        assert_eq!(stamp.verdict, "landed");
    }

    /// The same tree, once the delivery is older than the window, is History's.
    #[test]
    fn a_stale_delivery_does_not_keep_a_checkout_on_the_board() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let quiet = "/work/delivered-long-ago";
        sluice::record(
            home.path(),
            &sluice::Receipt {
                delivery_id: "d1".to_string(),
                repo: quiet.to_string(),
                branch: "feat/x".to_string(),
                delivered_sha: "aaaaaaa".to_string(),
                onto: "bbbbbbb".to_string(),
                candidate: Some("ccccccc".to_string()),
                verdict: sluice::Verdict::Landed,
                detail: String::new(),
                session: "gone".to_string(),
                decided_ms: NOW - DELIVERY_VISIBLE_MS - 1,
            },
        )
        .expect("receipt written");

        let events = vec![ev(
            NOW - 1_000,
            "other",
            EventKind::Hello,
            "/work/elsewhere",
        )];
        let view = build_from(&store, &events);
        assert!(
            !view.checkouts.iter().any(|card| card.id == quiet),
            "a day-old delivery belongs to History"
        );
    }

    /// V6: a note addressed to `repo:<primary>` reaches every sibling worktree,
    /// so the linked tree's card points at it. Reading the tree's own path as
    /// its repo_key hid the note from `latest_item` while `unread` counted it.
    #[test]
    fn a_note_to_the_repo_is_the_latest_item_of_a_linked_worktree() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let repo = "/work/harbard";
        let linked_tree = "/work/harbard/wt/fleet";
        let mut linked = ev(NOW - 3_000, "two", EventKind::Hello, linked_tree);
        linked.repo_key = Some(repo.to_string());

        // Written from somewhere else, addressed to the repository.
        let to_repo = note(
            ev(NOW - 1_000, "elsewhere", EventKind::Note, "/work/other"),
            NoteKind::Fyi,
            "the whole repo should see this",
            Some(Recipient {
                kind: RecipientKind::Repo,
                value: repo.to_string(),
            }),
        );
        let to_repo_id = to_repo.id.clone();

        let view = build_from(&store, &[linked, to_repo]);
        let card = view
            .checkouts
            .iter()
            .find(|card| card.id == linked_tree)
            .expect("the linked worktree is a card");
        assert_eq!(
            card.latest_item.as_deref(),
            Some(to_repo_id.as_str()),
            "a note to the repository is the tree's newest item"
        );
        assert!(card.unread >= 1, "and it is unread");
    }

    #[test]
    fn pending_debts_use_first_own_file_order_not_same_ms_ulid_order() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let ms = NOW - 1_000;
        let mut first = ev(ms, "earlier", EventKind::Note, tree);
        first.id = ulid::Ulid::from_parts(ms, 90).to_string();
        let mut old = note(
            ev(ms, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "old ask",
            None,
        );
        old.id = ulid::Ulid::from_parts(ms, 80).to_string();
        let mut joined = ev(ms, "new-reader", EventKind::Hello, tree);
        joined.id = ulid::Ulid::from_parts(ms, 70).to_string();
        let mut repeat = ev(ms, "earlier", EventKind::Hello, tree);
        repeat.id = ulid::Ulid::from_parts(ms, 60).to_string();
        let mut fresh = note(
            ev(ms, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "fresh ask",
            None,
        );
        fresh.id = ulid::Ulid::from_parts(ms, 1).to_string();
        let events = vec![first, old.clone(), joined, repeat, fresh.clone()];
        let view = build_from(&store, &events);
        let owed_on = |id: &str| {
            view.pending
                .iter()
                .find(|item| item.note == id)
                .unwrap()
                .owed_by
                .iter()
                .map(|entry| entry.session.as_str())
                .collect::<Vec<_>>()
        };
        let operator = format!("operator@{}", crate::hostname());
        assert_eq!(view.pending.len(), 2);
        assert!(owed_on(&old.id).contains(&"earlier"));
        assert!(!owed_on(&old.id).contains(&"new-reader"));
        assert!(
            owed_on(&old.id).contains(&operator.as_str()),
            "operator has no join prerequisite"
        );
        assert!(owed_on(&fresh.id).contains(&"earlier"));
        assert!(owed_on(&fresh.id).contains(&"new-reader"));
        for (session, old_eligible) in [("earlier", true), ("new-reader", false)] {
            let mut ledger = crate::wake::Ledger::new(session);
            for event in &events {
                ledger.observe(event);
            }
            assert_eq!(ledger.actionable(&old, NOW), old_eligible);
            assert!(ledger.actionable(&fresh, NOW));
        }
    }

    #[test]
    fn explicit_quiet_update_removes_entire_pending_thread_including_operator_debt() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let root = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "please act",
            None,
        );
        let mut sibling = note(
            ev(NOW - 3_000, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "updated request",
            None,
        );
        sibling.note_ref = Some(root.id.clone());
        let mut events = vec![
            ev(NOW - 5_000, "reader", EventKind::Hello, tree),
            root.clone(),
            sibling.clone(),
        ];
        let view = build_from(&store, &events);
        assert_eq!(view.pending.len(), 1);
        assert_eq!(view.pending[0].note, sibling.id);
        let mut quiet = note(
            ev(NOW - 2_000, "author", EventKind::Note, tree),
            NoteKind::Fyi,
            "no longer needed",
            None,
        );
        quiet.note_ref = Some(root.id.clone());
        events.push(quiet);
        assert!(build_from(&store, &events).pending.is_empty());
    }

    #[test]
    fn quiet_updates_do_not_cancel_debt_without_same_sender_recipient_and_reference() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "reader".to_string(),
        });
        let root = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Handoff,
            "please act",
            to.clone(),
        );
        for (sender, recipient, parent) in [
            ("other", to.clone(), Some(root.id.clone())),
            ("author", None, Some(root.id.clone())),
            ("author", to.clone(), None),
        ] {
            let mut quiet = note(
                ev(NOW - 2_000, sender, EventKind::Note, tree),
                NoteKind::Fyi,
                "no longer needed",
                recipient,
            );
            quiet.note_ref = parent;
            let events = [
                ev(NOW - 5_000, "reader", EventKind::Hello, tree),
                root.clone(),
                quiet,
            ];
            let view = build_from(&store, &events);
            assert_eq!(view.pending.len(), 1);
            assert_eq!(view.pending[0].note, root.id);
        }
    }

    #[test]
    fn ended_sender_expiry_matches_ledger_at_four_hours_and_reopens_on_hello() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let expiry = crate::wake::ENDED_NOTE_MAX_AGE_MS;
        let to = Some(Recipient {
            kind: RecipientKind::Harness,
            value: "codex".to_string(),
        });
        let old = note(
            ev(NOW - expiry, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "expired",
            to.clone(),
        );
        let fresh = note(
            ev(NOW - expiry + 1, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "still current",
            to.clone(),
        );
        let live_old = note(
            ev(NOW - expiry, "live-author", EventKind::Note, tree),
            NoteKind::Ask,
            "long live session",
            to,
        );
        let mut events = vec![
            ev(NOW - expiry - 1, "reader", EventKind::Hello, tree),
            old.clone(),
            fresh.clone(),
            live_old.clone(),
            ev(NOW - 2_000, "author", EventKind::Bye, tree),
            ev(NOW - 1_000, "reader", EventKind::Intent, tree),
        ];
        let view = build_from(&store, &events);
        assert_eq!(view.pending.len(), 2);
        assert!(!view.pending.iter().any(|item| item.note == old.id));
        assert!(view.pending.iter().any(|item| item.note == fresh.id));
        assert!(view.pending.iter().any(|item| item.note == live_old.id));
        let mut ledger = crate::wake::Ledger::new("reader");
        for event in &events {
            ledger.observe(event);
        }
        assert!(!ledger.actionable(&old, NOW));
        assert!(ledger.actionable(&fresh, NOW));
        assert!(ledger.actionable(&live_old, NOW));
        events.push(ev(NOW - 500, "author", EventKind::Hello, tree));
        assert_eq!(build_from(&store, &events).pending.len(), 3);
    }

    #[test]
    fn ended_sender_expiry_does_not_erase_required_operator_answer() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let expiry = crate::wake::ENDED_NOTE_MAX_AGE_MS;
        let broadcast = note(
            ev(NOW - expiry, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "operator still needed",
            None,
        );
        let human = note(
            ev(NOW - expiry - 1, "author", EventKind::Note, tree),
            NoteKind::Handoff,
            "human decision",
            Some(Recipient {
                kind: RecipientKind::Harness,
                value: HARNESS_HUMAN.to_string(),
            }),
        );
        let events = [
            ev(NOW - expiry - 2, "reader", EventKind::Hello, tree),
            human,
            broadcast,
            ev(NOW - 2_000, "author", EventKind::Bye, tree),
            ev(NOW - 1_000, "reader", EventKind::Intent, tree),
        ];
        let view = build_from(&store, &events);
        let operator = format!("operator@{}", crate::hostname());
        assert_eq!(view.pending.len(), 2);
        for item in &view.pending {
            assert_eq!(item.owed_total, 1);
            assert_eq!(item.owed_by.len(), 1);
            assert_eq!(item.owed_by[0].session, operator);
        }
    }

    /// An operator debt from a sender that is no longer live ends a week after
    /// it was asked. A crashed session never says `bye`, so its handoff to the
    /// human used to stay in "needs you" for months.
    #[test]
    fn an_orphaned_operator_handoff_expires_after_a_week() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let max_age = ORPHANED_OPERATOR_NOTE_MAX_AGE_MS;
        let to_human = || {
            Some(Recipient {
                kind: RecipientKind::Harness,
                value: HARNESS_HUMAN.to_string(),
            })
        };
        let old = note(
            ev(NOW - max_age - 1_000, "gone", EventKind::Note, tree),
            NoteKind::Handoff,
            "old decision",
            to_human(),
        );
        let recent = note(
            ev(NOW - max_age + 60_000, "quiet", EventKind::Note, tree),
            NoteKind::Handoff,
            "recent decision",
            to_human(),
        );
        let events = [
            ev(NOW - max_age - 2_000, "gone", EventKind::Hello, tree),
            old,
            ev(NOW - max_age + 50_000, "quiet", EventKind::Hello, tree),
            recent.clone(),
        ];
        let view = build_from(&store, &events);
        let ids: Vec<&str> = view.pending.iter().map(|item| item.note.as_str()).collect();
        assert_eq!(
            ids,
            vec![recent.id.as_str()],
            "only the note inside the week is owed"
        );
    }

    /// E7: a reader who went quiet without answering is still listed, with
    /// `presence: "stale"`, while somebody live can still answer. Filtering
    /// owed_by down to live readers made that presence unreachable and hid
    /// half the answer to "who owes this".
    #[test]
    fn a_stale_reader_stays_owed_while_someone_live_can_answer() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        // To the harness, not a broadcast: E8 gives a broadcast ask an operator
        // recipient, and this case is about agent presence.
        let ask = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "status?",
            Some(Recipient {
                kind: RecipientKind::Harness,
                value: "codex".to_string(),
            }),
        );
        let mut events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            // A is live; B last spoke over an hour ago, so it is stale.
            ev(NOW - 5_000, "live-a", EventKind::Hello, tree),
            ev(NOW - 3_600_000, "stale-b", EventKind::Hello, tree),
            ask.clone(),
        ];

        let view = build_from(&store, &events);
        assert_eq!(view.pending.len(), 1, "somebody live can still answer");
        let owed: Vec<(&str, Presence)> = view.pending[0]
            .owed_by
            .iter()
            .map(|entry| (entry.session.as_str(), entry.presence))
            .collect();
        assert!(
            owed.contains(&("live-a", Presence::Live)),
            "the live reader owes an answer: {owed:?}"
        );
        assert!(
            owed.contains(&("stale-b", Presence::Stale)),
            "the quiet reader is still owed, marked stale: {owed:?}"
        );

        // A answers. Nobody live is left, so the obligation is over — even
        // though B never said anything.
        let mut reply = note(
            ev(NOW - 3_000, "live-a", EventKind::Note, tree),
            NoteKind::Fyi,
            "done",
            None,
        );
        reply.note_ref = Some(ask.id.clone());
        events.push(reply);
        let view = build_from(&store, &events);
        assert!(
            view.pending.is_empty(),
            "a stale reader alone does not keep an item alive: {:?}",
            view.pending
        );
    }

    /// E8 by the address agents actually use: `harness:human` is how a session
    /// names the person at the console, since the operator card's session id is
    /// minted by serve and no agent can know it.
    #[test]
    fn a_note_to_harness_human_names_the_operator() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let operator = format!("operator@{}", crate::hostname());
        let handoff = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Handoff,
            "nobody merges to main until the audit lands",
            Some(Recipient {
                kind: RecipientKind::Harness,
                value: HARNESS_HUMAN.to_string(),
            }),
        );
        let events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            handoff.clone(),
        ];
        let view = build_from(&store, &events);
        let item = view
            .pending
            .iter()
            .find(|item| item.note == handoff.id)
            .expect("a handoff to harness:human is an obligation");
        assert_eq!(
            item.owed_by
                .iter()
                .map(|o| o.session.as_str())
                .collect::<Vec<_>>(),
            vec![operator.as_str()],
            "the operator, and only the operator, owes it"
        );
    }

    /// E8: the operator is a recipient. A handoff addressed to the operator's
    /// own session, and a broadcast ask, both owe the operator an answer — and
    /// an operator ack closes them. recipients_of walks agent sessions only, so
    /// before this an interrupt aimed at the console named nobody.
    #[test]
    fn the_operator_owes_an_answer_and_can_close_it() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let operator = format!("operator@{}", crate::hostname());

        let handoff = note(
            ev(NOW - 4_000, "author", EventKind::Note, tree),
            NoteKind::Handoff,
            "over to you",
            Some(Recipient {
                kind: RecipientKind::Session,
                value: operator.clone(),
            }),
        );
        let broadcast = note(
            ev(NOW - 3_500, "author", EventKind::Note, tree),
            NoteKind::Ask,
            "anyone?",
            None,
        );
        let mut events = vec![
            ev(NOW - 6_000, "author", EventKind::Hello, tree),
            handoff.clone(),
            broadcast.clone(),
        ];

        let view = build_from(&store, &events);
        let owed_on = |view: &FleetView, id: &str| -> Vec<String> {
            view.pending
                .iter()
                .find(|item| item.note == id)
                .map(|item| {
                    item.owed_by
                        .iter()
                        .map(|entry| entry.session.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            owed_on(&view, &handoff.id),
            vec![operator.clone()],
            "a handoff to the operator names the operator, and only them"
        );
        assert!(
            owed_on(&view, &broadcast.id).contains(&operator),
            "a broadcast ask reaches whoever is at the console"
        );

        // The operator answers both: POST /notes with a ref, authored by them.
        for target in [&handoff, &broadcast] {
            let mut ack = note(
                ev(NOW - 2_000, &operator, EventKind::Note, tree),
                NoteKind::Fyi,
                "seen",
                None,
            );
            ack.note_ref = Some(target.id.clone());
            events.push(ack);
        }
        let view = build_from(&store, &events);
        assert!(
            view.pending.is_empty(),
            "an operator ack closes what the operator owed: {:?}",
            view.pending
        );
    }

    /// One definition of "counts as live", so the header and the berth beneath
    /// it can never disagree — which is the drift this projection exists to end.
    #[test]
    fn the_counts_and_the_cards_agree_on_who_is_live() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let tree = "/work/harbard";
        let events = vec![
            ev(NOW - 5_000, "parent", EventKind::Hello, tree),
            child(ev(NOW - 4_000, "kid-a", EventKind::Hello, tree), "parent"),
            child(ev(NOW - 3_000, "kid-b", EventKind::Hello, tree), "parent"),
            // Said bye inside grace: visible, never live.
            child(ev(NOW - 2_000, "kid-b", EventKind::Bye, tree), "parent"),
            // Quiet for an hour: stale, never live.
            ev(NOW - 3_600_000, "quiet", EventKind::Hello, tree),
            // Another tree, so the totals are a sum and not one card.
            ev(NOW - 4_500, "solo", EventKind::Hello, "/work/other"),
        ];

        let view = build_from(&store, &events);
        let from_cards: u64 = view.checkouts.iter().map(|card| card.live).sum();
        let from_counts = view.counts.live;
        assert_eq!(
            from_counts, from_cards,
            "the header and the berths counted different people: {:?}",
            view.checkouts
        );
        let delegated_from_cards: u64 = view.checkouts.iter().map(|card| card.delegated).sum();
        assert_eq!(view.counts.delegated, delegated_from_cards);
        assert_eq!(from_counts, 2, "parent and solo");
        assert_eq!(delegated_from_cards, 1, "kid-a; kid-b said bye");
    }

    /// §6 on a board whose weight is NOT in `sessions`: twenty leased trees and
    /// fifty open obligations. Budgeting `sessions` alone and reserving a flat
    /// 8 KB for the rest let a frame like this sail past the ceiling.
    #[test]
    fn a_frame_heavy_outside_sessions_still_fits_the_budget() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut events = Vec::new();
        let mut leases = Vec::new();

        for tree_index in 0..20u64 {
            let tree = format!(
                "/Users/somebody/Projects/harbard/.claude/worktrees/long-worktree-name-{tree_index:02}"
            );
            let occupant = format!("occupant-with-a-realistic-uuid-shaped-id-{tree_index:02}");
            events.push(ev(
                NOW - 5_000 - tree_index,
                &occupant,
                EventKind::Hello,
                &tree,
            ));
            leases.push(WriterLeaseView {
                host: "Mac-mini-de-somebody.local".to_string(),
                worktree: tree.clone(),
                git_common_dir: "/Users/somebody/Projects/harbard/.git".to_string(),
                repo: "harbard".to_string(),
                session: occupant.clone(),
                harness: "claude-code".to_string(),
                branch: Some(format!(
                    "feat/a-branch-name-of-ordinary-length-{tree_index:02}"
                )),
                claimed_at_ms: NOW - 60_000,
                renewed_at_ms: NOW - 5_000,
                expires_at_ms: NOW + 600_000,
            });

            // Ended rows, so the trimmer has something it is allowed to shed.
            for ended in 0..20u64 {
                let gone = format!(
                    "ended-session-with-a-realistically-long-id-{tree_index:02}-{ended:02}"
                );
                events.push(ev(NOW - 100_000 - ended, &gone, EventKind::Hello, &tree));
                events.push(ev(NOW - 90_000 - ended, &gone, EventKind::Bye, &tree));
            }
        }

        // Fifty asks nobody has answered, each with a summary of real length,
        // from the occupants already on the board — the §6 sizing is twenty
        // checkouts, not twenty checkouts plus fifty more live sessions.
        for ask_index in 0..50u64 {
            let tree_index = ask_index % 20;
            let asker = format!("occupant-with-a-realistic-uuid-shaped-id-{tree_index:02}");
            let tree = format!(
                "/Users/somebody/Projects/harbard/.claude/worktrees/long-worktree-name-{tree_index:02}"
            );
            events.push(note(
                ev(NOW - 7_000 - ask_index, &asker, EventKind::Note, &tree),
                NoteKind::Ask,
                "does anybody know whether this branch is safe to rebase before the release cut",
                None,
            ));
        }

        let view = build(&FleetInputs {
            store: &store,
            events: &events,
            now_ms: NOW,
            stale_secs: 900,
            leases: &leases,
            hub_enabled: false,
            devices: 0,
        });
        let bytes = serde_json::to_string(&view).expect("the frame serializes");
        assert!(
            bytes.len() <= MAX_FRAME_BYTES,
            "the frame is {} bytes, over the {MAX_FRAME_BYTES} budget; sessions={} checkouts={} pending={}",
            bytes.len(),
            view.sessions.len(),
            view.checkouts.len(),
            view.pending.len()
        );
        assert!(
            view.sessions
                .iter()
                .any(|card| card.presence == Presence::Live),
            "a live agent is never dropped for size"
        );
    }
}
