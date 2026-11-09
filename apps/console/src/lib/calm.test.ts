import { describe, expect, test } from "bun:test";
import { calmModel, calmOn } from "./calm";
import { hhmm } from "./workspace";
import type { WsAgent, WsCard, WsMessage, WorkspaceView } from "./workspace";

const NOW = Date.UTC(2026, 10, 7, 17, 0);
const card = (id: string, kind: WsCard["kind"], extra: Partial<WsCard> = {}): WsCard => ({
  id,
  kind,
  channel: "harbard",
  agents: ["robots"],
  message: `m-${id}`,
  checkout: null,
  since_ms: NOW,
  resolved_ms: null,
  needs: true,
  ...extra,
});
const msg = (id: string, text: string) => ({ id, agent: "robots", text, ts_ms: NOW }) as unknown as WsMessage;
const view = (cards: WsCard[], working = 5): WorkspaceView =>
  ({
    agents: [
      ...Array.from({ length: working }, (_, i) => ({ key: `a${i}`, name: `a${i}`, presence: "working", lost: false })),
      { key: "robots", name: "robots", presence: "waiting", lost: false },
    ] as unknown as WsAgent[],
    cards,
    messages: [msg("m-b", "Install the desktop app now?\nIt takes 10 seconds."), msg("m-a", "Approve the change?")],
    deliveries: [],
  }) as unknown as WorkspaceView;

describe("calm mode", () => {
  test("is on in Swamped unless they left it for this Swamped", () => {
    expect(calmOn("swamped", 1000, null)).toBe(true);
    expect(calmOn("swamped", 1000, 1000)).toBe(false);
    expect(calmOn("swamped", 2000, 1000)).toBe(true, "a new Saturado is calm again");
    for (const s of ["available", "focused", "away", "reviewing", "done"] as const) expect(calmOn(s, 1000, null)).toBe(false);
  });

  test("shows only the most urgent item, its first line and the rest apart", () => {
    const m = calmModel(view([card("d", "dormant"), card("a", "approval"), card("b", "blocked")]), new Set(), NOW);
    expect(m.top?.id).toBe("card:b");
    expect(m.top?.title).toBe("Install the desktop app now?");
    expect(m.top?.detail).toBe("It takes 10 seconds.");
    expect(m.waiting).toBe(3);
    expect(m.working).toBe(5);
  });

  test("Later moves on to the next, and nothing left says so", () => {
    const v = view([card("a", "approval"), card("b", "blocked")]);
    expect(calmModel(v, new Set(["card:b"]), NOW).top?.id).toBe("card:a");
    const none = calmModel(v, new Set(["card:a", "card:b"]), NOW);
    expect(none.top).toBeNull();
    expect(calmModel(null, new Set(), NOW).top).toBeNull();
  });

  test("'No reply' is the operator's own message, quoted, under 'Nobody has answered your message' (AC5)", () => {
    const v = view([card("u", "unanswered", { message: "m-u" })]);
    v.messages.push({ id: "m-u", agent: "robots", from: "operator", text: "How is the export going?", ts_ms: NOW - 20 * 60_000 } as unknown as WsMessage);
    const top = calmModel(v, new Set(), NOW).top!;
    expect(top.title).toBe("Nobody has answered your message");
    expect(top.own).toBe(true);
    expect(top.quote).toBe("How is the export going?");
    expect(top.detail).toBe(`You wrote to robots at ${hhmm(NOW - 20 * 60_000)}.`);
    expect(top.title).not.toContain("export");
  });

  test("an agent's question is not quoted as theirs", () => {
    const top = calmModel(view([card("b", "blocked")]), new Set(), NOW).top!;
    expect(top.own).toBe(false);
    expect(top.quote).toBeNull();
  });
});
