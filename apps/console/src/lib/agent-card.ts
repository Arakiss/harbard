/**
 * The agent hover card: what it says, and when it opens and closes.
 *
 * It says only what serve read in the transcript: the brief, the face's
 * safe label, the last steps, and real progress signals (time since start,
 * tool calls, commits, the last test counts). There is no percentage and no
 * bar, because nothing measures one.
 */

import { createT } from "@/i18n";
import { elapsed } from "./activity";
import type { Progress } from "./progress";
import { agentName, dotTone, presenceText, type WsAgent } from "./workspace";

const t = createT("agents");

/** The card opens this long after the pointer or the focus arrives. */
export const OPEN_DELAY_MS = 300;
/** Leaving the trigger for the card is not leaving: this much grace. */
export const CLOSE_GRACE_MS = 150;

/** One piece of the "Progreso" line: a number in bold and its words. */
export interface ProgressPart {
  key: "running" | "steps" | "commits" | "testing";
  /** Words before the numbers ("En marcha", "Pruebas:"). */
  lead?: string;
  /** Numbers and their words, the numbers in bold. */
  values: { n: string; words: string }[];
  tone?: "ok" | "fail";
}

export interface CardModel {
  name: string;
  /** The spawn description, else the prompt's first line. */
  brief: string | null;
  /** The channels it works in, for the team pill. */
  teams: string[];
  /** Its agent type when it says more than `general-purpose`. */
  role: string | null;
  presence: { text: string; tone: "green" | "amber" | "red" | "grey" };
  progress: ProgressPart[];
}

/** "general-purpose · aa18a5f8" is the label; the type is its first part. */
function agentType(agent: Pick<WsAgent, "label">): string | null {
  const type = agent.label.split(" · ")[0]?.trim();
  return type ? type : null;
}

export function cardModel(
  agent: Pick<WsAgent, "name" | "label" | "kind" | "channels" | "description" | "brief" | "first_ms" | "lost" | "dormant" | "presence" | "last_action_ms">,
  progress: Progress | null | undefined,
  now: number,
): CardModel {
  const type = agentType(agent);
  const parts: ProgressPart[] = [];
  if (agent.first_ms) parts.push({ key: "running", lead: t("progressRunning"), values: [{ n: elapsed(now - agent.first_ms), words: "" }] });
  if (progress) {
    parts.push({ key: "steps", values: [{ n: String(progress.tool_calls), words: progress.tool_calls === 1 ? t("stepOne") : t("stepMany") }] });
    parts.push({ key: "commits", values: [{ n: String(progress.commits), words: progress.commits === 1 ? t("commitOne") : t("commitMany") }] });
    if (progress.test) {
      const run = progress.test;
      parts.push({
        key: "testing",
        lead: t("testsLead"),
        values: [
          { n: String(run.passed), words: run.passed === 1 ? t("passOne") : t("passMany") },
          { n: String(run.failed), words: run.failed === 1 ? t("failOne") : t("failMany") },
          ...(run.skipped > 0 ? [{ n: String(run.skipped), words: run.skipped === 1 ? t("skipOne") : t("skipMany") }] : []),
        ],
        tone: run.ok ? "ok" : "fail",
      });
    }
  }
  return {
    name: agentName(agent),
    brief: (agent.description ?? agent.brief)?.trim() || null,
    teams: agent.channels.slice(0, 2),
    // A teammate's type is often its own name: no pill says it twice.
    role: type && type !== "general-purpose" && type !== agent.name ? type : null,
    presence: { text: agent.lost ? t("noData") : presenceText(agent.presence), tone: agent.lost ? "grey" : dotTone(agent) },
    progress: parts,
  };
}

/** The "Progreso" line as plain text: "En marcha 2 h 14 min · 386 pasos · …". */
export function progressText(parts: ProgressPart[]): string {
  return parts
    .map((p) => [p.lead, p.values.map((v) => `${v.n}${v.words ? ` ${v.words}` : ""}`).join(", ")].filter(Boolean).join(" "))
    .join(" · ");
}

// ---------------------------------------------------------------------------
// Opening and closing
// ---------------------------------------------------------------------------

export interface Timers {
  set: (fn: () => void, ms: number) => unknown;
  clear: (t: unknown) => void;
}

export interface HoverIntent {
  /** The pointer or the focus reached the trigger. */
  enter: () => void;
  /** It left the trigger. */
  leave: () => void;
  /** The pointer or the focus reached the card. */
  enterCard: () => void;
  leaveCard: () => void;
  /** Esc: close now. */
  escape: () => void;
  /** Close now (another card opened, the trigger went away). */
  close: () => void;
  isOpen: () => boolean;
}

/**
 * Open after [`OPEN_DELAY_MS`], close on leave, Esc closes. Moving from
 * the trigger into the card keeps it open; leaving both closes it after
 * [`CLOSE_GRACE_MS`]. Pointer and keyboard focus drive the same machine.
 */
export function createHoverIntent(onChange: (open: boolean) => void, timers: Timers, delay = OPEN_DELAY_MS): HoverIntent {
  let open = false;
  let inTrigger = false;
  let inCard = false;
  let opening: unknown = null;
  let closing: unknown = null;
  const set = (v: boolean) => {
    if (open === v) return;
    open = v;
    onChange(v);
  };
  const cancel = () => {
    if (opening !== null) timers.clear(opening);
    if (closing !== null) timers.clear(closing);
    opening = closing = null;
  };
  const settle = () => {
    if (inTrigger || inCard) {
      if (closing !== null) timers.clear(closing);
      closing = null;
      if (!open && opening === null) {
        opening = timers.set(() => {
          opening = null;
          if (inTrigger || inCard) set(true);
        }, delay);
      }
      return;
    }
    if (opening !== null) timers.clear(opening);
    opening = null;
    if (open && closing === null) {
      closing = timers.set(() => {
        closing = null;
        if (!inTrigger && !inCard) set(false);
      }, CLOSE_GRACE_MS);
    }
  };
  const shut = () => {
    cancel();
    inTrigger = inCard = false;
    set(false);
  };
  return {
    enter() {
      inTrigger = true;
      settle();
    },
    leave() {
      inTrigger = false;
      settle();
    },
    enterCard() {
      inCard = true;
      settle();
    },
    leaveCard() {
      inCard = false;
      settle();
    },
    escape: shut,
    close: shut,
    isOpen: () => open,
  };
}

/** Where a card of `size` goes beside `rect`, inside a `view` of the window. */
export function placeCard(
  rect: { left: number; right: number; top: number; bottom: number },
  size: { width: number; height: number },
  view: { width: number; height: number },
  gap = 10,
): { left: number; top: number; side: "right" | "below" | "left"; arrow: number } {
  const clampY = (y: number) => Math.max(8, Math.min(y, view.height - size.height - 8));
  const clampX = (x: number) => Math.max(8, Math.min(x, view.width - size.width - 8));
  // Where the little arrow points: the trigger's middle, kept off the corners.
  const along = (mid: number, start: number, length: number) => Math.round(Math.max(22, Math.min(mid - start, length - 22)));
  const midY = (rect.top + rect.bottom) / 2;
  if (rect.right + gap + size.width + 8 <= view.width) {
    const top = clampY(rect.top - 8);
    return { left: rect.right + gap, top, side: "right", arrow: along(midY, top, size.height) };
  }
  if (rect.bottom + gap + size.height + 8 <= view.height) {
    const left = clampX(rect.left);
    return { left, top: rect.bottom + gap, side: "below", arrow: along((rect.left + rect.right) / 2, left, size.width) };
  }
  const top = clampY(rect.top - 8);
  return { left: clampX(rect.left - gap - size.width), top, side: "left", arrow: along(midY, top, size.height) };
}
