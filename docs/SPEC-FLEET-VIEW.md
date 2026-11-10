# Fleet view: one projection, one feed

This document is the contract for the fleet projection (`harbard fleet`),
`GET /api/v1/feed`, the additive stream frames, and the operator read cursors.
It adds to [SPEC-API.md](SPEC-API.md); no route there changes shape.

The projection is a single fold of the store. Its readers take its counts and
names and do not re-derive liveness, contested checkouts, pending obligations,
delivery receipts or display names from raw events. The HTTP route that served
it and its stream frame are retired: the console reads `/api/v1/workspace`, and
nothing read them. The
constitution in `SPEC.md` applies: §2 presence is evidence-based, §4 bus content
is data, §6 the operator write door stays narrow. Operator read cursors are the
operator's own state and are never an agent's.

## 1. The projection: `FleetView`

`harbard fleet --json` → `FleetView`; `harbard fleet` prints the same board
for a person. Computed from the store's retained history (active sessions can
span days) plus the lease, alias, membership, operator and sluice stores.

```jsonc
{
  "v": 1,
  "now_ms": 0,
  "as_of": "<ULID of the newest event folded in>",
  "stale_secs": 900,
  "counts": {
    "live": 0,        // sessions with presence=live AND ending=false AND depth=0
    "delegated": 0,   // presence=live, ending=false, depth>0
    "stale": 0,       // presence=stale (no bye, signal older than stale_secs)
    "pending": 0,     // items in `pending`
    "contested": 0    // checkouts with contested=true
  },
  "sessions":  [SessionCard],
  "checkouts": [CheckoutCard],
  "rooms":     [RoomCard],
  "pending":   [PendingItem],
  "operator":  { "session": "operator", "display_name": "…" },
  "reads":     { "<place-id>": "<event ULID>" },       // operator read cursors, §4
  "hub":       { "enabled": false, "devices": 0 }
}
```

`counts` are THE numbers. No surface computes its own. A count that a screen
needs and this object lacks is a spec change, not a client `filter().length`.

### 1.1 `SessionCard`

```jsonc
{
  "session": "<full id>",
  "display_name": "Lantern",        // alias::resolve, or the self-chosen name; never a raw id
  "self_named": true,               // the session chose its own callsign
  "epithet": "…",                   // alias::epithet, for avatars
  "harness": "claude-code|codex|cursor|other",
  "host": "…",
  "role": "main|herd|headless",
  "cwd": "/abs",
  "checkout": "<checkout id, §1.2>",   // null when cwd is not inside a git checkout
  "branch": "main",
  "parent": null,  "root": "<id>",  "depth": 0,  "descendants": 0,
  "identity": "observed|inferred|unknown",
  "intent": "…",  "intent_ms": 0,
  "presence": "live|stale|ended",   // §2 of the constitution; `ended` = bye observed
  "ending": false,                  // said bye, still inside the herd grace window
  "holds_lease": false,             // active writer lease on `checkout`
  "first_ms": 0,  "last_ms": 0,
  "signal_age_ms": 0                // now_ms - last_ms, so clients never need a clock
}
```

Rules:
- `display_name` is resolved server-side for every row, including subagents
  (a pronounceable hash-derived name such as `Widom` is the fallback; a UUID
  is not).
- `presence=ended` IS on the wire here. Ended rows are included only while
  `now_ms - last_ms < 6h`; older ended rows are History's job (`/feed`).
- `ending=true` rows are never counted as live.
- Order: live first (depth 0 before children), then stale, then ended; ties by
  `last_ms` desc. Children immediately follow their parent.

### 1.2 `CheckoutCard`

A checkout is one worktree (canonical path). Its `id` is the canonical
worktree path. `repo_key` groups sibling worktrees of one repository
(`SessionView.repo_key.or(repo)`).

```jsonc
{
  "id": "/Users/x/Projects/harbard/.claude/worktrees/fleet-view",
  "repo_key": "/Users/x/Projects/harbard",
  "label": "harbard",                       // project name (basename of repo_key)
  "worktree": "fleet-view",                 // null for the primary checkout
  "branch": "feat/fleet-view",
  "occupants": ["<session id>", …],         // presence=live or stale, any depth
  "live": 2,  "delegated": 1,               // same definitions as counts
  "contested": false,                       // topology::checkout_contested
  "rivals": ["<session id>", …],            // distinct writer-domain roots when contested
  "lease": WriterLeaseView | null,          // active writer lease, if any
  "last_delivery": DeliveryStamp | null,    // from the sluice receipts, as checkout_history does
  "last_signal_ms": 0,
  "latest_item": "<event ULID>" | null,     // newest feed item for this place (for unread)
  "unread": 0                               // feed items newer than reads["checkout:<id>"], max 99
}
```

Rules:
- A checkout appears while it has an occupant with presence live or stale, or
  an active lease, or a delivery in the last 24h. Otherwise it belongs to
  History.
- Order: contested first, then by `live` desc, then `last_signal_ms` desc.

### 1.3 `RoomCard`

```jsonc
{
  "name": "release-room",
  "visibility": "shared|limited",
  "members": [Member],                      // membership::Member as today
  "repo_alias": "/abs/repo" | null,         // membership::repo_alias
  "archived": false,
  "last_activity_ms": 0,
  "latest_item": "<ULID>" | null,
  "unread": 0                               // newer than reads["room:<name>"]; computed server-side
}
```

Archived rooms are not in the list; `GET /api/v1/channels` remains for hygiene.

### 1.4 `PendingItem`

An obligation the operator can see and act on. Derived server-side from
`NoteKind::owes_an_answer` and acks, using the same reader
rules as `status::has_unacked_handoff`.

```jsonc
{
  "note": "<event ULID>",
  "kind": "handoff|ask",
  "from": "<session id>",  "from_name": "Lantern",
  "asked_ms": 0,
  "summary": "first line ≤140 bytes, as report.rs shapes it",
  "owed_by": [ { "session": "<id>", "display_name": "…", "presence": "live|stale" } ],
  "owed_total": 2,                          // readers who owe it; > owed_by.len() only when §6 trimmed the list
  "acked_by": [ "<session id>" ],
  "place": "checkout:<id>" | "room:<name>" | "session:<id>" | "all"
}
```

Rules:
- An item exists only while at least one `owed_by` session is live. Unanswered
  readers who have gone quiet stay listed with `presence: "stale"` because they
  still owe the answer, but they cannot keep the item alive on their own.
- A handoff has exactly one `owed_by`; an ask has every session it reached.
- The operator is a recipient too: a note addressed to the operator's session,
  and a broadcast `ask`, both list the operator in `owed_by` with presence
  `live` (the console is always reachable). An operator ack closes it.
- An ack by the operator (`POST /notes` with `ref`) closes the operator's copy
  the same way a session ack does: the item disappears from `pending` in the
  next projection. No local dismiss state.

## 2. The feed: one place, one paginated timeline

`GET /api/v1/feed?place=<place-id>&before=<ULID>&limit=<n≤200>` → `FeedPage`.

Place ids:

| place | what it contains |
|---|---|
| `all` | every note, ack and operator word; lifecycle markers (hello/intent/bye/name) folded as markers |
| `activity` | notes of kind warning/handoff/ask addressed to the operator or broadcast, plus acks to the operator |
| `checkout:<canonical path>` | notes addressed to that repo (via `repo` OR `repo_key`, as delivery does), notes authored by its occupants, its occupants' lifecycle markers, sluice receipts for the tree |
| `session:<id>` | notes authored by or addressed to that session, its acks, its lifecycle markers |
| `room:<name>` | notes addressed to `channel:<name>` and their acks |

```jsonc
{
  "place": "checkout:/abs",
  "items": [FeedItem],          // newest first
  "oldest": "<ULID>", "newest": "<ULID>",
  "has_more": true
}
```

`FeedItem` is the envelope the stream also uses (§3):

```jsonc
{
  "event": Event,                            // verbatim store event (frozen shape)
  "author": { "session": "…", "display_name": "…", "harness": "…", "presence": "live|stale|ended" },
  "places": ["all", "checkout:/abs", "session:<id>", …],   // every place this item belongs to
  "marker": "hello|intent|bye|name" | null,  // lifecycle markers render as markers, not messages
  "thread": { "root": "<ULID>", "replies": 3 } | null,
  "obligation": { "kind": "handoff|ask", "open": true } | null
}
```

Rules:
- Pagination walks segments newest-first with `read_history_page` semantics.
  Unknown `before` → 409 `{error:"cursor_expired"}`.
- `places` is computed by the server with the same functions the CLI uses for
  delivery (`query::inbox` reach rules). The console never re-implements
  "does this note belong here".
- The 500-event `Recent` window is not a limit on the feed; History and the
  live feed are the same timeline with different scroll positions.

## 3. The stream

`GET /api/v1/stream` (existing route, additive frames):

| frame | data | when |
|---|---|---|
| `event: bus` | `Event` | unchanged, for legacy readers |
| `event: item` | `FeedItem` | same tick as `bus`, one per accepted event |
| `event: reset` | `{ "reason": "cursor_expired" \| "max_age" \| "slow_client" }` | instead of silently starting from now |

Rules:
- `Last-Event-ID` is honoured; `?after=` remains as an override.
- On `reset`, the client refetches what it shows and the visible feed page.
  The server never silently drops the gap.
- Every `id:` on a frame is the event ULID.
- Max age and slow-client eviction send `reset` first, then close.

## 4. Operator read cursors

`POST /api/v1/operator/reads` `{ "place": "<place-id>", "event": "<ULID>" }` →
200 `{ "reads": {…} }`. Loopback/operator only (same door as `/operator`).
Stored in `~/.harbard/operator.json` under `reads`. Never touches
`cursors/` (agents' cursors). `unread` in §1.2/§1.3 and a top-level
`counts.unread_all` derive from these.

## 5. Errors on the routes in this spec

Every non-2xx body is `{ "error": "<snake_case code>", "message": "<human>",
…detail }`. Codes: `unauthorized`, `forbidden_origin`, `not_found`,
`cursor_expired`, `bad_request` (with `field`), `too_large`, `unknown_place`,
`store_unwritable` (500: the request was good and the disk refused it).
Legacy routes keep their strings; the console maps only these.

## 6. Performance

- The serialized projection is ≤ 128 KB. When it is larger, it sheds in this
  order, cheapest loss first:
  1. `sessions` rows with `presence=ended`, oldest signal first, already
     recoverable from `/feed`;
  2. `owed_by` past 8 entries per `PendingItem`, live readers kept first;
     `owed_total` still reports how many readers really owe the answer;
  3. `sessions` rows with `presence=stale`, oldest signal first.

  `pending` itself is NEVER shed, and neither is a live row: an obligation the
  operator cannot see is a debt that goes unpaid, which costs more than a big
  projection does. The budget is measured on the whole serialized body, not on
  `sessions` alone: a board with 20 leased checkouts and 50 open asks is
  about 90 KB, and almost none of that is sessions.

  `counts` are computed before any shedding, so no number ever moves because a
  row was dropped for size.
- `/feed` never parses more segments than needed for one page.

## 7. Edge cases (decided)

| # | Scenario | Behaviour |
|---|---|---|
| E1 | Session with `cwd` outside any git checkout | `checkout: null`; appears under sessions, never creates a CheckoutCard |
| E2 | Two sessions, same repo, different worktrees | two CheckoutCards sharing `repo_key`; never contested with each other |
| E3 | Parent + subagents in one worktree | one writer domain: `contested=false`, `delegated=n` |
| E4 | Lease held by a session that is now `ended` | `lease` still shown (leases expire on their own clock); `holds_lease=true` on the ended row; not counted live |
| E5 | Alias file missing/corrupt | `display_name` = hash-derived preferred alias (`alias::preferred`), `self_named=false` |
| E6 | A note addressed to a session id prefix | already refused at write time; the feed only sees full ids |
| E7 | `ask` reaching 4 sessions, 2 answered | one PendingItem, `owed_by` = the 2 unanswered, each with its own `presence`; a reader who went quiet stays listed as `stale` while someone live can still answer, and the item disappears once none of the unanswered is live |
| E8 | Operator acks a handoff meant for a session | closes the operator's view only if the operator was a recipient; otherwise the item stays (the obligation is the session's) |
| E9 | Room with all members on one repo (`repo_alias`) | still a RoomCard; `repo_alias` set so the UI can say "same as checkout X" |
| E10 | `before` older than the oldest segment | `has_more=false`, empty `items`, 200 (not 409: the cursor is valid, there is just nothing older) |
| E11 | Stream client reconnects with an id older than the hub window | `event: reset {cursor_expired}` then live frames from now |
| E12 | Serve has no sluice receipts for a tree | `last_delivery: null` |
| E13 | Two operators (two browsers) | one `reads` map; last write wins; both read the same `reads` |
| E14 | An event with an unknown `event` kind (v1 compatibility) | folded into `all` with `marker:null`, rendered as raw text by the client |
| E15 | Hostile note text ("ignore previous instructions…") | verbatim in `event.text`; the server never interprets it; the console renders it as inert text |

The JSON schema for `FleetView` is `spec/harbard/2026-09-06/fleet.schema.json`.
