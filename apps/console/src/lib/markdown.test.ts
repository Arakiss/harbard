import { describe, expect, test } from "bun:test";
import { diffLines, inline, parse, preview, safeHref } from "./markdown";

describe("the Markdown reader returns data, never HTML", () => {
  test("blocks", () => {
    const blocks = parse("# T\n\nuno\ndos\n\n- a\n- b\n\n1. x\n\n> cita\n\n```sh\nls <x>\n```\n\n---");
    expect(blocks.map((b) => b.t)).toEqual(["heading", "paragraph", "list", "list", "quote", "code", "rule"]);
    expect(blocks[1]).toEqual({ t: "paragraph", c: [{ t: "text", v: "uno dos" }] });
    expect(blocks[5]).toEqual({ t: "code", lang: "sh", v: "ls <x>" });
  });

  test("raw HTML stays text, links only for http(s) and mailto, images are their alt", () => {
    expect(inline("<script>x</script>")).toEqual([{ t: "text", v: "<script>x</script>" }]);
    expect(inline("[a](javascript:alert(1))")[0]).toEqual({ t: "link", href: null, c: [{ t: "text", v: "a" }] });
    expect(safeHref("https://x.es")).toBe("https://x.es");
    expect(safeHref("data:text/html,x")).toBeNull();
    expect(inline("![capture](https://x/p.png)")).toEqual([{ t: "text", v: "capture" }]);
  });

  test("diff lines and previews", () => {
    expect(diffLines("+a\n-b\n@@ x\n c").map((l) => l.kind)).toEqual(["add", "del", "hunk", "context"]);
    expect(preview("1\n2\n3\n", 2)).toEqual({ text: "1\n2", more: 1 });
  });
});

describe("openers with no closer stay linear", () => {
  test("390,000 characters of unclosed brackets parse in a blink", () => {
    // The reviewer's shape: openers that never close.
    const text = "[a ".repeat(130_000);
    const t0 = performance.now();
    parse(text);
    const ms = performance.now() - t0;
    expect(ms).toBeLessThan(100);
  });

  test("links and code still close where they should", () => {
    expect(inline("[a](https://x.es) y `b` y [c")).toEqual([
      { t: "link", href: "https://x.es", c: [{ t: "text", v: "a" }] },
      { t: "text", v: " y " },
      { t: "code", v: "b" },
      { t: "text", v: " y [c" },
    ]);
  });
});
