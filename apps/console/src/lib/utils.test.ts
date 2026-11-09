import { describe, expect, test } from "bun:test";

import { TYPE_SCALE, cn } from "./utils";

/**
 * The type scale is role-named, so tailwind-merge reads every one of its
 * tokens as a colour unless it is told otherwise — and then a size and a
 * colour fight over the same slot and one of them disappears. It shipped: the
 * Edge strip's Acknowledge lost `text-primary-foreground` and rendered as a
 * cream label on a cream button.
 *
 * A type token is added to globals.css by editing one line, and nothing about
 * that edit suggests you also owe the merge a declaration. These tests are
 * where that debt gets collected.
 */
describe("cn keeps size and colour apart", () => {
  test.each([...TYPE_SCALE])("text-%s survives next to a colour", (step) => {
    expect(cn(`text-ink text-${step}`)).toBe(`text-ink text-${step}`);
    expect(cn(`text-${step} text-ink`)).toBe(`text-${step} text-ink`);
  });

  test("the Acknowledge button keeps its foreground", () => {
    const merged = cn(
      "bg-primary text-primary-foreground text-sm",
      "h-12 rounded-xl px-4 text-ui font-semibold",
    );
    expect(merged).toContain("text-primary-foreground");
    expect(merged).toContain("text-ui");
    expect(merged).not.toContain("text-sm");
  });

  test("same-group conflicts still collapse", () => {
    expect(cn("text-body text-ui")).toBe("text-ui");
    expect(cn("text-ink text-signal")).toBe("text-signal");
    expect(cn("px-2 px-4")).toBe("px-4");
  });
});
