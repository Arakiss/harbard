import { ROBOT_MODELS, ROBOT_SIZES, type RobotModel } from "./robots";

/**
 * A robot per agent, from its name: the model, its eye light and its tile,
 * picked by FNV-1a over code points, so the
 * same name is the same robot in every view, after every reload and on
 * every machine. 12 models × 4 lights × 4 tiles.
 */
const GLOWS = ["warm", "ice", "mint", "lilac"] as const;
export type Glow = (typeof GLOWS)[number] | "amber";

function nameHash(name: string): number {
  let x = 2166136261;
  for (const c of name) {
    x ^= c.codePointAt(0)!;
    x = Math.imul(x, 16777619);
  }
  return x >>> 0;
}

export interface RobotIdentity {
  model: RobotModel;
  glow: (typeof GLOWS)[number];
  /** 1..4: `--hb-robot-tile-<n>`. */
  tile: number;
  /** Per-agent timing, so idle breathing and blinking never march in step. */
  blinkS: number;
  blinkDelayS: number;
  breatheDelayS: number;
}

export function robotFor(name: string): RobotIdentity {
  const h = nameHash(name);
  return {
    model: ROBOT_MODELS[h % ROBOT_MODELS.length]!,
    glow: GLOWS[(h >>> 5) % GLOWS.length]!,
    tile: ((h >>> 9) % 4) + 1,
    blinkS: 4.4 + ((h >>> 13) % 21) / 10,
    blinkDelayS: -(((h >>> 18) % 50) / 10),
    breatheDelayS: -(((h >>> 23) % 46) / 10),
  };
}

/** The glow token of a light; blocked and waiting faces turn amber. */
export function glowVar(glow: Glow): string {
  return `var(--hb-robot-glow-${glow})`;
}

/** The smallest rendered size that covers `css` pixels at 2x. */
export function assetSize(css: number): number {
  return ROBOT_SIZES.find((s) => s >= css * 2) ?? ROBOT_SIZES[ROBOT_SIZES.length - 1]!;
}

export function bodySrcSet(model: string): string {
  return ROBOT_SIZES.map((s) => `/robots/${model}-${s}.webp ${s}w`).join(", ");
}

export function maskUrl(model: string, css: number): string {
  return `/robots/${model}-mask-${assetSize(css)}.png`;
}
