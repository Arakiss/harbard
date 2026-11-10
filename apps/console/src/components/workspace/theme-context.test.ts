import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import { STATE } from "@/lib/theme.fixture";
import { ThemeProvider, useThemeAttrs } from "./theme-context";

// The server render carries the stored theme (criterion 7 of the themes
// spec): the <style> and the root attributes are in the first HTML, so the
// browser never paints the light default before a stored custom theme.
// `.test.ts` without JSX: the console's tsconfig leaves test files out.
function Root() {
  return h("div", { className: "hb", ...useThemeAttrs() });
}

describe("the first HTML already carries the stored theme", () => {
  test("with a stored theme: the stylesheet and the root attributes", () => {
    const html = renderToString(h(ThemeProvider, { initial: STATE }, h(Root)));
    expect(html).toContain('<style data-hb-theme-style="">.hb{');
    expect(html).toContain("--hb-frame:#0d3b2e;");
    expect(html).toContain('@media (prefers-color-scheme: dark){.hb[data-hb-mode="system"]{');
    expect(html).toContain('data-hb-theme="jade"');
    expect(html).toContain('data-hb-mode="system"');
    expect(html.indexOf("<style")).toBeLessThan(html.indexOf('class="hb"'));
  });

  test("without serve: no stylesheet, the light default from workspace.css", () => {
    const html = renderToString(h(ThemeProvider, null, h(Root)));
    expect(html).not.toContain("<style");
    expect(html).toContain('data-hb-theme="ink"');
    expect(html).toContain('data-hb-mode="light"');
  });
});
