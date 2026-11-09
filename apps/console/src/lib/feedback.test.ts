import { describe, expect, test } from "bun:test";
import {
  controls,
  applyTap,
  applyUndo,
  bounceRise,
  bounceTarget,
  feedbackAllowed,
  messageTarget,
  standingFor,
  type FeedbackRecord,
  type FeedbackState,
} from "./feedback";

const rec = (over: Partial<FeedbackRecord> = {}): FeedbackRecord => ({
  id: "01K00000000000000000000001",
  ms: 1,
  target_kind: "message",
  target_id: "m-1",
  value: "unclear",
  reason: null,
  sample: null,
  undone: false,
  ...over,
});

describe("instant feedback", () => {
  test("no prompts at all while Focused, Swamped or Away", () => {
    for (const s of ["focused", "swamped", "away"]) expect(feedbackAllowed(s)).toBe(false);
    for (const s of [null, undefined, "available", "reviewing", "done"]) expect(feedbackAllowed(s)).toBe(true);
  });

  test("every target kind has its control and its copy", () => {
    const all = controls();
    expect(Object.keys(all).sort()).toEqual(["delivery", "dock", "message", "needs_you", "robot_state"]);
    expect(all.message.options.map((o) => o.label)).toContain("I don't understand");
    expect(all.delivery.question).toBe("Is this useful?");
    expect(all.delivery.options.map((o) => o.label)).toEqual(["Yes", "No", "Something's missing"]);
    expect(all.delivery.options.filter((o) => o.reason).map((o) => o.value)).toEqual(["no", "missing"]);
    expect(all.needs_you.options[0]?.label).toBe("This didn't need me");
    expect(all.robot_state.options[0]?.label).toBe("This isn't what it's doing");
    expect(all.dock.options[0]?.label).toBe("This bothered me");
  });

  test("a DONE is a delivery, another agent message is a message, the operator's own is neither", () => {
    expect(messageTarget({ from: "agent", label: "DONE" })).toBe("delivery");
    expect(messageTarget({ from: "agent", label: "START" })).toBe("message");
    expect(messageTarget({ from: "peer", label: null })).toBe("message");
    expect(messageTarget({ from: "operator", label: null })).toBeNull();
    expect(messageTarget({ from: "lead", label: null })).toBe("message");
  });

  test("a standing answer is found, an undone one is not, and undo restores the question", () => {
    const one = rec();
    let s: FeedbackState = { standing: [], last: null };
    s = applyTap(s, one);
    expect(standingFor(s, "message", "m-1")?.value).toBe("unclear");
    expect(standingFor(s, "delivery", "m-1")).toBeNull();
    expect(s.last?.id).toBe(one.id);
    s = applyTap(s, one);
    expect(s.standing.length).toBe(1);
    s = applyUndo(s, one);
    expect(standingFor(s, "message", "m-1")).toBeNull();
    expect(s.last).toBeNull();
  });

  test("a Dock bounce is a rise in what needs the operator, named by when it rang", () => {
    expect(bounceRise(0, 1)).toBe(true);
    expect(bounceRise(2, 3)).toBe(true);
    expect(bounceRise(3, 3)).toBe(false);
    expect(bounceRise(3, 1)).toBe(false);
    expect(bounceTarget(1_800_000_000_123)).toBe("bounce:1800000000123");
  });
});
