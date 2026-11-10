"use client";

import * as React from "react";
import { ROBOT_MODELS, type RobotModel } from "./robots";

/**
 * A season's robots: `/robots/<season>/models.json` lists the models that
 * have a variant, with each variant's screen box, in the shape of
 * ROBOT_MODELS; `<id>-<size>.webp` and `<id>-mask-<size>.png` sit beside
 * it for every size in ROBOT_SIZES. While the season is active a robot whose
 * model has a variant wears it, body, mask and screen; every other robot
 * stays as it is. The list is read when the season starts, so adding a
 * season's art needs no code.
 */
export interface SeasonRobots {
  /** The folder under `/robots/`. */
  dir: string;
  models: ReadonlyMap<string, RobotModel>;
}

const KNOWN = new Set(ROBOT_MODELS.map((m) => m.id));

function fraction(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v) && v >= 0 && v <= 1;
}

/** The variants in `raw` that name a known model and a screen inside the tile; the rest are dropped. */
export function parseSeasonModels(raw: unknown): Map<string, RobotModel> {
  const out = new Map<string, RobotModel>();
  if (!Array.isArray(raw)) return out;
  for (const item of raw) {
    if (typeof item !== "object" || item === null) continue;
    // Untrusted JSON dropped in with the art: read, then checked field by field.
    const { id, screen } = item as { id?: unknown; screen?: unknown };
    if (typeof id !== "string" || !KNOWN.has(id) || typeof screen !== "object" || screen === null) continue;
    const { x, y, w, h } = screen as Record<string, unknown>;
    if (!fraction(x) || !fraction(y) || !fraction(w) || !fraction(h)) continue;
    out.set(id, { id, screen: { x, y, w, h } });
  }
  return out;
}

/** One read per season and page; a season without art answers an empty map. */
const LOADS = new Map<string, Promise<Map<string, RobotModel>>>();

export function loadSeasonRobots(season: string, fetcher: typeof fetch = fetch): Promise<Map<string, RobotModel>> {
  if (!/^[a-z0-9-]+$/.test(season)) return Promise.resolve(new Map());
  let load = LOADS.get(season);
  if (!load) {
    load = fetcher(`/robots/${season}/models.json`, { cache: "no-store" })
      .then((res) => (res.ok ? res.json() : []))
      .then(parseSeasonModels, () => new Map<string, RobotModel>());
    LOADS.set(season, load);
  }
  return load;
}

const SeasonRobotsContext = React.createContext<SeasonRobots | null>(null);

/** Gives every robot below the active season's variants; none when `season` is null. */
export function SeasonRobotsProvider({ season, children }: { season: string | null; children: React.ReactNode }) {
  const [value, setValue] = React.useState<SeasonRobots | null>(null);
  React.useEffect(() => {
    if (!season) {
      setValue(null);
      return;
    }
    let alive = true;
    void loadSeasonRobots(season).then((models) => {
      if (alive) setValue(models.size > 0 ? { dir: season, models } : null);
    });
    return () => {
      alive = false;
    };
  }, [season]);
  return <SeasonRobotsContext.Provider value={value}>{children}</SeasonRobotsContext.Provider>;
}

/** The season's variant of `model`, and the folder its art is in, or null. */
export function useSeasonVariant(model: string): { dir: string; model: RobotModel } | null {
  const season = React.useContext(SeasonRobotsContext);
  const variant = season?.models.get(model);
  return season && variant ? { dir: season.dir, model: variant } : null;
}
