import { describe, expect, test } from "bun:test";
import { awayModel, awayTitle } from "./away";
import type { DigestItem } from "./profile";
import type { WsAgent, WsCard, WsDelivery, WsMessage, WorkspaceView } from "./workspace";

const MIN = 60_000;
const NOW = Date.UTC(2026, 10, 7, 17, 0);
const SINCE = NOW - 72 * MIN;

const agent = (key: string, name: string, extra: Partial<WsAgent> = {}) =>
  ({ key, name, presence: "working", lost: false, lead_session: `lead-${key}`, dormant: null, ...extra }) as unknown as WsAgent;
const card = (id: string, kind: WsCard["kind"], extra: Partial<WsCard> = {}): WsCard => ({
  id,
  kind,
  channel: "harbard",
  agents: ["robots"],
  message: null,
  checkout: null,
  since_ms: NOW - 10 * MIN,
  resolved_ms: null,
  needs: true,
  ...extra,
});
const delivery = (id: string, verdict: WsDelivery["verdict"], decided: number | null): WsDelivery => ({
  id,
  channel: "harbard",
  sha: "abc1234",
  branch: `branch-${id}`,
  verdict,
  created_ms: decided ?? NOW,
  decided_ms: decided,
  session: "s",
});
const done = (id: string, ts: number, text: string) =>
  ({ id, agent: "strip", label: "DONE", ts_ms: ts, text, from: "agent" }) as unknown as WsMessage;
const held = (id: string, kind: string, text: string, session = "lead-robots"): DigestItem => ({
  id,
  session,
  harness: "claude-code",
  kind,
  text,
  ts_ms: NOW - 20 * MIN,
});

function view(extra: Partial<WorkspaceView>): WorkspaceView {
  return {
    agents: [agent("robots", "robots"), agent("strip", "strip"), agent("keychain", "keychain"), agent("old", "old", { presence: "finished" }), agent("gone", "gone", { lost: true })],
    cards: [],
    deliveries: [],
    messages: [],
    ...extra,
  } as unknown as WorkspaceView;
}

describe("While you were away", () => {
  test("says how long they were away", () => {
    expect(awayTitle(SINCE, NOW)).toBe("While you were away · 1 h 12 min");
    expect(awayTitle(NOW - 40 * MIN, NOW)).toBe("While you were away · 40 min");
    expect(awayTitle(NOW - 26 * 60 * MIN, NOW)).toBe("While you were away · 26 h");
    expect(awayTitle(NOW - 20_000, NOW)).toBe("While you were away", "under a minute: no «0 min»");
  });

  test("puts what waits for them first, the most urgent on top", () => {
    const m = awayModel({
      view: view({
        cards: [
          card("d", "dormant"),
          card("b", "blocked"),
          card("a", "approval"),
          card("x", "blocked", { needs: false }),
          card("r", "blocked", { resolved_ms: NOW - MIN }),
        ],
      }),
      held: [held("n1", "fyi", "Themes installed"), held("n2", "ask", "Large or small?"), held("n3", "handoff", "Review PR 12")],
      since: SINCE,
      now: NOW,
    });
    expect(m.waiting.map((w) => w.id)).toEqual(["card:b", "card:a", "note:n3", "note:n2", "card:d"]);
    expect(m.waiting.find((w) => w.id === "note:n2")?.who).toBe("robots");
  });

  test("lists what finished while they were away, and only that", () => {
    const m = awayModel({
      view: view({
        deliveries: [delivery("1", "landed", NOW - 30 * MIN), delivery("2", "landed", SINCE - MIN), delivery("3", "conflict", NOW - 5 * MIN)],
        messages: [done("m1", NOW - 50 * MIN, "Team view ready.\nDetail"), done("m0", SINCE - 5 * MIN, "Before I left")],
      }),
      held: [held("n1", "fyi", "Themes installed · checked")],
      since: SINCE,
      now: NOW,
    });
    expect(m.done.map((d) => d.text)).toEqual(["Themes installed · checked", "Team view ready.", "branch-1 · verified"]);
  });

  test("shows who is still at work, never the finished or the lost", () => {
    const m = awayModel({ view: view({}), held: [], since: SINCE, now: NOW });
    expect(m.running.map((a) => a.key)).toEqual(["robots", "strip", "keychain"]);
  });

  test("is empty, not invented, without a view or a digest", () => {
    const m = awayModel({ view: null, held: [], since: SINCE, now: NOW });
    expect([m.waiting.length, m.done.length, m.running.length]).toEqual([0, 0, 0]);
  });
});
