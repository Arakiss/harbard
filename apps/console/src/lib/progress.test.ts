import { describe, expect, test } from "bun:test";
import { clock, relative, stepKind, stepText, testSummary } from "./progress";

const T0 = 1_800_000_000_000;

describe("a step in words", () => {
  test("as shown, with the safe label only", () => {
    const cases: [Parameters<typeof stepText>[0], string][] = [
      [{ state: "editing", tool: "Edit", detail: "activity.rs" }, "Editing activity.rs"],
      [{ state: "reading", tool: "Read", detail: "team.rs" }, "Reading team.rs"],
      [{ state: "reading", tool: "Grep", detail: "say_wait" }, "Searching for “say_wait”"],
      [{ state: "running", tool: "Bash", detail: "cargo build" }, "Running cargo build"],
      [{ state: "running", tool: "Bash", detail: null }, "Running a command"],
      [{ state: "running", tool: "Bash", detail: "git add", commit: true }, "Commit"],
      [{ state: "testing", tool: "Bash", detail: "cargo test" }, "Running tests · cargo test"],
      [{ state: "web", tool: "WebSearch", detail: null }, "Searching the web"],
      [{ state: "web", tool: "WebFetch", detail: "docs.rs" }, "Opening docs.rs"],
      [{ state: "delegating", tool: "Agent", detail: null }, "Delegating to an agent"],
      [{ state: "thinking", tool: "SendMessage", detail: null }, "Sending a message"],
      [{ state: "thinking", tool: "TodoWrite", detail: null }, "Updating its tasks"],
      [{ state: "thinking", tool: null, detail: null }, "Using a tool"],
    ];
    for (const [s, want] of cases) expect(stepText(s)).toBe(want);
    expect(stepKind({ state: "running", tool: "Bash", commit: true })).toBe("commit");
    expect(stepKind({ state: "reading", tool: "Glob" })).toBe("searching");
  });

  test("counts, times and the running clock", () => {
    expect(testSummary({ passed: 877, failed: 0, skipped: 0 })).toBe("877 pass, 0 fail");
    expect(testSummary({ passed: 1, failed: 1, skipped: 2 })).toBe("1 passes, 1 fails, 2 skipped");
    expect(relative(T0 - 2_000, T0)).toBe("just now");
    expect(relative(T0 - 12_000, T0)).toBe("12 s ago");
    expect(relative(T0 - 3 * 60_000, T0)).toBe("3 min ago");
    expect(relative(T0 - 2 * 3_600_000, T0)).toBe("2 h ago");
    expect(clock(12_400)).toBe("12 s");
    expect(clock(72_000)).toBe("1 min 12 s");
    expect(clock(120_000)).toBe("2 min");
    expect(clock((2 * 60 + 14) * 60_000 + 5_000)).toBe("2 h 14 min");
  });
});
