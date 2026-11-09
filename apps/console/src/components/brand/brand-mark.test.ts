import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import { BrandMark } from "./brand-mark";
import { MARK } from "./mark-geometry";
import { parseMark } from "./parse-mark";

// The rail tile draws the mark from mark-geometry.ts, which
// scripts/brand-geometry.sh writes from assets/brand/mark.svg. A new logo is
// one SVG change plus scripts/render-brand.sh; this fails if the second step
// was skipped.
const svg = readFileSync(join(import.meta.dir, "..", "..", "..", "..", "..", "assets", "brand", "mark.svg"), "utf8");

// A synthetic two-colour mark of the shape the next logo takes: an ink body
// with a masked gap, and a presence dot in its own colour.
const TWO_TONE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <defs><mask id="gap"><rect width="64" height="64" fill="#fff"/><circle cx="50" cy="14" r="10" fill="#000"/></mask></defs>
  <path mask="url(#gap)" fill="currentColor" fill-rule="evenodd" d="M8 8H56V56H8Z"/>
  <circle cx="50" cy="14" r="7" fill="#2bac76"/>
</svg>`;

describe("the console mark is the brand SVG's geometry", () => {
  test("mark-geometry.ts is what the parser reads from mark.svg", () => {
    expect(MARK).toEqual(parseMark(svg));
  });

  test("nothing else in the console hard-codes the geometry", () => {
    const shell = readFileSync(join(import.meta.dir, "..", "workspace", "shell.tsx"), "utf8");
    for (const s of MARK.shapes) {
      if (s.kind !== "path") continue;
      expect(shell).not.toContain(s.d);
    }
    expect(shell).toContain("<BrandMark");
  });
});

describe("the slot takes a two-colour, masked mark", () => {
  const two = parseMark(TWO_TONE);

  test("ink follows currentColor, the other fill is the presence dot, the mask is kept", () => {
    expect(two.shapes.map((s) => [s.kind, s.tone, s.mask ?? null])).toEqual([
      ["path", "ink", "gap"],
      ["circle", "presence", null],
    ]);
    expect(two.masks[0]!.shapes.map((s) => s.keep)).toEqual([true, false]);
    expect(two.shapes[0]).toMatchObject({ fillRule: "evenodd" });
  });

  test("a single-colour mark is all ink, whatever its fill", () => {
    const single = '<svg viewBox="0 0 34 40"><path fill="#ff8a52" d="M2 1h10v22h7v10h-7v6H2z"/></svg>';
    expect(parseMark(single).shapes.every((s) => s.tone === "ink")).toBe(true);
  });

  test("it renders with a scoped mask and the dot on --hb-green", () => {
    const html = renderToString(h("div", null, h(BrandMark, { mark: two }), h(BrandMark, { mark: two })));
    const ids = [...html.matchAll(/<mask id="([^"]+)"/g)].map((m) => m[1]);
    expect(ids.length).toBe(2);
    expect(new Set(ids).size).toBe(2);
    expect(html).toContain(`mask="url(#${ids[0]})"`);
    expect(html).toContain("fill:var(--hb-mark-dot, var(--hb-green))");
    expect(html).toContain('fill-rule="evenodd"');
  });

  test("what the slot cannot draw is refused by name", () => {
    expect(() => parseMark('<svg viewBox="0 0 8 8"><g><path d="M0 0h8v8z"/></g></svg>')).toThrow("<g>");
    expect(() => parseMark('<svg viewBox="0 0 8 8"><path transform="scale(2)" d="M0 0h8v8z"/></svg>')).toThrow("transform");
    expect(() => parseMark('<svg viewBox="0 0 8 8"><path mask="url(#nope)" d="M0 0h8v8z"/></svg>')).toThrow("#nope");
  });
});
