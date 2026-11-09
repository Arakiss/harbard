import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// The composer box shows focus with its own ring; the textarea inside must not add the
// generic .hb :focus-visible outline as a second, inner blue frame.
const css = readFileSync(join(import.meta.dir, "workspace.css"), "utf8");

test("the composer textarea drops the generic focus-visible outline", () => {
  const rule = css.match(/\.hb-composer-box textarea:focus-visible\s*\{([^}]*)\}/);
  expect(rule).not.toBeNull();
  expect(rule![1]).toContain("outline: none");
});

test("the composer box still shows focus with its own ring", () => {
  expect(css).toMatch(/\.hb-composer-box:focus-within\s*\{[^}]*box-shadow/);
});
