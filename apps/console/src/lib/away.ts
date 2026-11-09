/**
 * "While you were away": what happened while the operator was
 * Away, folded from what harbard already knows. The open "Needs you"
 * cards and the notes the state held (`harbard_core::hold`) that ask
 * something of them, by urgency; what finished since they left; and who is
 * still at work. Nothing here is invented: an empty section stays empty.
 */

import { type TranslationKey, createT } from "@/i18n";
import type { DigestItem } from "./profile";
import { URGENCY, messageById, needsItems, type WsAgent, type WsCard, type WorkspaceView } from "./workspace";

const t = createT("home");

const MIN = 60_000;

export interface AwayWaiting {
  /** `card:<id>` or `note:<id>`. */
  id: string;
  kind: WsCard["kind"] | "handoff" | "warning" | "ask";
  /** The agent's name, or a short session id. */
  who: string;
  /** The agent's key when the workspace knows it, for its avatar and Direct. */
  agent: string | null;
  text: string;
  reason: string;
  /** The chip's tone (`.hb-chip[data-tone]`). */
  tone: "red" | "amber" | "neutral";
}

export interface AwayDone {
  id: string;
  who: string;
  text: string;
}

export interface AwayModel {
  waiting: AwayWaiting[];
  done: AwayDone[];
  running: WsAgent[];
}

const REASON: Record<AwayWaiting["kind"], TranslationKey<"home">> = {
  blocked: "reasonBlocked",
  approval: "reasonApproval",
  handoff: "reasonHandoff",
  warning: "reasonWarning",
  ask: "reasonAsk",
  mention: "reasonMention",
  unanswered: "reasonUnanswered",
  two_writers: "reasonTwoWriters",
  dormant: "reasonDormant",
  check: "reasonCheck",
};

const TONE: Record<AwayWaiting["kind"], AwayWaiting["tone"]> = {
  blocked: "red",
  approval: "amber",
  handoff: "amber",
  warning: "amber",
  ask: "neutral",
  mention: "amber",
  unanswered: "neutral",
  two_writers: "neutral",
  dormant: "neutral",
  check: "amber",
};

/** "1 h 12 min", or null under a minute: there is no "0 min". */
export function awaySpan(since: number, now: number): string | null {
  const m = Math.max(0, Math.round((now - since) / MIN));
  if (m < 1) return null;
  if (m < 60) return `${m} min`;
  if (m < 24 * 60) return m % 60 === 0 ? `${m / 60} h` : `${Math.floor(m / 60)} h ${m % 60} min`;
  return `${Math.round(m / 60)} h`;
}

/** "While you were away · 1 h 12 min". */
export function awayTitle(since: number, now: number): string {
  const span = awaySpan(since, now);
  return span ? `${t("whileAway")} · ${span}` : t("whileAway");
}

function firstLine(text: string): string {
  return text.split("\n")[0]?.trim() ?? "";
}

/** The agent a bus session speaks for: the one whose lead it is, if the workspace knows it. */
function agentOf(view: WorkspaceView | null, session: string): WsAgent | null {
  return view?.agents.find((a) => a.lead_session === session || a.key === session) ?? null;
}

export function awayModel({
  view,
  held,
  since,
  now,
}: {
  view: WorkspaceView | null;
  held: DigestItem[];
  since: number;
  now: number;
}): AwayModel {
  const waiting: AwayWaiting[] = [];
  // Needs you's own order (AC8); the notes the state held slot in by kind.
  for (const c of view ? needsItems(view) : []) {
    const first = c.agents[0] ?? null;
    const a = first ? (view?.agents.find((x) => x.key === first) ?? null) : null;
    const msg = c.message && view ? messageById(view, c.message) : null;
    waiting.push({
      id: `card:${c.id}`,
      kind: c.kind,
      who: a?.name ?? first ?? "harbard",
      agent: a?.key ?? null,
      text: msg ? firstLine(msg.text) : "",
      reason: t(REASON[c.kind]),
      tone: TONE[c.kind],
    });
  }
  const done: AwayDone[] = [];
  for (const n of held) {
    const a = agentOf(view, n.session);
    const who = a?.name ?? n.session.slice(0, 8);
    if (n.kind === "handoff" || n.kind === "warning" || n.kind === "ask") {
      waiting.push({ id: `note:${n.id}`, kind: n.kind, who, agent: a?.key ?? null, text: n.text, reason: t(REASON[n.kind]), tone: TONE[n.kind] });
    } else {
      done.push({ id: `note:${n.id}`, who, text: n.text });
    }
  }
  waiting.sort((x, y) => URGENCY[x.kind] - URGENCY[y.kind]);
  for (const m of view?.messages ?? []) {
    if (m.label !== "DONE" || m.ts_ms < since || m.ts_ms > now) continue;
    done.push({ id: `msg:${m.id}`, who: view?.agents.find((a) => a.key === m.agent)?.name ?? m.agent, text: firstLine(m.text) });
  }
  for (const d of view?.deliveries ?? []) {
    if (d.verdict !== "landed" || d.decided_ms == null || d.decided_ms < since) continue;
    done.push({ id: `delivery:${d.id}`, who: d.channel ? `#${d.channel}` : "", text: t("deliveryVerified", { branch: d.branch }) });
  }
  const running = (view?.agents ?? []).filter((a) => a.presence === "working" && !a.lost);
  return { waiting, done, running };
}
