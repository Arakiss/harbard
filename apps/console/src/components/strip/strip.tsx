"use client";

import * as React from "react";
import { Bell, Factory, Package } from "lucide-react";
import { BrandMark } from "@/components/brand/brand-mark";
import { MarkOverlay, MarkUnderlay } from "@/components/brand/living-mark";
import { DesktopViewLink } from "@/components/desktop-view-link";
import { closeDesktopStrip } from "@/lib/desktop-view";
import { Robot } from "@/components/robot/robot";
import { StatusIcon, useSecondTick } from "@/components/robot/status";
import { useOsReducedMotion } from "@/components/workspace/parts";
import { useLogo, useMotionAttrs } from "@/components/workspace/shell";
import { useThemeAttrs } from "@/components/workspace/theme-context";
import { StripDecor, useFlurry } from "@/components/workspace/season-decor";
import { useAgentLive, useLive, useWorkspace, type LiveCtx } from "@/components/workspace/workspace-context";
import { elapsed, microState, statusParts } from "@/lib/activity";
import { hhmm, robotName, type WsAgent, type WsCard, type WorkspaceView } from "@/lib/workspace";
import {
  TODAY_OPEN_MS,
  NEEDS_MORE,
  SHELF_MORE,
  cardDestination,
  cardView,
  clock,
  closesStrip,
  stripFit,
  dimmed,
  today,
  todayLine,
  NAME_MAX,
  NAME_MIN,
  nameSize,
  needs,
  robotDestination,
  sentence,
  shelf,
  type LiveAgent,
} from "./model";
import { activeLocale, createT } from "@/i18n";

const t = createT("strip");

/** The shelf's inner width, its widest slot and its largest robot. */
const SHELF_INNER = 1444;
const SLOT_MAX = 206;
const ROBOT_MAX = 250;
/** A bust's shoulders take about this share of its width: neighbours just touch. */
const SHOULDERS = 0.84;
const MORE_W = 104;

/**
 * The strip, "The factory": what needs the operator on the left, the agents
 * of the last two hours as robots on a shelf in the middle, the clock and
 * the day's deliveries on the right. A tap opens the main window through the
 * desktop bridge; nothing here writes to serve.
 */
export function StripView() {
  const { state, now } = useWorkspace();
  const live = useLive();
  const theme = useThemeAttrs();
  const motion = useMotionAttrs(live);
  const os = useOsReducedMotion();
  const reduce = os || live?.motion === "reduce";
  const view = state.kind === "ready" ? state.view : null;
  const wall = useMinute(view?.now_ms ?? now);
  const dark = usePrefersDark();
  const dim = dimmed(wall, dark, view?.night_dim !== false);
  // Dimmed for the night, nothing falls.
  const decor = theme["data-hb-decor"] !== undefined;
  const flurry = useFlurry(decor && !dim);
  useStageFit();
  useCloseKeys();
  return (
    <div
      className="hb cs"
      lang={activeLocale()}
      {...theme}
      {...motion}
      data-dim={dim ? "" : undefined}
      data-hb-decor-still={decor && !flurry ? "" : undefined}
    >
      <div className="cs-stage">
        {decor ? <StripDecor /> : null}
        <Header live={live} reduce={reduce} failed={state.kind === "failed" || (state.kind === "ready" && state.error !== null)} />
        <main className="cs-body">
          {view ? (
            <>
              <Needs view={view} now={now} />
              <ShelfPanel view={view} live={live} reduce={reduce} wall={wall} />
            </>
          ) : (
            <>
              <section className="cs-panel cs-needs" aria-label={t("needsYou")} />
              <section className="cs-panel cs-shelf" aria-label={t("factory")}>
                <div className="cs-backdrop" aria-hidden />
                <div className="cs-front">
                  <p className="cs-quiet">{state.kind === "failed" ? t("notResponding") : t("connecting")}</p>
                </div>
              </section>
            </>
          )}
        </main>
        <div className="cs-dim" aria-hidden />
      </div>
    </div>
  );
}

const useIsoLayoutEffect = typeof window === "undefined" ? React.useEffect : React.useLayoutEffect;

/**
 * Scale the stage to the window: `--cs-k` on the root, which strip.css
 * turns into the stage's zoom. The pulse layout's inline script sets it
 * before the first paint; this keeps it right on every resize, and is the
 * only route if that script did not run.
 */
function useStageFit() {
  useIsoLayoutEffect(() => {
    const fit = () => {
      document.documentElement.style.setProperty("--cs-k", String(stripFit(window.innerWidth, window.innerHeight).k));
    };
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, []);
}

/** Esc and ⌘W hide the Strip window in the desktop app; in a browser tab they do nothing. */
function useCloseKeys() {
  React.useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (!closesStrip(e)) return;
      if (e.key !== "Escape") e.preventDefault();
      void closeDesktopStrip().catch(() => {});
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, []);
}

/**
 * Wall-clock time to the minute: the clock and the night dimming need no
 * more, so the strip re-renders once a minute, not once a second.
 * `fallback` (serve's clock) is the first render's, on the server too.
 */
function useMinute(fallback: number): number {
  const [minute, setMinute] = React.useState(() => Math.floor(fallback / 60_000));
  React.useEffect(() => {
    const tick = () => setMinute(Math.floor(Date.now() / 60_000));
    tick();
    const t = setInterval(tick, 1_000);
    return () => clearInterval(t);
  }, []);
  return minute * 60_000;
}

function usePrefersDark(): boolean {
  const [dark, setDark] = React.useState(false);
  React.useEffect(() => {
    const q = window.matchMedia("(prefers-color-scheme: dark)");
    setDark(q.matches);
    const on = (e: MediaQueryListEvent) => setDark(e.matches);
    q.addEventListener("change", on);
    return () => q.removeEventListener("change", on);
  }, []);
  return dark;
}

function Header({ live, reduce, failed }: { live: LiveCtx | null; reduce: boolean; failed: boolean }) {
  const logo = useLogo(live, reduce);
  return (
    <header className="cs-head" data-tauri-drag-region>
      <BrandMark
        className="cs-mark hb-living"
        label="harbard"
        data-logo={logo.state}
        data-logo-once={logo.once ?? undefined}
        underlay={<MarkUnderlay />}
        overlay={<MarkOverlay />}
      />
      <span className="cs-word" translate="no" aria-hidden>
        harbard
      </span>
      <span className="cs-sep" aria-hidden />
      <h1 className="cs-title">
        <Factory className="cs-title-icon" strokeWidth={2.2} aria-hidden />
        {t("factory")}
      </h1>
      {failed ? (
        <p className="cs-offline" role="status">
          {t("offlineNote")}
        </p>
      ) : null}
    </header>
  );
}

// ---------------------------------------------------------------------------
// Needs you
// ---------------------------------------------------------------------------

function Needs({ view, now }: { view: WorkspaceView; now: number }) {
  const { shown, more } = needs(view);
  return (
    <section className="cs-panel cs-needs" aria-labelledby="cs-needs-title" data-count={shown.length}>
      <div className="cs-needs-head">
        <span className="cs-bell" aria-hidden>
          <Bell strokeWidth={2.2} />
        </span>
        <h2 id="cs-needs-title">{t("needsYou")}</h2>
      </div>
      {shown.length === 0 ? <p className="cs-needs-none">{t("nothingNeedsYou")}</p> : null}
      {shown.map((card, i) => (i === 0 ? <BigCard key={card.id} card={card} view={view} now={now} /> : <SmallCard key={card.id} card={card} view={view} now={now} />))}
      {more > 0 ? (
        <DesktopViewLink target="overview" destination={NEEDS_MORE} className="cs-needs-more">
          {t("moreCount", { count: more })}
        </DesktopViewLink>
      ) : null}
    </section>
  );
}

function BigCard({ card, view, now }: { card: WsCard; view: WorkspaceView; now: number }) {
  const c = cardView(card, view, now);
  return (
    <article className="cs-card cs-card-big" data-tone={c.pill.tone} data-card={card.id}>
      <p className="cs-card-top">
        <span className="cs-card-who">{c.who}</span>
        <span className="cs-card-dot" aria-hidden>
          ·
        </span>
        <span className="cs-pill" data-tone={c.pill.tone}>
          {c.pill.text}
        </span>
        {c.hidden ? <span className="cs-hidden-note">{t("hidden")}</span> : null}
      </p>
      {c.text ? <p className="cs-card-text">{c.text}</p> : null}
      <DesktopViewLink target="overview" destination={cardDestination(card)} workspace={card.workspace} className="cs-reply" aria-label={c.action === "reply" ? t("replyToAria", { name: c.who }) : t("openInActivityAria", { name: c.who })}>
        {c.action === "reply" ? t("reply") : t("open")}
      </DesktopViewLink>
    </article>
  );
}

function SmallCard({ card, view, now }: { card: WsCard; view: WorkspaceView; now: number }) {
  const c = cardView(card, view, now);
  return (
    <DesktopViewLink target="overview" destination={cardDestination(card)} workspace={card.workspace} className="cs-card cs-card-small" data-tone={c.pill.tone} data-card={card.id} aria-label={c.hidden ? t("smallCardAriaHidden", { who: c.who, pill: c.pill.text }) : t("smallCardAria", { who: c.who, pill: c.pill.text })}>
      <span className="cs-card-who">{c.who}</span>
      <span className="cs-card-dot" aria-hidden>
        ·
      </span>
      <span className="cs-pill" data-tone={c.pill.tone}>
        {c.pill.text}
      </span>
      {c.hidden ? (
        <span className="cs-hidden-note" aria-hidden>
          {t("hidden")}
        </span>
      ) : null}
    </DesktopViewLink>
  );
}

// ---------------------------------------------------------------------------
// The shelf, and Today over it
// ---------------------------------------------------------------------------

function ShelfPanel({ view, live, reduce, wall }: { view: WorkspaceView; live: LiveCtx | null; reduce: boolean; wall: number }) {
  const liveAgents = (live?.agents ?? undefined) as ReadonlyMap<string, LiveAgent> | undefined;
  const now = live?.serverNow || view.now_ms;
  const s = shelf(view, now, liveAgents);
  const day = today(view, now);
  const [open, setOpen] = React.useState(false);
  React.useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => setOpen(false), TODAY_OPEN_MS);
    return () => clearTimeout(t);
  }, [open]);
  const n = s.shown.length;
  const room = SHELF_INNER - (s.more > 0 ? MORE_W : 0);
  const slot = Math.min(SLOT_MAX, Math.floor(room / Math.max(1, n)));
  // A bust is as tall as it is wide; its shoulders, a little narrower, just touch the next.
  const size = Math.min(ROBOT_MAX, Math.round(slot / SHOULDERS));
  // One name size for the whole shelf: the largest that fits every name.
  const names = useNameSize(s.shown.map((a) => a.name), slot);
  const style = { "--slot": `${slot}px`, "--bot": `${size}px`, "--name": `${names}px` } as React.CSSProperties;
  return (
    <>
      <section className="cs-panel cs-shelf" aria-label={t("factory")} data-empty={s.empty ? "" : undefined} style={style}>
        <div className="cs-backdrop" aria-hidden />
        <div className="cs-front" aria-hidden />
        <ul className="cs-bots" aria-label={s.empty ? t("nobodyWorking") : t("agentsLastTwoHours")}>
          {s.shown.map((agent) => (
            <ShelfBot key={agent.key} agent={agent} live={live} reduce={reduce} size={size} asleep={s.empty} />
          ))}
          {s.more > 0 ? (
            <li className="cs-slot cs-slot-more">
              <DesktopViewLink target="overview" destination={SHELF_MORE} className="cs-slot-link" aria-label={t("moreAgentsAria", { count: s.more })}>
                <span className="cs-more-disc">+{s.more}</span>
                <span className="cs-more-label">{t("moreLabel")}</span>
              </DesktopViewLink>
            </li>
          ) : null}
        </ul>
        {s.empty ? <p className="cs-empty-line">{t("nobodyWorking")}</p> : null}
        <TodayOverlay open={open} day={day} view={view} onClose={() => setOpen(false)} />
      </section>
      <aside className="cs-panel cs-side" aria-label={t("timeAndDeliveries")}>
        <Clock wall={wall} />
        <button type="button" className="cs-today" aria-expanded={open} aria-controls="cs-today-list" onClick={() => setOpen((o) => !o)}>
          <Package className="cs-today-icon" strokeWidth={2} aria-hidden />
          <span>{todayLine(day)}</span>
        </button>
      </aside>
    </>
  );
}

/**
 * The shelf's name size: the largest from NAME_MAX down that fits every
 * name in its slot, measured in the names' own font once it has loaded
 * (an estimate from the character count until then, and on the server).
 */
function useNameSize(names: string[], slot: number): number {
  const key = names.join("\n");
  const estimate = Math.min(NAME_MAX, ...names.map((n) => nameSize(n, slot)));
  const [measured, setMeasured] = React.useState<{ key: string; size: number } | null>(null);
  React.useEffect(() => {
    let alive = true;
    const measure = () => {
      const probe = document.querySelector(".cs-name");
      const ctx = document.createElement("canvas").getContext("2d");
      if (!probe || !ctx || !alive) return;
      const family = getComputedStyle(probe).fontFamily;
      ctx.font = `900 ${NAME_MAX}px ${family}`;
      const widest = Math.max(1, ...key.split("\n").map((n) => ctx.measureText(n).width));
      const fit = Math.floor((NAME_MAX * (slot - 12)) / widest);
      setMeasured({ key: `${key}|${slot}`, size: Math.max(NAME_MIN, Math.min(NAME_MAX, fit)) });
    };
    void document.fonts.ready.then(measure);
    return () => {
      alive = false;
    };
  }, [key, slot]);
  return measured?.key === `${key}|${slot}` ? measured.size : estimate;
}

function ShelfBot({ agent, live, reduce, size, asleep }: { agent: WsAgent; live: LiveCtx | null; reduce: boolean; size: number; asleep: boolean }) {
  const { activity, reactions } = useAgentLive(agent);
  return (
    <li className="cs-slot" data-agent={agent.key}>
      <DesktopViewLink target="overview" destination={robotDestination(agent)} workspace={agent.workspace} className="cs-slot-link" aria-label={t("openConversationAria", { name: agent.name })}>
        <Robot
          name={robotName(agent)}
          size={size}
          agentKey={agent.key}
          activity={asleep ? null : activity}
          face={asleep ? "asleep" : undefined}
          thresholds={live?.thresholds ?? null}
          now={live?.serverNow || undefined}
          reactions={asleep ? [] : reactions}
          baseline={live?.baseline}
          reduce={reduce}
          dot={null}
          className="cs-bot"
        />
        {asleep ? null : (
          <span className="cs-plate">
            <span className="cs-name">{agent.name}</span>
            {activity ? <ShelfStatus agent={agent} live={live} /> : null}
          </span>
        )}
      </DesktopViewLink>
    </li>
  );
}

const NBSP = "\u00a0";
// "No task right now" has no timer either: nothing is running.
const TIMELESS = new Set(["disconnected", "waiting", "asleep", "normal"]);

/** Under a robot: the state's icon, its words and a timer that ticks. */
function ShelfStatus({ agent, live }: { agent: WsAgent; live: LiveCtx | null }) {
  const { activity } = useAgentLive(agent);
  const offset = live?.offset ?? 0;
  const serverNow = live?.serverNow ?? 0;
  const wallNow = useSecondTick(serverNow - offset);
  if (!activity) return null;
  const now = wallNow + offset;
  const micro = live?.thresholds ? microState(activity, now, live.thresholds) : null;
  // A label ("cargo test") and a time ("2 min") never break inside, and a
  // " · " stays at the end of its line, never at the start of the next.
  const words = statusParts(activity, { reports: agent.reports.length, micro })
    .map((p) => ("code" in p ? p.code.replace(/ /g, NBSP) : p.text))
    .join("")
    .replace(/ · /g, `${NBSP}· `);
  // Waiting and asleep already say since when ("desde 14:20"): the clock form of the same timer.
  const time = TIMELESS.has(activity.state) ? null : elapsed(now - activity.since_ms);
  return (
    <span className="cs-status" data-state={activity.state} data-long={micro?.long ? "true" : undefined}>
      <span className="cs-status-chip" data-state={activity.state}>
        <StatusIcon state={activity.state} />
      </span>
      <span className="cs-status-text">
        {words}
        {time ? (
          <span className="cs-status-time" suppressHydrationWarning>
            {`${NBSP}· ${time.replace(/ /g, NBSP)}`}
          </span>
        ) : null}
      </span>
    </span>
  );
}

function TodayOverlay({ open, day, view, onClose }: { open: boolean; day: ReturnType<typeof today>; view: WorkspaceView; onClose: () => void }) {
  const shown = day.items.slice(0, 10);
  const rest = day.items.length - shown.length;
  return (
    <div id="cs-today-list" className="cs-today-sheet" data-open={open ? "" : undefined} aria-hidden={!open} onClick={onClose}>
      <h2 className="cs-today-title">{t("today")}</h2>
      {shown.length === 0 ? (
        <p className="cs-quiet">{t("nothingDeliveredToday")}</p>
      ) : (
        <ol className="cs-today-items">
          {shown.map((m) => (
            <li key={m.id} className="cs-today-item" data-verified={m.verified ? "" : undefined}>
              <span className="cs-today-dot" aria-hidden />
              <span className="cs-today-when">
                {hhmm(m.ts_ms)} · {view.agents.find((a) => a.key === m.agent)?.name ?? m.author}
              </span>
              <span className="cs-today-what">{sentence(m.text)}</span>
              <span className="cs-today-chips">
                <span className="cs-chip-done">{t("delivered")}</span>
                {m.verified ? <span className="cs-chip-ok">{t("verifiedChip")}</span> : null}
              </span>
            </li>
          ))}
        </ol>
      )}
      {rest > 0 ? <p className="cs-today-rest">{t("restInActivity", { count: rest })}</p> : null}
    </div>
  );
}

function Clock({ wall }: { wall: number }) {
  const c = clock(wall);
  return (
    <div className="cs-clock">
      <time className="cs-time" suppressHydrationWarning>
        {c.time}
      </time>
      <span className="cs-date" suppressHydrationWarning>
        {c.date}
      </span>
    </div>
  );
}
