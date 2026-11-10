"use client";

import * as React from "react";
import { SeasonRobotsProvider } from "@/components/robot/season-robots";
import { fetchTheme, postTheme } from "@/lib/api";
import { appliedOf, isUnchanged, rootAttrs, seasonEntry, themeCss, type Applied, type RootAttrs, type SeasonChoice, type ThemeEntry, type ThemeMode, type ThemeState } from "@/lib/theme";

/**
 * How often the shell asks serve whether the theme changed. Serve stats the
 * themes folder once a second, so an edited file repaints within ~3 s.
 */
const POLL_MS = 1_500;

/** A look the picker shows without choosing it. */
export interface Preview {
  entry?: ThemeEntry;
  mode?: ThemeMode;
}

interface Ctx {
  state: ThemeState | null;
  /** What is chosen (stored), before any preview. */
  chosen: Applied;
  /** What is painted: the choice, the season over it, the preview over both. */
  applied: Applied;
  preview: (p: Preview | null) => void;
  choose: (id: string) => Promise<void>;
  setMode: (mode: ThemeMode) => Promise<void>;
  setSeason: (season: SeasonChoice) => Promise<void>;
  setDecorations: (on: boolean) => Promise<void>;
  /** Run a theme write (save, import) and adopt serve's answer. */
  write: (run: () => Promise<ThemeState>) => Promise<ThemeState>;
}

const ThemeContext = React.createContext<Ctx | null>(null);

export function ThemeProvider({ initial, children }: { initial?: ThemeState; children: React.ReactNode }) {
  const [state, setState] = React.useState<ThemeState | null>(initial ?? null);
  const [preview, setPreview] = React.useState<Preview | null>(null);
  const revision = React.useRef<string | undefined>(initial?.revision);
  // Bumped when a write starts and when it lands: a poll that started
  // before either is stale and is dropped, so it never repaints the theme
  // the operator just left.
  const writes = React.useRef(0);

  const adopt = React.useCallback((next: ThemeState) => {
    revision.current = next.revision;
    setState(next);
  }, []);

  const write = React.useCallback(
    async (run: () => Promise<ThemeState>) => {
      writes.current += 1;
      try {
        const next = await run();
        adopt(next);
        return next;
      } finally {
        writes.current += 1;
      }
    },
    [adopt],
  );

  React.useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let controller: AbortController | undefined;
    const tick = async () => {
      clearTimeout(timer);
      if (document.visibilityState === "visible") {
        controller?.abort();
        controller = new AbortController();
        const started = writes.current;
        try {
          const body = await fetchTheme(revision.current, controller.signal);
          if (alive && started === writes.current && !isUnchanged(body) && Array.isArray(body.themes)) adopt(body);
        } catch {
          // serve is down or predates themes: keep what is painted.
        }
      }
      if (alive) timer = setTimeout(tick, POLL_MS);
    };
    timer = setTimeout(tick, initial ? POLL_MS : 0);
    const onVisible = () => {
      if (document.visibilityState === "visible") void tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      alive = false;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisible);
    };
    // The initial state is read once, on mount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const chosen = React.useMemo(() => appliedOf(state), [state]);
  const seasonal = React.useMemo(() => seasonEntry(state), [state]);
  const applied = React.useMemo<Applied>(
    () => ({ entry: preview?.entry ?? seasonal ?? chosen.entry, mode: preview?.mode ?? chosen.mode }),
    [preview, seasonal, chosen],
  );

  const choose = React.useCallback(
    async (id: string) => {
      await write(() => postTheme("theme-set", { theme: id }));
    },
    [write],
  );
  const setMode = React.useCallback(
    async (mode: ThemeMode) => {
      await write(() => postTheme("theme-mode", { mode }));
    },
    [write],
  );

  const setSeason = React.useCallback(
    async (season: SeasonChoice) => {
      await write(() => postTheme("theme-season", { season }));
    },
    [write],
  );
  const setDecorations = React.useCallback(
    async (on: boolean) => {
      await write(() => postTheme("theme-decorations", { on }));
    },
    [write],
  );

  const value = React.useMemo<Ctx>(
    () => ({ state, chosen, applied, preview: setPreview, choose, setMode, setSeason, setDecorations, write }),
    [state, chosen, applied, choose, setMode, setSeason, setDecorations, write],
  );
  const css = state && applied.entry ? themeCss(state, applied.entry) : null;
  return (
    <ThemeContext.Provider value={value}>
      {/* Raw: token names and values are filtered in themeCss to exclude
          anything that could close the rule or the element. */}
      {css ? <style data-hb-theme-style="" dangerouslySetInnerHTML={{ __html: css }} /> : null}
      <SeasonRobotsProvider season={state?.season?.active ? state.season.id : null}>{children}</SeasonRobotsProvider>
    </ThemeContext.Provider>
  );
}

export function useTheme(): Ctx | null {
  return React.useContext(ThemeContext);
}

/** `data-hb-theme`, `data-hb-mode` and the season's for a `.hb` root. */
export function useThemeAttrs(): RootAttrs {
  const ctx = React.useContext(ThemeContext);
  return rootAttrs(ctx?.applied ?? { entry: null, mode: "light" }, ctx?.state?.season);
}

