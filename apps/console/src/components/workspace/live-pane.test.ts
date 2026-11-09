import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { LiveStep } from "@/lib/live-agent";
import { ProseBlock, TimelineRow } from "./live-pane";

const NOW = 1_800_000_000_000;
const MIN = 60_000;

function text(html: string): string {
  return html
    .replace(/<!--.*?-->/g, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/&#x27;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

const step = (over: Partial<LiveStep>): LiveStep => ({
  n: 1,
  rev: 2,
  state: "reading",
  tool: "Read",
  detail: "activity.rs",
  started_ms: NOW - 2_000,
  ended_ms: NOW - 1_800,
  status: "ok",
  test: null,
  ...over,
});

describe("the timeline, as drawn", () => {
  test("a finished step: its mark, icon, label, start, duration and an ok pill", () => {
    const html = renderToString(h(TimelineRow, { step: step({}), now: NOW }));
    expect(html).toContain('class="hb-tl-mark" data-status="ok"');
    expect(html).toContain('data-kind="reading"');
    expect(text(html)).toMatch(/activity\.rs \d\d:\d\d:\d\d 0\.2 s ok$/);
  });

  test("a running step ticks its duration and shows the spinner", () => {
    const html = renderToString(h(TimelineRow, { step: step({ state: "testing", tool: "Bash", detail: "cargo test", started_ms: NOW - 48_000, ended_ms: null, status: "running" }), now: NOW }));
    expect(html).toContain("hb-tl-spin");
    expect(text(html)).toMatch(/cargo test \d\d:\d\d:\d\d 48 s running$/);
  });

  test("errors, unanswered calls, searches, commits and test counts", () => {
    const err = text(renderToString(h(TimelineRow, { step: step({ state: "testing", tool: "Bash", detail: "bun test", ended_ms: NOW - MIN, started_ms: NOW - 2 * MIN, status: "error", test: { passed: 41, failed: 2, skipped: 0 } }), now: NOW })));
    expect(err).toMatch(/bun test · .* \d\d:\d\d:\d\d 1 min error$/);
    expect(text(renderToString(h(TimelineRow, { step: step({ status: "stopped", tool: "SendMessage", state: "thinking", detail: null }), now: NOW })))).toMatch(/ no response$/);
    expect(text(renderToString(h(TimelineRow, { step: step({ tool: "Grep", detail: "say_wait" }), now: NOW })))).toMatch(/^Searching for “say_wait”/);
    const commit = renderToString(h(TimelineRow, { step: step({ state: "running", tool: "Bash", detail: "git add", commit: true }), now: NOW }));
    expect(commit).toContain('data-kind="commit"');
    expect(text(commit)).toMatch(/^Commit /);
  });
});

describe("the agent's words", () => {
  test("prose is text: paragraphs, lists and inline code, never HTML", () => {
    const html = renderToString(
      h(ProseBlock, { prose: { n: 0, ts_ms: NOW, text: "First **build** with `cargo build`.\n\n- one\n- two\n\n<script>alert(1)</script> <img src=x onerror=alert(1)>" } }),
    );
    expect(html).toContain("<strong>build</strong>");
    expect(html).toContain("<code>cargo build</code>");
    expect(html).toContain("<li>one</li>");
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;script&gt;");
  });
});

describe("the heading", () => {
  test("a finished agent says Finished, never En direct", async () => {
    const { liveHeading } = await import("@/lib/live-agent");
    expect(liveHeading("running", "working")).toEqual({ text: "Live", live: true });
    expect(liveHeading("waiting", "waiting")).toEqual({ text: "Live", live: true });
    expect(liveHeading("finished", "finished")).toEqual({ text: "Finished", live: false });
    expect(liveHeading("verified", "finished")).toEqual({ text: "Finished", live: false });
    expect(liveHeading("normal", "finished")).toEqual({ text: "Finished", live: false });
  });

  test("the view renders it: no live dot and no 'en direct' for a finished agent", async () => {
    const { LiveView } = await import("./live-pane");
    const agent = {
      key: "a1", name: "harbard-robots", label: "general-purpose · a1", lost: false, kind: "teammate" as const, lead_session: "s", lead_quiet_ms: 0,
      delivery: "ready" as const, state: "done", state_reason: "done_sent", presence: "finished" as const, dormant: null, last_action_ms: NOW - MIN,
      first_ms: NOW - 60 * MIN, brief: null, description: "Build harbard robot avatars", checkout: null, branch: null, build_dir: null, channels: [],
      deliveries: 1, verified_deliveries: 0, reports: [], unread: 0, last_message: null, can_stop: false,
      activity: { state: "finished" as const, since_ms: NOW - MIN, detail: null, source_ms: NOW }, reactions: [],
    };
    const done = renderToString(h(LiveView, { agent, onOpenChat: () => {} }));
    expect(done).toContain('data-finished="true"');
    expect(done).toContain('data-live="false"');
    expect(done).not.toContain("hb-live-dot");
    expect(text(done)).not.toMatch(/\blive\b/i);
    expect(text(done)).toContain("Finished");
    const working = renderToString(h(LiveView, { agent: { ...agent, presence: "working" as const, activity: { ...agent.activity, state: "running" as const } }, onOpenChat: () => {} }));
    expect(working).toContain("hb-live-dot");
    expect(text(working)).toContain("Live");
  });
});
