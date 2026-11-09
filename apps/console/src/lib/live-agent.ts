/**
 * One agent, step by step: the hover card's "Últimos pasos" and "Progreso",
 * and the private live view ("See it live"), as serve folds them
 * (`harbard_core::live`, `GET /api/v1/live/<agent>`).
 *
 * Everything here is the operator's own local data and nothing is posted anywhere.
 * serve sends safe labels (a basename, a host, a program and its
 * subcommand), the counts of a test run and never its output, and the
 * agent's prose with every credential-looking word masked. No thinking,
 * no tool output.
 */

import { createT, intlTag } from "@/i18n";
import type { Progress, StepRow } from "./progress";

export type { Progress, StepRow } from "./progress";

const t = createT("agents");

export interface LiveStep extends StepRow {
  n: number;
  rev: number;
}

export interface Prose {
  n: number;
  rev: number;
  ts_ms: number;
  text: string;
}

export interface LiveDelta {
  v: number;
  key: string;
  now_ms: number;
  rev: number;
  reset: boolean;
  first_ms: number | null;
  last_ms: number | null;
  steps: LiveStep[];
  prose: Prose[];
  progress: Progress;
}

/** What a client holds for one open agent. */
export interface LiveState {
  key: string;
  rev: number;
  /** serve's clock at the newest answer, and the browser's then. */
  serverNow: number;
  at: number;
  first_ms: number | null;
  last_ms: number | null;
  steps: LiveStep[];
  prose: Prose[];
  progress: Progress;
}

/** The open view asks serve this often while visible: a change shows within 2 s. */
export const LIVE_POLL_MS = 1_000;
/**
 * What the view's heading says: "En direct" while the agent works,
 * "Finished" once its turn ended with nothing left to do (its face is
 * finished or verified, or its presence is finished). A finished agent
 * is never shown as live.
 */
export function liveHeading(state: string | null | undefined, presence: string | null | undefined): { text: string; live: boolean } {
  const finished = state === "finished" || state === "verified" || presence === "finished";
  return finished ? { text: t("liveHeadingFinished"), live: false } : { text: t("liveHeadingLive"), live: true };
}

/** The longest wait between two asks after failures. */
export const LIVE_BACKOFF_MAX_MS = 30_000;
/** Rows a client keeps, like serve. */
const STEPS_KEPT = 400;
const PROSE_KEPT = 150;

function byN<T extends { n: number }>(old: T[], fresh: T[], cap: number): T[] {
  if (fresh.length === 0) return old;
  const map = new Map(old.map((x) => [x.n, x]));
  for (const x of fresh) map.set(x.n, x);
  const out = [...map.values()].sort((a, b) => a.n - b.n);
  return out.length > cap ? out.slice(out.length - cap) : out;
}

/** Fold one answer into what the client holds. A reset starts over. */
export function mergeLive(prev: LiveState | null, d: LiveDelta, at: number): LiveState {
  const base = !prev || d.reset || prev.key !== d.key ? null : prev;
  return {
    key: d.key,
    rev: d.rev,
    serverNow: d.now_ms,
    at,
    first_ms: d.first_ms,
    last_ms: d.last_ms,
    steps: byN(base?.steps ?? [], d.steps, STEPS_KEPT),
    prose: byN(base?.prose ?? [], d.prose, PROSE_KEPT),
    progress: d.progress,
  };
}

/** One row of the live view: a tool call or a paragraph group, in time order. */
export type LiveItem = { kind: "step"; at: number; step: LiveStep } | { kind: "prose"; at: number; prose: Prose };

export function liveItems(s: Pick<LiveState, "steps" | "prose">): LiveItem[] {
  const items: LiveItem[] = [
    ...s.steps.map((step) => ({ kind: "step" as const, at: step.started_ms, step })),
    ...s.prose.map((prose) => ({ kind: "prose" as const, at: prose.ts_ms, prose })),
  ];
  // Prose before the calls it announces when both carry the same instant.
  return items.sort((a, b) => a.at - b.at || (a.kind === b.kind ? 0 : a.kind === "prose" ? -1 : 1));
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

export function statusText(status: StepRow["status"]): string {
  switch (status) {
    case "running":
      return t("statusRunning");
    case "ok":
      return t("statusOk");
    case "error":
      return t("statusError");
    case "stopped":
      return t("statusStopped");
  }
}

/** "0,4 s", "12 s", "3 min 5 s", "1 h 2 min": how long a step took or has run. */
export function duration(ms: number): string {
  const v = Math.max(0, ms);
  if (v < 10_000) return `${(Math.round(v / 100) / 10).toLocaleString(intlTag(), { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`;
  const s = Math.floor(v / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 ? `${m} min ${s % 60} s` : `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}

// ---------------------------------------------------------------------------
// The poller
// ---------------------------------------------------------------------------

export interface PollerDeps {
  fetch: (after: number | null, signal: AbortSignal) => Promise<LiveDelta>;
  onDelta: (d: LiveDelta) => void;
  onError?: (e: unknown) => void;
  visible: () => boolean;
  onVisibility: (fn: () => void) => () => void;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (t: unknown) => void;
  every?: number;
}

/**
 * Ask serve for one agent's changes every second while the page is
 * visible. Hidden, it asks nothing and holds no timer; visible again, it
 * asks at once. `stop` aborts what is in flight.
 */
export function createLivePoller(deps: PollerDeps): { start: () => void; stop: () => void; paused: () => boolean } {
  const every = deps.every ?? LIVE_POLL_MS;
  let timer: unknown = null;
  let controller: AbortController | null = null;
  let after: number | null = null;
  let alive = false;
  let busy = false;
  let failures = 0;
  let off: (() => void) | null = null;
  const clear = () => {
    if (timer !== null) deps.clearTimer(timer);
    timer = null;
  };
  const schedule = () => {
    clear();
    // After a failure (serve down, an agent gone) it asks less and less often.
    const delay = failures > 0 ? Math.min(LIVE_BACKOFF_MAX_MS, every * 2 ** failures) : every;
    if (alive && deps.visible()) timer = deps.setTimer(tick, delay);
  };
  async function tick() {
    timer = null;
    if (!alive || !deps.visible() || busy) return;
    busy = true;
    controller = new AbortController();
    try {
      const d = await deps.fetch(after, controller.signal);
      if (!alive) return;
      after = d.rev;
      failures = 0;
      deps.onDelta(d);
    } catch (e) {
      if (alive && !(e instanceof DOMException && e.name === "AbortError")) {
        failures = Math.min(failures + 1, 8);
        deps.onError?.(e);
      }
    } finally {
      busy = false;
      controller = null;
    }
    schedule();
  }
  return {
    start() {
      if (alive) return;
      alive = true;
      off = deps.onVisibility(() => {
        if (!alive) return;
        if (deps.visible()) {
          if (timer === null && !busy) void tick();
        } else {
          clear();
          controller?.abort();
        }
      });
      if (deps.visible()) void tick();
    },
    stop() {
      alive = false;
      clear();
      controller?.abort();
      off?.();
      off = null;
    },
    paused: () => !alive || !deps.visible(),
  };
}
