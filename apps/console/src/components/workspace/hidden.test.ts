import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WorkspaceView, WsAgent, WsCard } from "@/lib/workspace";
import { CardItem } from "./activity";

const NOW = Date.UTC(2026, 10, 7, 19, 0);
const agent = (key: string, name: string, hidden: boolean) =>
  ({
    key,
    name,
    hidden,
    presence: "waiting",
    dormant: null,
    lost: false,
    reactions: [],
    activity: { state: "blocked", since_ms: NOW, detail: null, source_ms: NOW },
  }) as unknown as WsAgent;
const blocked = (who: string): WsCard => ({
  id: `blocked:${who}`,
  kind: "blocked",
  channel: "harbard",
  agents: [who],
  message: `m-${who}`,
  checkout: null,
  since_ms: NOW,
  resolved_ms: null,
  needs: true,
});
const view = {
  now_ms: NOW,
  agents: [agent("lima", "Laura", true), agent("rivet", "Owen", false)],
  messages: ["lima", "rivet"].map((k) => ({ id: `m-${k}`, agent: k, text: "Continue?", label: "BLOCKED", ts_ms: NOW, replies: [] })),
  cards: [blocked("lima"), blocked("rivet")],
  operator: "Dana",
  operator_initials: "DR",
} as unknown as WorkspaceView;

describe("Needs you, for a hidden agent", () => {
  test("a hidden agent's BLOCKED card shows, with a small 'hidden'", () => {
    const out = renderToStaticMarkup(h(CardItem, { view, card: view.cards[0]!, now: NOW }));
    expect(out).toContain("Blocked in #harbard");
    expect(out).toMatch(/class="hb-hidden-note"[^>]*>hidden</);
    expect(out).toContain("Laura");
  });

  test("a visible agent's card has no note", () => {
    const out = renderToStaticMarkup(h(CardItem, { view, card: view.cards[1]!, now: NOW }));
    expect(out).not.toContain("hb-hidden-note");
  });
});
