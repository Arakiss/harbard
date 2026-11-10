import { describe, expect, test } from "bun:test";
import { createBusPush, type BusSource } from "./bus-push";

/** A scripted stream that records what was done to it. */
function scriptedSource() {
  const listeners: Array<() => void> = [];
  let closed = false;
  const source: BusSource = {
    addEventListener: (_type, listener) => listeners.push(listener),
    close: () => {
      closed = true;
    },
  };
  return {
    source,
    append: () => listeners.forEach((listener) => listener()),
    closed: () => closed,
  };
}

const settle = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

describe("bus push", () => {
  test("a burst of appends becomes one look", async () => {
    const stream = scriptedSource();
    let looks = 0;
    const stop = createBusPush({ open: () => stream.source, onChange: () => looks++, settleMs: 20 });
    stream.append();
    stream.append();
    stream.append();
    await settle(40);
    expect(looks).toBe(1);
    stop();
  });

  test("appends after a settled burst are another look", async () => {
    const stream = scriptedSource();
    let looks = 0;
    const stop = createBusPush({ open: () => stream.source, onChange: () => looks++, settleMs: 10 });
    stream.append();
    await settle(25);
    stream.append();
    await settle(25);
    expect(looks).toBe(2);
    stop();
  });

  test("closing drops a pending look and closes the stream", async () => {
    const stream = scriptedSource();
    let looks = 0;
    const stop = createBusPush({ open: () => stream.source, onChange: () => looks++, settleMs: 10 });
    stream.append();
    stop();
    await settle(25);
    expect(looks).toBe(0);
    expect(stream.closed()).toBe(true);
  });

  test("without a stream nothing happens and closing is safe", () => {
    const stop = createBusPush({ open: () => null, onChange: () => {} });
    expect(() => stop()).not.toThrow();
  });
});
