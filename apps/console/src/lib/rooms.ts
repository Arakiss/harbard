/**
 * Rooms and the Lobby, as `GET /api/v1/workspace` projects them.
 *
 * A room is a named channel the operator can create, archive and delete; a
 * project channel belongs to a repository and can only be archived. serve
 * decides what exists and refuses what the contract forbids; this module holds
 * the small pure work the views share and words it through the dictionaries.
 */

import { createT, type TranslationKey } from "@/i18n";
import { harnessLabel } from "./harness";
import type { WorkspaceView, WsChannel, WsMessage } from "./workspace";

const t = createT("rooms");

export const LOBBY = "lobby";
export const COMMONS = "commons";
export const FEEDBACK = "feedback";

/** `harbard_core::membership::BUILT_IN_ROOMS`, in the order the sidebar lists them. */
export const BUILT_IN_ROOMS = [LOBBY, COMMONS, FEEDBACK] as const;

export type BuiltInRoom = (typeof BUILT_IN_ROOMS)[number];

export function isBuiltIn(name: string): name is BuiltInRoom {
  return BUILT_IN_ROOMS.some((room) => room === name);
}

export type LobbyHarness = "claude-code" | "codex" | "cursor" | "other";
export type LobbyPresence = "live" | "idle" | "stale" | "ended";

/** One card of the Lobby: a main session or a named agent of this workspace. */
export interface LobbyEntry {
  /** The same as `agents[].key` when the agent is in the view. */
  key: string;
  name: string;
  harness: LobbyHarness;
  project: string | null;
  /** The latest thing it said in `#lobby`, or null while it has not spoken. */
  intro: string | null;
  intro_ts_ms: number;
  presence: LobbyPresence;
  first_ms: number;
  last_ms: number;
}

/** `GET /api/v1/channels/<slug>/history`: oldest first. */
export interface RoomHistoryPage {
  messages: WsMessage[];
  next_before: string | null;
}

/** One line of `GET /api/v1/channels/deleted`. */
export interface DeletedRoom {
  name: string;
  deleted_ms: number;
  messages: number;
}

// ---------------------------------------------------------------------------
// Naming a room
// ---------------------------------------------------------------------------

/** `harbard_core::membership::is_valid_name`: 48 bytes is the bus's limit. */
const NAME_MAX = 48;
const NAME_CHARS = /^[a-z0-9._-]+$/;

/** `harbard_core::membership::normalize`: one name, one channel, however it was typed. */
export function normalizeRoomName(input: string): string {
  return input.trim().replace(/^#+/, "").trim().toLowerCase().replace(/\s/g, "-");
}

export type RoomNameProblem = "empty" | "too_long" | "invalid" | "built_in" | "taken";

/** Why a normalized name cannot be created here, or null when it can. */
export function roomNameProblem(name: string, taken: readonly string[]): RoomNameProblem | null {
  if (name === "") return "empty";
  if (name.length > NAME_MAX) return "too_long";
  if (!NAME_CHARS.test(name)) return "invalid";
  if (isBuiltIn(name)) return "built_in";
  return taken.includes(name) ? "taken" : null;
}

const NAME_PROBLEM: Record<RoomNameProblem, TranslationKey<"rooms">> = {
  empty: "nameEmpty",
  too_long: "nameTooLong",
  invalid: "nameInvalid",
  built_in: "nameBuiltIn",
  taken: "nameTaken",
};

export function roomNameMessage(problem: RoomNameProblem): string {
  return t(NAME_PROBLEM[problem], { max: NAME_MAX });
}

// ---------------------------------------------------------------------------
// The sidebar
// ---------------------------------------------------------------------------

export function isRoom(channel: Pick<WsChannel, "kind">): boolean {
  return channel.kind === "room";
}

/** A built-in room serve did not list: an older serve, or an empty workspace. */
function builtInPlaceholder(id: BuiltInRoom): WsChannel {
  return { id, repo: "", checkouts: [], members: [], last_ms: 0, warnings: [], kind: "room", purpose: null, archived: false, built_in: true };
}

/** A channel by id; the three built-in rooms exist whatever serve listed. */
export function findChannel(view: Pick<WorkspaceView, "channels">, id: string): WsChannel | undefined {
  return view.channels.find((c) => c.id === id) ?? (isBuiltIn(id) ? builtInPlaceholder(id) : undefined);
}

/** `#lobby`, which exists whatever serve listed. */
export function findChannelOrLobby(view: Pick<WorkspaceView, "channels">): WsChannel {
  return findChannel(view, LOBBY)!;
}

/** The built-in rooms first and always, then the operator's by name, then what is archived. */
export function sidebarRooms(view: Pick<WorkspaceView, "channels">): { rooms: WsChannel[]; archived: WsChannel[] } {
  const builtIn = BUILT_IN_ROOMS.map((id) => findChannel(view, id)!);
  const own = view.channels.filter((c) => isRoom(c) && !isBuiltIn(c.id) && !c.archived).sort((a, b) => a.id.localeCompare(b.id));
  return { rooms: [...builtIn, ...own], archived: view.channels.filter((c) => c.archived && !isBuiltIn(c.id)) };
}

/** Where Home opens: the first project channel; a room is never a default. */
export function defaultChannelId(view: Pick<WorkspaceView, "channels">): string | null {
  return view.channels.find((c) => !isRoom(c) && !c.archived)?.id ?? null;
}

export function channelHref(id: string): string {
  return id === LOBBY ? "/lobby" : `/c/${encodeURIComponent(id)}`;
}

export type ChannelAction = "archive" | "unarchive" | "delete";

/**
 * What the channel menu offers, the contract's rules and no more: a built-in
 * room nothing; a project channel archive and back; a room archive and back,
 * and delete only once archived, because serve refuses it before.
 */
export function channelActions(channel: Pick<WsChannel, "id" | "kind" | "archived" | "built_in">): ChannelAction[] {
  if (channel.built_in || isBuiltIn(channel.id)) return [];
  if (!channel.archived) return ["archive"];
  return isRoom(channel) ? ["unarchive", "delete"] : ["unarchive"];
}

const ACTION_LABEL: Record<ChannelAction, TranslationKey<"rooms">> = {
  archive: "actionArchive",
  unarchive: "actionUnarchive",
  delete: "actionDelete",
};

export function channelActionLabel(action: ChannelAction): string {
  return t(ACTION_LABEL[action]);
}

const REFUSALS: Readonly<Record<string, TranslationKey<"rooms">>> = {
  room_built_in: "refusedBuiltIn",
  project_channel: "refusedProject",
};

/** serve's refusal in the operator's words; a code this console does not know keeps serve's own. */
export function refusalMessage(code: string | undefined, fallback: string): string {
  const key = code ? REFUSALS[code] : undefined;
  return key ? t(key) : fallback;
}

// ---------------------------------------------------------------------------
// A room's messages
// ---------------------------------------------------------------------------

/** A room is flat: a reply is one more line, so unlike a channel it keeps them. */
export function roomStream(view: Pick<WorkspaceView, "messages">, room: string): WsMessage[] {
  return view.messages.filter((m) => m.channels.includes(room) && !m.structured);
}

/** History and the live view, each message once, oldest first. The live copy wins: it is newer. */
export function mergeRoomMessages(history: readonly WsMessage[], live: readonly WsMessage[]): WsMessage[] {
  const byId = new Map<string, WsMessage>();
  for (const m of history) if (!m.structured) byId.set(m.id, m);
  for (const m of live) byId.set(m.id, m);
  return [...byId.values()].sort((a, b) => a.ts_ms - b.ts_ms || a.id.localeCompare(b.id));
}

const BUILT_IN_PURPOSE: Record<BuiltInRoom, TranslationKey<"rooms">> = {
  lobby: "purposeLobby",
  commons: "purposeCommons",
  feedback: "purposeFeedback",
};

/** The header's line: what serve holds, or what a built-in room is for. */
export function roomPurpose(channel: Pick<WsChannel, "id" | "purpose">): string | null {
  if (channel.purpose) return channel.purpose;
  return isBuiltIn(channel.id) ? t(BUILT_IN_PURPOSE[channel.id]) : null;
}

/**
 * What the composer says above the box. A post in a built-in room wakes nobody
 * and enters no brief, and the box must not look like one that does. A room
 * with no live reader still takes the post: nothing said in a room is lost.
 */
export function roomComposerNotice(channel: Pick<WsChannel, "id" | "reach">): string | null {
  if (channel.id === LOBBY || channel.id === COMMONS) return t("noticeReachesNoWork", { room: channel.id });
  if (channel.reach === "ready") return null;
  return t("noticeNoReaders", { room: channel.id });
}

// ---------------------------------------------------------------------------
// #feedback
// ---------------------------------------------------------------------------

export const FEEDBACK_KINDS = ["bug", "friction", "idea", "praise"] as const;
export type FeedbackKind = (typeof FEEDBACK_KINDS)[number];

export interface FeedbackNote {
  /** Null for a line that does not open with a kind this console knows. */
  kind: FeedbackKind | null;
  text: string;
  /** From the `harbard <version>` line `harbard feedback` adds. */
  version: string | null;
}

const KIND_LINE = /^\[([a-z]+)\]\s*/;
const VERSION_LINE = /^harbard\s+(\S+)\s*$/;

function feedbackKind(word: string | undefined): FeedbackKind | null {
  return FEEDBACK_KINDS.find((kind) => kind === word) ?? null;
}

/** `[<kind>] <text>` on the first line and `harbard <version>` on a later one. */
export function parseFeedback(raw: string): FeedbackNote {
  const [first = "", ...rest] = raw.split("\n");
  const kind = feedbackKind(KIND_LINE.exec(first)?.[1]);
  const versionAt = rest.findIndex((line) => VERSION_LINE.test(line));
  const version = versionAt >= 0 ? (VERSION_LINE.exec(rest[versionAt]!)?.[1] ?? null) : null;
  const body = [kind ? first.replace(KIND_LINE, "") : first, ...rest.filter((_, i) => i !== versionAt)];
  return { kind, text: body.join("\n").trim(), version };
}

const KIND_LABEL: Record<FeedbackKind, TranslationKey<"rooms">> = {
  bug: "kindBug",
  friction: "kindFriction",
  idea: "kindIdea",
  praise: "kindPraise",
};

export function feedbackKindLabel(kind: FeedbackKind | null): string {
  return kind ? t(KIND_LABEL[kind]) : t("kindNote");
}

export function feedbackTone(kind: FeedbackKind | null): "red" | "amber" | "accent" | "green" | "neutral" {
  switch (kind) {
    case "bug":
      return "red";
    case "friction":
      return "amber";
    case "idea":
      return "accent";
    case "praise":
      return "green";
    default:
      return "neutral";
  }
}

// ---------------------------------------------------------------------------
// The Lobby
// ---------------------------------------------------------------------------

export type LobbyGroup = "here" | "around" | "gone";

const GROUP_OF: Record<LobbyPresence, LobbyGroup> = { live: "here", idle: "around", stale: "around", ended: "gone" };

const GROUP_TITLE: Record<LobbyGroup, TranslationKey<"rooms">> = {
  here: "groupHere",
  around: "groupAround",
  gone: "groupGone",
};

export function lobbyGroupTitle(group: LobbyGroup): string {
  return t(GROUP_TITLE[group]);
}

/** Here now, around, gone; serve's newest-first order holds inside each. */
export function lobbyGroups(entries: readonly LobbyEntry[]): { group: LobbyGroup; entries: LobbyEntry[] }[] {
  const order: LobbyGroup[] = ["here", "around", "gone"];
  return order.map((group) => ({ group, entries: entries.filter((e) => GROUP_OF[e.presence] === group) })).filter((g) => g.entries.length > 0);
}

export function harnessName(harness: LobbyHarness): string {
  return harness === "other" ? t("harnessOther") : harnessLabel(harness);
}

/** The filter box: by name, project, harness or what it said. */
export function filterLobby(entries: readonly LobbyEntry[], query: string): LobbyEntry[] {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return [...entries];
  const has = (s: string | null) => (s ?? "").toLocaleLowerCase().includes(q);
  return entries.filter((e) => has(e.name) || has(e.project) || has(harnessName(e.harness)) || has(e.intro));
}

const PRESENCE_WORD: Record<LobbyPresence, TranslationKey<"rooms">> = {
  live: "presenceLive",
  idle: "presenceIdle",
  stale: "presenceStale",
  ended: "presenceEnded",
};

export function lobbyPresenceWord(presence: LobbyPresence): string {
  return t(PRESENCE_WORD[presence]);
}

/** The robots lined up at the top of the Lobby: who is here, then who is around. */
export const LINEUP_MAX = 14;

export function lobbyLineup(entries: readonly LobbyEntry[]): { shown: LobbyEntry[]; more: number } {
  const here = entries.filter((e) => e.presence === "live");
  const pool = here.length > 0 ? here : entries.filter((e) => e.presence !== "ended");
  return { shown: pool.slice(0, LINEUP_MAX), more: Math.max(0, pool.length - LINEUP_MAX) };
}
