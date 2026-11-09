import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { blurbFor } from "./setup-copy";

// The wizard's copy lives in the dictionaries: none of its old strings stay in the source.
const source = readFileSync(join(import.meta.dir, "setup-screen.tsx"), "utf8");
const ENGLISH = [
  "Skip for now",
  "Detect harnesses",
  "Scanning this machine",
  "Looking for",
  "not found",
  "No harness detected",
  "Confirm which ones",
  "Wire ",
  "Already wired",
  "Wiring incomplete",
  "Writing hooks",
  "Back to choose",
  "Retry",
  "on the watch",
  "Opening the board",
  "Detect failed",
  "Install failed",
  "Could not save setup",
  ">Setup<",
];

describe("Set up harbard", () => {
  test("keeps no hard-coded copy in the screen", () => {
    expect(ENGLISH.filter((s) => source.includes(s))).toEqual([]);
  });

  test("says what each tool's hooks do by its id, whatever serve sends", () => {
    expect(blurbFor("claude")).toContain("Claude Code");
    expect(blurbFor("codex")).toContain("Codex");
    expect(blurbFor("cursor")).toContain("Cursor");
    expect(blurbFor("otra", "fallback")).toBe("fallback");
    expect(source).toContain("blurbFor(");
  });
});

describe("Set up harbard looks like the shell", () => {
  const frame = readFileSync(join(import.meta.dir, "..", "workspace", "shell.tsx"), "utf8");

  test("speaks the shell's classes and tokens, and shows the burbuja", () => {
    for (const old of ["bg-rail", "text-signal", "bg-signal", "setup-screen fixed", "@/components/chat/mark", "@/components/ui/button", "oklch("]) {
      expect(source.includes(old), old).toBe(false);
    }
    expect(source).toContain("BrandMark");
    expect(source).toContain('className="hb-main hb-setup"');
    expect(source).toContain("hb-btn hb-btn-primary");
  });

  test("its frame is the whole shell root, not a rail beside the old look", () => {
    const moreFrame = frame.slice(frame.indexOf("export function MoreFrame"));
    expect(moreFrame).toContain('className="hb"');
    expect(moreFrame).toContain('className="hb-panel"');
    expect(moreFrame).not.toContain("hb-railonly");
  });

  test("the failure message comes from the dictionary too", () => {
    expect(source).not.toContain("failed to wire");
  });
});
