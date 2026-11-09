/**
 * The living logo's state, shown by the desktop app on its Dock icon, Dock
 * badge and menu-bar icon (`desk_set_status`, apps/desktop/src-tauri/src/dock.rs).
 *
 * Call it where the living logo's state changes. In a browser tab it does
 * nothing; in the desktop shell it sends only what changed, so a render that
 * repeats the same state costs no IPC and the Dock is redrawn once per change.
 */

/** The living logo's lasting states (`LogoState` in activity.ts). */
export const DOCK_STATES = ["idle", "working", "needs", "blocked", "connecting", "disconnected"] as const;
export type DockState = (typeof DOCK_STATES)[number];

/** One-shot animations (`LogoOneShot` in activity.ts). */
export const DOCK_EVENTS = ["message", "verified"] as const;
export type DockEvent = (typeof DOCK_EVENTS)[number];

export type DesktopStatus = {
  state: DockState;
  /** Open "Needs you" items: what waits for the operator, nothing else. */
  badge: number;
  /** Open "Needs you" items alone: a rise rings the Dock bounce. */
  needs?: number;
  event?: DockEvent;
};

/**
 * The badge count the Dock shows: what waits for the operator. Unread agent
 * messages stay in Direct: counted here they reached 88 beside 2 real asks.
 */
export function dockBadge(needs: number): number {
  return Number.isFinite(needs) && needs > 0 ? Math.min(9999, Math.floor(needs)) : 0;
}

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
type Host = {
  __TAURI__?: { core?: { invoke?: Invoke } };
  __TAURI_INTERNALS__?: { invoke?: Invoke };
};

function hostInvoke(host: Host | undefined): Invoke | null {
  const invoke = host?.__TAURI__?.core?.invoke ?? host?.__TAURI_INTERNALS__?.invoke;
  return typeof invoke === "function" ? invoke : null;
}

/** Refuse what the native side would refuse, before it crosses IPC. */
export function validDesktopStatus(value: unknown): value is DesktopStatus {
  if (!value || typeof value !== "object") return false;
  const s = value as Record<string, unknown>;
  if (Object.keys(s).some((k) => !["state", "badge", "needs", "event"].includes(k))) return false;
  const count = (v: unknown) => typeof v === "number" && Number.isInteger(v);
  return (
    typeof s.state === "string" && (DOCK_STATES as readonly string[]).includes(s.state) &&
    count(s.badge) &&
    (s.needs === undefined || count(s.needs)) &&
    (s.event === undefined || (typeof s.event === "string" && (DOCK_EVENTS as readonly string[]).includes(s.event)))
  );
}

export type DesktopStatusSink = (status: DesktopStatus) => Promise<"sent" | "unchanged" | "browser" | "failed">;

/**
 * How often a live console repeats its last state. The desktop shows the
 * outage ring after 60 s without a report (`HEARTBEAT` in dock.rs), so a
 * console that stops (its page gone, its server down) is not mistaken for
 * one whose counts are still current. Native drops identical reports
 * without redrawing anything.
 */
export const KEEPALIVE_MS = 20_000;

type Schedule = (fn: () => void, ms: number) => () => void;
const defaultSchedule: Schedule = (fn, ms) => {
  const id = setTimeout(fn, ms);
  return () => clearTimeout(id);
};

/**
 * A sink bound to one host. A call that repeats the state the Dock is known
 * to show (the last one that succeeded, with nothing newer in flight) is
 * not sent. A call with an event is always sent: an event is a happening,
 * not a state, and the native side already limits animations to one every
 * 3 s. A failed call is forgotten, so the next identical call is sent.
 * Every call re-arms the keepalive, which repeats the last state.
 */
export function desktopStatusSink(
  host: Host | undefined = typeof window === "undefined" ? undefined : (window as unknown as Host),
  opts: { keepaliveMs?: number; schedule?: Schedule } = {},
): DesktopStatusSink {
  const keepaliveMs = opts.keepaliveMs ?? KEEPALIVE_MS;
  const schedule = opts.schedule ?? defaultSchedule;
  let shown: string | null = null;
  let issued = 0;
  let inFlight = 0;
  let cancel: (() => void) | null = null;
  let lastState: Record<string, unknown> | null = null;

  const arm = (invoke: Invoke) => {
    cancel?.();
    cancel = schedule(() => {
      cancel = null;
      if (!lastState) return;
      const again = lastState;
      void invoke("desk_set_status", again).catch(() => {}).finally(() => arm(invoke));
    }, keepaliveMs);
  };

  return async (status) => {
    if (!validDesktopStatus(status)) throw new Error("Invalid desktop status.");
    const invoke = hostInvoke(host);
    if (!invoke) return "browser";
    const args: Record<string, unknown> = { state: status.state, badge: status.badge };
    if (status.needs !== undefined) args.needs = status.needs;
    const key = JSON.stringify(args);
    lastState = { ...args };
    arm(invoke);
    if (status.event === undefined && inFlight === 0 && key === shown) return "unchanged";
    if (status.event !== undefined) args.event = status.event;
    const seq = ++issued;
    inFlight += 1;
    try {
      await invoke("desk_set_status", args);
      if (seq === issued) shown = key;
      return "sent";
    } catch {
      // An older desktop build has no such command: the Dock stays static.
      if (seq === issued) shown = null;
      return "failed";
    } finally {
      inFlight -= 1;
    }
  };
}

let shared: DesktopStatusSink | null = null;

/** The page's one sink. A no-op in the browser. */
export function setDesktopStatus(status: DesktopStatus): ReturnType<DesktopStatusSink> {
  shared ??= desktopStatusSink();
  return shared(status);
}

/** What the living logo shows, as the call site sees it on each render. */
export type LogoReport = {
  state: DockState;
  /** Open "Needs you" items and unread Direct, from the faces' slice. */
  needs: number;
  directs: number;
  /** The one-shot on screen, and a counter that moves for each new one. */
  once: DockEvent | null;
  onceSeq: number;
};

/**
 * Turns the living logo's renders into Dock reports: the badge is needs
 * plus directs, and a one-shot becomes an event exactly once, when its
 * counter moves. The sink deduplicates repeats and keeps the heartbeat.
 */
export function desktopStatusDriver(sink: DesktopStatusSink): (logo: LogoReport) => ReturnType<DesktopStatusSink> {
  let seenSeq = 0;
  return (logo) => {
    const fresh = logo.once !== null && logo.onceSeq !== seenSeq;
    seenSeq = logo.onceSeq;
    const needs = Number.isFinite(logo.needs) && logo.needs > 0 ? Math.floor(logo.needs) : 0;
    const status: DesktopStatus = { state: logo.state, badge: dockBadge(needs), needs };
    if (fresh && logo.once) status.event = logo.once;
    return sink(status);
  };
}

let sharedDriver: ReturnType<typeof desktopStatusDriver> | null = null;

/** The page's one driver, over the page's one sink. A no-op in the browser. */
export function reportLogo(logo: LogoReport): ReturnType<DesktopStatusSink> {
  sharedDriver ??= desktopStatusDriver((status) => setDesktopStatus(status));
  return sharedDriver(logo);
}
