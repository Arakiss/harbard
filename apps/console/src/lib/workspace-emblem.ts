/**
 * A workspace's emblem: a small flat composition drawn from its id, so it is
 * the same on every machine and survives a rename. Four big shapes on a 2×2
 * grid, in two colours over a third: nothing finer than a quarter of the
 * tile, because a rail tile is 42 pixels and fine patterns read as noise
 * there.
 */

export interface EmblemPalette {
  ground: string;
  one: string;
  two: string;
}

export const EMBLEM_PALETTES: readonly EmblemPalette[] = [
  { ground: "#f2c14e", one: "#d1495b", two: "#1f3a5f" },
  { ground: "#2a9d8f", one: "#f4e3b2", two: "#114b5f" },
  { ground: "#5b5bd6", one: "#ffb4a2", two: "#1e1b4b" },
  { ground: "#ef6f4c", one: "#ffe8c2", two: "#5a1e3d" },
  { ground: "#6dbe7b", one: "#fdf6d8", two: "#1e4d3b" },
  { ground: "#e889b5", one: "#fff1d6", two: "#5b1f6e" },
  { ground: "#6fa8dc", one: "#ffe08a", two: "#1b3a6b" },
];

export const EMBLEM_SIZE = 48;
const CELL = EMBLEM_SIZE / 2;

/** One filled shape, in the emblem's 48-unit box. */
export interface EmblemShape {
  d: string;
  color: string;
  transform: string;
}

export interface Emblem {
  palette: EmblemPalette;
  shapes: EmblemShape[];
}

/** FNV-1a: a stable 32-bit number from any text. */
export function seedOf(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** mulberry32: small, seedable, and the same in every engine. */
function generator(seed: number): () => number {
  let a = seed;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function pick<T>(list: readonly T[], r: number): T {
  return list[Math.min(list.length - 1, Math.floor(r * list.length))] as T;
}

const QUARTER = "M0 0H24A24 24 0 0 1 0 24Z";
const LEAF = "M0 0H12A12 12 0 0 1 24 12V24H12A12 12 0 0 1 0 12Z";
const HALF = "M0 0H24A12 12 0 0 1 0 0Z";
const DISC = "M12 3A9 9 0 1 1 12 21A9 9 0 1 1 12 3Z";

/** Quarters carry the composition; the rest are its accents. */
const SHAPES = [QUARTER, QUARTER, QUARTER, LEAF, LEAF, HALF, DISC] as const;

/** Where a workspace's palette falls when nothing else claims it. */
function paletteIndex(id: string): number {
  return Math.min(EMBLEM_PALETTES.length - 1, Math.floor(generator(seedOf(id))() * EMBLEM_PALETTES.length));
}

/**
 * The palette of each workspace, in the hub's order. Two workspaces never
 * share one while there are palettes to go round: side by side on the rail,
 * the same colours read as the same place.
 */
export function palettesFor(ids: readonly string[]): Map<string, number> {
  const out = new Map<string, number>();
  let used = new Set<number>();
  for (const id of ids) {
    if (used.size === EMBLEM_PALETTES.length) used = new Set();
    let at = paletteIndex(id);
    while (used.has(at)) at = (at + 1) % EMBLEM_PALETTES.length;
    used.add(at);
    out.set(id, at);
  }
  return out;
}

/** The emblem of workspace `id`; `palette` is its index from {@link palettesFor}. */
export function emblemOf(id: string, palette = paletteIndex(id)): Emblem {
  const random = generator(seedOf(id));
  random(); // The draw that picks the palette, so the drawing never depends on it.
  const colours = EMBLEM_PALETTES[palette % EMBLEM_PALETTES.length] as EmblemPalette;
  // Neighbours alternate, so the two colours always meet; which goes first is drawn.
  const flip = random() < 0.5;
  const shapes: EmblemShape[] = [];
  for (let cell = 0; cell < 4; cell++) {
    const x = (cell % 2) * CELL;
    const y = Math.floor(cell / 2) * CELL;
    const d = pick(SHAPES, random());
    const turn = Math.floor(random() * 4) * 90;
    const even = (cell + Math.floor(cell / 2)) % 2 === 0;
    shapes.push({
      d,
      color: even === flip ? colours.one : colours.two,
      transform: `translate(${x} ${y}) rotate(${turn} 12 12)`,
    });
  }
  return { palette: colours, shapes };
}
