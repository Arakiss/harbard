import { createT } from "@/i18n";
const t = createT("workspace");

/** Human label for a harness id; unknown values pass through inert. */
export function harnessLabel(harness: string): string {
  switch (harness) {
    case "claude-code":
      return "Claude Code";
    case "cursor":
      return "Cursor";
    case "codex":
      return "Codex";
    case "human":
      return t("harnessHuman");
    default:
      return harness;
  }
}

/** The chip tone of each step of Set up harbard (`.hb-chip[data-tone]`). */
export const SETUP_TONE = {
  found: "green",
  missing: "neutral",
  wired: "green",
  failed: "red",
  detected: "amber",
} as const;
