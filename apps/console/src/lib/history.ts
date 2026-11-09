/**
 * History: the sessions and agents of the last month, as
 * `GET /api/v1/history/agents` pages them (`harbard_core::history`).
 *
 * serve decides how each one ended and whether its conversation is
 * read-only; this module only words it, groups it by day and keeps the
 * pages in order. Every text here was masked by serve before it left.
 */

import { createT, intlTag, type TranslationKey } from "@/i18n";
import { dayLabel, hhmm, type WsAgent, type WsMessage } from "./workspace";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("history");

/** How an item ended: its own state, or silence (`sin_cierre`). */
export type Ending = "finished" | "closed" | "verified" | "no_closure";

export interface HistoryItem {
  /** The workspace agent: name, role line, face, delivery. */
  agent: WsAgent;
  /** The first channel it wrote in, and that channel's repository. */
  channel: string | null;
  repo: string | null;
  start_ms: number;
  end_ms: number;
  duration_ms: number;
  /** Its own last human line, masked. */
  last_line: string | null;
  /** Null while it is still working or waiting. */
  ending: Ending | null;
  /** Its session ended, or it has been quiet longer than the shell keeps. */
  read_only: boolean;
}

export interface HistoryChannel {
  id: string;
  repo: string;
  items: number;
}

/** `GET /api/v1/history/agents`: one page, newest first. */
export interface HistoryPage {
  items: HistoryItem[];
  /** The cursor of the next page; null on the last one. */
  next: string | null;
  /** Items matching the filter and the search, over every page. */
  total: number;
  /** Every channel of the month, for the filter. */
  channels: HistoryChannel[];
  now_ms: number;
  window_days: number;
}

/** `GET /api/v1/history/agents/{key}`: one conversation. */
export interface HistoryConversation {
  item: HistoryItem;
  messages: WsMessage[];
  /** More messages than serve sends at once: the oldest were left out. */
  truncated: boolean;
  operator: string;
  operator_initials: string;
  now_ms: number;
}

/** How many items a page asks for. */
export const PAGE_SIZE = 50;

/** The words the shell already uses for each ending. */
export const ENDING_KEY: Record<Ending, TranslationKey<"history">> = {
  finished: "endingFinished",
  closed: "endingClosed",
  verified: "endingVerified",
  no_closure: "endingNone",
};

/** Why an archived conversation has no composer. */
export function readOnlyNotice(): string {
  return t("readOnlyNotice");
}

export function endingText(item: Pick<HistoryItem, "ending">): string {
  return item.ending ? t(ENDING_KEY[item.ending]) : t("endingOngoing");
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** "menos de 1 min", "45 min", "2 h 5 min", "3 d 4 h". */
export function lasted(ms: number): string {
  const m = Math.floor(Math.max(0, ms) / MIN);
  if (m < 1) return t("underOneMinute");
  if (m < 60) return `${m} min`;
  if (ms < DAY) {
    const h = Math.floor(m / 60);
    const rest = m % 60;
    return rest ? `${h} h ${rest} min` : `${h} h`;
  }
  const d = Math.floor(ms / DAY);
  const h = Math.floor((ms % DAY) / HOUR);
  return h ? `${d} d ${h} h` : `${d} d`;
}

function shortDate(ms: number): string {
  return new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), day: "numeric", month: "short" }).format(ms).replace(".", "");
}

/** "08:30 – 10:00 · 1 h 30 min", with the dates when it crossed midnight. */
export function spanText(item: Pick<HistoryItem, "start_ms" | "end_ms" | "duration_ms">, now: number): string {
  const sameDay = dayLabel(item.start_ms, now) === dayLabel(item.end_ms, now);
  const from = sameDay ? hhmm(item.start_ms) : `${shortDate(item.start_ms)} ${hhmm(item.start_ms)}`;
  const to = sameDay ? hhmm(item.end_ms) : `${shortDate(item.end_ms)} ${hhmm(item.end_ms)}`;
  return `${from} – ${to} · ${lasted(item.duration_ms)}`;
}

/** Items under "Today", "Yesterday" and then the date, by the day each ended, newest first. */
export function historyByDay(items: HistoryItem[], now: number): { day: string; items: HistoryItem[] }[] {
  const out: { day: string; items: HistoryItem[] }[] = [];
  for (const item of [...items].sort((a, b) => b.end_ms - a.end_ms)) {
    const day = dayLabel(item.end_ms, now);
    const last = out[out.length - 1];
    if (last && last.day === day) last.items.push(item);
    else out.push({ day, items: [item] });
  }
  return out;
}

/** The query string of a page: only what was chosen. */
export function historyQuery(opts: { before?: string | null; channel?: string | null; q?: string | null; limit?: number }): string {
  const params = new URLSearchParams();
  if (opts.limit != null) params.set("limit", String(opts.limit));
  if (opts.before) params.set("before", opts.before);
  if (opts.channel?.trim()) params.set("channel", opts.channel.trim());
  if (opts.q?.trim()) params.set("q", opts.q.trim());
  const qs = params.toString().replace(/\+/g, "%20");
  return qs ? `?${qs}` : "";
}

/** The next page after the ones already shown; an item seen twice stays where it was first. */
export function mergePage(shown: HistoryItem[], next: HistoryItem[]): HistoryItem[] {
  const seen = new Set(shown.map((i) => i.agent.key));
  return [...shown, ...next.filter((i) => !seen.has(i.agent.key))];
}

/**
 * How Direct opens an agent: live, with its composer, while the shell has
 * it and its session has not ended; read-only when the session ended or only
 * the archive has it. `archived` is undefined until the archive answers.
 */
export function directMode(
  live: Pick<WsAgent, "delivery"> | null | undefined,
  archived: HistoryItem | null | undefined,
): "live" | "read-only" | "loading" | "missing" {
  if (live) return live.delivery === "ended" ? "read-only" : "live";
  if (archived === undefined) return "loading";
  return archived ? "read-only" : "missing";
}
