import { describe, expect, test } from "bun:test";
import { CLOSE_GRACE_MS, OPEN_DELAY_MS, cardModel, createHoverIntent, placeCard, progressText, type Timers } from "./agent-card";
import type { Progress } from "./progress";

function clock() {
  let now = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  let id = 0;
  const t: Timers = {
    set: (fn, ms) => {
      timers.set(++id, { at: now + ms, fn });
      return id;
    },
    clear: (x) => void timers.delete(x as number),
  };
  return {
    t,
    advance(ms: number) {
      const end = now + ms;
      for (;;) {
        const next = [...timers.entries()].sort((a, b) => a[1].at - b[1].at)[0];
        if (!next || next[1].at > end) break;
        timers.delete(next[0]);
        now = next[1].at;
        next[1].fn();
      }
      now = end;
    },
    pending: () => timers.size,
  };
}

describe("opening and closing the card", () => {
  test("it opens 300 ms after the pointer or the focus arrives, not before", () => {
    const c = clock();
    const changes: boolean[] = [];
    const h = createHoverIntent((v) => changes.push(v), c.t);
    expect(OPEN_DELAY_MS).toBe(300);
    h.enter();
    c.advance(299);
    expect(h.isOpen()).toBe(false);
    c.advance(1);
    expect(h.isOpen()).toBe(true);
    expect(changes).toEqual([true]);
  });

  test("passing over it does not open it", () => {
    const c = clock();
    const h = createHoverIntent(() => {}, c.t);
    h.enter();
    c.advance(200);
    h.leave();
    c.advance(1_000);
    expect(h.isOpen()).toBe(false);
    expect(c.pending()).toBe(0);
  });

  test("moving into the card keeps it open; leaving both closes it", () => {
    const c = clock();
    const h = createHoverIntent(() => {}, c.t);
    h.enter();
    c.advance(OPEN_DELAY_MS);
    h.leave();
    c.advance(CLOSE_GRACE_MS - 50);
    h.enterCard();
    c.advance(5_000);
    expect(h.isOpen()).toBe(true);
    h.leaveCard();
    c.advance(CLOSE_GRACE_MS);
    expect(h.isOpen()).toBe(false);
  });

  test("Esc closes it at once, even with the pointer or the focus still on it", () => {
    const c = clock();
    const h = createHoverIntent(() => {}, c.t);
    h.enter();
    c.advance(OPEN_DELAY_MS);
    h.enterCard();
    h.escape();
    expect(h.isOpen()).toBe(false);
    c.advance(5_000);
    expect(h.isOpen()).toBe(false);
    expect(c.pending()).toBe(0);
    // A new arrival opens it again.
    h.enter();
    c.advance(OPEN_DELAY_MS);
    expect(h.isOpen()).toBe(true);
  });

  test("keyboard: focus opens it, moving focus into the card keeps it, blur out of both closes it", () => {
    const c = clock();
    const h = createHoverIntent(() => {}, c.t);
    // Tab reaches the row: the same `enter` the pointer uses.
    h.enter();
    c.advance(OPEN_DELAY_MS);
    expect(h.isOpen()).toBe(true);
    // ↓ moves the focus to the card's first button: enterCard, then the
    // row's blur.
    h.enterCard();
    h.leave();
    c.advance(1_000);
    expect(h.isOpen()).toBe(true);
    // Tab out of the card to elsewhere.
    h.leaveCard();
    c.advance(CLOSE_GRACE_MS);
    expect(h.isOpen()).toBe(false);
  });

  test("the card sits beside its trigger and inside the window, its arrow on the trigger", () => {
    const view = { width: 1512, height: 945 };
    const size = { width: 440, height: 520 };
    expect(placeCard({ left: 80, right: 340, top: 300, bottom: 336 }, size, view)).toEqual({ left: 350, top: 292, side: "right", arrow: 26 });
    // Near the bottom it moves up to fit, and the arrow still points at the row.
    const low = placeCard({ left: 80, right: 340, top: 800, bottom: 836 }, size, view);
    expect(low.top).toBe(945 - 520 - 8);
    expect(low.top + low.arrow).toBe(818);
    // No room on the right: below, or on the left.
    expect(placeCard({ left: 1200, right: 1490, top: 100, bottom: 136 }, size, view).side).toBe("below");
    expect(placeCard({ left: 1200, right: 1490, top: 500, bottom: 536 }, size, view).side).toBe("left");
  });
});

const NOW = 1_800_000_000_000;
const MIN = 60_000;

const agent = {
  name: "harbard-direct",
  label: "general-purpose · aa18a5f8",
  kind: "teammate" as const,
  channels: ["harbard"],
  description: "Build the agent hover card and live view",
  brief: "You are the builder for two harbard features…",
  first_ms: NOW - 42 * MIN,
  lost: false,
  dormant: null,
  presence: "working" as const,
  last_action_ms: NOW - 10_000,
};

describe("what the card says", () => {
  test("name, team, role, presence and the brief", () => {
    const m = cardModel(agent, null, NOW);
    expect(m.brief).toBe("Build the agent hover card and live view");
    expect(m.teams).toEqual(["harbard"]);
    expect(m.role).toBeNull();
    expect(m.presence).toEqual({ text: "working", tone: "green" });
    expect(cardModel({ ...agent, description: null }, null, NOW).brief).toBe("You are the builder for two harbard features…");
    expect(cardModel({ ...agent, label: "codex:codex-rescue · a9f6" }, null, NOW).role).toBe("codex:codex-rescue");
    expect(cardModel({ ...agent, label: "harbard-direct · e3da83f4" }, null, NOW).role).toBeNull();
    expect(cardModel({ ...agent, channels: [] }, null, NOW).teams).toEqual([]);
    expect(cardModel({ ...agent, lost: true }, null, NOW).presence).toEqual({ text: "no data", tone: "grey" });
    expect(cardModel({ ...agent, presence: "waiting" }, null, NOW).presence).toEqual({ text: "waiting", tone: "amber" });
  });

  test("progress shows only real signals: time running, steps, commits, and the last test counts when seen", () => {
    expect(progressText(cardModel(agent, null, NOW).progress)).toBe("Running 42 min");
    const p: Progress = { tool_calls: 386, commits: 7, test: null, steps: [] };
    const long = { ...agent, first_ms: NOW - (2 * 60 + 14) * MIN };
    expect(progressText(cardModel(long, p, NOW).progress)).toBe("Running 2 h 14 min · 386 steps · 7 commits");
    const ok = cardModel(long, { ...p, test: { passed: 877, failed: 0, skipped: 0, ok: true, at_ms: NOW } }, NOW).progress;
    expect(progressText(ok)).toBe("Running 2 h 14 min · 386 steps · 7 commits · Tests: 877 pass, 0 fail");
    expect(ok.at(-1)!.tone).toBe("ok");
    const bad = cardModel(long, { ...p, commits: 1, test: { passed: 41, failed: 2, skipped: 3, ok: false, at_ms: NOW } }, NOW).progress;
    expect(progressText(bad)).toBe("Running 2 h 14 min · 386 steps · 1 commit · Tests: 41 pass, 2 fail, 3 skipped");
    expect(bad.at(-1)!.tone).toBe("fail");
    // Never a percentage or an estimate.
    expect(progressText(bad)).not.toMatch(/%|approx|estim|quedan/);
  });
});
