import { describe, expect, test } from "bun:test";
import { calmModel } from "./calm";
import {
  sessionGoneNotice,
  activitySelection,
  entryId,
  cardBody,
  dmHint,
  needsItems,
  readAllNotice,
  readMarkOf,
  replyCount,
  sessionGone,
  threads,
  todoItems,
  type WsAgent,
  type WsCard,
  type WsMessage,
  type WorkspaceView,
} from "./workspace";

// Synthetic data only.
const NOW = Date.UTC(2026, 10, 7, 21, 0);
const MIN = 60_000;

const agent = (key: string, name: string, extra: Partial<WsAgent> = {}): WsAgent =>
  ({ key, name, kind: "teammate", lead_session: "lead-1", delivery: "ready", lost: false, presence: "waiting", ...extra }) as unknown as WsAgent;
const message = (id: string, agentKey: string, ts: number, extra: Partial<WsMessage> = {}): WsMessage =>
  ({ id, agent: agentKey, from: "agent", author: agentKey, ts_ms: ts, text: `text ${id}`, label: null, channels: [], reply_to: null, replies: [], mentions: [], ...extra }) as unknown as WsMessage;
const card = (id: string, kind: WsCard["kind"], agentKey: string, msg: string | null, since: number, extra: Partial<WsCard> = {}): WsCard => ({
  id,
  kind,
  channel: null,
  agents: [agentKey],
  message: msg,
  checkout: null,
  since_ms: since,
  resolved_ms: null,
  needs: true,
  ...extra,
});
const view = (agents: WsAgent[], messages: WsMessage[], cards: WsCard[]): WorkspaceView =>
  ({ now_ms: NOW, operator: "Dana", agents, messages, cards, deliveries: [] }) as unknown as WorkspaceView;
const ids = (v: WorkspaceView) => todoItems(v).map((e) => (e.kind === "card" ? `card:${e.card.id}` : `msg:${e.message.id}`));

describe("All (AC3)", () => {
  test("shows an open BLOCKED once, as its card, and an answered one as its message", () => {
    const v = view(
      [agent("a1", "Ethan"), agent("a2", "Felix")],
      [
        message("b-open", "a1", NOW - 30 * MIN, { label: "BLOCKED", text: "Should I use the new API or the old one?" }),
        message("b-done", "a2", NOW - 50 * MIN, { label: "BLOCKED", text: "Should I drop the table?" }),
      ],
      [card("blocked:b-open", "blocked", "a1", "b-open", NOW - 30 * MIN)],
    );
    expect(ids(v)).toEqual(["card:blocked:b-open", "msg:b-done"]);
  });

  test("a mention is one row, its card, not the card and the message again", () => {
    const v = view(
      [agent("m1", "Sarah")],
      [message("m", "m1", NOW - 10 * MIN, { label: "DONE", text: "@dana publish it?" })],
      [card("mention:m", "mention", "m1", "m", NOW - 10 * MIN)],
    );
    expect(ids(v)).toEqual(["card:mention:m"]);
  });
});

describe("a BLOCKED nobody will read (AC6)", () => {
  const gone = agent("z1", "Owen", { lost: true, delivery: "not_on_bus" });
  const offBus = agent("z2", "Simon", { delivery: "not_on_bus" });
  const alive = agent("a1", "Ethan");

  test("its agent's session is gone when lost or its lead is off the bus or ended", () => {
    expect(sessionGone(gone)).toBe(true);
    expect(sessionGone(offBus)).toBe(true);
    expect(sessionGone(agent("e", "E", { delivery: "ended" }))).toBe(true);
    expect(sessionGone(alive)).toBe(false);
    expect(sessionGone(agent("s", "S", { delivery: "stale" }))).toBe(false, "stale may come back");
  });

  test("the reply box says nobody will read it before they write, not 'I will pass it on…'", () => {
    expect(sessionGoneNotice()).toBe("Its session is gone; nobody will read this");
    expect(dmHint(gone)).toBe("Its session is gone; nobody will read this.");
    expect(dmHint(alive)).toBe("Goes to Ethan through the lead.");
  });

  test("the card and the calm card say so", () => {
    const v = view([gone], [message("q", "z1", NOW - 5 * 60 * MIN, { label: "BLOCKED", text: "Should I drop the old table?" })], [card("blocked:q", "blocked", "z1", "q", NOW - 5 * 60 * MIN)]);
    expect(cardBody(v.cards[0]!, v, NOW).text).toBe("Its session is gone; nobody will read this.");
    expect(calmModel(v, new Set(), NOW).top?.detail).toBe("Its session is gone; nobody will read this.");
  });
});

describe("All is everything (AC9)", () => {
  test("unlabelled agent messages, the operator's notes and bus notes too; never a lead's word, a protocol record or a message twice", () => {
    const v = view(
      [agent("a1", "Ethan")],
      [
        message("plain", "a1", NOW - 1 * MIN, { text: "Carrying on" }),
        message("mine", "a1", NOW - 2 * MIN, { from: "operator", author: "Dana", text: "How is it going?" }),
        message("bus:1", "a1", NOW - 3 * MIN, { from: "session", text: "A note on the bus" }),
        message("lead", "a1", NOW - 4 * MIN, { from: "lead", text: "Thanks" }),
        message("proto", "a1", NOW - 5 * MIN, { structured: "shutdown_request", text: "{}" }),
        message("asked", "a1", NOW - 6 * MIN, { from: "operator", text: "Anything?" }),
      ],
      [card("unanswered:note:asked", "unanswered", "a1", "asked", NOW - 6 * MIN)],
    );
    expect(ids(v)).toEqual(["msg:plain", "msg:mine", "msg:bus:1", "card:unanswered:note:asked"]);
  });
});

describe("Threads (AC9)", () => {
  test("sorts by the last reply, not by the first message", () => {
    const v = view(
      [agent("a1", "Owen"), agent("a2", "Iris")],
      [
        message("rivet", "a1", NOW - 120 * MIN, { replies: ["r1"] }),
        message("r1", "a1", NOW - 5 * MIN, { from: "lead", reply_to: "rivet" }),
        message("coil", "a2", NOW - 60 * MIN, { replies: ["r2"] }),
        message("r2", "a2", NOW - 50 * MIN, { from: "lead", reply_to: "coil" }),
      ],
      [],
    );
    expect(threads(v).map((m) => m.id)).toEqual(["rivet", "coil"]);
  });

  test("'1 reply', singular (AC10)", () => {
    expect(replyCount(1)).toBe("1 reply");
    expect(replyCount(3)).toBe("3 replies");
  });
});

describe("one urgency order (AC8)", () => {
  test("Needs you and calm agree: a BLOCKED of hours ago comes before a dormant agent of minutes ago", () => {
    const v = view(
      [agent("b", "Felix"), agent("d", "Iris"), agent("m", "Sarah"), agent("u", "Stella")],
      [message("q", "b", NOW - 5 * 60 * MIN, { label: "BLOCKED" }), message("men", "m", NOW - 20 * MIN), message("note", "u", NOW - 30 * MIN, { from: "operator" })],
      [
        card("dormant:d", "dormant", "d", null, NOW - 1 * MIN),
        card("unanswered:note", "unanswered", "u", "note", NOW - 30 * MIN),
        card("mention:men", "mention", "m", "men", NOW - 20 * MIN),
        card("blocked:q", "blocked", "b", "q", NOW - 5 * 60 * MIN),
      ],
    );
    expect(needsItems(v).map((c) => c.kind)).toEqual(["blocked", "mention", "unanswered", "dormant"]);
    expect(calmModel(v, new Set(), NOW).top?.id).toBe(`card:${needsItems(v)[0]!.id}`);
    const later = new Set([`card:${needsItems(v)[0]!.id}`]);
    expect(calmModel(v, later, NOW).top?.id).toBe(`card:${needsItems(v)[1]!.id}`);
  });
});

describe("reading in Activity (AC7)", () => {
  test("Esc marks the open item read up to its own time, for its agent", () => {
    const v = view([agent("a1", "Ethan")], [message("m", "a1", NOW - 3 * MIN, { label: "DONE" })], [card("mention:m", "mention", "a1", "m", NOW - 3 * MIN)]);
    expect(readMarkOf(v, { kind: "message", message: v.messages[0]! })).toEqual({ agent: "a1", ts_ms: NOW - 3 * MIN });
    expect(readMarkOf(v, { kind: "card", card: v.cards[0]! })).toEqual({ agent: "a1", ts_ms: NOW - 3 * MIN });
    expect(readMarkOf(v, { kind: "card", card: card("two", "two_writers", "", null, NOW) })).toBeNull();
  });

  test("'Mark everything as read' says what it cleared and that Needs you does not change", () => {
    expect(readAllNotice({ directs: 4, channels: 2, activity: 6, todo: 3 })).toBe(
      "Marked as read: 4 in Direct messages and 2 in channels. Needs you doesn't change: each alert is resolved separately.",
    );
    expect(readAllNotice({ directs: 1, channels: 0, activity: 0, todo: 0 })).toBe(
      "Marked as read: 1 in Direct messages and 0 in channels. Needs you doesn't change: each alert is resolved separately.",
    );
    expect(readAllNotice({ directs: 0, channels: 0, activity: 2, todo: 0 })).toBe("Nothing was left unread. Needs you doesn't change: each alert is resolved separately.");
  });
});

describe("what Activity opens", () => {
  const entry = (id: string, resolved: number | null = null) =>
    ({ kind: "card", card: card(id, "handoff", "a", null, NOW, { resolved_ms: resolved } as never) }) as Parameters<typeof entryId>[0];

  test("a resolved card leaves the panel too: the next one takes its place", () => {
    const [a, b, c] = [entry("a"), entry("b"), entry("c")];
    expect(activitySelection([a, c], null, b, 1)).toBe(c);
  });

  test("resolving the last one opens the one above, and an empty list opens nothing", () => {
    const [a, b] = [entry("a"), entry("b")];
    expect(activitySelection([a], null, b, 1)).toBe(a);
    expect(activitySelection([], null, a, 0)).toBeNull();
  });

  test("an entry still listed is read fresh, so its button tells the truth", () => {
    const stale = entry("a");
    const fresh = entry("a", NOW);
    expect(activitySelection([fresh], null, stale, 0)).toBe(fresh);
  });

  test("a message being read stays open when it leaves the unread list", () => {
    const read = { kind: "message", message: message("m", "a", NOW) } as Parameters<typeof entryId>[0];
    expect(activitySelection([entry("a")], null, read, 0)).toBe(read);
  });

  test("what the address names wins", () => {
    const [a, b] = [entry("a"), entry("b")];
    expect(activitySelection([a, b], entryId(b), a, 0)).toBe(b);
  });
});
