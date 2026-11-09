import { describe, expect, test } from "bun:test";
import { cardHidden, directAgents, hiddenAgents, hideAction } from "./hidden";
import { openCards, sidebarAgents, type WorkspaceView, type WsAgent, type WsCard } from "./workspace";

// Synthetic: four agents of the last hour, Laura hidden and BLOCKED.
const NOW = Date.UTC(2026, 10, 7, 19, 0);
const MIN = 60_000;
const agent = (key: string, name: string, extra: Partial<WsAgent> = {}) =>
  ({
    key,
    name,
    kind: "teammate",
    presence: "working",
    lost: false,
    temporary: false,
    last_action_ms: NOW - MIN,
    last_message: `m-${key}`,
    activity: { state: "editing", since_ms: NOW - MIN, detail: null, source_ms: NOW },
    ...extra,
  }) as unknown as WsAgent;
const card = (id: string, kind: WsCard["kind"], agents: string[]): WsCard => ({
  id,
  kind,
  channel: "harbard",
  agents,
  message: null,
  checkout: null,
  since_ms: NOW - MIN,
  resolved_ms: null,
  needs: true,
});

const AGENTS = [
  agent("rivet", "Owen"),
  agent("lima", "Laura", { hidden: true, activity: { state: "blocked", since_ms: NOW, detail: null, source_ms: NOW } }),
  agent("nut", "Alice", { hidden: true }),
  agent("coil", "Iris"),
];
const view = {
  now_ms: NOW,
  agents: AGENTS,
  channels: [],
  messages: AGENTS.map((a) => ({ id: `m-${a.key}`, agent: a.key, ts_ms: NOW - MIN })),
  cards: [card("blocked:lima", "blocked", ["lima"]), card("mention:nut", "mention", ["nut"]), card("blocked:rivet", "blocked", ["rivet"])],
} as unknown as WorkspaceView;

describe("'Hide': hidden agents leave the lists", () => {
  test("the sidebar's Agents lists only the visible ones", () => {
    const { shown, rest, inactive } = sidebarAgents(view);
    expect([...shown, ...rest, ...inactive].map((a) => a.key).sort()).toEqual(["coil", "rivet"]);
  });

  test("Direct lists only the visible ones, the filter too", () => {
    expect(directAgents(view, "").map((a) => a.key).sort()).toEqual(["coil", "rivet"]);
    expect(directAgents(view, "lima")).toEqual([]);
  });

  test("'Hidden (N)': the hidden ones, by name", () => {
    expect(hiddenAgents(view).map((a) => a.name)).toEqual(["Alice", "Laura"]);
  });

  test("the menu item says Hide or Show, and posts the matching action", () => {
    expect(hideAction(AGENTS[0]!)).toEqual({ label: "Hide", action: "hide-agent" });
    expect(hideAction(AGENTS[1]!)).toEqual({ label: "Show", action: "show-agent" });
  });
});

describe("a hidden agent still needs the operator", () => {
  test("its BLOCKED and its @dana stay in Needs you, marked 'hidden'", () => {
    const open = openCards(view);
    expect(open.map((c) => c.id)).toEqual(["blocked:lima", "mention:nut", "blocked:rivet"]);
    expect(open.map((c) => cardHidden(view, c))).toEqual([true, true, false]);
  });
});
