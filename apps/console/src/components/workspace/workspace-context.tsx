"use client";

import * as React from "react";
import { WorkspaceRefused, fetchActivity, fetchWorkspace, postWorkspace } from "@/lib/api";
import type { Activity, Reaction, Thresholds } from "@/lib/activity";
import { desktopViewBridge } from "@/lib/desktop-view";
import { VISIBLE_ACTIVITY_MS, activityPoll } from "@/lib/poll-cadence";
import { shouldPull, type ActivityView, type PostWorkspace, type WsAgent, type WorkspaceView } from "@/lib/workspace";
import { ALL_WORKSPACES, WORKSPACE_CHANGE_EVENT, readScope } from "@/lib/current-workspace";

/**
 * A pull answered for another workspace than the one chosen now: the
 * operator switched while it was in flight. Dropped, never shown.
 */
function staleFor(answer: { workspace?: string }): boolean {
  const chosen = readScope();
  return chosen !== null && chosen !== ALL_WORKSPACES && answer.workspace !== undefined && answer.workspace !== chosen;
}

/** Pull cadence of the full view while the shell is open and visible. Nothing pulls it otherwise. */
const POLL_MS = 8_000;
/** The faces' slice is pulled this often while visible, so a change shows within 3 s. */
export const ACTIVITY_POLL_MS = VISIBLE_ACTIVITY_MS;

type State =
  | { kind: "loading" }
  | { kind: "ready"; view: WorkspaceView; at: number; error: string | null }
  | { kind: "failed"; error: string };

interface Ctx {
  state: State;
  /** serve's clock, carried forward by the time since the pull. */
  now: number;
  refresh: () => void;
  post: PostWorkspace;
}

const WorkspaceContext = React.createContext<Ctx | null>(null);

/** What the robots and the logo read: the newest faces, live. */
export interface LiveCtx {
  /** The newest per-agent slice, from whichever pull is fresher. */
  agents: Map<string, { activity: Activity; reactions: Reaction[]; last_action_ms: number | null }>;
  thresholds: Thresholds | null;
  /** serve's clock at the newest pull. */
  serverNow: number;
  /** serve's clock minus the browser's. */
  offset: number;
  /** serve's clock when this page first heard from it: older reactions are history. */
  baseline: number;
  connection: "connecting" | "connected" | "disconnected";
  blocked: number;
  /** Open items in "Needs you". */
  needs: number;
  /** Unread Direct; with `needs`, the desktop's Dock badge. */
  directs: number;
  /** The toggle in Preferences (the OS setting is read by CSS and by `useReducedMotion`). */
  motion: "reduce" | null;
  setMotion: (reduce: boolean) => Promise<void>;
}

const LiveContext = React.createContext<LiveCtx | null>(null);

/**
 * `hiddenDesktopPoll`: keep pulling the faces' slice while the desktop
 * shell's window is hidden, because this page reports to the Dock. Only the
 * main shell sets it; the strip, in the second window, does not.
 */
export function WorkspaceProvider({
  children,
  initial,
  hiddenDesktopPoll = false,
}: {
  children: React.ReactNode;
  initial?: WorkspaceView;
  hiddenDesktopPoll?: boolean;
}) {
  // Server and client first render the same instant: serve's own clock.
  const [state, setState] = React.useState<State>(() =>
    initial ? { kind: "ready", view: initial, at: initial.now_ms, error: null } : { kind: "loading" },
  );
  const [clientNow, setClientNow] = React.useState(() => initial?.now_ms ?? 0);
  const tickRef = React.useRef<() => void>(() => {});

  React.useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;
    const tick = async () => {
      clearTimeout(timer);
      if (document.visibilityState !== "visible") {
        timer = setTimeout(tick, POLL_MS);
        return;
      }
      controller?.abort();
      controller = new AbortController();
      try {
        const view = await fetchWorkspace(controller.signal);
        if (alive && !staleFor(view)) setState({ kind: "ready", view, at: Date.now(), error: null });
      } catch (error) {
        if (!alive || (error instanceof DOMException && error.name === "AbortError")) return;
        const message = error instanceof Error ? error.message : String(error);
        setState((prev) => (prev.kind === "ready" ? { ...prev, error: message } : { kind: "failed", error: message }));
      }
      if (alive) timer = setTimeout(tick, POLL_MS);
    };
    tickRef.current = () => void tick();
    setClientNow(Date.now());
    if (initial) timer = setTimeout(tick, POLL_MS);
    else void tick();
    const onVisible = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    // A switch reads the new workspace at once.
    const onSwitch = () => void tick();
    window.addEventListener(WORKSPACE_CHANGE_EVENT, onSwitch);
    const clock = setInterval(() => setClientNow(Date.now()), 30_000);
    return () => {
      alive = false;
      clearTimeout(timer);
      clearInterval(clock);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener(WORKSPACE_CHANGE_EVENT, onSwitch);
    };
    // The initial view is read once, on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The faces' slice: a light pull every two seconds while visible, and
  // every ten while the desktop shell's window is hidden (`activityPoll`).
  const [pulse, setPulse] = React.useState<{ view: ActivityView; at: number } | null>(null);
  const [pulseError, setPulseError] = React.useState(false);
  const [baseline, setBaseline] = React.useState<number>(() => initial?.now_ms ?? Number.POSITIVE_INFINITY);
  const [motionOverride, setMotionOverride] = React.useState<"reduce" | null | undefined>(undefined);
  React.useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;
    const desktop = hiddenDesktopPoll && desktopViewBridge().kind === "desktop";
    const tick = async () => {
      clearTimeout(timer);
      const cadence = activityPoll(document.visibilityState === "visible", desktop);
      if (cadence.fetch) {
        controller?.abort();
        controller = new AbortController();
        try {
          const view = await fetchActivity(controller.signal);
          if (!alive || staleFor(view)) return;
          setPulse({ view, at: Date.now() });
          setPulseError(false);
          setBaseline((b) => (Number.isFinite(b) ? b : view.now_ms));
        } catch (error) {
          if (!alive || (error instanceof DOMException && error.name === "AbortError")) return;
          setPulseError(true);
        }
      }
      if (alive) timer = setTimeout(tick, cadence.delay);
    };
    timer = setTimeout(tick, initial ? ACTIVITY_POLL_MS : 0);
    const onVisible = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    // The faces of the workspace left behind are not this one's.
    const onSwitch = () => {
      setPulse(null);
      void tick();
    };
    window.addEventListener(WORKSPACE_CHANGE_EVENT, onSwitch);
    return () => {
      alive = false;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener(WORKSPACE_CHANGE_EVENT, onSwitch);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  React.useEffect(() => {
    if (state.kind === "ready") setBaseline((b) => (Number.isFinite(b) ? b : state.view.now_ms));
  }, [state]);

  const refresh = React.useCallback(() => tickRef.current(), []);
  // A new message, answer or delivery state shows in the two-second slice's
  // `rev` first: pull the full view then, not at its own eight-second pace.
  const pulledFor = React.useRef<string | undefined>(undefined);
  React.useEffect(() => {
    if (state.kind !== "ready" || !pulse) return;
    const rev = pulse.view.rev;
    if (shouldPull(state.view.rev, rev) && pulledFor.current !== rev) {
      pulledFor.current = rev;
      refresh();
    }
  }, [pulse, state, refresh]);
  const post = React.useCallback<PostWorkspace>(
    async (action, body) => {
      const out = await postWorkspace(action, body);
      if (action !== "open") refresh();
      return out;
    },
    [refresh],
  );
  const now = state.kind === "ready" ? state.view.now_ms + Math.max(0, clientNow - state.at) : clientNow || Date.now();
  const value = React.useMemo(() => ({ state, now, refresh, post }), [state, now, refresh, post]);

  const setMotion = React.useCallback(
    async (reduce: boolean) => {
      setMotionOverride(reduce ? "reduce" : null);
      try {
        await postWorkspace("motion", { reduce });
      } finally {
        refresh();
      }
    },
    [refresh],
  );
  const live = React.useMemo<LiveCtx>(() => {
    const view = state.kind === "ready" ? state.view : null;
    const usePulse = pulse !== null && (!view || pulse.view.now_ms >= view.now_ms);
    const agents: LiveCtx["agents"] = new Map();
    for (const a of view?.agents ?? []) agents.set(a.key, { activity: a.activity, reactions: a.reactions, last_action_ms: a.last_action_ms });
    if (usePulse) for (const a of pulse.view.agents) agents.set(a.key, { activity: a.activity, reactions: a.reactions, last_action_ms: a.last_action_ms });
    const serverNow = usePulse ? pulse.view.now_ms : (view?.now_ms ?? 0);
    const at = usePulse ? pulse.at : state.kind === "ready" ? state.at : Date.now();
    const failed = state.kind === "failed" || (state.kind === "ready" && state.error !== null) || pulseError;
    const blocked = usePulse ? pulse.view.blocked : (view?.cards.filter((c) => c.kind === "blocked" && c.needs && c.resolved_ms === null).length ?? 0);
    const needs = usePulse ? pulse.view.needs : (view?.unread.activity ?? 0);
    const directs = usePulse ? pulse.view.directs : (view?.unread.directs ?? 0);
    const motion = motionOverride !== undefined ? motionOverride : usePulse ? pulse.view.motion : (view?.motion ?? null);
    return {
      agents,
      thresholds: usePulse ? pulse.view.thresholds : (view?.thresholds ?? null),
      serverNow,
      offset: serverNow && at ? serverNow - at : 0,
      baseline,
      connection: failed ? "disconnected" : state.kind === "loading" ? "connecting" : "connected",
      blocked,
      needs,
      directs,
      motion,
      setMotion,
    };
  }, [state, pulse, pulseError, baseline, motionOverride, setMotion]);
  // Once serve has the new choice, it is the truth again.
  React.useEffect(() => {
    if (motionOverride !== undefined && pulse && pulse.view.motion === motionOverride) setMotionOverride(undefined);
  }, [pulse, motionOverride]);
  return (
    <WorkspaceContext.Provider value={value}>
      <LiveContext.Provider value={live}>{children}</LiveContext.Provider>
    </WorkspaceContext.Provider>
  );
}

/** The live faces, or null outside the shell. */
export function useLive(): LiveCtx | null {
  return React.useContext(LiveContext);
}

/** The newest face of `agent`: the two-second slice when fresher than the view. */
export function useAgentLive(agent: Pick<WsAgent, "key" | "activity" | "reactions"> | null | undefined): {
  activity: Activity | null;
  reactions: Reaction[];
} {
  const live = useLive();
  if (!agent) return { activity: null, reactions: [] };
  const hit = live?.agents.get(agent.key);
  const activity = live?.connection === "disconnected" ? { ...(hit?.activity ?? agent.activity), state: "disconnected" as const } : (hit?.activity ?? agent.activity);
  return { activity, reactions: hit?.reactions ?? agent.reactions ?? [] };
}

export function useWorkspace(): Ctx {
  const ctx = React.useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace outside WorkspaceProvider");
  return ctx;
}

/** Null beside a tool of More tools, which loads no workspace (the rail is shared). */
export function useOptionalWorkspace(): Ctx | null {
  return React.useContext(WorkspaceContext);
}

/** The loaded view, or null outside the shell (the views under "More"). */
export function useOptionalView(): WorkspaceView | null {
  const ctx = React.useContext(WorkspaceContext);
  return ctx && ctx.state.kind === "ready" ? ctx.state.view : null;
}

/** The loaded view, or null while loading or failed. */
export function useView(): WorkspaceView | null {
  const { state } = useWorkspace();
  return state.kind === "ready" ? state.view : null;
}

export { WorkspaceRefused };
