import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { UpdateView } from "@/lib/update";
import { UpdateDialogStatic } from "./update-dialog";

const NOW = Date.now();

function view(over: Partial<UpdateView> = {}): UpdateView {
  return {
    installed: { version: "0.3.0-alpha.9", commit: "a955a72e0c4d4f1a9b2e7d3c5f8a6b1e2d3c4f5a" },
    prefs: { channel: "main", auto_check: true },
    source: { path: "/Users/x/harbard", problem: null },
    check: {
      checked_ms: NOW,
      channel: "main",
      status: "available",
      error: null,
      target: { version: "0.3.0-alpha.9", commit: "4be59701c9d84e2fa1b6c3d7e8f9a0b1c2d3e4f5", tag: null, title: null, date_ms: NOW },
      behind: 2,
      notes: "### Updates\n\n- A button to <script>alert(1)</script> update",
      changes: [
        { commit: "1111111aaaa", kind: "feat", scope: "console", subject: "a notice and a button", date_ms: NOW },
        { commit: "2222222bbbb", kind: "fix", scope: null, subject: "the app reopens", date_ms: NOW },
      ],
      changes_truncated: false,
    },
    run: null,
    ...over,
  };
}

describe("the update dialog", () => {
  test("offers the newer version with its notes, its commits and one button", () => {
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view() }));
    expect(html).toContain("New version available");
    expect(html).toContain("0.3.0-alpha.9");
    expect(html).toContain("4be5970");
    expect(html).toContain("What&#x27;s new");
    expect(html).toContain("Fixes");
    expect(html).toContain("A notice and a button");
    expect(html).toContain("Update now");
    // The notes are data: a tag in them is shown, never parsed.
    expect(html).not.toContain("<script>");
  });

  test("a checkout that cannot fast-forward says why and holds the button", () => {
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view({ source: { path: "/Users/x/harbard", problem: "dirty" } }) }));
    expect(html).toContain("unsaved changes");
    expect(html).toMatch(/data-testid="update-now"[^>]*disabled/);
  });

  test("a run shows its steps, and a failure its reason and a retry", () => {
    const run = {
      id: "r1",
      state: "failed" as const,
      started_ms: NOW - 20_000,
      finished_ms: NOW,
      from: "a955a72",
      to: "4be5970",
      error: { code: "step_failed", message: "console" },
      steps: [
        { id: "fetch" as const, state: "done" as const, started_ms: NOW - 20_000, finished_ms: NOW - 18_000, detail: null },
        { id: "cli" as const, state: "done" as const, started_ms: NOW - 18_000, finished_ms: NOW - 10_000, detail: null },
        { id: "serve" as const, state: "skipped" as const, started_ms: NOW - 10_000, finished_ms: NOW - 10_000, detail: null },
        { id: "console" as const, state: "failed" as const, started_ms: NOW - 10_000, finished_ms: NOW, detail: null },
        { id: "desktop" as const, state: "pending" as const, started_ms: null, finished_ms: null, detail: null },
      ],
      log_tail: ["error: next build failed"],
    };
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view({ run }), ctx: { followed: "r1" } }));
    expect(html).toContain("Couldn&#x27;t update");
    expect(html).toContain("Build the console and the app");
    expect(html).toContain("Not needed on this Mac");
    expect(html).toContain("Retry");
    expect(html).toContain("Show log");
  });

  test("a serve that stops answering mid-run reads as a restart, never an error", () => {
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view(), ctx: { restarting: true } }));
    expect(html).toContain("Restarting harbard");
    expect(html).not.toContain("Couldn");
  });

  test("an old failed run this page did not follow does not hide the offer", () => {
    const run = { id: "old", state: "failed" as const, started_ms: 0, finished_ms: 1, from: null, to: null, error: null, steps: [], log_tail: [] };
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view({ run }) }));
    expect(html).toContain("Update now");
  });

  test("after an update, the notes show what that run brought", () => {
    const base = view();
    const run = {
      id: "r2", state: "done" as const, started_ms: NOW - 60_000, finished_ms: NOW, from: "a955a72", to: base.check!.target!.commit,
      error: null, steps: [], log_tail: [], notes: "- What arrived", changes: base.check!.changes,
    };
    const current = { ...base.check!, status: "current" as const, behind: 0, notes: null, changes: [] };
    const html = renderToStaticMarkup(h(UpdateDialogStatic, { view: view({ check: current, run }) }));
    expect(html).toContain("What this version includes");
    expect(html).toContain("What arrived");
    expect(html).toContain("A notice and a button");
    expect(html).not.toContain("Update now");
  });
});
