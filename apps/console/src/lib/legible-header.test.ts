import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WorkspaceView, WsAgent, WsChannel } from "./workspace";
import { channelHeadline, channelPlaces } from "./legible";
import { ChannelHeader } from "@/components/workspace/home";

// A busy repository: 32 copies, three branches named in
// the header, 37 sessions in the last 24 h, one working now.
const NOW = new Date(2026, 10, 7, 22, 15).getTime();
const MIN = 60_000;
const agent = (key: string, last: number, presence: WsAgent["presence"] = "finished") =>
  ({ key, name: key, presence, dormant: null, lost: false, last_action_ms: NOW - last * MIN, label: key, reactions: [] }) as unknown as WsAgent;
const today = Array.from({ length: 36 }, (_, i) => agent(`cap${i}`, 10 + i));
const working = agent("cap86fix", 1, "working");
const yesterday = agent("old", 23 * 60);
const AGENTS = [...today, working, yesterday];
const REPO = "/Users/p/Projects/acme";
const ACME = {
  id: "acme",
  repo: REPO,
  checkouts: [
    { path: REPO, branch: "main" },
    { path: `${REPO}/.worktrees/cli-feedback`, branch: "feat/cli-feedback" },
    { path: `${REPO}/.worktrees/cli-flow-progress`, branch: "feat/cli-flow-progress" },
  ],
  members: AGENTS.map((a) => a.key),
  recent: AGENTS.map((a) => a.key),
  last_ms: NOW,
  active_ms: NOW - MIN,
  warnings: [],
} as unknown as WsChannel;
const view = { now_ms: NOW, agents: AGENTS, channels: [ACME], messages: [], cards: [] } as unknown as WorkspaceView;

describe("the channel header, without noise", () => {
  test("sessions today and who works now: today since midnight, not the last 24 h", () => {
    const head = channelHeadline(view, ACME);
    expect(head.sessions).toBe("37 sessions today");
    expect(head.now).toBe("1 working now");
    expect(head.agents.map((a) => a.key)).not.toContain("old");
  });

  test("nobody working: it says so; nobody today: the last 24 h", () => {
    const idle = { ...view, agents: AGENTS.map((a) => ({ ...a, presence: "finished" })) } as WorkspaceView;
    expect(channelHeadline(idle, ACME).now).toBe("nobody working now");
    const late = { ...ACME, recent: ["old"] } as WsChannel;
    expect(channelHeadline(view, late).sessions).toBe("1 session in the last 24 h");
    expect(channelHeadline({ ...view, agents: [agent("a", 1)] } as WorkspaceView, { ...ACME, recent: ["a"] }).sessions).toBe("1 session today");
  });

  test("paths and branches go behind the 'ℹ': the repository, then each copy with its branch", () => {
    const places = channelPlaces(ACME);
    expect(places.title).toBe("3 copies of ~/Projects/acme");
    // Under the repository a copy reads by its own folder: a long common
    // prefix cut the worktree name off.
    expect(places.copies[0]).toEqual({ path: "main copy", full: REPO, branch: "main" });
    expect(places.copies[1]).toEqual({ path: ".worktrees/cli-feedback", full: `${REPO}/.worktrees/cli-feedback`, branch: "feat/cli-feedback" });
    expect(channelPlaces({ ...ACME, checkouts: [{ path: "/Users/p/elsewhere/acme-copy", branch: null }] } as unknown as WsChannel).copies[0]!.path).toBe("~/elsewhere/acme-copy");
  });

  test("the header reads acme, its sessions today and who works now, with no path or branch in sight", () => {
    const out = renderToStaticMarkup(h(ChannelHeader, { view, channel: ACME }));
    const visible = out.replace(/<details class="hb-info"[\s\S]*?<\/details>/, "").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
    expect(visible).toContain("acme");
    expect(visible).toContain("37 sessions today");
    expect(visible).toContain("1 working now");
    expect(visible).not.toContain("copies");
    expect(visible).not.toContain("feat/");
    expect(visible).not.toContain("~/Projects");
    expect(out).toContain('aria-label="Checkouts and branches of #acme"');
    expect(out).toContain("feat/cli-flow-progress");
  });
});
