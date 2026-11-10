import { describe, expect, test } from "bun:test";
import { DEFAULT_THEME_ID, appliedOf, describeRefusal, pickerThemes, rootAttrs, seasonEntry, slugify, themeCss, validThemeName, type SeasonState, type ThemeEntry } from "./theme";
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
  test("the stored theme and mode, or the light default without serve", () => {
    expect(rootAttrs(appliedOf(STATE))).toEqual({ "data-hb-theme": "jade", "data-hb-mode": "system" });
    expect(rootAttrs(appliedOf(null))).toEqual({ "data-hb-theme": DEFAULT_THEME_ID, "data-hb-mode": "light" });
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

describe("the season", () => {
  const christmas: ThemeEntry = { id: "christmas", label: "Christmas", builtin: true, light: { frame: "#1b3a2c" }, dark: {}, share: "", season: "christmas" };
  const season = (over: Partial<SeasonState>): SeasonState => ({
    id: "christmas",
    theme: "christmas",
    choice: "auto",
    active: true,
    in_window: true,
    decorations: true,
    from: "12-01",
    to: "01-06",
    today: "2026-12-24",
    ...over,
  });
  const withSeason = (s: SeasonState) => ({ ...STATE, themes: [jade, christmas], season: s });

  test("paints its theme while active and leaves the choice as it was", () => {
    const state = withSeason(season({}));
    expect(seasonEntry(state)?.id).toBe("christmas");
    expect(appliedOf(state).entry?.id).toBe("jade");
  });

  test("paints nothing when off, outside its dates or from a serve without seasons", () => {
    expect(seasonEntry(withSeason(season({ active: false })))).toBeNull();
    expect(seasonEntry(STATE)).toBeNull();
  });

  test("is never offered in the theme picker", () => {
    expect(pickerThemes([jade, christmas]).map((t) => t.id)).toEqual(["jade"]);
  });

  test("marks the root while active, and its decorations only when they are on", () => {
    const applied = { entry: christmas, mode: "light" as const };
    expect(rootAttrs(applied, season({}))).toEqual({ "data-hb-theme": "christmas", "data-hb-mode": "light", "data-hb-season": "christmas", "data-hb-decor": "" });
    expect(rootAttrs(applied, season({ decorations: false }))["data-hb-decor"]).toBeUndefined();
    expect(rootAttrs(appliedOf(null), season({ active: false }))).toEqual({ "data-hb-theme": DEFAULT_THEME_ID, "data-hb-mode": "light" });
  });
});
