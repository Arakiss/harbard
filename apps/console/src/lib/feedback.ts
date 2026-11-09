/**
 * One-tap feedback (`harbard_core::feedback`): what each target asks, when
 * nothing is asked at all, and the client's copy of the standing answers.
 *
 * A tap is never a modal: one row of controls, shown on hover or focus, one
 * row per target, never shown again once answered (until undone), and
 * nothing at all while the operator is Focused, Swamped or Away.
 */

import { createT } from "@/i18n";

const t = createT("feedback");

/** A name's 👍 / 👎 is not here: the workshop's NameVotes keep it in names.json. */
export type TargetKind = "message" | "delivery" | "needs_you" | "robot_state" | "dock";

export type FeedbackSample = { state: string; label?: string | null };

export type FeedbackRecord = {
  id: string;
  ms: number;
  target_kind: TargetKind;
  target_id: string;
  value: string;
  reason: string | null;
  sample: FeedbackSample | null;
  undone: boolean;
};

/** `GET /api/v1/feedback`. */
export type FeedbackView = {
  v: number;
  /** Whether a sink is configured (a stat), and whether Preferences allow it. */
  memory_sink?: { configured: boolean; allowed: boolean };
  standing: FeedbackRecord[];
  last: FeedbackRecord | null;
  totals: Record<string, Record<string, number>>;
  /** "Was it you?": open, and answered in the last days. */
  checks?: import("./checks").DeskCheck[];
};

export type FeedbackState = { standing: FeedbackRecord[]; last: FeedbackRecord | null };

export type FeedbackOption = {
  value: string;
  label: string;
  /** A thumb drawn instead of the label; the label stays as its name. */
  icon?: "up" | "down";
  /** Asks for one optional line before it is sent. */
  reason?: boolean;
};

export type Control = {
  question?: string;
  options: FeedbackOption[];
  /** What the acknowledgement says for each value. */
  ack: Record<string, string>;
};

export function controls(): Record<TargetKind, Control> {
  return {
    message: {
      options: [
        { value: "up", label: t("messageUp"), icon: "up" },
        { value: "unclear", label: t("messageUnclear"), icon: "down" },
      ],
      ack: { up: t("ackMessageUp"), unclear: t("ackMessageUnclear") },
    },
    delivery: {
      question: t("deliveryQuestion"),
      options: [
        { value: "yes", label: t("deliveryYes") },
        { value: "no", label: t("deliveryNo"), reason: true },
        { value: "missing", label: t("deliveryMissing"), reason: true },
      ],
      ack: { yes: t("ackDeliveryYes"), no: t("ackDeliveryNo"), missing: t("ackDeliveryMissing") },
    },
    needs_you: {
      options: [{ value: "not_needed", label: t("needsYouNotNeeded") }],
      ack: { not_needed: t("ackNeedsYouNotNeeded") },
    },
    robot_state: {
      options: [{ value: "wrong", label: t("robotWrong") }],
      ack: { wrong: t("ackRobotWrong") },
    },
    dock: {
      options: [{ value: "annoying", label: t("dockAnnoying") }],
      ack: { annoying: t("ackDockAnnoying") },
    },
  };
}

/** Coordination states (profile) in which nothing asks for feedback. */
const QUIET_FOR_FEEDBACK: readonly string[] = ["focused", "swamped", "away"];

export function feedbackAllowed(operatorState: string | null | undefined): boolean {
  return !operatorState || !QUIET_FOR_FEEDBACK.includes(operatorState);
}

/**
 * The one target a message row asks about: a DONE from an agent is a
 * delivery ("Is this useful?"), any other message from an agent, a peer or the
 * lead is a message (👍 / 👎), and the operator's own messages ask nothing.
 */
export function messageTarget(m: { from: string; label: string | null | undefined }): TargetKind | null {
  if (m.from === "operator") return null;
  if (m.from === "agent" && m.label === "DONE") return "delivery";
  return "message";
}

export function standingFor(state: FeedbackState, kind: TargetKind, id: string): FeedbackRecord | null {
  return state.standing.find((r) => r.target_kind === kind && r.target_id === id && !r.undone) ?? null;
}

export function applyTap(state: FeedbackState, rec: FeedbackRecord): FeedbackState {
  const others = state.standing.filter((r) => r.id !== rec.id);
  return { standing: [rec, ...others], last: rec };
}

export function applyUndo(state: FeedbackState, rec: FeedbackRecord): FeedbackState {
  const standing = state.standing.filter((r) => r.id !== rec.id);
  const last = state.last?.id === rec.id ? null : state.last;
  return { standing, last };
}

/** The Dock bounces when what needs the operator rises (`DesktopStatus.needs`). */
export function bounceRise(prevNeeds: number, needs: number): boolean {
  return needs > prevNeeds;
}

export function bounceTarget(atMs: number): string {
  return `bounce:${Math.floor(atMs)}`;
}

/** How long "This bothered me" stays offered after a bounce. */
export const BOUNCE_OFFER_MS = 30 * 60_000;
