/**
 * The desktop's away suggestion (`apps/desktop/src-tauri/src/away.rs`): after
 * ten minutes away from the Mac or a locked screen it offers Away, and on
 * the way back it offers Available. This module only words it and refuses
 * what makes no sense; the state changes on the operator's click, never here.
 */

import { createT } from "@/i18n";
import type { StateId } from "./profile";

const t = createT("workspace");

export interface Suggestion {
  target: "away" | "available";
  title: string;
  body: string;
  accept: string;
  dismiss: string;
  /** The state's tone (`STATES`), for the card's icon. */
  tone: "grey" | "green";
}

export function suggestionFor(detail: unknown, current: StateId, enabled: boolean): Suggestion | null {
  if (!enabled || !detail || typeof detail !== "object") return null;
  const target = (detail as { state?: unknown }).state;
  if (target === "away") {
    if (current === "away" || current === "done") return null;
    return {
      target,
      title: t("suggestAwayTitle"),
      body: t("suggestAwayBody"),
      accept: t("suggestAwayAccept"),
      dismiss: t("suggestNotNow"),
      tone: "grey",
    };
  }
  if (target === "available") {
    if (current !== "away") return null;
    return {
      target,
      title: t("suggestBackTitle"),
      body: t("suggestBackBody"),
      accept: t("suggestBackAccept"),
      dismiss: t("suggestStayAway"),
      tone: "green",
    };
  }
  return null;
}
