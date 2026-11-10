/**
 * The console theme, as serve resolves it (`GET /api/v1/theme`,
 * `harbard_core::theme`). The tokens are computed and checked in Rust; this
 * file only turns them into the stylesheet the shell's root reads, so the
 * first HTML already carries the stored theme and a picker preview repaints
 * without a request.
 */

import { type TranslationKey, createT } from "@/i18n";
import { type ServeParams, pairWhat, serveText } from "./serve-text";
import { DEFAULT_THEME_ID } from "./theme-default";

export { DEFAULT_THEME_ID };

const t = createT("preferences");

export type ThemeMode = "light" | "dark" | "system";

export function modes(): readonly { id: ThemeMode; label: string }[] {
  return [
    { id: "light", label: t("modeLight") },
    { id: "dark", label: t("modeDark") },
    { id: "system", label: t("modeSystem") },
  ];
}

export type Tokens = Record<string, string>;

export interface ThemeEntry {
  id: string;
  label: string;
  builtin: boolean;
  file?: string;
  light: Tokens;
  dark: Tokens;
  /** `harbard-theme:v1:<id>:<hex>,...` */
  share: string;
  /** The file is refused now; these are its last accepted tokens. */
  stale?: boolean;
  /** A seasonal theme: painted by its season, never offered in the picker. */
  season?: string;
}

/** The season, today or next (`harbard_core::theme::season`). */
export interface SeasonState {
  id: string;
  /** The built-in theme that replaces the theme layer while active. */
  theme: string;
  choice: SeasonChoice;
  /** Whether the season paints now. */
  active: boolean;
  /** Today falls inside the dates, whatever the choice. */
  in_window: boolean;
  /** Snow, lights and the rest, while active. */
  decorations: boolean;
  /** `MM-DD`, both included. */
  from: string;
  to: string;
  /** The operator's local date the answer was computed for, `YYYY-MM-DD`. */
  today: string;
}

export type SeasonChoice = "auto" | "on" | "off";

/** Why a theme was refused: file, line, key, and the fix. */
export interface ThemeRefusal {
  file: string;
  line?: number;
  key?: string;
  code: string;
  message: string;
  fix: string;
  /** The console's codes for `message` and `fix`, filled from `params`. */
  message_code?: string;
  fix_code?: string;
  params?: ServeParams;
}

export interface ThemeState {
  version: number;
  revision: string;
  /** The chosen theme: the stored one, or the default when it cannot be. */
  theme: string;
  mode: ThemeMode;
  stored: { theme: string | null; mode: string | null };
  themes: ThemeEntry[];
  errors: ThemeRefusal[];
  notice?: string;
  /** The console's code for `notice`, filled from `notice_params`. */
  notice_code?: string;
  notice_params?: ServeParams;
  dir: string;
  /** "Notify in the Dock": the desktop app bounces once for a new item. */
  dock_attention?: boolean;
  /** Absent from a serve that predates seasons. */
  season?: SeasonState;
  modes: { light: Tokens; dark: Tokens };
}

export interface ThemeUnchanged {
  revision: string;
  unchanged: true;
}

/** The built-in themes by id; serve names them in English whatever the language. */
const BUILTIN_LABELS: Record<string, TranslationKey<"preferences">> = {
  ink: "themeInk",
  harbour: "themeHarbour",
  sage: "themeSage",
  melanzane: "themeMelanzane",
  christmas: "themeChristmas",
  clementine: "themeClementine",
  jade: "themeJade",
  lagoon: "themeLagoon",
  indigo: "themeIndigo",
  graphite: "themeGraphite",
  fjord: "themeFjord",
};

/** The id serve gives the unsaved custom theme while its colours are being edited. */
const DRAFT_ID = "personalizado";

/** A theme's name on screen: a built-in one translated, a user's as they wrote it. */
export function themeLabel(entry: Pick<ThemeEntry, "id" | "label" | "builtin">): string {
  if (entry.id === DRAFT_ID && !entry.builtin) return t("themeCustomDraft");
  const key = entry.builtin ? BUILTIN_LABELS[entry.id] : undefined;
  return key ? t(key) : entry.label;
}

export function isUnchanged(v: ThemeState | ThemeUnchanged): v is ThemeUnchanged {
  return (v as ThemeUnchanged).unchanged === true;
}

/** What the shell paints: a theme's tokens in a mode. */
export interface Applied {
  entry: ThemeEntry | null;
  mode: ThemeMode;
}

/** The operator's choice, which a season never changes. */
export function appliedOf(state: ThemeState | null): Applied {
  if (!state) return { entry: null, mode: "light" };
  return { entry: state.themes.find((t) => t.id === state.theme) ?? null, mode: state.mode };
}

/** The season's theme while it paints, or null. */
export function seasonEntry(state: ThemeState | null): ThemeEntry | null {
  const season = state?.season;
  if (!state || !season?.active) return null;
  return state.themes.find((t) => t.id === season.theme) ?? null;
}

/** The themes the picker offers: every one but the seasonal ones. */
export function pickerThemes(themes: readonly ThemeEntry[]): ThemeEntry[] {
  return themes.filter((t) => !t.season);
}

const NAME = /^[a-z0-9-]+$/;
// A token value is a colour, a shadow list or `light`/`dark`: never a brace,
// semicolon, quote or comment that could close the rule it is written into.
const VALUE = /^[#a-z0-9(),.%\s-]+$/i;

function declarations(tokens: Tokens): string {
  return Object.entries(tokens)
    .filter(([k, v]) => NAME.test(k) && VALUE.test(v))
    .map(([k, v]) => `--hb-${k}:${v};`)
    .join("");
}

/**
 * The stylesheet for `entry` in `mode`. Light tokens on `.hb`, dark ones
 * under `[data-hb-mode="dark"]` and, for "Same as system", under the
 * `prefers-color-scheme: dark` query, so the OS switch repaints live.
 * Unlayered, so it wins over the token block in `workspace.css`.
 */
export function themeCss(state: ThemeState, entry: ThemeEntry): string {
  const light = declarations({ ...state.modes.light, ...entry.light });
  const dark = declarations({ ...state.modes.dark, ...entry.dark });
  return (
    `.hb{${light}}` +
    `.hb[data-hb-mode="dark"]{${dark}}` +
    `@media (prefers-color-scheme: dark){.hb[data-hb-mode="system"]{${dark}}}`
  );
}

export interface RootAttrs {
  "data-hb-theme": string;
  "data-hb-mode": ThemeMode;
  /** The season painting now. */
  "data-hb-season"?: string;
  /** Its decorations are on. */
  "data-hb-decor"?: "";
}

/** The attributes on every `.hb` root. */
export function rootAttrs(applied: Applied, season?: SeasonState | null): RootAttrs {
  const attrs: RootAttrs = { "data-hb-theme": applied.entry?.id ?? DEFAULT_THEME_ID, "data-hb-mode": applied.mode };
  if (season?.active) {
    attrs["data-hb-season"] = season.id;
    if (season.decorations) attrs["data-hb-decor"] = "";
  }
  return attrs;
}

/** The `file` of a refusal that came from no file: serve names the source in English. */
const REFUSAL_SOURCE: Record<string, TranslationKey<"preferences">> = {
  "share code": "refusalShareCode",
  "custom theme": "refusalCustomTheme",
};

/** A refusal's params, with the contrast pair's description in this language. */
function refusalVars(r: ThemeRefusal): ServeParams | undefined {
  const what = r.params?.what_key;
  if (r.params === undefined || typeof what !== "string") return r.params;
  return { ...r.params, what: pairWhat(what, String(r.params.what ?? what)) };
}

function refusalMessage(r: ThemeRefusal): string {
  return serveText(r.message_code, refusalVars(r), r.message);
}

/** One line for a refusal, as the CLI prints it. */
export function describeRefusal(r: ThemeRefusal): string {
  const source = REFUSAL_SOURCE[r.file];
  const file = source ? t(source) : r.file;
  const where = `${file}${r.line ? `:${r.line}` : ""}`;
  const key = r.key ? `\`${r.key}\`: ` : "";
  const fix = serveText(r.fix_code, refusalVars(r), r.fix);
  return `${where}: ${key}${refusalMessage(r)}. ${t("refusalFix", { fix })}`;
}

/** Why the applied theme or mode differs from the stored one, in one sentence. */
export function themeNotice(state: ThemeState): string | null {
  if (!state.notice) return null;
  const params = state.notice_params;
  // The notice quotes a refusal, and that refusal has its own code.
  const refused = typeof params?.refusal === "number" ? state.errors[params.refusal] : undefined;
  return serveText(state.notice_code, refused ? { ...params, why: refusalMessage(refused) } : params, state.notice);
}

/** A custom theme name the picker may save: what serve accepts. */
export function validThemeName(name: string): boolean {
  return /^[a-z0-9][a-z0-9-]{0,39}$/.test(name);
}

/** A display name to a file name: "Mi tema" → "mi-tema". */
export function slugify(label: string): string {
  return label
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
}

/** The swatch a picker card draws: rail, sidebar, accent, in a mode. */
export function swatch(entry: ThemeEntry, mode: ThemeMode, prefersDark: boolean): { frame: string; sidebar: string; accent: string; ink: string; railInk: string } {
  const dark = mode === "dark" || (mode === "system" && prefersDark);
  const t = dark ? entry.dark : entry.light;
  return {
    frame: t.frame ?? "",
    sidebar: t.sidebar ?? "",
    accent: t.accent ?? "",
    ink: t["sidebar-ink"] ?? "",
    railInk: t["rail-ink"] ?? "",
  };
}
