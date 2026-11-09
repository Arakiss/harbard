import { describe, expect, test } from "bun:test";
import { statusText } from "./activity";
import { isInactive, sidebarAgents, type WsAgent, type WorkspaceView } from "./workspace";

// Agents the operator has stopped, and the collapsed "Inactive" group.
const NOW = Date.UTC(2026, 10, 7, 18, 0);
const MIN = 60_000;
const face = (state: string, detail: string | null = null) => ({ state, since_ms: NOW, detail, source_ms: NOW }) as unknown as WsAgent["activity"];
const agent = (key: string, state: string, quietMin: number, extra: Partial<WsAgent> = {}): WsAgent =>
  ({ key, name: key, temporary: false, lost: false, closed: false, last_action_ms: NOW - quietMin * MIN, activity: face(state), ...extra }) as unknown as WsAgent;

describe("an agent its lead stopped", () => {
  test("reads Closed, never No task right now", () => {
    expect(statusText(face("finished", "closed"))).toBe("Closed");
    expect(statusText(face("normal"))).toBe("No task right now");
  });

  test("moves into Inactive after ten minutes", () => {
    const closed = (m: number) => agent("c", "finished", m, { closed: true, activity: face("finished", "closed") });
    expect(isInactive(closed(9), NOW)).toBe(false);
    expect(isInactive(closed(11), NOW)).toBe(true);
  });
});

describe("the Inactive group", () => {
  test("idle, finished or without data for thirty minutes; busy agents always on top", () => {
    expect(isInactive(agent("idle", "normal", 29), NOW)).toBe(false);
    expect(isInactive(agent("idle", "normal", 31), NOW)).toBe(true);
    expect(isInactive(agent("done", "finished", 31), NOW)).toBe(true);
    expect(isInactive(agent("lost", "normal", 31, { lost: true }), NOW)).toBe(true);
    for (const busy of ["thinking", "running", "waiting", "blocked"]) expect(isInactive(agent(busy, busy, 120), NOW)).toBe(false);
  });

  test("the sidebar splits them, working agents first", () => {
    const view = {
      now_ms: NOW,
      agents: [agent("w", "running", 1), agent("b", "blocked", 90), agent("i", "normal", 45), agent("c", "finished", 15, { closed: true, activity: face("finished", "closed") })],
    } as unknown as WorkspaceView;
    const s = sidebarAgents(view);
    expect(s.shown.map((a) => a.key)).toEqual(["w", "b"]);
    expect(s.inactive.map((a) => a.key)).toEqual(["c", "i"]);
  });
});
