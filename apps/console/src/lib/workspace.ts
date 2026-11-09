/**
 * The workspace shell: channels, agents, messages and the cards that need the
 * operator, as `GET /api/v1/workspace` projects them.
 *
 * The server decides what is blocked, dormant, verified or two writers; this
 * module only words it, through the dictionaries, and does the small pure work the
 * views share (day groups, monograms, search). Nothing here invents a state
 * the server did not send.
 */

import { createT, intlTag, type TranslationKey } from "@/i18n";
import { type ServeParams, roleVars, serveText } from "./serve-text";
import type { WorkspaceTile } from "./current-workspace";
import type { Progress } from "./progress";
import type { Activity, Reaction, Thresholds } from "./activity";
import type { WsShipment } from "./deliveries";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("workspace");

export type Presence = "working" | "waiting" | "finished";
export type Label = "START" | "BLOCKED" | "FINDING" | "DONE";
export type Flag = "unlabeled" | "too_long" | "repeat" | "jargon";
/** A piece of a readable line: text, or a small grey chip with its whole value. */
export interface ReadablePart {
  kind: "text" | "sha" | "branch" | "path";
  text: string;
  full?: string;
  /** The console's code for a harbard-worded text, filled from `params`. */
  code?: string | null;
  params?: ServeParams;
}
export type JargonReason = "sha" | "review_code" | "word";
export type CardKind = "blocked" | "approval" | "dormant" | "two_writers" | "unanswered" | "mention" | "handoff" | "check";
export type Reach = "ready" | "stale" | "none" | "nobody";
export type Delivery = "ready" | "stale" | "ended" | "not_on_bus";

export interface WsChannel {
  id: string;
  repo: string;
  checkouts: { path: string; branch: string | null }[];
  members: string[];
  last_ms: number;
  warnings: { checkout: string; agents: string[]; since_ms: number }[];
  /** Sessions a note here is stored for: the members' leads and the sessions working in the repository. */
  readers?: { session: string; delivery: Delivery; lead: boolean }[];
  /** Whether a live session reads a note here now (`ready`), may come back (`stale`), or nobody will yet. */
  reach?: Reach;
  /** Newest write, message or live session; reading with git is not activity. */
  active_ms?: number;
  /** No activity in 14 days: under "More channels". */
  quiet?: boolean;
  /** Messages newer than the channel's read mark. */
  unread?: number;
  /** Who wrote or worked here as a session in the last 24 h, newest first: why the channel is here. */
  recent?: string[];
}

/** A file `harbard attach` copied, named in a message. */
export interface WsAttachment {
  /** `<ulid>/<name>`, what `GET /api/v1/attachments/<id>` serves. */
  id: string;
  name: string;
  kind: "image" | "markdown" | "diff" | "text" | "json" | "huginn";
  label: "before" | "after" | "report" | "code" | null;
  bytes: number;
  path: string;
}

/** Where one of the operator's notes stands, from the bus. */
export interface WsNoteDelivery {
  state: "sent" | "delivered" | "read" | "answered";
  sessions: string[];
  delivered: number;
  to_lead: boolean;
  nudged_ms: number | null;
  /** Who answered instead of the agent the note was for; absent from a serve older than the console. */
  answered_by?: string | null;
}

export interface WsAgent {
  /** Which workspace it lives in; only on the strip's whole-hub read. */
  workspace?: string;
  key: string;
  /** Its own name, or the first words of its brief when it has none. */
  name: string;
  /** Type and short id, for the profile. */
  label: string;
  /** Neither its lead nor itself wrote recently: grey, "no data since …". */
  lost: boolean;
  /** An in-process teammate or subagent (its lead relays), or a bus session of its own. */
  kind: "teammate" | "subagent" | "session";
  lead_session: string;
  /** How long the lead session has been silent on the bus. */
  lead_quiet_ms: number | null;
  delivery: Delivery;
  state: string;
  state_reason: string;
  presence: Presence;
  dormant: "amber" | "red" | null;
  last_action_ms: number | null;
  first_ms: number | null;
  brief: string | null;
  description: string | null;
  checkout: string | null;
  branch: string | null;
  build_dir: string | null;
  channels: string[];
  deliveries: number;
  verified_deliveries: number;
  reports: string[];
  unread: number;
  last_message: string | null;
  can_stop: boolean;
  /** The face: what serve read in the transcript, with dormant, blocked and verified laid over it. */
  activity: Activity;
  /** One-shot reactions of the last two minutes. */
  reactions: Reaction[];
  /** The hover card's last steps and real progress signals (absent from an older serve). */
  progress?: Progress;
  /** `robots` of "robots · constructor": what `@robots` reaches. */
  team?: string | null;
  role?: string | null;
  /** What an `@` names it by. */
  handle?: string;
  /** Its lead works in system temp space: listed only under "Sesiones temporales". */
  temporary?: boolean;
  /** The console's code for a `name` harbard worded, filled from `name_params`. */
  name_code?: string | null;
  name_params?: ServeParams;
  /** Under a workshop name: `<role> · <team>`, in the words of a known role. */
  role_line?: string | null;
  /** The console's code for a `role_line` built from a known role. */
  role_line_code?: string | null;
  role_line_params?: ServeParams;
  /** `auto`, `picked` (the session chose it) or `renamed` (the operator did). */
  name_source?: "auto" | "picked" | "renamed" | "launched" | null;
  /** The session's one-line reason for the name it picked. */
  name_reason?: string | null;
  /** The name the agent goes by itself (its Agent tool name). */
  own_name?: string | null;
  /** Its lead stopped it (TaskStop): "Closed". */
  closed?: boolean;
  /** The operator hid it ("Hide"): out of the lists; its cards still show (absent from an older serve). */
  hidden?: boolean;
}

export interface WsMessage {
  id: string;
  agent: string;
  from: "agent" | "lead" | "peer" | "operator" | "session";
  author: string;
  ts_ms: number;
  /** The human line. */
  text: string;
  /** Messages v2: the technical detail, under "Technical details". Null for v1 (absent from an older serve). */
  body?: string | null;
  label: Label | null;
  version: number | null;
  flags: Flag[];
  /** Why the human line reads as jargon (absent from an older serve). */
  jargon?: JargonReason[];
  /** A plain line shown in place of a line written for machines (absent from an older serve). */
  readable?: ReadablePart[] | null;
  /** `delivery`, `review` or `blocked`: a run of one kind from one team groups (absent from an older serve). */
  shape?: "delivery" | "review" | "blocked" | null;
  repeat_of: string | null;
  chars: number;
  files: string[];
  verified: { sha: string; branch: string; decided_ms: number } | null;
  channels: string[];
  reply_to: string | null;
  replies: string[];
  structured: string | null;
  undelivered: boolean;
  attachments?: WsAttachment[];
  /** For the operator's notes: sent, delivered, read, answered. */
  delivery?: WsNoteDelivery | null;
  /** The `@` tokens in it that name someone (highlighted). */
  mentions?: string[];
}

export interface WsCard {
  /** Which workspace it lives in; only on the strip's whole-hub read. */
  workspace?: string;
  id: string;
  kind: CardKind;
  channel: string | null;
  agents: string[];
  message: string | null;
  checkout: string | null;
  since_ms: number;
  resolved_ms: number | null;
  /** In "Needs you"; false for history such as an old overlap of writers. */
  needs: boolean;
  /** "Was it you?": what changed what every session reads, and its text. */
  check?: { what: "rule" | "context" | "memory_sink" | string; text: string };
}

export interface WsDelivery {
  id: string;
  channel: string | null;
  sha: string;
  branch: string;
  verdict: "landed" | "conflict" | "rejected" | "failed" | "pending";
  created_ms: number;
  decided_ms: number | null;
  session: string;
}

export interface WsSaved {
  id: string;
  kind: "message" | "report";
  saved_ms: number;
  agent: string | null;
  channel: string | null;
  text: string;
  ts_ms: number | null;
  path: string | null;
}

export interface WorkspaceView {
  v: number;
  now_ms: number;
  operator: string;
  operator_initials: string;
  thresholds: Thresholds;
  channels: WsChannel[];
  agents: WsAgent[];
  messages: WsMessage[];
  cards: WsCard[];
  deliveries: WsDelivery[];
  saved: WsSaved[];
  unread: { directs: number; activity: number; channels?: number; todo?: number };
  team_root_found: boolean;
  /** The sidebar's Agents section is folded (absent from an older serve). */
  agents_collapsed?: boolean;
  /** "Deliveries": every DONE and where its commit is now (absent from an older serve). */
  shipments?: WsShipment[];
  /** The commit serve runs, for "Installed". */
  running_commit?: string | null;
  format_changed: boolean;
  /** "reduce" when the operator turned motion off in Preferences. */
  motion: "reduce" | null;
  /** The strip dims after 22:00 in dark mode; false when turned off in Preferences (older serves omit it). */
  night_dim?: boolean;
  /** Changes with every message, reply, delivery state or card. */
  rev?: string;
  /** The workspace this view is for (absent from an older serve). */
  workspace?: string;
  /** One tile per workspace the console may show, with its badges. */
  workspaces?: WorkspaceTile[];
}

/** `GET /workspace?only=activity`: the faces' slice, polled every two seconds. */
export interface ActivityView {
  v: number;
  now_ms: number;
  thresholds: Thresholds;
  agents: { key: string; presence: Presence; dormant: "amber" | "red" | null; last_action_ms: number | null; activity: Activity; reactions: Reaction[] }[];
  unread: { directs: number; activity: number };
  /** Open items in "Needs you" and unread Direct, for the Dock. */
  needs: number;
  directs: number;
  blocked: number;
  motion: "reduce" | null;
  /** The full view's `rev`: when it differs, the console pulls the full view now. */
  rev?: string;
  /** The workspace this slice is for (absent from an older serve). */
  workspace?: string;
}

// ---------------------------------------------------------------------------
// The rail
// ---------------------------------------------------------------------------

export type Destination = "home" | "direct" | "activity" | "later";

/** The four destinations, always these, always in this order. */
export function railItems(): readonly { id: Destination; label: string; href: string }[] {
  return [
    { id: "home", label: t("railHome"), href: "/" },
    { id: "direct", label: t("railDirect"), href: "/direct" },
    { id: "activity", label: t("railActivity"), href: "/activity" },
    { id: "later", label: t("railLater"), href: "/later" },
  ];
}

/** Which destination a pathname belongs to. */
export function destinationOf(pathname: string): Destination | null {
  if (pathname === "/" || pathname.startsWith("/c/") || pathname.startsWith("/threads") || pathname.startsWith("/deliveries"))
    return "home";
  if (pathname.startsWith("/direct")) return "direct";
  if (pathname.startsWith("/activity")) return "activity";
  if (pathname.startsWith("/later")) return "later";
  return null;
}

/**
 * "More tools", at the end of Home's sidebar: what is left of the
 * previous console after its audit, each with what it is for. The previous
 * views (Overview, Inbox, Mensajes, Timeline, Team) were removed:
 * the shell covers them. History came back in the shell's design, because
 * the shell keeps only a day and nothing else showed the month before.
 */
export type MoreToolId = "strip" | "history" | "setup";

export function moreTools(): readonly { id: MoreToolId; title: string; description: string; href: string }[] {
  return [
    {
      id: "strip",
      title: "Strip",
      description: t("toolStripDescription"),
      href: "/pulse",
    },
    {
      id: "history",
      title: t("toolHistoryTitle"),
      description: t("toolHistoryDescription"),
      href: "/history",
    },
    {
      id: "setup",
      title: t("toolSetupTitle"),
      description: t("toolSetupDescription"),
      href: "/setup",
    },
  ];
}

/**
 * Where an old deep link of the removed console lands (`/?view=…`,
 * `/?setup=1`): the wizard, Activity for the old inbox, History for the
 * old History view, Home for the rest.
 * Null when the link is not an old one.
 */
export function legacyRedirect(params: Record<string, string | string[] | undefined>): string | null {
  if (params.setup !== undefined) return "/setup";
  if (params.view === "history") return "/history";
  if (params.view === "inbox") {
    // The note the old inbox opened becomes Activity's selection.
    return typeof params.note === "string" && params.note ? `/activity?${new URLSearchParams({ sel: params.note })}` : "/activity";
  }
  if (["view", "note", "session", "dependency"].some((k) => params[k] !== undefined)) return "/";
  return null;
}

/** An old `/more…` bookmark: where its view lives now, Home by default. */
export function moreRedirect(params: Record<string, string | string[] | undefined>): string {
  return legacyRedirect(params) ?? "/";
}

/** The fixed text behind the operator's avatar (spec part 2, rule 7). */
export function passwordNotice(): string {
  return t("passwordNotice");
}

// ---------------------------------------------------------------------------
// Wording
// ---------------------------------------------------------------------------

/** serve's default display name is English whatever language the console speaks. */
/** What serve reports until the operator names themselves. */
const DEFAULT_OPERATOR_NAME = "Operator";
export const DEFAULT_OPERATOR_INITIALS = "OP";

export function operatorName(name: string | null | undefined): string {
  return !name || name === DEFAULT_OPERATOR_NAME ? t("operatorDefaultName") : name;
}

const LABEL_CHIP: Record<Label, { text: TranslationKey<"workspace">; tone: "neutral" | "amber" | "red" | "green" }> = {
  START: { text: "labelStart", tone: "neutral" },
  FINDING: { text: "labelFinding", tone: "amber" },
  BLOCKED: { text: "labelBlocked", tone: "red" },
  DONE: { text: "labelDone", tone: "green" },
};

export function labelChip(label: Label): { text: string; tone: "neutral" | "amber" | "red" | "green" } {
  const chip = LABEL_CHIP[label];
  return { text: t(chip.text), tone: chip.tone };
}

/** Why a flag matters, for its tooltip. */
const FLAG_HINT: Record<Flag, TranslationKey<"workspace">> = {
  unlabeled: "flagHintUnlabeled",
  too_long: "flagHintTooLong",
  repeat: "flagHintRepeat",
  jargon: "flagHintJargon",
};

const JARGON_REASON: Record<JargonReason, TranslationKey<"workspace">> = {
  sha: "jargonSha",
  review_code: "jargonReviewCode",
  word: "jargonWord",
};

const PRESENCE_TEXT: Record<Presence, TranslationKey<"workspace">> = {
  working: "presenceWorking",
  waiting: "presenceWaiting",
  finished: "presenceFinished",
};

export function presenceText(presence: Presence): string {
  return t(PRESENCE_TEXT[presence]);
}

export function noDelivery(): string {
  return t("noDelivery");
}

/** Why nobody reads this agent, for a clause in a sentence; empty while one can. */
export function deliveryReason(delivery: Delivery): string {
  switch (delivery) {
    case "ready":
      return "";
    case "stale":
      return t("deliveryReasonStale");
    case "ended":
      return t("deliveryReasonEnded");
    case "not_on_bus":
      return t("deliveryReasonNotOnBus");
  }
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

/** "ahora mismo" / "hace 3 min" / "hace 2 h" / "hace 4 d". */
export function hace(ms: number, now: number): string {
  const delta = Math.max(0, now - ms);
  if (delta < 45_000) return t("justNow");
  if (delta < HOUR) return t("agoMinutes", { n: Math.max(1, Math.round(delta / MIN)) });
  if (delta < DAY) return t("agoHours", { n: Math.round(delta / HOUR) });
  return t("agoDays", { n: Math.round(delta / DAY) });
}

/** "52 min", "3 h", "2 d": how long, rounded the way a person says it. */
export function duration(ms: number): string {
  const m = Math.max(0, Math.round(ms / MIN));
  if (m < 90) return `${m} min`;
  const h = Math.round(ms / HOUR);
  if (h < 48) return `${h} h`;
  return `${Math.round(ms / DAY)} d`;
}

/** "22:41" on the operator's clock. */
export function hhmm(ms: number): string {
  return new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), hour: "2-digit", minute: "2-digit" }).format(ms);
}

function localDay(ms: number): string {
  return new Intl.DateTimeFormat("en-CA", { timeZone: operatorTimeZone(), year: "numeric", month: "2-digit", day: "2-digit" }).format(ms);
}

/** "today at 22:41", "yesterday at 9:05" or "on 5 Nov at 22:41": when something was saved. */
export function atDate(ms: number, now: number): string {
  const day = localDay(ms);
  if (day === localDay(now)) return t("todayAt", { time: hhmm(ms) });
  if (day === localDay(now - DAY)) return t("yesterdayAt", { time: hhmm(ms) });
  const date = new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), day: "numeric", month: "short" }).format(ms).replace(".", "");
  return t("dateAt", { date, time: hhmm(ms) });
}

/** "Today", "Yesterday", or "Thursday, 5 November". */
export function dayLabel(ms: number, now: number): string {
  const day = localDay(ms);
  if (day === localDay(now)) return t("dayToday");
  if (day === localDay(now - DAY)) return t("dayYesterday");
  return new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), weekday: "long", day: "numeric", month: "long" }).format(ms);
}

/** Messages grouped by the operator's day, oldest first, keeping order inside. */
export function byDay<T extends { ts_ms: number }>(items: T[], now: number): { day: string; items: T[] }[] {
  const out: { key: string; day: string; items: T[] }[] = [];
  for (const item of items) {
    const key = localDay(item.ts_ms);
    const last = out[out.length - 1];
    if (last && last.key === key) last.items.push(item);
    else out.push({ key, day: dayLabel(item.ts_ms, now), items: [item] });
  }
  return out.map(({ day, items }) => ({ day, items }));
}

/** Two letters: the initials of two words, or a first letter and the next consonant. */
export function monogram(name: string): string {
  const words = name.split(/[-_ .·]+/).filter((w) => /[a-záéíóúñ]/i.test(w));
  if (words.length >= 2) return (words[0]![0]! + words[1]![0]!).toUpperCase();
  const word = words[0] ?? name;
  const rest = word.slice(1);
  const consonant = rest.match(/[bcdfgjklmnpqrstvwxz]/i)?.[0] ?? rest[0] ?? "";
  return (word[0]! + consonant).toUpperCase();
}

/**
 * Muted avatar colours; the same name always gets the same one. They are the
 * shell's tokens (`--hb-tone-1..8`, workspace.css), so each mode can tune them.
 */
const TONES = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `var(--hb-tone-${n})`);

export function tone(name: string): string {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return TONES[h % TONES.length]!;
}

/** "since 22:33", or "since 5 Nov at 22:33" on another day. */
function sinceDate(ms: number, now: number): string {
  if (localDay(ms) === localDay(now)) return t("sinceTime", { time: hhmm(ms) });
  const date = new Intl.DateTimeFormat(intlTag(), { timeZone: operatorTimeZone(), day: "numeric", month: "short" }).format(ms).replace(".", "");
  return t("sinceDate", { date, time: hhmm(ms) });
}

/**
 * "working · 1 min ago"; "no activity since 22:33" when dormant;
 * "no data since 22:33" when nothing is known since (lost).
 */
export function presenceLine(agent: WsAgent, now: number): string {
  if (agent.lost) return agent.last_action_ms ? t("noDataSince", { since: sinceDate(agent.last_action_ms, now) }) : t("noData");
  if (agent.dormant && agent.last_action_ms) return t("noActivitySince", { time: hhmm(agent.last_action_ms) });
  const base = presenceText(agent.presence);
  return agent.last_action_ms ? `${base} · ${hace(agent.last_action_ms, now)}` : base;
}

/** The dot colour: dormant overrides presence. */
export function dotTone(agent: Pick<WsAgent, "presence" | "dormant">): "green" | "amber" | "red" | "grey" {
  if (agent.dormant === "red") return "red";
  if (agent.dormant === "amber") return "amber";
  return agent.presence === "working" ? "green" : agent.presence === "waiting" ? "amber" : "grey";
}

export function fileName(path: string): string {
  return path.replace(/\/+$/, "").split("/").pop() || path;
}

/** "~/Projects/x" for a path under the home directory. */
export function shortPath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~");
}

function list(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  return t("listJoin", { head: names.slice(0, -1).join(", "), last: names[names.length - 1] ?? "" });
}

/** The name an agent key shows; `lead:<session>` is a lead session itself. */
export function agentLabel(view: Pick<WorkspaceView, "agents">, key: string): string {
  if (key.startsWith("lead:")) return t("leadSessionNamed", { id: key.slice(5, 13) });
  const agent = view.agents.find((a) => a.key === key);
  return agent ? agentName(agent) : shownName(key.slice(0, 8));
}

/** "Blocked in #orchard", "Approval in #acme", "Notice in #orchard". */
export function cardReason(card: Pick<WsCard, "kind" | "channel">): string {
  const channel = card.channel;
  switch (card.kind) {
    case "blocked":
      return channel ? t("reasonBlockedIn", { channel }) : t("reasonBlocked");
    case "approval":
      return channel ? t("reasonApprovalIn", { channel }) : t("reasonApproval");
    case "unanswered":
      return channel ? t("reasonUnansweredIn", { channel }) : t("reasonUnanswered");
    case "mention":
      return channel ? t("reasonMentionIn", { channel }) : t("reasonMention");
    case "handoff":
      return channel ? t("reasonHandoffIn", { channel }) : t("reasonHandoff");
    case "check":
      return t("reasonCheck");
    default:
      return channel ? t("reasonAlertIn", { channel }) : t("reasonAlert");
  }
}

/**
 * "Was it you?": a rule approved, the context changed or the memory sink switched
 * on reaches every session, so serve asks the operator about each.
 */
export function checkBody(check: WsCard["check"]): { title: string; text: string } {
  const short = (text: string) => ellipsizeWords(text.replace(/\s+/g, " ").trim(), 160);
  switch (check?.what) {
    case "rule":
      return { title: t("checkRuleTitle", { text: short(check.text) }), text: t("checkRuleText") };
    case "context":
      return check.text.trim()
        ? { title: t("checkContextTitle", { text: short(check.text) }), text: t("checkContextText") }
        : { title: t("checkContextClearedTitle"), text: t("checkContextClearedText") };
    case "memory_sink":
      return { title: t("checkMemorySinkTitle"), text: t("checkMemorySinkText") };
    default:
      return { title: t("checkOtherTitle"), text: "" };
  }
}

/** Title and sentence of a warning card. */
export function cardBody(
  card: WsCard,
  view: Pick<WorkspaceView, "agents"> & { messages?: WsMessage[] },
  now: number,
): { title: string; text: string } {
  const names = card.agents.map((k) => agentLabel(view, k));
  if (card.kind === "two_writers") {
    const n = names.length;
    const title = n === 2 ? t("twoWritersTitleTwo") : t("twoWritersTitleMany", { n });
    const text = t("twoWritersText", { names: list(names), checkout: fileName(card.checkout ?? "") });
    return { title, text: card.needs ? text : `${text} ${t("twoWritersHistory")}` };
  }
  if (card.kind === "mention") {
    const m = card.message ? view.messages?.find((x) => x.id === card.message) : undefined;
    const who = card.agents[0] ? agentLabel(view, card.agents[0]) : t("anAgent");
    return { title: t("mentionTitle", { who }), text: m ? t("quotedAt", { text: m.text, time: hhmm(card.since_ms) }) : t("atTime", { time: hhmm(card.since_ms) }) };
  }
  if (card.kind === "handoff") {
    const m = card.message ? view.messages?.find((x) => x.id === card.message) : undefined;
    const who = card.agents[0] ? agentLabel(view, card.agents[0]) : t("anAgent");
    return { title: t("handoffTitle", { who }), text: m ? t("quotedAt", { text: m.text, time: hhmm(card.since_ms) }) : t("atTime", { time: hhmm(card.since_ms) }) };
  }
  if (card.kind === "unanswered") {
    const note = card.message ? view.messages?.find((m) => m.id === card.message) : undefined;
    const agent = card.agents[0] ? agentLabel(view, card.agents[0]) : null;
    const title = card.channel && !agent ? t("unansweredInChannel", { channel: card.channel }) : agent ? t("unansweredToAgent", { agent }) : t("unanswered");
    const nudged = note?.delivery?.nudged_ms ? ` ${t("resentAt", { time: hhmm(note.delivery.nudged_ms) })}` : "";
    const quote = note ? t("quotedAt", { text: note.text, time: hhmm(card.since_ms) }) : t("writtenAt", { time: hhmm(card.since_ms) });
    return { title, text: `${quote}${nudged}` };
  }
  if (card.kind === "check") return checkBody(card.check);
  if (card.kind === "dormant") {
    return {
      title: t("dormantTitle"),
      text: t("dormantText", { name: names[0] ?? t("anAgent"), time: hhmm(card.since_ms), duration: duration(now - card.since_ms) }),
    };
  }
  if (card.kind === "blocked") {
    const a = view.agents.find((x) => x.key === card.agents[0]);
    return { title: names[0] ?? "", text: a && sessionGone(a) ? `${sessionGoneNotice()}.` : "" };
  }
  return { title: names[0] ?? "", text: "" };
}

/**
 * When every lead the composer would reach is stale: the note is stored and
 * waits; the composer says so instead of presenting it as delivered.
 */
export function composerNotice(agents: Pick<WsAgent, "delivery" | "lead_quiet_ms">[]): string | null {
  if (agents.length === 0 || agents.some((a) => a.delivery === "ready")) return null;
  const stale = agents.filter((a) => a.delivery === "stale");
  if (stale.length === 0) return null;
  const quiet = Math.min(...stale.map((a) => a.lead_quiet_ms ?? 0));
  return t("composerStale", { time: duration(quiet) });
}

/**
 * What the composer says instead of a box, or null. Only when there is no
 * session at all to leave the note for: an agent always has its lead, so a
 * reply to an agent is never blocked; it is confirmed (`composerConfirm`).
 */
export function composerBlock(agents: Pick<WsAgent, "delivery">[]): string | null {
  if (agents.length === 0) return noReader();
  return null;
}

/** The question before sending when no live session would read it. */
export function nobodyLive(): string {
  return t("nobodyLive");
}

export function noReader(): string {
  return t("noReader");
}

/** For a reply to agents: the question to ask before sending, or null when a session reads it (or may come back). */
export function composerConfirm(agents: Pick<WsAgent, "delivery">[]): string | null {
  if (agents.length === 0 || agents.some((a) => a.delivery === "ready" || a.delivery === "stale")) return null;
  return nobodyLive();
}

/** The channel composer, from the server's reach: what blocks it, what it says above, what it asks before sending. */
export function channelComposer(channel: Pick<WsChannel, "reach" | "readers">): { blocked: string | null; notice: string | null; confirm: string | null } {
  switch (channel.reach) {
    case "ready":
      return { blocked: null, notice: null, confirm: null };
    case "stale":
      return { blocked: null, notice: t("channelStale"), confirm: null };
    case "none":
      return { blocked: null, notice: null, confirm: nobodyLive() };
    case "nobody":
      return { blocked: noReader(), notice: null, confirm: null };
    default:
      // A serve from before `reach`: as before, serve itself refuses with why.
      return { blocked: null, notice: null, confirm: null };
  }
}

/**
 * Who reads what is written in this conversation: the session itself, or the
 * lead an in-process agent depends on. Null when the view has no such session.
 */
export function noteReader(agents: Pick<WsAgent, "key" | "kind" | "name" | "lead_session">[], key: string): string | null {
  const agent = agents.find((a) => a.key === key);
  if (!agent) return null;
  if (agent.kind === "session") return leadDisplayName(agent.name);
  const lead = agents.find((a) => a.key === `session:${agent.lead_session}`);
  return lead ? leadDisplayName(lead.name) : null;
}

/** What a conversation is with: a name alone cannot say, two agents can share one. */
export function agentRelation(agent: Pick<WsAgent, "key" | "kind" | "name" | "lead_session">, agents: Pick<WsAgent, "key" | "kind" | "name" | "lead_session">[]): string {
  if (agent.kind === "session") return t("relationMainSession");
  const name = noteReader([agent, ...agents], agent.key) ?? t("relationOtherSession");
  return agent.kind === "teammate" ? t("relationTeammateOf", { name }) : t("relationSubagentOf", { name });
}

/** The line under one of the operator's notes, like a chat app's sent/seen. `reader` is {@link noteReader}'s. */
export function deliveryLine(d: WsNoteDelivery, reader: string | null = null): string {
  const many = d.sessions.length > 1;
  const named = many ? null : reader;
  const who = named
    ? t("toNamed", { name: named })
    : d.to_lead && !many
      ? t("toLead")
      : many
        ? t("toSessions", { delivered: d.delivered, total: d.sessions.length })
        : t("toSession", { id: d.sessions[0]?.slice(0, 8) ?? "" });
  let line: string;
  switch (d.state) {
    case "answered":
      line = d.answered_by ? t("repliedBy", { name: leadDisplayName(d.answered_by) }) : t("replied");
      break;
    case "read":
      line = named ? t("readBy", { name: named }) : d.to_lead && !many ? t("readByLead") : t("read");
      break;
    case "delivered":
      line = t("deliveredTo", { who });
      break;
    default:
      line = t("sent");
  }
  return d.nudged_ms ? t("lineResent", { line, time: hhmm(d.nudged_ms) }) : line;
}

/** How long after a re-send "Resend" stays off: serve's own window. */
const NUDGE_WINDOW_MS = 10 * 60_000;

/** Whether the note was re-sent recently enough that the button is off. */
export function nudgeShown(nudgedMs: number | null | undefined, now: number): boolean {
  return typeof nudgedMs === "number" && now - nudgedMs < NUDGE_WINDOW_MS;
}

/** The URL the console reads an attachment from (proxied to serve). */
export function attachmentUrl(a: Pick<WsAttachment, "id">): string {
  return `/api/harbard/attachments/${a.id.split("/").map(encodeURIComponent).join("/")}`;
}

/** An before/after pair of images, when a message holds one. */
export function beforeAfter(list: WsAttachment[]): { before: WsAttachment; after: WsAttachment } | null {
  const before = list.find((a) => a.kind === "image" && a.label === "before");
  // i18n-exempt: serve's label for the attachment, compared and never shown.
  const after = list.find((a) => a.kind === "image" && a.label === "after");
  return before && after ? { before, after } : null;
}

/** "12 kB", "3,4 MB". */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} kB`;
  return `${(bytes / (1024 * 1024)).toLocaleString(intlTag(), { maximumFractionDigits: 1 })} MB`;
}

/** Who the lead is, in the console. */
const LEAD_NAME = "Claude";
/** serve's handle and role for a lead: compared and typed after an `@`, never translated. */
export const LEAD_ROLE = "lead";
/** The name a lead answers to with an `@`: an id, not a word on screen. */
const LEAD_HANDLE = "coordinator";
const LEAD_AUTHOR = `${LEAD_NAME} · ${LEAD_ROLE}`;

/** The word for a lead's role, where the console shows it. */
export function leadRoleLabel(): string {
  return t("leadRole");
}

/** A message author with no agent behind it: a session known only by its id. */
const BARE_SESSION = /^session (\S{1,8})$/;

/** A name as shown: serve's fixed names, every unnamed lead's, are worded here. */
export function shownName(name: string): string {
  if (name === LEAD_AUTHOR) return t("leadAuthor");
  const id = BARE_SESSION.exec(name)?.[1];
  return id ? serveText("name.session.id", { id }, name) : name;
}

/** An agent's name as shown: a harbard-worded one in this language. */
export function agentName(agent: Pick<WsAgent, "name" | "name_code" | "name_params">): string {
  const params = agent.name_params;
  return serveText(agent.name_code, params && agent.name_code === "name.team_role" ? roleVars(params) : params, shownName(agent.name));
}

/** The line under a workshop name: a role harbard knows in this language. */
export function agentRoleLine(agent: Pick<WsAgent, "role_line" | "role_line_code" | "role_line_params">): string {
  const fallback = agent.role_line ?? "";
  const params = agent.role_line_params;
  return serveText(agent.role_line_code, params ? roleVars(params) : undefined, fallback);
}

/**
 * What an agent's robot is drawn from. Every lead session is named "Claude ·
 * lead", so a lead takes its look from its session key; anyone else
 * from its name. Every view draws through here, so a session looks the same
 * in the sidebar, its card, a channel and Strip.
 */
export function robotName(a: Pick<WsAgent, "kind" | "handle" | "key" | "name">): string {
  return a.kind === "session" && a.handle === LEAD_HANDLE ? a.key : a.name;
}

/**
 * The session that wrote a lead message: the message's own session, or the
 * lead of the agent it was written to. Null when neither is known.
 */
export function leadKeyOf(agents: Pick<WsAgent, "key" | "lead_session">[], agentKey: string | null | undefined): string | null {
  if (!agentKey) return null;
  if (agentKey.startsWith("session:")) return agentKey;
  const lead = agents.find((a) => a.key === agentKey)?.lead_session;
  return lead ? `session:${lead}` : null;
}

// ---------------------------------------------------------------------------
// Selections
// ---------------------------------------------------------------------------

/** Top-level messages of a channel: replies live in their thread. */
export function channelStream(view: WorkspaceView, channel: string): WsMessage[] {
  return view.messages.filter((m) => m.channels.includes(channel) && !m.reply_to && !m.structured && m.from !== "peer");
}

/** One agent's conversation, flat, as a direct message reads. */
export function conversation(view: WorkspaceView, agent: string): WsMessage[] {
  return view.messages.filter((m) => m.agent === agent && !m.structured);
}

export function messageById(view: WorkspaceView, id: string): WsMessage | undefined {
  return view.messages.find((m) => m.id === id);
}

/** Replies of a message, in order. */
export function thread(view: WorkspaceView, id: string): WsMessage[] {
  const parent = messageById(view, id);
  if (!parent) return [];
  return parent.replies
    .map((r) => messageById(view, r))
    .filter((m): m is WsMessage => Boolean(m))
    .sort((a, b) => a.ts_ms - b.ts_ms);
}

/** Copies of a message collapsed under it, by `repeat_of`. */
export function repeatsOf(view: WorkspaceView, id: string): WsMessage[] {
  return view.messages.filter((m) => m.repeat_of === id);
}

/** "Needs you": what needs the operator and is not resolved. */
export function openCards(view: WorkspaceView): WsCard[] {
  return view.cards.filter((c) => c.needs && c.resolved_ms === null);
}

/** A row of Activity's lists: a card, or a message. */
export function entryId(e: ActivityEntry): string {
  return e.kind === "card" ? `card:${e.card.id}` : `msg:${e.message.id}`;
}

/**
 * What Activity opens. A message you are reading stays open when it leaves
 * the unread list; a card that left the list was resolved, so the next one
 * takes its place, and an entry still listed is read fresh from the list.
 */
export function activitySelection(
  list: ActivityEntry[],
  picked: string | null,
  shown: ActivityEntry | null,
  position: number,
): ActivityEntry | null {
  const listed = (id: string | null) => list.find((e) => entryId(e) === id);
  const chosen = listed(picked);
  if (chosen) return chosen;
  if (shown && (!picked || entryId(shown) === picked)) {
    const fresh = listed(entryId(shown));
    if (fresh) return fresh;
    if (shown.kind === "message") return shown;
  }
  return list[Math.min(position, list.length - 1)] ?? null;
}

export type ActivityEntry = { kind: "card"; card: WsCard } | { kind: "message"; message: WsMessage };

/** Who All lists: the agents, the operator, and sessions writing on the bus. A lead's or a peer's word is a reply, read in its thread. */
const TODO_FROM: ReadonlySet<WsMessage["from"]> = new Set(["agent", "operator", "session"]);

/**
 * Activity's All, newest first: every card, open or not, and every
 * message of an agent (labelled or not, BLOCKED included), of the operator's and
 * of the bus (AC9). A protocol record (`structured`) is not a message. A
 * message a card already shows is that card's row, never a second one (AC3:
 * an open BLOCKED is its card; an answered one, which has none, is its
 * message).
 */
export function todoItems(view: WorkspaceView, max = 200): ActivityEntry[] {
  const carded = new Set(view.cards.map((c) => c.message).filter((m): m is string => m !== null));
  const cards: ActivityEntry[] = view.cards.map((card) => ({ kind: "card", card }));
  const said: ActivityEntry[] = view.messages
    .filter((m) => TODO_FROM.has(m.from) && !m.structured && !carded.has(m.id))
    .map((message) => ({ kind: "message", message }));
  const at = (e: ActivityEntry) => (e.kind === "card" ? e.card.since_ms : e.message.ts_ms);
  return [...cards, ...said].sort((a, b) => at(b) - at(a)).slice(0, max);
}

/**
 * Most urgent first, the one order Needs you and the calm card share
 * (AC8): a blocked agent, an approval, work handed over, a warning, a
 * mention, a question, an unanswered note, two writers, a dormant agent.
 */
export const URGENCY: Record<CardKind | "warning" | "ask", number> = {
  blocked: 0,
  // "Was it you?": something changed what every session reads.
  check: 1,
  approval: 1,
  handoff: 2,
  warning: 3,
  mention: 4,
  ask: 5,
  unanswered: 6,
  two_writers: 7,
  dormant: 8,
};

/** Needs you: the open cards by [`URGENCY`], the newest first within a kind. */
export function needsItems(view: WorkspaceView): WsCard[] {
  return openCards(view).sort((a, b) => URGENCY[a.kind] - URGENCY[b.kind] || b.since_ms - a.since_ms);
}

/** What Esc marks read for the open item: its agent, up to the item's own time. */
export function readMarkOf(view: WorkspaceView, entry: ActivityEntry): { agent: string; ts_ms: number } | null {
  if (entry.kind === "message") return { agent: entry.message.agent, ts_ms: entry.message.ts_ms };
  const m = entry.card.message ? view.messages.find((x) => x.id === entry.card.message) : undefined;
  const agent = m?.agent ?? entry.card.agents[0];
  if (!agent || !view.agents.some((a) => a.key === agent)) return null;
  return { agent, ts_ms: m?.ts_ms ?? entry.card.since_ms };
}

/**
 * After "Mark everything as read" (AC7): what it cleared, and that Needs
 * you does not go by reads. `before` is the count before the click.
 */
export function readAllNotice(before: { directs: number; channels?: number }): string {
  const tail = t("readAllTail");
  const channels = before.channels ?? 0;
  if (before.directs + channels === 0) return t("readAllNothing", { tail });
  return t("readAllDone", { directs: before.directs, channels, tail });
}

/** Agents of a channel, the ones that wrote or ran git there. */
export function members(view: WorkspaceView, channel: WsChannel): WsAgent[] {
  return channel.members
    .map((k) => view.agents.find((a) => a.key === k))
    .filter((a): a is WsAgent => Boolean(a));
}

/**
 * Why a channel is in the list, for its header: who worked in it in the
 * last 24 hours ("3 sessions in the last 24 h"), or, when nobody does,
 * when it last had activity, or that it was only read with git. `agents`
 * are the ones the line is about, for links to their conversations.
 */
export function channelWhy(view: WorkspaceView, channel: WsChannel): { line: string; agents: WsAgent[] } {
  const known = (keys: string[]) =>
    keys.map((k) => view.agents.find((a) => a.key === k)).filter((a): a is WsAgent => Boolean(a));
  const recent = known(channel.recent ?? []);
  if (recent.length > 0) {
    return { line: recent.length === 1 ? t("whySessionsOne") : t("whySessionsMany", { n: recent.length }), agents: recent };
  }
  const readers = members(view, channel);
  if (channel.active_ms) {
    return { line: t("whyIdle", { when: atDate(channel.active_ms, view.now_ms) }), agents: readers };
  }
  return {
    line: readers.length === 1 ? t("whyReadOne") : t("whyReadMany", { n: readers.length }),
    agents: readers,
  };
}

/** Agents with a record this recent appear in the sidebar (never a temporary one). */
const SIDEBAR_ACTIVE_MS = 24 * HOUR;
/** Rows shown before "See all". */
const SIDEBAR_ROWS = 8;

/**
 * The sidebar's agents: those active in the last 24 h, newest first, none
 * hidden. The first eight show; `rest` are behind "See all".
 */
/** A main session still running: the one to talk to, its subagents report to it. */
function isOpenSession(a: Pick<WsAgent, "kind" | "state" | "lost">): boolean {
  return a.kind === "session" && a.state !== "done" && !a.lost;
}

/** The name of whoever signs as a session: its callsign, or "Claude" for an unnamed lead. */
export function leadDisplayName(author: string): string {
  return author === LEAD_AUTHOR ? LEAD_NAME : author;
}

export function sidebarAgents(view: WorkspaceView): { sessions: WsAgent[]; shown: WsAgent[]; rest: WsAgent[]; inactive: WsAgent[] } {
  const listed = view.agents
    .filter((a) => !a.temporary && !a.hidden && (a.last_action_ms ?? 0) >= view.now_ms - SIDEBAR_ACTIVE_MS)
    .sort((a, b) => (b.last_action_ms ?? 0) - (a.last_action_ms ?? 0) || a.key.localeCompare(b.key));
  // The sessions you can talk to go on their own, whether or not they are busy.
  const sessions = listed.filter(isOpenSession);
  const agents = listed.filter((a) => !isOpenSession(a));
  const active = agents.filter((a) => !isInactive(a, view.now_ms));
  const inactive = agents.filter((a) => isInactive(a, view.now_ms));
  return { sessions, shown: active.slice(0, SIDEBAR_ROWS), rest: active.slice(SIDEBAR_ROWS), inactive };
}

/** Idle, finished or closed this long: under "Inactive". */
const INACTIVE_MS = 30 * MIN;
/** A closed agent (its lead stopped it) moves there sooner. */
const CLOSED_INACTIVE_MS = 10 * MIN;
/** Faces that are work in progress: always on top. */
const BUSY = new Set(["thinking", "running", "testing", "editing", "reading", "web", "delegating", "waiting", "blocked", "asleep"]);

/**
 * Whether an agent goes into the collapsed "Inactive" group: closed for
 * ten minutes, or idle ("No task"), finished or without data for thirty.
 * One that is working, waiting or blocked never does.
 */
export function isInactive(a: Pick<WsAgent, "activity" | "last_action_ms" | "closed" | "lost">, now: number): boolean {
  const state = a.activity?.state ?? "normal";
  if (BUSY.has(state) && !a.lost) return false;
  const quiet = now - (a.last_action_ms ?? 0);
  if (a.closed || a.activity?.detail === "closed") return quiet > CLOSED_INACTIVE_MS;
  return quiet > INACTIVE_MS;
}

/** Messages that have a thread, the one answered last first (AC9). */
export function threads(view: WorkspaceView): WsMessage[] {
  const at = new Map(view.messages.map((m) => [m.id, m.ts_ms]));
  const last = (m: WsMessage) => Math.max(m.ts_ms, ...m.replies.map((r) => at.get(r) ?? 0));
  return view.messages
    .filter((m) => m.replies.length > 0)
    .map((m) => ({ m, last: last(m) }))
    .sort((a, b) => b.last - a.last)
    .map(({ m }) => m);
}

/** "1 reply", "3 replies". */
export function replyCount(n: number): string {
  return n === 1 ? t("repliesOne") : t("repliesMany", { n });
}

export interface SearchResults {
  agents: WsAgent[];
  channels: WsChannel[];
  messages: WsMessage[];
  reports: { path: string; message: WsMessage }[];
}

/** One field for agents, channels, messages and reports, over what the view holds. */
export function search(view: WorkspaceView, query: string): SearchResults {
  const q = query.trim().toLocaleLowerCase("es");
  if (!q) return { agents: [], channels: [], messages: [], reports: [] };
  const has = (s: string | null | undefined) => (s ?? "").toLocaleLowerCase("es").includes(q);
  const agents = view.agents.filter((a) => !a.temporary && (has(a.name) || has(a.handle) || has(a.team) || has(a.description) || has(a.brief)));
  const channels = view.channels.filter((c) => has(c.id) || has(c.repo));
  const messages = view.messages.filter((m) => !m.structured && (has(m.text) || has(m.author))).sort((a, b) => b.ts_ms - a.ts_ms);
  const reports: { path: string; message: WsMessage }[] = [];
  const seen = new Set<string>();
  for (const m of view.messages) {
    for (const f of m.files) {
      if (!seen.has(f) && has(f)) {
        seen.add(f);
        reports.push({ path: f, message: m });
      }
    }
  }
  return { agents, channels, messages, reports };
}

// ---------------------------------------------------------------------------
// Parts 4 to 7: names, mentions, channels, live answers, read marks
// ---------------------------------------------------------------------------

/** The sidebar's agent filter: by display name, handle or team. */
export function filterAgents<T extends Pick<WsAgent, "name" | "handle" | "team">>(agents: T[], query: string): T[] {
  const q = query.trim().toLocaleLowerCase("es");
  if (!q) return agents;
  const has = (s: string | null | undefined) => (s ?? "").toLocaleLowerCase("es").includes(q);
  return agents.filter((a) => has(a.name) || has(a.handle) || has(a.team));
}

/** Channels with activity in 14 days, then the quiet ones under "More channels". */
export function sidebarChannels(view: Pick<WorkspaceView, "channels">): { shown: WsChannel[]; more: WsChannel[] } {
  return { shown: view.channels.filter((c) => !c.quiet), more: view.channels.filter((c) => c.quiet) };
}

/** Agents of throwaway sessions (system temp space), for "Sesiones temporales". */
export function temporaryAgents(view: Pick<WorkspaceView, "agents">): WsAgent[] {
  return view.agents.filter((a) => a.temporary && !a.hidden);
}

/** Whether the faces' slice says the full view changed: pull it now. */
export function shouldPull(viewRev: string | undefined, pulseRev: string | undefined): boolean {
  return Boolean(viewRev && pulseRev && viewRev !== pulseRev);
}

export interface MentionOption {
  /** What goes after the `@`. */
  handle: string;
  label: string;
  hint: string;
  kind: "agent" | "team" | "channel";
}

/** What an `@` can name here: agents by display name, their teams, and `@channel` in a channel. */
export function mentionOptions(view: Pick<WorkspaceView, "agents">, channel: string | null): MentionOption[] {
  // A closed agent (its lead stopped it) reads nothing: not offered.
  const agents = view.agents.filter((a) => !a.temporary && !a.closed && a.handle);
  const out: MentionOption[] = [];
  if (channel) out.push({ handle: "channel", label: "@channel", hint: t("mentionChannelHint", { channel }), kind: "channel" });
  const teams = new Map<string, number>();
  for (const a of agents) if (a.team && a.kind !== "session") teams.set(a.team.toLocaleLowerCase("es"), (teams.get(a.team.toLocaleLowerCase("es")) ?? 0) + 1);
  for (const [team, n] of [...teams.entries()].sort()) {
    if (n > 1) out.push({ handle: team, label: `@${team}`, hint: t("mentionTeamHint", { n }), kind: "team" });
  }
  for (const a of agents.slice().sort((x, y) => x.name.localeCompare(y.name, "es"))) {
    out.push({ handle: a.handle!, label: agentName(a), hint: a.role_line ? `${agentRoleLine(a)} · @${a.handle}` : `@${a.handle}`, kind: "agent" });
  }
  return out;
}

/** The `@word` being typed at `caret`, if any. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const before = text.slice(0, caret);
  const m = before.match(/(^|[\s(])@([\p{L}\p{N}._-]*)$/u);
  if (!m) return null;
  return { start: caret - m[2]!.length - 1, query: m[2]!.toLocaleLowerCase("es") };
}

/** Options that match what is typed after the `@`. */
export function matchMentions(options: MentionOption[], query: string): MentionOption[] {
  const q = query.toLocaleLowerCase("es");
  return options.filter((o) => !q || o.handle.includes(q) || o.label.toLocaleLowerCase("es").includes(q)).slice(0, 8);
}

/** The text with the `@word` at `start` replaced by the chosen handle. */
export function applyMention(text: string, start: number, caret: number, handle: string): { text: string; caret: number } {
  const insert = `@${handle} `;
  return { text: text.slice(0, start) + insert + text.slice(caret), caret: start + insert.length };
}

/** A message's text cut at the mentions it holds, for highlighting. */
export function mentionSegments(text: string, mentions: string[] | undefined): { text: string; mention: boolean }[] {
  if (!mentions || mentions.length === 0) return [{ text, mention: false }];
  const known = new Set(mentions.map((m) => m.toLocaleLowerCase("es")));
  const out: { text: string; mention: boolean }[] = [];
  let last = 0;
  for (const m of text.matchAll(/(^|[^\p{L}\p{N}.])@([\p{L}\p{N}._-]+)/gu)) {
    const token = m[2]!.replace(/[._-]+$/, "");
    if (!known.has(token.toLocaleLowerCase("es"))) continue;
    const at = m.index! + m[1]!.length;
    if (at > last) out.push({ text: text.slice(last, at), mention: false });
    out.push({ text: `@${token}`, mention: true });
    last = at + token.length + 1;
  }
  if (last < text.length) out.push({ text: text.slice(last), mention: false });
  return out;
}

/** Where a workshop name came from, in Profile. */
export function nameOrigin(agent: Pick<WsAgent, "name_source" | "name_reason">): string | null {
  switch (agent.name_source) {
    case "picked":
      return agent.name_reason ? t("originPickedWhy", { reason: agent.name_reason }) : t("originPicked");
    case "renamed":
      return t("originRenamed");
    case "launched":
      return t("originLaunched");
    case "auto":
      return t("originAuto");
    default:
      return null;
  }
}

/** What the DM composer says about the path a message takes. */
export function dmHint(agent: Pick<WsAgent, "kind" | "name"> & Partial<Pick<WsAgent, "lost" | "delivery">>, reader: string | null = null): string {
  if (sessionGone(agent)) return `${sessionGoneNotice()}.`;
  const name = agentName(agent);
  if (agent.kind === "session") return t("dmSession", { name });
  return reader ? t("dmVia", { name, reader }) : t("dmLead", { name });
}

/** What a reply box and a BLOCKED card say when nobody will read it (AC6). */
export function sessionGoneNotice(): string {
  return t("sessionGone");
}

/**
 * Nobody reads this agent now: it is lost, or its lead is off the bus or
 * ended. A stale lead may come back, so it is not gone.
 */
export function sessionGone(agent: Partial<Pick<WsAgent, "lost" | "delivery">>): boolean {
  return agent.lost === true || agent.delivery === "not_on_bus" || agent.delivery === "ended";
}

/** A message longer than this, in characters or lines, folds in a channel behind "See more". */
export const FOLD_CHARS = 480;
const FOLD_LINES = 8;

/**
 * Where a long message folds in a channel: the part shown before "See more",
 * cut at a word boundary, or null when it shows whole. Never used in the
 * thread pane, which always shows a message whole.
 */
export function foldText(text: string, chars = FOLD_CHARS, lines = FOLD_LINES): string | null {
  const byLines = text.split("\n");
  let head = byLines.length > lines ? byLines.slice(0, lines).join("\n") : text;
  if (head.length > chars) {
    const cut = head.slice(0, chars);
    const space = cut.search(/\s\S*$/);
    head = space > 0 ? cut.slice(0, space) : cut;
  }
  head = head.trimEnd();
  return head.length < text.trimEnd().length ? head : null;
}

/** At most `max` characters, cut at a word boundary with "…", never mid-word. */
export function ellipsizeWords(text: string, max: number): string {
  if (text.length <= max) return text;
  const cut = text.slice(0, max - 1);
  const space = cut.search(/\s\S*$/);
  return `${(space > 0 ? cut.slice(0, space) : cut).trimEnd()}…`;
}

/** The keyboard shortcuts the shell answers, for "Keyboard shortcuts". */
export function shortcuts(): readonly { keys: string; what: string }[] {
  return [
    { keys: "⌘K", what: t("shortcutSearch") },
    { keys: "Esc", what: t("shortcutMarkRead") },
    { keys: "⇧Esc", what: t("shortcutMarkAllRead") },
    { keys: t("shortcutKeyEnter"), what: t("shortcutSend") },
    { keys: t("shortcutKeyShiftEnter"), what: t("shortcutNewLine") },
    { keys: "@", what: t("shortcutMention") },
  ];
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

export type WorkspaceAction =
  | "rename-agent"
  | "name-vote"
  | "resolve"
  | "reopen"
  | "read"
  | "save"
  | "unsave"
  | "reply"
  | "open"
  | "motion"
  | "night-dim"
  | "nudge"
  | "read-channel"
  | "read-all"
  | "resolve-all"
  | "reopen-all"
  | "hide-agent"
  | "show-agent"
  | "show-all-agents"
  | "agents-collapsed"
  | "check-revoke";
export type PostWorkspace = (action: WorkspaceAction, body: Record<string, unknown>) => Promise<unknown>;

/** A file card asks serve to open the local file with the OS. Nothing is uploaded. */
export function openFile(path: string, post: PostWorkspace): Promise<unknown> {
  return post("open", { path });
}

/** Reply to one agent (optionally in a thread) or to a channel. */
export function replyBody(
  target: { agent: string; replyTo?: string | null } | { channel: string },
  text: string,
  anyway = false,
): Record<string, unknown> {
  const extra = anyway ? { anyway: true } : {};
  if ("channel" in target) return { channel: target.channel, text, ...extra };
  return target.replyTo ? { agent: target.agent, reply_to: target.replyTo, text, ...extra } : { agent: target.agent, text, ...extra };
}

/** Why serve refused a reply, in the composer's words. */
export function replyError(status: number | undefined, reason: string | undefined): string {
  if (status === 409) return reason === "no_reachable_member" ? t("replyNoChannelReader") : noDelivery();
  if (status === 422) return t("replyTooLong");
  return t("replySaveFailed");
}
