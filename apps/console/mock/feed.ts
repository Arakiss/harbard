/**
 * Mock feed — SPEC-FLEET-VIEW (`/feed`, the stream's `item` frames, the
 * operator's reads) against the same fixtures.
 *
 * The point of this file is parity, not cleverness: the console must be unable
 * to tell the mock from the real `serve`. Where the Rust side derives
 * something (a display name, a place), this derives it the same way and
 * nothing more — a mock that computes a nicer answer than the server is a mock
 * that hides a bug.
 *
 * Data is synthetic only: no real hosts, no secrets.
 */

const STALE_SECS = 900;
const HERD_BYE_GRACE_MS = 10 * 60 * 1000;

export type Presence = "live" | "stale" | "ended";

export interface MockRecipient {
  kind: "harness" | "session" | "repo" | "channel";
  value: string;
}

export interface MockEvent {
  v: number;
  id: string;
  ts: string;
  event: string;
  session: string;
  harness: string;
  host: string;
  cwd: string;
  repo: string | null;
  repo_key?: string | null;
  branch: string | null;
  role: string;
  parent?: string | null;
  text: string;
  to?: MockRecipient | null;
  kind?: string;
  ref?: string | null;
}

export interface MockSessionView {
  session: string;
  harness: string;
  host: string;
  cwd: string;
  repo: string | null;
  repo_key: string | null;
  branch: string | null;
  role: string;
  parent: string | null;
  root: string;
  depth: number;
  descendants: number;
  intent: string | null;
  intent_ms: number;
  last_kind: string;
  last_ms: number;
  first_ms: number;
  liveness: Presence;
  ending: boolean;
}

export const checkoutPlace = (id: string): string => `checkout:${id}`;
export const roomPlace = (name: string): string => `room:${name}`;
export const sessionPlace = (id: string): string => `session:${id}`;

function ulidMs(id: string): number {
  const ENC = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  let ms = 0;
  for (const ch of id.slice(0, 10)) {
    const index = ENC.indexOf(ch);
    if (index < 0) return 0;
    ms = ms * 32 + index;
  }
  return ms;
}

/** FNV-1a over the session id — the same draw the Rust side uses for names. */
function hash(session: string): bigint {
  let h = 0xcbf29ce484222325n;
  for (const byte of new TextEncoder().encode(session)) {
    h ^= BigInt(byte);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h;
}

const U64 = 0xffffffffffffffffn;

/** splitmix64's finalizer — mirrors `mix` in the core. */
function mix(x: bigint): bigint {
  x = ((x ^ (x >> 30n)) * 0xbf58476d1ce4e5b9n) & U64;
  x = ((x ^ (x >> 27n)) * 0x94d049bb133111ebn) & U64;
  return x ^ (x >> 31n);
}

// The given names, in the core's order (`workshop::nth_name`).
const GIVEN_NAMES = [
  "Abigail", "Ada", "Adele", "Agnes", "Alice", "Alison", "Amanda", "Amber", "Amelia", "Amy",
  "Andrea", "Angela", "Anita", "Anna", "Annie", "Audrey", "Ava", "Barbara", "Beatrice", "Bella",
  "Bethany", "Bianca", "Brenda", "Bridget", "Brooke", "Caitlin", "Camille", "Carla", "Carmen",
  "Carol", "Caroline", "Cassandra", "Catherine", "Cecilia", "Celia", "Charlotte", "Chloe",
  "Christine", "Claire", "Clara", "Claudia", "Colleen", "Cynthia", "Daisy", "Danielle",
  "Deborah", "Diana", "Diane", "Donna", "Dorothy", "Edith", "Eileen", "Elaine", "Eleanor",
  "Elena", "Elise", "Eliza", "Elizabeth", "Ella", "Ellen", "Eloise", "Emily", "Emma", "Erica",
  "Erin", "Esther", "Eva", "Evelyn", "Fiona", "Florence", "Frances", "Gemma", "Georgia",
  "Gillian", "Gloria", "Hannah", "Harriet", "Hazel", "Heather", "Heidi", "Helen", "Holly",
  "Imogen", "Ingrid", "Irene", "Iris", "Isabel", "Ivy", "Jacqueline", "Jane", "Janet",
  "Jasmine", "Jean", "Jenna", "Jennifer", "Jessica", "Joan", "Joanna", "Jocelyn", "Josephine",
  "Joyce", "Judith", "Julia", "Juliet", "Kate", "Katherine", "Kathleen", "Kayla", "Kelly",
  "Kimberly", "Kirsten", "Laura", "Lauren", "Leah", "Leslie", "Lillian", "Linda", "Lisa",
  "Louise", "Lucy", "Lydia", "Lynn", "Mabel", "Madeline", "Maggie", "Margaret", "Maria",
  "Marian", "Marilyn", "Martha", "Mary", "Maureen", "Megan", "Melanie", "Melissa", "Meredith",
  "Michelle", "Miranda", "Molly", "Monica", "Nadia", "Naomi", "Natalie", "Nicole", "Nina",
  "Nora", "Olivia", "Pamela", "Patricia", "Paula", "Pauline", "Penelope", "Philippa", "Phoebe",
  "Polly", "Priscilla", "Rachel", "Rebecca", "Regina", "Renee", "Rita", "Robyn", "Rosalind",
  "Rosemary", "Ruby", "Ruth", "Sabrina", "Samantha", "Sandra", "Sara", "Sarah", "Scarlett",
  "Selena", "Shannon", "Sharon", "Sheila", "Shirley", "Sophia", "Sophie", "Stella", "Stephanie",
  "Susan", "Sylvia", "Tabitha", "Tamara", "Teresa", "Tessa", "Thea", "Tiffany", "Valerie",
  "Vanessa", "Vera", "Veronica", "Victoria", "Violet", "Virginia", "Vivian", "Wendy", "Whitney",
  "Yvonne", "Zoe", "Aaron", "Adam", "Adrian", "Aidan", "Alan", "Albert", "Alec", "Alexander",
  "Alfred", "Andrew", "Angus", "Anthony", "Archie", "Arnold", "Arthur", "Austin", "Barry",
  "Ben", "Benjamin", "Bernard", "Blake", "Bradley", "Brandon", "Brendan", "Brett", "Brian",
  "Bruce", "Bryan", "Caleb", "Calvin", "Cameron", "Carl", "Carter", "Casey", "Charles",
  "Christian", "Christopher", "Clark", "Clifford", "Clive", "Colin", "Connor", "Conrad",
  "Craig", "Curtis", "Cyril", "Dale", "Damian", "Daniel", "Darren", "David", "Dean", "Declan",
  "Dennis", "Derek", "Desmond", "Dominic", "Donald", "Douglas", "Duncan", "Dustin", "Dylan",
  "Edgar", "Edmund", "Edward", "Edwin", "Elliot", "Emmett", "Eric", "Ernest", "Ethan", "Eugene",
  "Evan", "Felix", "Fergus", "Finn", "Francis", "Fraser", "Frederick", "Gabriel", "Gareth",
  "Gary", "Gavin", "George", "Gerald", "Gilbert", "Glenn", "Gordon", "Graham", "Gregory",
  "Harold", "Harrison", "Harry", "Harvey", "Hector", "Henry", "Herbert", "Howard", "Hugh",
  "Hugo", "Ian", "Isaac", "Ivan", "Jack", "Jacob", "Jake", "James", "Jared", "Jason", "Jasper",
  "Jeffrey", "Jeremy", "Jerome", "Jesse", "Joel", "John", "Jonah", "Jonathan", "Jordan",
  "Joseph", "Joshua", "Julian", "Justin", "Keith", "Kenneth", "Kevin", "Kieran", "Kyle",
  "Lance", "Lawrence", "Leo", "Leonard", "Lewis", "Liam", "Lionel", "Logan", "Louis", "Lucas",
  "Luke", "Malcolm", "Marcus", "Martin", "Marvin", "Matthew", "Maurice", "Maxwell", "Michael",
  "Miles", "Mitchell", "Morgan", "Murray", "Nathan", "Neil", "Nelson", "Nicholas", "Nigel",
  "Noah", "Norman", "Oliver", "Oscar", "Owen", "Patrick", "Paul", "Percy", "Peter", "Philip",
  "Preston", "Quentin", "Ralph", "Randall", "Raymond", "Reginald", "Rex", "Richard", "Robert",
  "Rodney", "Roger", "Roland", "Ronald", "Rory", "Ross", "Roy", "Rupert", "Russell", "Ryan",
  "Samuel", "Scott", "Sean", "Sebastian", "Seth", "Shane", "Simon", "Spencer", "Stanley",
  "Stephen", "Stuart", "Terence", "Theodore", "Thomas", "Timothy", "Tobias", "Todd", "Tony",
  "Travis", "Trevor", "Tristan", "Troy", "Tyler", "Victor", "Vincent", "Walter", "Warren",
  "Wayne", "Wesley", "Wilfred", "William", "Xavier", "Zachary",
];
const FALLBACK_CANDIDATES = 64;

/** The `k`-th given name for a session — mirrors `alias::candidate`. */
function candidateName(session: string, k: number): string {
  const at = Number(mix(hash(`${session}#${k}`)) % BigInt(GIVEN_NAMES.length));
  return GIVEN_NAMES[at]!.toLowerCase();
}

function roman(n: number): string {
  const digits: [number, string][] = [
    [1000, "m"], [900, "cm"], [500, "d"], [400, "cd"], [100, "c"], [90, "xc"],
    [50, "l"], [40, "xl"], [10, "x"], [9, "ix"], [5, "v"], [4, "iv"], [1, "i"],
  ];
  let out = "";
  for (const [value, symbols] of digits) {
    while (n >= value) {
      out += symbols;
      n -= value;
    }
  }
  return out;
}

export function preferredName(session: string): string {
  return candidateName(session, 0);
}

/**
 * Lowercase ASCII nickname. The core refuses misshapen, generic and recent
 * names at `harbard name` time; a fixture replays only accepted events, so
 * normalizing is all the mock owes.
 */
function sanitizeClaim(name: string): string {
  const cleaned = name
    .trim()
    .toLowerCase()
    .split("")
    .map((c) => (/[a-z0-9-]/.test(c) ? c : "-"))
    .join("")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
  return cleaned.slice(0, 32);
}

/**
 * The name ledger, as `alias::resolve` and `alias::claim` build it.
 *
 * A session that introduced itself (`event: name`) keeps its callsign; anyone
 * else gets the hash-derived preferred alias (E5). Collisions move to the next
 * hash-derived candidate exactly as the core does, so a fixture cannot
 * accidentally prove a uniqueness the real server would not give.
 */
export function resolveNames(events: MockEvent[]): {
  names: Map<string, string>;
  selfNamed: Set<string>;
} {
  const names = new Map<string, string>();
  const selfNamed = new Set<string>();
  const taken = new Set<string>();

  for (const event of events) {
    if (event.event !== "name" || !event.text?.trim()) continue;
    const wanted = sanitizeClaim(event.text);
    if (!wanted || taken.has(wanted)) continue;
    names.set(event.session, wanted);
    selfNamed.add(event.session);
    taken.add(wanted);
  }

  for (const event of events) {
    if (names.has(event.session)) continue;
    let chosen: string | undefined;
    for (let k = 0; k < FALLBACK_CANDIDATES && chosen === undefined; k += 1) {
      const candidate = candidateName(event.session, k);
      if (!taken.has(candidate)) chosen = candidate;
    }
    if (chosen === undefined) {
      const base = preferredName(event.session);
      chosen = `${base}-ii`;
      for (let n = 3; taken.has(chosen); n += 1) chosen = `${base}-${roman(n)}`;
    }
    names.set(event.session, chosen);
    taken.add(chosen);
  }

  return { names, selfNamed };
}

/**
 * Collapse the log into one row per session, with the herd grace window that
 * keeps a fan-out's children visible after `bye` without counting them live.
 */
export function computeViews(
  events: MockEvent[],
  now: number,
): MockSessionView[] {
  const order: string[] = [];
  const map = new Map<string, MockSessionView>();

  for (const event of events) {
    const ms = ulidMs(event.id);
    let view = map.get(event.session);
    if (!view) {
      view = {
        session: event.session,
        harness: event.harness,
        host: event.host,
        cwd: event.cwd,
        repo: event.repo,
        repo_key: event.repo_key ?? null,
        branch: event.branch,
        role: event.role,
        parent: event.parent ?? null,
        root: event.session,
        depth: 0,
        descendants: 0,
        intent: null,
        intent_ms: 0,
        last_kind: event.event,
        last_ms: 0,
        first_ms: ms,
        liveness: "stale",
        ending: false,
      };
      map.set(event.session, view);
      order.push(event.session);
    }
    view.harness = event.harness;
    view.host = event.host;
    view.cwd = event.cwd;
    view.repo = event.repo;
    view.repo_key = event.repo_key ?? null;
    view.branch = event.branch;
    if (event.parent) view.parent = event.parent;
    if (event.role === "herd") view.role = "herd";
    else if (view.role !== "herd") view.role = event.role;
    view.last_kind = event.event;
    if (ms >= view.last_ms) view.last_ms = ms;
    if (ms < view.first_ms) view.first_ms = ms;
    if ((event.event === "intent" || event.event === "hello") && event.text) {
      view.intent = event.text;
      view.intent_ms = ms;
    }
  }

  const threshold = STALE_SECS * 1000;
  const views = order
    .map((session) => map.get(session))
    .filter((view): view is MockSessionView => view != null);

  for (const view of views) {
    const fresh = now - view.last_ms <= threshold || view.last_ms >= now;
    view.liveness =
      view.last_kind === "bye" ? "ended" : fresh ? "live" : "stale";
  }

  // Lineage: root, depth, and recursive descendants from the parent edges.
  const parents = new Map(views.map((view) => [view.session, view.parent]));
  for (const view of views) {
    let current = view.session;
    let root = current;
    let depth = 0;
    const seen = new Set([current]);
    for (;;) {
      const parent = parents.get(current);
      if (!parent || seen.has(parent)) break;
      seen.add(parent);
      root = parent;
      depth += 1;
      current = parent;
    }
    view.root = root;
    view.depth = depth;
  }
  const byId = new Map(views.map((view) => [view.session, view]));
  for (const view of views) {
    let current = view.parent;
    const seen = new Set<string>();
    while (current && !seen.has(current)) {
      seen.add(current);
      const parent = byId.get(current);
      if (!parent) break;
      parent.descendants += 1;
      current = parent.parent;
    }
  }

  // Herd grace: a child that said bye while its parent is still live stays
  // visible, and is marked `ending` so nothing counts it as presence.
  const liveParents = new Set(
    views.filter((view) => view.liveness === "live").map((v) => v.session),
  );
  for (const view of views) {
    if (view.role !== "herd" || view.liveness === "live") continue;
    if (!view.parent || !liveParents.has(view.parent)) continue;
    if (view.last_kind !== "bye") continue;
    if (now - view.last_ms <= HERD_BYE_GRACE_MS) {
      view.liveness = "live";
      view.ending = true;
    }
  }
  return views;
}

/** True when this event belongs on a checkout's timeline. */
export function eventReachesCheckout(
  event: MockEvent,
  id: string,
  repoKey: string,
  occupants: string[],
): boolean {
  if (
    event.to &&
    event.to.kind === "repo" &&
    (event.to.value === id || event.to.value === repoKey)
  ) {
    return true;
  }
  if (event.repo === id && !event.to) return true;
  return occupants.includes(event.session) && event.repo === id;
}

// ---------------------------------------------------------------------------
// The feed
// ---------------------------------------------------------------------------

export type Place =
  | { kind: "all" }
  | { kind: "activity" }
  | { kind: "checkout"; value: string }
  | { kind: "session"; value: string }
  | { kind: "room"; value: string };

export function parsePlace(raw: string): Place | { error: string } {
  const trimmed = raw.trim();
  if (trimmed === "all") return { kind: "all" };
  if (trimmed === "activity") return { kind: "activity" };
  const split = trimmed.indexOf(":");
  if (split < 0) return { error: `unknown place: ${trimmed}` };
  const kind = trimmed.slice(0, split);
  const value = trimmed.slice(split + 1);
  if (!value.trim()) return { error: `place ${kind} needs a value` };
  if (kind === "checkout") return { kind: "checkout", value };
  if (kind === "session") return { kind: "session", value };
  if (kind === "room") return { kind: "room", value: value.toLowerCase() };
  return { error: `unknown place kind: ${kind}` };
}

export const placeId = (place: Place): string =>
  place.kind === "all" || place.kind === "activity"
    ? place.kind
    : `${place.kind}:${place.value}`;

/** What earns the operator's attention. */
function isActivity(event: MockEvent): boolean {
  if (event.event === "ack") return true;
  if (event.event !== "note") return false;
  if (!["warning", "handoff", "ask"].includes(event.kind ?? "fyi")) return false;
  if (!event.to) return true;
  return event.to.kind === "session" || event.to.kind === "harness";
}

/** Every place an event belongs to, by the same reach rules delivery uses. */
export function placesOf(
  event: MockEvent,
  views: MockSessionView[],
  rooms: string[],
  answered: Map<string, MockEvent>,
): string[] {
  const places = new Set<string>(["all"]);
  if (isActivity(event)) places.add("activity");
  places.add(sessionPlace(event.session));

  const inherit = (to: MockRecipient | null | undefined): void => {
    if (!to) return;
    if (to.kind === "session") places.add(sessionPlace(to.value));
    else if (to.kind === "channel") {
      const name = to.value.toLowerCase();
      if (rooms.includes(name)) places.add(roomPlace(name));
    } else if (to.kind === "repo") places.add(checkoutPlace(to.value));
  };
  inherit(event.to);

  // An ack travels with the note it answers, so an answer given in a room
  // stays in that room rather than following its recipient out of it.
  const target =
    event.event === "ack" ? event.text.trim() : (event.ref ?? "");
  if (target) inherit(answered.get(target)?.to);

  const checkouts = new Map<string, string>();
  const occupants = new Map<string, string[]>();
  for (const view of views) {
    if (!view.repo?.trim()) continue;
    checkouts.set(view.repo, view.repo_key ?? view.repo);
    occupants.set(view.repo, [
      ...(occupants.get(view.repo) ?? []),
      view.session,
    ]);
  }
  for (const [id, repoKey] of checkouts) {
    if (eventReachesCheckout(event, id, repoKey, occupants.get(id) ?? [])) {
      places.add(checkoutPlace(id));
    }
  }
  if (event.repo?.trim()) places.add(checkoutPlace(event.repo));
  if (event.to?.kind === "repo") {
    for (const [id, repoKey] of checkouts) {
      if (repoKey === event.to.value) places.add(checkoutPlace(id));
    }
  }
  return [...places].sort();
}

const markerOf = (event: MockEvent): string | null =>
  event.event === "hello" || event.event === "intent" || event.event === "bye"
    ? event.event
    : null;

/** One item envelope — the same shape the stream pushes. */
export function feedItem(
  event: MockEvent,
  views: MockSessionView[],
  rooms: string[],
  answered: Map<string, MockEvent>,
  replies: Map<string, number>,
  closed: Set<string>,
  names?: Map<string, string>,
): Record<string, unknown> {
  const view = views.find((candidate) => candidate.session === event.session);
  const root = event.ref ?? event.id;
  const replyCount = replies.get(root) ?? 0;
  return {
    event,
    author: {
      session: event.session,
      display_name: names?.get(event.session) ?? preferredName(event.session),
      harness: event.harness,
      presence: view?.liveness ?? "ended",
    },
    places: placesOf(event, views, rooms, answered),
    marker: markerOf(event),
    thread:
      replyCount === 0 && !event.ref
        ? null
        : { root, replies: replyCount },
    obligation:
      event.event === "note" &&
      (event.kind === "handoff" || event.kind === "ask")
        ? { kind: event.kind, open: !closed.has(event.id) }
        : null,
  };
}

/** Everything an envelope needs, gathered once per request. */
export function feedContext(events: MockEvent[], now: number) {
  const views = computeViews(events, now);
  const answered = new Map<string, MockEvent>();
  const replies = new Map<string, number>();
  const closed = new Set<string>();
  for (const event of events) {
    const target =
      event.event === "ack" ? event.text.trim() : (event.ref ?? "");
    if (!target) continue;
    closed.add(target);
    if (event.ref) replies.set(target, (replies.get(target) ?? 0) + 1);
  }
  for (const event of events) {
    if (event.event === "note" && closed.has(event.id)) {
      answered.set(event.id, event);
    }
  }
  // One name ledger for the whole page: a Bus author and its fleet row must
  // read the same, which is exactly what they did not do before (§1.1, U2).
  const { names } = resolveNames(events);
  return { views, answered, replies, closed, names };
}

export function belongs(
  event: MockEvent,
  place: Place,
  views: MockSessionView[],
  rooms: string[],
  answered: Map<string, MockEvent>,
): boolean {
  if (place.kind === "all") return true;
  if (place.kind === "activity") return isActivity(event);
  return placesOf(event, views, rooms, answered).includes(placeId(place));
}

/** One page of a place's timeline, newest first. */
export function feedPage(
  events: MockEvent[],
  now: number,
  place: Place,
  before: string | null,
  limit: number,
  rooms: string[],
): Record<string, unknown> {
  const { views, answered, replies, closed, names } = feedContext(events, now);
  const newestFirst = [...events].reverse();
  let start = 0;
  if (before) {
    const index = newestFirst.findIndex((event) => event.id === before);
    if (index < 0) return { error: "cursor_expired", cursor: before };
    start = index + 1;
  }
  const matching = newestFirst
    .slice(start)
    .filter((event) => belongs(event, place, views, rooms, answered));
  const page = matching.slice(0, limit);
  return {
    place: placeId(place),
    items: page.map((event) =>
      feedItem(event, views, rooms, answered, replies, closed, names),
    ),
    oldest: page.length > 0 ? (page[page.length - 1]?.id ?? null) : null,
    newest: page.length > 0 ? (page[0]?.id ?? null) : null,
    has_more: matching.length > page.length,
  };
}
