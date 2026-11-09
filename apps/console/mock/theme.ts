/**
 * A dark theme for the mock, so the console can be looked at in both modes
 * without the real serve. Off until `POST /__control/theme?mode=dark|light`
 * turns it on: every other scenario keeps the token block of workspace.css.
 *
 * `modes.dark` is `harbard_core::theme::mode_raw(Shade::Dark)`, copied without the glow and series tokens the light block already gives. The
 * frame family is derived by the core from a theme's colours; the values below
 * are chosen by eye to sit where that derivation lands for Aubergine.
 */
import type { ThemeState } from "../src/lib/theme";

const DARK_MODE: Record<string, string> = {
  list: "#1d2024",
  surface: "#1a1d21",
  "surface-raised": "#25282d",
  "surface-2": "#222529",
  line: "#2f3237",
  "line-strong": "#45484e",
  "line-hover": "#63666c",
  ink: "#e8e8e9",
  "ink-2": "#c9cacc",
  "ink-3": "#a7a9ac",
  "ink-4": "#919398",
  hover: "#212428",
  "hover-2": "#2a2d32",
  control: "#2c2f34",
  "file-hover": "#2a2d32",
  "btn-hover": "#2a2d32",
  "item-hover": "#24272b",
  selected: "#332d1c",
  tint: "#25232a",
  chip: "#302d35",
  "chip-ink": "#ddd8e0",
  "amber-bg": "#3b3014",
  "amber-ink": "#f2c661",
  "red-bg": "#3f1b1e",
  "red-ink": "#f4999d",
  "red-line": "#6e2c31",
  "green-bg": "#1c3320",
  "green-ink": "#94d38f",
  "green-line": "#2f5a35",
  "flag-bg": "#3a2e12",
  "flag-ink": "#f0c36a",
  "flag-line": "#6a5220",
  "ok-bg": "#16302a",
  "ok-ink": "#82d8b3",
  "ok-line": "#2b5b4a",
  "warn-bg": "#3a3016",
  "warn-ink": "#f1d9a0",
  "warn-line": "#5b4a20",
  "note-bg": "#332c1a",
  "banner-ink": "#f8bcbf",
  "warn-icon": "#e8a317",
  send: "#148567",
  "send-ink": "#ffffff",
  focus: "#5aa7e6",
  "focus-line": "#8a8d92",
  "menu-hover": "#1264a3",
  "menu-hover-ink": "#ffffff",
  "av-ink": "#ffffff",
  "av-operator": "#b04a65",
  "av-lead": "#6b6a6b",
  "panel-ring": "rgba(255, 255, 255, 0.08)",
  "sidebar-line": "rgba(255, 255, 255, 0.06)",
  rule: "rgba(255, 255, 255, 0.1)",
  scrim: "rgba(0, 0, 0, 0.6)",
  "robot-tile-1": "#33402f",
  "robot-tile-2": "#2f3a44",
  "robot-tile-3": "#443830",
  "robot-tile-4": "#393346",
  "tone-1": "#846f90",
  "tone-2": "#4f8a8e",
  "tone-3": "#b5784a",
  "tone-4": "#55895a",
  "tone-5": "#c46a5d",
  "tone-6": "#5f7fa8",
  "tone-7": "#9c6a8e",
  "tone-8": "#7d7846",
  "robot-off": "#6a6a6a",
  "st-term": "#0d0e10",
  "st-term-ink": "#ffffff",
  "st-web": "#5cbfdf",
  "st-orbit": "#d684da",
  "st-lens": "rgba(29, 32, 36, 0.6)",
  "color-scheme": "dark",
};

const DARK_FRAME: Record<string, string> = {
  frame: "#2a0b2b",
  "rail-ink": "#ffffff",
  "ws-tile": "#6d3273",
  sidebar: "#1f1a21",
  "sidebar-hover": "rgba(255, 255, 255, 0.07)",
  "sidebar-ink": "#e4dfe6",
  "sidebar-ink-2": "#b4adb7",
  accent: "#a65bb0",
  "accent-deep": "#7c3f86",
  "accent-hover": "#b56cbf",
  "accent-ink": "#ffffff",
  "accent-ink-2": "#f1e4f3",
  "accent-soft": "#3a2a3e",
  "accent-text": "#e0bfe6",
  link: "#d39bdb",
  badge: "#e3c6e8",
};

let mode: "light" | "dark" | null = null;

export function setMockThemeMode(next: string | null): void {
  mode = next === "dark" || next === "light" ? next : null;
}

/** The theme state, or null while no test or screenshot asked for one. */
export function mockTheme(): ThemeState | null {
  if (!mode) return null;
  return {
    version: 1,
    revision: `mock-${mode}`,
    theme: "aubergine",
    mode,
    stored: { theme: "aubergine", mode },
    themes: [{ id: "aubergine", label: "Aubergine", builtin: true, light: {}, dark: DARK_FRAME, share: "" }],
    errors: [],
    dir: "/work/themes",
    modes: { light: {}, dark: DARK_MODE },
  };
}
