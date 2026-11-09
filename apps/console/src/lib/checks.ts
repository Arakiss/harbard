/**
 * "Was it you?": the changes to what every session reads — a
 * rule approved, the context changed, the memory sink switched on — open until the operator
 * keeps or revokes them, and in view for a few days after (Retro,
 * Preferences), so a dismissed one still leaves a trace.
 */

import { createT, intlTag } from "@/i18n";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("workspace");

export type DeskCheck = {
  id: string;
  what: "rule" | "context" | "memory_sink" | string;
  text?: string;
  since_ms: number;
  /** `kept` or `revoked`; absent while unanswered. */
  outcome?: "kept" | "revoked" | string;
  answered_ms?: number;
};

function day(ms: number): string {
  return new Date(ms).toLocaleDateString(intlTag(), { day: "numeric", month: "long", timeZone: operatorTimeZone() });
}

function short(text: string): string {
  const t = text.replace(/\s+/g, " ").trim();
  return t.length > 80 ? `${t.slice(0, 79)}…` : t;
}

/** One line per check: what changed, and what the operator answered. */
export function checkLine(c: DeskCheck): string {
  const what =
    c.what === "rule"
      ? t("checkLineRule", { text: short(c.text ?? "") })
      : c.what === "context"
        ? c.text?.trim()
          ? t("checkLineContext", { text: short(c.text) })
          : t("checkLineContextCleared")
        : c.what === "memory_sink"
          ? t("checkLineMemorySink")
          : t("checkLineOther");
  const answer =
    c.outcome === "kept"
      ? t("checkLineKept", { day: day(c.answered_ms ?? c.since_ms) })
      : c.outcome === "revoked"
        ? t("checkLineRevoked", { day: day(c.answered_ms ?? c.since_ms) })
        : t("checkLineUnanswered");
  return `${what} · ${day(c.since_ms)} · ${answer}`;
}
