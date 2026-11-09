import { describe, expect, test } from "bun:test";
import { WORK_RECENT_MS, aboutWork, workItems, workKey } from "./work-items";
import type { WorkspaceView, WsAgent, WsMessage } from "./workspace";

const NOW = 1_794_086_000_000;

const agent = (key: string, extra: Partial<WsAgent> = {}) =>
  ({ key, name: key, role: null, description: null, brief: null, branch: null, state: "running", channels: ["northwind"], last_action_ms: NOW, ...extra }) as unknown as WsAgent;
const message = (id: string, who: string, extra: Partial<WsMessage> = {}) =>
  ({ id, agent: who, from: "agent", text: `text ${id}`, label: null, flags: [], ...extra }) as unknown as WsMessage;
const view = (agents: WsAgent[], cards: unknown[] = []) => ({ agents, cards }) as unknown as WorkspaceView;

describe("workKey", () => {
  test("the ticket, wherever the agent carries it", () => {
    expect(workKey(agent("a", { role: "lane-100001" }))).toBe("100001");
    expect(workKey(agent("a", { description: "AB#100002 expired owner invite" }))).toBe("100002");
    expect(workKey(agent("a", { branch: "fix/no-workspace-page-288834" }))).toBe("288834");
  });

  test("a ticket beats a PR; a PR alone still groups; nothing is nothing", () => {
    expect(workKey(agent("a", { role: "qa-4001", brief: "Test AB#100003 in the PR" }))).toBe("100003");
    expect(workKey(agent("a", { role: "qa-4001" }))).toBe("pr-4001");
    expect(workKey(agent("a", { role: "detail" }))).toBeNull();
  });
});

describe("workItems", () => {
  const lane = agent("a", { role: "lane-100001", brief: "Work on AB#100001 («Invitations can be seen»), P1" });

  test("takes the ticket's own title and joins the agents on one ticket", () => {
    const qa = agent("b", { role: "qa-4001", brief: "Review AB#100001", state: "idle" });
    const [item] = workItems(view([lane, qa]), "northwind", [], NOW);
    expect(item!.title).toBe("Invitations can be seen");
    expect(item!.agents.map((a) => a.key)).toEqual(["a", "b"]);
    expect(item!.status).toBe("working");
  });

  test("the line is the newest one written for a person", () => {
    const ms = [
      message("1", "a", { label: "DONE", text: "Invitations now show." }),
      message("2", "a", { flags: ["unlabeled", "jargon"], text: "rebased on 57442e7" }),
    ];
    expect(workItems(view([lane]), "northwind", ms, NOW)[0]!.line).toBe("Invitations now show.");
  });

  test("what needs them comes first, and old finished work leaves", () => {
    const a = agent("a", { role: "lane-111111" });
    const b = agent("b", { role: "lane-222222", state: "done", last_action_ms: NOW - 1000 });
    const c = agent("c", { role: "lane-333333", state: "done", last_action_ms: NOW - WORK_RECENT_MS - 1 });
    const d = agent("d", { role: "lane-444444", state: "idle" });
    const card = { needs: true, resolved_ms: null, agents: ["d"] };
    const items = workItems(view([a, b, c, d], [card]), "northwind", [], NOW);
    expect(items.map((i) => [i.key, i.status])).toEqual([
      ["444444", "needs"],
      ["111111", "working"],
      ["222222", "finished"],
    ]);
  });

  test("a block is said as a block, below what asks for them", () => {
    const a = agent("a", { role: "lane-111111" });
    const b = agent("b", { role: "lane-222222" });
    const cards = [
      { kind: "blocked", needs: true, resolved_ms: null, agents: ["a"] },
      { kind: "mention", needs: true, resolved_ms: null, agents: ["b"] },
    ];
    expect(workItems(view([a, b], cards), "northwind", [], NOW).map((i) => i.status)).toEqual(["needs", "blocked"]);
  });

  test("a message belongs by its author or by naming the ticket", () => {
    const item = { key: "100001", agents: [lane] };
    expect(aboutWork(message("1", "a"), item)).toBe(true);
    expect(aboutWork(message("2", "z", { text: "AB#100001: ready" }), item)).toBe(true);
    expect(aboutWork(message("3", "z"), item)).toBe(false);
  });
});
