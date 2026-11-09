import { describe, expect, test } from "bun:test";
import { REACTION_JOIN_MS, WORKING_STATES, elapsed, logoState, microState, nextReaction, spendReactions, statusText, type Reaction, type Thresholds } from "./activity";

const MIN = 60_000;
const NOW = 1_794_000_000_000;
const T: Thresholds = { amber_ms: 15 * MIN, red_ms: 45 * MIN, thinking_long_ms: 2 * MIN, waiting_sleepy_ms: 5 * MIN, running_long_ms: 10 * MIN };

/** The microstate at `minutes` into a state that started at NOW (a fake clock). */
const at = (state: Parameters<typeof microState>[0]["state"], minutes: number, t = T) => microState({ state, since_ms: NOW }, NOW + minutes * MIN, t);

describe("microstates come from since_ms and the desk's thresholds", () => {
  test("thinking over 2 min looks from side to side", () => {
    expect(at("thinking", 1.9)).toMatchObject({ face: "thinking", label: null });
    expect(at("thinking", 2)).toMatchObject({ face: "reading", label: "Thinking for a long time" });
  });

  test("waiting over 5 min dozes, and is asleep at 15", () => {
    expect(at("waiting", 4.9)).toMatchObject({ face: "waiting", sleepy: false });
    expect(at("waiting", 5)).toMatchObject({ face: "waiting", sleepy: true, label: "Waiting, getting sleepy" });
    expect(at("waiting", 15)).toMatchObject({ face: "asleep", label: "Asleep while waiting" });
  });

  test("running or testing over 10 min is flagged", () => {
    expect(at("running", 9.9)).toMatchObject({ face: "running", long: false });
    expect(at("running", 10)).toMatchObject({ face: "running", long: true });
    expect(at("testing", 10)).toMatchObject({ face: "running", long: true });
  });

  test("asleep over 45 min is flagged red", () => {
    expect(at("asleep", 44)).toMatchObject({ face: "asleep", long: false });
    expect(at("asleep", 45)).toMatchObject({ face: "asleep", long: true, label: "Asleep too long" });
  });

  test("the thresholds are the desk's, not constants", () => {
    const t = { ...T, thinking_long_ms: 30_000, running_long_ms: MIN };
    expect(at("thinking", 0.5, t).face).toBe("reading");
    expect(at("running", 1, t).long).toBe(true);
  });

  test("editing draws the typing face; every other state is itself", () => {
    expect(at("editing", 30).face).toBe("running");
    for (const s of ["normal", "reading", "web", "delegating", "blocked", "finished", "verified", "disconnected"] as const) {
      expect(at(s, 90).face).toBe(s);
    }
  });
});

describe("the status line says what agents.html says", () => {
  const line = (state: Parameters<typeof statusText>[0]["state"], detail: string | null = null, opts = {}) =>
    statusText({ state, since_ms: Date.UTC(2026, 10, 7, 0, 12), detail }, opts);
  test("per state", () => {
    expect(line("thinking")).toBe("Thinking…");
    expect(line("running", "cargo build")).toBe("Running cargo build");
    expect(line("running")).toBe("Running a command");
    expect(line("testing", "cargo test")).toBe("Running tests · cargo test");
    expect(line("editing", "workspace.css")).toBe("Editing workspace.css");
    expect(line("reading", "team.rs")).toBe("Reading team.rs");
    expect(line("web", "docs.rs")).toBe("On the web · docs.rs");
    expect(line("delegating", "2")).toBe("Delegating to 2 agents");
    expect(line("delegating", "1")).toBe("Delegating to 1 agent");
    expect(line("waiting")).toBe("Waiting in the background since 00:12");
    expect(line("asleep")).toBe("No activity since 00:12");
    expect(line("blocked", "Can it ask for the password twice?")).toBe("Blocked · “Can it ask for the password twice?”");
    expect(line("finished", null, { reports: 2 })).toBe("Finished · 2 reports");
    expect(line("verified")).toBe("Verified by harbard");
    expect(line("disconnected")).toBe("No connection to harbard");
  });

  test("the timer", () => {
    expect([elapsed(8_000), elapsed(104_000), elapsed(21 * MIN), elapsed(60 * MIN), elapsed(65 * MIN)]).toEqual(["8 s", "1 min", "21 min", "1 h", "1 h 5 min"]);
  });
});

describe("a reaction plays once, as it happens", () => {
  const r = (kind: Reaction["kind"], at_ms: number): Reaction => ({ kind, at_ms });

  test("events from before the page loaded never play", () => {
    expect(nextReaction("a", [r("glance", NOW - 1)], NOW, new Map(), 0)).toBeNull();
  });

  test("a new one plays once; a later poll with the same event does not replay it", () => {
    const played = new Map<string, number>();
    expect(nextReaction("a", [r("glance", NOW + 5)], NOW, played, 1_000)?.kind).toBe("glance");
    expect(nextReaction("a", [r("glance", NOW + 5)], NOW, played, 1_000 + REACTION_JOIN_MS)).toBeNull();
  });

  test("another view of the same agent mounted at once joins the same play", () => {
    const played = new Map<string, number>();
    nextReaction("a", [r("hop", NOW + 5)], NOW, played, 1_000);
    expect(nextReaction("a", [r("hop", NOW + 5)], NOW, played, 1_200)?.kind).toBe("hop");
  });

  test("of reactions arriving together the strongest plays and the rest are spent", () => {
    const played = new Map<string, number>();
    const both = [r("glance", NOW + 5), r("shake", NOW + 5)];
    expect(nextReaction("a", both, NOW, played, 1_000)?.kind).toBe("shake");
    expect(nextReaction("a", both, NOW, played, 1_000 + REACTION_JOIN_MS)).toBeNull();
  });
});

describe("long runs and the reduced-motion note", () => {
  test("an agent three minutes into a test run makes the logo working", () => {
    // Its newest record is the tool_use, 3 min old; its state is testing.
    expect(logoState({ connection: "connected", blocked: 0, needs: 0, lastActions: [NOW - 3 * MIN], working: 1, now: NOW })).toBe("working");
    expect(WORKING_STATES.has("testing") && WORKING_STATES.has("thinking") && !WORKING_STATES.has("waiting")).toBe(true);
  });

  test("reactions skipped under Reduce motion are spent, never played later", () => {
    const played = new Map<string, number>();
    const rs: Reaction[] = [{ kind: "glance", at_ms: NOW + 5 }, { kind: "hop", at_ms: NOW + 6 }];
    spendReactions("a", rs, NOW, played, 1_000);
    expect(nextReaction("a", rs, NOW, played, 1_000)).toBeNull();
    expect(nextReaction("a", rs, NOW, played, 60_000)).toBeNull();
  });
});

describe("the living logo's state", () => {
  const base = { connection: "connected" as const, blocked: 0, needs: 0, lastActions: [] as (number | null)[], working: 0, now: NOW };
  test("by precedence", () => {
    expect(logoState(base)).toBe("idle");
    expect(logoState({ ...base, lastActions: [NOW - 30_000] })).toBe("working");
    expect(logoState({ ...base, lastActions: [NOW - 61_000] })).toBe("idle");
    expect(logoState({ ...base, needs: 1, lastActions: [NOW] })).toBe("needs");
    expect(logoState({ ...base, blocked: 1, needs: 1 })).toBe("blocked");
    expect(logoState({ ...base, connection: "connecting", blocked: 1 })).toBe("connecting");
    expect(logoState({ ...base, connection: "disconnected", blocked: 1 })).toBe("disconnected");
  });
});
