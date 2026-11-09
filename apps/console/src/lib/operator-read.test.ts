import { expect, test } from "bun:test";
import { readCoversEvent } from "./operator-read";

const first = "01K0000000ZZZZZZZZZZZZZZZZ";
const second = "01K00000000000000000000000";
const events = [{ id: first }, { id: second }];

test("confirmed reads follow append order rather than random same-time suffixes", () => {
  expect(readCoversEvent(events, first, undefined)).toBe(false);
  expect(readCoversEvent(events, first, first)).toBe(true);
  expect(readCoversEvent(events, first, second)).toBe(true);
  expect(readCoversEvent(events, second, first)).toBe(false);
});

test("an absent cursor needs strictly later time, never an unknown same-time suffix", () => {
  expect(readCoversEvent(events, first, "01K00000010000000000000000")).toBe(true);
  expect(readCoversEvent(events, first, "01K0000000YYYYYYYYYYYYYYYY")).toBe(false);
  expect(readCoversEvent(events, first, "unresolved")).toBe(false);
});
