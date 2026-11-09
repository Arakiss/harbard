/**
 * A channel the operator can read even when agents ignore the protocol.
 *
 * The protocol flags (no label, too long, jargon, repeat) stay in the view
 * model for the Retro; the channel shows at most one quiet grey marker,
 * "written for machines", with the reasons in its tooltip, and only on the
 * first flagged message of a run of one author.
 */
import { createT, type TranslationKey } from "@/i18n";
import { channelMembers } from "./members";
import { serveText } from "./serve-text";
import { channelWhy, shortPath, type Flag, type JargonReason, type ReadablePart, type WorkspaceView, type WsAgent, type WsChannel, type WsMessage } from "./workspace";

const t = createT("activity");

export function machineText(): string {
  return t("machineText");
}

const FLAG_REASON: Record<Exclude<Flag, "jargon">, TranslationKey<"activity">> = {
  unlabeled: "reasonUnlabeled",
  too_long: "reasonTooLong",
  repeat: "reasonRepeat",
};

const JARGON_REASON: Record<JargonReason, TranslationKey<"activity">> = {
  sha: "reasonSha",
  review_code: "reasonReviewCode",
  word: "reasonWord",
};

export interface MachineMark {
  /** Why, in plain words, in the server's order. */
  reasons: string[];
  /** The tooltip. */
  hint: string;
  /** Flagged messages of the same run after this one, which show no marker. */
  more: number;
}

function reasonsOf(m: Pick<WsMessage, "flags" | "jargon">): string[] {
  const out: string[] = [];
  for (const f of m.flags) {
    if (f === "jargon") {
      const why = m.jargon ?? [];
      out.push(...(why.length > 0 ? why.map((r) => t(JARGON_REASON[r])) : [t("reasonNotPlain")]));
    } else {
      out.push(t(FLAG_REASON[f]));
    }
  }
  return out;
}

function hintOf(reasons: string[], more: number): string {
  const head = t("machineHint", { reasons: joinList(reasons) });
  if (more === 0) return head;
  return `${head} ${more === 1 ? t("machineHintNextOne") : t("machineHintNextMany", { count: more })}`;
}

/** The marker of one message on its own (a thread pane, Later): null when clean. */
export function machineMark(m: WsMessage): MachineMark | null {
  if (m.from === "operator") return null;
  const reasons = reasonsOf(m);
  if (reasons.length === 0) return null;
  return { reasons, hint: hintOf(reasons, 0), more: 0 };
}

/**
 * The markers of a stream, by message id: one per run of one author, on its
 * first flagged message. A run ends when someone else writes.
 */
export function machineMarks(messages: WsMessage[]): Map<string, MachineMark> {
  const out = new Map<string, MachineMark>();
  let runAuthor: string | null = null;
  let runMark: { id: string; reasons: string[]; more: number } | null = null;
  const close = () => {
    if (runMark) out.set(runMark.id, { reasons: runMark.reasons, hint: hintOf(runMark.reasons, runMark.more), more: runMark.more });
  };
  const ids = new Set(messages.map((m) => m.id));
  for (const m of messages) {
    // A repeat is drawn under its first copy, not on its own.
    if (m.repeat_of && ids.has(m.repeat_of)) continue;
    const author = `${m.from}\u0000${m.agent}\u0000${m.author}`;
    if (author !== runAuthor) {
      close();
      runAuthor = author;
      runMark = null;
    }
    const mark = machineMark(m);
    if (!mark) continue;
    if (runMark) runMark.more += 1;
    else runMark = { id: m.id, reasons: mark.reasons, more: 0 };
  }
  close();
  return out;
}

function joinList(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return t("listJoin", { items: items.slice(0, -1).join(", "), last: items[items.length - 1] ?? "" });
}

/** One piece of a readable line: a harbard-worded one in this language. */
export function readablePartText(part: ReadablePart): string {
  return serveText(part.code, part.params, part.text);
}

/** A readable line as plain text, chips at their short form. */
export function readableText(parts: ReadablePart[]): string {
  return parts.map(readablePartText).join("");
}

/**
 * What "Technical details" holds: with a readable line shown in its place,
 * the original line first, then the body; otherwise the body alone.
 */
export function techBody(m: Pick<WsMessage, "text" | "body" | "readable">): string | null {
  if (m.readable && m.readable.length > 0) return m.body ? `${m.text}\n\n${m.body}` : m.text;
  return m.body ?? null;
}

/**
 * The channel header's line: "37 sessions today · 1 working now". Today
 * runs from local midnight; with nobody today it falls back to the last
 * 24 h, then to when the channel last had activity. `agents` are the
 * sessions the first half counts, for the list behind it.
 */
export function channelHeadline(view: WorkspaceView, channel: WsChannel): { sessions: string; now: string | null; agents: WsAgent[] } {
  const midnight = new Date(view.now_ms);
  midnight.setHours(0, 0, 0, 0);
  const recent = (channel.recent ?? []).map((k) => view.agents.find((a) => a.key === k)).filter((a): a is WsAgent => Boolean(a));
  const today = recent.filter((a) => (a.last_action_ms ?? 0) >= midnight.getTime());
  const working = channelMembers(view, channel).now;
  const now = working > 0 ? t("channelWorkingNow", { count: working }) : t("channelNobodyWorking");
  const sessions = (n: number) => (n === 1 ? t("channelSessionsOne", { count: n }) : t("channelSessionsMany", { count: n }));
  if (today.length > 0) return { sessions: t("channelHeadlineToday", { sessions: sessions(today.length) }), now, agents: today };
  if (recent.length > 0) return { sessions: t("channelHeadlineRecent", { sessions: sessions(recent.length) }), now, agents: recent };
  const why = channelWhy(view, channel);
  return { sessions: why.line, now: working > 0 ? now : null, agents: why.agents };
}

/** What the header's "ℹ" holds: the repository and each copy with its branch. */
export function channelPlaces(channel: WsChannel): { title: string; copies: { path: string; full: string; branch: string | null }[] } {
  const root = channel.repo.replace(/\/+$/, "");
  const where = (path: string) =>
    path === root ? t("placesMainCopy") : path.startsWith(`${root}/`) ? path.slice(root.length + 1) : shortPath(path);
  const copies = channel.checkouts.map((c) => ({ path: where(c.path), full: c.path, branch: c.branch || null }));
  const n = copies.length;
  const title = n <= 1 ? shortPath(channel.repo) : t("placesCopies", { count: n, repo: shortPath(channel.repo) });
  return { title, copies };
}

/** A stream as drawn: single messages, and runs of one kind from one team. */
export type StreamItem =
  | { kind: "message"; message: WsMessage }
  | { kind: "run"; id: string; label: string; older: WsMessage[]; newest: WsMessage }
  | { kind: "internal"; id: string; label: string; messages: WsMessage[] };

/**
 * An agent's report to the session that launched it, with no line written
 * for the operator: work chatter, kept out of their reading. What answers,
 * names someone or carries a file stays in the channel. A session's note
 * full of jargon is coordination between sessions, the same thing.
 */
export function isInternal(m: WsMessage): boolean {
  const toLead = m.from === "agent" && m.flags.includes("unlabeled");
  const betweenSessions = m.id.startsWith("bus:") && (m.jargon ?? []).length > 0;
  return (
    (toLead || betweenSessions) &&
    m.replies.length === 0 &&
    !m.reply_to &&
    (m.attachments?.length ?? 0) === 0 &&
    (m.mentions?.length ?? 0) === 0
  );
}

/** "5 internal notices · Naomi, Celia, Jason and 2 more". */
function internalLabel(view: Pick<WorkspaceView, "agents">, messages: WsMessage[]): string {
  const names = [...new Set(messages.map((m) => view.agents.find((a) => a.key === m.agent)?.name ?? m.author))];
  const shown = names.slice(0, 3).join(", ");
  const who = names.length > 3 ? t("namesAndMore", { names: shown, count: names.length - 3 }) : shown;
  return messages.length === 1 ? t("internalOne", { count: messages.length, names: who }) : t("internalMany", { count: messages.length, names: who });
}

/** The fewest messages that make a run. */
const RUN_MIN = 3;
/** Older lines of a run shown before "See all". */
export const RUN_SHOWN = 3;

function runCount(shape: NonNullable<WsMessage["shape"]>, count: number): string {
  switch (shape) {
    case "delivery":
      return count === 1 ? t("runDeliveryOne", { count }) : t("runDeliveryMany", { count });
    case "review":
      return count === 1 ? t("runReviewOne", { count }) : t("runReviewMany", { count });
    case "blocked":
      return count === 1 ? t("runBlockerOne", { count }) : t("runBlockerMany", { count });
  }
}

/**
 * Consecutive messages of one kind (deliveries, reviews, blocks) from one
 * team's agents collapse into "5 deliveries from acme": the older ones as a
 * compact list, the newest whole. A message with a thread or attachments,
 * an operator note, the lead or anything else ends the run; a repeat drawn
 * under its first copy neither joins nor ends it.
 */
export function runs(
  view: Pick<WorkspaceView, "agents">,
  messages: WsMessage[],
  opts: { foldInternal?: boolean } = {},
): StreamItem[] {
  const ids = new Set(messages.map((m) => m.id));
  const teamOf = (m: WsMessage) => view.agents.find((a) => a.key === m.agent)?.team ?? null;
  const keyOf = (m: WsMessage): string | null => {
    if (m.from !== "agent" && m.from !== "session" && m.from !== "peer") return null;
    if (!m.shape || m.replies.length > 0 || m.reply_to || (m.attachments?.length ?? 0) > 0) return null;
    return `${m.shape}\u0000${teamOf(m) ?? ""}`;
  };
  const out: StreamItem[] = [];
  let run: WsMessage[] = [];
  let runKey: string | null = null;
  let internal: WsMessage[] = [];
  const flushInternal = () => {
    if (internal.length > 0) {
      out.push({
        kind: "internal",
        id: `internal:${internal[0]!.id}`,
        label: internalLabel(view, internal),
        messages: internal,
      });
    }
    internal = [];
  };
  const flush = () => {
    if (run.length >= RUN_MIN) {
      const newest = run[run.length - 1]!;
      const counted = runCount(newest.shape!, run.length);
      const team = teamOf(newest);
      out.push({
        kind: "run",
        id: `run:${run[0]!.id}`,
        label: team ? t("runOfTeam", { run: counted, team }) : counted,
        older: run.slice(0, -1),
        newest,
      });
    } else {
      out.push(...run.map((message) => ({ kind: "message" as const, message })));
    }
    run = [];
    runKey = null;
  };
  for (const m of messages) {
    if (opts.foldInternal && isInternal(m)) {
      flush();
      internal.push(m);
      continue;
    }
    flushInternal();
    if (m.repeat_of && ids.has(m.repeat_of)) {
      // Drawn under its first copy: keep it where it was, outside any run.
      if (run.length === 0) out.push({ kind: "message", message: m });
      continue;
    }
    const key = keyOf(m);
    if (key !== null && key === runKey) {
      run.push(m);
      continue;
    }
    flush();
    if (key !== null) {
      run = [m];
      runKey = key;
    } else {
      out.push({ kind: "message", message: m });
    }
  }
  flush();
  flushInternal();
  return out;
}
