import { describe, expect, test } from "bun:test";
import { DECOR_IDLE_MS, FLURRY_EVERY_MS, FLURRY_MS, decorAwake, inFlurry } from "./season-decor";

describe("the season's decorations", () => {
  test("move while the window is seen, focused and used in the last two minutes", () => {
    expect(decorAwake({ visible: true, focused: true, sinceInputMs: 0 })).toBe(true);
    expect(decorAwake({ visible: true, focused: true, sinceInputMs: DECOR_IDLE_MS - 1 })).toBe(true);
  });

  test("settle after two minutes without input, behind another window or out of sight", () => {
    expect(decorAwake({ visible: true, focused: true, sinceInputMs: DECOR_IDLE_MS })).toBe(false);
    expect(decorAwake({ visible: true, focused: false, sinceInputMs: 0 })).toBe(false);
    expect(decorAwake({ visible: false, focused: true, sinceInputMs: 0 })).toBe(false);
  });
});

describe("the Strip's snow", () => {
  test("falls for the first forty seconds of every five minutes", () => {
    const start = 12 * FLURRY_EVERY_MS;
    expect(inFlurry(start)).toBe(true);
    expect(inFlurry(start + FLURRY_MS - 1)).toBe(true);
  });

  test("rests for the rest of each five minutes", () => {
    const start = 12 * FLURRY_EVERY_MS;
    expect(inFlurry(start + FLURRY_MS)).toBe(false);
    expect(inFlurry(start + FLURRY_EVERY_MS - 1)).toBe(false);
  });
});
