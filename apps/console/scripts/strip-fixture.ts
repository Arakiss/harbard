/**
 * Synthetic workspace views for the strip: a typical shelf, a crowded one
 * and an empty one. No real transcript text.
 *
 * Times are relative to `now`; SKETCH_NOW fixes it for stable renders.
 */
import type { Activity, ActivityState } from "../src/lib/activity";
import type { ActivityView, WsAgent, WsCard, WsMessage, WorkspaceView } from "../src/lib/workspace";

export const SKETCH_NOW = Date.UTC(2026, 10, 7, 14, 52, 0);
const S = 1000;
const MIN = 60 * S;
const HOUR = 60 * MIN;

/** 14:52 → the ms of `hh:mm` on the same UTC day as `now`. */
function at(now: number, hh: number, mm: number): number {
  const d = new Date(now);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), hh, mm);
}

function agent(key: string, name: string, state: ActivityState, since: number, last: number, extra: Partial<WsAgent> = {}, detail: string | null = null): WsAgent {
  const activity: Activity = { state, since_ms: since, detail, source_ms: last };
  return {
    key,
    name,
    label: `general-purpose · ${key}`,
    lost: false,
    kind: "teammate",
    lead_session: "lead-1",
    lead_quiet_ms: 0,
    delivery: "ready",
    state: "running",
    state_reason: "mid_turn",
    presence: "working",
    dormant: null,
    last_action_ms: last,
    first_ms: last - HOUR,
    brief: "Synthetic brief",
    description: "Synthetic task",
    checkout: "/work/harbard",
    branch: "main",
    build_dir: null,
    channels: ["harbard"],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: false,
    activity,
    reactions: [],
    ...extra,
  };
}

function message(id: string, agentKey: string, author: string, ts: number, text: string, extra: Partial<WsMessage> = {}): WsMessage {
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
    channels: ["harbard"],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    ...extra,
  };
}

function base(now: number, agents: WsAgent[], messages: WsMessage[], cards: WsCard[]): WorkspaceView {
  return {
    v: 1,
    now_ms: now,
    operator: "Dana",
    operator_initials: "DR",
    thresholds: { amber_ms: 15 * MIN, red_ms: 45 * MIN, thinking_long_ms: 2 * MIN, waiting_sleepy_ms: 5 * MIN, running_long_ms: 10 * MIN },
    channels: [{ id: "harbard", repo: "/work/harbard", checkouts: [], members: agents.map((a) => a.key), last_ms: now, warnings: [] }],
    agents,
    messages,
    cards,
    deliveries: [],
    saved: [],
    unread: { directs: 0, activity: cards.length },
    team_root_found: true,
    format_changed: false,
    motion: null,
    night_dim: true,
  };
}

const DONE: [string, string, number, number, boolean][] = [
  ["holdout", "DONE · harbard · Two viewer reports, reviewed", 14, 41, true],
  ["ledger", "DONE · contoso · The ledger balances with the bank", 13, 27, true],
  ["recall", "DONE · orchard · Recall 40% faster", 12, 8, true],
  ["identity", "DONE · orchard · Operator identity in the keychain", 10, 36, true],
  ["parser", "DONE · harbard · The new composer, without reloads", 9, 50, false],
  ["mascot", "DONE · orchard · The mascot in three sizes", 9, 14, false],
];

/** A typical shelf: two cards, seven robots in seven states, six deliveries and four verified. */
export function sketchView(now = SKETCH_NOW): WorkspaceView {
  const agents = [
    agent("a01", "identity", "thinking", now - 8 * S, now - 8 * S),
    agent("a02", "ledger", "testing", now - 2 * MIN - 4 * S, now - 20 * S, {}, "cargo test"),
    agent("a03", "parser", "editing", now - 31 * S, now - 31 * S, {}, "workspace.css"),
    agent("a04", "recall", "reading", now - 44 * S, now - 44 * S, {}, "team.rs"),
    agent("a05", "ingest", "delegating", now - 4 * MIN, now - 70 * S, {}, "2"),
    agent("a06", "mascot", "asleep", at(now, 14, 20), at(now, 14, 20), { presence: "working", dormant: "amber" }),
    agent("a07", "holdout", "finished", at(now, 14, 41), at(now, 14, 41), { presence: "finished", state: "finished", reports: ["/tmp/viewer-1.md", "/tmp/viewer-2.md"] }),
    // In "Needs you", not on the shelf: their last action is over two hours old.
    agent("a08", "keychain-review", "blocked", at(now, 12, 31), at(now, 12, 31), { presence: "waiting", state: "blocked" }, "Can it ask for the password twice?"),
    agent("a09", "acme-runner", "asleep", at(now, 1, 12), at(now, 1, 12), { presence: "working", dormant: "red" }),
  ];
  const name = (key: string) => agents.find((a) => a.key === key)!.name;
  const keyOf = (n: string) => agents.find((a) => a.name === n)!.key;
  const messages: WsMessage[] = [
    message("m-blocked", "a08", "keychain-review", at(now, 12, 31), "BLOCKED · keychain-review · Can it ask for the password twice?", { label: "BLOCKED" }),
    ...DONE.map(([who, text, hh, mm, ok], i) =>
      message(`m-done-${i}`, keyOf(who), name(keyOf(who)), at(now, hh, mm), text, {
        label: "DONE",
        verified: ok ? { sha: `abc${i}def`, branch: "main", decided_ms: at(now, hh, mm) + MIN } : null,
      }),
    ),
  ];
  const cards: WsCard[] = [
    { id: "c-blocked", kind: "blocked", channel: "harbard", agents: ["a08"], message: "m-blocked", checkout: null, since_ms: at(now, 12, 31), resolved_ms: null, needs: true },
    { id: "c-dormant", kind: "dormant", channel: "acme", agents: ["a09"], message: null, checkout: null, since_ms: at(now, 1, 12), resolved_ms: null, needs: true },
  ];
  return base(now, agents, messages, cards);
}

const CROWD: [string, ActivityState, string | null][] = [
  ["identity", "thinking", null],
  ["ledger", "testing", "cargo test"],
  ["parser", "editing", "workspace.css"],
  ["recall", "reading", "team.rs"],
  ["mind-backup", "web", "docs.rs"],
  ["ingest", "delegating", "2"],
  ["kc-review-2", "running", "bun run build"],
  ["orchard-dx", "thinking", null],
  ["harbard-dock", "editing", "dock.tsx"],
  ["alerts-dashboard", "reading", "alerts.rs"],
  ["port-ledger", "testing", "bun test"],
];

/** Eleven agents working at once: eight robots and "+3". Every robot animates. */
export function crowdView(now = Date.now()): WorkspaceView {
  const agents = CROWD.map(([n, state, detail], i) => agent(`b${String(i).padStart(2, "0")}`, n, state, now - (20 + i * 13) * S, now - (5 + i * 7) * S, {}, detail));
  return base(now, agents, [], []);
}

/** Nobody active in the last two hours: the latest robots asleep. */
export function emptyView(now = SKETCH_NOW): WorkspaceView {
  const agents = ["identity", "harbard-themes", "parser", "recall", "harbard-view"].map((n, i) =>
    agent(`e${i}`, n, "finished", now - (3 + i) * HOUR, now - (3 + i) * HOUR, { presence: "finished", state: "finished" }),
  );
  return base(now, agents, [], []);
}

/** The faces' slice of a view, as serve's `?only=activity` answers. */
export function activitySlice(view: WorkspaceView): ActivityView {
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
  };
}
