import { describe, expect, test } from "bun:test";
import { DECOR_IDLE_MS, decorAwake } from "./season-decor";

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
