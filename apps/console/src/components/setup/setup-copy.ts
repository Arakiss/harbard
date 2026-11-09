/**
 * serve describes each tool in English (`blurb`), which would leave one line
 * of the wizard in another language when Spanish is chosen; the wizard words
 * each tool by its id instead.
 */

import { type TranslationKey, createT } from "@/i18n";

const t = createT("setup");

const BLURBS: Record<string, TranslationKey<"setup">> = {
  claude: "blurbClaude",
  codex: "blurbCodex",
  cursor: "blurbCursor",
};

export function blurbFor(target: string, fallback = ""): string {
  const key = BLURBS[target];
  return key ? t(key) : fallback;
}

/** "1 tool", "3 tools". */
export function tools(n: number): string {
  return n === 1 ? t("toolsOne", { count: n }) : t("toolsMany", { count: n });
}
