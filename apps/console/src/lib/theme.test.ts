import { describe, expect, test } from "bun:test";
import { appliedOf, describeRefusal, rootAttrs, slugify, themeCss, validThemeName, type ThemeEntry } from "./theme";
import { STATE, jade } from "./theme.fixture";

describe("the theme stylesheet", () => {
  const css = themeCss(STATE, jade);

  test("light on the root, dark under dark and under the system query", () => {
    expect(css).toStartWith(".hb{");
    expect(css).toContain("--hb-frame:#0d3b2e;");
    expect(css).toContain("--hb-rail-active:rgba(255, 255, 255, 0.22);");
    expect(css).toContain('.hb[data-hb-mode="dark"]{');
    expect(css).toContain('@media (prefers-color-scheme: dark){.hb[data-hb-mode="system"]{');
    const system = css.slice(css.indexOf("@media"));
    expect(system).toContain("--hb-frame:#0b3127;");
    expect(system).toContain("--hb-surface:#1a1d21;");
    expect(system).toContain("--hb-color-scheme:dark;");
  });

  test("a value that could close the rule or the element is dropped", () => {
    const evil: ThemeEntry = { ...jade, light: { frame: "red;}</style><script>x()</script>", "a}b": "#fff", accent: "#0f7a56" } };
    const out = themeCss(STATE, evil);
    expect(out).not.toContain("</style>");
    expect(out).not.toContain("script");
    expect(out).not.toContain("a}b");
    expect(out).toContain("--hb-accent:#0f7a56;");
  });
});

describe("what is applied", () => {
  test("the stored theme and mode, or light Aubergine without serve", () => {
    expect(rootAttrs(appliedOf(STATE))).toEqual({ "data-hb-theme": "jade", "data-hb-mode": "system" });
    expect(rootAttrs(appliedOf(null))).toEqual({ "data-hb-theme": "aubergine", "data-hb-mode": "light" });
  });

  test("a refusal reads like the CLI's", () => {
    expect(describeRefusal({ file: "/h/themes/x.toml", line: 3, key: "sidebar_hover", code: "unknown_key", message: "not a key", fix: "remove it" })).toBe(
      "/h/themes/x.toml:3: `sidebar_hover`: not a key. Fix: remove it",
    );
  });

  test("names the picker saves are names serve accepts", () => {
    expect(slugify("Mi Tema Ñandú")).toBe("mi-tema-nandu");
    expect(validThemeName(slugify("Mi Tema"))).toBe(true);
    expect(validThemeName("")).toBe(false);
    expect(validThemeName("-x")).toBe(false);
  });
});
