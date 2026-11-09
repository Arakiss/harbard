import { describe, expect, test } from "bun:test";
import { LIVE_BACKOFF_MAX_MS, LIVE_POLL_MS, createLivePoller, duration, liveItems, mergeLive, statusText, type LiveDelta, type LiveStep, type PollerDeps } from "./live-agent";

const T0 = 1_800_000_000_000;

function step(n: number, over: Partial<LiveStep> = {}): LiveStep {
  return { n, rev: n + 1, state: "running", tool: "Bash", detail: "cargo build", started_ms: T0 + n * 1000, ended_ms: null, status: "running", test: null, ...over };
}

function delta(over: Partial<LiveDelta> = {}): LiveDelta {
  return {
    v: 1,
    key: "a1",
    now_ms: T0,
    rev: 10,
    reset: false,
    first_ms: T0,
    last_ms: T0,
    steps: [],
    prose: [],
    progress: { tool_calls: 0, commits: 0, test: null, steps: [] },
    ...over,
  };
}

describe("the live view's state", () => {
  test("a delta updates a step in place and adds new ones; a reset starts over", () => {
    let s = mergeLive(null, delta({ reset: true, rev: 3, steps: [step(0), step(1)], prose: [{ n: 0, rev: 2, ts_ms: T0, text: "Hello." }] }), 0);
    expect(s.steps.map((x) => [x.n, x.status])).toEqual([
      [0, "running"],
      [1, "running"],
    ]);
    s = mergeLive(s, delta({ rev: 5, steps: [step(0, { status: "ok", ended_ms: T0 + 900, rev: 4 }), step(2, { rev: 5 })] }), 0);
    expect(s.steps.map((x) => [x.n, x.status])).toEqual([
      [0, "ok"],
      [1, "running"],
      [2, "running"],
    ]);
    expect(s.prose.length).toBe(1);
    expect(s.rev).toBe(5);
    s = mergeLive(s, delta({ reset: true, rev: 50, steps: [step(7)] }), 0);
    expect(s.steps.map((x) => x.n)).toEqual([7]);
    expect(s.prose).toEqual([]);
  });

  test("prose and calls interleave in time order, a reply before the calls it announces", () => {
    const items = liveItems({
      steps: [step(0, { started_ms: T0 + 2000 }), step(1, { started_ms: T0 + 5000 })],
      prose: [
        { n: 0, rev: 1, ts_ms: T0 + 2000, text: "Compilo." },
        { n: 1, rev: 1, ts_ms: T0 + 9000, text: "Ready." },
      ],
    });
    expect(items.map((i) => (i.kind === "step" ? `s${i.step.n}` : `p${i.prose.n}`))).toEqual(["p0", "s0", "s1", "p1"]);
  });
});

describe("words", () => {
  test("durations and status pills, as shown", () => {
    expect(duration(200)).toBe("0.2 s");
    expect(duration(400)).toBe("0.4 s");
    expect(duration(12_300)).toBe("12 s");
    expect(duration(48_000)).toBe("48 s");
    expect(duration(185_000)).toBe("3 min 5 s");
    expect((["running", "ok", "error", "stopped"] as const).map(statusText)).toEqual(["running", "ok", "error", "no response"]);
  });
});

/** A fake clock and page: timers fire when the test advances time. */
function harness(serve: (after: number | null) => LiveDelta) {
  let now = 0;
  let visible = true;
  const timers = new Map<number, { at: number; fn: () => void }>();
  let id = 0;
  const listeners = new Set<() => void>();
  const asked: { at: number; after: number | null }[] = [];
  const got: LiveDelta[] = [];
  const deps: PollerDeps = {
    fetch: async (after) => {
      asked.push({ at: now, after });
      return serve(after);
    },
    onDelta: (d) => got.push(d),
    visible: () => visible,
    onVisibility: (fn) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    setTimer: (fn, ms) => {
      timers.set(++id, { at: now + ms, fn });
      return id;
    },
    clearTimer: (t) => void timers.delete(t as number),
  };
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return {
    deps,
    asked,
    got,
    timers,
    async advance(ms: number) {
      const end = now + ms;
      await flush();
      await flush();
      for (;;) {
        const next = [...timers.entries()].sort((a, b) => a[1].at - b[1].at)[0];
        if (!next || next[1].at > end) break;
        timers.delete(next[0]);
        now = next[1].at;
        next[1].fn();
        await flush();
        await flush();
      }
      now = end;
    },
    async setVisible(v: boolean) {
      visible = v;
      for (const l of listeners) l();
      await flush();
      await flush();
    },
    get now() {
      return now;
    },
  };
}

describe("the live poller", () => {
  test("a change on serve reaches the view within 2 s, and each ask carries the cursor", async () => {
    let rev = 1;
    let changedAt: number | null = null;
    const h = harness((after) => delta({ rev, reset: after === null, steps: after !== null && after < rev ? [step(rev)] : [] }));
    const p = createLivePoller(h.deps);
    p.start();
    await h.advance(0);
    expect(h.asked[0]).toEqual({ at: 0, after: null });
    await h.advance(3_250);
    // serve changes between two asks.
    rev = 2;
    changedAt = h.now;
    await h.advance(2_000);
    const seen = h.got.find((d) => d.steps.some((s) => s.n === 2));
    expect(seen).toBeDefined();
    const at = h.asked[h.got.indexOf(seen!)]!.at;
    expect(at - changedAt).toBeLessThanOrEqual(LIVE_POLL_MS);
    expect(LIVE_POLL_MS).toBeLessThan(2_000);
    expect(h.asked.slice(1).every((a) => a.after !== null)).toBe(true);
    p.stop();
  });

  test("after failures it asks less and less often, and at the usual pace once serve answers", async () => {
    let down = true;
    const h = harness(() => {
      if (down) throw new Error("GET /live/a1 returned 404");
      return delta();
    });
    const p = createLivePoller(h.deps);
    p.start();
    await h.advance(40_000);
    const gaps = h.asked.slice(1).map((a, i) => a.at - h.asked[i]!.at);
    expect(gaps.slice(0, 4)).toEqual([2_000, 4_000, 8_000, 16_000]);
    expect(Math.max(...gaps)).toBeLessThanOrEqual(LIVE_BACKOFF_MAX_MS);
    down = false;
    await h.advance(LIVE_BACKOFF_MAX_MS);
    const n = h.asked.length;
    await h.advance(3_000);
    expect(h.asked.length - n).toBe(3);
    p.stop();
  });

  test("hidden, it asks nothing and holds no timer; visible again, it asks at once", async () => {
    const h = harness(() => delta());
    const p = createLivePoller(h.deps);
    p.start();
    await h.advance(2_500);
    const before = h.asked.length;
    expect(before).toBe(3);
    await h.setVisible(false);
    expect(p.paused()).toBe(true);
    expect(h.timers.size).toBe(0);
    await h.advance(60_000);
    expect(h.asked.length).toBe(before);
    await h.setVisible(true);
    expect(h.asked.length).toBe(before + 1);
    expect(p.paused()).toBe(false);
    p.stop();
    await h.advance(10_000);
    expect(h.asked.length).toBe(before + 1);
    expect(h.timers.size).toBe(0);
  });
});
