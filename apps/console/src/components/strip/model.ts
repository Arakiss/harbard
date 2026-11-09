/**
 * The strip's pure parts, "The factory": who stands on the shelf and
 * in what order, what "Needs you" shows, what "Today" counts, where a tap
 * goes, the clock's words and when the strip dims. The workspace view
 * (`GET /workspace`) and the live faces' slice are the only inputs; nothing
 * here invents a state serve did not send.
 */

import type { Activity } from "@/lib/activity";
import type { MainDestination } from "@/lib/desktop-view";
import { agentLabel, cardBody, hhmm, openCards, type WsAgent, type WsCard, type WsMessage, type WorkspaceView } from "@/lib/workspace";
import { cardHidden } from "@/lib/hidden";
import { createT, intlTag } from "@/i18n";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("strip");

const MIN = 60_000;
const HOUR = 60 * MIN;

/** An agent stands on the shelf when it did something this recently. */
const SHELF_ACTIVE_MS = 2 * HOUR;
/** Robots on the shelf; more show as "+N" at its end. */
export const SHELF_MAX = 8;
/** Cards in "Needs you"; more show as "+N more". */
export const NEEDS_MAX = 3;
/** "Today" closes by itself after this. */
export const TODAY_OPEN_MS = 20_000;

/** The shelf's order: working first, then waiting, sleeping and finished. */
export type ShelfGroup = "working" | "waiting" | "sleeping" | "finished";
const GROUP_RANK: Record<ShelfGroup, number> = { working: 0, waiting: 1, sleeping: 2, finished: 3 };

/** What the live slice knows of one agent, fresher than the view. */
export interface LiveAgent {
  activity: Activity;
  last_action_ms: number | null;
}

export function shelfGroup(agent: Pick<WsAgent, "presence" | "dormant">, activity?: Pick<Activity, "state"> | null): ShelfGroup {
  if (agent.dormant || activity?.state === "asleep") return "sleeping";
  if (agent.presence === "finished" || activity?.state === "finished" || activity?.state === "verified") return "finished";
  if (agent.presence === "waiting" || activity?.state === "waiting" || activity?.state === "blocked") return "waiting";
  return "working";
}

/** The newest thing the agent did: the live slice when it knows. */
function lastAction(agent: Pick<WsAgent, "key" | "last_action_ms">, live?: ReadonlyMap<string, LiveAgent>): number | null {
  const fresh = live?.get(agent.key)?.last_action_ms ?? null;
  const own = agent.last_action_ms;
  if (fresh === null) return own;
  return own === null ? fresh : Math.max(own, fresh);
}

export interface Shelf {
  /** At most SHELF_MAX, in shelf order. */
  shown: WsAgent[];
  /** Active agents that did not fit. */
  more: number;
  /** Nobody active: `shown` are the latest agents, asleep, for the empty shelf. */
  empty: boolean;
}

/**
 * Who stands on the shelf: the agents active in the last two hours, working
 * first, then waiting, sleeping and finished; inside a group the newest
 * first, then by key so robots never swap places on a tie. With nobody
 * active, the most recent agents sleep there instead.
 */
export function shelf(view: Pick<WorkspaceView, "agents" | "now_ms">, now: number, live?: ReadonlyMap<string, LiveAgent>): Shelf {
  const at = (a: WsAgent) => lastAction(a, live) ?? 0;
  const newest = (a: WsAgent, b: WsAgent) => at(b) - at(a) || a.key.localeCompare(b.key);
  const active = view.agents.filter((a) => !a.lost && at(a) > 0 && now - at(a) <= SHELF_ACTIVE_MS);
  if (active.length === 0) {
    const recent = view.agents.filter((a) => at(a) > 0).sort(newest);
    return { shown: recent.slice(0, SHELF_MAX), more: 0, empty: true };
  }
  const rank = (a: WsAgent) => GROUP_RANK[shelfGroup(a, live?.get(a.key)?.activity ?? a.activity)];
  const ordered = active.slice().sort((a, b) => rank(a) - rank(b) || newest(a, b));
  return { shown: ordered.slice(0, SHELF_MAX), more: Math.max(0, ordered.length - SHELF_MAX), empty: false };
}

/** The shelf's name sizes: bold names, down to what a long real name needs. */
export const NAME_MAX = 38;
export const NAME_MIN = 28;

/** A first estimate of a name's size in its slot, from its length (the strip then measures). */
export function nameSize(name: string, slot: number): number {
  // Lato Black averages a little over half an em per character.
  const fit = Math.floor((slot - 12) / (Math.max(1, [...name].length) * 0.56));
  return Math.max(NAME_MIN, Math.min(NAME_MAX, fit));
}

// ---------------------------------------------------------------------------
// Needs you
// ---------------------------------------------------------------------------

/** "Needs you": Activity's open cards, in its order, at most three. */
export function needs(view: WorkspaceView): { shown: WsCard[]; more: number } {
  const open = openCards(view);
  return { shown: open.slice(0, NEEDS_MAX), more: Math.max(0, open.length - NEEDS_MAX) };
}

/** The pill beside the name: why the card is there. */
export function cardPill(card: Pick<WsCard, "kind" | "since_ms">): { text: string; tone: "warn" | "quiet" | "alert" } {
  switch (card.kind) {
    case "blocked":
      return { text: t("pillBlocked"), tone: "warn" };
    case "approval":
      return { text: t("pillApproval"), tone: "warn" };
    case "dormant":
      return { text: t("pillIdleSince", { time: hhmm(card.since_ms) }), tone: "quiet" };
    default:
      return { text: t("pillAlert"), tone: "alert" };
  }
}

/** "BLOCKED · keychain-review · Can it…?" → "Can it…?": the sentence, without the protocol's label and name. */
export function sentence(text: string): string {
  const first = text.split("\n").find((l) => l.trim()) ?? "";
  const unlabelled = first.replace(/^\s*(START|BLOCKED|FINDING|DONE)(\s+v\d+)?\s*[·:-]\s*/, "");
  if (unlabelled === first) return first.trim();
  // The project or agent name the protocol puts second: one word, then "·".
  return unlabelled.replace(/^[^\s·]+\s*·\s*/, "").trim() || unlabelled.trim();
}

/**
 * A card as the strip draws it: who, why, what it says, and its button:
 * "Reply" when there is an agent to answer, "Abrir" for a warning
 * about several (two writers in one copy).
 */
export function cardView(
  card: WsCard,
  view: WorkspaceView,
  now: number,
): { who: string; pill: ReturnType<typeof cardPill>; text: string; agent: string | null; action: "reply" | "open"; hidden: boolean } {
  const message = card.message ? view.messages.find((m) => m.id === card.message) : undefined;
  const agent = card.agents[0] ?? null;
  const body = cardBody(card, view, now);
  const who = card.kind === "two_writers" ? body.title : agent ? agentLabel(view, agent) : body.title;
  const text = message ? sentence(message.text) : body.text;
  // A hidden agent still needs the operator: the card shows, marked "hidden".
  return { who, pill: cardPill(card), text, agent, action: card.kind === "two_writers" || !agent ? "open" : "reply", hidden: cardHidden(view, card) };
}

// ---------------------------------------------------------------------------
// Today
// ---------------------------------------------------------------------------

function localDay(ms: number): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: operatorTimeZone(), year: "numeric", month: "2-digit", day: "2-digit" }).format(ms);
}

export interface Today {
  /** Today's deliveries, newest first. */
  items: WsMessage[];
  deliveries: number;
  /** Only those with harbard's evidence (a landed receipt), never a claim in the text. */
  verified: number;
}

/**
 * "Today": the agents' DONE messages of today on the operator's clock ("Delivered"), each
 * once (a repeat is the same delivery), and how many harbard verified on a
 * landed receipt. What a message says about itself never counts.
 */
export function today(view: Pick<WorkspaceView, "messages">, now: number): Today {
  const today = localDay(now);
  const items = view.messages
    .filter((m) => m.from === "agent" && m.label === "DONE" && !m.repeat_of && !m.structured && localDay(m.ts_ms) === today)
    .sort((a, b) => b.ts_ms - a.ts_ms || a.id.localeCompare(b.id));
  return { items, deliveries: items.length, verified: items.filter((m) => m.verified !== null).length };
}

/** "Today: 6 deliveries · 4 verified". */
export function todayLine(h: Pick<Today, "deliveries" | "verified">): string {
  if (h.deliveries === 0) return t("todayNone");
  const e = h.deliveries === 1 ? t("deliveriesOne", { count: h.deliveries }) : t("deliveriesMany", { count: h.deliveries });
  const v = h.verified === 1 ? t("verifiedOne", { count: h.verified }) : t("verifiedMany", { count: h.verified });
  return t("todayLine", { deliveries: e, verified: v });
}

// ---------------------------------------------------------------------------
// Taps: the main window, through the desktop bridge
// ---------------------------------------------------------------------------

/** Tapping a robot: its Direct conversation. */
export function robotDestination(agent: Pick<WsAgent, "key">): MainDestination {
  return { view: "direct", session: agent.key };
}

/** "Reply" on a card: that item in Activity, with its message and the composer. */
export function cardDestination(card: Pick<WsCard, "id">): MainDestination {
  return { view: "activity", note: `card:${card.id}` };
}

/** "+N more": Activity's "Needs you". */
export const NEEDS_MORE: MainDestination = { view: "activity" };
/** "+N" at the shelf's end: every agent, in Direct. */
export const SHELF_MORE: MainDestination = { view: "direct" };

// ---------------------------------------------------------------------------
// Clock and night
// ---------------------------------------------------------------------------

/** "14:52" and "Saturday 7 Nov", on the operator's clock. */
export function clock(ms: number): { time: string; date: string } {
  const time = hhmm(ms);
  const parts = new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), weekday: "long", day: "numeric", month: "short" }).formatToParts(ms);
  const get = (type: string) => parts.find((p) => p.type === type)?.value ?? "";
  return { time, date: `${get("weekday")} ${get("day")} ${get("month").replace(".", "")}` };
}

function localHour(ms: number): number {
  return Number(new Intl.DateTimeFormat("en-GB", { timeZone: operatorTimeZone(), hour: "2-digit", hourCycle: "h23" }).format(ms));
}

/** The strip dims from 22:00 to 7:00 when the OS is dark, unless turned off in Preferences. */
export function dimmed(ms: number, osDark: boolean, enabled: boolean): boolean {
  if (!enabled || !osDark) return false;
  const h = localHour(ms);
  return h >= 22 || h < 7;
}

// ---------------------------------------------------------------------------
// The stage's fit, and the keys that close the window

/** The stage the strip is drawn on: a 2560×720 pixel grid, 32:9. */
const STAGE_W = 2560;
const STAGE_H = 720;

/**
 * How the 2560×720 stage fits a viewport: one uniform scale, the largest
 * that shows the whole strip, and the margins left over on each side, which
 * show the theme's frame colour (letterbox). Never a crop, never a stretch.
 * A degenerate viewport (0 or not a number) keeps the stage at 1.
 */
export function stripFit(width: number, height: number): { k: number; width: number; height: number; left: number; top: number } {
  const raw = Math.min(width / STAGE_W, height / STAGE_H);
  const k = Number.isFinite(raw) && raw > 0 ? raw : 1;
  const w = STAGE_W * k;
  const h = STAGE_H * k;
  return { k, width: w, height: h, left: Math.max(0, (width - w) / 2), top: Math.max(0, (height - h) / 2) };
}

/**
 * `stripFit`'s scale as an inline script, for the pulse layout to run
 * before the first paint (it needs the CSP nonce). A test runs it against
 * `stripFit` so the two cannot drift.
 */
export const STAGE_FIT_SCRIPT = `(function(){function k(){var s=Math.min(innerWidth/${STAGE_W},innerHeight/${STAGE_H});document.documentElement.style.setProperty("--cs-k",String(isFinite(s)&&s>0?s:1))}k();addEventListener("resize",k)})();`;

/**
 * Esc, and ⌘W (Ctrl+W elsewhere), hide the Strip window. Esc with a
 * modifier, or one a control already used (`defaultPrevented`), does not.
 */
export function closesStrip(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; defaultPrevented?: boolean }): boolean {
  if (e.defaultPrevented) return false;
  if (e.key === "Escape") return !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
  return (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "w";
}
