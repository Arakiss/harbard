import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";

// Structure checks for the shell, read from the sources: what must hold on
// every page without a browser (review lows 5).
const app = join(import.meta.dir, "..", "app");
const css = readFileSync(join(import.meta.dir, "..", "components", "workspace", "workspace.css"), "utf8");

function pages(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "api" ? [] : pages(path);
    return name === "page.tsx" ? [path] : [];
  });
}

/** The layouts that wrap a page, from its own folder up to the app root. */
function layoutsOf(page: string): string[] {
  const out: string[] = [];
  for (let dir = dirname(page); dir.startsWith(app); dir = dirname(dir)) {
    const layout = join(dir, "layout.tsx");
    if (existsSync(layout)) out.push(readFileSync(layout, "utf8"));
    if (dir === app) break;
  }
  return out;
}

function rule(selector: string): string {
  const at = css.indexOf(`${selector} {`);
  return at < 0 ? "" : css.slice(at, css.indexOf("}", at));
}

describe("every page has the rail", () => {
  test("each page sits under the shell or the More frame, except the strip", () => {
    const missing = pages(app)
      .map((p) => relative(app, p))
      .filter((rel) => !rel.includes("pulse"))
      .filter((rel) => !layoutsOf(join(app, rel)).some((l) => /<Shell>|<MoreFrame>/.test(l)));
    expect(missing).toEqual([]);
  });
});

describe("the stream reads cleanly", () => {
  test("the day separator does not float over the first line", () => {
    expect(rule(".hb-day")).not.toContain("sticky");
  });

  test("a file card never overflows the thread pane", () => {
    expect(rule(".hb-pane .hb-file")).toContain("min-width: 0");
    expect(rule(".hb-pane .hb-file")).toContain("max-width: 100%");
    expect(rule(".hb-file")).toContain("overflow: hidden");
  });
});

describe("feedback follows the operator's state", () => {
  const shell = readFileSync(join(import.meta.dir, "..", "components", "workspace", "shell.tsx"), "utf8");
  const layout = readFileSync(join(app, "(workspace)", "layout.tsx"), "utf8");

  test("the shell mounts FeedbackProvider inside the profile, with the state in force", () => {
    expect(shell).toMatch(/<FeedbackProvider operatorState=\{stateOf\(profile\?\.view \?\? null, Date\.now\(\)\)\.id\}>/);
    expect(shell.indexOf("<ProfileProvider")).toBeLessThan(shell.indexOf("<FeedbackProvider"));
    expect(layout).not.toContain("<FeedbackProvider");
  });
});
