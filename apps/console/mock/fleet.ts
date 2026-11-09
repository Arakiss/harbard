/**
 * Mock fleet projection — SPEC-FLEET-VIEW §1–§4 against the same fixtures.
 *
 * The point of this file is parity, not cleverness: the console must be unable
 * to tell the mock from the real `serve`, so the shape here is checked against
 * the committed schema by `fleet.test.ts`. Where the Rust side derives
 * something (a display name, a writer domain), this derives it the same way and
 * nothing more — a mock that computes a nicer answer than the server is a mock
 * that hides a bug.
 *
 * Data is synthetic only: no real hosts, no secrets.
 */

const STALE_SECS = 900;
const ENDED_VISIBLE_MS = 6 * 60 * 60 * 1000;
const DELIVERY_VISIBLE_MS = 24 * 60 * 60 * 1000;
const UNREAD_CAP = 99;
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

export interface MockLease {
  host: string;
  worktree: string;
  git_common_dir: string;
  repo: string;
  session: string;
  harness: string;
  branch: string | null;
  claimed_at_ms: number;
  renewed_at_ms: number;
  expires_at_ms: number;
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

/** The alias ledger (`~/.harbard/aliases.json`), as the fleet reads it. */
export interface MockAliases {
  by_session?: Record<string, string>;
  /** Sessions that chose their own callsign, rather than being given one. */
  self_named?: string[];
}

export interface FleetInputs {
  events: MockEvent[];
  now: number;
  leases: MockLease[];
  rooms: Record<string, Array<{ kind: string; value: string }>>;
  roomMeta: Record<
    string,
    { visibility?: string; last_note_ms?: number; purpose?: string }
  >;
  archived: string[];
  reads: Record<string, string>;
  /** The alias ledger, as `alias::read` returns it: names the sessions chose. */
  aliases?: MockAliases;
  operatorName: string;
  hubEnabled: boolean;
  devices: number;
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


const EPITHET_ADJ = ["quiet", "steady", "far", "patient", "cold", "long"];
const EPITHET_NOUN = ["crossing", "watch", "tide", "bearing", "channel"];

export function epithet(session: string): string {
  const h = hash(session);
  const adj = EPITHET_ADJ[Number(h % BigInt(EPITHET_ADJ.length))];
  const noun =
    EPITHET_NOUN[
      Number((h / BigInt(EPITHET_ADJ.length)) % BigInt(EPITHET_NOUN.length))
    ];
  return `the ${adj} ${noun}`;
}

const repositoryOf = (event: MockEvent): string | null =>
  event.repo_key ?? event.repo ?? null;

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

const occupiesCheckouts = (view: MockSessionView): boolean =>
  view.harness !== "human" && view.role !== "operator";

const countsLive = (view: MockSessionView): boolean =>
  view.liveness === "live" && !view.ending;

export function labelFor(repo: string): string {
  const parts = repo.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? repo;
}

/** Two or more distinct writer-domain roots in one tree. */
/** A bare `<host>-<pid>` session id: a shell ghost, not an agent that named itself. */
function looksLikeHostPidSession(session: string, host: string): boolean {
  const trimmed = host.trim();
  if (!trimmed) return false;
  const rest = session.startsWith(`${trimmed}-`)
    ? session.slice(trimmed.length + 1)
    : null;
  return rest !== null && rest.length > 0 && /^[0-9]+$/.test(rest);
}

/** How sure we are this session is who it says it is (topology::infer_identity_confidence). */
function identityOf(view: MockSessionView): "observed" | "inferred" | "unknown" {
  if (view.parent) return "observed";
  if (looksLikeHostPidSession(view.session, view.host)) return "unknown";
  if (view.harness === "other") return "inferred";
  return "observed";
}

function writerDomains(
  views: MockSessionView[],
  checkout: string,
  leaseHolders: Set<string>,
): string[] {
  const domains = new Set<string>();
  for (const view of views) {
    if (view.liveness !== "live") continue;
    if (view.repo !== checkout) continue;
    if (view.role === "operator" || view.harness === "human") continue;
    if (!leaseHolders.has(view.session)) {
      // The same two exclusions topology::is_writer_domain_member applies: a
      // Cursor Task or bare shell ghost is not a writer. An `other` session
      // with a real id and no parent IS one — dropping every parentless
      // `other` made the mock disagree with the server about who contends.
      const parentless = !view.parent && view.harness === "other";
      if (
        parentless &&
        (looksLikeHostPidSession(view.session, view.host) ||
          identityOf(view) === "unknown")
      ) {
        continue;
      }
    }
    domains.add(view.root);
  }
  return [...domains].sort();
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

export const eventReachesRoom = (event: MockEvent, room: string): boolean =>
  event.to?.kind === "channel" &&
  event.to.value.toLowerCase() === room.toLowerCase();

function unreadSince(
  cursor: string | undefined,
  latest: string | null,
  events: MockEvent[],
  belongs: (event: MockEvent) => boolean,
): number {
  if (!latest) return 0;
  if (!cursor) {
    return Math.min(events.filter(belongs).length, UNREAD_CAP);
  }
  if (cursor >= latest) return 0;
  return Math.min(
    events.filter((event) => event.id > cursor && belongs(event)).length,
    UNREAD_CAP,
  );
}

/** Notes that owe an answer, and who still owes it. */
function pendingItems(
  events: MockEvent[],
  views: MockSessionView[],
  nameOf: (session: string) => string,
  operator: { session: string; display_name: string },
): Array<Record<string, unknown>> {
  const closedBy = new Map<string, Set<string>>();
  for (const event of events) {
    const target =
      event.event === "ack"
        ? event.text.trim()
        : event.event === "note"
          ? (event.ref ?? "")
          : "";
    if (!target) continue;
    const set = closedBy.get(target) ?? new Set<string>();
    set.add(event.session);
    closedBy.set(target, set);
  }

  const reaches = (note: MockEvent, view: MockSessionView): boolean => {
    // A broadcast reaches everyone live, the operator included.
    if (!note.to) return true;
    switch (note.to.kind) {
      case "session":
        return note.to.value === view.session;
      case "harness":
        return note.to.value === view.harness;
      case "repo":
        return (
          note.to.value === view.repo ||
          note.to.value === (view.repo_key ?? view.repo)
        );
      default:
        return false;
    }
  };

  const items: Array<Record<string, unknown>> = [];
  for (const note of events) {
    if (note.event !== "note") continue;
    if (note.kind !== "handoff" && note.kind !== "ask") continue;
    const acked = closedBy.get(note.id) ?? new Set<string>();
    // Unanswered readers, live or stale (E7): a reader who went quiet still
    // owes the answer, and only a live one keeps the item alive.
    let owed = views.filter(
      (view) =>
        view.session !== note.session &&
        view.harness !== "human" &&
        view.role !== "operator" &&
        (view.liveness === "live" || view.liveness === "stale") &&
        !acked.has(view.session) &&
        reaches(note, view),
    );
    // E8: the operator is a recipient of an interrupt aimed at their own
    // session, and of a broadcast ask — and is always reachable, so live.
    const operatorOwes =
      !acked.has(operator.session) &&
      note.session !== operator.session &&
      (note.to
        ? (note.to.kind === "session" && note.to.value === operator.session) ||
          // Agents address the person at the console as `harness:human`.
          (note.to.kind === "harness" && note.to.value === "human")
        : note.kind === "ask");
    if (!operatorOwes && !owed.some((view) => view.liveness === "live")) continue;
    // A handoff with no target is a shape the CLI refuses today; the ones still
    // in the log behave as an ask, so no reader's debt is hidden by the cap.
    if (note.kind === "handoff" && note.to) {
      owed = operatorOwes
        ? []
        : owed.filter((view) => view.liveness === "live").slice(0, 1);
    }
    if (owed.length === 0 && !operatorOwes) continue;
    items.push({
      note: note.id,
      kind: note.kind,
      from: note.session,
      from_name: nameOf(note.session),
      asked_ms: ulidMs(note.id),
      summary: (note.text.split("\n")[0] ?? "").trim(),
      owed_by: [
        ...owed.map((view) => ({
          session: view.session,
          display_name: nameOf(view.session),
          presence: view.liveness,
        })),
        ...(operatorOwes
          ? [
              {
                session: operator.session,
                display_name: operator.display_name,
                presence: "live" as const,
              },
            ]
          : []),
      ],
      owed_total: owed.length + (operatorOwes ? 1 : 0),
      acked_by: [...acked].sort(),
      place: !note.to
        ? "all"
        : note.to.kind === "repo"
          ? checkoutPlace(note.to.value)
          : note.to.kind === "channel"
            ? roomPlace(note.to.value)
            : note.to.kind === "session"
              ? sessionPlace(note.to.value)
              : "all",
    });
  }
  items.sort(
    (a, b) =>
      (b.asked_ms as number) - (a.asked_ms as number) ||
      String(a.note).localeCompare(String(b.note)),
  );
  return items;
}

/** The whole board, from the fixtures. */
export function buildFleet(inputs: FleetInputs): Record<string, unknown> {
  const { events, now, leases, reads } = inputs;
  const views = computeViews(events, now);
  const { names, selfNamed } = resolveNames(events);
  const leaseHolders = new Set(leases.map((lease) => lease.session));

  const sessions = views
    .filter(
      (view) =>
        view.liveness !== "ended" || now - view.last_ms < ENDED_VISIBLE_MS,
    )
    .map((view) => ({
      session: view.session,
      display_name:
        view.harness === "human"
          ? inputs.operatorName
          : (inputs.aliases?.by_session?.[view.session] ??
            names.get(view.session) ??
            preferredName(view.session)),
      self_named:
        selfNamed.has(view.session) ||
        (inputs.aliases?.self_named?.includes(view.session) ?? false),
      epithet: epithet(view.session),
      harness: view.harness,
      host: view.host,
      role: view.role,
      cwd: view.cwd,
      checkout: view.repo?.trim() ? view.repo : null,
      branch: view.branch,
      parent: view.parent,
      root: view.root,
      depth: view.depth,
      descendants: view.descendants,
      identity: identityOf(view),
      intent: view.intent,
      intent_ms: view.intent_ms,
      presence: view.liveness,
      ending: view.ending,
      holds_lease: leaseHolders.has(view.session),
      first_ms: view.first_ms,
      last_ms: view.last_ms,
      signal_age_ms: Math.max(0, now - view.last_ms),
    }));

  // Presence order, with children immediately under their parent.
  const rank = (presence: string): number =>
    presence === "live" ? 0 : presence === "stale" ? 1 : 2;
  const known = new Set(sessions.map((card) => card.session));
  const children = new Map<string, typeof sessions>();
  const roots: typeof sessions = [];
  for (const card of sessions) {
    if (card.parent && known.has(card.parent)) {
      const list = children.get(card.parent) ?? [];
      list.push(card);
      children.set(card.parent, list);
    } else {
      roots.push(card);
    }
  }
  const sortRows = (rows: typeof sessions): void => {
    rows.sort(
      (a, b) =>
        rank(a.presence) - rank(b.presence) ||
        b.last_ms - a.last_ms ||
        a.session.localeCompare(b.session),
    );
  };
  sortRows(roots);
  for (const rows of children.values()) sortRows(rows);
  const orderedSessions: typeof sessions = [];
  const emit = (card: (typeof sessions)[number], seen: Set<string>): void => {
    if (seen.has(card.session)) return;
    seen.add(card.session);
    orderedSessions.push(card);
    for (const kid of children.get(card.session) ?? []) emit(kid, seen);
  };
  const seen = new Set<string>();
  for (const card of roots) emit(card, seen);

  // Checkouts: an occupant, a lease, or a recent delivery earns a card.
  const candidates = new Set<string>();
  for (const view of views) {
    if (!occupiesCheckouts(view)) continue;
    if (view.liveness === "ended") continue;
    if (view.repo?.trim()) candidates.add(view.repo);
  }
  for (const lease of leases) {
    if (lease.worktree) candidates.add(lease.worktree);
  }

  const checkouts = [...candidates]
    .map((id) => {
      const occupantViews = views.filter(
        (view) =>
          occupiesCheckouts(view) &&
          view.repo === id &&
          view.liveness !== "ended",
      );
      const repoKey =
        occupantViews.find((view) => view.repo_key)?.repo_key ?? id;
      const occupants = [...occupantViews]
        .sort(
          (a, b) => b.last_ms - a.last_ms || a.session.localeCompare(b.session),
        )
        .map((view) => view.session);
      const rivals = writerDomains(views, id, leaseHolders);
      const lease = leases.find((entry) => entry.worktree === id) ?? null;
      const latest =
        [...events]
          .reverse()
          .find((event) =>
            eventReachesCheckout(event, id, repoKey, occupants),
          )?.id ?? null;
      const lastSignal = Math.max(
        0,
        ...occupantViews.map((view) => view.last_ms),
        lease?.renewed_at_ms ?? 0,
      );
      return {
        id,
        repo_key: repoKey,
        label: labelFor(repoKey),
        worktree: id === repoKey ? null : labelFor(id),
        branch: occupantViews.find((view) => view.branch)?.branch ?? null,
        occupants,
        live: occupantViews.filter(
          (view) => countsLive(view) && view.depth === 0,
        ).length,
        delegated: occupantViews.filter(
          (view) => countsLive(view) && view.depth > 0,
        ).length,
        contested: rivals.length >= 2,
        rivals: rivals.length >= 2 ? rivals : [],
        lease,
        // The mock has no sluice receipts to read (E12).
        last_delivery: null as null,
        last_signal_ms: lastSignal,
        latest_item: latest,
        unread: unreadSince(
          reads[checkoutPlace(id)],
          latest,
          events,
          (event) => eventReachesCheckout(event, id, repoKey, occupants),
        ),
      };
    })
    .filter(
      (card) =>
        card.occupants.length > 0 ||
        card.lease !== null ||
        now - card.last_signal_ms < DELIVERY_VISIBLE_MS,
    );

  checkouts.sort(
    (a, b) =>
      Number(b.contested) - Number(a.contested) ||
      b.live - a.live ||
      b.last_signal_ms - a.last_signal_ms ||
      a.id.localeCompare(b.id),
  );

  const rooms = Object.entries(inputs.rooms)
    .filter(([name]) => !inputs.archived.includes(name))
    .map(([name, members]) => {
      const latest =
        [...events].reverse().find((event) => eventReachesRoom(event, name))
          ?.id ?? null;
      const meta = inputs.roomMeta[name];
      return {
        name,
        visibility: meta?.visibility === "limited" ? "limited" : "shared",
        members,
        repo_alias:
          members.length > 0 && members.every((m) => m.kind === "repo")
            ? (members[0]?.value ?? null)
            : null,
        archived: false,
        last_activity_ms: meta?.last_note_ms ?? 0,
        latest_item: latest,
        unread: unreadSince(reads[roomPlace(name)], latest, events, (event) =>
          eventReachesRoom(event, name),
        ),
      };
    })
    .sort(
      (a, b) =>
        b.last_activity_ms - a.last_activity_ms || a.name.localeCompare(b.name),
    );

  // Same ledger the session cards use: an obligation names the agent that owes
  // it exactly as the fleet row does, or the Inbox and the board disagree about
  // who is being chased.
  const nameOf = (session: string): string => {
    const view = views.find((candidate) => candidate.session === session);
    if (view?.harness === "human") return inputs.operatorName;
    return names.get(session) ?? preferredName(session);
  };
  const operatorCard = {
    session: "operator@night-watch.local",
    display_name: inputs.operatorName,
  };
  const pending = pendingItems(events, views, nameOf, operatorCard);

  return {
    v: 1,
    now_ms: now,
    as_of: events.length > 0 ? (events[events.length - 1]?.id ?? null) : null,
    stale_secs: STALE_SECS,
    counts: {
      live: orderedSessions.filter(
        (card) => card.presence === "live" && !card.ending && card.depth === 0,
      ).length,
      delegated: orderedSessions.filter(
        (card) => card.presence === "live" && !card.ending && card.depth > 0,
      ).length,
      stale: orderedSessions.filter((card) => card.presence === "stale").length,
      pending: pending.length,
      contested: checkouts.filter((card) => card.contested).length,
      unread_all: Math.min(
        checkouts.reduce((sum, card) => sum + card.unread, 0) +
          rooms.reduce((sum, card) => sum + card.unread, 0),
        UNREAD_CAP,
      ),
    },
    sessions: orderedSessions,
    checkouts,
    rooms,
    pending,
    operator: operatorCard,
    reads,
    hub: { enabled: inputs.hubEnabled, devices: inputs.devices },
  };
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
