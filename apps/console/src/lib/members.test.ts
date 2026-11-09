import { describe, expect, test } from "bun:test";
import { channelMembers, memberRole } from "./members";
import type { WorkspaceView, WsAgent, WsChannel } from "./workspace";

// Synthetic: #harbard has five agents, two working now.
const NOW = Date.UTC(2026, 10, 7, 19, 0);
const MIN = 60_000;
const agent = (key: string, presence: WsAgent["presence"], last: number, extra: Partial<WsAgent> = {}) =>
  ({ key, name: key, presence, last_action_ms: NOW - last * MIN, lost: false, label: `general-purpose · ${key}`, ...extra }) as unknown as WsAgent;

const AGENTS = [
  agent("brush", "finished", 90),
  agent("rivet", "working", 5, { role: "constructor", team: "harbard", role_line: "constructor · harbard" }),
  agent("lima", "waiting", 2),
  agent("nut", "working", 1),
  agent("coil", "finished", 300),
];
const channel = { id: "harbard", repo: "/r/harbard", checkouts: [], members: ["brush", "rivet", "lima", "nut", "coil", "gone"], last_ms: 0, warnings: [] } as WsChannel;
const view = { now_ms: NOW, agents: AGENTS, channels: [channel] } as unknown as WorkspaceView;

describe("the channel header's members say what their numbers mean", () => {
  test("a teammate in no channel of its own counts in its lead's", () => {
    const lead = agent("session:lead", "waiting", 1, { channels: ["harbard"] });
    const mate = agent("hammer", "working", 1, { channels: [], lead_session: "lead" });
    const elsewhere = agent("otra", "working", 1, { channels: [], lead_session: "nobody" });
    const room = { ...channel, members: [...channel.members, "session:lead"] } as unknown as WsChannel;
    const v = { ...view, agents: [...AGENTS, lead, mate, elsewhere] } as unknown as WorkspaceView;
    const m = channelMembers(v, room);
    expect(m.now).toBe(3);
    expect(m.ordered.map((a) => a.key)).toContain("hammer");
    expect(m.ordered.map((a) => a.key)).not.toContain("otra");
  });

  test("counts: every agent that worked here, and the ones working now", () => {
    const m = channelMembers(view, channel);
    expect(m.total).toBe(5);
    expect(m.now).toBe(2);
  });

  test("the tooltip and aria-label spell it out", () => {
    expect(channelMembers(view, channel).label).toBe("5 agents have worked in #harbard · 2 now");
  });

  test("singular, and nobody working now", () => {
    const one = { ...channel, members: ["brush"] };
    expect(channelMembers(view, one).label).toBe("1 agent has worked in #harbard · none now");
    expect(channelMembers(view, one).now).toBe(0);
  });

  test("the popover order: working first, newest first, then the rest by last activity", () => {
    expect(channelMembers(view, channel).ordered.map((a) => a.key)).toEqual(["nut", "rivet", "lima", "brush", "coil"]);
  });

  test("the stacked avatars are the first three of that order, so who works now shows", () => {
    expect(channelMembers(view, channel).stack.map((a) => a.key)).toEqual(["nut", "rivet", "lima"]);
  });

  test("a lost or dormant agent is not counted as working, whatever presence says", () => {
    const quiet = { ...view, agents: [agent("x", "working", 1, { lost: true }), agent("y", "working", 50, { dormant: "amber" })] } as WorkspaceView;
    const m = channelMembers(quiet, { ...channel, members: ["x", "y"] });
    expect(m.now).toBe(0);
    expect(m.label).toBe("2 agents have worked in #harbard · none now");
  });

  test("role · team for each row, from the workshop name or its type", () => {
    expect(memberRole(AGENTS[1]!)).toBe("constructor · harbard");
    expect(memberRole(agent("t", "working", 1, { role: "revisora", team: "protocol" }))).toBe("revisora · protocol");
    expect(memberRole(AGENTS[0]!)).toBe("general-purpose · brush");
  });
});
