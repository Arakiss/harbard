/**
 * The theme for the mock, so the console can be looked at in every built-in
 * theme, both modes and its season without the real serve. Off until
 * `POST /__control/theme?mode=dark|light` turns it on: every other scenario
 * keeps the token block of workspace.css.
 *
 * `/__control/theme` also takes `theme=<id>`, `season=auto|on|off`,
 * `decorations=on|off` and `today=YYYY-MM-DD` (the operator's local date the
 * season is judged on). `theme-tokens.json` is every built-in as serve
 * resolves it, written by `cargo test -p harbard-core write_console_files
 * -- --ignored` and checked against the core by a test.
 */
import type { SeasonState, ThemeEntry, ThemeState, Tokens } from "../src/lib/theme";
import resolved from "./theme-tokens.json";

interface SeasonDef {
  id: string;
  theme: string;
  from: string;
  to: string;
}

// The JSON is serve's `ResolvedTheme` and mode layer, checked by the core's test.
const THEMES = resolved.themes as unknown as ThemeEntry[];
const MODES = resolved.modes as { light: Tokens; dark: Tokens };
const SEASONS = resolved.seasons as SeasonDef[];

interface MockTheme {
  mode: "light" | "dark" | null;
  theme: string;
  season: "auto" | "on" | "off";
  decorations: boolean;
  today: string | null;
}

const initial = (): MockTheme => ({ mode: null, theme: resolved.default, season: "auto", decorations: true, today: null });
let current: MockTheme = initial();
let writes = 0;

/** `/__control/theme`: every parameter left out keeps its value; no `mode` turns the theme off. */
export function setMockTheme(params: URLSearchParams): void {
  const mode = params.get("mode");
  if (mode !== "dark" && mode !== "light") {
    current = initial();
    return;
  }
  const theme = params.get("theme");
  const season = params.get("season");
  const decorations = params.get("decorations");
  current = {
    mode,
    theme: theme && THEMES.some((t) => t.id === theme) ? theme : current.theme,
    season: season === "on" || season === "off" || season === "auto" ? season : current.season,
    decorations: decorations === null ? current.decorations : decorations !== "off",
    today: params.get("today") ?? current.today,
  };
  writes += 1;
}

function localToday(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function inWindow(def: SeasonDef, mmdd: string): boolean {
  return def.from <= def.to ? def.from <= mmdd && mmdd <= def.to : mmdd >= def.from || mmdd <= def.to;
}

/** As `harbard_core::theme::season::state`: the season today, or the next. */
function seasonOf(): SeasonState | undefined {
  const today = current.today ?? localToday();
  const mmdd = today.slice(5);
  const ordinal = (md: string) => (Number(md.slice(0, 2)) - 1) * 31 + Number(md.slice(3));
  const until = (s: SeasonDef) => (ordinal(s.from) - ordinal(mmdd) + 12 * 31) % (12 * 31);
  const def = SEASONS.find((s) => inWindow(s, mmdd)) ?? SEASONS.slice().sort((a, b) => until(a) - until(b))[0];
  if (!def) return undefined;
  const within = inWindow(def, mmdd);
  return {
    id: def.id,
    theme: def.theme,
    choice: current.season,
    active: current.season === "on" || (current.season === "auto" && within),
    in_window: within,
    decorations: current.decorations,
    from: def.from,
    to: def.to,
    today,
  };
}

/** The theme state, or null while no test or screenshot asked for one. */
export function mockTheme(): ThemeState | null {
  if (!current.mode) return null;
  const season = seasonOf();
  return {
    version: 1,
    revision: `mock-${writes}`,
    theme: current.theme,
    mode: current.mode,
    stored: { theme: current.theme, mode: current.mode },
    themes: THEMES,
    errors: [],
    dir: "/work/themes",
    dock_attention: true,
    ...(season ? { season } : {}),
    modes: MODES,
  };
}

/** `POST /api/v1/workspace/theme-*` on the mock: the writes Preferences makes. */
export function mockThemeWrite(action: string, body: Record<string, unknown>): ThemeState | null {
  if (!current.mode) current = { ...current, mode: "light" };
  if (action === "theme-set" && typeof body.theme === "string" && THEMES.some((t) => t.id === body.theme)) current.theme = body.theme;
  if (action === "theme-mode" && (body.mode === "light" || body.mode === "dark")) current.mode = body.mode;
  if (action === "theme-season" && (body.season === "auto" || body.season === "on" || body.season === "off")) current.season = body.season;
  if (action === "theme-decorations" && typeof body.on === "boolean") current.decorations = body.on;
  writes += 1;
  return mockTheme();
}
