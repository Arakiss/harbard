import { describe, expect, test } from "bun:test";
import {
  bannerLine,
  bannerMode,
  broughtBy,
  changesCount,
  errorText,
  groupChanges,
  pageIsStale,
  readDismissed,
  runSteps,
  sentenceCase,
  statusSentence,
  stepDetail,
  stepElapsed,
  targetName,
  toastFor,
  writeDismissed,
  type UpdateChange,
  type UpdateCheck,
  type UpdateRun,
  type UpdateView,
} from "./update";
import { hace } from "./workspace";

const NOW = Date.UTC(2026, 10, 8, 15, 0, 0);
const LAST_CHECK = hace(NOW - 5 * 60_000, NOW);

function check(over: Partial<UpdateCheck> = {}): UpdateCheck {
  return {
    checked_ms: NOW - 5 * 60_000,
    channel: "main",
    status: "available",
    error: null,
    target: { version: "0.3.0-alpha.9", commit: "b".repeat(40), tag: null, title: null, date_ms: NOW - 3_600_000 },
    behind: 11,
    notes: "### Algo\n\n- uno",
    changes: [],
    changes_truncated: false,
    ...over,
  };
}

function run(over: Partial<UpdateRun> = {}): UpdateRun {
  return {
    id: "run-1",
    state: "running",
    started_ms: NOW - 60_000,
    finished_ms: null,
    from: "a".repeat(40),
    to: "b".repeat(40),
    error: null,
    steps: [],
    log_tail: [],
    ...over,
  };
}

function view(over: Partial<UpdateView> = {}): UpdateView {
  return {
    installed: { version: "0.3.0-alpha.9", commit: "a".repeat(40) },
    prefs: { channel: "main", auto_check: true },
    source: { path: "/tmp/harbard", problem: null },
    check: check(),
    run: null,
    ...over,
  };
}

function memory(): Storage {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
    key: () => null,
    get length() {
      return m.size;
    },
  };
}

describe("the banner", () => {
  test("shows a new version until it is put off for that very target", () => {
    expect(bannerMode(view(), null, "a".repeat(7), false)).toBe("available");
    expect(bannerMode(view(), "b".repeat(40), "a".repeat(7), false)).toBeNull();
    // A newer target than the one dismissed shows again.
    expect(bannerMode(view(), "c".repeat(40), "a".repeat(7), false)).toBe("available");
  });

  test("stays quiet when up to date, on an error, or before the first check", () => {
    expect(bannerMode(view({ check: check({ status: "current", behind: 0 }) }), null, "", false)).toBeNull();
    expect(bannerMode(view({ check: check({ status: "error" }) }), null, "", false)).toBeNull();
    expect(bannerMode(view({ check: null }), null, "", false)).toBeNull();
    expect(bannerMode(null, null, "", false)).toBeNull();
  });

  test("a run shows whatever was dismissed, and so does a restart", () => {
    expect(bannerMode(view({ run: run() }), "b".repeat(40), "", false)).toBe("running");
    expect(bannerMode(null, null, "", true)).toBe("running");
  });

  test("a finished run asks for a reload only while the page runs the older build", () => {
    const done = run({ state: "done", finished_ms: NOW });
    const v = view({ run: done, check: check({ status: "current", behind: 0 }) });
    expect(bannerMode(v, null, "aaaaaaa", false)).toBe("done");
    expect(bannerMode(v, null, "bbbbbbb", false)).toBeNull();
    expect(pageIsStale(done, "")).toBe(false);
  });

  test("words the count", () => {
    expect(bannerLine(check())).toBe("A new version of harbard is available · 11 changes");
    expect(bannerLine(check({ behind: 0 }))).toBe("A new version of harbard is available");
    expect(changesCount(1)).toBe("1 change");
  });
});

describe("dismissal", () => {
  test("is kept per target commit and survives a broken store", () => {
    const s = memory();
    expect(readDismissed(s)).toBeNull();
    writeDismissed(s, "abc");
    expect(readDismissed(s)).toBe("abc");
    const broken = { getItem: () => { throw new Error("x"); }, setItem: () => { throw new Error("x"); } };
    expect(readDismissed(broken)).toBeNull();
    expect(() => writeDismissed(broken, "abc")).not.toThrow();
  });
});

describe("the toast after a reload", () => {
  test("once per recent finished run", () => {
    const done = run({ state: "done", finished_ms: NOW - 60_000 });
    expect(toastFor(done, NOW, null)).toBe(true);
    expect(toastFor(done, NOW, "run-1")).toBe(false);
    expect(toastFor(run({ state: "done", finished_ms: NOW - 11 * 60_000 }), NOW, null)).toBe(false);
    expect(toastFor(run({ state: "failed", finished_ms: NOW }), NOW, null)).toBe(false);
    expect(toastFor(run(), NOW, null)).toBe(false);
    expect(toastFor(null, NOW, null)).toBe(false);
  });
});

describe("the changelog", () => {
  const c = (kind: UpdateChange["kind"], subject: string): UpdateChange => ({ commit: subject, kind, scope: null, subject, date_ms: null });

  test("groups commits as a changelog does, in a fixed order, keeping order inside", () => {
    const groups = groupChanges([c("fix", "f1"), c("docs", "d1"), c("feat", "n1"), c("perf", "p1"), c("feat", "n2"), c("refactor", "r1")]);
    expect(groups.map((g) => [g.label, g.changes.map((x) => x.subject)])).toEqual([
      ["New", ["n1", "n2"]],
      ["Fixes", ["f1"]],
      ["Improvements", ["p1", "r1"]],
      ["Other", ["d1"]],
    ]);
    expect(groupChanges([])).toEqual([]);
  });

  test("capitalises a subject without touching the rest", () => {
    expect(sentenceCase("a rail tile per workspace")).toBe("A rail tile per workspace");
    expect(sentenceCase("")).toBe("");
  });

  test("names the target by its tag, its version, or main", () => {
    expect(targetName(check({ target: { version: "0.3.0-alpha.10", commit: "c", tag: "v0.3.0-alpha.10", title: null, date_ms: null } }), "0.3.0-alpha.9")).toBe("0.3.0-alpha.10");
    expect(targetName(check({ target: { version: "0.3.1", commit: "c", tag: null, title: null, date_ms: null } }), "0.3.0")).toBe("0.3.1");
    expect(targetName(check(), "0.3.0-alpha.9")).toBe("main");
  });
});

describe("the run", () => {
  test("draws every step pending before serve answers", () => {
    expect(runSteps(null).map((s) => [s.id, s.state])).toEqual([
      ["fetch", "pending"],
      ["cli", "pending"],
      ["hooks", "pending"],
      ["serve", "pending"],
      ["console", "pending"],
      ["desktop", "pending"],
    ]);
  });

  test("times a step while it runs and once it ends, never a skipped one", () => {
    expect(stepElapsed({ id: "cli", state: "running", started_ms: NOW - 72_000, finished_ms: null, detail: null }, NOW)).toBe("1 min 12 s");
    expect(stepElapsed({ id: "cli", state: "done", started_ms: NOW - 20_000, finished_ms: NOW - 5_000, detail: null }, NOW)).toBe("15 s");
    expect(stepElapsed({ id: "serve", state: "skipped", started_ms: NOW, finished_ms: NOW, detail: null }, NOW)).toBeNull();
  });

  test("words a failure by its code, and keeps serve's message for an unknown one", () => {
    expect(errorText({ code: "dirty", message: "x" })).toContain("unsaved changes");
    expect(errorText({ code: "new", message: "something new" })).toBe("something new");
    expect(errorText(null)).toBe("");
  });
});

describe("the status sentence", () => {
  test("covers every state", () => {
    expect(statusSentence(null, NOW)).toContain("isn't responding");
    expect(statusSentence(view({ check: null }), NOW)).toContain("No check");
    expect(statusSentence(view(), NOW)).toBe(`A new version of harbard is available · 11 changes. Last checked: ${LAST_CHECK}.`);
    expect(statusSentence(view({ check: check({ status: "current", behind: 0 }) }), NOW)).toBe(`You're up to date. Last checked: ${LAST_CHECK}.`);
    expect(statusSentence(view({ check: check({ status: "error", error: { code: "fetch_failed", message: "" } }) }), NOW)).toContain("Couldn't download");
  });
});

describe("serve's step details and what a run brought", () => {
  test("a skipped step says why in words; a failed one keeps its own line below; others say nothing", () => {
    expect(stepDetail({ id: "desktop", state: "skipped", started_ms: 0, finished_ms: 0, detail: "not_open" })).toEqual({ text: "The app wasn't open", where: "side" });
    expect(stepDetail({ id: "serve", state: "skipped", started_ms: 0, finished_ms: 0, detail: "not_installed" })).toEqual({ text: "Not installed on this Mac", where: "side" });
    expect(stepDetail({ id: "serve", state: "skipped", started_ms: 0, finished_ms: 0, detail: "new" })?.text).toBe("Not needed on this Mac");
    expect(stepDetail({ id: "console", state: "failed", started_ms: 0, finished_ms: 1, detail: "next build exited with status 1" })).toEqual({ text: "next build exited with status 1", where: "below" });
    expect(stepDetail({ id: "cli", state: "done", started_ms: 0, finished_ms: 1, detail: "whatever" })).toBeNull();
  });

  test("a finished run's notes and changes, when serve kept them", () => {
    const change = { commit: "c", kind: "feat" as const, scope: null, subject: "s", date_ms: null };
    expect(broughtBy(run({ state: "done", finished_ms: NOW, notes: " n ", changes: [change] }))).toEqual({ notes: "n", changes: [change], truncated: false });
    expect(broughtBy(run({ state: "done", finished_ms: NOW }))).toBeNull();
    expect(broughtBy(run({ notes: "n" }))).toBeNull();
    expect(broughtBy(null)).toBeNull();
  });
});
