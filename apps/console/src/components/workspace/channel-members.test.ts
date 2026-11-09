import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WorkspaceView, WsAgent, WsChannel } from "@/lib/workspace";
import { ChannelMembers, MembersList } from "./channel-members";

const NOW = Date.UTC(2026, 10, 7, 19, 0);
const MIN = 60_000;
const agent = (key: string, name: string, presence: WsAgent["presence"], last: number, extra: Partial<WsAgent> = {}) =>
  ({
    key,
    name,
    presence,
    dormant: null,
    last_action_ms: NOW - last * MIN,
    lost: false,
    label: `general-purpose · ${key}`,
    reactions: [],
    activity: { state: presence === "working" ? "editing" : "finished", since_ms: NOW - last * MIN, detail: null, source_ms: NOW },
    ...extra,
  }) as unknown as WsAgent;

const AGENTS = [
  agent("brush", "Hugo", "finished", 90, { role_line: "revisor · harbard" }),
  agent("rivet", "Owen", "working", 5, { role_line: "constructor · harbard" }),
  agent("nut", "Alice", "working", 1),
];
const channel = { id: "harbard", repo: "/r/harbard", checkouts: [], members: ["brush", "rivet", "nut"], last_ms: 0, warnings: [] } as WsChannel;
const view = { now_ms: NOW, agents: AGENTS, channels: [channel] } as unknown as WorkspaceView;

describe("the channel header's members button", () => {
  test("its tooltip and aria-label say what the numbers are; the button shows the total on one line", () => {
    const out = renderToStaticMarkup(h(ChannelMembers, { view, channel }));
    expect(out).toContain('aria-label="3 agents have worked in #harbard · 2 now"');
    expect(out).toContain('title="3 agents have worked in #harbard · 2 now"');
    expect(out).toContain('aria-haspopup="dialog"');
    // "1 now of 45" wrapped into four pieces; who works now is in the header line.
    expect(out).not.toContain("hb-members-now");
    expect(out).toMatch(/class="hb-members-total"[^>]*>3</);
  });

  test("the popover lists working first, then the rest, each linked to its conversation with role and state", () => {
    const out = renderToStaticMarkup(h(MembersList, { view, channel }));
    const order = [...out.matchAll(/href="\/direct\/([^"]+)"/g)].map((m) => m[1]);
    expect(order).toEqual(["nut", "rivet", "brush"]);
    expect(out).toContain("Working now (2)");
    expect(out).toContain("Earlier (1)");
    expect(out).toContain("constructor · harbard");
    expect(out).toContain("working · 1 min ago");
    expect(out).toContain("finished · 2 h ago");
  });
});
