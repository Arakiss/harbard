import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { channelWhy, type WorkspaceView, type WsAgent, type WsChannel } from "@/lib/workspace";
import { ChannelWhy } from "./home";

// A repository only read with git: two agents read it, nobody works there;
// #harbard has three agents working today.
const NOW = Date.UTC(2026, 10, 7, 19, 0);
const agent = (key: string, name: string) => ({ key, name }) as unknown as WsAgent;
const AGENTS = [agent("protocol", "Owen"), agent("brace", "Henry"), agent("view", "Julia"), agent("review", "Hugo")];
const channel = (c: Partial<WsChannel>) =>
  ({ id: "x", repo: "/r/x", checkouts: [], members: [], last_ms: 0, warnings: [], ...c }) as WsChannel;
const view = { now_ms: NOW, agents: AGENTS, channels: [], messages: [], cards: [] } as unknown as WorkspaceView;

const READ_ONLY = channel({ id: "acme-config", members: ["brace", "protocol"], recent: [], active_ms: 0, quiet: true });
const HARBARD = channel({ id: "harbard", members: ["protocol", "view", "review"], recent: ["protocol", "view", "review", "session:gone"], active_ms: NOW - 60_000 });

describe("the channel header says why the channel is here", () => {
  test("three agents working today: '3 sessions in the last 24 h', each a link to its conversation", () => {
    const why = channelWhy(view, HARBARD);
    expect(why.line).toBe("3 sessions in the last 24 h");
    expect(why.agents.map((a) => a.key)).toEqual(["protocol", "view", "review"]);
    const out = renderToStaticMarkup(h(ChannelWhy, { view, channel: HARBARD }));
    expect(out).toContain("<summary>3 sessions in the last 24 h<");
    expect(out).toContain('href="/direct/protocol"');
    expect(out).toContain(">Owen</a>");
    expect(out).toContain('href="/direct/review"');
  });

  test("one: '1 session in the last 24 h'", () => {
    expect(channelWhy(view, channel({ members: ["view"], recent: ["view"] })).line).toBe("1 session in the last 24 h");
  });

  test("#acme-config: nobody wrote there, two agents only read it with git, named and linked", () => {
    const why = channelWhy(view, READ_ONLY);
    expect(why.line).toBe("Nobody has written here: 2 agents read it with git");
    const out = renderToStaticMarkup(h(ChannelWhy, { view, channel: READ_ONLY }));
    expect(out).toContain('href="/direct/brace"');
    expect(out).toContain('href="/direct/protocol"');
  });

  test("quiet now, with older activity: when it last had any", () => {
    const old = channel({ members: ["view"], recent: [], active_ms: Date.UTC(2026, 10, 5, 17, 30) });
    expect(channelWhy(view, old).line).toBe("Nobody's working here now · last activity on 5 Nov at 17:30");
  });
});
