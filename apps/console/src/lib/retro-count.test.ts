import { afterEach, describe, expect, test } from "bun:test";
import { RULES_CHANGED, fetchRetroCount, postRule } from "./api";
import { RETRO_COUNT_READY_MS, RETRO_COUNT_WAIT_MS, retroCountDelay } from "./retro";

const realFetch = globalThis.fetch;
const g = globalThis as unknown as { window?: unknown };
const realWindow = g.window;
afterEach(() => {
  globalThis.fetch = realFetch;
  g.window = realWindow;
});

function answer(body: unknown): typeof fetch {
  const seen: string[] = [];
  const f = (async (url: string | URL | Request) => {
    seen.push(String(url));
    return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
  }) as unknown as typeof fetch;
  (f as unknown as { seen: string[] }).seen = seen;
  return f;
}

describe("Activity's weekly retro card (AC1)", () => {
  test("reads the cheap count, never the week's retro", async () => {
    const f = answer({ v: 1, candidates: 2, computed_ms: 1, computing: false });
    globalThis.fetch = f;
    const out = await fetchRetroCount();
    expect(out.candidates).toBe(2);
    const seen = (f as unknown as { seen: string[] }).seen;
    expect(seen).toHaveLength(1);
    expect(seen[0]).toEndWith("/retro/count");
  });

  test("asks again soon while the first pass has not landed, then rarely", () => {
    expect(retroCountDelay(null)).toBe(RETRO_COUNT_WAIT_MS);
    expect(retroCountDelay({ v: 1, candidates: null, computed_ms: null, computing: true })).toBe(RETRO_COUNT_WAIT_MS);
    expect(retroCountDelay({ v: 1, candidates: 3, computed_ms: 1, computing: false })).toBe(RETRO_COUNT_READY_MS);
    expect(RETRO_COUNT_WAIT_MS).toBeLessThan(RETRO_COUNT_READY_MS);
  });

  test("an approved rule tells the card to read the count again", async () => {
    globalThis.fetch = answer({ ok: true });
    const fired: string[] = [];
    g.window = { dispatchEvent: (e: Event) => fired.push(e.type) };
    await postRule("rule-approve", { key: "k", text: "t" });
    expect(fired).toEqual([RULES_CHANGED]);
  });
});
