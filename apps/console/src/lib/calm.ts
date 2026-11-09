/**
 * "Calm mode": in Swamped the console shows only the one thing
 * that needs the operator now, the top of Needs you, and a calm line;
 * everything else is collapsed. "Later" moves on to the next item, and
 * "Exit" leaves calm mode for this Swamped without changing the state.
 */

import { createT } from "@/i18n";
import { awayModel, type AwayWaiting } from "./away";
import type { StateId } from "./profile";
import { hhmm, sessionGone, sessionGoneNotice, type WorkspaceView } from "./workspace";

const t = createT("home");

export interface CalmItem extends AwayWaiting {
  /** The first line of what it asks. */
  title: string;
  /** The rest, in one quiet line. */
  detail: string;
  /** The message is the operator's own ("No reply"): quoted as theirs, never as the agent's question. */
  own: boolean;
  /** His message, when `own`. */
  quote: string | null;
}

export interface CalmModel {
  top: CalmItem | null;
  /** Open items in Needs you, the top one included. */
  waiting: number;
  /** Agents at work. */
  working: number;
}

/** On in Swamped, unless they left it for this Swamped (`exitedSince` is that Swamped's start). */
export function calmOn(state: StateId, since: number, exitedSince: number | null): boolean {
  return state === "swamped" && exitedSince !== since;
}

export function calmModel(view: WorkspaceView | null, later: Set<string>, now: number): CalmModel {
  const m = awayModel({ view, held: [], since: now, now });
  const next = m.waiting.find((w) => !later.has(w.id)) ?? null;
  let top: CalmItem | null = null;
  if (next) {
    const id = next.id.replace(/^card:/, "");
    const card = view?.cards.find((c) => c.id === id);
    const msg = card?.message ? view?.messages.find((x) => x.id === card.message) : undefined;
    const text = card?.message ? (msg?.text ?? "") : next.text;
    if (msg?.from === "operator") {
      top = {
        ...next,
        title: t("nobodyAnswered"),
        detail: t("youWroteTo", { who: next.who, time: hhmm(msg.ts_ms) }),
        own: true,
        quote: text.trim(),
      };
    } else {
      const [first = "", ...rest] = text.split("\n");
      const a = next.agent ? view?.agents.find((x) => x.key === next.agent) : undefined;
      // Nobody will read an answer: say so before the rest (AC6).
      const detail = card?.kind === "blocked" && a && sessionGone(a) ? `${sessionGoneNotice()}.` : rest.join(" ").trim();
      top = { ...next, title: first.trim() || next.reason, detail, own: false, quote: null };
    }
  }
  return { top, waiting: m.waiting.length, working: m.running.length };
}
