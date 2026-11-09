import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Every colour in the shell goes through a `--hb-*` token (criterion 2 of
// the themes spec): the token block in workspace.css is the only place a colour
// literal may sit, and the workspace components carry none, in a style, a class
// or a prop. A theme can only recolour what reads a token.
const workspaceDir = join(import.meta.dir, "..", "components", "workspace");
const brand = join(import.meta.dir, "..", "components", "brand");
const robot = join(import.meta.dir, "..", "components", "robot");
const strip = join(import.meta.dir, "..", "components", "strip");
/** Set up harbard, in the shell's frame since profile-9. */
const setup = join(import.meta.dir, "..", "components", "setup");
const css = readFileSync(join(workspaceDir, "workspace.css"), "utf8");
/** Stylesheets with no token block of their own: every colour is a var(). */
const OTHER_CSS = [join(robot, "robot.css"), join(brand, "mark.css"), join(strip, "strip.css"), join(workspaceDir, "direct.css"), join(workspaceDir, "rooms.css")];

const HEX = /(?<![\w&-])#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})\b/g;
const FUNCS = /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\(/gi;
// CSS named colours, minus the keywords that are not a colour choice.
const NAMED =
  "aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen".split(
    " ",
  );

/** The CSS with the token blocks cut out, and the blocks themselves. */
function split(text: string): { outside: string; blocks: string[] } {
  const blocks: string[] = [];
  const outside = text.replace(/\/\* tokens:start[\s\S]*?\/\* tokens:end \*\//g, (block) => {
    blocks.push(block);
    return "";
  });
  return { outside, blocks };
}

/** Values of every declaration (`prop: value;`, or the last one before
 * `}` without its semicolon), comments removed. */
function values(text: string): string[] {
  return [...text.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/(?:^|[{;\s])[-a-z]+\s*:\s*([^;{}]+)(?=[;}])/g)].map((m) => m[1] ?? "");
}

function literals(text: string): string[] {
  const found = [...(text.match(HEX) ?? []), ...(text.match(FUNCS) ?? [])];
  for (const v of values(text)) {
    const bare = v.toLowerCase().replace(/var\(--[a-z0-9-]+\)/g, " ").replace(/--[a-z0-9-]+/g, " ");
    for (const word of bare.split(/[^a-z]+/)) if (NAMED.includes(word)) found.push(word);
  }
  return found;
}

const PALETTE = "slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose";
/** Tailwind colour utilities: `bg-red-500`, `text-white`, `border-x-sky-300/50`. */
const TAILWIND = new RegExp(
  `\\b(?:text|bg|border|fill|stroke|ring|ring-offset|outline|shadow|from|to|via|decoration|accent|caret|divide|placeholder)(?:-[trblxyse])?-(?:(?:${PALETTE})-\\d{2,3}|white|black)\\b`,
  "g",
);
const QUOTED = new RegExp(`(?<!data-[a-z-]+=)["'\`](${NAMED.join("|")})["'\`]`, "gi");

/**
 * Quoted colour names a component may hold, with the reason: they are not
 * paint. Anything else quoted that names a colour is a literal.
 */
const ALLOWED_QUOTED: Record<string, string[]> = {
  // A mask's luminance: white keeps, black cuts (BrandMark's <mask>).
  "brand-mark.tsx": ["white", "black"],
  // Reading a mask fill out of the brand SVG.
  "parse-mark.ts": ["white"],
  // The favicon's <mask> luminance: white keeps, black cuts.
  "favicon.ts": ["white", "black"],
};

/** Colour literals in a component's source, comments removed: hex, colour
 * functions, Tailwind colour utilities, and quoted colour names (a style
 * value, an SVG fill or stroke, a class). */
function componentLiterals(name: string, text: string): string[] {
  const source = text.replace(/\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, "");
  const found = [...(source.match(HEX) ?? []), ...(source.match(FUNCS) ?? []), ...(source.match(TAILWIND) ?? [])];
  const allowed = ALLOWED_QUOTED[name] ?? [];
  for (const m of source.matchAll(QUOTED)) {
    const word = m[1]!.toLowerCase();
    if (!allowed.includes(word)) found.push(m[0]);
  }
  return found;
}

describe("colour literals live only in the token block", () => {
  test("workspace.css has one token block and it holds the colours", () => {
    const { blocks } = split(css);
    expect(blocks.length).toBe(1);
    expect(literals(blocks[0]!).length).toBeGreaterThan(50);
  });

  test("no colour literal in workspace.css outside the token block", () => {
    expect(literals(split(css).outside)).toEqual([]);
  });

  test("no colour literal in the robot, logo and Strip stylesheets", () => {
    for (const file of OTHER_CSS) {
      const text = readFileSync(file, "utf8");
      expect(split(text).blocks.length).toBe(0);
      expect({ file, found: literals(text) }).toEqual({ file, found: [] });
    }
  });

  test("the check catches a literal, a function and a named colour", () => {
    const sneaky = ".hb-x {\n  color: #fff;\n  background: rgba(0, 0, 0, 0.1);\n  border-color: white;\n}\n";
    expect(literals(split(css + sneaky).outside)).toEqual(["#fff", "rgba(", "white"]);
  });

  test("no colour literal in the workspace, brand, robot and Strip components", () => {
    const offenders: string[] = [];
    for (const dir of [workspaceDir, brand, robot, strip, setup]) {
      for (const name of readdirSync(dir).filter((n) => /\.(tsx|ts)$/.test(n) && !n.includes(".test."))) {
        for (const hit of componentLiterals(name, readFileSync(join(dir, name), "utf8"))) offenders.push(`${name}: ${hit}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  // Ported from the review (M2): nine literals the first version let
  // through, in a component and in workspace.css.
  test("the check catches the review's nine injected literals", () => {
    const probe = `export function Probe() {
  return (
    <div className="bg-red-500 text-zinc-700 border-sky-300" style={{ color: "white", background: "tomato" }}>
      <svg fill="black" stroke="crimson" />
    </div>
  );
}`;
    const rules = `.hb-probe-a { color: white }\n.hb-probe-b { border: 1px solid var(--hb-line); background: tomato }\n`;
    const found = [...componentLiterals("probe.tsx", probe), ...literals(split(css + rules).outside)];
    expect(found.length).toBe(9);
  });

  test("the avatar tones are tokens too", () => {
    const workspace = readFileSync(join(import.meta.dir, "workspace.ts"), "utf8");
    const tones = workspace.slice(workspace.indexOf("export const TONES"), workspace.indexOf("export function tone"));
    expect(tones.match(HEX)).toBeNull();
  });
});
