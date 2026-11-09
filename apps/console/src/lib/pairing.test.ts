import { describe, expect, test } from "bun:test";
import { confirmMatches, describeAction, deviceVersion, expiresLine, refusalText, statusLine, type Device } from "./pairing";
import { GET_ROOTS, POST_ROOTS } from "./upstream";

const NOW = 1_800_000_000_000;
const MIN = 60_000;

function device(over: Partial<Device> = {}): Device {
  return {
    id: "d1",
    name: "BOREALIS",
    host: "borealis",
    os: "linux",
    hub: false,
    access: "workshop",
    created_ms: NOW - 60 * MIN,
    joined_ms: NOW - 60 * MIN,
    last_seen_ms: NOW - 3 * MIN,
    revoked_ms: null,
    credential: true,
    recent: [],
    reads: 0,
    writes: 0,
    ...over,
  };
}

describe("Pairing helpers", () => {
  test("Operator needs the device's name, typed as it is shown", () => {
    expect(confirmMatches("borealis", "BOREALIS")).toBe(true);
    expect(confirmMatches("  BOREALIS ", "BOREALIS")).toBe(true);
    expect(confirmMatches("BOREALI", "BOREALIS")).toBe(false);
    expect(confirmMatches("", "BOREALIS")).toBe(false);
    expect(confirmMatches("", "")).toBe(false);
  });

  test("the status line says where each device stands", () => {
    expect(statusLine(device({ hub: true }), NOW)).toBe("This Mac");
    expect(statusLine(device(), NOW)).toBe("Seen 3 min ago");
    expect(statusLine(device({ revoked_ms: NOW - 2 * MIN, credential: false }), NOW)).toBe("Revoked 2 min ago");
    expect(statusLine(device({ credential: false }), NOW)).toBe("No credential: pair it again");
  });

  test("a log line reads as words, refusals marked", () => {
    expect(describeAction({ ts_ms: NOW, method: "GET", path: "/api/v1/profile", status: 403, write: false })).toBe("Tried to read your profile · denied");
    expect(describeAction({ ts_ms: NOW, method: "POST", path: "/api/v1/events", status: 201, write: true })).toBe("Published events");
    expect(describeAction({ ts_ms: NOW, method: "GET", path: "/api/v1/summary", status: 200, write: false })).toBe("Read the summary");
    expect(describeAction({ ts_ms: NOW, method: "POST", path: "/api/v1/notes", status: 403, write: true })).toBe("Tried to write to /api/v1/notes · denied");
  });

  test("serve's refusals come back in words", () => {
    expect(refusalText("confirm_required", 422)).toContain("type its name");
    expect(refusalText("hub_itself", 409)).toContain("can't be revoked");
    expect(refusalText("x", 500)).toContain("500");
  });

  test("the console proxy reaches the Pairing routes", () => {
    expect(GET_ROOTS.has("pairing")).toBe(true);
    expect(POST_ROOTS.has("pairing")).toBe(true);
  });
  test("a pending invite says when it stops working", () => {
    const i = { ref: "a1b2c3d4e5f6", name: "x", access: "workshop" as const, created_ms: NOW };
    expect(expiresLine({ ...i, expires_ms: NOW + 23 * 60 * MIN + 59 * MIN }, NOW)).toBe("Expires in 23 h");
    expect(expiresLine({ ...i, expires_ms: NOW + 40 * MIN }, NOW)).toBe("Expires in 40 min");
    expect(expiresLine({ ...i, expires_ms: NOW + 10_000 }, NOW)).toBe("Expires in under a minute");
    expect(refusalText("unknown_invite", 404)).toContain("no longer pending");
  });
});

describe("each device's harbard against the hub's", () => {
  const hub = { version: "0.3.0-alpha.9", commit: "a955a72e0c4d" };
  test("a different commit is behind; the same commit, short or long, is not", () => {
    expect(deviceVersion({ hub: false, version: "0.3.0-alpha.9", commit: "9433820aaaa" }, hub)).toEqual({ text: "0.3.0-alpha.9 · 9433820", behind: true });
    expect(deviceVersion({ hub: false, version: "0.3.0-alpha.9", commit: "a955a72" }, hub)?.behind).toBe(false);
  });
  test("without commits, the version decides; the hub's own row and old cards say nothing wrong", () => {
    expect(deviceVersion({ hub: false, version: "0.3.0-alpha.8", commit: null }, hub)?.behind).toBe(true);
    expect(deviceVersion({ hub: true, version: "0.3.0-alpha.8", commit: "x" }, hub)?.behind).toBe(false);
    expect(deviceVersion({ hub: false }, hub)).toBeNull();
    expect(deviceVersion({ hub: false, version: "0.3.0-alpha.8", commit: null }, null)?.behind).toBe(false);
  });
});
