import { describe, expect, test } from "bun:test";
import {
  applyMention,
  dmHint,
  ellipsizeWords,
  filterAgents,
  matchMentions,
  mentionAt,
  mentionOptions,
  mentionSegments,
  nudgeShown,
  search,
  shouldPull,
  sidebarAgents,
  sidebarChannels,
  temporaryAgents,
  type WsAgent,
  type WsChannel,
  type WorkspaceView,
} from "./workspace";

// Synthetic data only. Parts 4 to 7 of the human protocol.
const NOW = Date.UTC(2026, 10, 7, 17, 0);

function agent(key: string, name: string, extra: Partial<WsAgent> = {}): WsAgent {
  return {
    key,
    name,
    label: `general-purpose · ${key}`,
    lost: false,
    kind: "teammate",
    lead_session: "lead-1",
    lead_quiet_ms: 0,
    delivery: "ready",
    state: "running",
    state_reason: "mid_turn",
    presence: "working",
    dormant: null,
    last_action_ms: NOW - 60_000,
    first_ms: NOW - 3_600_000,
    brief: null,
    description: null,
    checkout: null,
    branch: null,
    build_dir: null,
    channels: ["harbard"],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: false,
    activity: { state: "normal", since_ms: NOW, detail: null, source_ms: NOW } as unknown as WsAgent["activity"],
    reactions: [],
    ...extra,
  };
}

function channel(id: string, extra: Partial<WsChannel> = {}): WsChannel {
  return { id, repo: `/w/${id}`, checkouts: [], members: [], last_ms: NOW, warnings: [], readers: [], reach: "ready", quiet: false, unread: 0, ...extra };
}

const AGENTS = [
  agent("a1", "robots · builder", { handle: "robots-builder", team: "robots" }),
  agent("a2", "robots · reviewer", { handle: "robots-review", team: "robots" }),
  agent("a3", "harbard · dock", { handle: "harbard-dock", team: "harbard" }),
  agent("s1", "Claude · lead", { handle: "coordinator", team: "Claude", kind: "session" }),
  agent("t1", "sandbox · session 16:40", { handle: "t1", temporary: true }),
];
const VIEW = {
  now_ms: NOW,
  agents: AGENTS,
  channels: [channel("harbard"), channel("acme-config", { quiet: true })],
  messages: [],
} as unknown as WorkspaceView;

describe("names, the sidebar and the filter (parts 4.2, 4.3, 4.4)", () => {
  test("the Agents filter matches display name, handle and team", () => {
    expect(filterAgents(AGENTS, "robots").map((a) => a.key)).toEqual(["a1", "a2"]);
    expect(filterAgents(AGENTS, "reviewer").map((a) => a.key)).toEqual(["a2"]);
    expect(filterAgents(AGENTS, "dock").map((a) => a.key)).toEqual(["a3"]);
    expect(filterAgents(AGENTS, "  ").length).toBe(AGENTS.length);
  });

  test("quiet repositories go under More channels, temporary sessions out of Agents", () => {
    const { shown, more } = sidebarChannels(VIEW);
    expect(shown.map((c) => c.id)).toEqual(["harbard"]);
    expect(more.map((c) => c.id)).toEqual(["acme-config"]);
    expect(sidebarAgents(VIEW).shown.map((a) => a.key)).not.toContain("t1");
    expect(temporaryAgents(VIEW).map((a) => a.key)).toEqual(["t1"]);
  });

  test("search finds agents, channels and never a temporary session", () => {
    const r = search(VIEW, "acme");
    expect(r.channels.map((c) => c.id)).toEqual(["acme-config"]);
    expect(search(VIEW, "sandbox").agents).toEqual([]);
    expect(search(VIEW, "builder").agents.map((a) => a.key)).toEqual(["a1"]);
  });

  test("a placeholder is cut at a word, never mid-word", () => {
    expect(ellipsizeWords("harbard · dock", 40)).toBe("harbard · dock");
    const cut = ellipsizeWords("Codex adversarial review of orchard keychain", 20);
    expect(cut).toBe("Codex adversarial…");
    expect(cut).not.toContain("harba");
  });

  test("the DM composer says which way a message goes", () => {
    expect(dmHint(AGENTS[0]!)).toBe("Goes to robots · builder through the lead.");
    expect(dmHint(AGENTS[3]!)).toBe("Goes straight to Claude · lead's session.");
  });
});

describe("mentions (part 4.1)", () => {
  test("@ offers the channel, teams of two or more and agents by display name", () => {
    const options = mentionOptions(VIEW, "harbard");
    expect(options.map((o) => o.handle)).toEqual(["channel", "robots", "coordinator", "harbard-dock", "robots-builder", "robots-review"]);
    expect(options.find((o) => o.handle === "robots")?.hint).toBe("team, 2 agents");
    expect(mentionOptions(VIEW, null).some((o) => o.handle === "channel")).toBe(false);
  });

  test("typing @rev finds the reviewer, and choosing it completes the word", () => {
    const text = "Pass it to @rev";
    const at = mentionAt(text, text.length);
    expect(at).toEqual({ start: 11, query: "rev" });
    expect(matchMentions(mentionOptions(VIEW, null), "rev").map((o) => o.handle)).toEqual(["robots-review"]);
    expect(applyMention(text, at!.start, text.length, "robots-review")).toEqual({ text: "Pass it to @robots-review ", caret: 26 });
    expect(mentionAt("correo a@b.es", 13)).toBeNull();
  });

  test("a mention is highlighted only when the server recognised it", () => {
    expect(mentionSegments("@robots mirad esto y @nadie", ["robots"])).toEqual([
      { text: "@robots", mention: true },
      { text: " mirad esto y @nadie", mention: false },
    ]);
    expect(mentionSegments("sin nada", [])).toEqual([{ text: "sin nada", mention: false }]);
  });
});

describe("Resend comes from the bus", () => {
  test("off for ten minutes after a re-send, whatever the page remembers", () => {
    expect(nudgeShown(NOW - 60_000, NOW)).toBe(true);
    expect(nudgeShown(NOW - 11 * 60_000, NOW)).toBe(false);
    expect(nudgeShown(null, NOW)).toBe(false);
  });
});

describe("live answers (part 5.1)", () => {
  test("a new rev in the two-second slice pulls the full view", () => {
    expect(shouldPull("aaa", "bbb")).toBe(true);
    expect(shouldPull("aaa", "aaa")).toBe(false);
    expect(shouldPull(undefined, "bbb")).toBe(false);
  });
});
