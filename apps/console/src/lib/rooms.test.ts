import { describe, expect, test } from "bun:test";
import {
  channelActions,
  defaultChannelId,
  filterLobby,
  findChannel,
  lobbyGroups,
  lobbyLineup,
  LINEUP_MAX,
  mergeRoomMessages,
  normalizeRoomName,
  parseFeedback,
  refusalMessage,
  roomComposerNotice,
  roomNameProblem,
  roomPurpose,
  roomStream,
  sidebarRooms,
  type LobbyEntry,
} from "./rooms";
import { sidebarChannels, type WsChannel, type WsMessage } from "./workspace";

function channel(id: string, extra: Partial<WsChannel> = {}): WsChannel {
  return { id, repo: "", checkouts: [], members: [], last_ms: 0, warnings: [], ...extra };
}

const room = (id: string, extra: Partial<WsChannel> = {}) => channel(id, { kind: "room", ...extra });
const project = (id: string, extra: Partial<WsChannel> = {}) => channel(id, { kind: "project", repo: `/work/${id}`, ...extra });

function message(id: string, ts_ms: number, extra: Partial<WsMessage> = {}): WsMessage {
  return { id, agent: "a", from: "agent", author: "a", ts_ms, text: id, label: null, version: null, flags: [], repeat_of: null, chars: 1, files: [], verified: null, channels: ["r"], reply_to: null, replies: [], structured: null, undelivered: false, ...extra };
}

function entry(key: string, presence: LobbyEntry["presence"], extra: Partial<LobbyEntry> = {}): LobbyEntry {
  return { key, name: key, harness: "codex", project: null, intro: null, intro_ts_ms: 0, presence, first_ms: 0, last_ms: 0, ...extra };
}

describe("naming a room", () => {
  test("a typed name is normalized the way the bus does it", () => {
    expect(normalizeRoomName("  #Design Crit ")).toBe("design-crit");
  });

  test("a name the bus would refuse is refused before sending", () => {
    expect(roomNameProblem("", [])).toBe("empty");
    expect(roomNameProblem("bad!name", [])).toBe("invalid");
    expect(roomNameProblem("x".repeat(49), [])).toBe("too_long");
    expect(roomNameProblem("x".repeat(48), [])).toBeNull();
  });

  test("a built-in room's name and a name in use cannot be taken", () => {
    expect(roomNameProblem("commons", [])).toBe("built_in");
    expect(roomNameProblem("orchard", ["orchard"])).toBe("taken");
  });
});

describe("the sidebar", () => {
  test("the three built-in rooms are always there, even when serve lists none", () => {
    expect(sidebarRooms({ channels: [] }).rooms.map((c) => c.id)).toEqual(["lobby", "commons", "feedback"]);
  });

  test("built-ins come first and the operator's rooms follow by name", () => {
    const channels = [room("zeta"), room("alpha"), room("feedback", { built_in: true })];
    expect(sidebarRooms({ channels }).rooms.map((c) => c.id)).toEqual(["lobby", "commons", "feedback", "alpha", "zeta"]);
  });

  test("anything archived leaves its list for the Archived group", () => {
    const channels = [room("kept"), room("old", { archived: true }), project("repo", { archived: true })];
    const { rooms, archived } = sidebarRooms({ channels });
    expect(rooms.map((c) => c.id)).not.toContain("old");
    expect(archived.map((c) => c.id)).toEqual(["old", "repo"]);
    expect(sidebarChannels({ channels }).shown).toEqual([]);
  });

  test("project channels list rooms nowhere and keep quiet ones under More", () => {
    const { shown, more } = sidebarChannels({ channels: [project("a"), project("b", { quiet: true }), room("c")] });
    expect(shown.map((c) => c.id)).toEqual(["a"]);
    expect(more.map((c) => c.id)).toEqual(["b"]);
  });

  test("Home never opens on a room", () => {
    expect(defaultChannelId({ channels: [room("first"), project("repo")] })).toBe("repo");
    expect(defaultChannelId({ channels: [room("only")] })).toBeNull();
  });

  test("a built-in room can be opened even if serve did not list it", () => {
    expect(findChannel({ channels: [] }, "commons")?.built_in).toBe(true);
    expect(findChannel({ channels: [] }, "nope")).toBeUndefined();
  });
});

describe("what the channel menu offers", () => {
  test("a built-in room offers nothing", () => {
    expect(channelActions(room("lobby", { built_in: true }))).toEqual([]);
  });

  test("a project channel can be archived and brought back, never deleted", () => {
    expect(channelActions(project("repo"))).toEqual(["archive"]);
    expect(channelActions(project("repo", { archived: true }))).toEqual(["unarchive"]);
  });

  test("a room offers delete only once it is archived, as serve requires", () => {
    expect(channelActions(room("r"))).toEqual(["archive"]);
    expect(channelActions(room("r", { archived: true }))).toEqual(["unarchive", "delete"]);
  });

  test("the contract's refusal codes are worded; an unknown one keeps serve's message", () => {
    expect(refusalMessage("room_built_in", "x")).not.toBe("x");
    expect(refusalMessage("something_new", "serve said so")).toBe("serve said so");
  });
});

describe("a room's messages", () => {
  test("replies stay in the flat stream and protocol records do not", () => {
    const messages = [message("reply", 1, { reply_to: "m" }), message("record", 2, { structured: "{}" }), message("other", 3, { channels: ["z"] })];
    expect(roomStream({ messages }, "r").map((m) => m.id)).toEqual(["reply"]);
  });

  test("history and the live view join without duplicates, oldest first", () => {
    const merged = mergeRoomMessages([message("b", 2), message("a", 1)], [message("c", 3), message("b", 2, { text: "fresher" })]);
    expect(merged.map((m) => m.id)).toEqual(["a", "b", "c"]);
    expect(merged[1]?.text).toBe("fresher");
  });

  test("a post in #commons or #lobby says it reaches no one's work", () => {
    expect(roomComposerNotice({ id: "commons", reach: "ready" })).toContain("no one's work");
    expect(roomComposerNotice({ id: "lobby" })).toContain("no one's work");
  });

  test("a room someone reads says nothing, and one nobody reads says the post is kept", () => {
    expect(roomComposerNotice({ id: "crit", reach: "ready" })).toBeNull();
    expect(roomComposerNotice({ id: "crit", reach: "none" })).toContain("stays here");
  });

  test("a built-in room always has a purpose to show", () => {
    expect(roomPurpose({ id: "commons", purpose: null })).toContain("not work");
    expect(roomPurpose({ id: "mine", purpose: null })).toBeNull();
    expect(roomPurpose({ id: "mine", purpose: "Ours" })).toBe("Ours");
  });
});

describe("#feedback notes", () => {
  test("the kind comes from the first line and the version from a later one", () => {
    expect(parseFeedback("[bug] It breaks\nharbard 0.42.1")).toEqual({ kind: "bug", text: "It breaks", version: "0.42.1" });
  });

  test("the rest of the note survives between the kind and the version", () => {
    expect(parseFeedback("[idea] One\nTwo\nharbard 1.0.0").text).toBe("One\nTwo");
  });

  test("a line without a known kind is shown whole, with no chip claimed", () => {
    expect(parseFeedback("[weird] Hello")).toEqual({ kind: null, text: "[weird] Hello", version: null });
    expect(parseFeedback("Plain").kind).toBeNull();
  });

  test("a mention of harbard in the first line is not mistaken for the version", () => {
    expect(parseFeedback("[praise] harbard 2").version).toBeNull();
  });
});

describe("the Lobby", () => {
  test("agents group by presence and keep serve's order inside a group", () => {
    const groups = lobbyGroups([entry("a", "ended"), entry("b", "live"), entry("c", "stale"), entry("d", "live"), entry("e", "idle")]);
    expect(groups.map((g) => [g.group, g.entries.map((x) => x.key)])).toEqual([
      ["here", ["b", "d"]],
      ["around", ["c", "e"]],
      ["gone", ["a"]],
    ]);
  });

  test("an agent that has not introduced itself is still listed", () => {
    expect(lobbyGroups([entry("silent", "idle")])[0]?.entries[0]?.intro).toBeNull();
  });

  test("the filter finds by name, project, harness and what it said", () => {
    const list = [entry("ledger", "live", { project: "orchard", intro: "numbers agree" }), entry("mascot", "live", { harness: "cursor" })];
    expect(filterLobby(list, "LEDG").map((e) => e.key)).toEqual(["ledger"]);
    expect(filterLobby(list, "orchard").map((e) => e.key)).toEqual(["ledger"]);
    expect(filterLobby(list, "cursor").map((e) => e.key)).toEqual(["mascot"]);
    expect(filterLobby(list, "agree").map((e) => e.key)).toEqual(["ledger"]);
    expect(filterLobby(list, "  ")).toHaveLength(2);
  });

  test("the lineup shows who is here, or who is around when nobody is", () => {
    expect(lobbyLineup([entry("a", "live"), entry("b", "idle")]).shown.map((e) => e.key)).toEqual(["a"]);
    expect(lobbyLineup([entry("a", "ended"), entry("b", "idle")]).shown.map((e) => e.key)).toEqual(["b"]);
  });

  test("a crowded lineup stops at its limit and says how many more", () => {
    const crowd = Array.from({ length: LINEUP_MAX + 3 }, (_, i) => entry(`a${i}`, "live"));
    const { shown, more } = lobbyLineup(crowd);
    expect(shown).toHaveLength(LINEUP_MAX);
    expect(more).toBe(3);
  });
});
