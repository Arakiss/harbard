"use client";

import type { Route } from "next";
import * as React from "react";
import { FeedbackRow } from "@/components/feedback/feedback";
import { createPortal } from "react-dom";
import { createT } from "@/i18n";
import { useRouter } from "next/navigation";
import { Ban, CheckCircle2, Code2, Coffee, EyeOff, FileText, Globe, Hourglass, MessageCircle, Moon, SquareTerminal, Users, WifiOff } from "lucide-react";
import { statusParts, type Activity, type ActivityState, type Micro, microState } from "@/lib/activity";
import { cardModel, createHoverIntent, placeCard, type HoverIntent } from "@/lib/agent-card";
import { clock, relative, stepKind, stepText, testSummary, type Progress, type StepRow } from "@/lib/progress";
import type { WsAgent } from "@/lib/workspace";
import { useSecondTick } from "@/components/robot/status";
import { useLiveAgent } from "./live-pane";
import { AgentAvatar } from "./parts";
import { STEP_ICON, type Icon } from "./step-icon";
import { useAgentLive, useLive, useOptionalWorkspace } from "./workspace-context";
import { hideAction } from "@/lib/hidden";

const t = createT("agents");

const CARD_WIDTH = 440;

/** The icon for what the agent is doing now. */
function nowIcon(state: ActivityState): Icon {
  switch (state) {
    case "running":
    case "testing":
      return SquareTerminal;
    case "editing":
      return Code2;
    case "reading":
      return FileText;
    case "web":
      return Globe;
    case "delegating":
      return Users;
    case "waiting":
      return Hourglass;
    case "asleep":
      return Moon;
    case "blocked":
      return Ban;
    case "finished":
    case "verified":
      return CheckCircle2;
    case "disconnected":
      return WifiOff;
    default:
      return Coffee;
  }
}

/** One step: icon, words, and when ("en curso" while it runs). */
function StepLine({ step, now }: { step: StepRow; now: number }) {
  const kind = stepKind(step);
  const Icon = STEP_ICON[kind];
  return (
    <li className="hb-step" data-status={step.status} data-kind={kind}>
      <span className="hb-step-ic" data-kind={kind} aria-hidden>
        <Icon className="size-4" strokeWidth={2} aria-hidden />
      </span>
      <span className="hb-step-text">
        {stepText(step)}
        {step.test ? <span className="hb-step-test"> · {testSummary(step.test)}</span> : null}
        {step.status === "error" ? <span className="hb-step-flag"> · {t("stepHadError")}</span> : null}
        {step.status === "stopped" ? <span className="hb-step-test"> · {t("statusStopped")}</span> : null}
      </span>
      <span className="hb-step-time" suppressHydrationWarning>
        {step.status === "running" ? t("statusRunning") : relative(step.ended_ms ?? step.started_ms, now)}
      </span>
    </li>
  );
}

/** "Pasando testing · cargo test · 1 min 12 s", ticking. */
function NowLine({ activity, micro, reports, now }: { activity: Activity; micro: Micro | null; reports: number; now: number }) {
  const Icon = nowIcon(activity.state);
  const words = statusParts(activity, { reports, micro })
    .map((p) => ("code" in p ? p.code : p.text))
    .join("");
  return (
    <div className="hb-card-now" data-state={activity.state} data-long={micro?.long ? "true" : undefined}>
      <span className="hb-card-now-ic" aria-hidden>
        <Icon className="size-[18px]" strokeWidth={2} aria-hidden />
      </span>
      <span className="hb-card-now-text">
        <strong>{words}</strong>
        {activity.state !== "disconnected" ? (
          <span className="hb-card-now-time" suppressHydrationWarning>
            {" "}
            · {clock(now - activity.since_ms)}
          </span>
        ) : null}
      </span>
    </div>
  );
}

/**
 * What the card shows, from plain props (rendered on its own in tests).
 * `avatar` is the large robot; `actions` the buttons under it.
 */
export function AgentCardView({
  id,
  agent,
  activity,
  micro,
  serverNow,
  offset,
  progress,
  avatar,
  onOpenChat,
  firstButton,
  extraActions,
}: {
  id?: string;
  agent: WsAgent;
  activity: Activity | null;
  micro: Micro | null;
  serverNow: number;
  offset: number;
  progress: Progress | null;
  avatar?: React.ReactNode;
  onOpenChat?: () => void;
  firstButton?: React.Ref<HTMLButtonElement>;
  extraActions?: React.ReactNode;
}) {
  const wall = useSecondTick(serverNow - offset);
  const now = wall + offset;
  const m = cardModel(agent, progress, now);
  const steps = progress?.steps ?? [];
  return (
    <div className="hb-card" id={id} role="dialog" aria-label={t("cardAria", { name: m.name })} data-agent-card={agent.key}>
      <div className="hb-card-head">
        <span className="hb-card-avatar">{avatar}</span>
        <div className="min-w-0">
          <div className="hb-card-name">{m.name}</div>
          <div className="hb-card-pills">
            {m.teams.map((t) => (
              <span key={t} className="hb-card-pill" data-kind="team">
                {t}
              </span>
            ))}
            {m.role ? (
              <span className="hb-card-pill" data-kind="role">
                {m.role}
              </span>
            ) : null}
            <span className="hb-card-pill" data-kind="presence" data-tone={m.presence.tone}>
              <span className="hb-card-pill-dot" aria-hidden />
              {m.presence.text}
            </span>
          </div>
          {m.brief ? <p className="hb-card-brief">{m.brief}</p> : null}
        </div>
      </div>
      <section className="hb-card-sec" aria-label={t("now")}>
        <h3 className="hb-card-h">{t("now")}</h3>
        {activity ? <NowLine activity={activity} micro={micro} reports={agent.reports.length} now={now} /> : <span className="hb-card-none">{t("noData")}</span>}
        {activity ? (
          <div className="hb-fb-host">
            <FeedbackRow kind="robot_state" id={agent.key} sample={{ state: activity.state, label: activity.detail }} />
          </div>
        ) : null}
      </section>
      <section className="hb-card-sec" aria-label={t("lastSteps")}>
        <h3 className="hb-card-h">{t("lastSteps")}</h3>
        {steps.length === 0 ? (
          <span className="hb-card-none">{t("noSteps")}</span>
        ) : (
          <ol className="hb-steps">
            {steps.map((s, i) => (
              <StepLine key={`${s.started_ms}-${i}`} step={s} now={now} />
            ))}
          </ol>
        )}
      </section>
      {m.progress.length > 0 ? (
        <section className="hb-card-sec" aria-label={t("progress")}>
          <h3 className="hb-card-h">{t("progress")}</h3>
          <p className="hb-card-progress" suppressHydrationWarning>
            {m.progress.map((p, i) => (
              <span key={p.key} data-progress={p.key} data-tone={p.tone}>
                {i > 0 ? " · " : null}
                {p.lead ? `${p.lead} ` : null}
                {p.values.map((v, j) => (
                  <React.Fragment key={j}>
                    {j > 0 ? ", " : null}
                    <strong>{v.n}</strong>
                    {v.words ? ` ${v.words}` : null}
                  </React.Fragment>
                ))}
              </span>
            ))}
          </p>
        </section>
      ) : null}
      <div className="hb-card-actions">
        <button ref={firstButton} type="button" className="hb-card-btn" onClick={onOpenChat}>
          <MessageCircle className="size-[18px]" aria-hidden /> {t("openConversation")}
        </button>
        {extraActions}
      </div>
    </div>
  );
}

/** Only one card is open at a time: opening one closes the other. */
let current: HoverIntent | null = null;

function browserTimers() {
  return {
    set: (fn: () => void, ms: number) => window.setTimeout(fn, ms),
    clear: (t: unknown) => window.clearTimeout(t as number),
  };
}

/**
 * Wraps an agent's avatar or name (or a sidebar row holding both): hover
 * or keyboard focus opens its card after 300 ms, leaving closes it, Esc
 * closes it and keeps the focus where it was. With the card open, ↓ moves
 * the focus into it; Esc there brings it back.
 */
export function AgentHover({
  agent,
  children,
  className,
  block = false,
}: {
  agent: WsAgent | null | undefined;
  children: React.ReactElement<Record<string, unknown>>;
  className?: string;
  block?: boolean;
}) {
  const [open, setOpen] = React.useState(false);
  const [pos, setPos] = React.useState<{ left: number; top: number; side: string; arrow: number } | null>(null);
  const wrap = React.useRef<HTMLSpanElement>(null);
  const card = React.useRef<HTMLDivElement>(null);
  const first = React.useRef<HTMLButtonElement>(null);
  const intent = React.useRef<HoverIntent | null>(null);
  const id = React.useId();
  if (!intent.current && typeof window !== "undefined") {
    intent.current = createHoverIntent((v) => {
      if (v) {
        if (current && current !== intent.current) current.close();
        current = intent.current;
      } else if (current === intent.current) current = null;
      setOpen(v);
    }, browserTimers());
  }
  React.useEffect(() => () => intent.current?.close(), []);
  React.useLayoutEffect(() => {
    if (!open || !wrap.current) return;
    const place = () => {
      const el = wrap.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const h = card.current?.offsetHeight ?? 520;
      setPos(placeCard(r, { width: CARD_WIDTH, height: h }, { width: window.innerWidth, height: window.innerHeight }, 14));
    };
    place();
    const raf = requestAnimationFrame(place);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);
  // Esc anywhere closes the open card.
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const inside = card.current?.contains(document.activeElement ?? null) ?? false;
      intent.current?.escape();
      if (inside) (wrap.current?.querySelector<HTMLElement>("a,button,[tabindex]") ?? wrap.current)?.focus();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);
  if (!agent) return children;
  const child = React.cloneElement(children, {
    "aria-describedby": open ? id : undefined,
    "aria-keyshortcuts": "ArrowDown",
    // The card replaces the native tooltip.
    title: undefined,
  });
  return (
    <span
      ref={wrap}
      className={`hb-hover ${block ? "hb-hover-block" : ""} ${className ?? ""}`}
      data-card-open={open ? "true" : undefined}
      onMouseEnter={() => intent.current?.enter()}
      onMouseLeave={() => intent.current?.leave()}
      onFocus={() => intent.current?.enter()}
      onBlur={(e) => {
        if (card.current?.contains(e.relatedTarget as Node | null)) return;
        intent.current?.leave();
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown" && intent.current?.isOpen()) {
          e.preventDefault();
          intent.current.enterCard();
          first.current?.focus();
        }
      }}
    >
      {child}
      {open ? (
        <CardPortal>
          <div
            ref={card}
            className="hb-card-wrap"
            data-side={pos?.side}
            style={{ left: pos?.left ?? -9999, top: pos?.top ?? -9999, width: CARD_WIDTH, ["--hb-card-arrow" as string]: `${pos?.arrow ?? 40}px` }}
            onMouseEnter={() => intent.current?.enterCard()}
            onMouseLeave={() => intent.current?.leaveCard()}
            onFocus={() => intent.current?.enterCard()}
            onBlur={(e) => {
              if (card.current?.contains(e.relatedTarget as Node | null)) return;
              if (wrap.current?.contains(e.relatedTarget as Node | null)) return;
              intent.current?.leaveCard();
              intent.current?.leave();
            }}
          >
            <LiveCard id={id} agent={agent} firstButton={first} onDone={() => intent.current?.close()} />
          </div>
        </CardPortal>
      ) : null}
    </span>
  );
}

/** The card goes inside the shell's root, so it keeps the theme's tokens and motion rules. */
function CardPortal({ children }: { children: React.ReactNode }) {
  const [root, setRoot] = React.useState<Element | null>(null);
  React.useEffect(() => setRoot(document.querySelector(".hb") ?? document.body), []);
  return root ? createPortal(children, root) : null;
}

/** The card with live data: the face from the 2 s slice, the steps from the open agent's live read. */
function LiveCard({ id, agent, firstButton, onDone }: { id: string; agent: WsAgent; firstButton: React.Ref<HTMLButtonElement>; onDone: () => void }) {
  const router = useRouter();
  const live = useLive();
  const { activity } = useAgentLive(agent);
  // Fresh steps while the card is open; the view's copy until the first answer.
  const fresh = useLiveAgent(agent.key, { prose: false, only: "progress" });
  const micro = activity && live?.thresholds ? microState(activity, live.serverNow, live.thresholds) : null;
  const post = useOptionalWorkspace()?.post ?? null;
  const hide = hideAction(agent);
  return (
    <AgentCardView
      id={id}
      agent={agent}
      activity={activity}
      micro={micro}
      serverNow={live?.serverNow ?? 0}
      offset={live?.offset ?? 0}
      progress={fresh.state?.progress ?? agent.progress ?? null}
      avatar={<AgentAvatar name={agent.name} agent={agent} size={104} dot={false} />}
      firstButton={firstButton}
      onOpenChat={() => {
        onDone();
        router.push(`/direct/${encodeURIComponent(agent.key)}` as Route);
      }}
      extraActions={
        <>
        <button
          type="button"
          className="hb-card-btn hb-card-btn-live"
          onClick={() => {
            onDone();
            router.push(`/direct/${encodeURIComponent(agent.key)}?direct=1` as Route);
          }}
        >
          <span className="hb-live-dot" aria-hidden /> {t("watchLive")}
        </button>
        {post ? (
          <button
            type="button"
            className="hb-card-btn hb-card-btn-quiet"
            onClick={() => {
              onDone();
              void post(hide.action, { agent: agent.key }).catch(() => {});
            }}
          >
            <EyeOff className="size-[18px]" aria-hidden /> {hide.label}
          </button>
        ) : null}
        </>
      }
    />
  );
}
