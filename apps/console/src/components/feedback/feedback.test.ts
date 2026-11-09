import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { FeedbackRecord, TargetKind } from "@/lib/feedback";
import { DockFeedbackView, FeedbackProvider, FeedbackRow } from "./feedback";

/** What a person reads: tags gone, entities decoded. */
function text(html: string): string {
  return html
    .replace(/<!--.*?-->/g, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/&#x27;|&#39;/g, "'")
    .replace(/&quot;/g, '"')
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

function row(kind: TargetKind, id: string, opts: { state?: string | null; standing?: FeedbackRecord[]; last?: FeedbackRecord | null; sample?: { state: string; label?: string | null } } = {}): string {
  return renderToString(
    h(
      FeedbackProvider,
      { operatorState: opts.state ?? null, initial: { standing: opts.standing ?? [], last: opts.last ?? null } },
      h(FeedbackRow, { kind, id, sample: opts.sample }),
    ),
  );
}

const rec = (kind: TargetKind, id: string, value: string, eventId = "01K00000000000000000000009"): FeedbackRecord => ({
  id: eventId,
  ms: 1,
  target_kind: kind,
  target_id: id,
  value,
  reason: null,
  sample: null,
  undone: false,
});

describe("one row of controls per target", () => {
  test("an agent message: thumbs up and thumbs down", () => {
    const html = row("message", "m-1");
    expect(html).toContain('data-fb-kind="message"');
    expect(html).toContain('aria-label="Makes sense"');
    expect(html).toContain('aria-label="I don&#x27;t understand"');
  });

  test("a delivery asks whether it is useful", () => {
    const t = text(row("delivery", "m-done"));
    expect(t).toContain("Is this useful?");
    for (const w of ["Yes", "No", "Something's missing"]) expect(t).toContain(w);
  });

  test("a Needs you card offers saying it did not need you", () => {
    expect(text(row("needs_you", "blocked:x"))).toContain("This didn't need me");
  });

  test("a robot state can be called wrong, carrying only state and label", () => {
    const html = row("robot_state", "a1", { sample: { state: "testing", label: "cargo test" } });
    expect(text(html)).toContain("This isn't what it's doing");
    expect(html).toContain('data-fb-sample="testing"');
  });

  test("a Dock bounce can be reported, only while the bounce is recent", () => {
    const now = 1_800_000_000_000;
    const shown = renderToString(
      h(FeedbackProvider, { operatorState: null, initial: { standing: [], last: null } }, h(DockFeedbackView, { bounce: { at_ms: now - 60_000, state: "needs" }, now })),
    );
    expect(text(shown)).toContain("This bothered me");
    expect(text(shown)).toContain("The Dock bounced");
    const old = renderToString(
      h(FeedbackProvider, { operatorState: null, initial: { standing: [], last: null } }, h(DockFeedbackView, { bounce: { at_ms: now - 31 * 60_000, state: "needs" }, now })),
    );
    expect(text(old)).toBe("");
    const none = renderToString(h(FeedbackProvider, { operatorState: null, initial: { standing: [], last: null } }, h(DockFeedbackView, { bounce: null, now })));
    expect(text(none)).toBe("");
  });

  test("no controls at all while Focused, Swamped or Away", () => {
    for (const state of ["focused", "swamped", "away"]) {
      for (const kind of ["message", "delivery", "needs_you", "robot_state", "dock"] as TargetKind[]) {
        expect(row(kind, "x", { state, sample: { state: "thinking" } })).not.toContain("hb-fb");
      }
    }
    expect(row("message", "x", { state: "available" })).toContain("hb-fb");
  });

  test("never asked twice: an answered target shows the acknowledgement, and Undo only for the last tap", () => {
    const given = rec("message", "m-1", "unclear");
    const last = text(row("message", "m-1", { standing: [given], last: given }));
    expect(last).toContain("Noted: unclear");
    expect(last).toContain("Undo");
    expect(last).not.toContain("Makes sense ");
    const older = text(row("message", "m-1", { standing: [given], last: rec("needs_you", "c1", "not_needed", "01K0000000000000000000000A") }));
    expect(older).toContain("Noted");
    expect(older).not.toContain("Undo");
  });

  test("outside the provider nothing renders", () => {
    expect(renderToString(h(FeedbackRow, { kind: "message", id: "m-1" }))).toBe("");
  });
});
