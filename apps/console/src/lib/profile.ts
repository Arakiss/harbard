/**
 * The operator's profile, status line and "Do not disturb", as
 * `GET /api/v1/profile` serves them (`harbard_core::profile`).
 *
 * serve stores and validates; this module only words it in the operator's
 * language and turns the menu's choices (30 minutes, tomorrow at 9:00) into
 * the moments serve stores, in the operator's own clock.
 */

import { createT } from "@/i18n";

const t = createT("profile");

export interface ProfileStatus {
  emoji: string;
  text: string;
  clear_at_ms?: number | null;
}

export interface ProfileHours {
  start: string;
  end: string;
  days: number[];
}

export interface Profile {
  name?: string | null;
  address?: string | null;
  time_zone?: string | null;
  hours?: ProfileHours | null;
  context?: string | null;
  status?: ProfileStatus | null;
  dnd?: { since_ms: number; until_ms?: number | null } | null;
  state?: { id: StateId; since_ms: number; until_ms?: number | null } | null;
  dnd_default?: DndChoice | null;
  digest_hourly?: boolean;
  away_suggest?: boolean;
  utc_offset_min?: number | null;
  photo?: string | null;
  photo_rev: number;
  updated_ms: number;
}

export interface ProfileView {
  profile: Profile;
  /** In force now: an expired status is already null. */
  status: ProfileStatus | null;
  quiet: boolean;
  quiet_until_ms: number | null;
  /** The coordination state in force (Available when none or ended). */
  state: StateId;
  state_until_ms: number | null;
  /** What the Dock may do in that state (`harbard_core::profile::dock_rules`). */
  dock: { bounce: boolean; alerts: boolean; one_shots: boolean };
  /** Present on a write that changed state: what the state left held. */
  digest?: Digest | null;
  now_ms: number;
  limits: { context: number; name: number; address: number; status_text: number; photo_bytes: number };
}

/** What a change may carry; `null` clears a field. */
export type ProfilePatch = Partial<{
  name: string | null;
  address: string | null;
  time_zone: string | null;
  hours: ProfileHours | null;
  context: string | null;
  status: ProfileStatus | null;
  dnd: { until_ms: number } | null;
  state: { id: StateId; until_ms?: number | null } | null;
  dnd_default: DndChoice | null;
  digest_hourly: boolean;
  away_suggest: boolean;
  utc_offset_min: number | null;
  photo: null;
}>;

export const CONTEXT_MAX = 1500;

export type StateId = "available" | "focused" | "swamped" | "away" | "reviewing" | "done";

export interface StateInfo {
  id: StateId;
  label: string;
  hint: string;
  tone: "green" | "accent" | "red" | "grey" | "blue" | "amber";
}

/**
 * The coordination states, in the menu's order, with the menu's copy
 * and a colour tone. What each one does to the Dock is serve's
 * (`dock`); what it asks of the agents comes with the brief. A function
 * because the words follow the language.
 */
export function states(): readonly StateInfo[] {
  return [
    { id: "available", label: t("stateAvailable"), hint: t("stateAvailableHint"), tone: "green" },
    { id: "focused", label: t("stateFocused"), hint: t("stateFocusedHint"), tone: "accent" },
    { id: "swamped", label: t("stateSwamped"), hint: t("stateSwampedHint"), tone: "red" },
    { id: "away", label: t("stateAway"), hint: t("stateAwayHint"), tone: "grey" },
    { id: "reviewing", label: t("stateReviewing"), hint: t("stateReviewingHint"), tone: "blue" },
    { id: "done", label: t("stateDoneForDay"), hint: t("stateDoneForDayHint"), tone: "amber" },
  ];
}

const QUIET_STATES: readonly StateId[] = ["swamped", "away", "done"];

/** The state in force at `now`, on this browser's clock too. */
export function stateOf(view: ProfileView | null, now: number): StateInfo {
  const ended = view?.state_until_ms != null && now >= view.state_until_ms;
  const id = !view || ended ? "available" : view.state;
  const all = states();
  return all.find((s) => s.id === id) ?? all[0]!;
}

/** The Dock's one-shot animations: off in the quiet states and under Do not disturb. */
export function oneShotsAllowed(state: StateId, quiet: boolean): boolean {
  return !quiet && !QUIET_STATES.includes(state);
}

/**
 * Done for the day ends at the next start of the working hours (today's, if
 * it has not come yet); without hours, tomorrow at 9:00.
 */
export function doneUntil(hours: ProfileHours | null | undefined, now: Date): number {
  if (!hours || hours.days.length === 0) return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9, 0).getTime();
  const [h, m] = hours.start.split(":").map(Number);
  for (let d = 0; d <= 7; d++) {
    const at = new Date(now.getFullYear(), now.getMonth(), now.getDate() + d, h ?? 9, m ?? 0);
    const iso = ((at.getDay() + 6) % 7) + 1;
    if (at.getTime() > now.getTime() && hours.days.includes(iso)) return at.getTime();
  }
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9, 0).getTime();
}
const MIN = 60_000;
const DND_MAX_MS = 14 * 24 * 60 * MIN;

/** Suggestions for the free status message; the coordination states are apart (`states`). */
export function statusPresets(): readonly ProfileStatus[] {
  return [
    { emoji: "💻", text: t("presetAtComputer") },
    { emoji: "📅", text: t("presetInMeeting") },
    { emoji: "🍽️", text: t("presetEating") },
    { emoji: "🚆", text: t("presetOnMyWay") },
  ];
}

export type ClearAfter = "nunca" | "30m" | "1h" | "4h" | "today";
export function clearAfterChoices(): readonly { id: ClearAfter; label: string }[] {
  return [
    { id: "nunca", label: t("clearNever") },
    { id: "30m", label: t("duration30Minutes") },
    { id: "1h", label: t("duration1Hour") },
    { id: "4h", label: t("duration4Hours") },
    // i18n-exempt: the choice's id, not copy.
    { id: "today", label: t("clearToday") },
  ];
}

export type DndChoice = "30m" | "1h" | "tomorrow";
export function dndChoices(): readonly { id: DndChoice; label: string }[] {
  return [
    { id: "30m", label: t("duration30Minutes") },
    { id: "1h", label: t("duration1Hour") },
    { id: "tomorrow", label: t("dndUntilTomorrow") },
  ];
}

export function weekdays(): readonly { day: number; short: string; long: string }[] {
  return [
    { day: 1, short: t("mondayShort"), long: t("monday") },
    { day: 2, short: t("tuesdayShort"), long: t("tuesday") },
    { day: 3, short: t("wednesdayShort"), long: t("wednesday") },
    { day: 4, short: t("thursdayShort"), long: t("thursday") },
    { day: 5, short: t("fridayShort"), long: t("friday") },
    { day: 6, short: t("saturdayShort"), long: t("saturday") },
    { day: 7, short: t("sundayShort"), long: t("sunday") },
  ];
}

/** When a status chosen now clears; null never. "hoy" ends at midnight. */
export function clearAt(choice: ClearAfter, now: Date): number | null {
  const t = now.getTime();
  switch (choice) {
    case "nunca":
      return null;
    case "30m":
      return t + 30 * MIN;
    case "1h":
      return t + 60 * MIN;
    case "4h":
      return t + 240 * MIN;
    // i18n-exempt: the choice's id, not copy.
    case "today":
      return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1).getTime();
  }
}

/** When "Do not disturb" chosen now ends. "Tomorrow" is the next calendar day, 9:00. */
export function dndUntil(choice: DndChoice, now: Date): number {
  const t = now.getTime();
  if (choice === "30m") return t + 30 * MIN;
  if (choice === "1h") return t + 60 * MIN;
  return new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9, 0).getTime();
}

/** A `datetime-local` value as the end of "Do not disturb", or null if it is not ahead or too far. */
export function customUntil(value: string, now: Date): number | null {
  if (!value) return null;
  const t = new Date(value).getTime();
  if (!Number.isFinite(t) || t <= now.getTime() || t - now.getTime() > DND_MAX_MS) return null;
  return t;
}

/** Characters as serve counts them (Unicode scalars), `\r\n` as one. */
export function contextCount(text: string): number {
  return [...text.replace(/\r\n/g, "\n")].length;
}

/** As serve's `CONTEXT_MAX_QUOTED_BYTES`: the context as the brief quotes it. */
const CONTEXT_MAX_QUOTED_BYTES = 3200;

/** "> " and a newline per line, in UTF-8 bytes, as `harbard_core::profile::quoted_bytes`. */
export function contextQuotedBytes(text: string): number {
  const enc = new TextEncoder();
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .reduce((n, line) => n + enc.encode(line).length + 3, 0);
}

/** Within both caps: serve accepts it, and the agents get it whole. */
export function contextFits(text: string): boolean {
  return contextCount(text) <= CONTEXT_MAX && contextQuotedBytes(text.trim()) <= CONTEXT_MAX_QUOTED_BYTES;
}

/** The status in force at `now`: serve already drops expired ones, the clock may have moved since. */
export function effectiveStatus(view: ProfileView | null, now: number): ProfileStatus | null {
  const s = view?.status ?? null;
  if (!s || (s.clear_at_ms != null && now >= s.clear_at_ms)) return null;
  return s;
}

export function isQuiet(view: ProfileView | null, now: number): boolean {
  if (!view?.quiet) return false;
  return view.quiet_until_ms == null || now < view.quiet_until_ms;
}

function hm(d: Date): string {
  return `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** A weekday as JS numbers it, Sunday first. */
function weekdayName(jsDay: number): string {
  switch (jsDay) {
    case 0:
      return t("sunday");
    case 1:
      return t("monday");
    case 2:
      return t("tuesday");
    case 3:
      return t("wednesday");
    case 4:
      return t("thursday");
    case 5:
      return t("friday");
    default:
      return t("saturday");
  }
}

/** Which of the three wordings a moment takes: today, tomorrow or a dated day. */
function dayOf(until: Date, now: Date): "today" | "tomorrow" | "dated" {
  if (sameDay(until, now)) return "today";
  return sameDay(until, new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)) ? "tomorrow" : "dated";
}

/** "until 18:30", "until tomorrow at 9:00", "until Thursday 12 at 9:00". */
export function quietLabel(untilMs: number, now: Date): string {
  const until = new Date(untilMs);
  const time = hm(until);
  const day = dayOf(until, now);
  if (day === "today") return t("untilTime", { time });
  if (day === "tomorrow") return t("untilTomorrow", { time });
  return t("untilDated", { weekday: weekdayName(until.getDay()), day: until.getDate(), time });
}

/** "Clears at 17:30", or null when it stays. */
export function statusExpiry(clearMs: number | null | undefined, now: Date): string | null {
  if (clearMs == null) return null;
  const until = new Date(clearMs);
  const time = hm(until);
  const day = dayOf(until, now);
  if (day === "today") return t("clearsAt", { time });
  if (day === "tomorrow") return t("clearsTomorrow", { time });
  return t("clearsDated", { weekday: weekdayName(until.getDay()), day: until.getDate(), time });
}

/** Minutes east of UTC, for serve to say times in the operator's clock. */
export function utcOffsetMin(now: Date): number {
  return -now.getTimezoneOffset();
}

/** Every IANA zone the browser knows, the operator's first. */
export function timeZones(current: string): string[] {
  let all: string[] = [];
  try {
    all = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.("timeZone") ?? [];
  } catch {
    all = [];
  }
  return [current, ...all.filter((z) => z !== current)];
}

/** One held note in a digest (`harbard_core::hold::DigestItem`). */
export interface DigestItem {
  id: string;
  session: string;
  harness: string;
  kind: "fyi" | "ask" | "warning" | "handoff" | string;
  text: string;
  ts_ms: number;
}

export interface Digest {
  id: string;
  state: StateId;
  reason: "left" | "reviewing" | "hourly";
  from_ms: number;
  to_ms: number;
  items: DigestItem[];
  read: boolean;
}

export interface Digests {
  items: Digest[];
  cut_ms: number;
}
