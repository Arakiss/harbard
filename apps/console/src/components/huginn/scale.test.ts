import { describe, expect, test } from "bun:test";
import { formatNumber, niceDomain, ringSegment, stack, thinnedLabels } from "./scale";

describe("chart domains", () => {
  test("bars always include zero", () => {
    const domain = niceDomain([40, 60], { includeZero: true });
    expect(domain.min).toBe(0);
    expect(domain.max).toBeGreaterThanOrEqual(60);
  });

  test("lines may start away from zero", () => {
    expect(niceDomain([40, 60], { includeZero: false }).min).toBeGreaterThan(0);
  });

  test("ticks carry no floating point error", () => {
    for (const tick of niceDomain([0.1, 0.7], { includeZero: true }).ticks) expect(String(tick).length).toBeLessThan(6);
  });

  test("a series of equal values still has a span", () => {
    const domain = niceDomain([5, 5], { includeZero: false });
    expect(domain.max).toBeGreaterThan(domain.min);
  });

  test("only missing values still give a usable domain", () => {
    const domain = niceDomain([null, null], { includeZero: true });
    expect(domain.max).toBeGreaterThan(domain.min);
  });
});

describe("stacking", () => {
  test("positives pile up and negatives pile down from zero", () => {
    expect(stack([2, -1, 3, null, -2])).toEqual([
      { start: 0, end: 2 },
      { start: 0, end: -1 },
      { start: 2, end: 5 },
      { start: 5, end: 5 },
      { start: -1, end: -3 },
    ]);
  });
});

describe("labels", () => {
  test("every label shows when they fit", () => {
    expect(thinnedLabels(4, 400, 64).size).toBe(4);
  });

  test("labels thin out evenly and keep the first when they do not fit", () => {
    const kept = thinnedLabels(100, 320, 64);
    expect(kept.has(0)).toBe(true);
    expect(kept.size).toBeLessThanOrEqual(5);
  });
});

describe("a ring segment", () => {
  test("a whole ring still draws, since a closed arc would have no length", () => {
    expect(ringSegment(50, 50, 40, 20, 0, Math.PI * 2)).toContain(" 0 1 1 ");
  });
});

describe("numbers", () => {
  test("use the language's separators and the unit", () => {
    expect(formatNumber(1234.5, "ms")).toBe("1,234.5 ms");
  });

  test("a missing value says so instead of showing zero", () => {
    expect(formatNumber(null)).toBe("no data");
  });
});
