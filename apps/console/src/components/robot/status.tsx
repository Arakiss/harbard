"use client";

import * as React from "react";
import { elapsed, statusParts, statusText, type Activity, type ActivityState, type Micro } from "@/lib/activity";

/** The status line's icon per state (agents.html), drawn in tokens. */
export function StatusIcon({ state }: { state: ActivityState }) {
  const svg = (children: React.ReactNode) => (
    <svg className="hb-ic" viewBox="0 0 14 14" aria-hidden focusable={false} data-icon={state}>
      {children}
    </svg>
  );
  switch (state) {
    case "thinking":
      return svg(
        <>
          <circle className="ic-dot" cx={3} cy={7} r={1.5} />
          <circle className="ic-dot" cx={7} cy={7} r={1.5} />
          <circle className="ic-dot" cx={11} cy={7} r={1.5} />
        </>,
      );
    case "running":
      return svg(
        <>
          <rect className="ic-term" x={0.5} y={1.5} width={13} height={11} rx={2.5} />
          <path className="ic-prompt" d="M3.5 5l2 2-2 2" strokeWidth={1.4} strokeLinecap="round" />
          <rect className="ic-caret" x={7} y={8.2} width={3.5} height={1.3} />
        </>,
      );
    case "testing":
      return svg(
        <>
          <rect className="ic-track" x={0.5} y={5} width={13} height={4} rx={2} />
          <rect className="ic-seg" x={4} y={5} width={6} height={4} rx={2} />
        </>,
      );
    case "editing":
      return svg(
        <>
          <path className="ic-ink" d="M1 12.5h9" strokeWidth={1.4} strokeLinecap="round" />
          <g className="ic-pen">
            <path d="M8.8 1.8l2.4 2.4-5.6 5.6-3 .6.6-3z" />
          </g>
        </>,
      );
    case "reading":
      return svg(
        <>
          <path className="ic-lines" d="M1 3h12M1 6h9M1 9h11" strokeWidth={1.3} strokeLinecap="round" />
          <g className="ic-lens">
            <circle cx={7} cy={7} r={3.2} strokeWidth={1.4} />
            <path d="M9.3 9.3l2.6 2.6" strokeWidth={1.6} strokeLinecap="round" />
          </g>
        </>,
      );
    case "web":
      return svg(
        <>
          <circle className="ic-web" cx={7} cy={7} r={5.8} strokeWidth={1.3} />
          <ellipse className="ic-web ic-mer" cx={7} cy={7} rx={2.6} ry={5.8} strokeWidth={1.2} />
          <path className="ic-web" d="M1.5 7h11" strokeWidth={1.1} />
        </>,
      );
    case "delegating":
      return svg(
        <>
          <circle className="ic-core" cx={7} cy={7} r={2.3} />
          <g className="ic-orb">
            <circle cx={7} cy={1.6} r={1.5} />
            <circle cx={7} cy={12.4} r={1.5} />
          </g>
        </>,
      );
    case "waiting":
      return svg(
        <g className="ic-glass">
          <path
            d="M3.5 1.5h7M3.5 12.5h7M4.5 1.5c0 3.2 5 3.8 5 5.5s-5 2.3-5 5.5M9.5 1.5c0 3.2-5 3.8-5 5.5s5 2.3 5 5.5"
            strokeWidth={1.3}
            fill="none"
            strokeLinecap="round"
          />
          <path className="ic-sand" d="M5.6 11.6h2.8L7 9.6z" />
        </g>,
      );
    case "asleep":
      return svg(
        <>
          <text className="ic-z" x={2} y={12}>
            z
          </text>
          <text className="ic-z" x={7} y={8}>
            z
          </text>
        </>,
      );
    case "blocked":
      return svg(
        <g className="ic-stop">
          <path d="M4.6 1h4.8L13 4.6v4.8L9.4 13H4.6L1 9.4V4.6z" />
          <rect className="ic-bar" x={4} y={6.2} width={6} height={1.6} rx={0.8} />
        </g>,
      );
    case "finished":
    case "verified":
      return svg(
        <>
          <circle className={state === "verified" ? "ic-ok" : "ic-done"} cx={7} cy={7} r={6} />
          <path className="ic-tick" d="M4.3 7.2l1.9 1.9 3.6-3.8" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" />
        </>,
      );
    case "disconnected":
      return svg(<circle className="ic-off" cx={7} cy={7} r={5} strokeWidth={1.6} />);
    default:
      return null;
  }
}

/* One second clock shared by every status line on the page; it stops while
   nobody listens and while the tab is hidden. */
let listeners = new Set<() => void>();
let ticker: ReturnType<typeof setInterval> | undefined;
let wall = 0;

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  if (!ticker) {
    ticker = setInterval(() => {
      if (document.visibilityState !== "visible") return;
      wall = Date.now();
      for (const l of listeners) l();
    }, 1000);
  }
  return () => {
    listeners.delete(fn);
    if (listeners.size === 0 && ticker) {
      clearInterval(ticker);
      ticker = undefined;
      listeners = new Set();
    }
  };
}

/** Wall-clock time, ticking once a second; `fallback` on the server. */
export function useSecondTick(fallback: number): number {
  return React.useSyncExternalStore(
    subscribe,
    () => wall || fallback,
    () => fallback,
  );
}

/**
 * "Running `cargo test` · 2 min": the icon, the words and a timer that
 * ticks in the browser. `offset` turns wall time into serve's clock.
 */
export function StatusLine({
  activity,
  micro,
  reports,
  serverNow,
  offset,
  showTime = true,
  className,
}: {
  activity: Pick<Activity, "state" | "since_ms" | "detail">;
  micro?: Micro | null;
  reports?: number;
  /** serve's clock at render (SSR and first paint). */
  serverNow: number;
  /** serve's clock minus the browser's, to tick from wall time. */
  offset: number;
  showTime?: boolean;
  className?: string;
}) {
  const wallNow = useSecondTick(serverNow - offset);
  const now = wallNow + offset;
  const parts = statusParts(activity, { reports, micro });
  return (
    <span className={`hb-status ${className ?? ""}`} data-state={activity.state} data-long={micro?.long ? "true" : undefined} title={statusText(activity, { reports, micro })}>
      <StatusIcon state={activity.state} />
      <span className="hb-status-text">
        {parts.map((p, i) => ("code" in p ? <code key={i}>{p.code}</code> : <React.Fragment key={i}>{p.text}</React.Fragment>))}
      </span>
      {showTime && activity.state !== "disconnected" ? (
        <span className="hb-status-time" suppressHydrationWarning>
          {elapsed(now - activity.since_ms)}
        </span>
      ) : null}
    </span>
  );
}
