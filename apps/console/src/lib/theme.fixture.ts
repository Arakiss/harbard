/** A theme state as serve answers it, for the theme tests. */
import type { ThemeEntry, ThemeState } from "./theme";

export const jade: ThemeEntry = {
  id: "jade",
  label: "Jade",
  builtin: true,
  light: { frame: "#0d3b2e", accent: "#0f7a56", "rail-active": "rgba(255, 255, 255, 0.22)" },
  dark: { frame: "#0b3127", accent: "#13805b" },
  share: "harbard-theme:v1:jade:…",
};

export const STATE: ThemeState = {
  version: 1,
  revision: "r1",
  theme: "jade",
  mode: "system",
  stored: { theme: "jade", mode: "system" },
  themes: [jade],
  errors: [],
  dir: "/tmp/home/themes",
  modes: { light: { surface: "#ffffff", "color-scheme": "light" }, dark: { surface: "#1a1d21", "color-scheme": "dark" } },
};

