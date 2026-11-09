/**
 * "Deliveries": every DONE an agent sent and where its commit is now, as
 * serve derives it (`harbard_core::shipments`): the DONE and its SHA, the
 * review agents that named that SHA, `git merge-base --is-ancestor` against
 * `main`, and the commit serve and the console run. This module only words
 * it and draws the steps; it invents no state.
 */

import { type TranslationKey, createT } from "@/i18n";
import { atDate, hhmm, type WorkspaceView } from "./workspace";

const t = createT("deliveries");

export type ShipState = "delivered" | "in_review" | "with_fixes" | "integrated" | "installed";
export type Untracked = "no_commit" | "no_repository" | "unknown_commit";

export interface WsReviewStep {
  agent: string;
  message: string;
  ts_ms: number;
  kind: "reviewing" | "fixes" | "reviewed";
}

export interface WsShipment {
  /** The DONE message. */
  id: string;
  agent: string;
  ts_ms: number;
  /** The DONE's human line. */
  text: string;
  channel: string | null;
  sha: string | null;
  commit: string | null;
  /** The id it reached main as, when a rebase gave it a new one. */
  landed_as?: string | null;
  state: ShipState;
  /** `installed` in serve's own repository; `integrated` in any other. */
  last_state: "installed" | "integrated";
  /** Not yet at `last_state`: what the sidebar counts. */
  pending: boolean;
  untracked: Untracked | null;
  review: WsReviewStep[];
}

/** The view header's sentence. */
export function deliveriesLine(): string {
  return t("line");
}

/** The sidebar count's tooltip. */
export function deliveriesHint(): string {
  return t("hint");
}

/** The secondary section, `harbard deliver` and `integrate`. */
export function queueLine(): string {
  return t("queueLine");
}

const STATE_LABEL: Record<ShipState, TranslationKey<"deliveries">> = {
  delivered: "stateDelivered",
  in_review: "stateInReview",
  with_fixes: "stateWithFixes",
  integrated: "stateIntegrated",
  installed: "stateInstalled",
};

export function stateLabel(state: ShipState): string {
  return t(STATE_LABEL[state]);
}

const ORDER: ShipState[] = ["delivered", "in_review", "with_fixes", "integrated", "installed"];

/** "Deliveries" in the sidebar: deliveries not installed yet (or not integrated, outside serve's repository). */
export function pendingCount(view: Pick<WorkspaceView, "shipments">): number {
  return (view.shipments ?? []).filter((s) => s.pending).length;
}

export interface Step {
  id: ShipState;
  label: string;
  /** This delivery went through it. A review it never had is skipped, not reached. */
  reached: boolean;
  /** Where it is now. */
  current: boolean;
}

/** The timeline of one delivery: four steps outside serve's repository, five in it. */
export function steps(s: Pick<WsShipment, "state" | "last_state" | "review">): Step[] {
  const upTo = ORDER.slice(0, ORDER.indexOf(s.last_state) + 1);
  const at = ORDER.indexOf(s.state);
  const reached: Record<ShipState, boolean> = {
    delivered: true,
    in_review: s.review.length > 0,
    with_fixes: s.review.some((r) => r.kind === "fixes") || s.state === "with_fixes",
    integrated: at >= ORDER.indexOf("integrated"),
    installed: s.state === "installed",
  };
  return upTo.map((id) => ({ id, label: stateLabel(id), reached: reached[id], current: id === s.state }));
}

/** When it was delivered, and when each review step happened. */
export function timesLine(s: Pick<WsShipment, "ts_ms" | "review">, now: number): string {
  const parts = [t("timesDelivered", { when: atDate(s.ts_ms, now) })];
  const first = (kind: WsReviewStep["kind"]) => s.review.find((r) => r.kind === kind);
  const started = first("reviewing");
  if (started) parts.push(t("timesReview", { time: hhmm(started.ts_ms) }));
  const fixes = first("fixes");
  if (fixes) parts.push(t("timesFixes", { time: hhmm(fixes.ts_ms) }));
  const clean = [...s.review].reverse().find((r) => r.kind === "reviewed");
  if (clean && !fixes) parts.push(t("timesCleanReview", { time: hhmm(clean.ts_ms) }));
  return parts.join(" · ");
}

export function untrackedText(why: Untracked | null): string | null {
  switch (why) {
    case "no_commit":
      return t("untrackedNoCommit");
    case "unknown_commit":
      return t("untrackedUnknownCommit");
    case "no_repository":
      return t("untrackedNoRepo");
    default:
      return null;
  }
}

const short = (c: string) => c.slice(0, 7);

/**
 * What is installed: the commit serve runs and the one the console was
 * built from. When they differ, the installed state may not hold for the console,
 * and the header says so.
 */
export function installedLine(view: Pick<WorkspaceView, "running_commit">, consoleCommit: string): { text: string; mismatch: boolean } {
  const serve = view.running_commit ?? null;
  if (!serve) return { text: t("installedUnknown"), mismatch: true };
  const same = consoleCommit && (serve.startsWith(consoleCommit) || consoleCommit.startsWith(serve));
  if (!consoleCommit || same) return { text: t("installedSame", { commit: short(serve) }), mismatch: false };
  return { text: t("installedMismatch", { serve: short(serve), console: short(consoleCommit) }), mismatch: true };
}

/** The commit this console was built from (`next.config.ts`), when it is a commit id. */
export function consoleCommit(): string {
  const c = process.env.NEXT_PUBLIC_HARBARD_COMMIT ?? "";
  return /^[0-9a-f]{7,40}$/i.test(c) ? c : "";
}
