import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseMark } from "./parse-mark";

// Inputs
// that change what is drawn must be refused by name, never drawn wrong.
const refused: [string, string, RegExp][] = [
  [
    "stroke-only ring (would draw a filled disc)",
    `<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" fill="none" stroke="currentColor" stroke-width="1"/></svg>`,
    /fill="none"|stroke/,
  ],
  [
    "ink given as style= (the dot would turn ink)",
    `<svg viewBox="0 0 10 10"><path style="fill:currentColor" d="M0 0h6v6H0z"/><circle cx="8" cy="8" r="2" fill="#2bac76"/></svg>`,
    /style/,
  ],
  [
    "single-quoted mask (the gap would be dropped)",
    `<svg viewBox="0 0 10 10"><defs><mask id="g"><rect width="10" height="10" fill="#fff"/><circle cx="8" cy="2" r="2"/></mask></defs><path mask='url(#g)' d="M0 0h10v10H0z"/></svg>`,
    /single-quoted/,
  ],
  ["opacity / fill-opacity (ignored)", `<svg viewBox="0 0 10 10"><path opacity="0.4" fill-opacity="0.5" d="M0 0h10v10H0z"/></svg>`, /opacity/],
  ["clip-path (ignored)", `<svg viewBox="0 0 10 10"><path clip-path="url(#c)" d="M0 0h10v10H0z"/></svg>`, /clip-path/],
  ["stroke on a filled shape", `<svg viewBox="0 0 10 10"><path stroke="#000" d="M0 0h10v10H0z"/></svg>`, /stroke/],
  ["fill none on its own", `<svg viewBox="0 0 10 10"><path fill="none" d="M0 0h10v10H0z"/></svg>`, /fill="none"/],
  [
    "a mask id with a space",
    `<svg viewBox="0 0 10 10"><defs><mask id="a gap"><rect width="10" height="10" fill="#fff"/></mask></defs><path mask="url(#a gap)" d="M0 0h10v10H0z"/></svg>`,
    /mask id/,
  ],
  [
    "two masks with one id",
    `<svg viewBox="0 0 10 10"><defs><mask id="g"><rect width="10" height="10" fill="#fff"/></mask><mask id="g"><rect width="5" height="5" fill="#fff"/></mask></defs><path mask="url(#g)" d="M0 0h10v10H0z"/></svg>`,
    /twice/,
  ],
];

describe("the mark parser refuses what it would draw wrong", () => {
  for (const [name, svg, reason] of refused) {
    test(`refuses ${name}`, () => {
      expect(() => parseMark(svg)).toThrow(reason);
    });
  }

  test("today's mark and a two-colour masked mark still parse", () => {
    const svg = readFileSync(join(import.meta.dir, "..", "..", "..", "..", "..", "assets", "brand", "mark.svg"), "utf8");
    // The burbuja: the bubble in ink with a masked gap, and the presence dot.
    expect(parseMark(svg).shapes.map((s) => [s.kind, s.tone, s.mask ?? null])).toEqual([
      ["path", "ink", "gap"],
      ["circle", "presence", null],
    ]);
    const two = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <defs><mask id="gap"><rect width="64" height="64" fill="#fff"/><circle cx="50" cy="14" r="10" fill="#000"/></mask></defs>
  <path mask="url(#gap)" fill="currentColor" fill-rule="evenodd" d="M8 8H56V56H8Z"/>
  <circle cx="50" cy="14" r="7" fill="#2bac76"/>
</svg>`;
    expect(parseMark(two).shapes.map((s) => s.tone)).toEqual(["ink", "presence"]);
  });
});
