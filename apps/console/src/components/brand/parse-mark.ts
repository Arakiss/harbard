/**
 * Read a brand mark SVG into the shapes `BrandMark` draws.
 *
 * The mark is a small, flat SVG: filled `path`, `circle` and `rect`
 * elements, and optional `<mask>`s in `<defs>` that cut gaps (white keeps,
 * black cuts). Each element has an allow-list of attributes, in double
 * quotes; anything else (groups, transforms, strokes, `fill="none"`,
 * `style`, opacity, clip paths, gradients, text, single-quoted attributes,
 * a mask id that is not a plain name or appears twice) is refused by name,
 * so a new logo that needs more fails loudly at `scripts/brand-geometry.sh`
 * instead of rendering wrong.
 *
 * Paint: a shape filled with `currentColor` is ink and follows its
 * container's colour (the rail tile follows the theme). Any other fill is
 * the presence tone (`--hb-mark-dot`, by default `--hb-green`). A mark with
 * no `currentColor` at all is single-colour, and every shape is ink.
 */

export type MarkTone = "ink" | "presence";

export type MarkGeometry =
  | { kind: "path"; d: string; fillRule?: "evenodd" | "nonzero" }
  | { kind: "circle"; cx: number; cy: number; r: number }
  | { kind: "rect"; x: number; y: number; width: number; height: number };

export type MarkShape = MarkGeometry & { tone: MarkTone; mask?: string };

/** A mask's shapes: `keep` true is white (shown), false is black (cut). */
export interface MarkMask {
  id: string;
  shapes: (MarkGeometry & { keep: boolean })[];
}

export interface Mark {
  viewBox: string;
  masks: MarkMask[];
  shapes: MarkShape[];
}

/** The attributes each element may carry; anything else is refused. */
const ATTRS: Record<string, ReadonlySet<string>> = {
  svg: new Set(["xmlns", "xmlns:xlink", "viewBox", "width", "height", "fill"]),
  defs: new Set(),
  mask: new Set(["id"]),
  path: new Set(["d", "fill", "fill-rule", "mask"]),
  circle: new Set(["cx", "cy", "r", "fill", "mask"]),
  rect: new Set(["x", "y", "width", "height", "fill", "mask"]),
};

const MASK_ID = /^[A-Za-z][A-Za-z0-9_-]*$/;

function attrs(name: string, tag: string): Record<string, string> {
  const single = /([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*'/.exec(tag);
  if (single) throw new Error(`brand mark: single-quoted attribute ${single[1]} on <${name}>; use double quotes`);
  const out: Record<string, string> = {};
  const rest = tag.replace(/([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*"([^"]*)"/g, (_, k: string, v: string) => {
    if (k in out) throw new Error(`brand mark: ${k} twice on <${name}>`);
    out[k] = v;
    return " ";
  });
  const stray = rest.replace(/\//g, " ").trim();
  if (stray) throw new Error(`brand mark: cannot read "${stray}" on <${name}>`);
  const allowed = ATTRS[name]!;
  for (const k of Object.keys(out)) {
    if (!allowed.has(k)) throw new Error(`brand mark: ${k} on <${name}> is not supported (allowed: ${[...allowed].join(", ") || "none"})`);
  }
  if (name !== "svg" && out.fill?.trim() === "none") {
    throw new Error(`brand mark: fill="none" on <${name}> is not supported; every shape is filled`);
  }
  return out;
}

function num(a: Record<string, string>, k: string, fallback = 0): number {
  const v = a[k];
  if (v === undefined) return fallback;
  const n = Number(v);
  if (!Number.isFinite(n)) throw new Error(`brand mark: ${k}="${v}" is not a number`);
  return n;
}

function geometry(name: string, a: Record<string, string>): MarkGeometry {
  if (name === "path") {
    if (!a.d) throw new Error("brand mark: a path without d");
    const rule = a["fill-rule"];
    return rule === "evenodd" || rule === "nonzero" ? { kind: "path", d: a.d, fillRule: rule } : { kind: "path", d: a.d };
  }
  if (name === "circle") return { kind: "circle", cx: num(a, "cx"), cy: num(a, "cy"), r: num(a, "r") };
  return { kind: "rect", x: num(a, "x"), y: num(a, "y"), width: num(a, "width"), height: num(a, "height") };
}

/** A mask shape keeps what is under it when its fill is light (luminance). */
function isKeep(fill: string | undefined): boolean {
  const f = (fill ?? "").trim().toLowerCase();
  if (f === "white") return true;
  if (!f.startsWith("#")) return false;
  const hex = f.slice(1);
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex.slice(0, 6);
  if (!/^[0-9a-f]{6}$/.test(full)) return false;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16));
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b! > 127;
}

export function parseMark(svg: string): Mark {
  const text = svg.replace(/<!--[\s\S]*?-->/g, "").replace(/<\?xml[\s\S]*?\?>/g, "");
  const viewBox = /<svg\b[^>]*\bviewBox="([^"]+)"/.exec(text)?.[1];
  if (!viewBox) throw new Error("brand mark: the <svg> has no viewBox");
  const masks: MarkMask[] = [];
  const raw: { geometry: MarkGeometry; fill?: string; mask?: string }[] = [];
  let inMask: MarkMask | null = null;
  let svgFill: string | undefined;
  for (const m of text.matchAll(/<(\/?)([a-zA-Z][-a-zA-Z0-9]*)\b([^>]*?)(\/?)>/g)) {
    const [, closing, name, body, selfClosing] = m as unknown as [string, string, string, string, string];
    if (!(name in ATTRS)) throw new Error(`brand mark: <${name}> is not supported (path, circle, rect, mask)`);
    const a = closing ? {} : attrs(name, body);
    if (name === "svg" && !closing) svgFill = a.fill?.trim();
    if (name === "mask") {
      if (closing) inMask = null;
      else {
        if (!a.id) throw new Error("brand mark: a <mask> without id");
        if (!MASK_ID.test(a.id)) throw new Error(`brand mark: mask id "${a.id}" must be a plain name (letters, digits, - and _)`);
        if (masks.some((k) => k.id === a.id)) throw new Error(`brand mark: mask id "${a.id}" appears twice`);
        inMask = { id: a.id, shapes: [] };
        masks.push(inMask);
        if (selfClosing) inMask = null;
      }
      continue;
    }
    if (closing || name === "svg" || name === "defs") continue;
    const g = geometry(name, a);
    if (inMask) {
      inMask.shapes.push({ ...g, keep: isKeep(a.fill) });
      continue;
    }
    let mask: string | undefined;
    if (a.mask !== undefined) {
      mask = /^url\(#([A-Za-z][A-Za-z0-9_-]*)\)$/.exec(a.mask.trim())?.[1];
      if (!mask) throw new Error(`brand mark: mask="${a.mask}" on <${name}> must be url(#id)`);
    }
    if (a.fill === undefined && svgFill === "none") {
      throw new Error(`brand mark: <${name}> has no fill under <svg fill="none">, so it would not be drawn`);
    }
    raw.push({ geometry: g, fill: a.fill, mask });
  }
  if (raw.length === 0) throw new Error("brand mark: no shapes");
  const twoTone = raw.some((r) => r.fill?.trim() === "currentColor");
  const shapes: MarkShape[] = raw.map((r) => {
    const tone: MarkTone = !twoTone || r.fill?.trim() === "currentColor" ? "ink" : "presence";
    const shape: MarkShape = { ...r.geometry, tone };
    if (r.mask) {
      if (!masks.some((k) => k.id === r.mask)) throw new Error(`brand mark: mask #${r.mask} is not defined`);
      shape.mask = r.mask;
    }
    return shape;
  });
  return { viewBox, masks, shapes };
}
