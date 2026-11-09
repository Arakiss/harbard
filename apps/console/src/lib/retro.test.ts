import { describe, expect, test } from "bun:test";
import { setActiveLocale } from "@/i18n";
import { memorySinkNote, sparkPath, tiles, type RetroBody } from "./retro";

const body: RetroBody = {
  v: 1,
  retro: {
    v: 1,
    now_ms: 1_794_074_400_000,
    since_ms: 1_794_074_400_000 - 7 * 86_400_000,
    scope: "week",
    lead: null,
    agents: 8,
    delivery: {
      brief_to_install_min: [],
      brief_to_install_p50_min: 150,
      review_rounds: { direct: 2, profile: 1, protocol: 1 },
      findings: [
        { class: "secret_leak", count: 3, branches: ["direct", "profile", "protocol"], repeats: true, days: ["2026-11-07"] },
        { class: "file_race", count: 2, branches: ["direct", "protocol"], repeats: true, days: ["2026-11-07"] },
      ],
    },
    messages: { not_understood: 1, jargon_chips: 0, what_does_it_mean: 1 },
    waiting: { unanswered: [{ note: "n1", at_ms: 1, open_min: 180 }], answered: 1, answer_p50_min: 12, idle_min: 40, stragglers: ["keychain-review-2"] },
    harness: { stalls: [{ agent: "harbard-profile", tool: "Bash", pattern: "rm_variable", at_ms: 1, minutes: 60 }], stall_min: 60, two_writers: 1, repeats: 0 },
    names: { names_down: 2, generated: ["keychain-review-2"], english_descriptions: ["a", "b"] },
    feedback: { name: { down: 2 } },
    daily: [0, 0, 0, 0, 0, 0, 5].map((findings, i) => ({ day: `2026-11-0${i + 1}`, findings, not_understood: i === 6 ? 1 : 0, stalls: i === 6 ? 1 : 0, unanswered: i === 6 ? 1 : 0 })),
    lessons: [],
  },
  candidates: [],
  rules: [],
  retired: [],
  finished_teams: [],
};

describe("the Retro view's numbers", () => {
  test("simple numbers with their labels, from the retro alone", () => {
    const t = tiles(body.retro);
    const by = Object.fromEntries(t.map((x) => [x.id, x]));
    expect(by.findings?.value).toBe("5");
    expect(by.findings?.label).toBe("Findings in review");
    expect(by.findings?.detail).toBe("3 secret leaks, 2 file-read races");
    expect(by.repeated?.value).toBe("2");
    expect(by.install?.value).toBe("2.5 h");
    expect(by.unanswered?.value).toBe("1");
    expect(by.stalls?.value).toBe("1");
    expect(by.stalls?.detail).toBe("60 min stopped");
    expect(by.unclear?.value).toBe("1");
    expect(by.rounds?.value).toBe("4");
    expect(by.findings?.series).toEqual([0, 0, 0, 0, 0, 0, 5]);
    expect(t.every((x) => !/[A-Z]{3,}|_/.test(x.label))).toBe(true);
  });

  test("a sparkline spans its box, baseline at the bottom, and an empty one is flat", () => {
    const d = sparkPath([0, 2, 4], 100, 20);
    expect(d).toBe("M1 19 L50 10 L99 1");
    expect(sparkPath([0, 0, 0], 100, 20)).toBe("M1 19 L50 19 L99 19");
    expect(sparkPath([], 100, 20)).toBe("");
  });
});

describe("what the console says when a rule stays only in harbard", () => {
  const skipped = (code: string, note: string, params: Record<string, string> = {}) => ({ saved: false, code, note, params });

  test("a known code is rendered in the operator's language, with its params", () => {
    setActiveLocale("es");
    try {
      expect(memorySinkNote(skipped("memory_sink_locked", "english text"))).toContain("harbard no la desbloquea");
      expect(memorySinkNote(skipped("memory_sink_refused", "english text", { reason: "duplicate" }))).toContain("(duplicate)");
    } finally {
      setActiveLocale("en");
    }
  });

  test("a code this console does not know falls back to the English serve sent", () => {
    expect(memorySinkNote(skipped("memory_sink_from_the_future", "Serve's own words."))).toBe("Serve's own words.");
  });
});
