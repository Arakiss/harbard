import { describe, expect, spyOn, test } from "bun:test";
import { operatorTimeZone } from "./time-zone";

describe("the operator's time zone", () => {
  // Fails if the preload stops running: every test that asserts a wall-clock
  // time would then depend on the machine it runs on.
  test("the test run is pinned, whatever the machine says", () => {
    expect(operatorTimeZone()).toBe("UTC");
    expect(process.env.TZ).toBe("UTC");
  });

  test("a runtime that cannot say falls back to UTC", () => {
    const failing = spyOn(Intl, "DateTimeFormat").mockImplementation(() => {
      throw new Error("no ICU");
    });
    try {
      expect(operatorTimeZone()).toBe("UTC");
    } finally {
      failing.mockRestore();
    }
  });
});
