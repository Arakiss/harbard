/**
 * What an agent is doing, as serve derives it (`harbard_core::activity`),
 * and the little the console decides on its own: the microstate (how long a
 * state has lasted, from `since_ms` and the desk's thresholds), the face a
 * state shows, the status line's words, and `disconnected` when serve does
 * not answer. Nothing here invents a state serve did not send.
 */

import { createT, intlTag } from "@/i18n";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("activity");

export type ActivityState =
  | "normal"
  | "thinking"
  | "running"
  | "testing"
  | "editing"
  | "reading"
  | "web"
  | "delegating"
  | "waiting"
  | "asleep"
  | "blocked"
  | "finished"
  | "verified"
  | "disconnected";

export interface Activity {
  state: ActivityState;
  since_ms: number;
  /** A basename, a Grep pattern, a host, or `cargo test`; never more. */
  detail: string | null;
  source_ms: number;
}

export type ReactionKind = "glance" | "spark" | "hop" | "shake" | "boot";

export interface Reaction {
  kind: ReactionKind;
  at_ms: number;
}

export interface Thresholds {
  amber_ms: number;
  red_ms: number;
  thinking_long_ms: number;
  waiting_sleepy_ms: number;
  running_long_ms: number;
}

/** The faces the robots draw: twelve, one per observable state. */
export type Face =
  | "normal"
  | "thinking"
  | "running"
  | "reading"
  | "web"
  | "delegating"
  | "waiting"
  | "asleep"
  | "blocked"
  | "finished"
  | "verified"
  | "disconnected";

export const FACES: readonly Face[] = [
  "normal",
  "thinking",
  "running",
  "reading",
  "web",
  "delegating",
  "waiting",
  "asleep",
  "blocked",
  "finished",
  "verified",
  "disconnected",
];

/** A state with its microstate: the face to draw and the words to say. */
export interface Micro {
  face: Face;
  /** Waiting past the sleepy threshold: eyes half closed. */
  sleepy: boolean;
  /** A microstate that asks for attention (long run, too long asleep). */
  long: boolean;
  /** "Thinking a long while", or null when the state reads as itself. */
  label: string | null;
}

/**
 * The microstate: thinking for over 2 min looks
 * from side to side; waiting for over 5 min dozes; a run over 10 min is
 * flagged; asleep for over 45 min is flagged red. Thresholds come from the
 * desk (`thresholds`), the age from `since_ms`.
 */
export function microState(activity: Pick<Activity, "state" | "since_ms">, now: number, limits: Thresholds): Micro {
  const age = Math.max(0, now - activity.since_ms);
  const s = activity.state;
  const plain = (face: Face): Micro => ({ face, sleepy: false, long: false, label: null });
  switch (s) {
    case "thinking":
      return age >= limits.thinking_long_ms ? { face: "reading", sleepy: false, long: false, label: t("microThinkingLong") } : plain("thinking");
    case "waiting":
      if (age >= limits.amber_ms) return { face: "asleep", sleepy: false, long: false, label: t("microAsleepWaiting") };
      return age >= limits.waiting_sleepy_ms ? { face: "waiting", sleepy: true, long: false, label: t("microSleepyWaiting") } : plain("waiting");
    case "running":
    case "testing":
      return age >= limits.running_long_ms ? { face: "running", sleepy: false, long: true, label: t("microRunningLong") } : plain("running");
    case "editing":
      return plain("running");
    case "asleep":
      return age >= limits.red_ms ? { face: "asleep", sleepy: false, long: true, label: t("microAsleepLong") } : plain("asleep");
    default:
      return plain(s);
  }
}

/** "8 s", "2 min", "1 h", "1 h 5 min": the status line's timer. */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

function hhmm(ms: number): string {
  return new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), hour: "2-digit", minute: "2-digit" }).format(ms);
}

/** One part of a status line: plain words, or a label in code style. */
export type StatusPart = { text: string } | { code: string };

/**
 * The status line's words (agents.html, section 2): "Running `cargo
 * test`", "On the web · docs.rs", "Waiting in the background since 01:12".
 */
export function statusParts(
  activity: Pick<Activity, "state" | "since_ms" | "detail">,
  opts: { reports?: number; micro?: Micro | null } = {},
): StatusPart[] {
  const d = activity.detail;
  const code = (prefix: string): StatusPart[] => (d ? [{ text: `${prefix} ` }, { code: d }] : [{ text: prefix }]);
  const micro = opts.micro?.label ? ` · ${opts.micro.label.toLowerCase()}` : "";
  switch (activity.state) {
    case "thinking":
      return [{ text: opts.micro?.label ? `${opts.micro.label}…` : t("statusThinking") }];
    case "testing":
      return d ? [{ text: `${t("statusRunningTests")} · ` }, { code: d }, ...(micro ? [{ text: micro }] : [])] : [{ text: `${t("statusRunningTests")}${micro}` }];
    case "running":
      // serve names only known programs; anything else is "un comando".
      return [...(d ? code(t("statusRunning")) : [{ text: t("statusRunningCommand") }]), ...(micro ? [{ text: micro }] : [])];
    case "editing":
      return code(t("statusEditing"));
    case "reading":
      return code(t("statusReading"));
    case "web":
      return [{ text: d ? t("statusOnWeb", { host: d }) : t("statusSearchingWeb") }];
    case "delegating": {
      const n = Number(d ?? "1") || 1;
      return [{ text: n === 1 ? t("statusDelegatingOne", { count: n }) : t("statusDelegatingMany", { count: n }) }];
    }
    case "waiting":
      return [{ text: opts.micro?.sleepy ? t("statusWaitingSleepy", { time: hhmm(activity.since_ms) }) : t("statusWaiting", { time: hhmm(activity.since_ms) }) }];
    case "asleep":
      return [{ text: opts.micro?.long ? t("statusAsleepLong", { time: hhmm(activity.since_ms) }) : t("statusIdleSince", { time: hhmm(activity.since_ms) }) }];
    case "blocked":
      return [{ text: d ? t("statusBlockedDetail", { detail: d }) : t("statusBlocked") }];
    case "finished": {
      // Its lead stopped it (TaskStop): closed, never "No task".
      if (d === "closed") return [{ text: t("statusClosed") }];
      const r = opts.reports ?? 0;
      if (r === 0) return [{ text: t("statusFinished") }];
      return [{ text: r === 1 ? t("statusFinishedReportsOne", { count: r }) : t("statusFinishedReportsMany", { count: r }) }];
    }
    case "verified":
      return [{ text: t("statusVerified") }];
    case "disconnected":
      return [{ text: t("statusDisconnected") }];
    default:
      return [{ text: t("statusIdle") }];
  }
}

/** The status line as plain text, for titles and tests. */
export function statusText(activity: Pick<Activity, "state" | "since_ms" | "detail">, opts: Parameters<typeof statusParts>[1] = {}): string {
  return statusParts(activity, opts)
    .map((p) => ("code" in p ? p.code : p.text))
    .join("");
}

// ---------------------------------------------------------------------------
// Reactions: each plays once, as it happens, in every view that shows it.
// ---------------------------------------------------------------------------

/** How long one reaction runs, in ms. */
export const REACTION_MS: Record<ReactionKind, number> = {
  hop: 700,
  glance: 900,
  spark: 800,
  shake: 550,
  boot: 1600,
};

/** When several arrive together, the one to play. */
const REACTION_RANK: Record<ReactionKind, number> = { boot: 5, shake: 4, hop: 3, spark: 2, glance: 1 };

/** Instances mounted within this of the first play join it; later ones do not replay. */
export const REACTION_JOIN_MS = 1_500;

/**
 * The reaction to play now, if any: newer than the moment this page first
 * heard from serve (`baseline`), not already played, and the highest-ranked
 * of what is fresh. `played` maps `key:kind:at` to when it first played
 * (shared by every instance of the same agent on the page).
 */
export function nextReaction(
  key: string,
  reactions: readonly Reaction[],
  baseline: number,
  played: Map<string, number>,
  wallNow: number,
): { id: string; kind: ReactionKind } | null {
  const fresh = reactions
    .filter((r) => r.at_ms > baseline)
    .map((r) => ({ id: `${key}:${r.kind}:${r.at_ms}`, kind: r.kind }))
    .filter((r) => {
      const at = played.get(r.id);
      return at === undefined || wallNow - at < REACTION_JOIN_MS;
    })
    .sort((a, b) => REACTION_RANK[b.kind] - REACTION_RANK[a.kind]);
  const pick = fresh[0];
  if (!pick) return null;
  if (!played.has(pick.id)) played.set(pick.id, wallNow);
  // Lower-ranked reactions of the same moment are spent with it.
  for (const r of fresh.slice(1)) if (!played.has(r.id)) played.set(r.id, wallNow - REACTION_JOIN_MS);
  return pick;
}

// ---------------------------------------------------------------------------
// The living logo
// ---------------------------------------------------------------------------

/**
 * Mark every fresh reaction of `key` as already played without playing it:
 * under Reduce motion a reaction is skipped, and turning motion back on
 * within two minutes must not replay it.
 */
export function spendReactions(key: string, reactions: readonly Reaction[], baseline: number, played: Map<string, number>, wallNow: number): void {
  for (const r of reactions) {
    if (r.at_ms <= baseline) continue;
    const id = `${key}:${r.kind}:${r.at_ms}`;
    if (!played.has(id)) played.set(id, wallNow - REACTION_JOIN_MS);
  }
}

export type LogoState = "idle" | "working" | "needs" | "blocked" | "connecting" | "disconnected";
export type LogoOneShot = "message" | "verified";

/** Any agent active within this makes the logo "working". */
const WORKING_MS = 60_000;

/** States in which an agent is at work even while its transcript is quiet (a long test run). */
export const WORKING_STATES: ReadonlySet<ActivityState> = new Set(["thinking", "running", "testing", "editing", "reading", "web", "delegating"]);

/**
 * The logo's lasting state, by precedence: no serve, still connecting, an
 * open BLOCKED, anything else in "Needs you", an agent active in the last
 * minute or in a working state now, else rest.
 */
export function logoState(input: {
  connection: "connecting" | "connected" | "disconnected";
  blocked: number;
  needs: number;
  lastActions: readonly (number | null)[];
  /** Agents whose face is a working state (WORKING_STATES). */
  working: number;
  now: number;
}): LogoState {
  if (input.connection === "disconnected") return "disconnected";
  if (input.connection === "connecting") return "connecting";
  if (input.blocked > 0) return "blocked";
  if (input.needs > 0) return "needs";
  if (input.working > 0 || input.lastActions.some((ms) => ms !== null && input.now - ms <= WORKING_MS)) return "working";
  return "idle";
}
