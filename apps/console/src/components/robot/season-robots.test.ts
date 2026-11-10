import { describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { bodySrcSet, maskUrl } from "./identity";
import { ROBOT_SIZES } from "./robots";
import { loadSeasonRobots, parseSeasonModels } from "./season-robots";

const PUBLIC_ROBOTS = join(import.meta.dir, "..", "..", "..", "public", "robots");

describe("a season's robot list", () => {
  test("keeps known models with a screen inside the tile and drops the rest", () => {
    const models = parseSeasonModels([
      { id: "antenna", screen: { x: 0.2, y: 0.38, w: 0.4, h: 0.31 } },
      { id: "unknown-model", screen: { x: 0.1, y: 0.1, w: 0.5, h: 0.5 } },
      { id: "dome", screen: { x: 0.1, y: 0.1, w: 1.5, h: 0.5 } },
      { id: "../dials", screen: { x: 0.1, y: 0.1, w: 0.5, h: 0.5 } },
      { id: "radio" },
      "teapot",
    ]);
    expect([...models.keys()]).toEqual(["antenna"]);
    expect(models.get("antenna")?.screen).toEqual({ x: 0.2, y: 0.38, w: 0.4, h: 0.31 });
  });

  test("is empty when the file is not a list", () => {
    expect(parseSeasonModels({ antenna: {} }).size).toBe(0);
    expect(parseSeasonModels(null).size).toBe(0);
  });

  test("is read once per season, and a season without art reads as no variants", async () => {
    const asked: string[] = [];
    const missing = (async (url: string) => {
      asked.push(url);
      return new Response("not found", { status: 404 });
    }) as typeof fetch;
    expect((await loadSeasonRobots("no-art-season", missing)).size).toBe(0);
    expect((await loadSeasonRobots("no-art-season", missing)).size).toBe(0);
    expect(asked).toEqual(["/robots/no-art-season/models.json"]);
    expect((await loadSeasonRobots("../etc", missing)).size).toBe(0);
    expect(asked).toHaveLength(1);
  });

  test("a variant's body and mask come from the season's folder", () => {
    expect(bodySrcSet("antenna", "/robots/christmas")).toStartWith("/robots/christmas/antenna-36.webp 36w");
    expect(maskUrl("antenna", 36, "/robots/christmas")).toBe("/robots/christmas/antenna-mask-72.png");
    expect(maskUrl("antenna", 36)).toBe("/robots/antenna-mask-72.png");
  });
});

/** Art dropped into `public/robots/<season>/` must be whole: every size of every listed model. */
describe("the seasons' art on disk", () => {
  const seasons = readdirSync(PUBLIC_ROBOTS, { withFileTypes: true }).filter((d) => d.isDirectory() && existsSync(join(PUBLIC_ROBOTS, d.name, "models.json")));
  for (const season of seasons) {
    test(`${season.name}: every listed model has its body and mask at every size`, () => {
      const dir = join(PUBLIC_ROBOTS, season.name);
      const raw: unknown = JSON.parse(readFileSync(join(dir, "models.json"), "utf8"));
      const models = parseSeasonModels(raw);
      expect(models.size).toBe(Array.isArray(raw) ? raw.length : -1);
      const missing = [...models.keys()].flatMap((id) => ROBOT_SIZES.flatMap((s) => [`${id}-${s}.webp`, `${id}-mask-${s}.png`])).filter((f) => !existsSync(join(dir, f)));
      expect(missing).toEqual([]);
    });
  }
});
