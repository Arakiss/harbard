import { expect, test } from "bun:test";
import { DOCK_EVENTS, DOCK_STATES, KEEPALIVE_MS, desktopStatusDriver, desktopStatusSink, dockBadge, setDesktopStatus, validDesktopStatus } from "./desktop-status";

function host() {
  const calls: [string, Record<string, unknown> | undefined][] = [];
  const invoke = async (command: string, args?: Record<string, unknown>) => {
    calls.push([command, args]);
  };
  return { calls, h: { __TAURI__: { core: { invoke } } } };
}

test("in a browser tab the wrapper does nothing", async () => {
  expect(await desktopStatusSink(undefined)({ state: "working", badge: 2 })).toBe("browser");
  expect(await desktopStatusSink({})({ state: "working", badge: 2 })).toBe("browser");
  // bun test has no window: the shared sink is the browser one.
  expect(await setDesktopStatus({ state: "idle", badge: 0 })).toBe("browser");
});

test("the desktop shell gets desk_set_status with only the fields given", async () => {
  const { calls, h } = host();
  const sink = desktopStatusSink(h);
  expect(await sink({ state: "needs", badge: 3, needs: 1 })).toBe("sent");
  expect(await sink({ state: "working", badge: 0, event: "message" })).toBe("sent");
  expect(calls).toEqual([
    ["desk_set_status", { state: "needs", badge: 3, needs: 1 }],
    ["desk_set_status", { state: "working", badge: 0, event: "message" }],
  ]);
});

test("identical consecutive calls cross IPC once", async () => {
  const { calls, h } = host();
  const sink = desktopStatusSink(h);
  expect(await sink({ state: "blocked", badge: 4, needs: 2 })).toBe("sent");
  expect(await sink({ state: "blocked", badge: 4, needs: 2 })).toBe("unchanged");
  expect(await sink({ state: "blocked", badge: 5, needs: 2 })).toBe("sent");
  expect(await sink({ state: "blocked", badge: 4, needs: 2 })).toBe("sent");
  expect(calls.length).toBe(3);
});

test("the internals bridge works too, and a failure is retried on the next call", async () => {
  const calls: unknown[] = [];
  let fail = true;
  const h = { __TAURI_INTERNALS__: { invoke: async (c: string, a?: Record<string, unknown>) => { calls.push([c, a]); if (fail) throw new Error("x"); } } };
  const sink = desktopStatusSink(h);
  expect(await sink({ state: "idle", badge: 0 })).toBe("failed");
  fail = false;
  expect(await sink({ state: "idle", badge: 0 })).toBe("sent");
  expect(calls.length).toBe(2);
});

test("unknown states, events, fields and non-integer counts never reach native code", async () => {
  const { calls, h } = host();
  const sink = desktopStatusSink(h);
  const bad: unknown[] = [
    { state: "asleep", badge: 0 },
    { state: "idle", badge: 1.5 },
    { state: "idle", badge: "3" },
    { state: "idle", badge: 0, event: "fiesta" },
    { state: "idle", badge: 0, colour: "red" },
    { state: "idle", badge: 0, needs: Number.NaN },
    null,
  ];
  for (const status of bad) {
    expect(validDesktopStatus(status)).toBe(false);
    await expect(sink(status as never)).rejects.toThrow("Invalid desktop status");
  }
  expect(calls).toEqual([]);
  for (const state of DOCK_STATES) expect(validDesktopStatus({ state, badge: 0 })).toBe(true);
  for (const event of DOCK_EVENTS) expect(validDesktopStatus({ state: "idle", badge: 0, event })).toBe(true);
});

test("the badge counts Needs you plus unread Direct, capped like the native side", () => {
  expect(dockBadge(0)).toBe(0);
  expect(dockBadge(2)).toBe(2);
  expect(dockBadge(-1)).toBe(0);
  expect(dockBadge(Number.NaN)).toBe(0);
  expect(dockBadge(12000)).toBe(9999);
});

test("a second, separate one-shot event with an unchanged state still reaches the Dock", async () => {
  const { calls, h } = host();
  const sink = desktopStatusSink(h);
  expect(await sink({ state: "working", badge: 1, event: "message" })).toBe("sent");
  expect(await sink({ state: "working", badge: 1, event: "message" })).toBe("sent");
  expect(await sink({ state: "working", badge: 1, event: "verified" })).toBe("sent");
  // The state the events left behind is known: a plain repeat is a no-op.
  expect(await sink({ state: "working", badge: 1 })).toBe("unchanged");
  expect(calls.length).toBe(3);
});

test("an identical call made while one is still in flight is not lost when that one fails", async () => {
  let n = 0;
  let reject: (() => void) | undefined;
  const h = {
    __TAURI__: {
      core: {
        invoke: (_c: string) => {
          n += 1;
          return n === 1 ? new Promise<void>((_, rej) => { reject = () => rej(new Error("x")); }) : Promise.resolve();
        },
      },
    },
  };
  const sink = desktopStatusSink(h);
  const first = sink({ state: "needs", badge: 2, needs: 2 });
  const second = await sink({ state: "needs", badge: 2, needs: 2 });
  reject!();
  expect(await first).toBe("failed");
  expect(second).toBe("sent");
  expect(await sink({ state: "needs", badge: 2, needs: 2 })).toBe("unchanged");
});

// Heartbeat: the native side shows the outage ring after 60 s without a
// report, so a live console repeats its last state well within that.
test("the desktop sink repeats its last state on a keepalive, without the event", async () => {
  const { calls, h } = host();
  const timers: { fn: () => void; ms: number }[] = [];
  const sink = desktopStatusSink(h, { keepaliveMs: 20_000, schedule: (fn, ms) => { timers.push({ fn, ms }); return () => {}; } });
  expect(await sink({ state: "working", badge: 1, event: "message" })).toBe("sent");
  expect(timers.at(-1)?.ms).toBe(20_000);
  timers.at(-1)!.fn();
  await Promise.resolve();
  expect(calls.at(-1)).toEqual(["desk_set_status", { state: "working", badge: 1 }]);
  expect(KEEPALIVE_MS).toBeLessThanOrEqual(30_000);
});

test("a browser tab arms no keepalive", async () => {
  let armed = 0;
  const sink = desktopStatusSink(undefined, { schedule: () => { armed += 1; return () => {}; } });
  expect(await sink({ state: "idle", badge: 0 })).toBe("browser");
  expect(armed).toBe(0);
});

// The call site: what the living logo shows becomes one desk_set_status.
test("a logo state change produces the right desk_set_status call, with each new one-shot once", async () => {
  const { calls, h } = host();
  const drive = desktopStatusDriver(desktopStatusSink(h, { schedule: () => () => {} }));
  await drive({ state: "connecting", needs: 0, directs: 0, once: null, onceSeq: 0 });
  await drive({ state: "needs", needs: 2, directs: 1, once: null, onceSeq: 0 });
  await drive({ state: "needs", needs: 2, directs: 1, once: null, onceSeq: 0 });
  await drive({ state: "working", needs: 0, directs: 3, once: "message", onceSeq: 1 });
  // The same one-shot still on screen at the next render is not a new one.
  await drive({ state: "working", needs: 0, directs: 3, once: "message", onceSeq: 1 });
  await drive({ state: "working", needs: 0, directs: 3, once: "verified", onceSeq: 2 });
  expect(calls).toEqual([
    ["desk_set_status", { state: "connecting", badge: 0, needs: 0 }],
    ["desk_set_status", { state: "needs", badge: 2, needs: 2 }],
    ["desk_set_status", { state: "working", badge: 0, needs: 0, event: "message" }],
    ["desk_set_status", { state: "working", badge: 0, needs: 0, event: "verified" }],
  ]);
});
