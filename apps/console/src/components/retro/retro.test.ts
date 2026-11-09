import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { RetroBody } from "@/lib/retro";
import { RetroView, WeeklyRetroCardView } from "./retro";

function text(html: string): string {
  return html.replace(/<!--.*?-->/g, "").replace(/<[^>]+>/g, " ").replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
}

const NOW = 1_794_074_400_000;
const SHARED = "When two or more builders need the same security primitive, one owns it and the others reuse it";

const body: RetroBody = {
  v: 1,
  retro: {
    v: 1, now_ms: NOW, since_ms: NOW - 7 * 86_400_000, scope: "week", lead: null, agents: 8,
    delivery: { brief_to_install_min: [], brief_to_install_p50_min: 90, review_rounds: { direct: 2 }, findings: [{ class: "secret_leak", count: 3, branches: ["a", "b", "c"], repeats: true, days: ["2026-11-07"] }] },
    messages: { not_understood: 1, jargon_chips: 0, what_does_it_mean: 0 },
    waiting: { unanswered: [], answered: 0, answer_p50_min: null, idle_min: 0, stragglers: [] },
    harness: { stalls: [], stall_min: 0, two_writers: 0, repeats: 0 },
    names: { names_down: 0, generated: [], english_descriptions: [] },
    feedback: { message: { unclear: 1 } },
    daily: [0, 0, 0, 0, 0, 0, 3].map((findings, i) => ({ day: `2026-11-0${i + 1}`, findings, not_understood: 0, stalls: 0, unanswered: 0 })),
    lessons: [],
  },
  candidates: [{ key: "shared_security_primitive", title: "One security primitive, one owner", evidence: "3 secret leaks in 3 branches (a, b, c).", rule: SHARED, count: 3, dates: ["2026-11-07"] }],
  rules: [{ id: "01R", key: "rm_variable", title: "An rm on a variable stops the agent", text: 'A rm on a variable stalls on a prompt; use "${VAR:?}"', approved_ms: NOW, edited: false, memory_sink: false }],
  retired: [],
  finished_teams: ["11111111-2222"],
};

const noop = async () => {};

describe("the Retro view", () => {
  test("metrics as numbers and sparklines, with a table view", () => {
    const html = renderToString(h(RetroView, { body, scope: { kind: "week" }, onScope: () => {}, act: noop }));
    const t = text(html);
    expect(t).toContain("Findings in review");
    expect(t).toContain("3 secret leaks");
    expect(html).toContain('data-spark="findings"');
    expect(t).toContain("See the data by day");
    expect(t).toContain("This week");
  });

  test("each candidate offers approve, edit and discard, and active rules retire", () => {
    const t = text(renderToString(h(RetroView, { body, scope: { kind: "week" }, onScope: () => {}, act: noop })));
    expect(t).toContain("One security piece, one owner");
    expect(t).toContain(SHARED);
    for (const w of ["Approve", "Edit", "Discard", "Retire"]) expect(t).toContain(w);
    expect(t).toContain("Active rules");
    expect(t).toContain('use "${VAR:?}"');
  });

  test("the save tick shows only when a sink is configured and allowed in Preferences, unticked, and a note says what happened", () => {
    const absent = text(renderToString(h(RetroView, { body, scope: { kind: "week" }, onScope: () => {}, act: noop })));
    expect(absent).not.toContain("Save to external memory");
    const notAllowed = text(renderToString(h(RetroView, { body: { ...body, memory_sink: { configured: true, allowed: false } }, scope: { kind: "week" }, onScope: () => {}, act: noop })));
    expect(notAllowed).not.toContain("Save to external memory");
    const present = renderToString(h(RetroView, { body: { ...body, memory_sink: { configured: true, allowed: true } }, scope: { kind: "week" }, onScope: () => {}, act: noop, notice: "The external memory is locked" }));
    expect(text(present)).toContain("Save to external memory");
    expect(present).not.toMatch(/type="checkbox"[^>]*checked/);
    expect(text(present)).toContain("The external memory is locked");
  });

  test("the weekly card in Activity names how many rules wait, and hides when none", () => {
    expect(text(renderToString(h(WeeklyRetroCardView, { candidates: 3 })))).toContain("3 suggested rules");
    expect(renderToString(h(WeeklyRetroCardView, { candidates: 0 }))).toBe("");
  });
});
