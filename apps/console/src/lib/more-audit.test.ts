import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { legacyRedirect, moreRedirect, moreTools } from "./workspace";

// The audit of "More": the previous console's views were removed with their
// routes, and what was kept is a clear entry of "More tools". Read
// from the app folder (and from the build's route manifest when there is
// one), so a removed view cannot come back unnoticed.
const console_ = join(import.meta.dir, "..", "..");
const app = join(console_, "src", "app");

/** Every page's URL, route groups `(…)` left out. */
function routes(dir = app): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "api" ? [] : routes(path);
    if (name !== "page.tsx") return [];
    const segments = relative(app, dir)
      .split(sep)
      .filter((s) => s && !(s.startsWith("(") && s.endsWith(")")));
    return [`/${segments.join("/")}`];
  });
}

const REMOVED = ["/more", "/team"];

/** The page of a route, by its URL, or null. */
function pageOf(route: string): string | null {
  const find = (dir: string): string | null => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory() && name !== "api") {
        const hit = find(path);
        if (hit) return hit;
      } else if (name === "page.tsx") {
        const segments = relative(app, dir)
          .split(sep)
          .filter((x) => x && !(x.startsWith("(") && x.endsWith(")")));
        if (`/${segments.join("/")}` === route) return path;
      }
    }
    return null;
  };
  return find(app);
}

describe("the views removed from More", () => {
  test("only redirect: an old bookmark lands somewhere useful, never on the old view", () => {
    for (const r of REMOVED) {
      const page = pageOf(r);
      expect(page, `${r} has a redirecting page`).not.toBeNull();
      const text = readFileSync(page!, "utf8");
      expect(text).toContain("redirect(");
      // No component rendered: a redirect and nothing else (types like Promise<Record<…>> are fine).
      expect(text).not.toMatch(/<[A-Z][A-Za-z.]*[\s/>]/);
    }
    expect(existsSync(join(app, "(bus)"))).toBe(false);
  });

  test("are not a view in the built app either, when there is a build", () => {
    const manifest = join(console_, ".next", "app-path-routes-manifest.json");
    if (!existsSync(manifest)) return;
    const built = Object.values(JSON.parse(readFileSync(manifest, "utf8")) as Record<string, string>);
    for (const m of moreTools()) expect(built).toContain(m.href);
  });

  test("leave nothing that links to them", () => {
    const src = join(console_, "src");
    const hits: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const path = join(dir, name);
        if (statSync(path).isDirectory()) walk(path);
        else if (/\.(ts|tsx)$/.test(name) && !name.endsWith(".test.ts")) {
          const text = readFileSync(path, "utf8");
          // A link, a push or a redirect to one of them; serve's `/api/v1/team` is not a page.
          if (/(?:href\s*[:=]\s*\{?|push\(|replace\(|redirect\()\s*["'`]\/(?:more|team)\b/.test(text)) hits.push(relative(src, path));
        }
      }
    };
    walk(src);
    expect(hits).toEqual([]);
  });

  test("old deep links land where their content lives now", () => {
    expect(legacyRedirect({ setup: "1" })).toBe("/setup");
    expect(legacyRedirect({ shell: "desktop", setup: "1" })).toBe("/setup");
    expect(legacyRedirect({ view: "inbox", note: "n1" })).toBe("/activity?sel=n1");
    expect(legacyRedirect({ view: "inbox" })).toBe("/activity");
    expect(moreRedirect({})).toBe("/");
    expect(moreRedirect({ view: "inbox", note: "card:x" })).toBe("/activity?sel=card%3Ax");
    // The old History view has a home again: History.
    expect(legacyRedirect({ view: "history" })).toBe("/history");
    expect(moreRedirect({ view: "history" })).toBe("/history");
    expect(legacyRedirect({ shell: "desktop" })).toBeNull();
    expect(legacyRedirect({})).toBeNull();
  });
});

describe("More tools", () => {
  test("lists only what was kept, each with a title, a one-line description and a page", () => {
    expect(moreTools().map((m) => m.id)).toEqual(["strip", "history", "setup"]);
    const all = routes();
    for (const m of moreTools()) {
      expect(m.title.length).toBeGreaterThan(2);
      expect(m.description.length).toBeGreaterThan(20);
      expect(m.description).not.toContain("\n");
      expect(all).toContain(m.href);
    }
  });

  test("each entry has its icon and the button says what it is", () => {
    const menu = readFileSync(join(console_, "src", "components", "workspace", "more-tools.tsx"), "utf8");
    for (const m of moreTools()) expect(menu).toMatch(new RegExp(`\\b${m.id}:\\s*[A-Z]\\w+`));
    expect(menu).toContain("More tools");
  });
});
