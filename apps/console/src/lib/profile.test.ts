import { describe, expect, test } from "bun:test";
import {
  CONTEXT_MAX,
  clearAt,
  contextCount,
  contextFits,
  contextQuotedBytes,
  customUntil,
  dndChoices,
  dndUntil,
  effectiveStatus,
  isQuiet,
  doneUntil,
  oneShotsAllowed,
  quietLabel,
  stateOf,
  states,
  statusExpiry,
  statusPresets,
  type ProfileView,
} from "./profile";

// Local times, so the expectations hold in any time zone the tests run in.
const at = (h: number, m = 0, day = 7) => new Date(2026, 10, day, h, m);
const MIN = 60_000;

function view(over: Partial<ProfileView["profile"]> = {}, now = at(17)): ProfileView {
  return {
    profile: { photo_rev: 0, updated_ms: 0, ...over },
    status: null,
    quiet: false,
    quiet_until_ms: null,
    state: "available",
    state_until_ms: null,
    dock: { bounce: true, alerts: true, one_shots: true },
    now_ms: now.getTime(),
    limits: { context: 1500, name: 60, address: 120, status_text: 100, photo_bytes: 2 * 1024 * 1024 },
  };
}

describe("the status line", () => {
  test("suggests messages that are not one of the six states", () => {
    expect(statusPresets().map((p) => p.text)).toEqual(["At the computer", "In a meeting", "Eating", "On my way"]);
    for (const p of statusPresets()) expect(states().map((s) => s.label)).not.toContain(p.text);
  });

  test("clears after the time chosen, or never", () => {
    const now = at(17, 10);
    expect(clearAt("nunca", now)).toBeNull();
    expect(clearAt("30m", now)).toBe(now.getTime() + 30 * MIN);
    expect(clearAt("1h", now)).toBe(now.getTime() + 60 * MIN);
    expect(clearAt("4h", now)).toBe(now.getTime() + 240 * MIN);
    expect(clearAt("today", now)).toBe(at(0, 0, 8).getTime());
  });

  test("is gone once its time has come, even before serve says so", () => {
    const clear = at(17, 30).getTime();
    const v = view({ status: { emoji: "🎯", text: "Concentrado", clear_at_ms: clear } });
    v.status = v.profile.status ?? null;
    expect(effectiveStatus(v, at(17, 29).getTime())?.text).toBe("Concentrado");
    expect(effectiveStatus(v, clear)).toBeNull();
    expect(statusExpiry(clear, at(17, 0))).toBe("Clears at 17:30");
    expect(statusExpiry(at(9, 0, 8).getTime(), at(17, 0))).toBe("Clears tomorrow at 9:00");
    expect(statusExpiry(null, at(17))).toBeNull();
  });
});

describe("Do not disturb", () => {
  test("ends after 30 min, 1 h or tomorrow at 9:00", () => {
    expect(dndChoices().map((c) => c.label)).toEqual(["30 minutes", "1 hour", "Until tomorrow at 9:00"]);
    const now = at(17, 10);
    expect(dndUntil("30m", now)).toBe(now.getTime() + 30 * MIN);
    expect(dndUntil("1h", now)).toBe(now.getTime() + 60 * MIN);
    expect(dndUntil("tomorrow", now)).toBe(at(9, 0, 8).getTime());
    // Just after midnight "tomorrow" is still the next calendar day.
    expect(dndUntil("tomorrow", at(0, 30, 8))).toBe(at(9, 0, 9).getTime());
  });

  test("a custom end must be ahead and within two weeks", () => {
    const now = at(17);
    expect(customUntil("2026-11-07T19:45", now)).toBe(at(19, 45).getTime());
    expect(customUntil("2026-11-07T16:00", now)).toBeNull();
    expect(customUntil("2026-12-30T10:00", now)).toBeNull();
    expect(customUntil("", now)).toBeNull();
  });

  test("is on until its end, and says until when", () => {
    const until = at(18, 30).getTime();
    const v = view({ dnd: { since_ms: at(17).getTime(), until_ms: until } });
    v.quiet = true;
    v.quiet_until_ms = until;
    expect(isQuiet(v, at(18, 29).getTime())).toBe(true);
    expect(isQuiet(v, until)).toBe(false);
    expect(isQuiet(null, until)).toBe(false);
    expect(quietLabel(until, at(17))).toBe("until 18:30");
    expect(quietLabel(at(9, 0, 8).getTime(), at(17))).toBe("until tomorrow at 9:00");
    expect(quietLabel(at(9, 0, 12).getTime(), at(17))).toBe("until Thursday 12 at 9:00");
  });
});

describe("Context for the agents", () => {
  test("counts characters the way serve does, not UTF-16 units", () => {
    expect(contextCount("ñandú")).toBe(5);
    expect(contextCount("🎯🎯")).toBe(2);
    expect(contextCount("a\r\nb")).toBe(3);
    expect(CONTEXT_MAX).toBe(1500);
  });
});

describe("the coordination states", () => {
  test("are the six of the menu, with their copy", () => {
    expect(states().map((s) => [s.id, s.label, s.hint])).toEqual([
      ["available", "Available", "they can ask you"],
      ["focused", "Focused", "they only interrupt if something blocks them"],
      ["swamped", "Swamped", "they decide and write to you as little as possible"],
      ["away", "Away", "they keep working; a summary when you're back"],
      ["reviewing", "Reviewing", "send me what's waiting for my approval"],
      ["done", "Done for the day", "they finish what they're on and stop"],
    ]);
  });

  test("Done for the day lasts until the next start of the working hours", () => {
    const hours = { start: "09:00", end: "18:00", days: [1, 2, 3, 4, 5] };
    // Saturday 7 Nov 2026, 19:00 → Monday 9 Nov, 9:00.
    expect(doneUntil(hours, at(19))).toBe(at(9, 0, 9).getTime());
    // Monday 9 Nov at 7:00, before work → that same morning.
    expect(doneUntil(hours, at(7, 0, 9))).toBe(at(9, 0, 9).getTime());
    // Without working hours: tomorrow at 9:00.
    expect(doneUntil(null, at(19))).toBe(at(9, 0, 8).getTime());
  });

  test("the state in force falls back to Available once it ends", () => {
    const v = view();
    v.state = "done";
    v.state_until_ms = at(18).getTime();
    expect(stateOf(v, at(17).getTime()).id).toBe("done");
    expect(stateOf(v, at(18).getTime()).id).toBe("available");
    expect(stateOf(null, 0).id).toBe("available");
  });

  test("the Dock's one-shots stop in the quiet states and under Do not disturb", () => {
    expect(states().filter((s) => !oneShotsAllowed(s.id, false)).map((s) => s.id)).toEqual(["swamped", "away", "done"]);
    expect(oneShotsAllowed("available", true)).toBe(false);
  });
});

describe("the context's room in the agents' brief", () => {
  test("counts what the brief quotes, in bytes, like serve", () => {
    expect(contextQuotedBytes("ab\ncd")).toBe(10);
    expect(contextQuotedBytes("ñ")).toBe(5);
    expect(contextFits("x".repeat(1500))).toBe(true);
    expect(contextFits(vec("x", 750))).toBe(true);
    expect(contextFits("😀".repeat(1000))).toBe(false);
  });
});

function vec(s: string, n: number): string {
  return Array.from({ length: n }, () => s).join("\n");
}
