import { readCoversEvent } from "../src/lib/operator-read";
import { mockDeskWrite, workspaceBody } from "./workspace";
import { updateBody, updateWrite } from "./update";
import { deletedRooms, isRoomAction, resetRooms, roomAction, roomHistory } from "./rooms";
import { mockTheme, mockThemeWrite, setMockTheme } from "./theme";
/**
 * Mock `harbard serve` — the frozen SPEC-API contract, verbatim, in bun.
 *
 * The console develops against this while the real Rust `serve` lands in
 * parallel. Both bind `127.0.0.1:7419` and speak the same contract, so the swap
 * is a base-URL change and nothing else (executor brief). This process holds an
 * in-memory event log built from a synthetic fixture; it recomputes the
 * `SessionView` read-model per request exactly as the core does, and it drives
 * the SSE stream on every append.
 *
 * Everything under `/api/v1` is the real contract. Endpoints under `/__control`
 * are a test-only door (append/reset/ready) that the console never calls — they
 * exist so Playwright can drive appends and scenario switches deterministically.
 *
 * Data is synthetic only: no real hosts, no secrets.
 */

import {
  feedContext,
  feedItem,
  feedPage,
  parsePlace,
  placeId,
  type MockEvent,
} from "./feed";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(HERE, "fixtures");

const PORT = Number(process.env.PORT ?? 7419);
const HOSTNAME = "127.0.0.1";
const STALE_SECS = 900;
const EVENT_WINDOW_MAX = 500;
const DEFAULT_FIXTURE = process.env.HARBARD_FIXTURE ?? "fleet";

// ---------------------------------------------------------------------------
// Event schema (mirror of the Rust core's serialized shape)
// ---------------------------------------------------------------------------

type EventKind = "hello" | "intent" | "note" | "bye" | "ack";
// `operator` is real: POST /notes stamps it, and fixtures need it to write
// the operator's own handoffs into the log (the pending tray reads those).
type Role = "main" | "herd" | "headless" | "operator";
type Liveness = "live" | "stale" | "ended";

interface Recipient {
  kind: "harness" | "session" | "repo" | "channel";
  value: string;
}

interface BusEvent {
  v: number;
  id: string;
  ts: string;
  event: EventKind;
  session: string;
  harness: string;
  host: string;
  cwd: string;
  repo: string | null;
  branch: string | null;
  role: Role;
  parent?: string | null;
  text: string;
  to?: Recipient | null;
  /** Note kind (F5): absent means `fyi`, the quiet default. */
  kind?: "fyi" | "warning" | "ask" | "handoff";
  ref?: string | null;
}

interface SessionView {
  session: string;
  harness: string;
  host: string;
  cwd: string;
  repo: string | null;
  branch: string | null;
  role: Role;
  parent: string | null;
  intent: string | null;
  last_ms: number;
  last_kind: EventKind;
  first_ms: number;
  liveness: Liveness;
}

interface FixtureEvent {
  event: EventKind;
  session: string;
  harness: string;
  host?: string;
  cwd: string;
  repo?: string | null;
  branch?: string | null;
  role?: Role;
  parent?: string | null;
  text?: string;
  to?: Recipient | null;
  kind?: BusEvent["kind"];
  /** Parent note id when this note replies to another. */
  ref?: string | null;
  /** Seconds before boot `now` this event was appended. */
  age_s: number;
}

interface Fixture {
  name: string;
  events: FixtureEvent[];
}

// ---------------------------------------------------------------------------
// ULID (Crockford base32). Deterministic: 48-bit ms prefix + monotonic suffix,
// so ids are valid, time-sortable, and stable across boots for the same fixture.
// ---------------------------------------------------------------------------

const ENC = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

function encodeTime(ms: number): string {
  let out = "";
  let n = ms;
  for (let i = 0; i < 10; i++) {
    out = ENC[n % 32] + out;
    n = Math.floor(n / 32);
  }
  return out;
}

function encodeSuffix(seq: number): string {
  let out = "";
  let n = seq;
  for (let i = 0; i < 16; i++) {
    out = ENC[n % 32] + out;
    n = Math.floor(n / 32);
  }
  return out;
}

function ulid(ms: number, seq: number): string {
  return encodeTime(ms) + encodeSuffix(seq);
}

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

let events: BusEvent[] = [];
let seq = 0;
let fixtureName = DEFAULT_FIXTURE;
let channelReads: Record<string, string> = {};
/** Operator read cursors, keyed by place id (SPEC-FLEET-VIEW §4). */
let operatorReads: Record<string, string> = {};

const ROOM_FIXTURE = "rooms";
const ROOM_NAME = "release-room";
const ROOM_MEMBERS = [{ kind: "repo", value: "/work/harbard" }];
const ROOM_META = {
  created_ms: 1_786_700_000_000,
  last_note_ms: null,
  purpose: "Decisions and bounded release coordination",
  visibility: "limited",
};

/**
 * Writer leases per scenario — who owns which checkout (mirror of
 * `WriterLeaseView` in harbard-core). Leases are NOT events: nothing on the bus
 * announces a claim, so the console polls this endpoint. Windows are minted per
 * request so a lease is always live for the length of a test run.
 */
interface WriterLease {
  claim_id: string;
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

type LeaseSeed = {
  worktree: string;
  session: string;
  harness: string;
  branch: string | null;
};

const LEASE_SEEDS: Record<string, LeaseSeed[]> = {
  fleet: [
    {
      worktree: "/work/ledger-web",
      session: "cygnus-1a8d4c",
      harness: "claude-code",
      branch: "main",
    },
    {
      worktree: "/work/atlas-api",
      session: "aurora-4f2c9a",
      harness: "claude-code",
      branch: "feat/key-rotation",
    },
  ],
  copresence: [],
  empty: [],
  xss: [],
};

function leases(now: number): WriterLease[] {
  const seeds = LEASE_SEEDS[fixtureName] ?? [];
  return seeds.map((seed) => ({
    claim_id: `01MOCKCLAIM${seed.session.slice(0, 6).toUpperCase()}`,
    host: "night-watch.local",
    worktree: seed.worktree,
    git_common_dir: `${seed.worktree}/.git`,
    repo: seed.worktree.split("/").filter(Boolean).pop() ?? seed.worktree,
    session: seed.session,
    harness: seed.harness,
    branch: seed.branch,
    claimed_at_ms: now - 600_000,
    renewed_at_ms: now - 20_000,
    expires_at_ms: now + 880_000,
  }));
}

function loadFixture(name: string): Fixture {
  const raw = readFileSync(join(FIXTURES, `${name}.json`), "utf8");
  return JSON.parse(raw) as Fixture;
}

function materialize(fx: Fixture, bootNow: number): BusEvent[] {
  // Oldest first (largest age), so array order == append order == file order.
  const ordered = [...fx.events].sort((a, b) => b.age_s - a.age_s);
  const out = ordered.map((e) => {
    const ms = bootNow - Math.round(e.age_s * 1000);
    return {
      v: 1,
      id: ulid(ms, seq++),
      ts: new Date(ms).toISOString(),
      event: e.event,
      session: e.session,
      harness: e.harness,
      host: e.host ?? "night-watch.local",
      cwd: e.cwd,
      repo: e.repo ?? null,
      branch: e.branch ?? null,
      role: e.role ?? "main",
      parent: e.parent ?? null,
      text: e.text ?? "",
      to: e.to ?? null,
      ...(e.kind ? { kind: e.kind } : {}),
      // Fixture `ref` may be a literal id or `{ref:N}` resolved below.
      ...(e.ref ? { ref: e.ref } : {}),
    } satisfies BusEvent;
  });
  // Ids are minted at boot, so a fixture ack cannot name its target literally.
  // `{ref:N}` in a text (or note.ref) resolves to the Nth materialized event's
  // id — the ack then reads exactly like cmd_ack wrote it.
  //
  // The index is positional, so inserting an event ahead of a target silently
  // re-points every later ref at the wrong note — which is exactly what adding
  // seven `name` events to this fixture did. Refuse loudly instead: a fixture
  // that points an ack at a `name` event is not a scenario, it is a bug that
  // will be read as behaviour.
  const refTarget = (raw: string, n: number): string => {
    const target = out[n];
    if (!target) throw new Error(`fixture ${raw}: no event at index ${n}`);
    if (target.event !== "note") {
      throw new Error(
        `fixture ${raw}: index ${n} is a ${target.event}, not a note — ` +
          `an ack can only close a note (did an event get inserted above it?)`,
      );
    }
    return target.id;
  };
  for (const ev of out) {
    ev.text = ev.text.replace(/\{ref:(\d+)\}/g, (raw, n) =>
      refTarget(raw, Number(n)),
    );
    if (ev.ref?.startsWith("{ref:")) {
      const m = /^\{ref:(\d+)\}$/.exec(ev.ref);
      if (m) ev.ref = refTarget(ev.ref, Number(m[1]));
    }
  }
  return out;
}

function resetTo(name: string): void {
  seq = 0;
  fixtureName = name;
  channelReads = {};
  operatorReads = {};
  events = materialize(loadFixture(name), Date.now());
}

function ulidMs(id: string): number {
  const head = id.slice(0, 10).toUpperCase();
  if (head.length < 10) return 0;
  let ms = 0;
  for (const ch of head) {
    const v = ENC.indexOf(ch);
    if (v < 0) return 0;
    ms = ms * 32 + v;
  }
  return ms;
}

// ---------------------------------------------------------------------------
// Read model: collapse events into SessionView[] (mirror of core `sessions()`)
// ---------------------------------------------------------------------------

function computeSessions(now: number): SessionView[] {
  const order: string[] = [];
  const map = new Map<string, SessionView>();

  for (const ev of events) {
    const ms = ulidMs(ev.id);
    let view = map.get(ev.session);
    if (!view) {
      view = {
        session: ev.session,
        harness: ev.harness,
        host: ev.host,
        cwd: ev.cwd,
        repo: ev.repo,
        branch: ev.branch,
        role: ev.role,
        parent: ev.parent ?? null,
        intent: null,
        last_ms: 0,
        last_kind: ev.event,
        first_ms: ms,
        liveness: "stale",
      };
      map.set(ev.session, view);
      order.push(ev.session);
    }
    // Latest event (file order) owns the metadata.
    view.harness = ev.harness;
    view.host = ev.host;
    view.cwd = ev.cwd;
    view.repo = ev.repo;
    view.branch = ev.branch;
    view.role = ev.role;
    view.parent = ev.parent ?? null;
    view.last_kind = ev.event;
    if (ms >= view.last_ms) view.last_ms = ms;
    if (ms < view.first_ms) view.first_ms = ms;
    if ((ev.event === "intent" || ev.event === "hello") && ev.text) {
      view.intent = ev.text;
    }
  }

  const threshold = STALE_SECS * 1000;
  const out = order
    .map((s) => map.get(s))
    .filter((v): v is SessionView => v != null);
  for (const view of out) {
    const fresh = now - view.last_ms <= threshold || view.last_ms >= now;
    const ended = view.last_kind === "bye";
    view.liveness = ended ? "ended" : fresh ? "live" : "stale";
  }
  // Contract: newest signal first.
  out.sort((a, b) => b.last_ms - a.last_ms);
  return out;
}

function clip(text: string, max: number): string {
  const t = text.trim();
  const chars = [...t];
  if (chars.length <= max) return t;
  return `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

function checkoutHistory(now: number) {
  const views = computeSessions(now).filter((s) => s.harness !== "human");
  const byRepo = new Map<string, SessionView[]>();
  for (const view of views) {
    const key = view.repo?.trim() || "(no checkout)";
    const list = byRepo.get(key) ?? [];
    list.push(view);
    byRepo.set(key, list);
  }
  const checkouts = [...byRepo.entries()].map(([repo, occupantsIn]) => {
    const occupants = occupantsIn.map((view) => {
      const lastNote = [...events]
        .reverse()
        .find(
          (e) =>
            e.session === view.session &&
            e.event === "note" &&
            e.text.trim().length > 0,
        );
      const intent = view.intent?.trim() || "no declared intent";
      const epitaph =
        view.liveness === "ended"
          ? lastNote
            ? `${intent} · last: ${clip(lastNote.text, 72)}`
            : `${intent} · left no note`
          : null;
      return {
        session: view.session,
        harness: view.harness,
        liveness: view.liveness,
        intent: view.intent,
        epitaph,
        last_ms: view.last_ms,
      };
    });
    occupants.sort((a, b) => b.last_ms - a.last_ms);
    const marks = events
      .filter((e) => {
        const same =
          (e.repo ?? "(no checkout)") === repo ||
          (!e.repo && repo === "(no checkout)");
        return (
          same &&
          (e.event === "note" ||
            e.event === "intent" ||
            e.event === "bye" ||
            e.event === "ack")
        );
      })
      .map((e) => ({
        id: e.id,
        ms: ulidMs(e.id),
        event: e.event,
        kind: e.event === "note" ? (e.kind ?? "fyi") : e.event,
        session: e.session,
        text: clip(e.text, 96),
      }))
      .sort((a, b) => b.ms - a.ms)
      .slice(0, 10);
    const last_ms = Math.max(
      0,
      ...occupants.map((o) => o.last_ms),
      ...marks.map((m) => m.ms),
    );
    const label = repo.split(/[/\\]/).filter(Boolean).at(-1) ?? repo;
    return {
      repo,
      label,
      last_ms,
      live: occupants.filter((o) => o.liveness === "live").length,
      ended: occupants.filter((o) => o.liveness === "ended").length,
      occupants,
      marks,
      last_delivery: null,
    };
  });
  checkouts.sort((a, b) => b.last_ms - a.last_ms);
  return { checkouts, now_ms: now };
}

function noteTrace(id: string) {
  const matches = events.filter(
    (e) => e.event === "note" && e.id.startsWith(id),
  );
  if (matches.length === 0) return null;
  if (matches.length > 1) return null;
  const note = matches[0];
  if (!note) return null;
  return {
    note_id: note.id,
    kind: note.kind ?? "fyi",
    to: note.to ? `${note.to.kind}:${note.to.value}` : null,
    author: note.session,
    via: "mock",
    appended_ms: ulidMs(note.id),
    deliveries: [] as Array<{
      session: string;
      verb: string;
      ms: number;
      envelope: string;
      continued_after_ms: number | null;
    }>,
    pending: computeSessions(Date.now())
      .filter((s) => s.harness !== "human" && s.session !== note.session)
      .map((s) => ({
        session: s.session,
        harness: s.harness,
        verdict:
          s.liveness === "ended"
            ? "this reader has ended — the note will never be picked up (last seen never)"
            : "no hook activity ever recorded",
      })),
  };
}

/** The room names the feed may place an item in. */
function roomNames(): string[] {
  return fixtureName === ROOM_FIXTURE ? [ROOM_NAME] : [];
}

// ---------------------------------------------------------------------------
// SSE subscribers
// ---------------------------------------------------------------------------

type Sub = (chunk: string) => void;
const subs = new Set<Sub>();

function broadcast(ev: BusEvent): void {
  const now = Date.now();
  // The legacy frame, unchanged for readers that only know `bus`.
  let frame = `id: ${ev.id}\nevent: bus\ndata: ${JSON.stringify(ev)}\n\n`;

  // The envelope, on the same tick — so the console never has to ask "does
  // this belong on my screen".
  const all = events as unknown as MockEvent[];
  const { views, answered, replies, closed, names } = feedContext(all, now);
  const item = feedItem(
    ev as unknown as MockEvent,
    views,
    roomNames(),
    answered,
    replies,
    closed,
    names,
  );
  frame += `id: ${ev.id}\nevent: item\ndata: ${JSON.stringify(item)}\n\n`;

  for (const send of subs) send(frame);
}

/** Tell a client its resume point is gone rather than starting silently. */
export function resetFrame(reason: string): string {
  return `event: reset\ndata: ${JSON.stringify({ reason })}\n\n`;
}

function appendEvent(
  partial: Partial<FixtureEvent> & { event: EventKind },
  publish = true,
): BusEvent {
  const now = Date.now();
  const ev: BusEvent = {
    v: 1,
    id: ulid(now, seq++),
    ts: new Date(now).toISOString(),
    event: partial.event,
    session: partial.session ?? "unknown",
    harness: partial.harness ?? "other",
    host: partial.host ?? "night-watch.local",
    cwd: partial.cwd ?? "/",
    repo: partial.repo ?? null,
    branch: partial.branch ?? null,
    role: partial.role ?? "main",
    parent: partial.parent ?? null,
    text: partial.text ?? "",
    to: partial.to ?? null,
    ...(partial.kind ? { kind: partial.kind } : {}),
    ...(partial.ref ? { ref: partial.ref } : {}),
  };
  events.push(ev);
  if (publish) broadcast(ev);
  return ev;
}

/** Mirror core thread_for and serve thread_body: full bodies, append order. */
function noteThread(id: string) {
  let root = events.find((event) => event.id === id);
  if (!root) return null;
  for (let hop = 0; hop < 16 && root.ref; hop += 1) {
    const parent = events.find((event) => event.id === root!.ref);
    if (!parent || parent === root) break;
    root = parent;
  }
  const head = root;
  const ids = [head.id];
  const seen = new Set(ids);
  for (let scanned = 0; scanned < ids.length; scanned += 1) {
    const parent = ids[scanned];
    for (const event of events) {
      if (seen.has(event.id)) continue;
      const ackTarget = event.ref ?? event.text.split(/[ \t\n\r\v\f]+/).find(Boolean);
      const child = event.event !== "ack" && event.ref === parent;
      const ack = event.event === "ack" && ackTarget !== undefined && /^[0-9A-Za-z]{26}$/.test(ackTarget) && ackTarget === parent && (event.ref != null || parent === head.id);
      if (child || ack) { ids.push(event.id); seen.add(event.id); }
    }
  }
  const { names } = feedContext(events as unknown as MockEvent[], Date.now());
  const ordered = [head, ...events.filter((event) => event.id !== head.id && seen.has(event.id))];
  const messages = ordered.map((event) => {
    const alias = names.get(event.session);
    return { id: event.id, ts: event.ts, event: event.event, kind: event.kind ?? "fyi", session: event.session,
      name: alias ? alias.charAt(0).toUpperCase() + alias.slice(1) : null,
      harness: event.harness, to: event.to ? `${event.to.kind}:${event.to.value}` : null, ref: event.ref ?? null, text: event.text };
  });
  return { thread: head.id, count: messages.length, messages };
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

const JSON_CT = "application/json; charset=utf-8";

/** Loopback console origins, including portless named URLs. */
function isLocalConsoleOrigin(origin: string): boolean {
  try {
    const u = new URL(origin);
    // An Origin has no path/query/auth; reject anything that sniffed like one.
    if (u.username || u.password || u.pathname !== "/" || u.search || u.hash) {
      return false;
    }
    if (u.protocol !== "http:" && u.protocol !== "https:") return false;
    const host = u.hostname;
    if (host === "localhost" || host === "127.0.0.1") {
      return u.protocol === "http:";
    }
    return host.endsWith(".localhost") || host.endsWith(".local");
  } catch {
    return false;
  }
}

function corsHeaders(origin: string | null): Record<string, string> {
  const h: Record<string, string> = {
    // Must match the real `serve`: without POST and the content-type header
    // announced here, a browser blocks the F2 write at the preflight and the
    // composer fails with no request ever reaching this process.
    "Access-Control-Allow-Methods": "GET, HEAD, POST",
    "Access-Control-Allow-Headers": "content-type",
    Vary: "Origin",
  };
  if (origin && isLocalConsoleOrigin(origin)) {
    h["Access-Control-Allow-Origin"] = origin;
  }
  return h;
}

function json(body: unknown, origin: string | null, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": JSON_CT, ...corsHeaders(origin) },
  });
}

function logLine(method: string, path: string): void {
  // Method + path only, never query values (contract: server conventions).
  process.stderr.write(`${method} ${path}\n`);
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

resetTo(DEFAULT_FIXTURE);

const STARTED = Date.now();

const server = Bun.serve({
  hostname: HOSTNAME,
  port: PORT,
  idleTimeout: 0,
  async fetch(req) {
    const url = new URL(req.url);
    const path = url.pathname;
    const origin = req.headers.get("origin");
    logLine(req.method, path);

    // Test-only control door — never part of the contract, never called by the
    // console. Kept POST so it can never be mistaken for a read endpoint.
    if (path.startsWith("/__control/")) {
      return handleControl(req, url, path, origin);
    }

    if (req.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders(origin) });
    }

    // The F2 write door, mirrored from the real `serve`: author identity is
    // assigned here and is never a client input, so the console cannot
    // impersonate an agent against the mock either.
    // Theme writes answer with the theme state, as serve does (mock/theme.ts).
    if (path.startsWith("/api/v1/workspace/theme-") && req.method === "POST") {
      const body = (await req.json().catch(() => ({}))) as Record<string, unknown>;
      return json(mockThemeWrite(path.slice("/api/v1/workspace/".length), body), origin);
    }
    // The workspace shell's desk writes (read marks, motion…): accepted, not kept.
    if (path.startsWith("/api/v1/workspace/") && req.method === "POST") {
      const body = (await req.json().catch(() => ({}))) as Record<string, unknown>;
      mockDeskWrite(path.slice("/api/v1/workspace/".length), body);
      return json({ ok: true }, origin);
    }
    // "Update harbard": a believable newer main and a run on a clock (mock/update.ts).
    if (path.startsWith("/api/v1/update/") && req.method === "POST") {
      const { status, body } = await updateWrite(path.slice("/api/v1/update/".length), req);
      return json(body, origin, status);
    }
    if (path === "/api/v1/notes" && req.method === "POST") {
      return handlePostNote(req, origin);
    }

    if (path === "/api/v1/channels" && req.method === "POST") {
      return handlePostChannel(req, origin);
    }

    // §4 — the operator's own reading position. Refused to satellites the same
    // way the real serve refuses them: a paired phone marking work read on a
    // screen nobody was looking at is not a feature.
    if (path === "/api/v1/operator/reads" && req.method === "POST") {
      return handlePostReads(req, origin);
    }

    if (
      (req.method === "GET" || req.method === "HEAD") &&
      path.startsWith("/api/v1/notes/") &&
      path.endsWith("/trace")
    ) {
      const id = path.slice("/api/v1/notes/".length, -"/trace".length);
      const report = noteTrace(id);
      if (!report) return json({ error: "unknown_note", id }, origin, 404);
      return json(report, origin);
    }

    if ((req.method === "GET" || req.method === "HEAD") && path.startsWith("/api/v1/notes/") && path.endsWith("/thread")) {
      const id = path.slice("/api/v1/notes/".length, -"/thread".length).trim();
      if (!id || id.includes("/")) return json({ error: "expected /api/v1/notes/{id}/thread" }, origin, 400);
      const thread = noteThread(id);
      return thread ? json(thread, origin) : json({ error: "unknown_note", id }, origin, 404);
    }

    if (req.method !== "GET" && req.method !== "HEAD") {
      return json(
        {
          error:
            "method not allowed; this API is GET/HEAD only, except POST /api/v1/notes",
        },
        origin,
        405,
      );
    }

    if (path === "/api/v1/channels/deleted") return json(deletedRooms(), origin);
    if (path === "/api/v1/history/agents") return json({ items: [], next: null, total: 0, channels: [], now_ms: Date.now(), window_days: 31 }, origin);
    const roomHistoryPath = /^\/api\/v1\/channels\/([^/]+)\/history$/.exec(path);
    if (roomHistoryPath) {
      const answer = roomHistory(decodeURIComponent(roomHistoryPath[1]!), url.searchParams.get("before"), url.searchParams.get("limit"));
      return json(answer.body, origin, answer.status);
    }
    if (path === "/api/v1/theme") {
      const theme = mockTheme();
      if (!theme) return json({ error: "no theme in this scenario" }, origin, 404);
      return json(url.searchParams.get("since") === theme.revision ? { revision: theme.revision, unchanged: true } : theme, origin);
    }

    switch (path) {
      case "/api/v1/profile": {
        // Available, nothing typed: the shape the menu reads.
        return json(
          {
            profile: { photo_rev: 0, updated_ms: 0 },
            status: null,
            quiet: false,
            quiet_until_ms: null,
            state: "available",
            state_until_ms: null,
            dock: { bounce: true, alerts: true, one_shots: true },
            now_ms: Date.now(),
            limits: { context: 1500, name: 60, address: 120, status_text: 100, photo_bytes: 2 * 1024 * 1024 },
          },
          origin,
        );
      }
      case "/api/v1/workspace": {
        return json(workspaceBody(fixtureName, url.searchParams.get("only")), origin);
      }
      case "/api/v1/snapshot": {
        const now = Date.now();
        const eventWindow = readEvents(url);
        return json(
          {
            sessions: computeSessions(now),
            ...eventWindow,
            channels:
              fixtureName === ROOM_FIXTURE ? { [ROOM_NAME]: ROOM_MEMBERS } : {},
            archived: [],
            channel_meta:
              fixtureName === ROOM_FIXTURE ? { [ROOM_NAME]: ROOM_META } : {},
            channel_reads: channelReads,
            by_session: {},
            epithets: {},
            delivery: [],
            leases: leases(now),
            leases_known: true,
            now_ms: now,
          },
          origin,
        );
      }
      case "/api/v1/feed": {
        const raw = url.searchParams.get("place") ?? "all";
        const place = parsePlace(raw);
        if ("error" in place) {
          return json(
            { error: "unknown_place", message: place.error, place: raw },
            origin,
            400,
          );
        }
        const rawLimit = url.searchParams.get("limit");
        let limit = 50;
        if (rawLimit !== null) {
          const parsed = Number(rawLimit);
          if (!Number.isInteger(parsed) || parsed < 1 || parsed > 200) {
            return json(
              {
                error: "bad_request",
                message: "limit must be between 1 and 200",
                field: "limit",
              },
              origin,
              400,
            );
          }
          limit = parsed;
        }
        const page = feedPage(
          events as unknown as MockEvent[],
          Date.now(),
          place,
          url.searchParams.get("before"),
          limit,
          roomNames(),
        );
        if ("error" in page) {
          return json(
            {
              error: "cursor_expired",
              message:
                "the before cursor is no longer in the store; refetch from the newest page",
              cursor: page.cursor,
            },
            origin,
            409,
          );
        }
        return json(page, origin);
      }
      case "/api/v1/sessions": {
        const now = Date.now();
        return json({ sessions: computeSessions(now), now_ms: now }, origin);
      }
      case "/api/v1/events": {
        return json(readEvents(url), origin);
      }
      case "/api/v1/leases": {
        const now = Date.now();
        return json({ leases: leases(now), now_ms: now }, origin);
      }
      case "/api/v1/history/sessions": {
        const now = Date.now();
        const sessions = computeSessions(now).map((s) =>
          s.liveness === "ended" ? { ...s, liveness: "stale" as const } : s,
        );
        return json({ sessions, now_ms: now, window: "all" }, origin);
      }
      case "/api/v1/history/checkouts": {
        return json(checkoutHistory(Date.now()), origin);
      }
      case "/api/v1/stream": {
        // `?after=` is the explicit override; `Last-Event-ID` is what a
        // reconnecting EventSource sends on its own.
        const after =
          url.searchParams.get("after") ?? req.headers.get("last-event-id");
        return streamResponse(origin, after);
      }
      case "/api/v1/doctor": {
        return json(doctor(), origin);
      }
      case "/api/v1/setup": {
        return json(
          {
            needs_setup: false,
            completed_ms: STARTED,
            skipped: [],
            wired: ["claude", "codex", "cursor"],
            harnesses: [
              {
                target: "claude",
                harness: "claude-code",
                path: "~/.claude/settings.json",
                config_exists: true,
                ok: true,
                present: true,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: null,
                blurb:
                  "Claude Code hooks call harbard so Claude hears notes on the bus.",
              },
              {
                target: "codex",
                harness: "codex",
                path: "~/.codex/hooks.json",
                config_exists: true,
                ok: true,
                present: true,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: null,
                blurb: "Codex hooks wire hello/brief/bye for the fleet board.",
              },
              {
                target: "cursor",
                harness: "cursor",
                path: "~/.cursor/hooks.json",
                config_exists: true,
                ok: true,
                present: true,
                recommended: false,
                missing_events: [],
                retired_lingering: [],
                hint: null,
                blurb: "Cursor hooks inject briefs into agent context.",
              },
            ],
            loopback_install: true,
          },
          origin,
        );
      }
      case "/api/v1/learning": {
        return json(
          {
            since_ms: STARTED,
            now_ms: Date.now(),
            counters: {
              notes: 0,
              injected: 0,
              injected_pct: null,
              time_to_inject_p50_ms: null,
              acks: 0,
              unacked_handoffs: 0,
            },
          },
          origin,
        );
      }
      case "/api/v1/pairing": {
        // Preferences › Pairing: only this Mac, so the section shows its
        // empty state ("Only this Mac") instead of "harbard is not responding".
        return json(
          {
            devices: [
              {
                id: "mock-hub",
                name: "night-watch",
                host: "night-watch.local",
                os: "macos",
                hub: true,
                access: "operator",
                created_ms: 0,
                joined_ms: STARTED,
                last_seen_ms: STARTED,
                revoked_ms: null,
                credential: true,
                recent: [],
                reads: 0,
                writes: 0,
              },
            ],
            now_ms: Date.now(),
          },
          origin,
        );
      }
      case "/api/v1/update": {
        return json(updateBody(), origin);
      }
      case "/api/v1/meta": {
        return json(
          {
            name: "harbard",
            version: "0.1.0-mock",
            host: "night-watch.local",
            started_ms: STARTED,
            protocol: "harbard/protocol/v1",
          },
          origin,
        );
      }
      case "/api/v1/protocol": {
        return json(
          {
            version: "harbard/protocol/v1",
            spec: "harbard/2026-08-02",
            depth:
              url.searchParams.get("depth") === "full" ? "full" : "compact",
            correction_codes: [
              "intent_stale",
              "ack_pending",
              "contested_checkout",
              "lease_held",
              "lease_unprotected",
              "broadcast_handoff",
              "hooks_unhealthy",
            ],
            deprecated_brief_aliases: {
              status: "intent_stale",
              ack: "ack_pending",
            },
            receipt_stages: [
              "delivered",
              "observed",
              "applied",
              "rejected",
              "expired",
            ],
            capabilities: [],
            rules: [],
            text: "harbard · protocol (mock)",
          },
          origin,
        );
      }
      default:
        return json({ error: `unknown path: ${path}` }, origin, 404);
    }
  },
});

function readEvents(url: URL): {
  events: BusEvent[];
  total: number;
  returned: number;
  newest_id: string | null;
} {
  const after = url.searchParams.get("after");
  const limitRaw = Number(url.searchParams.get("limit") ?? EVENT_WINDOW_MAX);
  const limit = Math.max(
    1,
    Math.min(
      EVENT_WINDOW_MAX,
      Number.isFinite(limitRaw) ? limitRaw : EVENT_WINDOW_MAX,
    ),
  );
  let start = 0;
  if (after) {
    const idx = events.findIndex((e) => e.id === after);
    start = idx >= 0 ? idx + 1 : 0;
  }
  const window = events.slice(start);
  const returnedEvents = window.slice(0, limit);
  return {
    events: returnedEvents,
    total: window.length,
    returned: returnedEvents.length,
    newest_id: events.length ? events[events.length - 1]!.id : null,
  };
}

function doctor(): Record<string, unknown> {
  const sessions = new Set(events.map((e) => e.session));
  return {
    store_root: "/mock/.harbard",
    store_exists: true,
    segments: events.length ? 1 : 0,
    total_lines: events.length,
    events: events.length,
    corrupt_lines: 0,
    unknown_events: 0,
    cursors: sessions.size,
    retention_days: 14,
    oldest_segment: events.length
      ? new Date(ulidMs(events[0]!.id)).toISOString().slice(0, 10)
      : null,
    newest_segment: events.length
      ? new Date(ulidMs(events[events.length - 1]!.id))
          .toISOString()
          .slice(0, 10)
      : null,
    leases: {
      healthy: true,
      total: 0,
      active: 0,
      stale: 0,
      corrupt: 0,
      temp_files: 0,
      inaccessible: 0,
    },
    open_resolutions: 0,
    expired_resolutions: 0,
    channels: {
      orphan: [],
      armed: [],
      dormant: [],
      legacy_unknown_age: [],
      armed_past_grace: [],
    },
    open_channel_unused: [],
    armed_channel_unused: 0,
    hooks: {
      healthy: true,
      harnesses: [
        {
          target: "claude",
          harness: "claude-code",
          path: "/mock/.claude/settings.json",
          config_exists: true,
          ok: true,
          missing_events: [],
          retired_lingering: [],
          hint: null,
        },
        {
          target: "codex",
          harness: "codex",
          path: "/mock/.codex/hooks.json",
          config_exists: true,
          ok: true,
          missing_events: [],
          retired_lingering: [],
          hint: null,
        },
        {
          target: "cursor",
          harness: "cursor",
          path: "/mock/.cursor/hooks.json",
          config_exists: true,
          ok: true,
          missing_events: [],
          retired_lingering: [],
          hint: null,
        },
      ],
    },
  };
}

/**
 * `POST /api/v1/operator/reads` — where the operator has read up to.
 *
 * Mirrors the real door: a satellite is refused, the place must be one the
 * console can name, and the event must be a full id.
 */
async function handlePostReads(
  req: Request,
  origin: string | null,
): Promise<Response> {
  if (req.headers.get("x-harbard-device")) {
    return json(
      {
        error: "forbidden_origin",
        message: "satellites cannot move the operator's read cursors",
      },
      origin,
      403,
    );
  }
  let parsed: unknown;
  try {
    parsed = await req.json();
  } catch {
    return json(
      { error: "bad_request", message: "invalid JSON", field: "body" },
      origin,
      400,
    );
  }
  const body = (parsed ?? {}) as { place?: unknown; event?: unknown };
  const rawPlace = typeof body.place === "string" ? body.place.trim() : "";
  if (!rawPlace) {
    return json(
      { error: "bad_request", message: "place is required", field: "place" },
      origin,
      400,
    );
  }
  const place = parsePlace(rawPlace);
  if ("error" in place) {
    return json(
      { error: "unknown_place", message: place.error, place: rawPlace },
      origin,
      400,
    );
  }
  const event = typeof body.event === "string" ? body.event.trim() : "";
  if (!/^[0-9A-Za-z]{26}$/.test(event)) {
    return json(
      {
        error: "bad_request",
        message: "event must be a full event id (26 characters)",
        field: "event",
      },
      origin,
      400,
    );
  }
  const key = placeId(place);
  const previous = operatorReads[key];
  if (!previous || readCoversEvent(events, previous, event)) operatorReads[key] = event;
  return json({ reads: operatorReads }, origin);
}

function streamResponse(
  origin: string | null,
  after: string | null = null,
): Response {
  let heartbeat: ReturnType<typeof setInterval> | null = null;
  let onClose: Sub | null = null;
  const stream = new ReadableStream({
    start(controller) {
      const enc = new TextEncoder();
      const send: Sub = (chunk) => {
        try {
          controller.enqueue(enc.encode(chunk));
        } catch {
          /* subscriber gone mid-flush */
        }
      };
      onClose = send;
      subs.add(send);
      // Prime the stream so the client's `open` fires promptly.
      send(": stream open\n\n");
      // A resume point the log no longer holds is a gap, and the client is
      // told so it can refetch rather than being handed live frames.
      const known = after
        ? events.some((event) => event.id === after)
        : true;
      if (!known) {
        send(resetFrame("cursor_expired"));
      } else if (after) {
        const index = events.findIndex((event) => event.id === after);
        const now = Date.now();
        const all = events as unknown as MockEvent[];
        const { views, answered, replies, closed, names } = feedContext(all, now);
        for (const event of events.slice(index + 1)) {
          send(`id: ${event.id}\nevent: bus\ndata: ${JSON.stringify(event)}\n\n`);
          const item = feedItem(
            event as unknown as MockEvent,
            views,
            roomNames(),
            answered,
            replies,
            closed,
            names,
          );
          send(`id: ${event.id}\nevent: item\ndata: ${JSON.stringify(item)}\n\n`);
        }
      }
      heartbeat = setInterval(() => send(": heartbeat\n\n"), 15_000);
    },
    cancel() {
      if (onClose) subs.delete(onClose);
      if (heartbeat) clearInterval(heartbeat);
    },
  });
  return new Response(stream, {
    status: 200,
    headers: {
      "content-type": "text/event-stream; charset=utf-8",
      "cache-control": "no-cache, no-transform",
      connection: "keep-alive",
      ...corsHeaders(origin),
    },
  });
}

/** `POST /api/v1/notes` (F2), mirroring the real `serve`'s refusals. */
async function handlePostNote(
  req: Request,
  origin: string | null,
): Promise<Response> {
  if (origin && !isLocalConsoleOrigin(origin)) {
    return json({ error: "forbidden origin" }, null, 403);
  }
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return json({ error: "invalid JSON body" }, origin, 400);
  }
  const record = (body ?? {}) as Record<string, unknown>;
  const text = typeof record.text === "string" ? record.text.trim() : "";
  if (!text) {
    return json({ error: "field `text` must not be empty" }, origin, 400);
  }
  if (new TextEncoder().encode(text).length > 2000) {
    return json({ error: "text exceeds the 2000-byte limit" }, origin, 422);
  }
  let to: BusEvent["to"] = null;
  if (record.to != null) {
    const t = record.to as Record<string, unknown>;
    const kind = typeof t.kind === "string" ? t.kind : "";
    const value = typeof t.value === "string" ? t.value.trim() : "";
    // Parity with the real serve: `channel` is a valid kind
    // there since F3; the mock rejecting it made channel posts untestable.
    if (!["harness", "session", "repo", "channel"].includes(kind) || !value) {
      return json(
        {
          error:
            "`to` must be {kind: harness|session|repo|channel, value: non-empty}",
        },
        origin,
        400,
      );
    }
    to = { kind: kind as NonNullable<BusEvent["to"]>["kind"], value };
  }
  // Parity with the real serve: an optional `kind`, refused when unknown.
  let noteKind: BusEvent["kind"];
  if (record.kind != null) {
    const k = record.kind;
    if (k !== "fyi" && k !== "warning" && k !== "ask" && k !== "handoff") {
      return json(
        {
          error: `unknown kind "${String(k)}"; expected fyi | warning | ask | handoff`,
        },
        origin,
        400,
      );
    }
    noteKind = k;
  }
  if ((noteKind === "warning" || noteKind === "ask") && !to) {
    return json(
      { error: "An interrupt or request needs a named recipient." },
      origin,
      400,
    );
  }
  if (noteKind === "handoff") {
    const target = to;
    const session =
      target?.kind === "session"
        ? computeSessions(Date.now()).find(
            (view) => view.session === target.value,
          )
        : null;
    if (
      !session ||
      session.liveness !== "live" ||
      session.harness === "human"
    ) {
      return json(
        { error: "Handoff needs exactly one live session recipient." },
        origin,
        400,
      );
    }
  }
  if (
    to?.kind === "channel" &&
    (fixtureName !== ROOM_FIXTURE ||
      to.value !== ROOM_NAME ||
      ROOM_MEMBERS.length === 0)
  ) {
    return json({ error: "This channel has no recipients." }, origin, 422);
  }
  let noteRef: string | undefined;
  if (record.ref != null) {
    if (
      typeof record.ref !== "string" ||
      !/^[0-9A-HJKMNP-TV-Z]{26}$/i.test(record.ref)
    ) {
      return json(
        {
          error: `ref "${String(record.ref)}" is not an event id (expected a 26-character ULID)`,
        },
        origin,
        400,
      );
    }
    const parent = events.find((e) => e.id === record.ref);
    if (parent && parent.event !== "note") {
      return json(
        {
          error: `ref "${record.ref}" points to a ${parent.event}, not a note`,
        },
        origin,
        400,
      );
    }
    noteRef = record.ref;
  }
  const ev = appendEvent({
    event: "note",
    session: "operator@night-watch.local",
    harness: "human",
    role: "operator",
    cwd: "",
    text,
    to,
    kind: noteKind,
    ...(noteRef ? { ref: noteRef } : {}),
  });
  return json({ event: ev }, origin, 201);
}

/** Room mutations needed by the console E2E contract. */
async function handlePostChannel(
  req: Request,
  origin: string | null,
): Promise<Response> {
  if (origin && !isLocalConsoleOrigin(origin)) {
    return json({ error: "forbidden origin" }, null, 403);
  }
  const body = (await req.json().catch(() => null)) as Record<
    string,
    unknown
  > | null;
  if (!body) return json({ error: "invalid JSON body" }, origin, 400);

  const action = body.action;
  if (isRoomAction(action)) {
    const answer = roomAction(action, body);
    return json(answer.body, origin, answer.status);
  }
  const name = typeof body.name === "string" ? body.name : "";
  if (fixtureName !== ROOM_FIXTURE || name !== ROOM_NAME) {
    return json({ error: `unknown Room "${name}"` }, origin, 404);
  }
  if (action !== "mark_read") {
    return json(
      { error: `unsupported mock Room action "${String(action)}"` },
      origin,
      400,
    );
  }
  const eventId = typeof body.event_id === "string" ? body.event_id : "";
  const note = events.find(
    (event) =>
      event.id === eventId &&
      event.event === "note" &&
      event.to?.kind === "channel" &&
      event.to.value === name,
  );
  if (!note) {
    return json({ error: "event_id is not a note in this Room" }, origin, 400);
  }
  const previous = channelReads[name];
  if (!previous || eventId > previous) channelReads[name] = eventId;
  return json(
    {
      ok: true,
      name,
      changed: previous !== channelReads[name],
      event_id: eventId,
    },
    origin,
  );
}

async function handleControl(
  req: Request,
  url: URL,
  path: string,
  origin: string | null,
): Promise<Response> {
  if (path === "/__control/ready") {
    return json({ ready: true, events: events.length }, origin);
  }
  if (req.method !== "POST") {
    return json({ error: "control endpoints are POST" }, origin, 405);
  }
  if (path === "/__control/reset") {
    const name = url.searchParams.get("scenario") ?? DEFAULT_FIXTURE;
    resetTo(name);
    resetRooms();
    return json({ ok: true, scenario: name, events: events.length }, origin);
  }
  if (path === "/__control/theme") {
    setMockTheme(url.searchParams);
    return json({ ok: true }, origin);
  }
  if (path === "/__control/append") {
    const body = (await req
      .json()
      .catch(() => ({}))) as Partial<FixtureEvent> & {
      event?: EventKind;
    };
    const ev = appendEvent({ event: body.event ?? "note", ...body });
    return json({ ok: true, id: ev.id }, origin);
  }
  if (path === "/__control/append-silent") {
    const body = (await req
      .json()
      .catch(() => ({}))) as Partial<FixtureEvent> & {
      event?: EventKind;
    };
    const ev = appendEvent({ event: body.event ?? "note", ...body }, false);
    return json({ ok: true, id: ev.id }, origin);
  }
  return json({ error: `unknown control path: ${path}` }, origin, 404);
}

process.on("SIGINT", () => server.stop(true));
process.on("SIGTERM", () => server.stop(true));

// Discovery line on stdout, like the real serve's `--json` startup line.
process.stdout.write(
  `${JSON.stringify({ port: PORT, pid: process.pid, store: "/mock/.harbard", fixture: DEFAULT_FIXTURE })}\n`,
);
