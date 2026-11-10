/**
 * The `readme` scenario: the workspace the README's screenshots and demo are
 * taken from (scripts/readme-media.ts). An invented web shop, `storefront`,
 * with its checkout service and its docs, and nine robots working on them.
 * Synthetic data only: no real person, company, host or path.
 *
 * The view is a function of a clock and a step. The clock starts at a fixed
 * moment (`README_ANCHOR`) on every reset and then runs in real time, so the
 * pictures always show the same afternoon. The step moves the demo forward
 * (`POST /__control/readme?step=N`): a finding arrives, the robots change what
 * they are doing, one gets blocked on a question, a delivery lands.
 */
import type { Activity, ActivityState, Reaction } from "../src/lib/activity";
import type { WsShipment } from "../src/lib/deliveries";
import type { LobbyEntry, LobbyHarness } from "../src/lib/rooms";
import type { ActivityView, WorkspaceView, WsAgent, WsCard, WsChannel, WsMessage } from "../src/lib/workspace";

/** Wednesday 14 October 2026, 15:12 UTC: an ordinary working afternoon. */
export const README_ANCHOR = Date.UTC(2026, 9, 14, 15, 12, 0);
/** The last step of the demo. */
export const README_LAST_STEP = 5;

const S = 1000;
const MIN = 60 * S;
const HOUR = 60 * MIN;

const state = {
  anchor: README_ANCHOR,
  startedAt: Date.now(),
  step: 0,
  /** When each step was reached, on the scenario's clock. */
  stepAt: [README_ANCHOR] as number[],
  replies: [] as WsMessage[],
};

/** The scenario's clock: the anchor, plus the real time since the reset. */
export function readmeNow(): number {
  return state.anchor + (Date.now() - state.startedAt);
}

/** Back to the first step, at the anchor (or at `anchor`, an epoch in ms). */
export function resetReadme(anchor: number = README_ANCHOR): void {
  state.anchor = anchor;
  state.startedAt = Date.now();
  state.step = 0;
  state.stepAt = [anchor];
  state.replies = [];
}

/** `POST /__control/readme?step=N[&anchor=ms]`. */
export function readmeControl(params: URLSearchParams): { step: number; now_ms: number } {
  const anchor = Number(params.get("anchor"));
  if (params.has("anchor") && Number.isFinite(anchor) && anchor > 0) resetReadme(anchor);
  const step = Number(params.get("step"));
  if (params.has("step") && Number.isInteger(step) && step >= 0 && step <= README_LAST_STEP) {
    // One step at a time happens now; a jump spreads the skipped steps over
    // the last minutes, so a still does not show them all at the same time.
    const now = readmeNow();
    for (let s = state.step + 1; s <= step; s += 1) state.stepAt[s] = now - (step - s) * 4 * MIN;
    state.step = step;
  }
  return { step: state.step, now_ms: readmeNow() };
}

/** A reply the operator sends from the console: kept, so the next read shows it. */
export function readmeReply(body: Record<string, unknown>, author: string): void {
  if (typeof body.text !== "string" || !body.text.trim()) return;
  const channel = typeof body.channel === "string" ? body.channel : null;
  const agent = typeof body.agent === "string" ? body.agent : null;
  const ts = readmeNow();
  state.replies.push({
    ...message(`op-${state.replies.length}`, agent ?? "operator", author, ts, body.text.trim(), channel ?? "storefront"),
    from: "operator",
    reply_to: typeof body.reply_to === "string" ? body.reply_to : null,
    mentions: body.text.match(/@[a-z0-9-]+/g) ?? [],
    delivery: { state: "delivered", sessions: ["lead-storefront"], delivered: 1, to_lead: true, nudged_ms: null },
  });
}

// ---------------------------------------------------------------------------
// The cast
// ---------------------------------------------------------------------------

interface Robot {
  key: string;
  name: string;
  harness: LobbyHarness;
  channel: string;
  description: string;
  branch: string;
}

const ROBOTS: Robot[] = [
  { key: "r01", name: "cart", harness: "claude-code", channel: "storefront", description: "Move the cart drawer to server actions", branch: "feat/cart-actions" },
  { key: "r02", name: "tax-rules", harness: "codex", channel: "checkout-api", description: "Refresh the VAT table and its tests", branch: "fix/vat-2026" },
  { key: "r03", name: "search", harness: "cursor", channel: "storefront", description: "Find why sold-out sizes show in search", branch: "main" },
  { key: "r04", name: "reviewer", harness: "claude-code", channel: "checkout-api", description: "Review what lands on main today", branch: "main" },
  { key: "r05", name: "lighthouse", harness: "codex", channel: "storefront", description: "Make the product page load faster", branch: "perf/fonts" },
  { key: "r06", name: "migrations", harness: "claude-code", channel: "storefront", description: "Add the index the order history needs", branch: "feat/orders-index" },
  { key: "r07", name: "i18n", harness: "claude-code", channel: "storefront", description: "Check prices and copy in every locale", branch: "main" },
  { key: "r08", name: "changelog", harness: "codex", channel: "docs", description: "Write Friday's release notes", branch: "docs/release-notes" },
  { key: "r09", name: "images", harness: "cursor", channel: "storefront", description: "Serve product photos as AVIF", branch: "perf/avif" },
];

const robot = (key: string): Robot => ROBOTS.find((r) => r.key === key)!;

/** What each robot is doing, per step: state, detail, and how long ago it started. */
type Doing = [ActivityState, string | null, number];

function doing(key: string, step: number): Doing {
  const table: Record<string, Doing[]> = {
    r01: [["editing", "CartDrawer.tsx", 40 * S], ["editing", "CartDrawer.tsx", 40 * S], ["testing", "bun test", 4 * S], ["testing", "bun test", 9 * S], ["running", "bun run build", 3 * S], ["running", "bun run build", 6 * S]],
    r02: [["testing", "go test ./...", 70 * S], ["testing", "go test ./...", 75 * S], ["editing", "vat_rates.go", 5 * S], ["editing", "vat_rates.go", 9 * S], ["editing", "vat_rates.go", 14 * S], ["thinking", null, 3 * S]],
    r03: [["reading", "search-index.ts", 25 * S], ["reading", "search-index.ts", 28 * S], ["reading", "inventory.ts", 3 * S], ["reading", "inventory.ts", 8 * S], ["editing", "webhook.ts", 4 * S], ["editing", "webhook.ts", 8 * S]],
    r04: [["thinking", null, 12 * S], ["thinking", null, 15 * S], ["reading", "CartDrawer.tsx", 3 * S], ["reading", "CartDrawer.tsx", 7 * S], ["reading", "fonts.css", 3 * S], ["reading", "fonts.css", 6 * S]],
    r05: [["editing", "fonts.css", 50 * S], ["editing", "fonts.css", 55 * S], ["running", "bun run build", 4 * S], ["testing", "lighthouse", 5 * S], ["verified", null, 2 * S], ["verified", null, 5 * S]],
    r06: [["running", "db:migrate --dry-run", 90 * S], ["running", "db:migrate --dry-run", 95 * S], ["thinking", null, 6 * S], ["blocked", "May I lock the orders table for 40 seconds?", 2 * S], ["blocked", "May I lock the orders table for 40 seconds?", 5 * S], ["editing", "0042_orders_index.sql", 3 * S]],
    r07: [["web", "unicode.org", 20 * S], ["thinking", null, 2 * S], ["editing", "price.ts", 5 * S], ["editing", "price.ts", 9 * S], ["testing", "bun test", 4 * S], ["testing", "bun test", 8 * S]],
    r08: [["finished", null, 38 * MIN], ["finished", null, 38 * MIN], ["finished", null, 38 * MIN], ["finished", null, 38 * MIN], ["finished", null, 38 * MIN], ["finished", null, 38 * MIN]],
    r09: [["asleep", null, 52 * MIN], ["asleep", null, 52 * MIN], ["asleep", null, 52 * MIN], ["asleep", null, 52 * MIN], ["asleep", null, 52 * MIN], ["asleep", null, 52 * MIN]],
  };
  const rows = table[key]!;
  return rows[Math.min(step, rows.length - 1)]!;
}

function agentAt(r: Robot, now: number, step: number): WsAgent {
  const [activityState, detail, ago] = doing(r.key, step);
  // A state reached at this step started when the step did, plus what has passed since.
  const since = Math.min(now - ago, now);
  const activity: Activity = { state: activityState, since_ms: since, detail, source_ms: since };
  const finished = activityState === "finished";
  const asleep = activityState === "asleep";
  const blocked = activityState === "blocked";
  const reactions: Reaction[] = [];
  if (r.key === "r05" && step >= 4) reactions.push({ kind: "spark", at_ms: state.stepAt[4] ?? now });
  if (r.key === "r06" && step >= 3 && step < 5) reactions.push({ kind: "shake", at_ms: state.stepAt[3] ?? now });
  if (r.key === "r07" && step >= 1) reactions.push({ kind: "glance", at_ms: state.stepAt[1] ?? now });
  return {
    key: r.key,
    name: r.name,
    label: `teammate · ${r.key}`,
    lost: false,
    kind: "teammate",
    lead_session: `lead-${r.channel}`,
    lead_quiet_ms: 0,
    delivery: finished ? "ended" : "ready",
    state: finished ? "finished" : blocked ? "blocked" : "running",
    state_reason: finished ? "finished" : blocked ? "asked" : "mid_turn",
    presence: finished ? "finished" : blocked ? "waiting" : "working",
    dormant: asleep ? "amber" : null,
    last_action_ms: since,
    first_ms: now - 3 * HOUR + Number(r.key.slice(1)) * 7 * MIN,
    brief: r.description,
    description: r.description,
    checkout: `/work/${r.channel}`,
    branch: r.branch,
    build_dir: null,
    channels: [r.channel],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: !finished,
    activity,
    reactions,
    handle: r.name,
    role_line: null,
  };
}

// ---------------------------------------------------------------------------
// The conversation
// ---------------------------------------------------------------------------

function message(id: string, agentKey: string, author: string, ts: number, text: string, channel: string, extra: Partial<WsMessage> = {}): WsMessage {
  return {
    id,
    agent: agentKey,
    from: "agent",
    author,
    ts_ms: ts,
    text,
    label: null,
    version: null,
    flags: [],
    repeat_of: null,
    chars: text.length,
    files: [],
    verified: null,
    channels: [channel],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    ...extra,
  };
}

/** `hh:mm` UTC on the scenario's day. */
function at(now: number, hh: number, mm: number): number {
  const d = new Date(now);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), hh, mm);
}

const sha = (n: number) => `${(0x3c9e7a1 + n * 0x10f3).toString(16)}${"0".repeat(33)}`.slice(0, 40);

function said(key: string, id: string, ts: number, text: string, extra: Partial<WsMessage> = {}): WsMessage {
  const r = robot(key);
  const label = (["START", "FINDING", "BLOCKED", "DONE"] as const).find((l) => text.startsWith(`${l} · `)) ?? null;
  return message(id, key, r.name, ts, text, r.channel, { label, ...extra });
}

function conversation(now: number, step: number): WsMessage[] {
  const verified = (n: number, ts: number) => ({ sha: sha(n), branch: "main", decided_ms: ts + 3 * MIN });
  const out: WsMessage[] = [
    said("r08", "m-docs-1", at(now, 11, 20), "START · changelog · Collecting what changed this week for Friday's notes."),
    said("r02", "m-api-1", at(now, 12, 2), "FINDING · tax-rules · The VAT table still has last year's rate for Ireland; one checkout test hides it."),
    said("r01", "m-sf-1", at(now, 12, 58), "START · cart · Moving the cart drawer to server actions so the total updates without a reload."),
    said("r03", "m-sf-2", at(now, 13, 5), "FINDING · search · Sold-out sizes still show in search: the index is only rebuilt at night.", {
      body: "search-index.ts rebuilds from the nightly export, so stock that sells out during the day stays searchable. A webhook from inventory would close the gap.",
    }),
    {
      ...message("m-sf-3", "r03", "Dana", at(now, 13, 9), "@search write it up as a ticket, and leave the index alone until Friday's release.", "storefront"),
      from: "operator",
      mentions: ["@search"],
      replies: ["m-sf-4"],
      delivery: { state: "answered", sessions: ["lead-storefront"], delivered: 1, to_lead: true, nudged_ms: null, answered_by: null },
    },
    said("r03", "m-sf-4", at(now, 13, 12), "Ticket written. The index stays as it is; I am drafting the webhook on a branch.", { reply_to: "m-sf-3" }),
    said("r02", "m-api-2", at(now, 13, 24), "DONE · tax-rules · Every EU rate matches the 2026 table, and the test now fails on a stale one.", { verified: verified(1, at(now, 13, 24)) }),
    said("r05", "m-sf-5", at(now, 13, 31), "FINDING · lighthouse · The product page downloads 1.2 MB of fonts, and two of the weights are never used."),
    said("r01", "m-sf-6", at(now, 13, 48), "DONE · cart · The cart drawer updates its total without a reload.", { verified: verified(2, at(now, 13, 48)), files: ["src/cart/CartDrawer.tsx"] }),
    said("r04", "m-api-3", at(now, 14, 2), "Reviewed tax-rules and cart: no fixes needed, both can land.", { channels: ["checkout-api"] }),
    said("r08", "m-docs-2", at(now, 14, 34), "DONE · changelog · Friday's release notes, with one line per change a customer can see.", { verified: verified(3, at(now, 14, 34)) }),
    said("r05", "m-sf-7", at(now, 14, 40), "START · lighthouse · Dropping the unused font weights and preloading the one the hero image uses."),
    said("r06", "m-sf-8", at(now, 14, 51), "START · migrations · Adding an index on orders by customer, so order history stops timing out."),
  ];
  const stepped = (n: number) => state.stepAt[n] ?? now;
  if (step >= 1) out.push(said("r07", "m-sf-9", stepped(1), "FINDING · i18n · German prices are rounded before VAT instead of after, so some totals are a cent off."));
  if (step >= 3) {
    out.push(
      said("r06", "m-sf-10", stepped(3), "BLOCKED · migrations · May I lock the orders table for about 40 seconds to build the index, or should it wait until tonight?"),
    );
  }
  if (step >= 4) {
    out.push(
      said("r05", "m-sf-11", stepped(4), "DONE · lighthouse · The product page loads 840 KB less and shows its first image 0.6 s sooner.", {
        verified: verified(4, stepped(4)),
        files: ["src/styles/fonts.css"],
      }),
    );
  }
  return out;
}

function shipments(messages: WsMessage[]): WsShipment[] {
  const order: Record<string, WsShipment["state"]> = { "m-sf-11": "delivered", "m-docs-2": "in_review", "m-sf-6": "integrated", "m-api-2": "integrated" };
  return messages
    .filter((m) => m.label === "DONE")
    .map((m, i): WsShipment => {
      const shipState = order[m.id] ?? "delivered";
      const review: WsShipment["review"] =
        shipState === "integrated" || shipState === "in_review"
          ? [{ agent: "r04", message: m.id, ts_ms: m.ts_ms + 9 * MIN, kind: shipState === "in_review" ? "reviewing" : "reviewed" }]
          : [];
      return {
        id: m.id,
        agent: m.agent,
        ts_ms: m.ts_ms,
        text: m.text.replace(/^DONE\s*·\s*[^·]+·\s*/, ""),
        channel: m.channels[0] ?? null,
        sha: sha(i + 10).slice(0, 7),
        commit: null,
        state: shipState,
        last_state: "integrated",
        pending: shipState !== "integrated",
        untracked: null,
        review,
      };
    })
    .sort((a, b) => b.ts_ms - a.ts_ms);
}

function channel(id: string, now: number, keys: string[], unread = 0): WsChannel {
  return {
    id,
    repo: `/work/${id}`,
    checkouts: [{ path: `/work/${id}`, branch: "main" }],
    members: keys,
    last_ms: now,
    warnings: [],
    reach: "ready",
    active_ms: now,
    quiet: false,
    unread,
    recent: keys,
    kind: "project",
    purpose: null,
    archived: false,
    built_in: false,
  };
}

function room(id: string, now: number, purpose: string, keys: string[]): WsChannel {
  return { id, repo: "", checkouts: [], members: keys, last_ms: now - 20 * MIN, warnings: [], kind: "room", purpose, archived: false, built_in: false, reach: "ready", recent: [] };
}

const INTROS: Record<string, string> = {
  r01: "I keep the cart honest: what you see is what you pay.",
  r02: "Tax tables, rounding rules and the tests that catch both.",
  r03: "If it is in stock, you will find it. If it is not, you should not.",
  r05: "Every kilobyte has to earn its place on the product page.",
  r06: "Schema changes, one careful step at a time.",
};

function lobby(agents: WsAgent[], now: number): LobbyEntry[] {
  return agents
    .map((a): LobbyEntry => {
      const r = robot(a.key);
      return {
        key: a.key,
        name: a.name,
        harness: r.harness,
        project: r.channel,
        intro: INTROS[a.key] ?? null,
        intro_ts_ms: INTROS[a.key] ? (a.first_ms ?? now) + 2 * MIN : 0,
        presence: a.presence === "finished" ? "ended" : a.dormant ? "stale" : a.presence === "working" ? "live" : "idle",
        first_ms: a.first_ms ?? now - HOUR,
        last_ms: a.last_action_ms ?? now,
      };
    })
    .sort((a, b) => b.last_ms - a.last_ms);
}

/** The whole view at the current step. */
export function readmeView(): WorkspaceView {
  const now = readmeNow();
  const step = state.step;
  const agents = ROBOTS.map((r) => agentAt(r, now, step));
  const messages = [...conversation(now, step), ...state.replies].sort((a, b) => a.ts_ms - b.ts_ms);
  const cards: WsCard[] = [
    { id: "c-images", kind: "dormant", channel: "storefront", agents: ["r09"], message: null, checkout: null, since_ms: now - 52 * MIN, resolved_ms: null, needs: true },
  ];
  if (step >= 3 && step < 5) {
    cards.unshift({ id: "c-migrations", kind: "blocked", channel: "storefront", agents: ["r06"], message: "m-sf-10", checkout: null, since_ms: state.stepAt[3] ?? now, resolved_ms: null, needs: true });
  }
  const storefront = ROBOTS.filter((r) => r.channel === "storefront").map((r) => r.key);
  const view: WorkspaceView = {
    v: 1,
    now_ms: now,
    operator: "Dana",
    operator_initials: "DR",
    thresholds: { amber_ms: 15 * MIN, red_ms: 45 * MIN, thinking_long_ms: 2 * MIN, waiting_sleepy_ms: 5 * MIN, running_long_ms: 10 * MIN },
    channels: [
      channel("storefront", now, storefront),
      channel("checkout-api", now, ["r02", "r04"], 2),
      channel("docs", now, ["r08"]),
      room("release", now, "Friday's release, step by step", ["r04", "r08"]),
    ],
    agents,
    messages,
    cards,
    deliveries: [],
    saved: [],
    unread: { directs: 0, activity: cards.length },
    team_root_found: true,
    format_changed: false,
    motion: null,
    night_dim: false,
    rev: `readme-${step}-${state.replies.length}`,
    lobby: lobby(agents, now),
  };
  view.shipments = shipments(messages);
  return view;
}

/** The faces' slice of the view, with the `rev` that makes the console pull the rest. */
export function readmeActivity(view: WorkspaceView): ActivityView {
  return {
    v: view.v,
    now_ms: view.now_ms,
    thresholds: view.thresholds,
    agents: view.agents.map((a) => ({ key: a.key, presence: a.presence, dormant: a.dormant, last_action_ms: a.last_action_ms, activity: a.activity, reactions: a.reactions })),
    unread: view.unread,
    needs: view.cards.filter((c) => c.needs && c.resolved_ms === null).length,
    directs: view.unread.directs,
    blocked: view.cards.filter((c) => c.kind === "blocked" && c.needs && c.resolved_ms === null).length,
    motion: view.motion,
    rev: view.rev,
  };
}
