import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import { MemorySinkSectionView } from "./memory-sink-section";

describe("Preferences › external memory", () => {
  test("saving to external memory is off by default", () => {
    const html = renderToString(h(MemorySinkSectionView, { state: { configured: true, allowed: false }, onChange: () => {} }));
    expect(html).toContain("Allow saving to external memory");
    expect(html).not.toMatch(/<input[^>]*checked/);
  });
  test("without a configured sink it cannot be turned on and it says so", () => {
    const html = renderToString(h(MemorySinkSectionView, { state: { configured: false, allowed: false }, onChange: () => {} }));
    expect(html).toMatch(/<input[^>]*disabled/);
    expect(html).toContain("No external memory is configured");
  });
  test("on, it says what it does and what it never does", () => {
    const html = renderToString(h(MemorySinkSectionView, { state: { configured: true, allowed: true }, onChange: () => {} }));
    expect(html).toMatch(/<input[^>]*checked/);
    expect(html).toContain("never unlocks");
  });
});
