import { describe, expect, test } from "bun:test";
import {
  labelChip,
  withoutLabelPrefix,
  moreTools,
  noDelivery,
  passwordNotice,
  railItems,
  byDay,
  cardBody,
  cardReason,
  channelStream,
  nobodyLive,
  noReader,
  attachmentUrl,
  beforeAfter,
  channelComposer,
  composerBlock,
  composerConfirm,
  composerNotice,
  agentRelation,
  deliveryLine,
  dmHint,
  noteReader,
  conversation,
  dayLabel,
  destinationOf,
  duration,
  sidebarAgents,
  dotTone,
  monogram,
  openCards,
  openFile,
  presenceLine,
  repeatsOf,
  replyBody,
  replyError,
  search,
  thread,
  tone,
  type WsAgent,
  type WsAttachment,
  type WsChannel,
  type WsMessage,
  type WorkspaceView,
} from "./workspace";

// 2026-11-06T22:41:00Z. Synthetic data only.
const NOW = Date.UTC(2026, 10, 6, 22, 41);
const MIN = 60_000;

function agent(key: string, name: string, extra: Partial<WsAgent> = {}): WsAgent {
  return {
    key,
    name,
    kind: "teammate",
    lead_session: "lead-1",
    delivery: "ready",
    state: "running",
    state_reason: "mid_turn",
    presence: "working",
    dormant: null,
    last_action_ms: NOW - MIN,
    first_ms: NOW - 60 * MIN,
    brief: "Synthetic brief",
    description: "Synthetic task",
    checkout: "/work/orchard-wt-identity",
    branch: "keychain",
    build_dir: null,
    channels: ["orchard"],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: false,
    label: `general-purpose · ${key}`,
    lead_quiet_ms: 0,
    lost: false,
    ...extra,
  };
}

function message(id: string, agentKey: string, ts: number, extra: Partial<WsMessage> = {}): WsMessage {
  return {
    id,
    agent: agentKey,
    from: "agent",
    author: agentKey,
    ts_ms: ts,
    text: `text ${id}`,
    body: null,
    label: null,
    version: null,
    flags: [],
    jargon: [],
    repeat_of: null,
    chars: 10,
    files: [],
    verified: null,
    channels: ["orchard"],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    attachments: [],
    delivery: null,
    ...extra,
  };
}

function view(agents: WsAgent[], messages: WsMessage[]): WorkspaceView {
  return {
    v: 1,
    now_ms: NOW,
    operator: "Dana",
    operator_initials: "DR",
    thresholds: { amber_ms: 15 * MIN, red_ms: 45 * MIN },
    channels: [{ id: "orchard", repo: "/work/orchard", checkouts: [], members: agents.map((a) => a.key), last_ms: NOW, warnings: [] }],
    agents,
    messages,
    cards: [],
    deliveries: [],
    saved: [],
    unread: { directs: 0, activity: 0 },
    team_root_found: true,
    format_changed: false,
  };
}

describe("the rail", () => {
  test("has exactly Home, Direct, Activity, Later, in that order", () => {
    expect(railItems().map((d) => d.label)).toEqual(["Home", "DMs", "Activity", "Later"]);
  });

  test("every page of the shell belongs to one destination", () => {
    expect(destinationOf("/")).toBe("home");
    expect(destinationOf("/c/orchard")).toBe("home");
    expect(destinationOf("/threads")).toBe("home");
    expect(destinationOf("/deliveries")).toBe("home");
    expect(destinationOf("/direct/a1")).toBe("direct");
    expect(destinationOf("/activity")).toBe("activity");
    expect(destinationOf("/later")).toBe("later");
    expect(destinationOf("/search")).toBeNull();
  });

  test("the tools live under More tools, never in the rail", () => {
    const rail = new Set(railItems().map((d) => d.href));
    for (const m of moreTools()) expect(rail.has(m.href)).toBe(false);
    expect(moreTools().map((m) => m.title)).toEqual(["Strip", "History", "Set up harbard"]);
  });

  test("the avatar menu carries the fixed password notice", () => {
    expect(passwordNotice()).toBe(
      "harbard never asks you for a password. If a window asks for one and you didn't trigger it, cancel it.",
    );
  });
});

describe("wording", () => {
  test("labels become the four chips", () => {
    expect(labelChip("START")).toEqual({ text: "Starting", tone: "neutral" });
    expect(labelChip("FINDING")).toEqual({ text: "Finding", tone: "amber" });
    expect(labelChip("BLOCKED")).toEqual({ text: "Blocked", tone: "red" });
    expect(labelChip("DONE")).toEqual({ text: "Delivered", tone: "green" });
  });

  test("monograms read well", () => {
    expect(monogram("keychain")).toBe("KC");
    expect(monogram("keychain-review-2")).toBe("KR");
    expect(monogram("harbard-view")).toBe("HV");
    expect(monogram("identity-review")).toBe("IR");
    expect(monogram("mind-backup")).toBe("MB");
    expect(monogram("acme-a3")).toBe("AA");
    expect(monogram("Dana Reyes")).toBe("DR");
    expect(tone("keychain")).toBe(tone("keychain"));
  });

  test("days are Today, Yesterday, then a date, on the operator's clock", () => {
    expect(dayLabel(NOW - 10 * MIN, NOW)).toBe("Today");
    expect(dayLabel(NOW - 24 * 60 * MIN, NOW)).toBe("Yesterday");
    expect(dayLabel(Date.UTC(2026, 10, 3, 12), NOW)).toBe("Tuesday 3 November");
    const groups = byDay([message("a", "x", NOW - 25 * 60 * MIN), message("b", "x", NOW - 2 * MIN), message("c", "x", NOW - MIN)], NOW);
    expect(groups.map((g) => [g.day, g.items.length])).toEqual([["Yesterday", 1], ["Today", 2]]);
  });

  test("presence: working, waiting, finished; dormant amber then red", () => {
    expect(presenceLine(agent("a", "keychain"), NOW)).toBe("working · 1 min ago");
    expect(presenceLine(agent("a", "x", { presence: "waiting", last_action_ms: NOW - 3 * MIN }), NOW)).toBe("waiting · 3 min ago");
    const quiet = agent("a", "x", { dormant: "amber", last_action_ms: Date.UTC(2026, 10, 6, 22, 20) });
    expect(presenceLine(quiet, NOW)).toBe("no activity since 22:20");
    expect(dotTone(quiet)).toBe("amber");
    expect(dotTone({ presence: "working", dormant: "red" })).toBe("red");
    expect(dotTone({ presence: "working", dormant: null })).toBe("green");
    expect(dotTone({ presence: "waiting", dormant: null })).toBe("amber");
    expect(dotTone({ presence: "finished", dormant: null })).toBe("grey");
  });

  test("each card says why it reaches the operator", () => {
    expect(cardReason({ kind: "blocked", channel: "orchard" })).toBe("Blocked in #orchard");
    expect(cardReason({ kind: "approval", channel: "acme" })).toBe("Approval in #acme");
    expect(cardReason({ kind: "two_writers", channel: "orchard" })).toBe("Alert in #orchard");
    expect(cardReason({ kind: "dormant", channel: "orchard" })).toBe("Alert in #orchard");
    const v = view([agent("k", "keychain"), agent("r", "keychain-review-2")], []);
    const two = cardBody(
      { id: "c", kind: "two_writers", channel: "orchard", agents: ["k", "r"], message: null, checkout: "/work/orchard-wt-identity", since_ms: NOW - 5 * MIN, resolved_ms: null, needs: true },
      v,
      NOW,
    );
    expect(two).toEqual({ title: "Two agents in the same checkout", text: "keychain and keychain-review-2 in orchard-wt-identity." });
    const seven = cardBody(
      { id: "c", kind: "two_writers", channel: "orchard", agents: ["a", "b", "c", "d", "e", "f", "g"], message: null, checkout: "/w/x", since_ms: NOW, resolved_ms: null, needs: true },
      v,
      NOW,
    );
    expect(seven.title).toBe("7 agents in the same checkout");
    const past = cardBody(
      { id: "c", kind: "two_writers", channel: "orchard", agents: ["k", "r"], message: null, checkout: "/w/x", since_ms: NOW, resolved_ms: null, needs: false },
      v,
      NOW,
    );
    expect(past.text).toBe("keychain and keychain-review-2 in x. They no longer write at the same time; kept as history.");
    const dormant = cardBody(
      { id: "d", kind: "dormant", channel: "orchard", agents: ["k"], message: null, checkout: null, since_ms: NOW - 52 * MIN, resolved_ms: null, needs: true },
      v,
      NOW,
    );
    expect(dormant.text).toBe("keychain has been mid-turn doing nothing since 21:49 (52 min).");
    expect(duration(25 * 60 * MIN)).toBe("25 h");
  });
});

describe("selections", () => {
  test("a channel shows top-level messages; replies open in their thread, in order", () => {
    const parent = message("p", "r", NOW - 30 * MIN, { label: "FINDING", replies: ["r2", "r1"] });
    const r1 = message("r1", "r", NOW - 20 * MIN, { from: "lead", reply_to: "p" });
    const r2 = message("r2", "r", NOW - 10 * MIN, { from: "operator", reply_to: "p" });
    const v = view([agent("r", "keychain-review-2")], [parent, r1, r2]);
    expect(channelStream(v, "orchard").map((m) => m.id)).toEqual(["p"]);
    expect(thread(v, "p").map((m) => m.id)).toEqual(["r1", "r2"]);
    expect(conversation(v, "r").map((m) => m.id)).toEqual(["p", "r1", "r2"]);
  });

  test("the sidebar lists agents active in 24 h, newest first, eight before See all", () => {
    // Working agents: an idle one quiet for an hour would be under Inactive.
    const working = { state: "running", since_ms: NOW, detail: null, source_ms: NOW } as unknown as WsAgent["activity"];
    const agents = Array.from({ length: 10 }, (_, i) => agent(`a${i}`, `agent-${i}`, { last_action_ms: NOW - (i + 1) * 60 * MIN, activity: working }));
    agents.push(agent("old", "old", { presence: "finished", last_action_ms: NOW - 25 * 60 * MIN }));
    const { shown, rest } = sidebarAgents(view(agents, []));
    expect(shown.map((a) => a.key)).toEqual(["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"]);
    expect(rest.map((a) => a.key)).toEqual(["a8", "a9"]);
  });

  test("Needs you leaves out history and resolved cards", () => {
    const base = { kind: "two_writers" as const, channel: "orchard", agents: [], message: null, checkout: "/w/x", since_ms: NOW };
    const v = view([], []);
    v.cards = [
      { ...base, id: "live", resolved_ms: null, needs: true },
      { ...base, id: "old", resolved_ms: null, needs: false },
      { ...base, id: "done", resolved_ms: NOW, needs: true },
    ];
    expect(openCards(v).map((c) => c.id)).toEqual(["live"]);
  });

  test("an agent nothing is known about since reads no data, grey", () => {
    const lost = agent("l", "legibility-qa", { lost: true, presence: "finished", last_action_ms: Date.UTC(2026, 10, 5, 21, 46) });
    expect(presenceLine(lost, NOW)).toBe("no data since 5 Nov at 21:46");
    expect(dotTone(lost)).toBe("grey");
    const today = agent("t", "x", { lost: true, presence: "finished", last_action_ms: NOW - 3 * 60 * MIN });
    expect(presenceLine(today, NOW)).toBe("no data since 19:41");
  });

  test("repeats collapse under the first copy", () => {
    const first = message("m1", "k", NOW - 10 * MIN);
    const again = message("m2", "k", NOW - 5 * MIN, { flags: ["repeat"], repeat_of: "m1" });
    const v = view([agent("k", "keychain")], [first, again]);
    expect(repeatsOf(v, "m1").map((m) => m.id)).toEqual(["m2"]);
  });

  test("one search field finds agents, messages and reports", () => {
    const m = message("m", "k", NOW, { text: "Team view ready", files: ["/Users/x/harbard-view.md"] });
    const v = view([agent("k", "harbard-view")], [m]);
    expect(search(v, "team").messages.map((x) => x.id)).toEqual(["m"]);
    expect(search(v, "harbard-view").agents.map((a) => a.key)).toEqual(["k"]);
    expect(search(v, "view.md").reports.map((r) => r.path)).toEqual(["/Users/x/harbard-view.md"]);
    expect(search(v, "  ").messages).toEqual([]);
  });
});

describe("writes", () => {
  test("opening a file card asks serve to open that local path, and nothing else", async () => {
    const calls: [string, Record<string, unknown>][] = [];
    await openFile("/Users/x/Documents/keychain.md", async (action, body) => {
      calls.push([action, body]);
      return { ok: true };
    });
    expect(calls).toEqual([["open", { path: "/Users/x/Documents/keychain.md" }]]);
  });

  test("a reply names the agent or the channel; serve picks the lead session", () => {
    expect(replyBody({ agent: "k" }, "hello")).toEqual({ agent: "k", text: "hello" });
    expect(replyBody({ agent: "k", replyTo: "m1" }, "hello")).toEqual({ agent: "k", reply_to: "m1", text: "hello" });
    expect(replyBody({ channel: "orchard" }, "hello")).toEqual({ channel: "orchard", text: "hello" });
  });

  test("a stale lead: the note is stored and the composer says it will wait", () => {
    const stale = agent("k", "keychain", { delivery: "stale", lead_quiet_ms: 5 * 60 * MIN });
    expect(composerBlock([stale])).toBeNull();
    expect(composerNotice([stale])).toBe("its main session has been silent for 5 h; the note will wait for it to come back");
    expect(composerNotice([agent("r", "ready")])).toBeNull();
    expect(composerNotice([stale, agent("r", "ready")])).toBeNull();
    expect(composerNotice([agent("s", "s", { delivery: "stale", lead_quiet_ms: 20 * MIN })])).toBe(
      "its main session has been silent for 20 min; the note will wait for it to come back",
    );
  });

  test("with no live session the composer asks before sending, and blocks only with no session at all", () => {
    expect(composerBlock([agent("k", "keychain")])).toBeNull();
    // An agent always has a lead to leave the note for: confirmed, not blocked.
    expect(composerBlock([agent("k", "keychain", { delivery: "not_on_bus" })])).toBeNull();
    expect(composerConfirm([agent("k", "keychain", { delivery: "not_on_bus" })])).toBe(nobodyLive());
    expect(nobodyLive()).toBe("Nobody will read it until a session opens; send anyway?");
    expect(composerConfirm([agent("k", "keychain", { delivery: "stale" })])).toBeNull();
    expect(composerConfirm([agent("a", "a", { delivery: "ended" }), agent("b", "b")])).toBeNull();
    expect(composerBlock([])).toBe(noReader());
    expect(replyError(409, "not_on_bus")).toBe(noDelivery());
    expect(replyError(409, "no_reachable_member")).toContain("Nobody in this channel");
    // The stale line of today is gone everywhere.
    for (const text of [noDelivery(), noReader(), replyError(409, "no_reachable_member")]) {
      expect(text).not.toContain("yet");
    }
    expect(replyBody({ channel: "harbard" }, "hello", true)).toEqual({ channel: "harbard", text: "hello", anyway: true });
  });

  test("a channel composer follows the server's reach", () => {
    const ch = (reach: WsChannel["reach"]) => channelComposer({ reach, readers: [] });
    expect(ch("ready")).toEqual({ blocked: null, notice: null, confirm: null });
    expect(ch("stale").notice).toContain("will wait");
    expect(ch("none")).toEqual({ blocked: null, notice: null, confirm: nobodyLive() });
    expect(ch("nobody").blocked).toBe(noReader());
    // An older serve sends no reach: the composer stays open and serve answers.
    expect(channelComposer({ readers: [] } as unknown as WsChannel)).toEqual({ blocked: null, notice: null, confirm: null });
  });
});

describe("the operator's notes say what became of them", () => {
  const d = (state: "sent" | "delivered" | "read" | "answered", extra = {}) => ({
    state,
    sessions: ["lead-1"],
    delivered: state === "sent" ? 0 : 1,
    to_lead: true,
    nudged_ms: null,
    ...extra,
  });

  test("a note its lead answered says who answered, not that the agent did", () => {
    expect(deliveryLine(d("answered", { answered_by: "Walter" }), "Walter")).toBe("Replied by Walter");
    expect(deliveryLine(d("answered", { answered_by: null }), "Walter")).toBe("Replied");
  });

  test("a note names who reads it when the conversation knows", () => {
    expect(deliveryLine(d("delivered"), "Walter")).toBe("Delivered to Walter");
    expect(deliveryLine(d("read"), "Walter")).toBe("Read by Walter");
    expect(deliveryLine(d("delivered", { to_lead: false, sessions: ["40e37185-46c4"] }), "Oscar")).toBe("Delivered to Oscar");
  });

  test("the reader of a note is the session itself, or the lead of an agent", () => {
    const lead = agent("session:lead-1", "Walter", { kind: "session" });
    const helper = agent("a1", "Benjamin");
    const orphan = agent("a2", "Nora", { lead_session: "gone" });
    const agents = [lead, helper, orphan];
    expect(noteReader(agents, "session:lead-1")).toBe("Walter");
    expect(noteReader(agents, "a1")).toBe("Walter");
    expect(noteReader(agents, "a2")).toBeNull();
    expect(noteReader(agents, "")).toBeNull();
  });

  test("a conversation says what it is talking to", () => {
    const lead = agent("session:lead-1", "Walter", { kind: "session" });
    expect(agentRelation(lead, [lead])).toBe("Main session");
    expect(agentRelation(agent("a1", "Benjamin"), [lead])).toBe("Teammate of Walter");
    expect(agentRelation(agent("a1", "Benjamin", { kind: "subagent" }), [lead])).toBe("Subagent of Walter");
    expect(agentRelation(agent("a1", "Benjamin", { lead_session: "gone" }), [lead])).toBe("Teammate of another session");
  });

  test("the composer says an agent does not read its notes itself", () => {
    expect(dmHint(agent("a1", "Benjamin"), "Walter")).toBe("Benjamin doesn't read this directly: it goes to Walter, who has to pass it on.");
    expect(dmHint(agent("session:s", "Oscar", { kind: "session" }), "Oscar")).toBe("Goes straight to Oscar's session.");
  });

  test("sent, delivered, read, answered", () => {
    expect(deliveryLine(d("sent"))).toBe("Sent");
    expect(deliveryLine(d("delivered"))).toBe("Delivered to the lead");
    expect(deliveryLine(d("read"))).toBe("Read by the lead");
    expect(deliveryLine(d("answered"))).toBe("Replied");
    expect(deliveryLine(d("delivered", { sessions: ["a", "b"], delivered: 1 }))).toBe("Delivered to 1 of 2 sessions");
    expect(deliveryLine(d("delivered", { to_lead: false, sessions: ["0123456789"] }))).toBe("Delivered to session 01234567");
    expect(deliveryLine(d("sent", { nudged_ms: NOW }))).toBe("Sent · re-sent at 22:41");
  });

  test("a no reply card names the channel and quotes the note", () => {
    const note = message("note:1", "", NOW - 12 * MIN, {
      from: "operator",
      author: "Dana",
      agent: "",
      text: "Probando, quiero saber si esto realmente funciona ya",
      channels: ["harbard"],
      delivery: d("delivered"),
    });
    const card = { id: "unanswered:note:1", kind: "unanswered" as const, channel: "harbard", agents: [], message: "note:1", checkout: null, since_ms: NOW - 12 * MIN, resolved_ms: null, needs: true };
    const v = { ...view([], [note]), cards: [card] };
    expect(cardReason(card)).toBe("No reply in #harbard");
    expect(cardBody(card, v, NOW)).toEqual({
      title: "Nobody has answered your message in #harbard",
      text: "“Probando, quiero saber si esto realmente funciona ya”, at 22:29.",
    });
  });
});

describe("jargon and attachments", () => {
  test("an attachment is read through the console's proxy, and antes/after pair up", () => {
    const att = (id: string, label: WsAttachment["label"], kind: WsAttachment["kind"] = "image"): WsAttachment => ({
      id,
      name: id.split("/")[1]!,
      kind,
      label,
      bytes: 10,
      path: `/h/.harbard/attachments/${id}`,
    });
    expect(attachmentUrl(att("01A/before 1.png", "before"))).toBe("/api/harbard/attachments/01A/before%201.png");
    const pair = beforeAfter([att("01A/a.png", "before"), att("01C/r.md", "report", "markdown"), att("01B/d.png", "after")]);
    expect([pair?.before.id, pair?.after.id]).toEqual(["01A/a.png", "01B/d.png"]);
    expect(beforeAfter([att("01A/a.png", "before")])).toBeNull();
  });
});

describe("withoutLabelPrefix", () => {
  test("a labelled message keeps only its sentence beside the chip", () => {
    expect(withoutLabelPrefix("DONE · cart · The cart drawer updates its total.", "DONE")).toBe("The cart drawer updates its total.");
  });
  test("a revision without a name keeps what changed", () => {
    expect(withoutLabelPrefix("DONE v2 · Fixed the rounding.", "DONE")).toBe("Fixed the rounding.");
  });
  test("a sentence with its own separators keeps them", () => {
    expect(withoutLabelPrefix("FINDING · i18n · Totals · German prices round early.", "FINDING")).toBe("Totals · German prices round early.");
  });
  test("only the first line is touched", () => {
    expect(withoutLabelPrefix("BLOCKED · migrations · May I lock it?\nDetails follow.", "BLOCKED")).toBe("May I lock it?\nDetails follow.");
  });
  test("an unlabelled message or another shape is shown as written", () => {
    expect(withoutLabelPrefix("DONE · cart · x", null)).toBe("DONE · cart · x");
    expect(withoutLabelPrefix("Done with the cart", "DONE")).toBe("Done with the cart");
    expect(withoutLabelPrefix("START · cart · x", "DONE")).toBe("START · cart · x");
    expect(withoutLabelPrefix("DONE · cart", "DONE")).toBe("DONE · cart");
  });
});
