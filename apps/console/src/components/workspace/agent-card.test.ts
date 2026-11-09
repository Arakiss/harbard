import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { Activity, ActivityState } from "@/lib/activity";
import type { Progress, StepRow } from "@/lib/progress";
import type { WsAgent } from "@/lib/workspace";
import { AgentCardView } from "./agent-card";

const NOW = 1_800_000_000_000;
const MIN = 60_000;

const agent: WsAgent = {
  key: "direct-card",
  name: "harbard-direct",
  label: "general-purpose · aa18a5f8",
  lost: false,
  kind: "teammate",
  lead_session: "s1",
  lead_quiet_ms: 0,
  delivery: "ready",
  state: "running",
  state_reason: "mid_turn",
  presence: "working",
  dormant: null,
  last_action_ms: NOW - 10_000,
  first_ms: NOW - 42 * MIN,
  brief: "You are the builder for two harbard features",
  description: "Build the agent hover card and live view",
  checkout: null,
  branch: null,
  build_dir: null,
  channels: ["harbard"],
  deliveries: 0,
  verified_deliveries: 0,
  reports: [],
  unread: 0,
  last_message: null,
  can_stop: false,
  activity: { state: "thinking", since_ms: NOW - 5_000, detail: null, source_ms: NOW },
  reactions: [],
};

const steps: StepRow[] = [
  { state: "testing", tool: "Bash", detail: "cargo test", started_ms: NOW - 20_000, ended_ms: null, status: "running", test: null },
  { state: "editing", tool: "Edit", detail: "card.tsx", started_ms: NOW - 3 * MIN, ended_ms: NOW - 3 * MIN + 80, status: "ok", test: null },
  { state: "testing", tool: "Bash", detail: "bun test", started_ms: NOW - 6 * MIN, ended_ms: NOW - 5 * MIN, status: "error", test: { passed: 41, failed: 2, skipped: 0 } },
  { state: "running", tool: "Bash", detail: "git add", started_ms: NOW - 8 * MIN, ended_ms: NOW - 8 * MIN + 30, status: "ok", test: null, commit: true },
  { state: "web", tool: "WebFetch", detail: "docs.rs", started_ms: NOW - 9 * MIN, ended_ms: NOW - 9 * MIN + 900, status: "ok", test: null },
];

const progress: Progress = { tool_calls: 118, commits: 3, test: { passed: 41, failed: 2, skipped: 0, ok: false, at_ms: NOW - 5 * MIN }, steps };

function card(activity: Activity | null, p: Progress | null = progress): string {
  return renderToString(h(AgentCardView, { agent, activity, micro: null, serverNow: NOW, offset: 0, progress: p }));
}

/** What a person reads: tags gone, entities decoded. */
function text(html: string): string {
  return html
    .replace(/<!--.*?-->/g, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/&#x27;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/\s+/g, " ")
    .trim();
}

describe("the card, per state", () => {
  const cases: [ActivityState, string | null, string][] = [
    ["thinking", null, "Thinking…"],
    ["running", "cargo build", "Running cargo build"],
    ["testing", "cargo test", "Running tests · cargo test"],
    ["editing", "card.tsx", "Editing card.tsx"],
    ["reading", "team.rs", "Reading team.rs"],
    ["web", "docs.rs", "docs.rs"],
    ["delegating", "2", "2"],
    ["waiting", null, "Waiting"],
    ["asleep", null, "No activity since"],
    ["blocked", null, "Blocked"],
    ["finished", null, "Finished"],
    ["verified", null, "Verified"],
    ["disconnected", null, "connection"],
  ];
  for (const [state, detail, words] of cases) {
    test(`${state}: "Now" says it with the sidebar's safe label and a ticking clock`, () => {
      const html = card({ state, since_ms: NOW - (MIN + 12_000), detail, source_ms: NOW });
      const t = text(html);
      expect(html).toContain(`class="hb-card-now" data-state="${state}"`);
      expect(t).toContain(words);
      expect(t).toContain("Now");
      expect(t).toContain("harbard-direct");
      expect(t).toContain("harbard working");
      expect(t).toContain("Build the agent hover card and live view");
      expect(t).toContain("Open conversation");
      if (state !== "disconnected") expect(t).toContain("· 1 min 12 s");
    });
  }

  test("no activity reads as no data, never a guess", () => {
    expect(text(card(null))).toContain("Now no data");
  });
});

describe("last steps and progress", () => {
  test("the last five steps, with an icon per kind, the words and when", () => {
    const html = card(agent.activity);
    const t = text(html);
    expect(html.match(/class="hb-step"/g)?.length).toBe(5);
    for (const kind of ["testing", "editing", "commit", "web"]) expect(html).toContain(`data-kind="${kind}"`);
    expect(t).toContain("cargo test running");
    expect(t).toMatch(/card\.tsx .*2 min/);
    expect(t).toMatch(/bun test · .* · with an error .*5 min/);
    expect(t).toMatch(/Commit .*7 min/);
    expect(t).toMatch(/docs\.rs .*8 min/);
  });

  test("progress: one line of real numbers; no bar, no percentage", () => {
    const html = card(agent.activity);
    const t = text(html);
    expect(t).toContain("Progress Running 42 min · 118 steps · 3 commits · Tests: 41 pass, 2 fail");
    expect(html).toContain('data-tone="fail"');
    expect(t).not.toContain("%");
    expect(html).not.toMatch(/role="progressbar"|<progress|<meter/);
  });

  test("an older serve with no progress shows the time only and no empty counters", () => {
    const t = text(card(agent.activity, null));
    expect(t).toContain("No steps yet.");
    expect(t).toContain("Progress Running 42 min");
    expect(t).not.toMatch(/\d+ steps/);
    expect(t).not.toMatch(/\d+ commits?/);
  });
});
