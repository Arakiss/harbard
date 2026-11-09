"use client";

import * as React from "react";
import { Check, Loader2, Lock, MessageCircle, Minus, X } from "lucide-react";
import { createT, intlTag } from "@/i18n";
import { fetchLive } from "@/lib/api";
import { createLivePoller, duration, liveHeading, liveItems, mergeLive, statusText, type LiveState, type LiveStep, type Prose } from "@/lib/live-agent";
import { stepKind, stepText, testSummary } from "@/lib/progress";
import { agentName, type WsAgent } from "@/lib/workspace";
import { useSecondTick } from "@/components/robot/status";
import { AgentAvatar, AgentStatus } from "./parts";
import { STEP_ICON } from "./step-icon";
import { useAgentLive } from "./workspace-context";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("agents");

/**
 * One agent's live read, polled every second while the page is visible and
 * this is mounted; nothing while hidden. Only the agents with an open card
 * or view are read. `prose: false` leaves the agent's words out (the card).
 */
export function useLiveAgent(
  key: string | null,
  { prose, only }: { prose: boolean; only?: "progress" },
): { state: LiveState | null; error: boolean; paused: boolean } {
  const [state, setState] = React.useState<LiveState | null>(null);
  const [error, setError] = React.useState(false);
  const [paused, setPaused] = React.useState(false);
  React.useEffect(() => {
    setState(null);
    if (!key) return;
    const poller = createLivePoller({
      fetch: (after, signal) => fetchLive(key, after, prose, signal, only),
      onDelta: (d) => {
        setError(false);
        setState((prev) => mergeLive(prev, d, Date.now()));
      },
      onError: () => setError(true),
      visible: () => document.visibilityState === "visible",
      onVisibility: (fn) => {
        const on = () => {
          setPaused(document.visibilityState !== "visible");
          fn();
        };
        document.addEventListener("visibilitychange", on);
        return () => document.removeEventListener("visibilitychange", on);
      },
      setTimer: (fn, ms) => window.setTimeout(fn, ms),
      clearTimer: (t) => window.clearTimeout(t as number),
    });
    setPaused(document.visibilityState !== "visible");
    poller.start();
    return () => poller.stop();
  }, [key, prose, only]);
  return { state, error, paused };
}

/** Inline code (`x`) and bold (**x**) in one line of prose; nothing else is markup. */
function inline(line: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  const re = /`([^`\n]+)`|\*\*([^*\n]+)\*\*/g;
  let last = 0;
  let i = 0;
  for (const m of line.matchAll(re)) {
    if (m.index > last) out.push(line.slice(last, m.index));
    out.push(m[1] !== undefined ? <code key={i++}>{m[1]}</code> : <strong key={i++}>{m[2]}</strong>);
    last = m.index + m[0].length;
  }
  if (last < line.length) out.push(line.slice(last));
  return out;
}

/** The agent's words as paragraphs, lists and headings; text only, never HTML. */
export function ProseBlock({ prose }: { prose: Pick<Prose, "text" | "ts_ms" | "n"> }) {
  const blocks = prose.text.split(/\n{2,}/).filter((b) => b.trim());
  return (
    <div className="hb-live-prose" data-prose={prose.n}>
      {blocks.map((b, i) => {
        const lines = b.split("\n");
        if (lines.every((l) => /^\s*([-*•]|\d+\.)\s+/.test(l))) {
          return (
            <ul key={i}>
              {lines.map((l, j) => (
                <li key={j}>{inline(l.replace(/^\s*([-*•]|\d+\.)\s+/, ""))}</li>
              ))}
            </ul>
          );
        }
        if (/^#{1,6}\s/.test(lines[0]!) && lines.length === 1) return <p key={i} className="hb-live-h">{inline(lines[0]!.replace(/^#+\s*/, ""))}</p>;
        return (
          <p key={i}>
            {lines.map((l, j) => (
              <React.Fragment key={j}>
                {j > 0 ? <br /> : null}
                {inline(l)}
              </React.Fragment>
            ))}
          </p>
        );
      })}
    </div>
  );
}

const hhmmss = (ms: number) =>
  new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(ms);

const MARK = { ok: Check, error: X, stopped: Minus } as const;

/** One tool call on the timeline: its mark on the line, icon, label, start, duration and status pill. */
export function TimelineRow({ step, now }: { step: LiveStep; now: number }) {
  const kind = stepKind(step);
  const Icon = STEP_ICON[kind];
  const took = (step.ended_ms ?? now) - step.started_ms;
  const Mark = step.status === "running" ? null : MARK[step.status];
  return (
    <li className="hb-tl-item hb-tl-step" data-status={step.status} data-kind={kind} data-n={step.n}>
      <span className="hb-tl-mark" data-status={step.status} aria-hidden>
        {Mark ? <Mark className="size-3.5" strokeWidth={3} aria-hidden /> : null}
      </span>
      <div className="hb-tl-card">
        <span className="hb-tl-ic" data-kind={kind} aria-hidden>
          <Icon className="size-[18px]" strokeWidth={2} aria-hidden />
        </span>
        <span className="hb-tl-label">
          {stepText(step)}
          {step.test ? <span className="hb-tl-test"> · {testSummary(step.test)}</span> : null}
        </span>
        <time className="hb-tl-start" dateTime={new Date(step.started_ms).toISOString()}>
          {hhmmss(step.started_ms)}
        </time>
        <span className="hb-tl-dur" suppressHydrationWarning>
          {duration(took)}
        </span>
        <span className="hb-tl-pill" data-status={step.status}>
          {step.status === "running" ? <Loader2 className="hb-tl-spin size-4" aria-hidden /> : null}
          {statusText(step.status)}
        </span>
      </div>
    </li>
  );
}

/**
 * "See it live": what one agent is doing, as it happens. Local and
 * private: the timeline of its tool calls and its own words, masked; no
 * thinking, no tool output.
 */
export function LiveView({ agent, onOpenChat }: { agent: WsAgent; onOpenChat: () => void }) {
  const { state, error, paused } = useLiveAgent(agent.key, { prose: true });
  const face = useAgentLive(agent).activity;
  const heading = liveHeading(face?.state, agent.presence);
  const name = agentName(agent);
  const offset = state ? state.serverNow - state.at : 0;
  const wall = useSecondTick(Date.now());
  const now = wall + offset;
  const items = React.useMemo(() => (state ? liveItems(state) : []), [state]);
  const scroller = React.useRef<HTMLOListElement>(null);
  const pinned = React.useRef(true);
  const lastRev = state?.rev ?? 0;
  React.useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lastRev]);
  return (
    <main
      className="hb-detail hb-live"
      aria-label={heading.live ? t("liveAria", { name }) : t("liveFinishedAria", { name })}
      data-paused={paused ? "true" : undefined}
      data-finished={heading.live ? undefined : "true"}
      data-live-agent={agent.key}
    >
      <header className="hb-live-head">
        <AgentAvatar name={agent.name} agent={agent} size={64} />
        <div className="min-w-0">
          <div className="hb-live-title">
            <span className="hb-live-name">{name}</span>
            <span className="hb-live-kind">{agent.kind === "teammate" ? t("kindAgent") : t("kindSubagent")}</span>
          </div>
          {agent.description || agent.brief ? <p className="hb-live-brief">{agent.description ?? agent.brief}</p> : null}
          <AgentStatus agent={agent} className="hb-live-status" />
        </div>
        <button type="button" className="hb-card-btn hb-live-chat" onClick={onOpenChat}>
          <MessageCircle className="size-[18px]" aria-hidden /> {t("openConversation")}
        </button>
      </header>
      <section className="hb-live-panel" aria-label={heading.text}>
        <p className="hb-live-private">
          <Lock className="size-4" aria-hidden /> {t("livePrivate")}
          {paused ? ` · ${t("livePaused")}` : error ? ` · ${t("liveOffline")}` : null}
        </p>
        <h2 className="hb-live-on" data-live={heading.live ? "true" : "false"}>
          {heading.live ? (
            <span className="hb-live-dot hb-live-dot-big" aria-hidden data-paused={paused || error ? "true" : undefined} />
          ) : (
            <span className="hb-live-done" aria-hidden>
              <Check className="size-4" strokeWidth={3} aria-hidden />
            </span>
          )}
          {heading.text}
        </h2>
        <ol
          className="hb-tl"
          ref={scroller}
          aria-live="polite"
          aria-relevant="additions"
          onScroll={(e) => {
            const el = e.currentTarget;
            pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          }}
        >
          {!state ? <li className="hb-empty">{t("liveReading", { name })}</li> : null}
          {state && items.length === 0 ? <li className="hb-empty">{t("liveNothingYet", { name })}</li> : null}
          {items.map((it) =>
            it.kind === "step" ? (
              <TimelineRow key={`s${it.step.n}`} step={it.step} now={now} />
            ) : (
              <li key={`p${it.prose.n}`} className="hb-tl-item hb-tl-said">
                <span className="hb-tl-said-av" aria-hidden>
                  <AgentAvatar name={agent.name} agent={agent} size={36} dot={false} />
                </span>
                <ProseBlock prose={it.prose} />
              </li>
            ),
          )}
        </ol>
        <p className="hb-live-foot">{t("liveFoot")}</p>
      </section>
    </main>
  );
}
