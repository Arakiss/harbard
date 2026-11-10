import { describe, expect, test } from "bun:test";
import {
  deliveriesHint,
  deliveriesLine,
  installedLine,
  pendingCount,
  stateLabel,
  steps,
  timesLine,
  untrackedText,
  type ShipState,
  type WsShipment,
} from "./deliveries";
import { atDate, type WorkspaceView } from "./workspace";

// 2026-11-07 19:00 UTC. Synthetic data only.
const NOW = Date.UTC(2026, 10, 7, 19, 0);
const MIN = 60_000;
const ship = (extra: Partial<WsShipment>): WsShipment => ({
  id: "m1",
  agent: "spring",
  ts_ms: NOW - 60 * MIN,
  text: "The header says who is working",
  channel: "harbard",
  sha: "3df5bf6",
  commit: "3df5bf6000000000000000000000000000000000",
  state: "delivered",
  last_state: "installed",
  pending: true,
  untracked: null,
  review: [],
  ...extra,
});
const view = (shipments: WsShipment[], running: string | null = "59ee93d000000000000000000000000000000000") =>
  ({ now_ms: NOW, shipments, running_commit: running, agents: [] }) as unknown as WorkspaceView;

describe("Deliveries says what it is", () => {
  test("the header's one sentence and the sidebar's tooltip", () => {
    expect(deliveriesLine()).toBe("What the agents have finished and how far each commit has got");
    expect(deliveriesHint()).toBe("Deliveries that have not arrived yet");
  });

  test("the sidebar counts what is not installed yet, never what cannot be followed", () => {
    const v = view([
      ship({ id: "a", state: "delivered" }),
      ship({ id: "b", state: "integrated" }),
      ship({ id: "c", state: "installed", pending: false }),
      ship({ id: "d", untracked: "no_commit", sha: null, commit: null, pending: false }),
    ]);
    expect(pendingCount(v)).toBe(2);
    expect(pendingCount({ ...v, shipments: undefined } as unknown as WorkspaceView)).toBe(0);
  });

  test("the five states by their ids", () => {
    const states: ShipState[] = ["delivered", "in_review", "with_fixes", "integrated", "installed"];
    expect(states.map(stateLabel)).toEqual(["Delivered", "In review", "With fixes", "Integrated", "Installed"]);
  });
});

describe("each delivery's timeline", () => {
  test("delivered, nobody reviewing: only the first step is reached", () => {
    expect(steps(ship({})).map((s) => [s.id, s.reached, s.current])).toEqual([
      ["delivered", true, true],
      ["in_review", false, false],
      ["with_fixes", false, false],
      ["integrated", false, false],
      ["installed", false, false],
    ]);
  });

  test("reviewed with findings, then integrated: every step up to integrated, which is current", () => {
    const s = ship({
      state: "integrated",
      review: [
        { agent: "brush", message: "r1", ts_ms: NOW - 50 * MIN, kind: "reviewing" },
        { agent: "brush", message: "r2", ts_ms: NOW - 40 * MIN, kind: "fixes" },
      ],
    });
    expect(steps(s).map((x) => [x.id, x.reached, x.current])).toEqual([
      ["delivered", true, false],
      ["in_review", true, false],
      ["with_fixes", true, false],
      ["integrated", true, true],
      ["installed", false, false],
    ]);
  });

  test("integrated straight away: review and fixes are skipped, not reached", () => {
    const s = steps(ship({ state: "installed", pending: false }));
    expect(s.filter((x) => x.reached).map((x) => x.id)).toEqual(["delivered", "integrated", "installed"]);
    expect(s.find((x) => x.current)?.id).toBe("installed");
  });

  test("outside serve's repository the timeline ends at integrated", () => {
    expect(steps(ship({ last_state: "integrated" })).map((x) => x.id)).toEqual(["delivered", "in_review", "with_fixes", "integrated"]);
  });

  test("the times, on the operator's clock", () => {
    const s = ship({
      review: [
        { agent: "brush", message: "r1", ts_ms: NOW - 50 * MIN, kind: "reviewing" },
        { agent: "brush", message: "r2", ts_ms: NOW - 40 * MIN, kind: "fixes" },
      ],
    });
    const when = atDate(NOW - 60 * MIN, NOW);
    expect(timesLine(s, NOW)).toBe(`delivered ${when} · review at 18:10 · fixes at 18:20`);
    const clean = ship({ review: [{ agent: "brush", message: "r3", ts_ms: NOW - 30 * MIN, kind: "reviewed" }] });
    expect(timesLine(clean, NOW)).toBe(`delivered ${when} · reviewed without fixes at 18:30`);
  });

  test("why one cannot be followed", () => {
    expect(untrackedText("no_commit")).toBe("Names no commit: there's nothing to integrate.");
    expect(untrackedText("unknown_commit")).toBe("harbard can't find this commit in the repository.");
    expect(untrackedText("no_repository")).toBe("Doesn't belong to any repository.");
    expect(untrackedText(null)).toBeNull();
  });
});

describe("the installed commit", () => {
  test("serve and the console from the same commit", () => {
    expect(installedLine(view([]), "59ee93d")).toEqual({ text: "You have commit 59ee93d installed (serve and console).", mismatch: false });
  });

  test("the console from another commit: said, so the installed state is not trusted blindly", () => {
    expect(installedLine(view([]), "3df5bf6")).toEqual({
      text: "serve runs commit 59ee93d and the console runs 3df5bf6: reload the console after installing.",
      mismatch: true,
    });
  });

  test("an older serve that does not say its commit", () => {
    expect(installedLine(view([], null), "3df5bf6")).toEqual({ text: "serve doesn't say which commit it runs, so there's no way to know what's installed.", mismatch: true });
  });
});
